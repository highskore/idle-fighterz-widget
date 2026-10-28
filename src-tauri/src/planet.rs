// The game on the Rust core (cargo feature `rust-core`): planet-core's `Bridge` runs natively behind one Tauri command
// per call (game/rust/HUD.md), a tick thread steps it and pushes each frame's message to the HUD page as a `planet`
// event, and the save lives in a file. Hidden, the page gets nothing: the game catches up once a second and the app
// shows its power level and notifications itself. Nothing here draws: see `Renderer`. See NATIVE.md.
use planet_core::{
    bridge::Bridge,
    hud::Cue,
    num::{js_string, to_exponential, to_fixed},
    Game, DT,
};
use std::{
    fs,
    panic::AssertUnwindSafe,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State};

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
    /// the rate the page's widget.js picked (awake, the frame-rate setting), 0 until it says
    fps: Mutex<f64>,
}

/// What a hidden catch-up leaves for the app: the power level, and the notifications the page would have sent.
pub struct Caught {
    pub pl: f64,
    pub notes: Vec<(&'static str, String)>,
}

impl Core {
    /// A new game seeded with `seed`, carrying over what the save in `file` keeps (its daily bonus date).
    pub fn new(seed: u32, file: PathBuf) -> Self {
        let mut bridge = Bridge::new(seed, SIZE.0, SIZE.1, SIZE.2);
        let small = fs::metadata(&file).is_ok_and(|m| m.len() <= MAX_SAVE as u64);
        if let Some(saved) = small.then(|| fs::read_to_string(&file).ok()).flatten().filter(|s| loadable(s)) {
            bridge.load(&saved);
        }
        Core { bridge: Mutex::new(bridge), file, fps: Mutex::new(0.0) }
    }

