//! The screensaver's live view of the widget's run. The widget is the server (`Hub`): while a screensaver is connected
//! it draws every step and sends each frame; the screensaver (`Client`) shows them, and falls back to a demo run of its
//! own on any error. The run never touches the disk; the only file is the link's key. See saver/README.md.
//!
//! Wire format, all integers little-endian:
//! - the handshake. Screensaver: hello (`"IFZS"`, version `u16`, flags `u16` (0), user `u32`: the uid on macOS, 0 on
//!   Windows) and a 16-byte nonce. Widget: its hello, its nonce, and HMAC-SHA256(key, "IFZS widget" ‖ both nonces).
//!   Screensaver: HMAC-SHA256(key, "IFZS saver" ‖ both nonces). The key is 32 random bytes the widget writes at launch
//!   where only the user can read them (`endpoint`); either side drops a peer that can't prove it, within 2 s
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
    /// the other end doesn't hold this user's key (or ours can't be read)
    Auth,
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

// ── transport: TCP on loopback (the macOS screensaver host's sandbox allows network clients and nothing else of
// ours), a named pipe on Windows. Neither says who is at the other end, so both sides prove they hold this user's key
// (a random file only the user can read, new each time the widget starts) before a frame moves

#[cfg(not(windows))]
pub type Conn = std::net::TcpStream;
#[cfg(windows)]
pub type Conn = std::fs::File;

/// Where the widget listens.
#[derive(Clone, Debug)]
pub enum Addr {
    #[cfg(not(windows))]
    Tcp(Vec<std::net::SocketAddr>),
    #[cfg(windows)]
    Pipe(String),
}

/// Where the widget listens, and the key both sides prove they hold.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub at: Addr,
    pub key: std::path::PathBuf,
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

// the user's own home: inside the screensaver host's sandbox HOME is its container, under the home
#[cfg(not(windows))]
fn home() -> std::path::PathBuf {
    let h = std::env::var("HOME").unwrap_or_default();
    let real = h.find("/Library/Containers/").map_or(h.as_str(), |i| &h[..i]);
    std::path::PathBuf::from(real)
}

/// This user's endpoint. macOS: four loopback ports picked by uid (below the ephemeral range), the key in
/// `~/Library/Application Support/com.highskore.idlefighterz.saver/key`, which the screensaver's sandbox can read.
/// Windows: a pipe named for the user's SID, the key in `%LOCALAPPDATA%\IdleFighterZ\saver.key`.
pub fn endpoint() -> Endpoint {
    #[cfg(windows)]
    {
        let who = pipe::user_sid()
            .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect());
        let local = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_default();
        Endpoint { at: Addr::Pipe(format!(r"\\.\pipe\idle-fighterz-saver-{who}")), key: local.join(r"IdleFighterZ\saver.key") }
    }
    #[cfg(not(windows))]
    {
        let base = 41000 + (user() % 2000) as u16 * 4;
        Endpoint {
            at: Addr::Tcp((base..base + 4).map(|p| std::net::SocketAddr::from(([127, 0, 0, 1], p))).collect()),
            key: home().join("Library/Application Support/com.highskore.idlefighterz.saver/key"),
        }
    }
}

pub const KEY_LEN: usize = 32;
const NONCE: usize = 16;
const MAC: usize = 32;
/// how long either side waits for the other's half of the handshake
const SHAKE: Duration = Duration::from_secs(2);

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

fn mac(key: &[u8; KEY_LEN], role: &[u8], nc: &[u8], ns: &[u8]) -> HmacSha256 {
    use hmac::Mac;
    let mut m = HmacSha256::new_from_slice(key).expect("any key length");
    m.update(role);
    m.update(nc);
    m.update(ns);
    m
}

fn tag(key: &[u8; KEY_LEN], role: &[u8], nc: &[u8], ns: &[u8]) -> [u8; MAC] {
    use hmac::Mac;
    mac(key, role, nc, ns).finalize().into_bytes().into()
}

// constant time
fn check(key: &[u8; KEY_LEN], role: &[u8], nc: &[u8], ns: &[u8], got: &[u8]) -> Result<(), Error> {
    use hmac::Mac;
    mac(key, role, nc, ns).verify_slice(got).map_err(|_| Error::Auth)
}

fn nonce() -> Result<[u8; NONCE], Error> {
    let mut n = [0u8; NONCE];
    getrandom::fill(&mut n).map_err(|_| Error::Auth)?;
    Ok(n)
}

/// The widget's side: a new random key, written where only this user can read it (0600 in a 0700 folder), replacing
/// the last one.
pub fn new_key(path: &std::path::Path) -> io::Result<[u8; KEY_LEN]> {
    let mut k = [0u8; KEY_LEN];
    getrandom::fill(&mut k).map_err(|e| io::Error::other(e.to_string()))?;
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        o.mode(0o600);
    }
    o.open(&tmp)?.write_all(&k)?;
    std::fs::rename(&tmp, path)?;
    Ok(k)
}

