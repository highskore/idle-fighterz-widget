use super::*;
use std::sync::atomic::AtomicUsize;

fn msg_payload(m: &[u8]) -> &[u8] {
    assert_eq!(m[0], KIND_FRAME);
    let len = u32::from_le_bytes([m[1], m[2], m[3], m[4]]) as usize;
    assert_eq!(m.len(), 5 + len);
    &m[5..]
}

fn noise(n: usize, seed: &mut u32) -> Vec<u8> {
    (0..n)
        .map(|_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            *seed as u8
        })
        .collect()
}

fn frame_payload(w: u16, h: u16, enc: u8, power: &[u8], data: &[u8]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&w.to_le_bytes());
    p.extend_from_slice(&h.to_le_bytes());
    p.push(enc);
    p.push(power.len() as u8);
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(power);
    p.extend_from_slice(data);
    p
}

#[test]
fn hello_round_trips_and_checks_magic_and_version() {
    assert_eq!(parse_hello(&hello(501)), Ok(501));
    let mut h = hello(501);
    h[0] = b'X';
    assert_eq!(parse_hello(&h), Err(Error::Magic));
    let mut h = hello(501);
    h[4] = 1;
    assert_eq!(parse_hello(&h), Err(Error::Version(1)));
}

#[test]
fn a_flat_frame_goes_as_runs_and_comes_back_whole() {
    let (w, h) = (400, 356);
    let mut rgba = vec![0u8; w * h * 4];
    rgba[4 * (w * 100 + 200)..4 * (w * 100 + 260)].fill(200);
    let sky = Sky { world: 8, night: 200 };
    let m = encode_frame(&rgba, w, h, "1.2M", sky).unwrap();
    assert_eq!(msg_payload(&m)[4], ENC_RLE);
    assert!(m.len() < 200, "{} bytes", m.len());
    assert_eq!(decode_frame(msg_payload(&m)), Ok(Frame { w, h, power: "1.2M".into(), sky, rgba }));
}

#[test]
fn a_noisy_frame_goes_raw() {
    let mut seed = 7;
    let rgba = noise(64 * 48 * 4, &mut seed);
    let m = encode_frame(&rgba, 64, 48, "", Sky::default()).unwrap();
    assert_eq!(msg_payload(&m)[4], ENC_RAW);
    assert_eq!(decode_frame(msg_payload(&m)).unwrap().rgba, rgba);
}

#[test]
fn runs_longer_than_a_u16_split() {
    let (w, h) = (1024, 100);
    let rgba = vec![9u8; w * h * 4];
    let m = encode_frame(&rgba, w, h, "", Sky::default()).unwrap();
    assert_eq!(decode_frame(msg_payload(&m)).unwrap().rgba, rgba);
}

#[test]
fn the_power_level_is_cut_on_a_char_boundary() {
    let long = "é".repeat(40);
    let f = decode_frame(msg_payload(&encode_frame(&[0; 4], 1, 1, &long, Sky::default()).unwrap())).unwrap();
    assert_eq!(f.power, "é".repeat(16));
}

#[test]
fn encode_refuses_bad_sizes() {
    assert!(encode_frame(&[], 0, 1, "", Sky::default()).is_none());
    assert!(encode_frame(&vec![0; (MAX_SIDE + 1) * 4], MAX_SIDE + 1, 1, "", Sky::default()).is_none());
    assert!(encode_frame(&[0; 8], 1, 1, "", Sky::default()).is_none());
}

#[test]
fn decode_refuses_what_doesnt_add_up() {
    let run = |k: u16| [k.to_le_bytes().as_slice(), &[1, 2, 3, 4]].concat();
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (vec![1, 0, 1, 0, 0], "short"),
        (frame_payload(0, 1, ENC_RAW, b"", &[]), "zero width"),
        (frame_payload(MAX_SIDE as u16 + 1, 1, ENC_RAW, b"", &vec![0; (MAX_SIDE + 1) * 4]), "too wide"),
        (frame_payload(1, 1, ENC_RAW, &[b'9'; MAX_POWER + 1], &[0; 4]), "power too long"),
        (frame_payload(1, 1, ENC_RAW, &[0xff, 0xfe], &[0; 4]), "power not utf-8"),
        (frame_payload(2, 2, ENC_RAW, b"", &[0; 12]), "raw short"),
        (frame_payload(2, 2, ENC_RAW, b"", &[0; 20]), "raw long"),
        (frame_payload(2, 2, ENC_RLE, b"", &run(0)), "a run of zero"),
        (frame_payload(2, 2, ENC_RLE, b"", &run(5)), "runs past the end"),
        (frame_payload(2, 2, ENC_RLE, b"", &run(3)), "runs short"),
        (frame_payload(2, 2, ENC_RLE, b"", &[run(4).as_slice(), &[1]].concat()), "a broken run"),
        (frame_payload(2, 2, ENC_RLE, b"", &run(1).repeat(5)), "too many runs"),
        (frame_payload(2, 2, 7, b"", &[0; 16]), "unknown encoding"),
    ];
    for (p, what) in cases {
        assert!(matches!(decode_frame(&p), Err(Error::Bad(_))), "{what}");
    }
}

