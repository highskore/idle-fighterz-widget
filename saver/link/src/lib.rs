//! The screensaver's live view of the widget's run. The widget is the server (`Hub`): while a screensaver is connected
//! it draws every step and sends each frame; the screensaver (`Client`) shows them, and falls back to a demo run of its
//! own on any error. Nothing touches the disk. See saver/README.md.
//!
//! Wire format, all integers little-endian:
//! - hello, both ways (the screensaver first): `"IFZS"`, version `u16`, flags `u16` (0), user `u32` (the uid on
//!   macOS, 0 on Windows, where the pipe's name is per user)
//! - then server to screensaver only, messages: kind `u8`, payload length `u32`, payload. Kind 1 is a frame: width
//!   `u16`, height `u16`, encoding `u8`, power length `u8`, the power level (UTF-8), then the pixels: RGBA8 rows top to
//!   bottom (encoding 0), or runs of `count u16` + one RGBA8 pixel (encoding 1). Other kinds are skipped.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const MAGIC: [u8; 4] = *b"IFZS";
pub const VERSION: u16 = 1;
pub const HELLO_LEN: usize = 12;
pub const KIND_FRAME: u8 = 1;
pub const ENC_RAW: u8 = 0;
pub const ENC_RLE: u8 = 1;
/// widest and tallest frame either side accepts (the widget's are at most 280×258 game pixels)
pub const MAX_SIDE: usize = 1024;
pub const MAX_POWER: usize = 32;
const FRAME_HEAD: usize = 6;
/// the biggest payload a reader allocates: a frame of MAX_SIDE² pixels in runs of one (6 bytes a pixel)
pub const MAX_PAYLOAD: usize = FRAME_HEAD + MAX_POWER + MAX_SIDE * MAX_SIDE * 6;
/// screensavers served at once (a multi-display Mac runs one view per screen)
pub const MAX_CLIENTS: usize = 6;
/// a client whose write has been stuck this long no longer counts as watching
const STUCK: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Io(io::ErrorKind),
    /// nothing arrived before the read timed out (the stream is still in step)
    Idle,
    Magic,
    Version(u16),
    User,
    TooBig(usize),
    Bad(&'static str),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e.kind())
    }
}

pub fn hello(user: u32) -> [u8; HELLO_LEN] {
    let mut b = [0u8; HELLO_LEN];
    b[..4].copy_from_slice(&MAGIC);
    b[4..6].copy_from_slice(&VERSION.to_le_bytes());
    b[8..12].copy_from_slice(&user.to_le_bytes());
    b
}