/// The screensaver's side: the key, if it's a regular file of the right size that this user owns and no one else can
/// read or write.
pub fn read_key(path: &std::path::Path) -> Result<[u8; KEY_LEN], Error> {
    let m = std::fs::symlink_metadata(path).map_err(|_| Error::Auth)?;
    if !m.is_file() || m.len() != KEY_LEN as u64 {
        return Err(Error::Auth);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.uid() != user() || m.mode() & 0o077 != 0 {
            return Err(Error::Auth);
        }
    }
    let mut k = [0u8; KEY_LEN];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut k)).map_err(|_| Error::Auth)?;
    Ok(k)
}

// reads exactly `buf` within `d`, or fails
fn recv(c: &mut Conn, buf: &mut [u8], d: Duration) -> Result<(), Error> {
    #[cfg(not(windows))]
    {
        let _ = c.set_read_timeout(Some(d));
        fill(c, buf, true, 0)
    }
    #[cfg(windows)]
    {
        // a synchronous pipe read can't time out: wait until the bytes are there, then read them
        let end = Instant::now() + d;
        let mut at = 0;
        while at < buf.len() {
            let n = pipe::available(c)?;
            if n > 0 {
                let upto = (at + n).min(buf.len());
                at += c.read(&mut buf[at..upto])?;
            } else if Instant::now() >= end {
                return Err(Error::Io(io::ErrorKind::TimedOut));
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn set_timeouts(c: &Conn, read: Duration, write: Duration) {
    let _ = c.set_read_timeout(Some(read));
    let _ = c.set_write_timeout(Some(write));
    let _ = c.set_nodelay(true);
}

// the screensaver's half of the handshake: our hello and nonce; the widget's hello, nonce and proof; our proof
fn shake_client(c: &mut Conn, key: &[u8; KEY_LEN]) -> Result<(), Error> {
    let nc = nonce()?;
    c.write_all(&[hello(user()).as_slice(), &nc].concat())?;
    let mut h = [0u8; HELLO_LEN];
    recv(c, &mut h, SHAKE)?;
    if parse_hello(&h)? != user() {
        return Err(Error::User);
    }
    let mut rest = [0u8; NONCE + MAC];
    recv(c, &mut rest, SHAKE)?;
    let (ns, proof) = rest.split_at(NONCE);
    check(key, b"IFZS widget", &nc, ns, proof)?;
    c.write_all(&tag(key, b"IFZS saver", &nc, ns))?;
    Ok(())
}

// the widget's half
fn shake_server(c: &mut Conn, key: &[u8; KEY_LEN]) -> Result<(), Error> {
    let mut first = [0u8; HELLO_LEN + NONCE];
    recv(c, &mut first, SHAKE)?;
    let h: [u8; HELLO_LEN] = first[..HELLO_LEN].try_into().unwrap();
    let nc = &first[HELLO_LEN..];
    let theirs = parse_hello(&h);
    if theirs == Err(Error::Magic) {
        return Err(Error::Magic);
    }
    let ns = nonce()?;
    // our hello either way, so a screensaver of another version knows to play its own run; a proof only for this user
    let proof = if theirs == Ok(user()) { tag(key, b"IFZS widget", nc, &ns) } else { [0; MAC] };
    c.write_all(&[hello(user()).as_slice(), &ns, &proof].concat())?;
    if theirs? != user() {
        return Err(Error::User);
    }
    let mut got = [0u8; MAC];
    recv(c, &mut got, SHAKE)?;
    check(key, b"IFZS saver", nc, &ns, &got)
}

/// Connects as a screensaver: the first address (or the pipe) where the other end proves it holds the key. Frame reads
/// on the connection time out after a second (TCP), so a reader can look up between frames.
pub fn connect(ep: &Endpoint) -> Result<Conn, Error> {
    let key = read_key(&ep.key)?;
    match &ep.at {
        #[cfg(not(windows))]
        Addr::Tcp(addrs) => {
            let mut last = Error::Io(io::ErrorKind::NotFound);
            for a in addrs {
                let tried = std::net::TcpStream::connect_timeout(a, Duration::from_millis(250)).map_err(Error::from).and_then(|mut c| {
                    set_timeouts(&c, SHAKE, SHAKE);
                    shake_client(&mut c, &key)?;
                    let _ = c.set_read_timeout(Some(Duration::from_secs(1)));
                    Ok(c)
                });
                match tried {
                    Ok(c) => return Ok(c),
                    Err(e) => last = e,
                }
            }
            Err(last)
        }
        #[cfg(windows)]
        Addr::Pipe(name) => {
            let mut c = std::fs::OpenOptions::new().read(true).write(true).open(name)?;
            // the pipe's server must run as this user, and then prove it holds the key
            if !pipe::server_is_us(&c) {
                return Err(Error::Auth);
            }
            shake_client(&mut c, &key)?;
            Ok(c)
        }
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
    fn bind(at: &Addr) -> io::Result<Listener> {
        match at {
            #[cfg(not(windows))]
            Addr::Tcp(addrs) => {
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
            Addr::Pipe(name) => {
                let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
                // the first instance claims the name now, so a squatter (or a second widget) fails here, not later
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
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE},
        Security::{
            Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
            GetLengthSid, GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        },
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
        System::{
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PeekNamedPipe, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
                PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
            },
            Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
        },
    };

    // runs `f` on the SID of the user `process` runs as
    unsafe fn with_token_user<T>(process: HANDLE, f: impl FnOnce(PSID) -> Option<T>) -> Option<T> {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut tok) == 0 {
            return None;
        }
        let mut buf = vec![0u64; 64];
        let mut len = 0u32;
        let ok = GetTokenInformation(tok, TokenUser, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut len);
        CloseHandle(tok);
        if ok == 0 {
            return None;
        }
        f((*(buf.as_ptr() as *const TOKEN_USER)).User.Sid)
    }

    // the SID's bytes: two SIDs are the same exactly when these are
    unsafe fn sid_bytes(sid: PSID) -> Option<Vec<u8>> {
        Some(std::slice::from_raw_parts(sid as *const u8, GetLengthSid(sid) as usize).to_vec())
    }

    /// This process's user as a SID string (S-1-5-21-…).
    pub fn user_sid() -> Option<String> {
        // SAFETY: the string is read up to its NUL, then LocalFree'd
        unsafe {
            with_token_user(GetCurrentProcess(), |sid| {
                let mut s: *mut u16 = std::ptr::null_mut();
                if ConvertSidToStringSidW(sid, &mut s) == 0 {
                    return None;
                }
                let n = (0..).take_while(|&i| *s.add(i) != 0).count();
                let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
                LocalFree(s.cast());
                Some(out)
            })
        }
    }

    /// Whether the process serving the pipe `f` runs as this user.
    pub fn server_is_us(f: &std::fs::File) -> bool {
        // SAFETY: the process handle is closed; the SIDs are copied out while their buffers live
        unsafe {
            let mut pid = 0u32;
            if GetNamedPipeServerProcessId(f.as_raw_handle() as HANDLE, &mut pid) == 0 {
                return false;
            }
            let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if p.is_null() {
                return false;
            }
            let theirs = with_token_user(p, |s| sid_bytes(s));
            CloseHandle(p);
            let ours = with_token_user(GetCurrentProcess(), |s| sid_bytes(s));
            theirs.is_some() && theirs == ours
        }
    }

    /// A new instance of the pipe that only this user can open (`first`: failing if anyone has the name already), as a
    /// raw handle.
    pub fn create(name: &[u16], first: bool) -> std::io::Result<isize> {
        let sid = user_sid().ok_or_else(|| std::io::Error::other("no user SID"))?;
        // protected DACL: generic all for this user's SID, nothing for anyone else
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})").encode_utf16().chain([0]).collect();
        let open = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
        // SAFETY: `name` and `sddl` are NUL-terminated UTF-16; the descriptor lives until after CreateNamedPipeW
        unsafe {
            let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
            let h = CreateNamedPipeW(
                name.as_ptr(),
                open,
                PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                1 << 16,
                1 << 12,
                0,
                &sa,
            );
            let e = std::io::Error::last_os_error();
            LocalFree(sd);
            if h == INVALID_HANDLE_VALUE {
                return Err(e);
            }
            Ok(h as isize)
        }
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

    /// Bytes waiting to be read, without blocking.
    pub fn available(f: &std::fs::File) -> Result<usize, super::Error> {
        let mut n = 0u32;
        // SAFETY: `f` is an open pipe handle; only the count is written
        if unsafe { PeekNamedPipe(f.as_raw_handle() as HANDLE, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut n, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(n as usize)
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

    /// Serves `ep`, writing a new key to `ep.key`.
    pub fn start_on(ep: &Endpoint, changed: impl Fn() + Send + Sync + 'static) -> io::Result<Hub> {
        let mut l = Listener::bind(&ep.at)?;
        // a new key each launch, once the address is ours
        let key = new_key(&ep.key)?;
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
                    let _ = std::thread::Builder::new().name("saver-client".into()).spawn(move || serve(inner, c, key));
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

fn serve(inner: Arc<Inner>, mut c: Conn, key: [u8; KEY_LEN]) {
    #[cfg(not(windows))]
    set_timeouts(&c, SHAKE, SHAKE);
    let greet = shake_server(&mut c, &key);
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
