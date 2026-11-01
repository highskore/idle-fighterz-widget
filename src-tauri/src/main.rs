#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window behind the widget on Windows
// The power planet as a desktop pet: one frameless, transparent window showing the bundled page, a settings
// window, and a menu-bar icon. Settings are app config in a JSON file; the game itself is never saved.
use serde_json::{json, Value};
use std::{fs, path::PathBuf, sync::{mpsc::Sender, Condvar, Mutex}, time::Duration};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

mod focus;
mod place;
mod popover;
mod prefs;
mod system;
#[cfg(target_os = "macos")]
mod notes;
#[cfg(all(target_os = "macos", feature = "native-renderer", feature = "rust-core"))]
mod native;
#[cfg(feature = "rust-core")]
mod photo;
#[cfg(feature = "rust-core")]
#[macro_use]
mod planet;
#[cfg(feature = "rust-core")]
mod recap;
mod screensaver;
#[cfg(feature = "rust-core")]
mod spectacle;
mod update;

// every setting and its default; a saved file only overrides keys listed here
const DEFAULTS: &str = r#"{
  "on_top": true, "size": 400, "opacity": 1, "layout": 7, "hud": "hover", "clouds": "hover", "cloud_shape": "c", "cloud_amount": 2, "mode": "floating", "menubar_power": true, "fps": 30, "tray_icon": "auto", "update_check": true,
  "notify": { "raid": true, "shenron": true, "trip": true, "pod": false, "ascend": true },
  "quiet": { "on": false, "from": "22:00", "to": "08:00" },
  "ghost": false, "auto_hide": true, "battery_auto": true, "raid_windows": false, "screen_beam": false,
  "follow_focus": true, "snooze_until": 0, "recap": true, "sound": null, "profiles": [], "skin": "classic", "theme": "light",
  "ki_buttons": "meter", "shortcut": "Alt+Cmd+KeyP",
  "settings_style": "compact", "tray_style": "menu", "onboarded": false
}"#;
#[cfg(not(target_os = "macos"))]
const SHORTCUT: &str = "Ctrl+Alt+KeyP"; // Win+Alt+P / Super+Alt+P belong to the desktop; ⌥⌘P stays the macOS default