// a reader that hands out `len` bytes of a header then endless zeros, counting what it was asked for
struct Endless {
    head: Vec<u8>,
    at: usize,
    read: Arc<AtomicUsize>,
}

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(1 << 16);
        for b in &mut buf[..n] {
            *b = self.head.get(self.at).copied().unwrap_or(0);
            self.at += 1;
        }
        self.read.fetch_add(n, Ordering::Relaxed);
        Ok(n)
    }
}

#[test]
fn a_huge_length_is_refused_before_reading_or_allocating() {
    let read = Arc::new(AtomicUsize::new(0));
    let len = (MAX_PAYLOAD as u32 + 1).to_le_bytes();
    let mut r = Endless { head: vec![KIND_FRAME, len[0], len[1], len[2], len[3]], at: 0, read: read.clone() };
    assert_eq!(read_message(&mut r), Err(Error::TooBig(MAX_PAYLOAD + 1)));
    assert!(read.load(Ordering::Relaxed) <= 1 << 16);
    let mut r = Endless { head: vec![KIND_FRAME, 0xff, 0xff, 0xff, 0xff], at: 0, read };
    assert_eq!(read_message(&mut r), Err(Error::TooBig(u32::MAX as usize)));
}

#[test]
fn a_cut_off_message_is_an_error() {
    let m = encode_frame(&[5; 16], 2, 2, "7", Sky::default()).unwrap();
    for cut in 0..m.len() {
        assert!(read_frame(&mut &m[..cut]).is_err(), "cut at {cut}");
    }
    assert!(read_frame(&mut &m[..]).is_ok());
}

#[test]
fn other_kinds_are_skipped() {
    let mut s = vec![9u8, 3, 0, 0, 0, 1, 2, 3];
    s.extend(encode_frame(&[5; 4], 1, 1, "x", Sky::default()).unwrap());
    assert_eq!(read_frame(&mut &s[..]).unwrap().power, "x");
}

#[test]
fn garbage_never_panics() {
    let mut seed = 0x1234_5678;
    let good = encode_frame(&[1, 2, 3, 4, 1, 2, 3, 4, 9, 9, 9, 9, 0, 0, 0, 0], 2, 2, "42", Sky::default()).unwrap();
    for i in 0..20_000 {
        let mut b = if i % 2 == 0 { noise(1 + i % 64, &mut seed) } else { good.clone() };
        if i % 2 == 1 {
            let k = noise(2, &mut seed);
            let at = k[0] as usize % b.len();
            b[at] = k[1];
        }
        let _ = read_frame(&mut &b[..]);
        let _ = decode_frame(&b);
    }
}

// ── over the real transport

static N: AtomicUsize = AtomicUsize::new(0);

fn temp_key() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ifz-link-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))).join("key")
}

