#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window behind the widget on Windows
// The power planet as a desktop pet: one frameless, transparent window showing the bundled page, a settings
// window, and a menu-bar icon. Settings are app config in a JSON file; the game itself is never saved.
use serde_json::{json, Value};
use std::{fs, path::PathBuf, sync::{mpsc::Sender, Condvar, Mutex}, time::Duration};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

#[cfg(target_os = "macos")]
mod notes;
#[cfg(all(target_os = "macos", feature = "native-renderer"))]
mod native;
#[cfg(feature = "rust-core")]
#[macro_use]
mod planet;

// window sides in points, square: the planet sits in the middle with room for its visitors
const SIZES: [(&str, &str, f64); 3] = [("small", "Small", 300.0), ("medium", "Medium", 400.0), ("large", "Large", 560.0)];
// every setting and its default; a saved file only overrides keys listed here
const DEFAULTS: &str = r#"{
  "on_top": true, "size": "medium", "opacity": 1, "layout": 7, "hud": "hover", "clouds": "hover", "cloud_shape": "c", "cloud_amount": 2, "mode": "floating", "menubar_power": true, "fps": 30, "tray_icon": "auto",
  "notify": { "raid": true, "shenron": true, "trip": true, "pod": false, "ascend": true },
  "quiet": { "on": false, "from": "22:00", "to": "08:00" },
  "shortcut": "Alt+Cmd+KeyP"
}"#;
#[cfg(not(target_os = "macos"))]
const SHORTCUT: &str = "Ctrl+Alt+KeyP"; // Win+Alt+P / Super+Alt+P belong to the desktop; ⌥⌘P stays the macOS default

struct Tray {
    show: MenuItem<Wry>,
    top: CheckMenuItem<Wry>,
    sizes: Vec<CheckMenuItem<Wry>>,
    login: CheckMenuItem<Wry>,
    #[cfg(target_os = "linux")]
    menu: Menu<Wry>,
    #[cfg(target_os = "linux")]
    power: MenuItem<Wry>, // a greyed-out first item, in the menu only while the power level is shown
}

struct State {
    settings: Mutex<Value>,
    file: PathBuf,
    tray: Mutex<Option<Tray>>,
    power: Mutex<String>, // the power level as the widget last sent it, for the menu-bar title
    transparent: bool,    // false on a Linux desktop with no compositor: floating can't work, so it's always window mode
    wayland: bool,        // no global shortcut and no pointer polling there
    tray_nudge: Mutex<Option<Sender<()>>>, // wakes the tray icon's watcher (Windows, Linux) when settings change
    #[cfg(feature = "rust-core")]
    sent: Mutex<std::collections::HashMap<&'static str, std::time::Instant>>, // the hidden game's last notification of each kind
}

// whether the widget is on screen. threads that only work for a shown widget wait on this instead of polling the
// window (every is_visible() is a round trip through the main thread)
struct Shown {
    on: Mutex<(bool, u64)>, // shown, and a count of changes (to it or to the settings)
    changed: Condvar,
}

impl Shown {
    fn new() -> Self {
        Shown { on: Mutex::new((true, 0)), changed: Condvar::new() }
    }
    fn set(&self, on: bool) {
        let mut s = self.on.lock().unwrap();
        *s = (on, s.1 + 1);
        self.changed.notify_all();
    }
    /// wakes a nap: the settings changed
    fn poke(&self) {
        self.on.lock().unwrap().1 += 1;
        self.changed.notify_all();
    }
    fn get(&self) -> bool {
        self.on.lock().unwrap().0
    }
    /// blocks until the widget shows
    fn wait(&self) {
        drop(self.changed.wait_while(self.on.lock().unwrap(), |s| !s.0));
    }
    /// blocks for `d` (None: for good), or until the widget shows or the settings change
    #[cfg_attr(not(feature = "rust-core"), allow(dead_code))]
    fn nap(&self, d: Option<Duration>) {
        let s = self.on.lock().unwrap();
        let n = s.1;
        match d {
            Some(d) => drop(self.changed.wait_timeout_while(s, d, |s| !s.0 && s.1 == n)),
            None => drop(self.changed.wait_while(s, |s| !s.0 && s.1 == n)),
        }
    }
}