// menu handles, cloned out of `State::tray` before any call on them: off the main thread a menu call waits for the main
// thread, which may be waiting for that lock (on Linux the shortcut and second-launch callbacks run off it)
#[derive(Clone)]
struct Tray {
    show: MenuItem<Wry>,
    top: CheckMenuItem<Wry>,
    ghost: CheckMenuItem<Wry>,
    sizes: Vec<CheckMenuItem<Wry>>,
    profiles: Submenu<Wry>,
    snooze: Submenu<Wry>,
    resume: MenuItem<Wry>,
    login: CheckMenuItem<Wry>,
    menu: Menu<Wry>,
    update: MenuItem<Wry>, // "Update to X.Y.Z", in the menu only while there is one
    check: MenuItem<Wry>,
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
    tray_profiles: Mutex<Option<Vec<String>>>, // the profile names the tray lists now
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
    /// the count of changes so far, for `wait_change`
    #[cfg_attr(not(feature = "rust-core"), allow(dead_code))]
    fn changes(&self) -> u64 {
        self.on.lock().unwrap().1
    }
    /// blocks until the widget shows or hides or the settings change, if none has since `changes` said `n`
    #[cfg_attr(not(feature = "rust-core"), allow(dead_code))]
    fn wait_change(&self, n: u64) {
        drop(self.changed.wait_while(self.on.lock().unwrap(), |s| s.1 == n));
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

// the defaults with a saved file over them
fn initial_settings(saved: Option<&Value>) -> Value {
    let mut settings: Value = serde_json::from_str(DEFAULTS).unwrap();
    #[cfg(not(target_os = "macos"))]
    {
        settings["shortcut"] = json!(SHORTCUT);
    }
    // X11 only, and best effort: off unless picked
    #[cfg(target_os = "linux")]
    {
        settings["auto_hide"] = json!(false);
    }
    if let Some(saved) = saved {
        merge(&mut settings, &prefs::migrate(saved));
    }
    if !prefs::LAYOUTS.contains(&settings["layout"].as_u64().unwrap_or(0)) {
        settings["layout"] = json!(7); // layouts 4 and 5 were dropped
    }
    prefs::known_look(&mut settings);
    settings
}

// to a temp file renamed into place, so a crash or a full disk mid-write leaves the old file whole
fn save(file: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    let done = fs::write(&tmp, bytes).and_then(|_| fs::rename(&tmp, file));
    if done.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    done
}

fn merge(base: &mut Value, over: &Value) {
    let (Some(b), Some(o)) = (base.as_object_mut(), over.as_object()) else { return };
    for (k, v) in o {
        match b.get_mut(k) {
            Some(bv) if bv.is_object() => merge(bv, v),
            Some(bv) if bv.is_null() => *bv = v.clone(), // a default of null takes any value (sound: the page's own until set)
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
    // the frame rate the widget draws at, and the one picked in settings
    let pick = s["fps"].as_f64().unwrap_or(30.0);
    let rate = system::frame_rate(pick, s["battery_auto"].as_bool().unwrap_or(true), app.state::<system::Sys>().saving.load(std::sync::atomic::Ordering::Relaxed));
    s["fps_pick"] = s["fps"].take();
    s["fps"] = json!(rate as u64);
    s["battery_saving"] = json!(rate != pick);
    s["size"] = json!(prefs::size(&s["size"]) as u64);
    s["profile"] = json!(prefs::active(&s));
    let left = (s["snooze_until"].as_f64().unwrap_or(0.0) - now_ms()) / 1000.0;
    s["snooze_left"] = json!(if left > 0.0 { focus::left(left as i64) } else { String::new() });
    s["photo"] = json!(photo_ready(app));
    s["version"] = json!(app.package_info().version.to_string());
    s["popover_ok"] = json!(cfg!(not(target_os = "linux")));
    #[cfg(feature = "rust-core")]
    let spectacle = spectacle::AVAILABLE && photo_ready(app);
    #[cfg(not(feature = "rust-core"))]
    let spectacle = false;
    s["spectacle_ok"] = json!(spectacle);
    s
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

// photo mode needs the game on the Rust core
fn photo_ready(app: &AppHandle) -> bool {
    #[cfg(feature = "rust-core")]
    return app.try_state::<planet::Core>().is_some();
    #[cfg(not(feature = "rust-core"))]
    {
        let _ = app;
        false
    }
}

// the pages hear the settings again, for a change that isn't theirs (the battery saver)
fn publish(app: &AppHandle) {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    let _ = app.emit("settings", with_login(app, s));
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
    let mut settings = settings;
    if settings.get("size").is_some() {
        settings["size"] = json!(prefs::size(&settings["size"]) as u64);
    }
    prefs::vet(&mut settings);
    let state = app.state::<State>();
    let was_style = state.settings.lock().unwrap()["settings_style"].clone();
    let s = {
        let mut cur = state.settings.lock().unwrap();
        merge(&mut cur, &settings);
        let _ = save(&state.file, &serde_json::to_vec_pretty(&*cur).unwrap());
        cur.clone()
    };
    if let Some(on) = settings.get("login").and_then(Value::as_bool) {
        let auto = app.autolaunch();
        let _ = if on { auto.enable() } else { auto.disable() };
    }
    apply(&app, &s, settings.get("size").is_some() || settings.get("mode").is_some());
    if settings.get("snooze_until").is_some() {
        if let Some(nudge) = app.try_state::<SnoozeNudge>() {
            nudge.0.notify_all();
        }
    }
    app.state::<Shown>().poke();
    if s["settings_style"] != was_style {
        restyle_settings(&app, &s["settings_style"]);
    }
    // "show the intro again": the card is on the widget, so the widget shows
    if settings.get("onboarded") == Some(&json!(false)) {
        show_widget(&app, true);
    }
    // only a new pick: on Linux every check is a D-Bus call, and a slider drag sends a burst of updates
    if settings.get("tray_icon").is_some() {
        if let Some(nudge) = app.state::<State>().tray_nudge.lock().unwrap().as_ref() {
            let _ = nudge.send(());
        }
    }
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
    let Some(win) = app.get_webview_window("main") else { return };
    let on_top = s["on_top"].as_bool().unwrap_or(true);
    let side = prefs::size(&s["size"]);
    let _ = win.set_always_on_top(on_top);
    if resize {
        let _ = win.set_size(LogicalSize::new(side, side));
        place::settle(app, false); // a bigger widget near an edge stays on screen
    }
    // window mode is an opaque card: it casts a shadow and can be resized freely; floating, it's neither
    let windowed = s["mode"] == "window" || !app.state::<State>().transparent;
    let _ = win.set_shadow(windowed);
    let _ = win.set_resizable(windowed);
    let tray = app.state::<State>().tray.lock().unwrap().clone();
    if let Some(t) = tray {
        let _ = t.top.set_checked(on_top);
        let _ = t.ghost.set_checked(s["ghost"].as_bool().unwrap_or(false));
        let _ = t.ghost.set_enabled(!windowed);
        for (item, snap) in t.sizes.iter().zip(prefs::SNAPS) {
            let _ = item.set_checked(side == snap.2);
        }
        let _ = t.login.set_checked(app.autolaunch().is_enabled().unwrap_or(false));
        tray_profiles(app, &t, s);
    }
    // the popover opens on a left click; right-click (and ⌃-click) keeps the menu
    let pop = prefs::popover(&s["tray_style"]);
    if let Some(tray) = app.tray_by_id("tray") {
        let _ = tray.set_show_menu_on_left_click(!pop);
    }
    if !pop {
        popover::hide(app);
    }
    show_power(app, s);
}

// the tray's profiles: rebuilt when their names change, the one the settings match checked
fn tray_profiles(app: &AppHandle, t: &Tray, s: &Value) {
    let names = prefs::names(s);
    let active = prefs::active(s);
    // not held over the menu calls (see Tray): set_settings may get here off the main thread
    let stale = app.state::<State>().tray_profiles.lock().unwrap().replace(names.clone()).as_ref() != Some(&names);
    if stale {
        if let Ok(items) = t.profiles.items() {
            for item in items {
                let _ = t.profiles.remove(&item);
            }
        }
        let none = None::<&str>;
        for (k, name) in names.iter().enumerate() {
            if let Ok(item) = CheckMenuItem::with_id(app, format!("profile:{k}"), name, true, false, none) {
                let _ = t.profiles.append(&item);
            }
        }
        if names.is_empty() {
            if let Ok(item) = MenuItem::with_id(app, "profile:none", "No Profiles Yet", false, none) {
                let _ = t.profiles.append(&item);
            }
        }
        if let (Ok(sep), Ok(item)) = (PredefinedMenuItem::separator(app), MenuItem::with_id(app, "settings", "Save or Manage in Settings…", true, none)) {
            let _ = t.profiles.append(&sep);
            let _ = t.profiles.append(&item);
        }
    }
    if let Ok(items) = t.profiles.items() {
        for (item, name) in items.iter().filter_map(|i| i.as_check_menuitem()).zip(&names) {
            let _ = item.set_checked(active.as_ref() == Some(name));
        }
    }
}

// wakes the snooze ticker when a snooze starts or ends
struct SnoozeNudge(Condvar, Mutex<()>);

// the snooze in the tray: how long it has left (every 30 s), and resume; at its end, notifications are back
fn watch_snooze(app: AppHandle) {
    app.manage(SnoozeNudge(Condvar::new(), Mutex::new(())));
    std::thread::spawn(move || {
        let mut shown = String::new();
        loop {
            let until = app.state::<State>().settings.lock().unwrap()["snooze_until"].as_f64().unwrap_or(0.0);
            let left = (until - now_ms()) / 1000.0;
            if until > 0.0 && left <= 0.0 {
                let _ = set_settings(app.clone(), json!({ "snooze_until": 0 }));
                continue;
            }
            let text = if left > 0.0 { format!("Notifications Snoozed · {} left", focus::left(left as i64)) } else { "Snooze Notifications".into() };
            if text != shown {
                let tray = app.state::<State>().tray.lock().unwrap().clone();
                if let Some(t) = tray {
                    let _ = t.snooze.set_text(&text);
                    let _ = t.resume.set_enabled(left > 0.0);
                }
                shown = text;
                publish(&app); // and settings shows the time left too
            }
            let nudge = app.state::<SnoozeNudge>();
            let wait = if left > 0.0 { Duration::from_secs_f64(left.clamp(0.5, 30.0)) } else { Duration::from_secs(3600) };
            drop(nudge.0.wait_timeout(nudge.1.lock().unwrap(), wait));
        }
    });
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
    let Some(win) = app.get_webview_window("main") else { return };
    let _ = if show { win.show() } else { win.hide() }; // hidden, the page gets no animation frames and stops drawing
    // WebView2 doesn't hear that its window hid: unless told, the page stays "visible" and keeps drawing
    #[cfg(target_os = "windows")]
    let _ = win.with_webview(move |w| unsafe {
        let _ = w.controller().SetIsVisible(show);
    });
    app.state::<Shown>().set(show);
    app.state::<system::Sys>().auto_hidden.store(false, std::sync::atomic::Ordering::Relaxed); // shown or hidden by hand
    // and the page hears it from the app too, wherever document.hidden doesn't follow the window
    let _ = win.emit("shown", show);
    #[cfg(feature = "rust-core")]
    if !show && app.try_state::<photo::Photo>().is_some() {
        photo::exit(app);
    }
    let item = app.state::<State>().tray.lock().unwrap().as_ref().map(|t| t.show.clone());
    if let Some(item) = item {
        let _ = item.set_text(if show { "Hide Planet" } else { "Show Planet" });
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
    let style = app.state::<State>().settings.lock().unwrap()["settings_style"].clone();
    let (page, width, height) = prefs::settings_page(&style);
    let w = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App(page.into()))
        .title("settings")
        .inner_size(width, height)
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

// the settings style changed: an open settings window becomes the other one, resized and centred
fn restyle_settings(app: &AppHandle, style: &Value) {
    let Some(w) = app.get_webview_window("settings") else { return };
    let (page, width, height) = prefs::settings_page(style);
    let Ok(url) = w.url().and_then(|u| u.join(page).map_err(|e| tauri::Error::Anyhow(e.into()))) else { return };
    let _ = w.set_size(LogicalSize::new(width, height));
    // centred by its new size: center() may still see the old one
    if let Ok(Some(m)) = w.current_monitor() {
        let (a, k) = (m.work_area(), m.scale_factor());
        let x = a.position.x as f64 + (a.size.width as f64 - width * k).max(0.0) / 2.0;
        let y = a.position.y as f64 + (a.size.height as f64 - height * k).max(0.0) / 2.0;
        let _ = w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }
    let _ = w.navigate(url);
}

#[tauri::command]
fn quit(app: AppHandle) {
    let _ = app.save_window_state(StateFlags::POSITION);
    app.exit(0);
}

// widget.js says whether it's hidden and stepping the game headless; with IDLE_FIGHTERZ_SMOKE=<file> (the CI smoke tests)
// that goes to the file, and the widget hides itself 8 s after launch
#[tauri::command]
fn page_state(hidden: bool, away: bool, ticking: bool) {
    smoke(&format!("page hidden={hidden} away={away} ticking={ticking}"));
}

fn smoke(line: &str) {
    use std::io::Write;
    let Some(f) = std::env::var_os("IDLE_FIGHTERZ_SMOKE") else { return };
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
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| smoke("hidden: the core plays on"));
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
    if focus::quiet_now(&s["quiet"]) {
        return;
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

// every notification, from the page or the hidden game: held back while snoozed, or while the OS is in Focus / Do Not
// Disturb with that setting on. The Focus probe may take a D-Bus call or a file read: off the caller's thread
#[tauri::command]
fn notify(app: AppHandle, title: String, body: String) {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    let (follow, until) = (s["follow_focus"].as_bool().unwrap_or(true), s["snooze_until"].as_f64().unwrap_or(0.0));
    std::thread::spawn(move || {
        let focus = if follow { focus::probe() } else { None };
        if focus::hold(follow, focus, until, now_ms()) {
            smoke(&format!("notification held: {title} (focus={focus:?}, snoozed={})", focus::snoozed(until, now_ms())));
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let _ = app;
            notes::post(title, body);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = app.notification().builder().title(title).body(body).show();
    });
}

// is the OS in Focus / Do Not Disturb now: true, false, or null where it can't be told without a permission
#[tauri::command]
async fn focus_status() -> Option<bool> {
    focus::probe()
}

// "30", "60", "180", "tomorrow", or "off" to resume
#[tauri::command]
fn snooze(app: AppHandle, choice: String) -> Result<(), String> {
    let until = if choice == "off" {
        0.0
    } else {
        let now = chrono::Local::now();
        let end = focus::snooze_end(now.naive_local(), &choice).ok_or("no such snooze")?;
        let end = end.and_local_timezone(chrono::Local).earliest().ok_or("no such local time")?;
        end.timestamp_millis() as f64
    };
    smoke(&format!("snooze: {choice}"));
    set_settings(app, json!({ "snooze_until": until }))
}

#[tauri::command]
fn save_profile(app: AppHandle, name: String) -> Result<(), String> {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    set_settings(app, json!({ "profiles": prefs::save(&s, &name)? }))
}

#[tauri::command]
fn delete_profile(app: AppHandle, name: String) -> Result<(), String> {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    set_settings(app, json!({ "profiles": prefs::delete(&s, &name) }))
}

#[tauri::command]
fn apply_profile(app: AppHandle, name: String) -> Result<(), String> {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    let p = prefs::find(&s, &name).ok_or("no such profile")?;
    smoke(&format!("profile: {name}"));
    set_settings(app, p)
}

// the game's sound, switched in the widget: kept with the settings so a profile can carry it
#[tauri::command]
fn set_sound(app: AppHandle, on: bool) -> Result<(), String> {
    if app.state::<State>().settings.lock().unwrap()["sound"] == json!(on) {
        return Ok(());
    }
    set_settings(app, json!({ "sound": on }))
}

#[tauri::command]
fn photo(app: AppHandle) {
    #[cfg(feature = "rust-core")]
    photo::enter(&app);
    let _ = app;
}

#[tauri::command]
fn photo_exit(app: AppHandle) {
    #[cfg(feature = "rust-core")]
    photo::exit(&app);
    let _ = app;
}

// The game at a glance for the popover and the settings preview: the menu-bar power level, and on the Rust core the HUD,
// the upgrades ready and the world; with the tray's own state (shown, login, an update) for the popover's items
#[tauri::command]
fn glance(app: AppHandle) -> Value {
    #[cfg(feature = "rust-core")]
    let game = app.try_state::<planet::Core>().and_then(|c| c.glance()).and_then(|g| serde_json::from_str::<Value>(&g).ok());
    #[cfg(not(feature = "rust-core"))]
    let game: Option<Value> = None;
    let shown = app.state::<Shown>().get();
    let tray = app.state::<State>().tray.lock().unwrap().clone();
    let update = tray.filter(|t| t.menu.get(t.update.id()).is_some() && t.update.is_enabled().unwrap_or(false)).and_then(|t| t.update.text().ok());
    json!({
        "power": app.state::<State>().power.lock().unwrap().clone(),
        "game": game,
        "shown": shown,
        "update": update,
        "screensaver": screensaver::bundled(&app).is_some(),
    })
}

// The widget's last frame for the settings preview: its width and height (u32, little-endian), then RGBA. Empty while
// the game runs in the web view (no frames here)
#[tauri::command]
fn preview_frame(app: AppHandle) -> tauri::ipc::Response {
    #[cfg(feature = "rust-core")]
    if let Some((rgba, w, h, _)) = app.try_state::<planet::Core>().and_then(|c| c.frame()) {
        let mut out = Vec::with_capacity(8 + rgba.len());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&rgba);
        return tauri::ipc::Response::new(out);
    }
    let _ = app;
    tauri::ipc::Response::new(Vec::new())
}

// the popover's items do what the tray menu's do, by the same ids; then it closes
#[tauri::command]
fn tray_action(app: AppHandle, id: String) {
    popover::hide(&app);
    menu_action(app, &id);
}

// the first-run card, finished or skipped: it doesn't show again (until settings' "show the intro again")
#[tauri::command]
fn intro_done(app: AppHandle) -> Result<(), String> {
    smoke("intro done");
    set_settings(app, json!({ "onboarded": true }))
}

#[tauri::command]
fn hide_popover(app: AppHandle) {
    popover::hide(&app);
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
    // release builds abort on a panic, with no console on Windows: the smoke log says where
    if std::env::var_os("IDLE_FIGHTERZ_SMOKE").is_some() {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            smoke(&format!("panic: {info}"));
            default(info);
        }));
    }
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
            $($m)::+![get_settings, set_settings, tray_title, hide_widget, open_settings, quit, notify, notify_permission, request_notify_permission, open_notify_settings, page_state, focus_status, snooze, save_profile, delete_profile, apply_profile, set_sound, photo, photo_exit, glance, preview_frame, tray_action, hide_popover, intro_done]
        };
    }
    #[cfg(not(feature = "rust-core"))]
    let app = app.invoke_handler(handler!(tauri::generate_handler));
    #[cfg(feature = "rust-core")]
    let app = app.invoke_handler(handler!(planet_commands));
    // every move of the widget wakes the snap watcher, which acts once the drag is over
    let app = app.on_window_event(|w, e| match (e, w.label()) {
        (tauri::WindowEvent::Moved(_), "main") => {
            if let Some(m) = w.app_handle().try_state::<std::sync::Arc<place::Mover>>() {
                m.moved();
            }
        }
        (tauri::WindowEvent::Focused(false), popover::LABEL) => popover::blurred(w.app_handle()),
        // Alt+F4 and the like hide the widget as the menu does: destroyed, it would be gone for good
        (tauri::WindowEvent::CloseRequested { api, .. }, "main") => {
            api.prevent_close();
            show_widget(w.app_handle(), false);
        }
        _ => {}
    });
    app.setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // no dock icon: it lives in the menu bar
            let file = app.path().app_config_dir()?.join("settings.json");
            let saved = fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let settings = initial_settings(saved.as_ref());
            let main = app.get_webview_window("main").expect("main window");
            let _ = main.set_title(&app.package_info().name);
            let (transparent, wayland) = (can_be_transparent(&main), wayland(&main));
            app.manage(system::Sys::default());
            app.manage(State { settings: Mutex::new(settings.clone()), file, tray: Mutex::new(None), power: Mutex::new(String::new()), transparent, wayland, tray_nudge: Mutex::new(None), #[cfg(feature = "rust-core")] sent: Mutex::default(), tray_profiles: Mutex::new(None) });
            popover::init(app.handle());
            tray(app.handle())?;
            apply(app.handle(), &settings, true);
            watch_snooze(app.handle().clone());
            update::start(app.handle());
            // global-hotkey is an X11 key grab: on Wayland it only fires while an XWayland window has focus, so it isn't
            // bound at all, and settings says to use a desktop shortcut instead
            if wayland {
                eprintln!("wayland: no global show/hide shortcut");
            } else if let Err(e) = bind(app.handle(), "", settings["shortcut"].as_str().unwrap_or("")) {
                eprintln!("show/hide shortcut not bound: {e}");
            }
            #[cfg(target_os = "macos")]
            notes::init(app.handle());
            // the game runs here, and the window shows only its HUD (dist/hud.html). IDLE_FIGHTERZ_ENGINE=web keeps 0.1's
            // game.js in the page (index.html); =canvas keeps the frames on the page's canvas on macOS too
            #[cfg(feature = "rust-core")]
            if std::env::var("IDLE_FIGHTERZ_ENGINE").as_deref() == Ok("web") {
                eprintln!("engine: game.js in the web view (IDLE_FIGHTERZ_ENGINE=web)");
            } else {
                app.manage(planet::Core::new(planet::seed(), app.path().app_data_dir()?.join("power.json")));
                #[cfg(all(target_os = "macos", feature = "native-renderer"))]
                if std::env::var("IDLE_FIGHTERZ_ENGINE").as_deref() != Ok("canvas") {
                    // floating, the frames go in a layer under the page; window mode, or if the layer fails, on its canvas
                    match native::Under::new(app.handle()) {
                        Ok(u) => app.state::<planet::Core>().set_surface(Box::new(u)),
                        Err(e) => eprintln!("engine: no layer under the page ({e})"),
                    }
                }
                // the screensaver mirrors the run (macOS, Windows): it connects, and the tick draws for it until it goes
                #[cfg(any(target_os = "macos", windows))]
                {
                    let h = app.handle().clone();
                    match saver_link::Hub::start(move || h.state::<Shown>().poke()) {
                        Ok(hub) => app.state::<planet::Core>().set_saver(hub),
                        Err(e) => eprintln!("screensaver: no link to it ({e})"),
                    }
                }
                eprintln!("engine: the Rust core, behind the HUD page");
                smoke("engine: the Rust core");
                let https = app.config().app.windows.iter().any(|w| w.label == "main" && w.use_https_scheme);
                main.navigate(hud_url(&main.url()?, https))?;
                planet::run(app.handle().clone(), planet::Hooks { idle_fps: fps, plays_hidden, hidden: hidden_tick, moment: spectacle::moment });
                app.manage(photo::Photo::default());
                let photo = MenuItem::with_id(app, "photo", "Photo Mode", true, None::<&str>)?;
                if let Some(t) = app.state::<State>().tray.lock().unwrap().as_ref() {
                    t.menu.insert(&photo, 2)?; // after settings
                }
                recap::watch(app.handle());
            }
            if !wayland {
                watch_pointer(app.handle().clone());
                place::watch(app.handle(), app.path().app_config_dir()?.join("places.json"));
            }
            system::watch(app.handle(), wayland);
            #[cfg(debug_assertions)]
            debug_open(app.handle());
            if std::env::var_os("IDLE_FIGHTERZ_SMOKE").is_some() {
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
            if let (tauri::RunEvent::Exit, Some(core)) = (&e, app.try_state::<planet::Core>()) {
                core.stop(); // the tick thread saves no more after this
            }
            let _ = (app, e);
        });
}

// hud.html beside the page the window loads. At setup on Windows the web view is still on about:blank, which nothing
// joins onto: there it goes on the app's own origin, tauri's protocol as WebviewUrl::App resolves it
#[cfg(feature = "rust-core")]
fn hud_url(page: &tauri::Url, https: bool) -> tauri::Url {
    page.join("hud.html").unwrap_or_else(|_| {
        let origin = match (cfg!(any(windows, target_os = "android")), https) {
            (true, false) => "http://tauri.localhost/",
            (true, true) => "https://tauri.localhost/",
            (false, _) => "tauri://localhost/",
        };
        tauri::Url::parse(origin).and_then(|u| u.join("hud.html")).expect("a fixed URL")
    })
}

// the frame-rate setting: 15, 30 (the default) or 60, and 15 in place of 30 while the battery saver is on
#[cfg_attr(not(feature = "rust-core"), allow(dead_code))]
fn fps(app: &AppHandle) -> f64 {
    let s = app.state::<State>().settings.lock().unwrap().clone();
    let pick = s["fps"].as_f64().filter(|f| [15.0, 30.0, 60.0].contains(f)).unwrap_or(30.0);
    system::frame_rate(pick, s["battery_auto"].as_bool().unwrap_or(true), app.state::<system::Sys>().saving.load(std::sync::atomic::Ordering::Relaxed))
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
            // in ghost mode the widget takes no clicks, so it doesn't wake to the pointer either, until ⌥ is held
            let ghosted = app.state::<system::Sys>().ghosted.load(std::sync::atomic::Ordering::Relaxed);
            let inside = !ghosted && x >= 0.0 && y >= 0.0 && x < s.width as f64 / f && y < s.height as f64 / f;
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
    let ghost = CheckMenuItem::with_id(app, "ghost", "Ghost Mode (Click-Through)", true, false, none)?;
    let sizes = prefs::SNAPS
        .iter()
        .map(|(id, label, _)| CheckMenuItem::with_id(app, format!("size:{id}"), *label, true, false, none))
        .collect::<tauri::Result<Vec<_>>>()?;
    let more = MenuItem::with_id(app, "settings", "Any Size in Settings…", true, none)?;
    let size = Submenu::with_items(app, "Size", true, &[&sizes[0], &sizes[1], &sizes[2], &PredefinedMenuItem::separator(app)?, &more])?;
    let profiles = Submenu::with_items(app, "Profiles", true, &[])?;
    let snoozes = [("30", "30 Minutes"), ("60", "1 Hour"), ("180", "3 Hours"), ("tomorrow", "Until Tomorrow")]
        .iter()
        .map(|(id, label)| MenuItem::with_id(app, format!("snooze:{id}"), *label, true, none))
        .collect::<tauri::Result<Vec<_>>>()?;
    let resume = MenuItem::with_id(app, "snooze:off", "Resume Notifications", false, none)?;
    let snoozing = Submenu::with_items(app, "Snooze Notifications", true, &[&snoozes[0], &snoozes[1], &snoozes[2], &snoozes[3], &PredefinedMenuItem::separator(app)?, &resume])?;
    let login = CheckMenuItem::with_id(app, "login", "Launch at Login", true, false, none)?;
    let name = &app.package_info().name;
    let quit_item = MenuItem::with_id(app, "quit", format!("Quit {name}"), true, none)?;
    let update = MenuItem::with_id(app, "update", "", true, none)?;
    let check = MenuItem::with_id(app, "check_update", "Check for Updates…", true, none)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &settings, &sep, &top, &size, &profiles, &snoozing, &login, &sep, &check, &sep, &quit_item])?;
    // only in builds that carry the screensaver (saver/README.md)
    if screensaver::bundled(app).is_some() {
        menu.insert(&MenuItem::with_id(app, "screensaver", "Install Screensaver…", true, none)?, 8)?;
    }

    // the scouter is a black template image that macOS tints; Windows and Linux don't, so there it's black or white to
    // match the taskbar (or as picked in settings), and follows it when the theme changes
    #[cfg(target_os = "macos")]
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = {
        let taskbar = Taskbar::new();
        let white = white_scouter(app, || taskbar.guess());
        watch_taskbar(app.clone(), taskbar, white);
        scouter(white)?
    };
    TrayIconBuilder::with_id("tray")
        .icon(icon)
        .icon_as_template(true)
        .tooltip(name)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| menu_action(app.clone(), event.id().as_ref()))
        .on_tray_icon_event(|tray, e| {
            let app = tray.app_handle();
            let TrayIconEvent::Click { button: MouseButton::Left, button_state, rect, .. } = e else { return };
            if !prefs::popover(&app.state::<State>().settings.lock().unwrap()["tray_style"]) {
                return;
            }
            // as each one's menus do: macOS opens on the press, Windows on the release
            if (button_state == MouseButtonState::Down) != cfg!(target_os = "macos") {
                return;
            }
            let icon = popover::icon_rect(&rect);
            #[cfg(target_os = "macos")]
            if popover::control_held() {
                popover::hide(app);
                tray_menu_at(app, icon);
                return;
            }
            popover::toggle(app, icon);
        })
        .build(app)?;
    menu.insert(&ghost, 4)?; // after always on top
    let tray = Tray {
        show,
        top,
        ghost,
        sizes,
        profiles,
        snooze: snoozing,
        resume,
        login,
        menu,
        update,
        check,
        #[cfg(target_os = "linux")]
        power: MenuItem::with_id(app, "power", "", false, none)?,
    };
    *app.state::<State>().tray.lock().unwrap() = Some(tray);
    Ok(())
}