fn test_endpoint() -> Endpoint {
    #[cfg(windows)]
    let at = Addr::Pipe(format!(r"\\.\pipe\idle-fighterz-saver-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    #[cfg(not(windows))]
    let at = Addr::Tcp(vec![std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap()]);
    Endpoint { at, key: temp_key() }
}

fn raw(ep: &Endpoint) -> Conn {
    match &ep.at {
        #[cfg(not(windows))]
        Addr::Tcp(a) => std::net::TcpStream::connect(a[0]).unwrap(),
        #[cfg(windows)]
        Addr::Pipe(n) => std::fs::OpenOptions::new().read(true).write(true).open(n).unwrap(),
    }
}

fn until(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_screensaver_attaches_gets_frames_and_detaches() {
    let ep = test_endpoint();
    let changes = Arc::new(AtomicUsize::new(0));
    let c2 = changes.clone();
    let hub = Hub::start_on(&ep, move || {
        c2.fetch_add(1, Ordering::Relaxed);
    })
    .unwrap();
    assert!(!hub.attached());
    let mut c = connect(&ep).unwrap();
    until("attach", || hub.attached());
    assert_eq!(changes.load(Ordering::Relaxed), 1);
    let rgba: Vec<u8> = (0..40 * 30 * 4).map(|i| (i / 64) as u8).collect();
    hub.publish(&rgba, 40, 30, "9.9K", Sky::default());
    let f = c.read_frame().unwrap();
    assert_eq!((f.w, f.h, f.power.as_str()), (40, 30, "9.9K"));
    assert_eq!(f.rgba, rgba);
    drop(c);
    // the hub hears of it on its next write
    until("detach", || {
        hub.publish(&rgba, 40, 30, "", Sky::default());
        !hub.attached()
    });
    until("the change", || changes.load(Ordering::Relaxed) == 2);
}

#[cfg(unix)]
#[test]
fn the_key_is_the_users_alone() {
    use std::os::unix::fs::PermissionsExt;
    let p = temp_key();
    let k = new_key(&p).unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(p.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(read_key(&p), Ok(k));
    assert_ne!(new_key(&p).unwrap(), k, "a new key each launch");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(read_key(&p), Err(Error::Auth), "readable by the group");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&p, [0u8; KEY_LEN + 1]).unwrap();
    assert_eq!(read_key(&p), Err(Error::Auth), "the wrong size");
    std::fs::remove_file(&p).unwrap();
    std::os::unix::fs::symlink("/etc/hosts", &p).unwrap();
    assert_eq!(read_key(&p), Err(Error::Auth), "a link");
    assert_eq!(read_key(&temp_key()), Err(Error::Auth), "none");
}

#[test]
fn a_screensaver_without_the_key_gets_nothing() {
    let ep = test_endpoint();
    let hub = Hub::start_on(&ep, || {}).unwrap();
    // another key: it can't check the widget's proof, nor make its own
    let other = Endpoint { at: ep.at.clone(), key: temp_key() };
    new_key(&other.key).unwrap();
    assert_eq!(connect(&other).err(), Some(Error::Auth));
    // a stranger that goes through the motions with a made-up proof
    let mut c = raw(&ep);
    c.write_all(&[hello(user()).as_slice(), &[7; NONCE]].concat()).unwrap();
    let mut back = [0u8; HELLO_LEN + NONCE + MAC];
    c.read_exact(&mut back).unwrap();
    c.write_all(&[9; MAC]).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(!hub.attached());
    hub.publish(&[1; 4], 1, 1, "secret", Sky::default());
    let mut b = [0u8; 1];
    assert!(!matches!(c.read(&mut b), Ok(1)), "no frame for it");
}

#[test]
fn another_version_or_user_is_turned_away() {
    let ep = test_endpoint();
    let hub = Hub::start_on(&ep, || {}).unwrap();
    for (version, who) in [(9u8, user()), (VERSION as u8, user().wrapping_add(1))] {
        let mut c = raw(&ep);
        let mut h = hello(who);
        h[4] = version;
        c.write_all(&[h.as_slice(), &[1; NONCE]].concat()).unwrap();
        let mut back = [0u8; HELLO_LEN + NONCE + MAC];
        c.read_exact(&mut back).unwrap();
        assert_eq!(parse_hello(&back[..HELLO_LEN].try_into().unwrap()), Ok(user()), "our hello, so it knows");
        assert_eq!(&back[HELLO_LEN + NONCE..], &[0; MAC], "and no proof");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!hub.attached());
    }
}

#[test]
fn a_silent_screensaver_is_dropped_after_the_handshake_timeout() {
    let ep = test_endpoint();
    let _hub = Hub::start_on(&ep, || {}).unwrap();
    let mut c = raw(&ep);
    let t = Instant::now();
    let mut b = [0u8; 1];
    // the widget closes it: end of file, or an error
    assert!(!matches!(c.read(&mut b), Ok(1)));
    assert!(t.elapsed() >= SHAKE - Duration::from_millis(200) && t.elapsed() < SHAKE * 3, "{:?}", t.elapsed());
}

#[cfg(not(windows))]
#[test]
fn a_server_that_isnt_ours_is_refused() {
    use std::net::TcpListener;
    let serve = |reply: Vec<u8>| {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let ep = Endpoint { at: Addr::Tcp(vec![l.local_addr().unwrap()]), key: temp_key() };
        new_key(&ep.key).unwrap();
        std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut h = [0u8; HELLO_LEN + NONCE];
            let _ = c.read_exact(&mut h);
            let _ = c.write_all(&reply);
            std::thread::sleep(Duration::from_secs(3));
        });
        ep
    };
    // a widget from before frames carried the sky
    let mut v1 = hello(user()).to_vec();
    v1[4] = 1;
    assert_eq!(connect(&serve(v1)).err(), Some(Error::Version(1)));
    assert_eq!(connect(&serve(b"HTTP/1.1 400 Bad Request\r\n\r\n".repeat(3))).err(), Some(Error::Magic));
    // a squatter on the port: our hello, but no proof
    let squat = [hello(user()).as_slice(), &[0; NONCE + MAC]].concat();
    assert_eq!(connect(&serve(squat)).err(), Some(Error::Auth));
    // one that never answers
    let t = Instant::now();
    assert!(matches!(connect(&serve(Vec::new())), Err(Error::Io(_))));
    assert!(t.elapsed() < SHAKE * 2);
}