fn merge(base: &mut Value, over: &Value) {
    let (Some(b), Some(o)) = (base.as_object_mut(), over.as_object()) else { return };
    for (k, v) in o {
        match b.get_mut(k) {
            Some(bv) if bv.is_object() => merge(bv, v),
            Some(bv) if std::mem::discriminant(bv) == std::mem::discriminant(v) => *bv = v.clone(),
            _ => {}
        }
    }
}

// what the pages see: the settings, plus whether login launch is on, the app's name (productName, the one place it's set),
// the OS (for key labels) and whether the window can be see-through (if not, the mode reads as window)
fn with_login(app: &AppHandle, mut s: Value) -> Value {
    s["login"] = json!(app.autolaunch().is_enabled().unwrap_or(false));
    s["name"] = json!(app.package_info().name);
    s["os"] = json!(std::env::consts::OS);
    let transparent = app.state::<State>().transparent;
    s["transparent"] = json!(transparent);
    s["wayland"] = json!(app.state::<State>().wayland);
    if !transparent {
        s["mode"] = json!("window");
    }
    s
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Value {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    with_login(&app, s)
}

// merges a partial update, saves it, and applies it to the window, the tray and both pages. a shortcut that can't be
// bound (malformed, or another app holds it) is refused and nothing is saved
#[tauri::command]
fn set_settings(app: AppHandle, settings: Value) -> Result<(), String> {
    if let Some(new) = settings.get("shortcut").and_then(Value::as_str) {
        let old = app.state::<State>().settings.lock().unwrap()["shortcut"].as_str().unwrap_or("").to_string();
        if new != old && !app.state::<State>().wayland {
            bind(&app, &old, new)?;
        }
    }
    let state = app.state::<State>();
    let s = {
        let mut cur = state.settings.lock().unwrap();
        merge(&mut cur, &settings);
        let _ = fs::create_dir_all(state.file.parent().unwrap());
        let _ = fs::write(&state.file, serde_json::to_vec_pretty(&*cur).unwrap());
        cur.clone()
    };
    if let Some(on) = settings.get("login").and_then(Value::as_bool) {
        let auto = app.autolaunch();
        let _ = if on { auto.enable() } else { auto.disable() };
    }
    apply(&app, &s, settings.get("size").is_some() || settings.get("mode").is_some());
    app.state::<Shown>().poke();
    let _ = app.emit("settings", with_login(&app, s));
    Ok(())
}

// the global show/hide shortcut, so the widget comes back even when the menu-bar icon is hidden behind the notch.
// "" turns it off
fn bind(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    if !old.is_empty() {
        let _ = gs.unregister(old);
    }
    if new.is_empty() {
        return Ok(());
    }
    gs.on_shortcut(new, |app, _, e| {
        if e.state == ShortcutState::Pressed {
            let visible = app.get_webview_window("main").map(|w| w.is_visible().unwrap_or(true)).unwrap_or(true);
            show_widget(app, !visible);
        }
    })
    .map_err(|e| {
        if !old.is_empty() {
            let _ = bind(app, "", old);
        }
        e.to_string()
    })
}

// `resize`: snap to the size preset (on launch, and when the size or mode is picked); otherwise a window-mode window
// keeps the size it was dragged to
fn apply(app: &AppHandle, s: &Value, resize: bool) {
    let win = app.get_webview_window("main").expect("main window");
    let on_top = s["on_top"].as_bool().unwrap_or(true);
    let size = SIZES.iter().position(|(id, _, _)| s["size"] == *id).unwrap_or(1);
    let _ = win.set_always_on_top(on_top);
    if resize {
        let _ = win.set_size(LogicalSize::new(SIZES[size].2, SIZES[size].2));
    }
    // window mode is an opaque card: it casts a shadow and can be resized freely; floating, it's neither
    let windowed = s["mode"] == "window" || !app.state::<State>().transparent;
    let _ = win.set_shadow(windowed);
    let _ = win.set_resizable(windowed);
    if let Some(t) = app.state::<State>().tray.lock().unwrap().as_ref() {
        let _ = t.top.set_checked(on_top);
        for (k, item) in t.sizes.iter().enumerate() {
            let _ = item.set_checked(k == size);
        }
        let _ = t.login.set_checked(app.autolaunch().is_enabled().unwrap_or(false));
    }
    if let Some(nudge) = app.state::<State>().tray_nudge.lock().unwrap().as_ref() {
        let _ = nudge.send(());
    }
    show_power(app, s);
}

// the power level beside the menu-bar icon, or the icon alone when that's turned off. Windows trays have no title, so
// it goes in the tooltip. Linux trays (AppIndicator) have no tooltip, and only some panels show its label beside the
// icon (GNOME's AppIndicator extension does, KDE Plasma doesn't), so there it's also the menu's first item
fn show_power(app: &AppHandle, s: &Value) {
    let Some(tray) = app.tray_by_id("tray") else { return };
    let power = app.state::<State>().power.lock().unwrap().clone();
    let on = s["menubar_power"].as_bool().unwrap_or(true) && !power.is_empty();
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(on.then_some(power));
    #[cfg(target_os = "linux")]
    {
        let _ = tray.set_title(on.then_some(&power));
        let items = app.state::<State>().tray.lock().unwrap().as_ref().map(|t| (t.menu.clone(), t.power.clone()));
        if let Some((menu, item)) = items {
            let listed = menu.get(item.id()).is_some();
            if on {
                let _ = item.set_text(format!("Power {power}"));
            }
            if on && !listed {
                let _ = menu.insert(&item, 0);
            } else if !on && listed {
                let _ = menu.remove(&item);
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let name = &app.package_info().name;
        let _ = tray.set_tooltip(Some(if on { format!("{name} {power}") } else { name.clone() }));
    }
}

#[tauri::command]
fn tray_title(app: AppHandle, text: String) {
    let state = app.state::<State>();
    *state.power.lock().unwrap() = text;
    let s = state.settings.lock().unwrap().clone();
    show_power(&app, &s);
}

fn show_widget(app: &AppHandle, show: bool) {
    let win = app.get_webview_window("main").expect("main window");
    let _ = if show { win.show() } else { win.hide() }; // hidden, the page gets no animation frames and stops drawing
    // WebView2 doesn't hear that its window hid: unless told, the page stays "visible" and keeps drawing
    #[cfg(target_os = "windows")]
    let _ = win.with_webview(move |w| unsafe {
        let _ = w.controller().SetIsVisible(show);
    });
    app.state::<Shown>().set(show);
    // and the page hears it from the app too, wherever document.hidden doesn't follow the window
    let _ = win.emit("shown", show);
    if let Some(t) = app.state::<State>().tray.lock().unwrap().as_ref() {
        let _ = t.show.set_text(if show { "Hide Planet" } else { "Show Planet" });
    }
}

#[tauri::command]
fn hide_widget(app: AppHandle) {
    show_widget(&app, false);
}

#[tauri::command]
async fn open_settings(app: AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("settings") {
        w.show()?;
        return w.set_focus();
    }
    let w = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
        .title("settings")
        .inner_size(380.0, 883.0)
        .decorations(false) // the page draws its own slim title bar with a close button
        .transparent(app.state::<State>().transparent)
        .shadow(true)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .center()
        .build()?;
    w.set_focus()
}

#[tauri::command]
fn quit(app: AppHandle) {
    let _ = app.save_window_state(StateFlags::POSITION);
    app.exit(0);
}

// widget.js says whether it's hidden and stepping the game headless; with POWERRR_SMOKE=<file> (the CI smoke tests)
// that goes to the file, and the widget hides itself 8 s after launch
#[tauri::command]
fn page_state(hidden: bool, away: bool, ticking: bool) {
    smoke(&format!("page hidden={hidden} away={away} ticking={ticking}"));
}

fn smoke(line: &str) {
    use std::io::Write;
    let Some(f) = std::env::var_os("POWERRR_SMOKE") else { return };
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(f) {
        let _ = writeln!(f, "{line}");
    }
}

// a hidden widget keeps playing if anything shows it: a notification or the power level in the menu bar (widget.js
// decides the same for game.js)
#[cfg(feature = "rust-core")]
fn plays_hidden(app: &AppHandle) -> bool {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    s["menubar_power"].as_bool().unwrap_or(true) || s["notify"].as_object().is_some_and(|n| n.values().any(|v| v.as_bool() == Some(true)))
}

// a hidden catch-up of the game on the Rust core: the menu-bar power level and notifications, as widget.js would
#[cfg(feature = "rust-core")]
fn hidden_tick(app: &AppHandle, caught: planet::Caught) {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    let power = planet::short(caught.pl);
    if s["menubar_power"].as_bool().unwrap_or(true) && *app.state::<State>().power.lock().unwrap() != power {
        tray_title(app.clone(), power);
    }
    for (kind, arg) in caught.notes {
        notify_hidden(app, &s, kind, &arg);
    }
}

// widget.js's notifications: each kind its own toggle, none in quiet hours, at most one a minute per kind
#[cfg(feature = "rust-core")]
fn notify_hidden(app: &AppHandle, s: &Value, kind: &'static str, arg: &str) {
    use chrono::Timelike;
    let mins = |hm: &Value| {
        let hm = hm.as_str().unwrap_or("");
        let mut p = hm.split(':').map(|x| x.trim().parse::<u32>().unwrap_or(0));
        p.next().unwrap_or(0) * 60 + p.next().unwrap_or(0)
    };
    let q = &s["quiet"];
    if q["on"].as_bool().unwrap_or(false) {
        let t = chrono::Local::now();
        let (now, a, b) = (t.hour() * 60 + t.minute(), mins(&q["from"]), mins(&q["to"]));
        if if a <= b { now >= a && now < b } else { now >= a || now < b } {
            return;
        }
    }
    if !s["notify"][kind].as_bool().unwrap_or(false) {
        return;
    }
    let (title, body) = match kind {
        "raid" => (format!("{arg} is attacking"), "tap him before he gets away"),
        "shenron" => ((if arg == "porunga" { "porunga awakens" } else { "the eternal dragon awakens" }).into(), "make your wish"),
        "trip" => (format!("off to {arg}"), "a space pod takes you there"),
        "pod" => ("a saiyan space pod".into(), "tap it for ×2"),
        "ascend" => ("you can ascend".into(), "reset for ki seeds and a lasting boost"),
        _ => return,
    };
    let state = app.state::<State>();
    let mut sent = state.sent.lock().unwrap();
    if sent.get(kind).is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
        return;
    }
    sent.insert(kind, std::time::Instant::now());
    drop(sent);
    notify(app.clone(), title, body.into());
}

#[tauri::command]
fn notify(app: AppHandle, title: String, body: String) {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        notes::post(title, body);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app.notification().builder().title(title).body(body).show();
}

// "granted", "denied", "ask" or "unavailable"
#[tauri::command]
async fn notify_permission() -> &'static str {
    #[cfg(target_os = "macos")]
    return notes::permission();
    #[cfg(not(target_os = "macos"))]
    "granted"
}

// asks macOS once; the answer arrives as a "permission" event
#[tauri::command]
fn request_notify_permission() {
    #[cfg(target_os = "macos")]
    notes::request(|| {});
}

// once denied, only System Settings can turn them back on. elsewhere permission is always "granted", so it's never offered
#[tauri::command]
fn open_notify_settings(app: AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let id = &app.config().identifier;
        let _ = std::process::Command::new("open").arg(format!("x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={id}")).spawn();
    }
    let _ = app;
}