// the native menu, under the icon, for a ⌃-click while the icon opens the popover (a trackpad may have no right-click):
// popped up from the popover's window, shrunk to a point where the menu should open (AppKit pops a menu up only from
// a window on screen). The popup returns once the menu closes
#[cfg(target_os = "macos")]
fn tray_menu_at(app: &AppHandle, icon: popover::R) {
    let menu = app.state::<State>().tray.lock().unwrap().as_ref().map(|t| t.menu.clone());
    let (Some(menu), Some(w)) = (menu, popover::anchor_for_menu(app, icon)) else { return };
    let _ = w.set_size(LogicalSize::new(1.0, 1.0));
    let _ = w.show();
    let _ = w.popup_menu_at(&menu, tauri::LogicalPosition::new(0.0, 0.0));
    let _ = w.hide();
    let _ = w.set_size(LogicalSize::new(popover::SIZE.0, popover::SIZE.1));
}

// The tray menu's items, by id; the popover's items call the same
fn menu_action(app: AppHandle, id: &str) {
    match id {
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
        // both wait on the network: off the main thread
        "check_update" => drop(std::thread::spawn(move || update::check_now(&app, true))),
        "update" => drop(std::thread::spawn(move || update::install_now(&app))),
        "screensaver" => match screensaver::install(&app) {
            Ok(next) => notify(app, "screensaver installed".into(), next.into()),
            Err(e) => notify(app, "couldn't install the screensaver".into(), e),
        },
        "top" => {
            let on = !app.state::<State>().settings.lock().unwrap()["on_top"].as_bool().unwrap_or(true);
            let _ = set_settings(app, json!({ "on_top": on }));
        }
        "ghost" => {
            let on = !app.state::<State>().settings.lock().unwrap()["ghost"].as_bool().unwrap_or(false);
            let _ = set_settings(app, json!({ "ghost": on }));
        }
        "login" => {
            let on = !app.autolaunch().is_enabled().unwrap_or(false);
            let _ = set_settings(app, json!({ "login": on }));
        }
        "photo" => photo(app),
        id => {
            if let Some(size) = id.strip_prefix("size:") {
                let _ = set_settings(app, json!({ "size": size }));
            } else if let Some(choice) = id.strip_prefix("snooze:") {
                let _ = snooze(app, choice.into());
            } else if let Some(k) = id.strip_prefix("profile:").and_then(|k| k.parse::<usize>().ok()) {
                let s = app.state::<State>().settings.lock().unwrap().clone();
                if let Some(name) = prefs::names(&s).get(k).cloned() {
                    let _ = apply_profile(app.clone(), name);
                }
                // a check item toggles itself on a click: put the checks back as the settings say
                let s = app.state::<State>().settings.lock().unwrap().clone();
                let tray = app.state::<State>().tray.lock().unwrap().clone();
                if let Some(t) = tray {
                    tray_profiles(&app, &t, &s);
                }
            }
        }
    }
}

