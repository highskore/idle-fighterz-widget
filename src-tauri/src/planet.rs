// The game on the Rust core (cargo feature `rust-core`): planet-core's `Bridge` runs natively behind one Tauri command
// per call (game/rust/HUD.md), a tick thread steps it and pushes each frame's message to the HUD page as a `planet`
// event, and the save lives in a file. Nothing here draws: see `Renderer`. See NATIVE.md.
use planet_core::{bridge::Bridge, Game};
use std::{
    fs,
    io::Read,
    panic::AssertUnwindSafe,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
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

pub struct Core {
    bridge: Mutex<Bridge>,
    file: PathBuf,
    // true once the app is exiting: held across every write, so the exit save is the last one and none interleave
    stopped: Mutex<bool>,
    /// the pointer is over the widget: frames go out at the hovered rate
    pub hovered: AtomicBool,
}

impl Core {
    /// A new game seeded with `seed`, carrying over what the save in `file` keeps (its daily bonus date).
    pub fn new(seed: u32, file: PathBuf) -> Self {
        let mut bridge = Bridge::new(seed, SIZE.0, SIZE.1, SIZE.2);
        if let Some(saved) = read_save(&file).filter(|s| loadable(s)) {
            bridge.load(&saved);
        }
        Core { bridge: Mutex::new(bridge), file, stopped: Mutex::new(false), hovered: AtomicBool::new(false) }
    }

    // only in debug builds does a panic in the game fail just the one call: release builds have panic = "abort", so
    // there it ends the app and this catches nothing
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

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

pub fn seed() -> u32 {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() ^ d.as_secs() as u32).unwrap_or(1);
    n.max(1)
}

/// Steps the game and sends the HUD what changed: at the frame-rate setting (`idle_fps`), doubled up to 60 while the
/// pointer is over the widget, 4 times a second while it's hidden. Saves every 3 s of play.
pub fn run(app: AppHandle, idle_fps: fn(&AppHandle) -> f64) {
    std::thread::spawn(move || {
        let core = app.state::<Core>();
        let (mut last, mut saved, mut checked) = (Instant::now(), Instant::now(), Instant::now());
        let mut visible = true;
        loop {
            if core.stopped() {
                return;
            }
            if checked.elapsed() >= Duration::from_millis(250) {
                checked = Instant::now();
                visible = app.get_webview_window("main").and_then(|w| w.is_visible().ok()).unwrap_or(true);
            }
            let idle = idle_fps(&app);
            let fps = if !visible { 4.0 } else if core.hovered.load(Ordering::Relaxed) { (idle * 2.0).min(60.0) } else { idle };
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
// the page), and `spots`/`probe` were the web's lockstep twin's. The app steps, frames and saves the game itself, so
// `load`, `step`, `step_one`, `frame`, `save` and `playing` aren't the page's to call

// a HUD page loaded again mounts again: the game carries on
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

/// Every `planet_*` command, for `invoke_handler` (main.rs adds the app's own).
macro_rules! planet_commands {
    ($($more:path),* $(,)?) => {
        tauri::generate_handler![
            $($more,)*
            crate::planet::planet_start, crate::planet::planet_resend, crate::planet::planet_resize, crate::planet::planet_pointer_down, crate::planet::planet_pointer_move,
            crate::planet::planet_pointer_up, crate::planet::planet_pointer_cancel, crate::planet::planet_key, crate::planet::planet_train,
            crate::planet::planet_kaioken, crate::planet::planet_buy, crate::planet::planet_learn, crate::planet::planet_respec, crate::planet::planet_learn_angel,
            crate::planet::planet_auto_next, crate::planet::planet_pick_wish, crate::planet::planet_pick_hero, crate::planet::planet_menu,
            crate::planet::planet_angels, crate::planet::planet_ascend_info, crate::planet::planet_god_info, crate::planet::planet_zeno_info,
            crate::planet::planet_card
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
}