fn main() {
    let app = tauri::Builder::default().manage(Shown::new());
    // macOS reopens the running app itself (RunEvent::Reopen below); elsewhere a second launch would be a second
    // widget and a second tray icon, so it shows the first one's widget instead and exits
    #[cfg(not(target_os = "macos"))]
    let app = app.plugin(tauri_plugin_single_instance::init(|app, _, _| show_widget(app, true)));
    let app = app
        .plugin(tauri_plugin_window_state::Builder::new().with_state_flags(StateFlags::POSITION).build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build());
    #[cfg(not(target_os = "macos"))]
    let app = app.plugin(tauri_plugin_notification::init());
    macro_rules! handler {
        ($($m:ident)::+) => {
            $($m)::+![get_settings, set_settings, tray_title, hide_widget, open_settings, quit, notify, notify_permission, request_notify_permission, open_notify_settings, page_state]
        };
    }
    #[cfg(not(feature = "rust-core"))]
    let app = app.invoke_handler(handler!(tauri::generate_handler));
    #[cfg(feature = "rust-core")]
    let app = app.invoke_handler(handler!(planet_commands));
    app.setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // no dock icon: it lives in the menu bar
            let file = app.path().app_config_dir()?.join("settings.json");
            let mut settings: Value = serde_json::from_str(DEFAULTS).unwrap();
            #[cfg(not(target_os = "macos"))]
            {
                settings["shortcut"] = json!(SHORTCUT);
            }
            let saved = fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            if let Some(saved) = &saved {
                merge(&mut settings, saved);
            }
            if ![7, 1, 2, 3, 6].contains(&settings["layout"].as_u64().unwrap_or(0)) {
                settings["layout"] = json!(7); // layouts 4 and 5 were dropped
            }
            let main = app.get_webview_window("main").expect("main window");
            let _ = main.set_title(&app.package_info().name);
            let (transparent, wayland) = (can_be_transparent(&main), wayland(&main));
            app.manage(State { settings: Mutex::new(settings.clone()), file, tray: Mutex::new(None), power: Mutex::new(String::new()), transparent, wayland, tray_nudge: Mutex::new(None), #[cfg(feature = "rust-core")] sent: Mutex::default() });
            tray(app.handle())?;
            apply(app.handle(), &settings, true);
            // global-hotkey is an X11 key grab: on Wayland it only fires while an XWayland window has focus, so it isn't
            // bound at all, and settings says to use a desktop shortcut instead
            if wayland {
                eprintln!("wayland: no global show/hide shortcut");
            } else if let Err(e) = bind(app.handle(), "", settings["shortcut"].as_str().unwrap_or("")) {
                eprintln!("show/hide shortcut not bound: {e}");
            }
            #[cfg(target_os = "macos")]
            notes::init(app.handle());
            // the game runs here, and the window shows only its HUD (dist/hud.html, npm run page:hud)
            #[cfg(feature = "rust-core")]
            {
                app.manage(planet::Core::new(planet::seed(), app.path().app_data_dir()?.join("power.json")));
                main.navigate(main.url()?.join("hud.html")?)?;
                planet::run(app.handle().clone(), planet::Hooks { idle_fps: fps, plays_hidden, hidden: hidden_tick });
            }
            if !wayland {
                watch_pointer(app.handle().clone());
            }
            // a hidden setting, read straight from the file: it isn't in DEFAULTS, so settings never save it
            #[cfg(all(target_os = "macos", feature = "native-renderer"))]
            if let Some(src) = saved.as_ref().and_then(native::wanted) {
                native::start(app.handle(), src, fps)?;
            }
            if std::env::var_os("POWERRR_SMOKE").is_some() {
                let app = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(8));
                    let h = app.clone();
                    let _ = app.run_on_main_thread(move || show_widget(&h, false));
                    smoke("hid the widget");
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("power planet failed to start")
        .run(|app, e| {
            // opening the app again (Finder, Spotlight, open -a) brings the widget back: no menu bar needed
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = e {
                show_widget(app, true);
            }
            #[cfg(feature = "rust-core")]
            if let tauri::RunEvent::Exit = e {
                app.state::<planet::Core>().save();
            }
            let _ = (app, e);
        });
}

// the frame-rate setting: 15, 30 (the default) or 60
#[cfg_attr(not(any(feature = "rust-core", all(target_os = "macos", feature = "native-renderer"))), allow(dead_code))]
fn fps(app: &AppHandle) -> f64 {
    app.state::<State>().settings.lock().unwrap()["fps"].as_f64().filter(|f| [15.0, 30.0, 60.0].contains(f)).unwrap_or(30.0)
}

// on X11 with no compositing manager, transparent pixels come out black: the widget then sticks to window mode
#[cfg(target_os = "linux")]
fn can_be_transparent(w: &tauri::WebviewWindow) -> bool {
    use gtk::prelude::WidgetExt;
    let ok = w.gtk_window().ok().and_then(|g| WidgetExt::screen(&g)).is_some_and(|s| s.is_composited() && s.rgba_visual().is_some());
    if !ok {
        eprintln!("no compositor: transparency is off, the widget stays in window mode");
    }
    ok
}
#[cfg(not(target_os = "linux"))]
fn can_be_transparent(_: &tauri::WebviewWindow) -> bool {
    true
}

// Wayland tells no app where the pointer or its own window is (tao reports 0,0), so the poll below would always see
// the pointer "inside". there, as on X11, an unfocused window still gets pointer moves, and the page hears them itself
#[cfg(target_os = "linux")]
fn wayland(w: &tauri::WebviewWindow) -> bool {
    use gtk::{glib::object::ObjectExt, prelude::WidgetExt};
    w.gtk_window().ok().map(|g| g.display().type_().name().contains("Wayland")).unwrap_or(false)
}
#[cfg(not(target_os = "linux"))]
fn wayland(_: &tauri::WebviewWindow) -> bool {
    false
}

// an inactive window gets no mouse-moved events, so the page couldn't tell it's being hovered: tell it, 8 times a second
// while it shows
fn watch_pointer(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            app.state::<Shown>().wait(); // hidden, nothing to hover: no polling at all
            std::thread::sleep(Duration::from_millis(120));
            if !app.state::<Shown>().get() {
                continue;
            }
            let Some(w) = app.get_webview_window("main") else { continue };
            let (Ok(c), Ok(p), Ok(s), Ok(f)) = (app.cursor_position(), w.outer_position(), w.outer_size(), w.scale_factor()) else { continue };
            let (x, y) = ((c.x - p.x as f64) / f, (c.y - p.y as f64) / f);
            let inside = x >= 0.0 && y >= 0.0 && x < s.width as f64 / f && y < s.height as f64 / f;
            let now = inside.then_some((x.round() as i32, y.round() as i32));
            if now != last {
                let _ = w.emit("pointer", json!({ "inside": inside, "x": x, "y": y }));
                last = now;
            }
        }
    });
}