/// The other side's user, if its hello is ours and the same version.
pub fn parse_hello(b: &[u8; HELLO_LEN]) -> Result<u32, Error> {
    if b[..4] != MAGIC {
        return Err(Error::Magic);
    }
    let v = u16::from_le_bytes([b[4], b[5]]);
    if v != VERSION {
        return Err(Error::Version(v));
    }
    Ok(u32::from_le_bytes([b[8], b[9], b[10], b[11]]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub power: String,
    /// RGBA8, straight alpha, rows top to bottom
    pub rgba: Vec<u8>,
}

/// The whole frame message (kind, length, payload), run-length encoded unless raw is smaller. None if the size is out
/// of range or `rgba` isn't `w`×`h` pixels. The power level is cut to MAX_POWER bytes.
pub fn encode_frame(rgba: &[u8], w: usize, h: usize, power: &str) -> Option<Vec<u8>> {
    if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) || rgba.len() != w * h * 4 {
        return None;
    }
    let mut cut = power.len().min(MAX_POWER);
    while !power.is_char_boundary(cut) {
        cut -= 1;
    }
    let power = &power.as_bytes()[..cut];
    let mut runs = Vec::with_capacity(rgba.len() / 4);
    let mut px = rgba.as_chunks::<4>().0.iter();
    let mut cur = px.next().unwrap();
    let mut n: u16 = 1;
    for p in px {
        if p == cur && n < u16::MAX {
            n += 1;
        } else {
            runs.extend_from_slice(&n.to_le_bytes());
            runs.extend_from_slice(cur);
            cur = p;
            n = 1;
        }
    }
    runs.extend_from_slice(&n.to_le_bytes());
    runs.extend_from_slice(cur);
    let (enc, pixels) = if runs.len() < rgba.len() { (ENC_RLE, &runs[..]) } else { (ENC_RAW, rgba) };
    let len = FRAME_HEAD + power.len() + pixels.len();
    let mut m = Vec::with_capacity(5 + len);
    m.push(KIND_FRAME);
    m.extend_from_slice(&(len as u32).to_le_bytes());
    m.extend_from_slice(&(w as u16).to_le_bytes());
    m.extend_from_slice(&(h as u16).to_le_bytes());
    m.push(enc);
    m.push(power.len() as u8);
    m.extend_from_slice(power);
    m.extend_from_slice(pixels);
    Some(m)
}

/// A frame message's payload, checked: the size in range, the power level valid UTF-8, and exactly `w`×`h` pixels.
pub fn decode_frame(p: &[u8]) -> Result<Frame, Error> {
    if p.len() < FRAME_HEAD {
        return Err(Error::Bad("short frame"));
    }
    let w = u16::from_le_bytes([p[0], p[1]]) as usize;
    let h = u16::from_le_bytes([p[2], p[3]]) as usize;
    let (enc, plen) = (p[4], p[5] as usize);
    if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
        return Err(Error::Bad("frame size"));
    }
    if plen > MAX_POWER || p.len() < FRAME_HEAD + plen {
        return Err(Error::Bad("power level"));
    }
    let power = std::str::from_utf8(&p[FRAME_HEAD..FRAME_HEAD + plen]).map_err(|_| Error::Bad("power level"))?.to_string();
    let data = &p[FRAME_HEAD + plen..];
    let n = w * h;
    let rgba = match enc {
        ENC_RAW if data.len() == n * 4 => data.to_vec(),
        ENC_RAW => return Err(Error::Bad("pixel count")),
        ENC_RLE => {
            if !data.len().is_multiple_of(6) || data.len() / 6 > n {
                return Err(Error::Bad("pixel count"));
            }
            let mut out = Vec::with_capacity(n * 4);
            for run in data.as_chunks::<6>().0 {
                let k = u16::from_le_bytes([run[0], run[1]]) as usize;
                if k == 0 || out.len() / 4 + k > n {
                    return Err(Error::Bad("pixel count"));
                }
                for _ in 0..k {
                    out.extend_from_slice(&run[2..6]);
                }
            }
            if out.len() != n * 4 {
                return Err(Error::Bad("pixel count"));
            }
            out
        }
        _ => return Err(Error::Bad("encoding")),
    };
    Ok(Frame { w, h, power, rgba })
}

