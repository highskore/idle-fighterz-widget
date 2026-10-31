// The game on the Rust core (cargo feature `rust-core`): planet-core's `Bridge` runs natively behind one Tauri command
// per call (game/rust/HUD.md), a tick thread steps it, planet-render draws it, and each frame's message goes to the HUD
// page as a `planet` event. The frame itself goes to a `Surface` (the macOS layer under the page) or, failing that, the
// page fetches it and puts it on its canvas. The save lives in a file. Hidden, the page gets nothing: the game catches
// up once a second undrawn and the app shows its power level and notifications itself. See NATIVE.md.
use planet_core::{
    big::Big,
    bridge::Bridge,
    data::{Hero, ALL_FORMS},
    hud::Cue,
    num::{js_string, sci, to_exponential, to_fixed},
    sprite, Game, DT,
};
use planet_render::{
    scene::{Scene, View},
    Framebuffer,
};
use std::{
    fs,
    io::Read,
    panic::AssertUnwindSafe,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{ipc::Response, AppHandle, Emitter, Manager, State};

// a frame with nothing in it, which isn't worth an event
const EMPTY: &str = r#"{"events":[],"cues":[]}"#;
// the game's buffer until the page says its size
const SIZE: (f64, f64, f64) = (133.0, 133.0, 3.0);
// planet-core's JSON parser has no depth limit yet, so a deeper or bigger save is never parsed (a real one: a few KB, 2 deep)
const MAX_SAVE: usize = 1 << 20;
const MAX_DEPTH: usize = 32;
// `Bridge::step` runs at most 8 steps a call and drops the rest: a catch-up goes in pieces under that
const PIECE: f64 = 7.0 * DT;
// a hidden catch-up plays at most this many seconds (a sleeping machine doesn't count), as game.js's hidden ticker does
const MAX_CATCH_UP: f64 = 60.0;
const HIDDEN_TICK: Duration = Duration::from_secs(1);
const HIDDEN_SAVE: Duration = Duration::from_secs(30);

pub struct Core {
    bridge: Mutex<Bridge>,
    file: PathBuf,
    // true once the app is exiting: held across every write, so the exit save is the last one and none interleave
    stopped: Mutex<bool>,
    /// the rate the page's widget.js picked (awake, the frame-rate setting), 0 until it says
    fps: Mutex<f64>,
    /// locked after `bridge` whenever both are
    paint: Mutex<Paint>,
    surface: OnceLock<Box<dyn Surface>>,
    /// the screensavers mirroring the run (saver/README.md)
    saver: OnceLock<saver_link::Hub>,
    /// photo mode: the shown game holds still, and plays the time on resume
    paused: Mutex<bool>,
}

/// What a hidden catch-up leaves for the app: the power level, and the notifications the page would have sent.
pub struct Caught {
    pub pl: Big,
    pub notes: Vec<(&'static str, String)>,
}

/// Shows the drawn frames in place of the page's canvas: the macOS layer under the HUD (feature native-renderer).
pub trait Surface: Send + Sync {
    /// Where the page's canvas is (`[x, y, w, h]` in CSS pixels), the page's opacity and whether the widget floats.
    /// True if this shows the frames from now on; false if the page should.
    fn place(&self, rect: [f64; 4], opacity: f64, bare: bool) -> bool;
    /// A frame: RGBA8, straight alpha, rows top to bottom. False if it can't show frames any more: the page takes over.
    fn show(&self, rgba: &[u8], w: u32, h: u32) -> bool;
    /// Called once on the tick thread, before it first sleeps.
    fn tick_thread(&self) {}
    /// The tick's sleep between frames.
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// planet-render drawing the game as the page's canvas would: its size and scale, floating or window mode, the clouds.
pub struct Painter {
    scene: Scene,
    fb: Framebuffer,
    view: View,
}

impl Default for Painter {
    fn default() -> Self {
        Painter { scene: Scene::new(), fb: Framebuffer::new(1, 1), view: View { scale: 1.0, bare: true, clouds: None, cloud_amt: 2.0 } }
    }
}

impl Painter {
    /// `st.W`×`st.H` at `st.scale`; the game's `resize` takes the same.
    pub fn resize(&mut self, w: f64, h: f64, scale: f64) {
        let (w, h) = (w.clamp(1.0, 4096.0) as usize, h.clamp(1.0, 4096.0) as usize);
        if (w, h) != (self.fb.w, self.fb.h) {
            self.fb = Framebuffer::new(w, h);
        }
        self.view.scale = scale;
    }

    /// `BARE` (floating), `st.clouds` ('a'..'e', or none) and `st.cloudAmt`.
    pub fn set_view(&mut self, bare: bool, clouds: Option<char>, amount: f64) {
        self.view.bare = bare;
        self.view.clouds = clouds.filter(|c| ('a'..='e').contains(c));
        self.view.cloud_amt = amount.clamp(1.0, 5.0);
    }

    /// One frame, after the step's logic: game.js's draw, then widget.js's rim when floating.
    pub fn draw(&mut self, g: &mut Game, dt: f64) {
        self.scene.draw(&mut self.fb, g, &self.view, dt);
        if self.view.bare {
            planet_render::px::rim(&mut self.fb);
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.fb.w as u32, self.fb.h as u32)
    }

    pub fn rgba(&self) -> &[u8] {
        self.fb.rgba()
    }

    pub fn scale(&self) -> f64 {
        self.view.scale
    }

    /// One bit a pixel, set where anything is drawn (alpha > 0), rows top to bottom: widget.js's empty-sky test.
    pub fn mask(&self) -> Vec<u8> {
        let mut m = vec![0u8; self.fb.buf.len().div_ceil(8)];
        for (k, p) in self.fb.buf.iter().enumerate() {
            if p >> 24 != 0 {
                m[k / 8] |= 1 << (k % 8);
            }
        }
        m
    }
}

/// The drawing's state between ticks.
#[derive(Default)]
struct Paint {
    painter: Painter,
    /// the page has said how it shows the game (`planet_view`): until then nothing is drawn
    on: bool,
    /// the surface shows the frames, not the page
    native: bool,
    /// time not yet stepped: drawn, the game steps here rather than in `Bridge::step`, so only a tick's last step draws
    bank: f64,
    /// where the page's canvas is (`planet_view`), CSS pixels: the spectacles start from there
    canvas: [f64; 4],
    /// the last saga fight and full beam the spectacles were told of, and what they haven't taken yet
    seen_boss: u32,
    seen_beam: f64,
    moments: Vec<Moment>,
}

impl Paint {
    // `Bridge::step_drawn`, drawing only the last of the steps, as page.html draws only the last of a battery-saver
    // frame's steps; true if it drew
    fn step(&mut self, b: &mut Bridge, elapsed: f64, now_ms: f64) -> bool {
        b.clock(now_ms);
        self.bank += elapsed;
        let mut n = 0;
        while self.bank >= DT && n < 8 {
            self.bank -= DT;
            n += 1;
        }
        if n == 8 {
            self.bank = 0.0;
        }
        for i in 1..=n {
            if i < n {
                b.step_one_drawn(&mut |_, _| {});
            } else {
                b.step_one_drawn(&mut |g, dt| self.painter.draw(g, dt));
            }
        }
        n > 0
    }

    // a saga villain's fight starting (his arrival over) and a full beam fired, each once, for the spectacles
    fn spot(&mut self, g: &Game) {
        let (fw, fh) = self.painter.size();
        let [cx, cy, cw, ch] = self.canvas;
        if fw <= 1 || cw <= 0.0 || ch <= 0.0 {
            return;
        }
        let (kx, ky) = (cw / fw as f64, ch / fh as f64);
        if let Some(b) = g.tl.boss.as_ref().filter(|b| b.saga && !b.uni && !b.movie && b.intro.is_none() && g.t >= b.go && !b.dead && !b.fled) {
            if b.id != self.seen_boss {
                self.seen_boss = b.id;
                let name = if b.name == "past vegeta" { "vegeta" } else { b.name.as_str() };
                let sprite = planet_core::data::VILLAINS.iter().position(|v| v.0 == name).and_then(|i| sprite::baked().get(&format!("VILLAINS.{i}.spr")));
                if let Some(sprite) = sprite {
                    self.moments.push(Moment::Raid { id: b.id, sprite, x: cx + b.x * kx, y: cy + b.y * ky, scale: b.k * kx });
                }
            }
        }
        if let Some(b) = g.inp.beam.as_ref().filter(|b| b.full && b.t0 != self.seen_beam) {
            self.seen_beam = b.t0;
            let (x, y, dx, dy) = (cx + b.x * kx, cy + b.y * ky, b.dx * kx, b.dy * ky);
            self.moments.push(Moment::Beam { x, y, dx, dy, kame: g.kame(), ff: b.ff, kaio: b.kaio, ape: b.ape });
        }
    }
}

/// What the spectacles play (spectacle/), in the page's CSS pixels.
pub enum Moment {
    /// a saga villain's fight has started: boss `id`, standing at (`x`, `y`), each of his sprite's pixels drawn `scale`
    /// CSS pixels across
    Raid { id: u32, sprite: &'static sprite::Sprite, x: f64, y: f64, scale: f64 },
    /// a full-charge beam, from the hero's hand at (`x`, `y`) toward (`dx`, `dy`): a kamehameha (or else a galick gun),
    /// a final flash, under kaioken or as the great ape
    Beam { x: f64, y: f64, dx: f64, dy: f64, kame: bool, ff: bool, kaio: bool, ape: bool },
}

impl Core {
    /// A new game seeded with `seed`, carrying over what the save in `file` keeps (its daily bonus date).
    pub fn new(seed: u32, file: PathBuf) -> Self {
        let mut bridge = Bridge::new(seed, SIZE.0, SIZE.1, SIZE.2);
        bridge.game.set_view(SIZE.0, SIZE.1, true); // the widget's layout: the planet at the site's size, centred
        if let Some(saved) = read_save(&file).filter(|s| loadable(s)) {
            bridge.load(&saved);
        }
        Core { bridge: Mutex::new(bridge), file, stopped: Mutex::new(false), fps: Mutex::new(0.0), paint: Mutex::default(), surface: OnceLock::new(), saver: OnceLock::new(), paused: Mutex::new(false) }
    }

    /// Frames go to the screensavers `hub` serves while any is watching.
    #[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
    pub fn set_saver(&self, hub: saver_link::Hub) {
        let _ = self.saver.set(hub);
    }

    /// Whether a screensaver is watching: then the game is drawn and plays even while hidden.
    fn watched(&self) -> bool {
        self.saver.get().is_some_and(|h| h.attached())
    }

    // the frame just drawn, to the screensavers
    fn publish(&self, b: &Bridge, p: &Paint) {
        if let Some(h) = self.saver.get().filter(|h| h.attached()) {
            let (w, ht) = p.painter.size();
            let sky = saver_link::Sky { world: b.game.s.world.min(255) as u8, night: (b.game.tl.night_k.clamp(0.0, 1.0) * 255.0).round() as u8 };
            h.publish(p.painter.rgba(), w as usize, ht as usize, &short(b.game.hud.pl), sky);
        }
    }

    /// Hidden with a screensaver watching: plays `elapsed` (a long gap undrawn, then one drawn step, as `tick` does at
    /// the game's 30 steps a second), sends the frame to the screensaver, and takes what `catch_up` takes.
    pub fn saver_tick(&self, mut elapsed: f64, now_ms: f64, kept: &mut Vec<Cue>, ascend: &mut bool) -> Option<Caught> {
        self.with(|b| {
            let mut p = self.paint();
            if !p.on && p.painter.size() == (1, 1) {
                // the page never said its size: the game's own
                p.painter.resize(SIZE.0, SIZE.1, SIZE.2);
            }
            // leaving a step and a half, so float error can't leave less than the one step to draw
            while elapsed > 2.0 * DT {
                let piece = (elapsed - 1.5 * DT).min(PIECE);
                b.step(piece, now_ms);
                elapsed -= piece;
            }
            if p.step(b, elapsed, now_ms) {
                self.publish(b, &p);
            }
            drop(p);
            drain(b, kept, ascend)
        })
        .ok()
    }

    /// The wait until the next step is due while drawing for a screensaver (`run`'s drawn wait).
    fn saver_wait(&self) -> Duration {
        let bank = self.paint().bank;
        Duration::from_secs_f64((DT - bank + DT / 2.0).clamp(0.0, 1.0))
    }

    /// Frames go to `s` while it takes them (see `Surface::place`).
    #[cfg_attr(not(all(target_os = "macos", feature = "native-renderer")), allow(dead_code))]
    pub fn set_surface(&self, s: Box<dyn Surface>) {
        let _ = self.surface.set(s);
    }

    /// Time banked towards the next step, while drawing (the tick paces itself by it).
    fn bank(&self) -> Option<f64> {
        let p = self.paint();
        p.on.then_some(p.bank)
    }

    fn paint(&self) -> std::sync::MutexGuard<'_, Paint> {
        self.paint.lock().unwrap_or_else(|e| e.into_inner())
    }

    // only in debug builds does a panic in the game fail just the one call: release builds have panic = "abort", so
    // there it ends the app and this catches nothing
    fn with<T>(&self, f: impl FnOnce(&mut Bridge) -> T) -> Result<T, String> {
        let mut b = self.bridge.lock().unwrap_or_else(|e| e.into_inner());
        std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut b))).map_err(|e| {
            e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_else(|| "the game panicked".into())
        })
    }

    /// Steps by `elapsed` real seconds, drawing the last step once the page has said how it shows the game; the frame's
    /// message, unless it carries nothing. A frame the page should put on its canvas adds `"px":true`.
    pub fn tick(&self, elapsed: f64, now_ms: f64) -> Option<String> {
        let (msg, page_px) = self
            .with(|b| {
                let mut p = self.paint();
                let mut page_px = false;
                if !p.on {
                    b.step(elapsed, now_ms);
                } else if p.step(b, elapsed, now_ms) {
                    p.spot(&b.game);
                    self.publish(b, &p);
                    match self.surface.get().filter(|_| p.native) {
                        Some(s) => {
                            let (w, h) = p.painter.size();
                            if !s.show(p.painter.rgba(), w, h) {
                                p.native = false;
                                page_px = true;
                            }
                        }
                        None => page_px = true,
                    }
                }
                (b.frame_json(), page_px)
            })
            .ok()?;
        if page_px {
            return Some(format!("{},\"px\":true}}", &msg[..msg.len() - 1]));
        }
        (msg != EMPTY).then_some(msg)
    }

    /// Hidden: runs every fixed step `elapsed` seconds cover, as stepping frame by frame would, and takes what the page
    /// would have got. Toasts and sounds go; notifications come back; the cues that leave something on the page stay in
    /// `kept` for when it shows. `ascend` is whether ascending was open, for its notification.
    pub fn catch_up(&self, mut elapsed: f64, now_ms: f64, kept: &mut Vec<Cue>, ascend: &mut bool) -> Option<Caught> {
        self.with(|b| {
            while elapsed > 0.0 {
                let piece = elapsed.min(PIECE);
                b.step(piece, now_ms);
                elapsed -= piece;
            }
            drain(b, kept, ascend)
        })
        .ok()
    }

    /// Shown again: the page gets the kept cues and the whole HUD with the next frame.
    pub fn shown(&self, kept: &mut Vec<Cue>) {
        let _ = self.with(|b| {
            let later = std::mem::replace(&mut b.game.cues, std::mem::take(kept));
            b.game.cues.extend(later);
            b.resend();
        });
    }

    /// Photo mode: while paused the shown game isn't stepped or drawn; on resume it plays the time it missed.
    pub fn pause(&self, on: bool) {
        *self.paused.lock().unwrap_or_else(|e| e.into_inner()) = on;
    }

    fn paused(&self) -> bool {
        *self.paused.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Steps all but the last of a long `elapsed` (a pause's) undrawn, so `tick` draws the rest: no time is lost, and
    /// what happened meanwhile reaches the page with the next frame, but its toasts go as a hidden catch-up's do (so the
    /// one saying where the photo went is seen). What's left for `tick`.
    fn fast_forward(&self, mut elapsed: f64, now_ms: f64) -> f64 {
        if elapsed <= 0.5 {
            return elapsed;
        }
        let _ = self.with(|b| {
            while elapsed > 2.0 * DT {
                let piece = (elapsed - 1.5 * DT).min(PIECE);
                b.step(piece, now_ms);
                elapsed -= piece;
            }
            b.game.events.clear();
        });
        elapsed
    }

    /// The last frame drawn, its size, and the scale it's shown at (pixels per point). None before the first.
    pub fn frame(&self) -> Option<(Vec<u8>, u32, u32, f64)> {
        let p = self.paint();
        let (w, h) = p.painter.size();
        (p.on && w > 1).then(|| (p.painter.rgba().to_vec(), w, h, p.painter.scale()))
    }

    /// What the shown game has done since last asked that the spectacles play.
    pub fn moments(&self) -> Vec<Moment> {
        std::mem::take(&mut self.paint().moments)
    }

    /// Whether boss `id`'s fight goes on (he hasn't been beaten or got away).
    pub fn raid_on(&self, id: u32) -> bool {
        self.with(|b| b.game.tl.boss.as_ref().is_some_and(|x| x.id == id && !x.dead && !x.fled)).unwrap_or(false)
    }

    /// While the villain is out on the desktop the planet is drawn without him (`Game::boss_away`).
    pub fn boss_away(&self, on: bool) {
        let _ = self.with(|b| b.game.boss_away = on);
    }

    /// Reduced motion, as the page said (`planet_view`'s `calm`).
    pub fn calm(&self) -> bool {
        self.with(|b| b.game.calm).unwrap_or(true)
    }

    /// The game as the away recap sees it.
    pub fn snap(&self) -> Option<crate::recap::Snap> {
        self.with(|b| crate::recap::Snap {
            pl: b.game.hud.pl,
            saga: b.game.s.saga,
            form: b.game.hud.form_name.to_string(),
            dragon: b.game.hud.shen,
            ascend: can_ascend(&b.game),
        })
        .ok()
    }

    /// The game at a glance, for the menu-bar popover and the settings preview: the HUD as `ui.update` gets it, with
    /// how many upgrades are affordable and the world's name.
    pub fn glance(&self) -> Option<String> {
        self.with(|b| {
            let ready = b.game.shop().iter().filter(|r| r.can).count();
            let world = planet_core::hud::WORLD_FULL.get(b.game.hud.world as usize).copied().unwrap_or("");
            let hud = b.game.hud.to_json();
            format!("{{\"hud\":{hud},\"ready\":{ready},\"world\":{}}}", serde_json::Value::from(world))
        })
        .ok()
    }

    pub fn can_ascend(&self) -> bool {
        self.with(|b| can_ascend(&b.game)).unwrap_or(false)
    }

    /// Saves, unless the app is exiting (then `stop` has saved for the last time).
    pub fn save(&self) {
        let stopped = self.stopped.lock().unwrap_or_else(|e| e.into_inner());
        if !*stopped {
            self.write();
        }
    }

    /// Stops the tick thread's saves and saves once more, on exit.
    pub fn stop(&self) {
        let mut stopped = self.stopped.lock().unwrap_or_else(|e| e.into_inner());
        if !*stopped {
            *stopped = true;
            self.write();
        }
    }

    fn stopped(&self) -> bool {
        *self.stopped.lock().unwrap_or_else(|e| e.into_inner())
    }

    // to a temp file renamed into place, so a crash or a full disk mid-write leaves the old save whole
    fn write(&self) {
        let Ok(text) = self.with(|b| b.game.save_json()) else { return };
        let _ = fs::create_dir_all(self.file.parent().unwrap());
        let tmp = self.file.with_extension("json.tmp");
        if fs::write(&tmp, text).is_err() || fs::rename(&tmp, &self.file).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

// the save, if it's a regular file (not a link) of at most MAX_SAVE bytes, read through a cap in case it grows meanwhile
fn read_save(file: &PathBuf) -> Option<String> {
    if !fs::symlink_metadata(file).is_ok_and(|m| m.is_file()) {
        return None;
    }
    let mut text = String::new();
    fs::File::open(file).ok()?.take(MAX_SAVE as u64 + 1).read_to_string(&mut text).ok()?;
    (text.len() <= MAX_SAVE).then_some(text)
}

// at most MAX_SAVE bytes and MAX_DEPTH levels of [ and {, outside strings
fn loadable(s: &str) -> bool {
    let (mut depth, mut text, mut escaped) = (0usize, false, false);
    for c in s.bytes() {
        match c {
            _ if escaped => escaped = false,
            b'\\' if text => escaped = true,
            b'"' => text = !text,
            b'[' | b'{' if !text => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return false;
                }
            }
            b']' | b'}' if !text => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    s.len() <= MAX_SAVE
}

// what a hidden step leaves: toasts and sounds go; notifications come back; the cues that leave something on the page stay
// in `kept` for when it shows
fn drain(b: &mut Bridge, kept: &mut Vec<Cue>, ascend: &mut bool) -> Caught {
    b.game.events.clear();
    let mut notes = Vec::new();
    for c in b.game.cues.drain(..) {
        match c {
            Cue::Notify(kind, arg) => notes.push((kind, arg)),
            Cue::WishDone | Cue::Achieved | Cue::Zeno | Cue::TreeGift(_) if !kept.contains(&c) => kept.push(c),
            _ => {}
        }
    }
    // widget.js's ui.shop: ascend notifies when it opens
    let can = can_ascend(&b.game);
    if can && !*ascend {
        notes.push(("ascend", String::new()));
    }
    *ascend = can;
    Caught { pl: b.game.hud.pl, notes }
}

fn can_ascend(g: &Game) -> bool {
    g.shop().iter().any(|r| r.key == "ascend" && r.can)
}

/// widget.js `short`: the power level beside the menu-bar icon, e.g. 1.2K, 3.4M, 5.6B, then 7.8e12; past 1e300,
/// page.html's `sci` (4.5e1,204)
pub fn short(n: impl Into<Big>) -> String {
    const SUF: [&str; 4] = ["", "K", "M", "B"];
    let n = n.into();
    let Some(n) = n.as_plain() else {
        let (m, x) = sci(n, 1);
        return m + &x;
    };
    let n = n.floor();
    if n < 1000.0 {
        return js_string(n);
    }
    let mut e = (n.log10() / 3.0).floor() as i32;
    let mut m = n / 10f64.powf((e * 3) as f64);
    if to_fixed(m, 1).parse::<f64>().unwrap() >= 1000.0 {
        e += 1;
        m /= 1000.0;
    }
    if (e as usize) < SUF.len() {
        to_fixed(m, 1) + SUF[e as usize]
    } else {
        to_exponential(n, 1).replace('+', "")
    }
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

/// the saga villain the `i`th raid brings (they come round again after the last)
pub fn villain(i: u32) -> &'static str {
    planet_core::data::VILLAINS[i as usize % planet_core::data::VILLAINS.len()].0
}

pub fn seed() -> u32 {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() ^ d.as_secs() as u32).unwrap_or(1);
    n.max(1)
}

/// What the tick needs from the app.
pub struct Hooks {
    /// the frame-rate setting, until the page says its rate
    pub idle_fps: fn(&AppHandle) -> f64,
    /// whether a hidden game keeps playing (a notification or the menu-bar power level is on); if not, it pauses
    pub plays_hidden: fn(&AppHandle) -> bool,
    /// a hidden catch-up's power level and notifications
    pub hidden: fn(&AppHandle, Caught),
    /// a raid or a full beam on the shown widget, for the spectacles
    pub moment: fn(&AppHandle, Moment),
}

/// Steps the game and sends the HUD what changed, at the rate the page picks (widget.js: the frame-rate setting,
/// doubled while awake). Hidden, it catches up once a second and the page gets nothing, or pauses as game.js does.
/// Saves every 3 s of play, every 30 s hidden.
pub fn run(app: AppHandle, hooks: Hooks) {
    std::thread::spawn(move || {
        let core = app.state::<Core>();
        let shown = app.state::<crate::Shown>();
        let (mut last, mut saved) = (Instant::now(), Instant::now());
        let mut kept = Vec::new();
        let surface = core.surface.get();
        if let Some(s) = surface {
            s.tick_thread();
        }
        loop {
            if core.stopped() {
                return;
            }
            if shown.get() && core.paused() {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            if shown.get() {
                let fps = Some(*core.fps.lock().unwrap()).filter(|f| *f > 0.0).unwrap_or_else(|| (hooks.idle_fps)(&app));
                // the game steps 30 times a second: a faster tick would find nothing new every other time. Drawn, the
                // tick wakes half a step after its last step is due, so a late wake still finds it and an early one
                // doesn't find the next: every tick draws
                let steps = (1.0 / (fps * DT)).round().max(1.0);
                let wait = core.bank().map_or(steps * DT, |bank| steps * DT - bank + DT / 2.0);
                let wait = Duration::from_secs_f64(wait.clamp(0.0, 1.0));
                match surface {
                    Some(s) => s.sleep(wait),
                    None => std::thread::sleep(wait),
                }
                let now = Instant::now();
                let elapsed = core.fast_forward((now - last).as_secs_f64(), now_ms());
                last = now;
                if let Some(msg) = core.tick(elapsed, now_ms()) {
                    let _ = app.emit_to("main", "planet", msg);
                }
                for m in core.moments() {
                    (hooks.moment)(&app, m);
                }
                if saved.elapsed() >= Duration::from_secs(3) && core.with(|b| b.game.playing).unwrap_or(false) {
                    saved = Instant::now();
                    core.save();
                }
                continue;
            }
            let mut ascend = core.can_ascend();
            let mut told = Instant::now();
            loop {
                if core.stopped() {
                    return;
                }
                // a screensaver watching: drawn, 30 steps a second, until it goes (its hub pokes `shown` either way)
                let watched = core.watched();
                let plays = watched || (hooks.plays_hidden)(&app);
                shown.nap(if watched { Some(core.saver_wait()) } else { plays.then_some(HIDDEN_TICK) });
                let now = Instant::now();
                let elapsed = (now - last).as_secs_f64().min(MAX_CATCH_UP);
                last = now;
                // paused, the time hidden isn't played (game.js: no steps while hidden)
                if plays {
                    let caught = if watched { core.saver_tick(elapsed, now_ms(), &mut kept, &mut ascend) } else { core.catch_up(elapsed, now_ms(), &mut kept, &mut ascend) };
                    // the menu bar and notifications as often as a hidden catch-up has them, or a notification sooner
                    if let Some(c) = caught.filter(|c| !watched || !c.notes.is_empty() || told.elapsed() >= HIDDEN_TICK) {
                        told = Instant::now();
                        if !shown.get() {
                            (hooks.hidden)(&app, c);
                        }
                    }
                    if saved.elapsed() >= HIDDEN_SAVE && core.with(|b| b.game.playing).unwrap_or(false) {
                        saved = Instant::now();
                        core.save();
                    }
                }
                if shown.get() {
                    break;
                }
            }
            core.shown(&mut kept);
        }
    });
}

// ── the calls (HUD.md's table), as `planet_<call>` with HUD.md's argument names. `new` is the app's (the game outlives
// the page), and `spots`/`probe` were the web's lockstep twin's. The app steps, frames and saves the game itself, so
// `load`, `step`, `step_one`, `frame`, `save` and `playing` aren't the page's to call

// a HUD page loaded again mounts again: the game carries on
// widget.js's frame rate for the shown widget
#[tauri::command]
pub fn planet_fps(core: State<'_, Core>, fps: f64) {
    *core.fps.lock().unwrap() = fps.clamp(0.0, 60.0);
}

#[tauri::command(rename_all = "snake_case")]
pub fn planet_start(core: State<'_, Core>, today: String, now_ms: f64) -> Result<(), String> {
    core.with(|b| if !b.game.playing { b.start(&today, now_ms) })
}

#[tauri::command]
pub fn planet_resend(core: State<'_, Core>) -> Result<(), String> {
    core.with(|b| b.resend())
}

#[tauri::command]
pub fn planet_resize(core: State<'_, Core>, w: f64, h: f64, scale: f64) -> Result<(), String> {
    core.with(|b| {
        b.game.resize(w, h, scale);
        core.paint().painter.resize(w, h, scale);
    })
}

/// How the page shows the game: floating (`bare`) or window mode, its clouds (`st.clouds`, '' for none) and their
/// `amount`, reduced motion, where its canvas is (CSS pixels) and its opacity. True if the app shows the frames itself
/// (the layer under the page); false if the page should fetch them (`planet_pixels`) when a message says `px`.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn planet_view(core: State<'_, Core>, bare: bool, clouds: String, amount: f64, calm: bool, x: f64, y: f64, w: f64, h: f64, opacity: f64) -> Result<bool, String> {
    core.with(|b| {
        b.set_calm(calm);
        let mut p = core.paint();
        p.painter.set_view(bare, clouds.chars().next(), amount);
        p.canvas = [x, y, w, h];
        p.native = core.surface.get().is_some_and(|s| s.place([x, y, w, h], opacity, bare));
        p.on = true;
        p.native
    })
}

/// The last frame drawn, RGBA8 (straight alpha, as ImageData holds it), `st.W`×`st.H`.
#[tauri::command]
pub fn planet_pixels(core: State<'_, Core>) -> Response {
    Response::new(core.paint().painter.rgba().to_vec())
}

/// Where the last frame has anything drawn, one bit a pixel (`Painter::mask`), for the page's empty-sky drag test.
#[tauri::command]
pub fn planet_mask(core: State<'_, Core>) -> Response {
    Response::new(core.paint().painter.mask())
}

/// A hero's idle sprite as `{w, h, col}` (`col`: 0xRRGGBB, or -1 for none): `id`'s, for the characters tab's
/// thumbnail (`thumb`), or with no `id` the one the share card shows (the hero's form, or fused).
#[tauri::command]
pub fn planet_sprite(core: State<'_, Core>, id: Option<String>) -> Result<String, String> {
    core.with(|b| {
        let g = &b.game;
        let spr = match &id {
            Some(id) => &sprite::hero(id).forms[0].idle,
            None => {
                let hero = sprite::hero(match g.s.hero {
                    Hero::Vegeta => "vegeta",
                    Hero::Goku => "goku",
                    Hero::Gohan => "gohan",
                    Hero::Trunks => "trunks",
                });
                let fi = ALL_FORMS.iter().position(|f| f.0 == g.forms[g.s.form].name).unwrap_or(0);
                if g.b.fuse_until > g.t {
                    &hero.fused.idle
                } else {
                    &hero.forms[fi].idle
                }
            }
        };
        serde_json::json!({ "w": spr.w, "h": spr.h, "col": spr.col }).to_string()
    })
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn planet_pointer_down(core: State<'_, Core>, x: f64, y: f64, bx: f64, by: f64, id: i32, button: i32, touch: bool) -> Result<(), String> {
    core.with(|b| b.game.pointer_down(x, y, bx, by, id, button, touch))
}

#[tauri::command]
pub fn planet_pointer_move(core: State<'_, Core>, x: f64, y: f64, bx: f64, by: f64, id: i32) -> Result<(), String> {
    core.with(|b| b.game.pointer_move(x, y, bx, by, id))
}

#[tauri::command]
pub fn planet_pointer_up(core: State<'_, Core>, bx: f64, by: f64, id: i32) -> Result<(), String> {
    core.with(|b| b.game.pointer_up(bx, by, id))
}

#[tauri::command]
pub fn planet_itx(core: State<'_, Core>) -> Result<(), String> {
    core.with(|b| b.game.itx_open())
}

#[tauri::command]
pub fn planet_itx_jump(core: State<'_, Core>, w: u32) -> Result<(), String> {
    core.with(|b| b.game.itx_jump(w))
}

#[tauri::command]
pub fn planet_pointer_cancel(core: State<'_, Core>) -> Result<(), String> {
    core.with(|b| b.game.pointer_cancel())
}

#[tauri::command]
pub fn planet_key(core: State<'_, Core>, k: String) -> Result<bool, String> {
    core.with(|b| b.game.key(&k))
}

#[tauri::command]
pub fn planet_train(core: State<'_, Core>) -> Result<(), String> {
    core.with(|b| b.game.train())
}

#[tauri::command]
pub fn planet_kaioken(core: State<'_, Core>) -> Result<(), String> {
    core.with(|b| b.game.kaioken())
}

#[tauri::command]
pub fn planet_buy(core: State<'_, Core>, key: String) -> Result<bool, String> {
    core.with(|b| b.game.buy(&key))
}

/// `n` levels at once, 0 for as many as you can afford (shift- and alt-click, or the ×10 / max toggle on touch).
#[tauri::command]
pub fn planet_buy_n(core: State<'_, Core>, key: String, n: u32) -> Result<bool, String> {
    core.with(|b| b.buy_n(&key, n))
}

/// The undo button on a big buy's toast.
#[tauri::command]
pub fn planet_undo(core: State<'_, Core>, id: u32) -> Result<bool, String> {
    core.with(|b| b.undo(id))
}

/// A shop row's hover card, as JSON.
#[tauri::command]
pub fn planet_shop_info(core: State<'_, Core>, key: String) -> Result<String, String> {
    core.with(|b| b.shop_info(&key))
}

/// The shop's prices buying `n` at a time (0: max), as JSON.
#[tauri::command]
pub fn planet_bulk(core: State<'_, Core>, n: u32) -> Result<String, String> {
    core.with(|b| b.bulk(n))
}

#[tauri::command]
pub fn planet_learn(core: State<'_, Core>, id: String) -> Result<(), String> {
    core.with(|b| b.game.learn(&id))
}

#[tauri::command]
pub fn planet_respec(core: State<'_, Core>) -> Result<(), String> {
    core.with(|b| b.game.respec())
}

#[tauri::command]
pub fn planet_learn_angel(core: State<'_, Core>, id: String) -> Result<(), String> {
    core.with(|b| b.game.learn_angel(&id))
}

/// `wear(kind, id)`: wears a look the milestones unlocked, or takes it off.
#[tauri::command]
pub fn planet_wear(core: State<'_, Core>, kind: String, id: String) -> Result<(), String> {
    core.with(|b| b.game.wear(&kind, &id))
}

#[tauri::command]
pub fn planet_auto_next(core: State<'_, Core>) -> Result<(), String> {
    core.with(|b| b.game.auto_next())
}

#[tauri::command]
pub fn planet_pick_wish(core: State<'_, Core>, key: String) -> Result<(), String> {
    core.with(|b| b.game.pick(&key))
}

#[tauri::command]
pub fn planet_pick_hero(core: State<'_, Core>, id: String) -> Result<bool, String> {
    core.with(|b| b.game.pick_hero(&id))
}

#[tauri::command]
pub fn planet_menu(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.game.menu_json())
}

#[tauri::command]
pub fn planet_angels(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.game.angels_json())
}

#[tauri::command]
pub fn planet_ascend_info(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.game.ascend_info_json())
}

#[tauri::command]
pub fn planet_god_info(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.game.god_info_json())
}