fn tray(app: &AppHandle) -> tauri::Result<()> {
    let none = None::<&str>;
    let show = MenuItem::with_id(app, "show", "Hide Planet", true, none)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, none)?;
    let top = CheckMenuItem::with_id(app, "top", "Always on Top", true, true, none)?;
    let sizes = SIZES
        .iter()
        .map(|(id, label, _)| CheckMenuItem::with_id(app, *id, *label, true, false, none))
        .collect::<tauri::Result<Vec<_>>>()?;
    let size = Submenu::with_items(app, "Size", true, &[&sizes[0], &sizes[1], &sizes[2]])?;
    let login = CheckMenuItem::with_id(app, "login", "Launch at Login", true, false, none)?;
    let name = &app.package_info().name;
    let quit_item = MenuItem::with_id(app, "quit", format!("Quit {name}"), true, none)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &settings, &sep, &top, &size, &login, &sep, &quit_item])?;

    // the scouter is a black template image that macOS tints; Windows and Linux don't, so there it's black or white to
    // match the taskbar (or as picked in settings), and follows it when the theme changes
    #[cfg(target_os = "macos")]
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = {
        let taskbar = Taskbar::new();
        let white = white_scouter(app, &taskbar);
        watch_taskbar(app.clone(), taskbar, white);
        scouter(white)?
    };
    TrayIconBuilder::with_id("tray")
        .icon(icon)
        .icon_as_template(true)
        .tooltip(name)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let app = app.clone();
            let id = event.id().as_ref().to_string();
            match id.as_str() {
                "show" => {
                    let visible = app.get_webview_window("main").map(|w| w.is_visible().unwrap_or(true)).unwrap_or(true);
                    show_widget(&app, !visible);
                }
                "settings" => {
                    tauri::async_runtime::spawn(async move {
                        let _ = open_settings(app).await;
                    });
                }
                "quit" => quit(app),
                "top" => {
                    let on = !app.state::<State>().settings.lock().unwrap()["on_top"].as_bool().unwrap_or(true);
                    let _ = set_settings(app, json!({ "on_top": on }));
                }
                "login" => {
                    let on = !app.autolaunch().is_enabled().unwrap_or(false);
                    let _ = set_settings(app, json!({ "login": on }));
                }
                size => {
                    let _ = set_settings(app, json!({ "size": size }));
                }
            }
        })
        .build(app)?;
    *app.state::<State>().tray.lock().unwrap() = Some(Tray {
        show,
        top,
        sizes,
        login,
        #[cfg(target_os = "linux")]
        menu,
        #[cfg(target_os = "linux")]
        power: MenuItem::with_id(app, "power", "", false, none)?,
    });
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn scouter(white: bool) -> tauri::Result<Image<'static>> {
    let png: &[u8] = if white { include_bytes!("../icons/tray-white.png") } else { include_bytes!("../icons/tray.png") };
    Image::from_bytes(png)
}

