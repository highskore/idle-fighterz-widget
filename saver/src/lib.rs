//! The screensaver's frames: the running widget's, mirrored over saver-link while it sends them, or else a demo run of
//! its own on the Rust core. The macOS .saver (saver/macos) calls the C functions at the bottom; the Windows .scr
//! (scr.rs) uses `Saver` directly.

use planet_core::{bridge::Bridge, num, Game, DT};
use planet_render::{
    scene::{Scene, View},
    Framebuffer,
};
use saver_link::{Endpoint, Error};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The demo's canvas: the medium floating widget's, in game pixels.
pub const DEMO_W: usize = 400;
pub const DEMO_H: usize = 356;
/// A live frame older than this, and the demo takes over.
const LIVE_FOR: Duration = Duration::from_millis(1500);
/// Not polled for this long (the screensaver stopped, or its host lost track of it), and the connection closes, so the
/// widget stops drawing for it.
const UNWATCHED: Duration = Duration::from_secs(2);
const RETRY: Duration = Duration::from_secs(2);

struct Shared {
    /// the newest frame from the widget, until a poll takes it
    latest: Mutex<Option<saver_link::Frame>>,
    /// when the newest frame came
    got: Mutex<Option<Instant>>,
    /// ms since `epoch` of the last poll, plus one
    polled: AtomicU64,
    stop: AtomicBool,
    epoch: Instant,
}

impl Shared {
    fn watched(&self) -> bool {
        let at = self.polled.load(Ordering::Relaxed);
        at != 0 && self.epoch.elapsed().as_millis() as u64 + 1 - at < UNWATCHED.as_millis() as u64
    }
}

/// What to show now.
pub struct Shot<'a> {
    /// RGBA8, straight alpha, rows top to bottom
    pub rgba: &'a [u8],
    pub w: usize,
    pub h: usize,
    pub power: &'a str,
    /// the widget's run, not the demo
    pub live: bool,
    /// a new frame since the last poll
    pub fresh: bool,
}

pub struct Saver {
    shared: Arc<Shared>,
    demo: Option<Demo>,
    last: Instant,
    rgba: Vec<u8>,
    w: usize,
    h: usize,
    power: String,
    live: bool,
}

impl Saver {
    /// Looks for the widget at this user's endpoint.
    pub fn new() -> Saver {
        Self::with(Some(saver_link::endpoint()))
    }

    /// `None`: demo only.
    pub fn with(ep: Option<Endpoint>) -> Saver {
        let shared = Arc::new(Shared {
            latest: Mutex::new(None),
            got: Mutex::new(None),
            polled: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            epoch: Instant::now(),
        });
        if let Some(ep) = ep {
            let s = shared.clone();
            let _ = std::thread::Builder::new().name("saver-link".into()).spawn(move || follow(&s, &ep));
        }
        Saver { shared, demo: None, last: Instant::now(), rgba: Vec::new(), w: 0, h: 0, power: String::new(), live: false }
    }

    /// Call once a display frame (at most 30 times a second): the newest live frame, or the demo stepped to now.
    pub fn poll(&mut self) -> Shot<'_> {
        let now = Instant::now();
        let elapsed = (now - self.last).as_secs_f64();
        self.last = now;
        self.shared.polled.store(self.shared.epoch.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
        let mut fresh = false;
        let live = self.shared.got.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|at| at.elapsed() < LIVE_FOR);
        if live {
            if let Some(f) = self.shared.latest.lock().unwrap_or_else(|e| e.into_inner()).take() {
                self.rgba = f.rgba;
                (self.w, self.h, self.power) = (f.w, f.h, f.power);
                self.live = true;
                fresh = true;
            }
        }
        if !live || !self.live {
            let demo = self.demo.get_or_insert_with(|| Demo::new(seed()));
            if demo.step(elapsed, now_ms()) || self.live || self.w == 0 {
                self.rgba.clear();
                self.rgba.extend_from_slice(demo.fb.rgba());
                (self.w, self.h, self.power) = (DEMO_W, DEMO_H, short(demo.b.game.hud.pl));
                fresh = true;
            }
            self.live = false;
        }
        Shot { rgba: &self.rgba, w: self.w, h: self.h, power: &self.power, live: self.live, fresh }
    }
}