#[tauri::command]
pub fn planet_zeno_info(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.game.zeno_info_json())
}

/// Where the planet is drawn now: its centre and radius in buffer pixels (the camera the renderer and the hit tests use,
/// shake included), and the buffer's size, for the page to map onto its canvas (the first-run card's ring).
#[tauri::command]
pub fn planet_spot(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| {
        let c = &b.game.cam;
        format!("{{\"cx\":{},\"cy\":{},\"r\":{},\"w\":{},\"h\":{}}}", c.cx, c.cy + c.bob, c.r, c.w, c.h)
    })
}

#[tauri::command]
pub fn planet_card(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.game.card_json())
}

/// Every `planet_*` command, for `invoke_handler` (main.rs adds the app's own).
macro_rules! planet_commands {
    ($($more:path),* $(,)?) => {
        tauri::generate_handler![
            $($more,)*
            crate::planet::planet_start, crate::planet::planet_resend, crate::planet::planet_resize, crate::planet::planet_pointer_down, crate::planet::planet_pointer_move,
            crate::planet::planet_pointer_up, crate::planet::planet_pointer_cancel, crate::planet::planet_key, crate::planet::planet_train,
            crate::planet::planet_kaioken, crate::planet::planet_buy, crate::planet::planet_learn, crate::planet::planet_respec, crate::planet::planet_learn_angel,
            crate::planet::planet_auto_next, crate::planet::planet_wear, crate::planet::planet_pick_wish, crate::planet::planet_pick_hero, crate::planet::planet_menu,
            crate::planet::planet_angels, crate::planet::planet_ascend_info, crate::planet::planet_god_info, crate::planet::planet_zeno_info,
            crate::planet::planet_card, crate::planet::planet_fps, crate::planet::planet_view, crate::planet::planet_pixels,
            crate::planet::planet_mask, crate::planet::planet_sprite, crate::planet::planet_itx, crate::planet::planet_itx_jump,
            crate::planet::planet_spot, crate::planet::planet_buy_n, crate::planet::planet_undo, crate::planet::planet_shop_info,
            crate::planet::planet_bulk
        ]
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tauri::{
        ipc::{CallbackFn, InvokeBody},
        test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY},
        webview::InvokeRequest,
        WebviewWindow, WebviewWindowBuilder,
    };

    const SEED: u32 = 7;
    const SHOP: [&str; 6] = ["boots", "partner", "gravity", "scouter", "house", "kaio"];

    fn app(file: PathBuf) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>) {
        let app = mock_builder().manage(Core::new(SEED, file)).invoke_handler(planet_commands![]).build(mock_context(noop_assets())).unwrap();
        let w = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        (app, w)
    }

    fn call(w: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Value {
        let req = InvokeRequest {
            cmd: format!("planet_{cmd}"),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: w.url().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(w, req).unwrap_or_else(|e| panic!("{cmd}: {e}")).deserialize().unwrap()
    }

    fn file(name: &str) -> PathBuf {
        let f = std::env::temp_dir().join(format!("idle-fighterz-test-{}-{name}.json", std::process::id()));
        let _ = fs::remove_file(&f);
        f
    }

    // a minute of play driven the HUD's way (commands over IPC, frames from the tick) against the same calls on a Bridge
    #[test]
    fn commands_send_what_the_bridge_sends() {
        let (app, w) = app(file("same"));
        let core = app.state::<Core>();
        let mut b = Bridge::new(SEED, SIZE.0, SIZE.1, SIZE.2);
        b.game.set_view(SIZE.0, SIZE.1, true);
        let (today, t0) = ("Wed Oct 28 2026", 1.7e12);

        b.game.resize(200.0, 134.0, 3.0);
        call(&w, "resize", json!({ "w": 200.0, "h": 134.0, "scale": 3.0 }));
        b.start(today, t0);
        call(&w, "start", json!({ "today": today, "now_ms": t0 }));
        let mut frames = 0;
        for i in 0..1800 {
            let now = t0 + i as f64 * 33.0;
            let elapsed = if i % 7 == 3 { 0.05 } else { 1.0 / 30.0 };
            b.step(elapsed, now);
            let want = b.frame_json();
            assert_eq!(core.tick(elapsed, now), (want != EMPTY).then(|| want.clone()), "frame {i}");
            frames += (want != EMPTY) as u32;
            let msg: Value = serde_json::from_str(&want).unwrap();
            if let Some(cue) = msg["cues"].as_array().unwrap().iter().find(|c| c[0] == "wish") {
                let key = cue[1].as_array().unwrap().last().unwrap()["key"].as_str().unwrap().to_string();
                b.game.pick(&key);
                call(&w, "pick_wish", json!({ "key": key }));
            }

            if i % 5 == 0 {
                b.game.pointer_down(300.0, 200.0, 100.0, 66.0, 1, 0, false);
                b.game.pointer_up(100.0, 66.0, 1);
                call(&w, "pointer_down", json!({ "x": 300.0, "y": 200.0, "bx": 100.0, "by": 66.0, "id": 1, "button": 0, "touch": false }));
                call(&w, "pointer_up", json!({ "bx": 100.0, "by": 66.0, "id": 1 }));
            }
            if i % 200 == 100 {
                b.game.pointer_down(300.0, 200.0, 100.0, 66.0, 1, 0, false);
                call(&w, "pointer_down", json!({ "x": 300.0, "y": 200.0, "bx": 100.0, "by": 66.0, "id": 1, "button": 0, "touch": false }));
                for k in 1..8 {
                    let (x, y) = (300.0 + k as f64 * 9.0, 200.0 - k as f64 * 3.0);
                    b.game.pointer_move(x, y, x / 3.0, y / 3.0, 1);
                    call(&w, "pointer_move", json!({ "x": x, "y": y, "bx": x / 3.0, "by": y / 3.0, "id": 1 }));
                }
                b.game.pointer_cancel();
                call(&w, "pointer_cancel", json!({}));
            }
            if i % 45 == 7 {
                assert_eq!(call(&w, "key", json!({ "k": " " })), json!(b.game.key(" ")));
            }
            if i % 30 == 0 {
                for k in SHOP {
                    assert_eq!(call(&w, "buy", json!({ "key": k })), json!(b.game.buy(k)), "buy {k}");
                }
            }
            if i % 60 == 30 {
                b.game.train();
                call(&w, "train", json!({}));
                b.game.kaioken();
                call(&w, "kaioken", json!({}));
            }
            if i % 10 == 0 {
                assert_eq!(core.with(|c| c.frame_json()).unwrap(), b.frame_json(), "frame {i} on request");
            }
            if i % 300 == 0 {
                let menu = b.game.menu_json();
                assert_eq!(call(&w, "menu", json!({})), json!(menu));
                for n in serde_json::from_str::<Value>(&menu).unwrap()["tree"].as_array().unwrap() {
                    if n["can"] == json!(true) {
                        let id = n["id"].as_str().unwrap();
                        b.game.learn(id);
                        call(&w, "learn", json!({ "id": id }));
                    }
                }
                for (cmd, want) in [
                    ("angels", b.game.angels_json()),
                    ("ascend_info", b.game.ascend_info_json()),
                    ("god_info", b.game.god_info_json()),
                    ("zeno_info", b.game.zeno_info_json()),
                    ("card", b.game.card_json()),
                ] {
                    assert_eq!(call(&w, cmd, json!({})), json!(want), "{cmd} at {i}");
                }
                assert_eq!(core.with(|c| c.game.save_json()).unwrap(), b.game.save_json(), "save at {i}");
            }
        }
        assert!(frames > 100, "{frames} frames sent");
        assert!(core.with(|c| c.game.playing).unwrap());
        b.resend();
        call(&w, "resend", json!({}));
        assert_eq!(core.with(|c| c.frame_json()).unwrap(), b.frame_json());
        assert_eq!(core.with(|c| c.game.save_json()).unwrap(), b.game.save_json());
    }

    // the app steps, frames and saves the game: the page can't, nor load a save of its own
    #[test]
    fn the_page_cannot_drive_the_game_loop() {
        let (_app, w) = app(file("gone"));
        for cmd in ["load", "step", "step_one", "frame", "save", "playing"] {
            let req = InvokeRequest {
                cmd: format!("planet_{cmd}"),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: w.url().unwrap(),
                body: InvokeBody::Json(json!({ "text": "{}", "elapsed": 1e9, "now_ms": 0.0 })),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            };
            assert!(get_ipc_response(&w, req).is_err(), "planet_{cmd} is still callable");
        }
    }

    // half an hour of the game hidden, caught up once a second, every 5 s and in odd batches, against the same game
    // stepped frame by frame as the shown widget steps it: the same steps, the same game and the same notifications
    #[test]
    fn a_hidden_catch_up_plays_the_same_game() {
        let (today, t0) = ("Wed Oct 28 2026", 1.7e12);
        let new = || {
            let core = Core::new(SEED, file("unused"));
            core.with(|b| b.start(today, t0)).unwrap();
            core
        };
        let now = |c: &Core| t0 + c.with(|b| (b.t * 1000.0).floor()).unwrap();
        let notes_of = |msg: &str| -> Vec<(String, String)> {
            let v: Value = serde_json::from_str(msg).unwrap();
            v["cues"].as_array().unwrap().iter().filter(|c| c[0] == "notify").map(|c| (c[1].as_str().unwrap().into(), c[2].as_str().unwrap_or("").into())).collect() // a pod's has no argument
        };
        for batches in [vec![1.0], vec![5.0], vec![0.37, 2.9, 1.0, 9.5, 0.04]] {
            let hidden = new();
            let (mut kept, mut ascend, mut notes, mut played) = (Vec::new(), false, Vec::new(), 0.0);
            for k in 0.. {
                let dt = batches[k % batches.len()];
                if played + dt > 1800.0 {
                    break;
                }
                played += dt;
                let c = hidden.catch_up(dt, now(&hidden), &mut kept, &mut ascend).unwrap();
                notes.extend(c.notes.into_iter().filter(|n| n.0 != "ascend").map(|(k, a)| (k.to_string(), a)));
            }
            let t = hidden.with(|b| b.t).unwrap();
            assert!((t - played).abs() < DT, "{played} s played as {t} s");

            let shown = new();
            let mut want = Vec::new();
            while shown.with(|b| b.t).unwrap() < t {
                if let Some(msg) = shown.tick(DT, now(&shown)) {
                    want.extend(notes_of(&msg));
                }
            }
            assert_eq!(shown.with(|b| b.t).unwrap(), t, "batches {batches:?}");
            assert!(want.len() > 10, "{} notifications in half an hour", want.len());
            assert_eq!(notes, want, "batches {batches:?}");
            // achievements are stamped with the wall clock when the step ran, which batching moves: only which ones count
            let state = |c: &Core| {
                let mut save: Value = serde_json::from_str(&c.with(|b| b.game.save_json()).unwrap()).unwrap();
                for v in save["ach"].as_object_mut().unwrap().values_mut() {
                    *v = json!(true);
                }
                (save, c.with(|b| (b.game.hud.clone(), b.game.shop_json(), b.game.s.pl)).unwrap())
            };
            assert_eq!(state(&hidden), state(&shown), "batches {batches:?}");
        }
    }

    // hidden, with a screensaver connected, each tick draws and sends it the frame; with none, nothing is drawn
    #[test]
    fn a_screensaver_gets_the_hidden_game_drawn() {
        #[cfg(windows)]
        let at = saver_link::Addr::Pipe(format!(r"\\.\pipe\idle-fighterz-planet-test-{}", std::process::id()));
        #[cfg(not(windows))]
        let at = saver_link::Addr::Tcp(vec![std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap()]);
        let ep = saver_link::Endpoint { at, key: std::env::temp_dir().join(format!("idle-fighterz-test-{}-saver-key", std::process::id())).join("key") };
        let core = Core::new(SEED, file("saver"));
        core.set_saver(saver_link::Hub::start_on(&ep, || {}).unwrap());
        let (mut kept, mut ascend) = (Vec::new(), false);
        core.with(|b| b.start("Wed Oct 28 2026", 1.7e12)).unwrap();
        assert!(!core.watched());
        core.saver_tick(DT, 1.7e12, &mut kept, &mut ascend);
        assert_eq!(core.paint().painter.size(), (SIZE.0 as u32, SIZE.1 as u32), "drawn at the game's size: the page never said its own");
        let mut c = saver_link::connect(&ep).unwrap();
        let t = Instant::now();
        while !core.watched() {
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        // a long gap is played undrawn, then one step drawn and sent
        let t0 = core.with(|b| b.t).unwrap();
        core.saver_tick(2.0, 1.7e12 + 2000.0, &mut kept, &mut ascend);
        assert!((core.with(|b| b.t).unwrap() - t0 - 2.0).abs() < 2.0 * DT);
        let f = c.read_frame().unwrap();
        assert_eq!((f.w, f.h), (SIZE.0 as usize, SIZE.1 as usize));
        assert_eq!(f.rgba, core.paint().painter.rgba());
        assert!(f.rgba.chunks(4).any(|p| p[3] != 0), "the frame has the planet in it");
        assert_eq!(f.power, short(core.with(|b| b.game.hud.pl).unwrap()));
        assert_eq!(f.sky.world as u32, core.with(|b| b.game.s.world).unwrap());
        drop(c);
        let t = Instant::now();
        while core.watched() {
            assert!(t.elapsed() < Duration::from_secs(5), "the screensaver went, the hub should hear it");
            core.saver_tick(DT, 1.7e12, &mut kept, &mut ascend);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    // the first-run card rings the planet where planet_spot says it is: at every widget size (1× below 520 points, 2×
    // from there, as world.js sizes the buffer), the disc drawn has that centre and radius, to within 2 pixels
    #[test]
    fn the_spot_is_where_the_planet_is_drawn() {
        for size in [300.0, 340.0, 400.0, 452.0, 518.0, 520.0, 560.0, 640.0, 720.0f64] {
            let (app, w) = app(file(&format!("spot{size}")));
            let core = app.state::<Core>();
            let scale = if size >= 520.0 { 2.0 } else { 1.0 };
            let side = (size / scale).ceil();
            call(&w, "resize", json!({ "w": side, "h": side, "scale": scale }));
            let args = json!({ "bare": true, "clouds": "", "amount": 2.0, "calm": true, "x": 0.0, "y": 0.0, "w": size, "h": size, "opacity": 1.0 });
            call(&w, "view", args);
            call(&w, "start", json!({ "today": "Wed Oct 28 2026", "now_ms": 1.7e12 }));
            for i in 0..40 {
                core.tick(DT, 1.7e12 + i as f64 * 33.0);
            }
            let spot: Value = serde_json::from_str(call(&w, "spot", json!({})).as_str().unwrap()).unwrap();
            let (cx, cy, r) = (spot["cx"].as_f64().unwrap(), spot["cy"].as_f64().unwrap(), spot["r"].as_f64().unwrap());
            assert_eq!((spot["w"].as_f64(), spot["h"].as_f64()), (Some(side), Some(side)));
            let (rgba, fw, fh, _) = core.frame().unwrap();
            let solid = |x: f64, y: f64| {
                let (x, y) = (x.round() as i64, y.round() as i64);
                x >= 0 && y >= 0 && x < fw as i64 && y < fh as i64 && rgba[(y as usize * fw as usize + x as usize) * 4 + 3] > 0
            };
            // how far the drawing reaches from the centre in 32 directions: trees, huts and the hero only reach further,
            // so the nearest edge is the disc's
            let reach: Vec<f64> = (0..32)
                .map(|k| {
                    let a = k as f64 * std::f64::consts::PI / 16.0;
                    let mut d = 0.0;
                    while solid(cx + (d + 0.5) * a.cos(), cy + (d + 0.5) * a.sin()) && d < side {
                        d += 0.5;
                    }
                    d
                })
                .collect();
            // the disc's edge (its outline and the floating rim included) is where most directions stop
            let edge = |d: f64| d >= r - 2.0 && d <= r + 3.0;
            let near = reach.iter().cloned().fold(f64::MAX, f64::min);
            assert!(near >= r - 2.0, "{size} pt: the disc stops at {near}, inside r = {r}");
            let rims: Vec<f64> = reach.iter().cloned().filter(|d| edge(*d)).collect();
            let drawn = rims.iter().sum::<f64>() / rims.len() as f64;
            assert!(rims.len() >= 16 && (drawn - r).abs() <= 2.0, "{size} pt: the disc is {drawn} across {} directions, the spot says r = {r}", rims.len());
            // the centre: across each pair of opposite directions both at the disc's edge, the middle is the centre
            let mut pairs = 0;
            for k in 0..16 {
                let (a, b) = (reach[k], reach[k + 16]);
                if edge(a) && edge(b) {
                    pairs += 1;
                    assert!((a - b).abs() / 2.0 <= 2.0, "{size} pt: off centre along direction {k}: {a} vs {b}");
                }
            }
            assert!(pairs >= 3, "{size} pt: only {pairs} clear diameters: {reach:?}");
        }
    }

    // once the page says how it shows the game, each tick draws its last step only (as page.html draws only the last of
    // a battery-saver frame's steps), the message asks the page for it, and the page gets that frame
    #[test]
    fn a_tick_draws_its_last_step() {
        let (app, w) = app(file("drawn"));
        let core = app.state::<Core>();
        let mut b = Bridge::new(SEED, SIZE.0, SIZE.1, SIZE.2);
        b.game.set_view(SIZE.0, SIZE.1, true);
        let mut p = Painter::default();
        let (today, t0) = ("Wed Oct 28 2026", 1.7e12);

        b.game.resize(400.0, 356.0, 1.0);
        p.resize(400.0, 356.0, 1.0);
        p.set_view(true, Some('c'), 2.0);
        call(&w, "resize", json!({ "w": 400.0, "h": 356.0, "scale": 1.0 }));
        let args = json!({ "bare": true, "clouds": "c", "amount": 2.0, "calm": false, "x": 0.0, "y": 0.0, "w": 400.0, "h": 356.0, "opacity": 1.0 });
        assert_eq!(call(&w, "view", args), json!(false), "no layer: the page shows the frames");
        b.start(today, t0);
        call(&w, "start", json!({ "today": today, "now_ms": t0 }));
        let mut bank = 0.0;
        for i in 0..300 {
            let elapsed = [DT, 2.0 * DT, 0.05, 0.0][i % 4];
            let now = t0 + i as f64 * 33.0;
            b.clock(now);
            bank += elapsed;
            let mut n = 0;
            while bank >= DT && n < 8 {
                bank -= DT;
                n += 1;
            }
            for k in 1..=n {
                b.step_one_drawn(&mut |g, dt| if k == n { p.draw(g, dt) });
            }
            let msg: Value = serde_json::from_str(&core.tick(elapsed, now).unwrap_or_else(|| EMPTY.into())).unwrap();
            assert_eq!(msg["px"] == json!(true), n > 0, "tick {i}: a frame for the page");
            b.frame_json();
            if n > 0 {
                let req = InvokeRequest {
                    cmd: "planet_pixels".into(),
                    callback: CallbackFn(0),
                    error: CallbackFn(1),
                    url: w.url().unwrap(),
                    body: InvokeBody::Json(json!({})),
                    headers: Default::default(),
                    invoke_key: INVOKE_KEY.to_string(),
                };
                let got = get_ipc_response(&w, req).unwrap();
                assert!(got.deserialize::<Vec<u8>>().unwrap() == p.rgba(), "tick {i}: the frame");
            }
        }
        assert!(p.rgba().chunks(4).any(|c| c[3] == 255), "something drawn");
        assert_eq!(core.with(|c| c.game.save_json()).unwrap(), b.game.save_json());
        assert_eq!(core.with(|c| c.game.rng.draws()).unwrap(), b.game.rng.draws(), "the drawing's random draws");
    }

    // a layer that stops taking frames mid-game: from that frame on the page draws them, and the layer isn't asked again
    #[test]
    fn a_failing_surface_hands_the_frames_to_the_page() {
        use std::sync::{atomic::{AtomicU32, Ordering}, Arc};
        struct Fails(Arc<AtomicU32>);
        impl Surface for Fails {
            fn place(&self, _: [f64; 4], _: f64, bare: bool) -> bool {
                bare
            }
            fn show(&self, _: &[u8], _: u32, _: u32) -> bool {
                self.0.fetch_add(1, Ordering::Relaxed) < 3
            }
        }
        let (app, w) = app(file("fails"));
        let core = app.state::<Core>();
        let shown = Arc::new(AtomicU32::new(0));
        core.set_surface(Box::new(Fails(shown.clone())));
        let (today, t0) = ("Wed Oct 28 2026", 1.7e12);
        call(&w, "resize", json!({ "w": 400.0, "h": 356.0, "scale": 1.0 }));
        let args = json!({ "bare": true, "clouds": "", "amount": 2.0, "calm": false, "x": 0.0, "y": 0.0, "w": 400.0, "h": 356.0, "opacity": 1.0 });
        assert_eq!(call(&w, "view", args), json!(true), "the layer shows the frames");
        call(&w, "start", json!({ "today": today, "now_ms": t0 }));
        let px: Vec<bool> = (0..8)
            .map(|i| {
                let msg: Value = serde_json::from_str(&core.tick(DT, t0 + i as f64 * 33.0).unwrap_or_else(|| EMPTY.into())).unwrap();
                msg["px"] == json!(true)
            })
            .collect();
        assert_eq!(px, [false, false, false, true, true, true, true, true]);
        assert_eq!(shown.load(Ordering::Relaxed), 4, "not asked again once it failed");
    }

    // The painter against game.js drawing the same frame headless (the game's rust/difftest/frame.js, through
    // scripts/frame.cjs), at the widget's canvas sizes and settings, byte for byte. Needs node; skips without
    #[test]
    #[cfg_attr(not(all(target_os = "macos", target_arch = "aarch64")), ignore = "game.js's maths is V8's on arm64; elsewhere it may round an ulp apart")]
    fn the_painter_draws_what_game_js_draws() {
        // the canvas in CSS pixels: floating, the dock's band (7vmin, 18-26px, + 18px) is cut off the bottom; window
        // mode is inside a 1px border, and resizes freely
        const SMALL: (u32, u32) = (300, 261);
        const MEDIUM: (u32, u32) = (400, 356);
        const LARGE: (u32, u32) = (560, 516);
        const RING: (u32, u32) = (400, 400);
        const WIN_M: (u32, u32) = (398, 398);
        const WIN_L: (u32, u32) = (558, 558);
        const WIN_FREE: (u32, u32) = (640, 420);
        // (hook, canvas, frames, clouds: None for window mode, Some("") for none, Some("c2") for shape c amount 2)
        type Case<'a> = (&'a str, (u32, u32), u32, Option<&'a str>);
        // no #shenron (a storm on Rust since game PR #35) or #raid (boss intro and mechanics since game #51/#52): game.js is frozen
        let cases: [Case; 25] = [
            ("", MEDIUM, 200, Some("")),
            ("", MEDIUM, 200, Some("c2")),
            ("", SMALL, 200, Some("c2")),
            ("", LARGE, 200, Some("c2")),
            ("", RING, 200, Some("a1")),
            ("", MEDIUM, 200, Some("b3")),
            ("", MEDIUM, 200, Some("d4")),
            ("", LARGE, 200, Some("e5")),
            ("#namek", MEDIUM, 400, Some("")),
            ("#snake", MEDIUM, 400, Some("")),
            ("#snake", MEDIUM, 1810, Some("c2")),
            ("#top", MEDIUM, 60, Some("c3")),
            ("#ozaru", LARGE, 300, Some("")),
            ("#hero-goku#form5", SMALL, 120, Some("c2")),
            ("#itx", MEDIUM, 60, Some("c2")),
            ("#westcity", MEDIUM, 400, Some("e2")),
            ("#beerus", SMALL, 300, Some("")),
            ("", WIN_M, 200, None),
            ("#namek", WIN_M, 400, None),
            ("#vegeta", WIN_L, 400, None),
            ("#top", WIN_M, 60, None),
            ("#snake", WIN_M, 400, None),
            ("#kame", WIN_FREE, 400, None),
            ("#cooler", WIN_M, 600, None),
            ("#lookout", WIN_L, 400, None),
        ];
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/frame.cjs");
        let tmp = std::env::temp_dir().join(format!("idle-fighterz-frames-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let mut failed = Vec::new();
        for (i, &(hook, (w, h), frames, clouds)) in cases.iter().enumerate() {
            let (bare, shape) = match clouds {
                Some(c) => (true, c.chars().next().map(|s| (s, c[1..].parse::<u32>().unwrap()))),
                None => (false, None),
            };
            let out = tmp.join(format!("case{i}"));
            let mut cmd = std::process::Command::new("node");
            cmd.arg(&script).arg(&out).args(["--px", "--hook", hook, "--w", &w.to_string(), "--h", &h.to_string(), "--frames", &frames.to_string()]);
            if bare {
                cmd.arg("--widget");
                if let Some((s, a)) = shape {
                    cmd.args(["--clouds", &s.to_string(), "--amt", &a.to_string()]);
                }
            } else {
                cmd.arg("--windowed");
            }
            let Ok(o) = cmd.output() else {
                eprintln!("node not found: skipping the painter's pixel test");
                return;
            };
            assert!(o.status.success(), "frame.cjs: {}", String::from_utf8_lossy(&o.stderr));
            let meta: Value = serde_json::from_slice(&o.stdout).unwrap();
            let want = fs::read(out.with_extension("rgba")).unwrap();

            // as the page: st.scale 2 from 520 CSS pixels wide, 1 below, and st.W×st.H the canvas over it, rounded up
            let scale = if w >= 520 { 2.0 } else { 1.0 };
            let (cw, ch) = ((w as f64 / scale).ceil(), (h as f64 / scale).ceil());
            assert_eq!((cw, ch), (meta["W"].as_f64().unwrap(), meta["H"].as_f64().unwrap()));
            let mut g = Game::new(1);
            g.set_view(cw, ch, true);
            g.resize(cw, ch, scale);
            g.preview(hook);
            g.start();
            let mut p = Painter::default();
            p.resize(cw, ch, scale);
            p.set_view(bare, shape.map(|s| s.0), shape.map_or(2.0, |s| s.1 as f64));
            let mut t = 0.0;
            for f in 0..frames {
                if f == frames - 1 {
                    // as frame.js: headless game.js never steps its particles, so neither side has any going in
                    g.fx.clear();
                    g.floats.clear();
                    g.flash_t = 0.0;
                }
                t += DT;
                g.sim(t, DT);
                if f == frames - 1 {
                    p.draw(&mut g, DT);
                }
                g.timeouts();
            }
            let got = p.rgba();
            let px = want.chunks(4).zip(got.chunks(4)).filter(|(a, b)| a != b).count();
            let same_game = g.rng.draws() == meta["draws"].as_u64().unwrap() && g.s.world as u64 == meta["world"].as_u64().unwrap();
            let opaque = got.chunks(4).filter(|c| c[3] > 0).count();
            eprintln!("{:<18} {:>3}x{:<3} {:<6} f{:<5} {}x{} {:>6} px drawn: {}", if hook.is_empty() { "(home)" } else { hook }, w, h, clouds.unwrap_or("window"), frames, cw, ch, opaque, if px == 0 && same_game { "identical".into() } else { format!("{px} px differ") });
            if px != 0 || !same_game || want.len() != got.len() {
                failed.push(format!("{hook} {w}x{h} {clouds:?}"));
            }
        }
        let _ = fs::remove_dir_all(&tmp);
        assert!(failed.is_empty(), "the painter differs from game.js: {failed:?}");
    }

    #[test]
    fn short_matches_widget_js() {
        for (n, want) in [(0.0, "0"), (999.9, "999"), (1000.0, "1.0K"), (1234.0, "1.2K"), (999_949.0, "999.9K"), (999_950.0, "1.0M"), (3.45e6, "3.5M"), (5.6e9, "5.6B"), (7.85e12, "7.9e12"), (1e15, "1.0e15")] {
            assert_eq!(short(n), want, "{n}");
        }
        assert_eq!(short(Big::new(4.5, 1204)), "4.5e1,204", "past 1e300, as page.html's sci");
        assert_eq!(short(Big::new(7.12, 1_234_567)), "7.1e1.2M");
    }

    #[test]
    fn the_save_comes_back() {
        let f = file("save");
        let core = Core::new(SEED, f.clone());
        core.with(|b| b.start("Wed Oct 28 2026", 1.7e12)).unwrap();
        core.tick(1.0, 1.7e12);
        core.save();
        assert_eq!(fs::read_to_string(&f).unwrap(), core.with(|b| b.game.save_json()).unwrap());
        let again = Core::new(SEED + 1, f.clone());
        assert!(again.with(|b| b.game.save_json()).unwrap().contains("\"daily\":\"Wed Oct 28 2026\""));
        let _ = fs::remove_file(f);
    }

    #[test]
    fn the_save_is_replaced_whole_and_not_after_exit() {
        let f = file("atomic");
        let core = Core::new(SEED, f.clone());
        core.with(|b| b.start("Wed Oct 28 2026", 1.7e12)).unwrap();
        core.save();
        assert!(f.exists() && !f.with_extension("json.tmp").exists());
        core.stop();
        let last = fs::read_to_string(&f).unwrap();
        core.tick(5.0, 1.7e12 + 5000.0);
        core.save();
        assert_eq!(fs::read_to_string(&f).unwrap(), last, "saved after the exit save");
        let _ = fs::remove_file(f);
    }

    #[test]
    fn only_a_small_regular_file_is_read() {
        let f = file("big");
        fs::write(&f, " ".repeat(MAX_SAVE + 1)).unwrap();
        assert!(read_save(&f).is_none());
        fs::write(&f, "{}").unwrap();
        assert_eq!(read_save(&f).as_deref(), Some("{}"));
        #[cfg(unix)]
        {
            let link = file("link");
            std::os::unix::fs::symlink(&f, &link).unwrap();
            assert!(read_save(&link).is_none(), "a link is followed");
            let _ = fs::remove_file(link);
        }
        let _ = fs::remove_file(f);
    }

    #[test]
    fn a_crafted_save_is_never_parsed() {
        let f = file("deep");
        fs::write(&f, "[".repeat(100_000)).unwrap();
        let core = Core::new(SEED, f.clone());
        assert!(!core.with(|b| b.game.playing).unwrap());
        assert!(!loadable(&"{\"a\":".repeat(50_000)));
        assert!(loadable(&format!("{{\"daily\":\"[[[{}\"}}", "[".repeat(40))), "brackets in a string are text");
        assert!(!loadable(&" ".repeat(MAX_SAVE + 1)));
        let _ = fs::remove_file(f);
    }

    // the shown game tells the spectacles once when a saga villain's fight starts and once a full beam, in the page's
    // pixels; a beam short of full, nothing
    #[test]
    fn raids_and_full_beams_reach_the_spectacles_once() {
        let core = Core::new(SEED, file("moments"));
        core.with(|b| b.start("Wed Oct 28 2026", 1.7e12)).unwrap();
        {
            let mut p = core.paint();
            p.painter.resize(SIZE.0, SIZE.1, SIZE.2);
            (p.canvas, p.on) = ([10.0, 20.0, 266.0, 266.0], true);
        }
        let mut now = 1.7e12;
        let mut tick = |core: &Core| {
            now += 4.0 * DT * 1000.0;
            core.tick(4.0 * DT, now);
            core.moments()
        };
        let mut raids = Vec::new();
        for _ in 0..(600.0 / (4.0 * DT)) as usize {
            for m in tick(&core) {
                if let Moment::Raid { id, x, y, sprite, scale } = m {
                    assert!((0.5..3.0).contains(&scale), "his size on the page: {scale}");
                    assert!((10.0..276.0).contains(&x) && (20.0..286.0).contains(&y), "on the canvas: {x},{y}");
                    assert!(sprite.w > 10);
                    raids.push(id);
                }
            }
            if !raids.is_empty() {
                break;
            }
        }
        assert_eq!(raids.len(), 1, "a saga raid within ten minutes");
        let id = raids[0];
        assert!(core.raid_on(id) && !core.raid_on(id + 1));
        assert!(tick(&core).iter().all(|m| !matches!(m, Moment::Raid { .. })), "once");

        let fire = |core: &Core, full: bool, t0: f64| {
            core.with(|b| {
                let c = if full { 2.5 } else { 1.0 };
                let beam = planet_core::input::Beam { t0, c, x: 60.0, y: 70.0, dx: 1.0, dy: 0.0, z: 0.0, ff: false, full, kaio: false, ape: false };
                b.game.inp.beam = Some(beam);
            })
            .unwrap();
        };
        let t = core.with(|b| b.game.t).unwrap();
        fire(&core, false, t);
        assert!(tick(&core).is_empty(), "a short beam isn't one");
        fire(&core, true, t + 0.01);
        let ms = tick(&core);
        assert!(matches!(ms[..], [Moment::Beam { x: 130.0, y: 160.0, dx: 2.0, dy: 0.0, .. }]), "the canvas is the buffer at 2×");
        assert!(tick(&core).is_empty(), "once");
    }
}

#[cfg(test)]
mod away_tests {
    use super::*;
    use crate::spectacle::scene;

    // the widget (1×) and its frame over a made-up desktop (never the real screen), with the villain on a window if he's out
    fn desktop(frame: &[u8], fw: usize, fh: usize, villain: Option<(&[scene::Image; 2], (f64, f64), f64)>) -> scene::Canvas {
        let (w, h) = (1200usize, 720usize);
        let mut c = scene::Canvas::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let t = y as f64 / h as f64;
                let k = (y * w + x) * 4;
                c.rgba[k..k + 4].copy_from_slice(&[(40.0 + 60.0 * t) as u8, (60.0 + 40.0 * t) as u8, (110.0 - 30.0 * t) as u8, 255]);
            }
        }
        let pane = |c: &mut scene::Canvas, x: usize, y: usize, pw: usize, ph: usize| {
            for yy in y..y + ph {
                for xx in x..x + pw {
                    let k = (yy * w + xx) * 4;
                    let v = if yy < y + 28 { [214, 214, 220, 255] } else { [246, 246, 248, 255] };
                    c.rgba[k..k + 4].copy_from_slice(&v);
                }
            }
        };
        pane(&mut c, 60, 330, 520, 330);
        pane(&mut c, 180, 220, 300, 90);
        let img = scene::Image { w: fw, h: fh, rgba: frame.to_vec() };
        c.feet(&img, 1200.0 - fw as f64 / 2.0 - 20.0, fh as f64 + 10.0, 1.0, 1.0);
        if let Some((faces, (x, y), px)) = villain {
            c.feet(&faces[0], x, y, px, 1.0);
        }
        c
    }

    // While the villain is out on your windows the widget draws its planet without him; when he leaps back, he's there
    // again. SPECTACLE_CAPTURE=<dir> keeps the three moments as PNGs
    #[test]
    fn the_widget_draws_no_boss_while_he_is_out() {
        let core = Core::new(7, std::env::temp_dir().join(format!("idle-fighterz-test-{}-away.json", std::process::id())));
        // a 300 pt widget, drawn 1×, as on a desktop
        core.with(|b| {
            b.start("Wed Oct 28 2026", 1.7e12);
            b.game.resize(300.0, 300.0, 1.0);
        })
        .unwrap();
        {
            let mut p = core.paint();
            p.painter.resize(300.0, 300.0, 1.0);
            (p.canvas, p.on) = ([0.0, 0.0, 300.0, 300.0], true);
        }
        let mut now = 1.7e12;
        let mut raid = None;
        for _ in 0..(600.0 / (4.0 * DT)) as usize {
            now += 4.0 * DT * 1000.0;
            core.tick(4.0 * DT, now);
            raid = core.moments().into_iter().find_map(|m| if let Moment::Raid { sprite, scale, .. } = m { Some((sprite, scale)) } else { None });
            if raid.is_some() {
                break;
            }
        }
        let (sprite, scale) = raid.expect("a saga raid within ten minutes");
        // past his entrance, then the same moment drawn with him in and out
        for _ in 0..60 {
            now += 4.0 * DT * 1000.0;
            core.tick(4.0 * DT, now);
        }
        // and not mid-hit (a hit flashes him white)
        while core.with(|b| b.game.tl.boss.as_ref().is_some_and(|x| x.flash > 0.0)).unwrap() {
            now += DT * 1000.0;
            core.tick(DT, now);
        }
        let draw = |away: bool| {
            core.boss_away(away);
            core.with(|b| {
                let mut p = core.paint();
                p.painter.draw(&mut b.game, 0.0);
                p.painter.rgba().to_vec()
            })
            .unwrap()
        };
        let (here, away, back) = (draw(false), draw(true), draw(false));
        let (w, h) = (300, 300);
        let changed = |a: &[u8], b: &[u8]| (0..w * h).filter(|&i| a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4]).count();
        // a few pixels move between any two draws (sparkles); he is far more than that
        let (gone, moved) = (changed(&here, &away), changed(&here, &back));
        assert!(gone > 100 && gone > 5 * moved, "he's gone from the planet: {gone} pixels, {moved} between two draws of him");

        let Some(dir) = std::env::var_os("SPECTACLE_CAPTURE") else { return };
        let dir = std::path::PathBuf::from(dir);
        fs::create_dir_all(&dir).unwrap();
        let faces = scene::faces(sprite.w, sprite.h, &sprite.col);
        for (name, frame, out) in [("1-boss-here", &here, false), ("2-boss-away", &away, true), ("3-boss-back", &back, false)] {
            let c = desktop(frame, w, h, out.then_some((&faces, (330.0, 220.0), scale)));
            let mut e = png::Encoder::new(std::io::BufWriter::new(fs::File::create(dir.join(format!("widget-{name}.png"))).unwrap()), 1200, 720);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header().unwrap().write_image_data(&c.rgba).unwrap();
        }
    }
}