// Debug builds only: IDLE_FIGHTERZ_DEBUG_OPEN=popover, menu (the ⌃-click menu) or settings opens it 3 s after launch,
// for testing where no one can click the tray icon (a menu bar that hides it). The popover goes under the icon, or
// under the menu bar's right end if the icon isn't shown
#[cfg(debug_assertions)]
fn debug_open(app: &AppHandle) {
    let Some(what) = std::env::var("IDLE_FIGHTERZ_DEBUG_OPEN").ok().filter(|w| !w.is_empty()) else { return };
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        let h = app.clone();
        let _ = app.run_on_main_thread(move || {
            // IDLE_FIGHTERZ_DEBUG_ICON=x,y,w,h (physical pixels) stands in for the icon's rect
            let given = std::env::var("IDLE_FIGHTERZ_DEBUG_ICON").ok().and_then(|v| {
                let n: Vec<f64> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                (n.len() == 4).then(|| popover::R { x: n[0], y: n[1], w: n[2], h: n[3] })
            });
            let icon = given.or_else(|| h.tray_by_id("tray").and_then(|t| t.rect().ok().flatten()).map(|r| popover::icon_rect(&r)).filter(|r| r.w > 0.0)).unwrap_or_else(|| {
                let m = h.primary_monitor().ok().flatten();
                let (w, s) = m.as_ref().map_or((1440.0, 1.0), |m| (m.size().width as f64, m.scale_factor()));
                popover::R { x: w - 260.0 * s, y: 0.0, w: 30.0 * s, h: 24.0 * s }
            });
            eprintln!("debug: opening {what} at {icon:?}");
            match what.as_str() {
                "popover" => popover::show(&h, icon),
                #[cfg(target_os = "macos")]
                "menu" => tray_menu_at(&h, icon),
                // "settings", "settings:<tab>" (the full window's look, play, notify, keys or about), or "settings:switch"
                // (the other settings style, picked in the window)
                _ => {
                    let h = h.clone();
                    let tab = what.split_once(':').map(|(_, t)| t.replace(|c: char| !c.is_ascii_alphabetic(), ""));
                    tauri::async_runtime::spawn(async move {
                        let _ = open_settings(h.clone()).await;
                        if let (Some(tab), Some(w)) = (tab, h.get_webview_window("settings")) {
                            std::thread::sleep(Duration::from_secs(2));
                            let _ = w.eval(if tab == "switch" {
                                "set({ settings_style: document.querySelector('[data-seg=settings_style] .on')?.dataset.v === 'full' ? 'compact' : 'full' })".to_string()
                            } else {
                                format!("document.querySelector('#tabs [data-t={tab}]')?.click()")
                            });
                        }
                    });
                }
            }
        });
    });
}

