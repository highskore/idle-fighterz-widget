//! The screensaver's frames: the running widget's, mirrored over saver-link while it sends them, or else a demo run of
//! its own on the Rust core, put over the site's sky and cloud sea (sky.rs) at a whole scale that makes the planet about
//! 60% of the screen's height. The macOS .saver (saver/macos) calls the C functions at the bottom; the Windows .scr
//! (scr.rs) uses `Saver` directly.

pub mod sky;

use planet_core::{big::Big, bridge::Bridge, num, Game, DT};
use planet_render::{
    scene::{Scene, View},
    Framebuffer,
};
use saver_link::{Endpoint, Error, Sky};
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
/// The planet's height, as a share of the screen's, at most.
const PLANET: f64 = 0.6;
/// Where the planet's centre sits, as a share of the screen's height from the top.
const PLANET_Y: f64 = 0.42;
/// The biggest backdrop drawn, in game pixels a side (a very wide or very small view).
const MAX_BACK: f64 = 4096.0;

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

/// What to show now: the whole screen, in game pixels each `scale` device pixels across, from the top left.
pub struct Shot<'a> {
    /// RGBA8, opaque, rows top to bottom
    pub rgba: &'a [u8],
    pub w: usize,
    pub h: usize,
    pub scale: f64,
    pub power: &'a str,
    /// the middle of the power level's line, in device pixels from the top, and its size (0: no room for it)
    pub power_y: f64,
    pub power_px: f64,
    /// 0xRRGGBB: the word "power" and the number, dark by day and light by night
    pub muted: u32,
    pub ink: u32,
    /// the widget's run, not the demo
    pub live: bool,
    /// a new picture since the last poll
    pub fresh: bool,
}

pub struct Saver {
    shared: Arc<Shared>,
    demo: Option<Demo>,
    last: Instant,
    /// the newest frame, the widget's or the demo's: RGBA8, straight alpha
    rgba: Vec<u8>,
    w: usize,
    h: usize,
    power: String,
    sky: Sky,
    live: bool,
    back: Back,
}

/// The frame over the sky and the sea, at the screen's size.
struct Back {
    fb: Framebuffer,
    scale: f64,
    /// the device size it was drawn for
    dev: (f64, f64),
    pal: Option<(Sky, [u32; 4])>,
    power_y: f64,
    power_px: f64,
    night: f64,
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
        let back = Back { fb: Framebuffer::new(1, 1), scale: 1.0, dev: (0.0, 0.0), pal: None, power_y: 0.0, power_px: 0.0, night: 0.0 };
        Saver { shared, demo: None, last: Instant::now(), rgba: Vec::new(), w: 0, h: 0, power: String::new(), sky: Sky::default(), live: false, back }
    }

    /// Call once a display frame (at most 30 times a second) with the screen's size in device pixels: the newest live
    /// frame, or the demo stepped to now, over the sky. Nothing to show while `w` is 0.
    pub fn poll(&mut self, dev_w: f64, dev_h: f64) -> Shot<'_> {
        let dev = (dev_w.clamp(1.0, 16384.0), dev_h.clamp(1.0, 16384.0));
        let mut fresh = self.take();
        if self.w > 0 && (fresh || dev != self.back.dev) {
            self.compose(dev);
            fresh = true;
        }
        let b = &self.back;
        let k = (b.night >= 0.5) as usize;
        Shot {
            rgba: if self.w > 0 { b.fb.rgba() } else { &[] },
            w: if self.w > 0 { b.fb.w } else { 0 },
            h: b.fb.h,
            scale: b.scale,
            power: &self.power,
            power_y: b.power_y,
            power_px: b.power_px,
            muted: sky::MUTED[k],
            ink: sky::INK[k],
            live: self.live,
            fresh,
        }
    }

    // the newest live frame, or the demo stepped to now; true if there's a new one
    fn take(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = (now - self.last).as_secs_f64();
        self.last = now;
        self.shared.polled.store(self.shared.epoch.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
        let mut fresh = false;
        let live = self.shared.got.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|at| at.elapsed() < LIVE_FOR);
        if live {
            if let Some(f) = self.shared.latest.lock().unwrap_or_else(|e| e.into_inner()).take() {
                self.rgba = f.rgba;
                (self.w, self.h, self.power, self.sky) = (f.w, f.h, f.power, f.sky);
                self.live = true;
                fresh = true;
            }
        }
        if !live || !self.live {
            let demo = self.demo.get_or_insert_with(|| Demo::new(seed()));
            if demo.step(elapsed, now_ms()) || self.live || self.w == 0 {
                self.rgba.clear();
                self.rgba.extend_from_slice(demo.fb.rgba());
                (self.w, self.h, self.power, self.sky) = (DEMO_W, DEMO_H, short(demo.b.game.hud.pl), demo.sky());
                fresh = true;
            }
            self.live = false;
        }
        fresh
    }

    // the frame over the sky at the biggest whole scale (in device pixels) that keeps the planet within PLANET of the
    // screen's height, or shrunk to fit a small preview; the sea scrolls by the screensaver's own clock
    fn compose(&mut self, dev: (f64, f64)) {
        let b = &mut self.back;
        let r = planet_r(self.w, self.h);
        let fit = dev.1 * PLANET / (2.0 * r);
        b.scale = if fit >= 1.0 { fit.floor() } else { fit.max(dev.0.max(dev.1) / MAX_BACK) };
        b.dev = dev;
        let (w, h) = ((dev.0 / b.scale).ceil() as usize, (dev.1 / b.scale).ceil() as usize);
        if (w, h) != (b.fb.w, b.fb.h) {
            b.fb = Framebuffer::new(w, h);
        }
        b.night = self.sky.night as f64 / 255.0;
        let world = self.sky.world as u32;
        let pal = match b.pal {
            Some((s, pal)) if s == self.sky => pal,
            _ => {
                let pal = sky::sea_pal(world, b.night);
                b.pal = Some((self.sky, pal));
                pal
            }
        };
        let t = self.shared.epoch.elapsed().as_secs_f64();
        sky::fill(&mut b.fb, sky::sky(b.night));
        sky::draw_sea(&mut b.fb, &pal, world, t, false);
        let cy = (h as f64 * PLANET_Y).round();
        let (x, y) = ((w as i64 - self.w as i64) / 2, cy as i64 - self.h as i64 / 2);
        sky::blend(&mut b.fb, &self.rgba, self.w, self.h, x, y);
        sky::draw_sea(&mut b.fb, &pal, world, t, true);
        // between the planet (with the trees and the runner standing on it) and the clouds, no taller than the gap
        let below = cy + r * 1.3;
        let gap = (sky::sea_top(h as f64) - below) * b.scale;
        b.power_px = (dev.1 * 0.028).clamp(12.0, 56.0).min(gap * 0.8);
        if b.power_px < 9.0 {
            b.power_px = 0.0;
        }
        b.power_y = below * b.scale + gap / 2.0;
    }
}