impl Default for Saver {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

// the connection's thread: connects while the screensaver polls, keeps the newest frame, and lets go (so the widget
// stops drawing for it) once the screensaver stops polling or is dropped
fn follow(s: &Shared, ep: &Endpoint) {
    while !s.stop.load(Ordering::Relaxed) {
        if !s.watched() {
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        let Ok(mut c) = saver_link::connect(ep) else {
            std::thread::sleep(RETRY);
            continue;
        };
        while !s.stop.load(Ordering::Relaxed) && s.watched() {
            match saver_link::read_frame(&mut c) {
                Ok(f) => {
                    *s.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
                    *s.got.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                }
                Err(Error::Idle) => {}
                Err(_) => {
                    // the widget went, or sent something we won't show: the demo plays until a retry finds it again
                    drop(c);
                    std::thread::sleep(RETRY);
                    break;
                }
            }
        }
    }
}

/// A fresh run of the game, drawn as the medium floating widget.
pub struct Demo {
    pub b: Bridge,
    scene: Scene,
    pub fb: Framebuffer,
    view: View,
    bank: f64,
}

impl Demo {
    pub fn new(seed: u32) -> Demo {
        let (w, h) = (DEMO_W as f64, DEMO_H as f64);
        let mut b = Bridge::new(seed, w, h, 1.0);
        b.game.set_view(w, h, true);
        b.start("", now_ms());
        Demo { b, scene: Scene::new(), fb: Framebuffer::new(DEMO_W, DEMO_H), view: View { scale: 1.0, bare: true, clouds: None, cloud_amt: 2.0 }, bank: DT }
    }

    /// Runs the fixed steps `elapsed` seconds cover (at most 8), drawing the last; true if it drew.
    pub fn step(&mut self, elapsed: f64, now_ms: f64) -> bool {
        self.b.clock(now_ms);
        self.bank = (self.bank + elapsed).min(8.0 * DT);
        let n = (self.bank / DT).floor() as u32;
        self.bank -= n as f64 * DT;
        let (scene, fb, view) = (&mut self.scene, &mut self.fb, &self.view);
        for i in 1..=n {
            if i < n {
                self.b.step_one_drawn(&mut |_, _| {});
            } else {
                self.b.step_one_drawn(&mut |g: &mut Game, dt| {
                    scene.draw(fb, g, view, dt);
                    planet_render::px::rim(fb);
                });
            }
        }
        n > 0
    }
}

/// widget.js `short`, as the widget's menu-bar power level: 1.2K, 3.4M, 5.6B, then 7.8e12.
pub fn short(n: f64) -> String {
    const SUF: [&str; 4] = ["", "K", "M", "B"];
    let n = n.floor();
    if !n.is_finite() || n < 1000.0 {
        return num::js_string(n);
    }
    let mut e = (n.log10() / 3.0).floor() as i32;
    let mut m = n / 10f64.powf((e * 3) as f64);
    if num::to_fixed(m, 1).parse::<f64>().unwrap_or(0.0) >= 1000.0 {
        e += 1;
        m /= 1000.0;
    }
    if (e as usize) < SUF.len() {
        num::to_fixed(m, 1) + SUF[e as usize]
    } else {
        num::to_exponential(n, 1).replace('+', "")
    }
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

fn seed() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() ^ d.as_secs() as u32).unwrap_or(1).max(1)
}

// ── the C interface for the macOS .saver. A panic fails the call (no frame), never the screensaver host

#[repr(C)]
pub struct IfzShot {
    pub rgba: *const u8,
    pub w: u32,
    pub h: u32,
    /// NUL-terminated UTF-8
    pub power: [u8; saver_link::MAX_POWER + 1],
    pub live: bool,
    pub fresh: bool,
}

#[no_mangle]
pub extern "C" fn ifz_saver_new() -> *mut Saver {
    std::panic::catch_unwind(|| Box::into_raw(Box::new(Saver::new()))).unwrap_or(std::ptr::null_mut())
}

/// Fills `out`; false if there's nothing to show. `out.rgba` stays valid until the next call with `s`.
///
/// # Safety
/// `s` from `ifz_saver_new` and not yet freed, `out` writable; one thread at a time per `s`.
#[no_mangle]
pub unsafe extern "C" fn ifz_saver_poll(s: *mut Saver, out: *mut IfzShot) -> bool {
    if s.is_null() || out.is_null() {
        return false;
    }
    let (s, out) = (&mut *s, &mut *out);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let shot = s.poll();
        if shot.w == 0 {
            return false;
        }
        out.rgba = shot.rgba.as_ptr();
        (out.w, out.h, out.live, out.fresh) = (shot.w as u32, shot.h as u32, shot.live, shot.fresh);
        out.power = [0; saver_link::MAX_POWER + 1];
        out.power[..shot.power.len()].copy_from_slice(shot.power.as_bytes());
        true
    }))
    .unwrap_or(false)
}