// the tray icon setting: "light" is the white scouter (for dark taskbars), "dark" the black one, "auto" the taskbar's theme
#[cfg(not(target_os = "macos"))]
fn white_scouter(app: &AppHandle, taskbar: &Taskbar) -> bool {
    let pick = app.state::<State>().settings.lock().unwrap()["tray_icon"].as_str().unwrap_or("auto").to_string();
    match pick.as_str() {
        "light" => true,
        "dark" => false,
        _ => taskbar.dark(),
    }
}

// every few seconds, and at once when settings change, swaps the scouter if it should change. only "auto" asks the
// system (cheap: a registry value, or one D-Bus call; no event covers the Windows taskbar alone)
#[cfg(not(target_os = "macos"))]
fn watch_taskbar(app: AppHandle, taskbar: Taskbar, mut white: bool) {
    let (nudge, woken) = std::sync::mpsc::channel();
    *app.state::<State>().tray_nudge.lock().unwrap() = Some(nudge);
    std::thread::spawn(move || loop {
        let _ = woken.recv_timeout(std::time::Duration::from_secs(3));
        let now = white_scouter(&app, &taskbar);
        if now != white {
            if let (Some(tray), Ok(icon)) = (app.tray_by_id("tray"), scouter(now)) {
                if tray.set_icon(Some(icon)).is_ok() {
                    white = now;
                }
            }
        }
    });
}