// fills `buf`, riding out read timeouts once a message has begun (up to `patience` of them); Idle if nothing came
fn fill(r: &mut impl Read, buf: &mut [u8], begun: bool, mut patience: u32) -> Result<(), Error> {
    let mut at = 0;
    while at < buf.len() {
        match r.read(&mut buf[at..]) {
            Ok(0) => return Err(Error::Io(io::ErrorKind::UnexpectedEof)),
            Ok(n) => at += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                if at == 0 && !begun {
                    return Err(Error::Idle);
                }
                if patience == 0 {
                    return Err(e.into());
                }
                patience -= 1;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// The next message as (kind, payload). A length over MAX_PAYLOAD is refused before anything is allocated.
pub fn read_message(r: &mut impl Read) -> Result<(u8, Vec<u8>), Error> {
    let mut head = [0u8; 5];
    fill(r, &mut head, false, 4)?;
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_PAYLOAD {
        return Err(Error::TooBig(len));
    }
    let mut p = vec![0u8; len];
    fill(r, &mut p, true, 4)?;
    Ok((head[0], p))
}

/// The next frame, skipping messages of other kinds.
pub fn read_frame(r: &mut impl Read) -> Result<Frame, Error> {
    loop {
        let (kind, p) = read_message(r)?;
        if kind == KIND_FRAME {
            return decode_frame(&p);
        }
    }
}

// ── transport: TCP on loopback (the macOS screensaver host's sandbox allows network clients, nothing else of ours),
// a named pipe on Windows

#[cfg(not(windows))]
pub type Conn = std::net::TcpStream;
#[cfg(windows)]
pub type Conn = std::fs::File;

/// Where the widget listens.
#[derive(Clone, Debug)]
pub enum Endpoint {
    #[cfg(not(windows))]
    Tcp(Vec<std::net::SocketAddr>),
    #[cfg(windows)]
    Pipe(String),
}

/// This user's uid (macOS, Linux), 0 on Windows.
pub fn user() -> u32 {
    #[cfg(unix)]
    {
        extern "C" {
            fn getuid() -> u32;
        }
        // SAFETY: getuid has no preconditions and can't fail
        unsafe { getuid() }
    }
    #[cfg(not(unix))]
    0
}

/// This user's endpoint: four loopback ports picked by uid (below macOS's ephemeral range), or a pipe named for the user.
pub fn endpoint() -> Endpoint {
    #[cfg(windows)]
    {
        let who: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        Endpoint::Pipe(format!(r"\\.\pipe\idle-fighterz-saver-{who}"))
    }
    #[cfg(not(windows))]
    {
        let base = 41000 + (user() % 2000) as u16 * 4;
        Endpoint::Tcp((base..base + 4).map(|p| std::net::SocketAddr::from(([127, 0, 0, 1], p))).collect())
    }
}

fn timeouts(c: &Conn, d: Option<Duration>) {
    #[cfg(not(windows))]
    {
        let _ = c.set_read_timeout(d);
        let _ = c.set_write_timeout(d);
        let _ = c.set_nodelay(true);
    }
    #[cfg(windows)]
    let _ = (c, d);
}

/// Connects as a screensaver: the first address (or the pipe) that answers with our hello. The connection's reads
/// time out after a second (TCP), so a reader can look up between frames.
pub fn connect(ep: &Endpoint) -> Result<Conn, Error> {
    let shake = |mut c: Conn| -> Result<Conn, Error> {
        timeouts(&c, Some(Duration::from_secs(1)));
        c.write_all(&hello(user()))?;
        let mut h = [0u8; HELLO_LEN];
        fill(&mut c, &mut h, true, 1)?;
        if parse_hello(&h)? != user() {
            return Err(Error::User);
        }
        Ok(c)
    };
    match ep {
        #[cfg(not(windows))]
        Endpoint::Tcp(addrs) => {
            let mut last = Error::Io(io::ErrorKind::NotFound);
            for a in addrs {
                match std::net::TcpStream::connect_timeout(a, Duration::from_millis(250)).map_err(Error::from).and_then(shake) {
                    Ok(c) => return Ok(c),
                    Err(e) => last = e,
                }
            }
            Err(last)
        }
        #[cfg(windows)]
        Endpoint::Pipe(name) => shake(std::fs::OpenOptions::new().read(true).write(true).open(name)?),
    }
}

enum Listener {
    #[cfg(not(windows))]
    Tcp(std::net::TcpListener),
    /// the pipe's name, and the instance the next client will open (a raw handle)
    #[cfg(windows)]
    Pipe { name: Vec<u16>, next: Option<isize> },
}

impl Listener {
    fn bind(ep: &Endpoint) -> io::Result<Listener> {
        match ep {
            #[cfg(not(windows))]
            Endpoint::Tcp(addrs) => {
                let mut last = io::Error::from(io::ErrorKind::AddrInUse);
                for a in addrs {
                    match std::net::TcpListener::bind(a) {
                        Ok(l) => return Ok(Listener::Tcp(l)),
                        Err(e) => last = e,
                    }
                }
                Err(last)
            }
            #[cfg(windows)]
            Endpoint::Pipe(name) => {
                let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
                // the first instance claims the name now, so a second widget fails here rather than at accept
                let next = Some(pipe::create(&name, true)?);
                Ok(Listener::Pipe { name, next })
            }
        }
    }

    fn accept(&mut self) -> io::Result<Conn> {
        match self {
            #[cfg(not(windows))]
            Listener::Tcp(l) => l.accept().map(|(c, _)| c),
            #[cfg(windows)]
            Listener::Pipe { name, next } => {
                let h = match next.take() {
                    Some(h) => h,
                    None => pipe::create(name, false)?,
                };
                pipe::connect(h)
            }
        }
    }
}

#[cfg(windows)]
mod pipe {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
        System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT},
    };

    /// A new instance of the pipe (`first`: failing if another process has the name), as a raw handle.
    pub fn create(name: &[u16], first: bool) -> std::io::Result<isize> {
        let open = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
        // SAFETY: `name` is NUL-terminated UTF-16
        let h = unsafe { CreateNamedPipeW(name.as_ptr(), open, PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 1 << 16, 1 << 12, 0, std::ptr::null()) };
        if h == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        Ok(h as isize)
    }

    /// Waits for a client to open the instance `h`; the File owns it from then on.
    pub fn connect(h: isize) -> std::io::Result<std::fs::File> {
        let h = h as HANDLE;
        // SAFETY: `h` is a pipe instance from `create`, owned here: kept by the File, or closed on failure
        unsafe {
            if ConnectNamedPipe(h, std::ptr::null_mut()) == 0 && GetLastError() != ERROR_PIPE_CONNECTED {
                let e = std::io::Error::last_os_error();
                CloseHandle(h);
                return Err(e);
            }
            Ok(std::fs::File::from_raw_handle(h as _))
        }
    }
}

// ── the widget's side

struct Slot {
    next: Mutex<Option<Arc<Vec<u8>>>>,
    ready: Condvar,
    gone: AtomicBool,
    /// ms since the hub started at which the current write began, plus one; 0 when not writing
    writing: AtomicU64,
}

struct Inner {
    slots: Mutex<Vec<Arc<Slot>>>,
    /// connections being greeted, which count towards MAX_CLIENTS
    pending: Mutex<usize>,
    changed: Box<dyn Fn() + Send + Sync>,
    epoch: Instant,
}

impl Inner {
    fn ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + 1
    }
}