/// The planet's radius in a frame of the widget's layout (planet-core's `layout` for the widget).
fn planet_r(w: usize, h: usize) -> f64 {
    (w.min(h) as f64 * 0.2).round().clamp(56.0, 70.0)
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
            match c.read_frame() {
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

    /// The demo's world and night.
    pub fn sky(&self) -> Sky {
        let g = &self.b.game;
        Sky { world: g.s.world.min(255) as u8, night: (g.tl.night_k.clamp(0.0, 1.0) * 255.0).round() as u8 }
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

/// widget.js `short`, as the widget's menu-bar power level: 1.2K, 3.4M, 5.6B, then 7.8e12; past 1e300, page.html's
/// `sci` (4.5e1,204).
pub fn short(n: impl Into<Big>) -> String {
    const SUF: [&str; 4] = ["", "K", "M", "B"];
    let n = n.into();
    let Some(n) = n.as_plain() else {
        let (m, x) = num::sci(n, 1);
        return m + &x;
    };
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
    pub scale: f64,
    /// NUL-terminated UTF-8
    pub power: [u8; saver_link::MAX_POWER + 1],
    pub power_y: f64,
    pub power_px: f64,
    pub muted: u32,
    pub ink: u32,
    pub live: bool,
    pub fresh: bool,
}

#[no_mangle]
pub extern "C" fn ifz_saver_new() -> *mut Saver {
    std::panic::catch_unwind(|| Box::into_raw(Box::new(Saver::new()))).unwrap_or(std::ptr::null_mut())
}

/// Fills `out` for a screen of `w`×`h` device pixels; false if there's nothing to show. `out.rgba` stays valid until
/// the next call with `s`.
///
/// # Safety
/// `s` from `ifz_saver_new` and not yet freed, `out` writable; one thread at a time per `s`.
#[no_mangle]
pub unsafe extern "C" fn ifz_saver_poll(s: *mut Saver, w: f64, h: f64, out: *mut IfzShot) -> bool {
    if s.is_null() || out.is_null() {
        return false;
    }
    let (s, out) = (&mut *s, &mut *out);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let shot = s.poll(w, h);
        if shot.w == 0 {
            return false;
        }
        out.rgba = shot.rgba.as_ptr();
        (out.w, out.h, out.scale, out.live, out.fresh) = (shot.w as u32, shot.h as u32, shot.scale, shot.live, shot.fresh);
        (out.power_y, out.power_px, out.muted, out.ink) = (shot.power_y, shot.power_px, shot.muted, shot.ink);
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

    // the composed pixel at (x, y) of the shot, as RGBA
    fn at(shot: &Shot, x: usize, y: usize) -> [u8; 4] {
        let k = (y * shot.w + x) * 4;
        shot.rgba[k..k + 4].try_into().unwrap()
    }

    #[test]
    fn with_no_widget_it_plays_its_own_run_over_the_sky() {
        let mut s = Saver::with(Some(test_endpoint()));
        let shot = s.poll(2560.0, 1600.0);
        assert!(!shot.live);
        assert_eq!(shot.power, "0");
        // the planet (radius 70 in the demo's frame) at a whole scale, at most 60% of the height
        assert_eq!(shot.scale, 6.0);
        assert!(shot.w as f64 * shot.scale >= 2560.0 && shot.h as f64 * shot.scale >= 1600.0, "covers the screen");
        assert_eq!(shot.rgba.len(), shot.w * shot.h * 4);
        assert!(shot.rgba.chunks(4).all(|p| p[3] == 255), "opaque");
        assert_eq!(at(&shot, 0, 0), [0xfc, 0xfc, 0xfb, 255], "the site's sky by day");
        assert_ne!(at(&shot, 0, shot.h - 1), [0xfc, 0xfc, 0xfb, 255], "the cloud sea along the bottom");
        assert_ne!(at(&shot, shot.w / 2, (shot.h as f64 * PLANET_Y) as usize), [0xfc, 0xfc, 0xfb, 255], "the planet in the middle");
        assert!(shot.power_px >= 12.0 && shot.power_y > 1600.0 * 0.6 && shot.power_y < 1600.0, "{} at {}", shot.power_px, shot.power_y);
        assert_eq!(shot.ink, sky::INK[0]);
        // 1080p: a scale of 4
        assert_eq!(s.poll(1920.0, 1080.0).scale, 4.0);
        // a small preview: shrunk to fit, whole
        let shot = s.poll(300.0, 190.0);
        assert!(shot.scale < 1.0 && shot.w as f64 * shot.scale >= 300.0);
    }

    #[test]
    fn it_mirrors_the_widget_then_falls_back_when_it_goes() {
        let ep = test_endpoint();
        let hub = Hub::start_on(&ep, || {}).unwrap();
        let mut s = Saver::with(Some(ep));
        let rgba = [200u8, 10, 10, 255].repeat(20 * 10);
        let night = Sky { world: 1, night: 255 };
        until("a live frame", || {
            hub.publish(&rgba, 20, 10, "1.5M", night);
            s.poll(640.0, 400.0).live && s.w == 20
        });
        assert_eq!((s.w, s.h, s.power.as_str(), &s.rgba[..], s.sky), (20, 10, "1.5M", &rgba[..], night));
        let shot = s.poll(640.0, 400.0);
        // the frame, centred, over the night sky; light text
        assert_eq!(at(&shot, shot.w / 2, (shot.h as f64 * PLANET_Y) as usize), [200, 10, 10, 255]);
        assert_eq!(at(&shot, 0, 0), [0x07, 0x07, 0x0d, 255]);
        assert_eq!(shot.ink, sky::INK[1]);
        // the widget stops sending (quit, or hung): the demo takes over
        until("the demo", || !s.poll(640.0, 400.0).live);
        assert_eq!(s.w, DEMO_W);
    }

    #[test]
    fn a_stopped_screensaver_lets_go_of_the_widget() {
        let ep = test_endpoint();
        let hub = Hub::start_on(&ep, || {}).unwrap();
        let mut s = Saver::with(Some(ep));
        until("attached", || {
            s.poll(64.0, 40.0);
            hub.attached()
        });
        // no more polls (stopAnimation never came): the connection closes within UNWATCHED and a frame
        until("detached by silence", || {
            hub.publish(&[0; 4], 1, 1, "", Sky::default());
            !hub.attached()
        });
        until("attached again", || {
            s.poll(64.0, 40.0);
            hub.publish(&[0; 4], 1, 1, "", Sky::default());
            hub.attached()
        });
        drop(s);
        until("detached by drop", || {
            hub.publish(&[0; 4], 1, 1, "", Sky::default());
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
                    let _ = c.write_all(&saver_link::encode_frame(&[255; 4], 1, 1, "fake", Sky::default()).unwrap());
                }
            });
            let mut s = Saver::with(Some(ep));
            for _ in 0..20 {
                assert!(!s.poll(64.0, 40.0).live);
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
        assert_eq!(short(Big::new(4.5, 1204)), "4.5e1,204");
    }

    #[test]
    fn the_c_calls_survive_nulls() {
        unsafe {
            assert!(!ifz_saver_poll(std::ptr::null_mut(), 1.0, 1.0, std::ptr::null_mut()));
            ifz_saver_free(std::ptr::null_mut());
        }
    }
}