    // a panic in the game fails the one call (debug builds; release aborts on panic) and leaves the game usable
    fn with<T>(&self, f: impl FnOnce(&mut Bridge) -> T) -> Result<T, String> {
        let mut b = self.bridge.lock().unwrap_or_else(|e| e.into_inner());
        std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut b))).map_err(|e| {
            e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_else(|| "the game panicked".into())
        })
    }

    /// Steps by `elapsed` real seconds; the frame's message, unless it carries nothing.
    pub fn tick(&self, elapsed: f64, now_ms: f64) -> Option<String> {
        let msg = self
            .with(|b| {
                b.step(elapsed, now_ms);
                b.frame_json()
            })
            .ok()?;
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

    pub fn can_ascend(&self) -> bool {
        self.with(|b| can_ascend(&b.game)).unwrap_or(false)
    }

    pub fn save(&self) {
        let Ok(text) = self.with(|b| b.game.save_json()) else { return };
        let _ = fs::create_dir_all(self.file.parent().unwrap());
        let _ = fs::write(&self.file, text);
    }
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

fn can_ascend(g: &Game) -> bool {
    g.shop().iter().any(|r| r.key == "ascend" && r.can)
}

/// widget.js `short`: the power level beside the menu-bar icon, e.g. 1.2K, 3.4M, 5.6B, then 7.8e12.
pub fn short(n: f64) -> String {
    const SUF: [&str; 4] = ["", "K", "M", "B"];
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
        loop {
            if shown.get() {
                let fps = Some(*core.fps.lock().unwrap()).filter(|f| *f > 0.0).unwrap_or_else(|| (hooks.idle_fps)(&app));
                std::thread::sleep(Duration::from_secs_f64(1.0 / fps));
                let now = Instant::now();
                let elapsed = (now - last).as_secs_f64();
                last = now;
                if let Some(msg) = core.tick(elapsed, now_ms()) {
                    let _ = app.emit_to("main", "planet", msg);
                }
                if saved.elapsed() >= Duration::from_secs(3) && core.with(|b| b.game.playing).unwrap_or(false) {
                    saved = Instant::now();
                    core.save();
                }
                continue;
            }
            let mut ascend = core.can_ascend();
            loop {
                let plays = (hooks.plays_hidden)(&app);
                shown.nap(plays.then_some(HIDDEN_TICK));
                let now = Instant::now();
                let elapsed = (now - last).as_secs_f64().min(MAX_CATCH_UP);
                last = now;
                // paused, the time hidden isn't played (game.js: no steps while hidden)
                if plays {
                    if let Some(c) = core.catch_up(elapsed, now_ms(), &mut kept, &mut ascend) {
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

/// Where stage 1's planet-render draws the game: RGBA8, straight alpha, rows top to bottom, `size()` pixels. Nothing
/// implements it yet; `Frames` hands it to the native renderer (stage 5b).
#[allow(dead_code)]
pub trait Renderer: Send {
    fn size(&self) -> (u32, u32);
    fn draw(&mut self, game: &Game, rgba: &mut [u8]);
}

/// The native renderer's source over this game: the tick thread steps it, so a step only draws.
#[cfg(all(target_os = "macos", feature = "native-renderer"))]
#[allow(dead_code)]
pub struct Frames<R: Renderer> {
    app: AppHandle,
    renderer: R,
    buf: Vec<u8>,
}

#[cfg(all(target_os = "macos", feature = "native-renderer"))]
#[allow(dead_code)]
impl<R: Renderer> Frames<R> {
    pub fn new(app: AppHandle, renderer: R) -> Self {
        let (w, h) = renderer.size();
        Frames { app, renderer, buf: vec![0; (w * h * 4) as usize] }
    }
}

#[cfg(all(target_os = "macos", feature = "native-renderer"))]
impl<R: Renderer> crate::native::FrameSource for Frames<R> {
    fn size(&self) -> (u32, u32) {
        self.renderer.size()
    }
    fn step(&mut self, _dt: f32) {
        let core = self.app.state::<Core>();
        let b = core.bridge.lock().unwrap();
        self.renderer.draw(&b.game, &mut self.buf);
    }
    fn frame(&self) -> &[u8] {
        &self.buf
    }
}

// ── the calls (HUD.md's table), as `planet_<call>` with HUD.md's argument names. `new` is the app's (the game outlives
// the page), and `spots`/`probe` were the web's lockstep twin's

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

#[tauri::command(rename_all = "snake_case")]
pub fn planet_load(core: State<'_, Core>, text: String) -> Result<(), String> {
    if !loadable(&text) {
        return Err("not a save".into());
    }
    core.with(|b| b.load(&text))
}

#[tauri::command(rename_all = "snake_case")]
pub fn planet_step(core: State<'_, Core>, elapsed: f64, now_ms: f64) -> Result<u32, String> {
    core.with(|b| b.step(elapsed, now_ms))
}

#[tauri::command(rename_all = "snake_case")]
pub fn planet_step_one(core: State<'_, Core>, now_ms: f64) -> Result<f64, String> {
    core.with(|b| {
        b.clock(now_ms);
        b.step_one()
    })
}

#[tauri::command]
pub fn planet_frame(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.frame_json())
}

#[tauri::command]
pub fn planet_resend(core: State<'_, Core>) -> Result<(), String> {
    core.with(|b| b.resend())
}

#[tauri::command]
pub fn planet_resize(core: State<'_, Core>, w: f64, h: f64, scale: f64) -> Result<(), String> {
    core.with(|b| b.game.resize(w, h, scale))
}

#[tauri::command]
pub fn planet_pointer_down(core: State<'_, Core>, x: f64, y: f64, bx: f64, by: f64) -> Result<(), String> {
    core.with(|b| b.game.pointer_down(x, y, bx, by))
}

#[tauri::command]
pub fn planet_pointer_move(core: State<'_, Core>, x: f64, y: f64, bx: f64, by: f64) -> Result<(), String> {
    core.with(|b| b.game.pointer_move(x, y, bx, by))
}

#[tauri::command]
pub fn planet_pointer_up(core: State<'_, Core>, bx: f64, by: f64) -> Result<(), String> {
    core.with(|b| b.game.pointer_up(bx, by))
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

#[tauri::command]
pub fn planet_card(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.game.card_json())
}

#[tauri::command]
pub fn planet_save(core: State<'_, Core>) -> Result<String, String> {
    core.with(|b| b.game.save_json())
}

#[tauri::command]
pub fn planet_playing(core: State<'_, Core>) -> Result<bool, String> {
    core.with(|b| b.game.playing)
}

/// Every `planet_*` command, for `invoke_handler` (main.rs adds the app's own).
macro_rules! planet_commands {
    ($($more:path),* $(,)?) => {
        tauri::generate_handler![
            $($more,)*
            crate::planet::planet_start, crate::planet::planet_load, crate::planet::planet_step, crate::planet::planet_step_one, crate::planet::planet_frame,
            crate::planet::planet_resend, crate::planet::planet_resize, crate::planet::planet_pointer_down, crate::planet::planet_pointer_move,
            crate::planet::planet_pointer_up, crate::planet::planet_pointer_cancel, crate::planet::planet_key, crate::planet::planet_train,
            crate::planet::planet_kaioken, crate::planet::planet_buy, crate::planet::planet_learn, crate::planet::planet_respec, crate::planet::planet_learn_angel,
            crate::planet::planet_auto_next, crate::planet::planet_pick_wish, crate::planet::planet_pick_hero, crate::planet::planet_menu,
            crate::planet::planet_angels, crate::planet::planet_ascend_info, crate::planet::planet_god_info, crate::planet::planet_zeno_info,
            crate::planet::planet_card, crate::planet::planet_save, crate::planet::planet_playing, crate::planet::planet_fps
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
        let f = std::env::temp_dir().join(format!("powerrr-test-{}-{name}.json", std::process::id()));
        let _ = fs::remove_file(&f);
        f
    }

    // a minute of play driven the HUD's way (commands over IPC, frames from the tick) against the same calls on a Bridge
    #[test]
    fn commands_send_what_the_bridge_sends() {
        let (app, w) = app(file("same"));
        let core = app.state::<Core>();
        let mut b = Bridge::new(SEED, SIZE.0, SIZE.1, SIZE.2);
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
            if let Some(list) = serde_json::from_str::<Value>(&want).unwrap()["wish"].as_array() {
                let key = list.last().unwrap()["key"].as_str().unwrap().to_string();
                b.game.pick(&key);
                call(&w, "pick_wish", json!({ "key": key }));
            }

            if i % 5 == 0 {
                b.game.pointer_down(300.0, 200.0, 100.0, 66.0);
                b.game.pointer_up(100.0, 66.0);
                call(&w, "pointer_down", json!({ "x": 300.0, "y": 200.0, "bx": 100.0, "by": 66.0 }));
                call(&w, "pointer_up", json!({ "bx": 100.0, "by": 66.0 }));
            }
            if i % 200 == 100 {
                b.game.pointer_down(300.0, 200.0, 100.0, 66.0);
                call(&w, "pointer_down", json!({ "x": 300.0, "y": 200.0, "bx": 100.0, "by": 66.0 }));
                for k in 1..8 {
                    let (x, y) = (300.0 + k as f64 * 9.0, 200.0 - k as f64 * 3.0);
                    b.game.pointer_move(x, y, x / 3.0, y / 3.0);
                    call(&w, "pointer_move", json!({ "x": x, "y": y, "bx": x / 3.0, "by": y / 3.0 }));
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
                assert_eq!(call(&w, "frame", json!({})), json!(b.frame_json()), "frame {i} on request");
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
                    ("save", b.game.save_json()),
                ] {
                    assert_eq!(call(&w, cmd, json!({})), json!(want), "{cmd} at {i}");
                }
            }
        }
        assert!(frames > 100, "{frames} frames sent");
        assert_eq!(call(&w, "step", json!({ "elapsed": 0.1, "now_ms": t0 })), json!(b.step(0.1, t0)));
        b.clock(t0);
        assert_eq!(call(&w, "step_one", json!({ "now_ms": t0 })), json!(b.step_one()));
        assert_eq!(call(&w, "playing", json!({})), json!(true));
        b.resend();
        call(&w, "resend", json!({}));
        assert_eq!(call(&w, "frame", json!({})), json!(b.frame_json()));
        assert_eq!(call(&w, "save", json!({})), json!(b.game.save_json()));
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
            v["cues"].as_array().unwrap().iter().filter(|c| c[0] == "notify").map(|c| (c[1].as_str().unwrap().into(), c[2].as_str().unwrap().into())).collect()
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

    #[test]
    fn short_matches_widget_js() {
        for (n, want) in [(0.0, "0"), (999.9, "999"), (1000.0, "1.0K"), (1234.0, "1.2K"), (999_949.0, "999.9K"), (999_950.0, "1.0M"), (3.45e6, "3.5M"), (5.6e9, "5.6B"), (7.85e12, "7.9e12"), (1e15, "1.0e15")] {
            assert_eq!(short(n), want, "{n}");
        }
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
    fn a_crafted_save_is_never_parsed() {
        let f = file("deep");
        fs::write(&f, "[".repeat(100_000)).unwrap();
        let core = Core::new(SEED, f.clone());
        assert!(!core.with(|b| b.game.playing).unwrap());
        let (app, w) = app(file("deep-cmd"));
        let req = |text: String| InvokeRequest {
            cmd: "planet_load".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: w.url().unwrap(),
            body: InvokeBody::Json(json!({ "text": text })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        assert!(get_ipc_response(&w, req("{\"a\":".repeat(50_000))).is_err());
        assert!(get_ipc_response(&w, req(format!("{{\"daily\":\"[[[{}\"}}", "[".repeat(40)))).is_ok(), "brackets in a string are text");
        assert!(!loadable(&" ".repeat(MAX_SAVE + 1)));
        drop(app);
        let _ = fs::remove_file(f);
    }
}