#[cfg(not(target_os = "macos"))]
fn scouter(white: bool) -> tauri::Result<Image<'static>> {
    let png: &[u8] = if white { include_bytes!("../icons/tray-white.png") } else { include_bytes!("../icons/tray.png") };
    Image::from_bytes(png)
}

// the tray icon setting: "light" is the white scouter (for dark taskbars), "dark" the black one, "auto" the taskbar's theme
#[cfg(not(target_os = "macos"))]
fn white_scouter(app: &AppHandle, auto: impl FnOnce() -> bool) -> bool {
    let pick = app.state::<State>().settings.lock().unwrap()["tray_icon"].as_str().unwrap_or("auto").to_string();
    match pick.as_str() {
        "light" => true,
        "dark" => false,
        _ => auto(),
    }
}

// at once, then every few seconds and when the pick changes, swaps the scouter if it should change. only "auto" asks
// the system (cheap: a registry value, or one D-Bus call; no event covers the Windows taskbar alone)
#[cfg(not(target_os = "macos"))]
fn watch_taskbar(app: AppHandle, mut taskbar: Taskbar, mut white: bool) {
    let (nudge, woken) = std::sync::mpsc::channel();
    *app.state::<State>().tray_nudge.lock().unwrap() = Some(nudge);
    std::thread::spawn(move || loop {
        let now = white_scouter(&app, || taskbar.dark());
        if now != white {
            if let (Some(tray), Ok(icon)) = (app.tray_by_id("tray"), scouter(now)) {
                if tray.set_icon(Some(icon)).is_ok() {
                    white = now;
                }
            }
        }
        // a picked colour changes only with the pick
        let auto = app.state::<State>().settings.lock().unwrap()["tray_icon"].as_str().unwrap_or("auto") == "auto";
        let _ = if auto { woken.recv_timeout(std::time::Duration::from_secs(3)).ok() } else { woken.recv().ok() };
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
    fn guess(&self) -> bool {
        self.dark()
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
// at startup on the main thread (GTK lives there). With no preference, white: most panels are dark (GNOME's top bar is).
// The portal is only asked from the watcher's thread: a slow or hung session bus must not stall startup
#[cfg(target_os = "linux")]
struct Taskbar {
    bus: Option<Option<zbus::blocking::Connection>>, // connected on first use
    gtk_dark: Option<bool>,
}
#[cfg(target_os = "linux")]
impl Taskbar {
    fn new() -> Self {
        use gtk::glib::object::ObjectExt;
        let theme = gtk::Settings::default().and_then(|s| s.property::<Option<String>>("gtk-theme-name"));
        Taskbar { bus: None, gtk_dark: theme.map(|t| t.to_lowercase().contains("dark")) }
    }
    // the first icon, without the portal; the watcher corrects it at once
    fn guess(&self) -> bool {
        self.gtk_dark.unwrap_or(true)
    }
    fn dark(&mut self) -> bool {
        let bus = self.bus.get_or_insert_with(|| zbus::blocking::Connection::session().ok());
        match bus.as_ref().and_then(color_scheme) {
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tauri::{
        ipc::{CallbackFn, InvokeBody},
        test::{get_ipc_response, mock_builder, INVOKE_KEY},
        webview::InvokeRequest,
        WebviewUrl, WebviewWindowBuilder,
    };

    // Windows has the web view on about:blank at setup: hud.html then goes on the app's origin, not nowhere (0.2.0's
    // first Windows smoke run aborted on that join)
    #[cfg(feature = "rust-core")]
    #[test]
    fn the_hud_page_has_a_url_before_the_page_loads() {
        let url = |s: &str| tauri::Url::parse(s).unwrap();
        assert_eq!(super::hud_url(&url("tauri://localhost/"), false), url("tauri://localhost/hud.html"));
        assert_eq!(super::hud_url(&url("http://tauri.localhost/index.html"), false), url("http://tauri.localhost/hud.html"));
        assert_eq!(super::hud_url(&url("http://localhost:1420/"), false), url("http://localhost:1420/hud.html"));
        let blank = if cfg!(windows) { ["http://tauri.localhost/hud.html", "https://tauri.localhost/hud.html"] } else { ["tauri://localhost/hud.html"; 2] };
        assert_eq!(super::hud_url(&url("about:blank"), false), url(blank[0]));
        assert_eq!(super::hud_url(&url("about:blank"), true), url(blank[1]));
    }

    // a settings.json as 0.1.1 saved it: its keys carry over, the ones added since get their defaults, layout 4 is gone
    #[test]
    fn a_0_1_1_settings_file_merges() {
        let saved = json!({
            "on_top": false, "size": "large", "opacity": 0.8, "layout": 4, "hud": "always", "clouds": "never", "cloud_shape": "c",
            "cloud_amount": 3, "mode": "window",
            "notify": { "raid": false, "shenron": true, "trip": true, "pod": true, "ascend": true },
            "quiet": { "on": true, "from": "23:00", "to": "07:00" },
            "shortcut": "Alt+Cmd+KeyO"
        });
        let s = super::initial_settings(Some(&saved));
        assert_eq!((s["fps"].clone(), s["menubar_power"].clone(), s["tray_icon"].clone()), (json!(30), json!(true), json!("auto")));
        assert_eq!(s["layout"], json!(7));
        assert_eq!((s["on_top"].clone(), s["size"].clone(), s["mode"].clone()), (json!(false), json!(560), json!("window")), "large is 560 on the slider");
        assert_eq!(s["notify"]["pod"], json!(true));
        assert_eq!(s["quiet"]["from"], json!("23:00"));
        assert_eq!(s["shortcut"], json!("Alt+Cmd+KeyO"));
        for layout in [7, 1, 2, 3, 6] {
            assert_eq!(super::initial_settings(Some(&json!({ "layout": layout })))["layout"], json!(layout));
        }
        assert_eq!(super::initial_settings(Some(&json!({ "layout": 5 })))["layout"], json!(7));
    }

    // a settings.json as 0.2.1 saved it, battery saver picked: ghost mode, auto-hide and the automatic battery saver
    // come in at their defaults, and the pick stays
    #[test]
    fn a_0_2_1_settings_file_merges() {
        let saved = json!({ "on_top": true, "size": "small", "fps": 15, "mode": "floating", "tray_icon": "dark", "menubar_power": false });
        let s = super::initial_settings(Some(&saved));
        assert_eq!((s["ghost"].clone(), s["battery_auto"].clone()), (json!(false), json!(true)));
        assert_eq!(s["auto_hide"], json!(!cfg!(target_os = "linux")));
        assert_eq!((s["fps"].clone(), s["size"].clone(), s["menubar_power"].clone()), (json!(15), json!(300), json!(false)));
        let picked = super::initial_settings(Some(&json!({ "ghost": true, "auto_hide": false, "battery_auto": false })));
        assert_eq!((picked["ghost"].clone(), picked["auto_hide"].clone(), picked["battery_auto"].clone()), (json!(true), json!(false), json!(false)));
    }

    // a settings.json from before this batch: the size slider takes its preset, and the new settings their defaults; a
    // saved game sound (null until the widget says) and profiles come back as saved
    #[test]
    fn a_0_2_x_settings_file_gets_the_new_settings() {
        let s = super::initial_settings(Some(&json!({ "size": "medium", "ghost": true })));
        assert_eq!(s["size"], json!(400));
        assert_eq!((s["follow_focus"].clone(), s["recap"].clone(), s["snooze_until"].clone()), (json!(true), json!(true), json!(0)));
        assert_eq!((s["sound"].clone(), s["profiles"].clone()), (json!(null), json!([])));
        assert_eq!((s["skin"].clone(), s["theme"].clone(), s["ki_buttons"].clone()), (json!("classic"), json!("light"), json!("meter")), "today's look");
        let s = super::initial_settings(Some(&json!({ "skin": "drawer", "theme": "auto", "ki_buttons": "classic" })));
        assert_eq!((s["skin"].clone(), s["theme"].clone(), s["ki_buttons"].clone()), (json!("drawer"), json!("auto"), json!("classic")), "a saved pick stays");
        let s = super::initial_settings(Some(&json!({ "skin": "manga", "theme": "sepia", "ki_buttons": "gauge" })));
        assert_eq!((s["skin"].clone(), s["theme"].clone(), s["ki_buttons"].clone()), (json!("classic"), json!("light"), json!("meter")), "one this version doesn't know");
        let profiles = json!([{ "name": "Work", "settings": { "size": 300, "ghost": true } }]);
        let s = super::initial_settings(Some(&json!({ "size": 452, "sound": true, "profiles": profiles })));
        assert_eq!((s["size"].clone(), s["sound"].clone(), s["profiles"].clone()), (json!(452), json!(true), profiles));
    }

    // a first launch has no file: the first-run card is due, and the windows are today's. Any file from before (0.2.x)
    // has met the widget: no card. A file that asked for the intro again keeps asking
    #[test]
    fn a_first_launch_gets_the_card_and_an_upgrade_does_not() {
        let fresh = super::initial_settings(None);
        assert_eq!((fresh["onboarded"].clone(), fresh["settings_style"].clone(), fresh["tray_style"].clone()), (json!(false), json!("compact"), json!("menu")));
        let upgraded = super::initial_settings(Some(&json!({ "size": 400, "ghost": false, "profiles": [] })));
        assert_eq!((upgraded["onboarded"].clone(), upgraded["settings_style"].clone(), upgraded["tray_style"].clone()), (json!(true), json!("compact"), json!("menu")));
        assert_eq!(super::initial_settings(Some(&json!({ "onboarded": false })))["onboarded"], json!(false));
        let picked = super::initial_settings(Some(&json!({ "settings_style": "full", "tray_style": "popover", "onboarded": true })));
        assert_eq!((picked["settings_style"].clone(), picked["tray_style"].clone()), (json!("full"), json!("popover")));
        assert_eq!(super::initial_settings(Some(&json!({ "settings_style": 3 })))["settings_style"], json!("compact"), "not a string: ignored");
    }

    // the settings each window has a row for (data-s), as a set
    fn rows(page: &str) -> std::collections::BTreeSet<String> {
        page.split("data-s=\"").skip(1).filter_map(|r| r.split('"').next()).filter(|k| !k.contains('$')).map(String::from).collect()
    }

    // every setting the app keeps has a row in both settings windows, and neither window has a row the other lacks
    #[test]
    fn both_settings_windows_have_every_setting() {
        let (compact, full) = (rows(include_str!("../../settings.html")), rows(include_str!("../../settings-full.html")));
        let row = |k: &str| match k {
            "profiles" => "profile",
            "snooze_until" => "snooze",
            "onboarded" => "intro",
            k => k,
        }
        .to_string();
        let kept: std::collections::BTreeSet<String> = serde_json::from_str::<serde_json::Value>(super::DEFAULTS).unwrap().as_object().unwrap().keys().map(|k| row(k)).collect();
        for k in &kept {
            assert!(compact.contains(k), "the compact settings have no row for {k}");
            assert!(full.contains(k), "the full settings have no row for {k}");
        }
        assert_eq!(compact.difference(&full).collect::<Vec<_>>(), Vec::<&String>::new(), "only in the compact settings");
        assert_eq!(full.difference(&compact).collect::<Vec<_>>(), Vec::<&String>::new(), "only in the full settings");
        assert!(compact.contains("login") && full.contains("login") && full.contains("permission"));
    }

    // every command the app registers is in build.rs's list and granted to some window: one that isn't, the ACL refuses
    // whatever page calls it (0.5.0's planet_wear)
    #[test]
    fn every_registered_command_is_built_and_granted() {
        let list = |src: &str, from: &str| -> Vec<String> {
            let body = src.split(from).nth(1).unwrap().split(']').next().unwrap();
            body.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).filter(|w| !w.is_empty()).map(|w| w.rsplit("::").next().unwrap().to_string()).collect()
        };
        let mut commands = list(include_str!("main.rs"), "$($m)::+![");
        commands.extend(list(include_str!("planet.rs"), "$($more,)*"));
        assert!(commands.len() > 50 && commands.contains(&"planet_wear".to_string()), "{commands:?}");
        let built = include_str!("../build.rs");
        let granted = [include_str!("../capabilities/default.json"), include_str!("../capabilities/settings.json"), include_str!("../capabilities/popover.json")].concat();
        for cmd in &commands {
            assert!(built.contains(&format!("\"{cmd}\"")), "{cmd} isn't in build.rs's COMMANDS");
            assert!(granted.contains(&format!("\"allow-{}\"", cmd.replace('_', "-"))), "no capability allows {cmd}");
        }
    }

    // with no handlers registered, a granted command fails as "not found" and any other is refused by the ACL first
    #[test]
    fn each_window_may_call_only_its_own_commands() {
        let app = mock_builder().invoke_handler(tauri::generate_handler![]).build(tauri::generate_context!(test = true)).unwrap();
        let open = |label: &str| WebviewWindowBuilder::new(&app, label, WebviewUrl::App("index.html".into())).build().unwrap();
        let (main, settings, popover) = (open("main"), open("settings"), open("popover"));
        let call = |w: &tauri::WebviewWindow<tauri::test::MockRuntime>, cmd: &str| {
            let req = InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: w.url().unwrap(),
                body: InvokeBody::Json(json!({})),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            };
            !get_ipc_response(w, req).unwrap_err().to_string().contains("not allowed")
        };
        for cmd in ["get_settings", "tray_title", "hide_widget", "open_settings", "notify", "page_state", "set_sound", "photo", "photo_exit", "intro_done", "planet_start", "planet_buy", "planet_fps", "planet_itx", "planet_itx_jump", "planet_view", "planet_pixels", "planet_mask", "planet_sprite", "planet_spot", "planet_buy_n", "planet_shop_info", "planet_bulk", "planet_wear"] {
            assert!(call(&main, cmd), "main may call {cmd}");
        }
        for cmd in ["set_settings", "quit", "open_notify_settings", "request_notify_permission", "snooze", "save_profile", "apply_profile", "delete_profile", "planet_step", "planet_load", "planet_save", "glance", "preview_frame", "tray_action", "hide_popover"] {
            assert!(!call(&main, cmd), "main may not call {cmd}");
        }
        for cmd in ["get_settings", "set_settings", "quit", "notify_permission", "request_notify_permission", "open_notify_settings", "focus_status", "snooze", "save_profile", "delete_profile", "apply_profile", "glance", "preview_frame"] {
            assert!(call(&settings, cmd), "settings may call {cmd}");
        }
        for cmd in ["notify", "tray_title", "hide_widget", "page_state", "set_sound", "photo", "photo_exit", "planet_start", "planet_buy", "planet_buy_n", "planet_fps", "planet_view", "planet_pixels", "planet_mask", "tray_action", "hide_popover"] {
            assert!(!call(&settings, cmd), "settings may not call {cmd}");
        }
        for cmd in ["get_settings", "set_settings", "glance", "tray_action", "hide_popover"] {
            assert!(call(&popover, cmd), "the popover may call {cmd}");
        }
        for cmd in ["notify", "tray_title", "hide_widget", "set_sound", "photo", "planet_start", "planet_buy", "planet_buy_n", "planet_pixels", "preview_frame", "save_profile", "snooze", "open_settings"] {
            assert!(!call(&popover, cmd), "the popover may not call {cmd}");
        }
    }
}