// Windows keeps the taskbar's theme apart from the apps' (a dark taskbar with light apps is Windows 10's default)
#[cfg(target_os = "windows")]
struct Taskbar;
#[cfg(target_os = "windows")]
impl Taskbar {
    fn new() -> Self {
        Taskbar
    }
    fn dark(&self) -> bool {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let light = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .and_then(|k| k.get_value::<u32, _>("SystemUsesLightTheme"));
        light.unwrap_or(0) == 0 // no value: a Windows 10 from before light taskbars
    }
}

// Linux: the freedesktop settings portal's color scheme, where there is one (live); else the GTK theme's name, read once
// at startup on the main thread (GTK lives there). With no preference, white: most panels are dark (GNOME's top bar is)
#[cfg(target_os = "linux")]
struct Taskbar {
    bus: Option<zbus::blocking::Connection>,
    gtk_dark: Option<bool>,
}
#[cfg(target_os = "linux")]
impl Taskbar {
    fn new() -> Self {
        use gtk::glib::object::ObjectExt;
        let theme = gtk::Settings::default().and_then(|s| s.property::<Option<String>>("gtk-theme-name"));
        Taskbar { bus: zbus::blocking::Connection::session().ok(), gtk_dark: theme.map(|t| t.to_lowercase().contains("dark")) }
    }
    fn dark(&self) -> bool {
        match self.bus.as_ref().and_then(color_scheme) {
            Some(2) => false, // prefers light
            Some(_) => true,  // 1 prefers dark, 0 no preference
            None => self.gtk_dark.unwrap_or(true),
        }
    }
}

// org.freedesktop.appearance color-scheme: ReadOne, or Read (older portals) which wraps it in one more variant
#[cfg(target_os = "linux")]
fn color_scheme(bus: &zbus::blocking::Connection) -> Option<u32> {
    use zbus::zvariant::{OwnedValue, Value};
    let read = |method: &str| -> Option<OwnedValue> {
        let reply = bus
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                method,
                &("org.freedesktop.appearance", "color-scheme"),
            )
            .ok()?;
        reply.body().deserialize::<OwnedValue>().ok()
    };
    let v = read("ReadOne").or_else(|| read("Read"))?;
    match &*v {
        Value::U32(n) => Some(*n),
        Value::Value(inner) => match **inner {
            Value::U32(n) => Some(n),
            _ => None,
        },
        _ => None,
    }
}