#[cfg(not(windows))]
#[test]
fn a_genuine_server_sending_an_oversized_frame_is_an_error() {
    let ep = test_endpoint();
    let key = new_key(&ep.key).unwrap();
    let Addr::Tcp(a) = &ep.at;
    let l = std::net::TcpListener::bind(a[0]).unwrap();
    std::thread::spawn(move || {
        let (mut c, _) = l.accept().unwrap();
        shake_server(&mut c, &key).unwrap();
        let _ = c.write_all(&[KIND_FRAME, 0xff, 0xff, 0xff, 0x7f]);
        std::thread::sleep(Duration::from_secs(1));
    });
    let mut c = connect(&ep).unwrap();
    assert!(matches!(c.read_frame(), Err(Error::TooBig(_))));
}

#[test]
fn no_more_than_max_clients_are_served() {
    let ep = test_endpoint();
    let hub = Hub::start_on(&ep, || {}).unwrap();
    let _held: Vec<Session> = (0..MAX_CLIENTS).map(|_| connect(&ep).unwrap()).collect();
    until("all attached", || hub.inner.slots.lock().unwrap().len() == MAX_CLIENTS);
    assert!(connect(&ep).is_err());
}

// a widget with the key that misbehaves after a genuine handshake: `send` gets the stream and the session key
#[cfg(not(windows))]
fn rogue(port_seen: Option<u16>, send: impl FnOnce(&mut Conn, [u8; KEY_LEN]) + Send + 'static) -> Endpoint {
    let ep = test_endpoint();
    let key = new_key(&ep.key).unwrap();
    let Addr::Tcp(a) = &ep.at;
    let l = std::net::TcpListener::bind(a[0]).unwrap();
    std::thread::spawn(move || {
        let (mut c, _) = l.accept().unwrap();
        let port = port_seen.map_or_else(|| dialled(&c, true), |p| p.to_le_bytes());
        if let Ok(sk) = shake_server_on(&mut c, &key, port) {
            send(&mut c, sk);
        }
        std::thread::sleep(Duration::from_secs(1));
    });
    ep
}

#[cfg(not(windows))]
fn tagged(sk: &[u8; KEY_LEN], seq: u64, m: &[u8]) -> Vec<u8> {
    use hmac::Mac;
    let t: [u8; MAC] = frame_tag(sk, seq, m[0], &m[5..]).finalize().into_bytes().into();
    [m, &t].concat()
}

#[cfg(not(windows))]
#[test]
fn frames_are_tagged_in_order() {
    let a = encode_frame(&[1; 16], 2, 2, "a", Sky::default()).unwrap();
    let b = encode_frame(&[2; 16], 2, 2, "b", Sky::default()).unwrap();
    let mut s = connect(&rogue(None, move |c, sk| {
        c.write_all(&tagged(&sk, 0, &a)).unwrap();
        c.write_all(&tagged(&sk, 1, &b)).unwrap();
    }))
    .unwrap();
    assert_eq!(s.read_frame().unwrap().power, "a");
    assert_eq!(s.read_frame().unwrap().power, "b");
}

#[cfg(not(windows))]
type Rogue = Box<dyn Fn(&[u8; KEY_LEN]) -> Vec<u8> + Send>;

#[cfg(not(windows))]
#[test]
fn a_tampered_replayed_reordered_or_untagged_frame_is_refused() {
    let m = encode_frame(&[1; 16], 2, 2, "a", Sky::default()).unwrap();
    let cases: Vec<(&str, Rogue)> = vec![
        ("tampered", {
            let m = m.clone();
            Box::new(move |sk| {
                let mut t = tagged(sk, 0, &m);
                t[12] ^= 1;
                t
            })
        }),
        ("replayed", {
            let m = m.clone();
            Box::new(move |sk| [tagged(sk, 0, &m), tagged(sk, 0, &m)].concat())
        }),
        ("reordered", {
            let m = m.clone();
            Box::new(move |sk| tagged(sk, 1, &m))
        }),
        ("another session's key", {
            let m = m.clone();
            Box::new(move |_| tagged(&[7; KEY_LEN], 0, &m))
        }),
        ("untagged", {
            let m = m.clone();
            Box::new(move |_| [m.as_slice(), &[0; MAC]].concat())
        }),
    ];
    for (what, bytes) in cases {
        let mut s = connect(&rogue(None, move |c, sk| c.write_all(&bytes(&sk)).unwrap())).unwrap();
        let refused = if what == "replayed" { s.read_frame().is_ok() && s.read_frame() == Err(Error::Auth) } else { s.read_frame() == Err(Error::Auth) };
        assert!(refused, "{what}");
    }
}

#[cfg(not(windows))]
#[test]
fn a_proof_for_another_port_is_refused() {
    // as a relay on one port passing on the widget's proof for its own
    let ep = rogue(Some(1), |_, _| {});
    assert_eq!(connect(&ep).err(), Some(Error::Auth));
}