/// # Safety
/// `s` from `ifz_saver_new` (or null), not used again.
#[no_mangle]
pub unsafe extern "C" fn ifz_saver_free(s: *mut Saver) {
    if !s.is_null() {
        drop(Box::from_raw(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saver_link::Hub;

    fn until(what: &str, mut f: impl FnMut() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(8), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    static N: AtomicU64 = AtomicU64::new(0);

    fn test_endpoint() -> Endpoint {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let key = std::env::temp_dir().join(format!("ifz-saver-test-{}-{n}", std::process::id())).join("key");
        #[cfg(windows)]
        let at = saver_link::Addr::Pipe(format!(r"\\.\pipe\idle-fighterz-saver-libtest-{}-{n}", std::process::id()));
        #[cfg(not(windows))]
        let at = saver_link::Addr::Tcp(vec![std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap()]);
        Endpoint { at, key }
    }

    #[test]
    fn with_no_widget_it_plays_its_own_run() {
        let mut s = Saver::with(Some(test_endpoint()));
        let shot = s.poll();
        assert!(!shot.live);
        assert_eq!((shot.w, shot.h, shot.rgba.len()), (DEMO_W, DEMO_H, DEMO_W * DEMO_H * 4));
        assert!(shot.rgba.chunks(4).any(|p| p[3] != 0), "the demo drew nothing");
        assert_eq!(shot.power, "0");
    }

    #[test]
    fn it_mirrors_the_widget_then_falls_back_when_it_goes() {
        let ep = test_endpoint();
        let hub = Hub::start_on(&ep, || {}).unwrap();
        let mut s = Saver::with(Some(ep));
        let rgba = vec![7u8; 20 * 10 * 4];
        until("a live frame", || {
            hub.publish(&rgba, 20, 10, "1.5M");
            let shot = s.poll();
            shot.live && shot.w == 20
        });
        let shot = s.poll();
        assert_eq!((shot.w, shot.h, shot.power, shot.rgba), (20, 10, "1.5M", &rgba[..]));
        // the widget stops sending (quit, or hung): the demo takes over
        until("the demo", || !s.poll().live);
        assert_eq!(s.poll().w, DEMO_W);
    }

    #[test]
    fn a_stopped_screensaver_lets_go_of_the_widget() {
        let ep = test_endpoint();
        let hub = Hub::start_on(&ep, || {}).unwrap();
        let mut s = Saver::with(Some(ep));
        until("attached", || {
            s.poll();
            hub.attached()
        });
        // no more polls (stopAnimation never came): the connection closes within UNWATCHED and a frame
        until("detached by silence", || {
            hub.publish(&[0; 4], 1, 1, "");
            !hub.attached()
        });
        until("attached again", || {
            s.poll();
            hub.publish(&[0; 4], 1, 1, "");
            hub.attached()
        });
        drop(s);
        until("detached by drop", || {
            hub.publish(&[0; 4], 1, 1, "");
            !hub.attached()
        });
    }

    #[test]
    fn a_widget_that_cant_prove_itself_means_the_demo() {
        #[cfg(not(windows))]
        {
            use std::io::{Read, Write};
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let ep = Endpoint { at: saver_link::Addr::Tcp(vec![l.local_addr().unwrap()]), key: test_endpoint().key };
            saver_link::new_key(&ep.key).unwrap();
            std::thread::spawn(move || {
                for c in l.incoming() {
                    let Ok(mut c) = c else { continue };
                    let mut h = [0u8; saver_link::HELLO_LEN + 16];
                    let _ = c.read_exact(&mut h);
                    // our hello, a nonce and a made-up proof, then a frame
                    let _ = c.write_all(&saver_link::hello(saver_link::user()));
                    let _ = c.write_all(&[3; 48]);
                    let _ = c.write_all(&saver_link::encode_frame(&[255; 4], 1, 1, "fake").unwrap());
                }
            });
            let mut s = Saver::with(Some(ep));
            for _ in 0..20 {
                assert!(!s.poll().live);
                std::thread::sleep(Duration::from_millis(30));
            }
        }
    }

    #[test]
    fn short_matches_the_menu_bar() {
        assert_eq!(short(999.0), "999");
        assert_eq!(short(1234.0), "1.2K");
        assert_eq!(short(999_950.0), "1.0M");
        assert_eq!(short(5.6e9), "5.6B");
        assert_eq!(short(7.8e12), "7.8e12");
    }

    #[test]
    fn the_c_calls_survive_nulls() {
        unsafe {
            assert!(!ifz_saver_poll(std::ptr::null_mut(), std::ptr::null_mut()));
            ifz_saver_free(std::ptr::null_mut());
        }
    }
}