/// The widget's server: screensavers connect, and while any is connected (`attached`) the widget `publish`es each
/// frame. `changed` runs whenever one connects or goes.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

impl Hub {
    pub fn start(changed: impl Fn() + Send + Sync + 'static) -> io::Result<Hub> {
        Self::start_on(&endpoint(), changed)
    }

    pub fn start_on(ep: &Endpoint, changed: impl Fn() + Send + Sync + 'static) -> io::Result<Hub> {
        let mut l = Listener::bind(ep)?;
        let inner = Arc::new(Inner { slots: Mutex::default(), pending: Mutex::new(0), changed: Box::new(changed), epoch: Instant::now() });
        let hub = Hub { inner: inner.clone() };
        std::thread::Builder::new().name("saver-accept".into()).spawn(move || loop {
            match l.accept() {
                Ok(c) => {
                    let inner = inner.clone();
                    let mut pending = inner.pending.lock().unwrap();
                    if *pending + inner.slots.lock().unwrap().len() >= MAX_CLIENTS {
                        continue;
                    }
                    *pending += 1;
                    drop(pending);
                    let _ = std::thread::Builder::new().name("saver-client".into()).spawn(move || serve(inner, c));
                }
                Err(_) => std::thread::sleep(Duration::from_secs(1)),
            }
        })?;
        Ok(hub)
    }

    /// Whether a screensaver is watching: connected, and not stuck on a write.
    pub fn attached(&self) -> bool {
        let now = self.inner.ms();
        self.inner.slots.lock().unwrap().iter().any(|s| {
            let w = s.writing.load(Ordering::Relaxed);
            !s.gone.load(Ordering::Relaxed) && (w == 0 || now - w < STUCK.as_millis() as u64)
        })
    }

    /// Sends a frame (RGBA8, `w`×`h`) and the power level to every screensaver; a newer frame replaces one not yet sent.
    pub fn publish(&self, rgba: &[u8], w: usize, h: usize, power: &str) {
        let slots = self.inner.slots.lock().unwrap();
        if slots.is_empty() {
            return;
        }
        let Some(m) = encode_frame(rgba, w, h, power) else { return };
        let m = Arc::new(m);
        for s in slots.iter() {
            *s.next.lock().unwrap() = Some(m.clone());
            s.ready.notify_one();
        }
    }
}

fn serve(inner: Arc<Inner>, mut c: Conn) {
    let greet = (|| -> Result<(), Error> {
        timeouts(&c, Some(Duration::from_secs(2)));
        let mut h = [0u8; HELLO_LEN];
        fill(&mut c, &mut h, true, 0)?;
        // our hello either way, so a screensaver of another version knows to play its own run
        c.write_all(&hello(user()))?;
        if parse_hello(&h)? != user() {
            return Err(Error::User);
        }
        Ok(())
    })();
    *inner.pending.lock().unwrap() -= 1;
    if greet.is_err() {
        return;
    }
    let slot = Arc::new(Slot { next: Mutex::new(None), ready: Condvar::new(), gone: AtomicBool::new(false), writing: AtomicU64::new(0) });
    inner.slots.lock().unwrap().push(slot.clone());
    (inner.changed)();
    loop {
        let next = {
            let g = slot.next.lock().unwrap();
            let (mut g, _) = slot.ready.wait_timeout_while(g, Duration::from_secs(1), |n| n.is_none()).unwrap();
            g.take()
        };
        let Some(m) = next else { continue };
        slot.writing.store(inner.ms(), Ordering::Relaxed);
        if c.write_all(&m).is_err() {
            break;
        }
        slot.writing.store(0, Ordering::Relaxed);
    }
    slot.gone.store(true, Ordering::Relaxed);
    inner.slots.lock().unwrap().retain(|s| !Arc::ptr_eq(s, &slot));
    (inner.changed)();
}

#[cfg(test)]
mod tests;
