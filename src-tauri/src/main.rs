#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window behind the widget on Windows
// The power planet as a desktop pet: one frameless, transparent window showing the bundled page, a settings
// window, and a menu-bar icon. Settings are app config in a JSON file; the game itself is never saved.
use serde_json::{json, Value};
use std::{fs, path::PathBuf, sync::Mutex};
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

// window sides in points, square: the planet sits in the middle with room for its visitors
const SIZES: [(&str, &str, f64); 3] = [("small", "Small", 300.0), ("medium", "Medium", 400.0), ("large", "Large", 560.0)];
// every setting and its default; a saved file only overrides keys listed here
const DEFAULTS: &str = r#"{
  "on_top": true, "size": "medium", "opacity": 1, "layout": 7, "hud": "hover", "clouds": "hover", "cloud_shape": "c", "cloud_amount": 2, "mode": "floating", "menubar_power": true, "fps": 30,
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
}

struct State {
    settings: Mutex<Value>,
    file: PathBuf,
    tray: Mutex<Option<Tray>>,
    power: Mutex<String>, // the power level as the widget last sent it, for the menu-bar title
    transparent: bool,    // false on a Linux desktop with no compositor: floating can't work, so it's always window mode
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
        if new != old {
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
    show_power(app, s);
}

// the power level beside the menu-bar icon, or the icon alone when that's turned off. Windows trays have no title, so
// it goes in the tooltip; Linux trays have no tooltip, but an AppIndicator label beside the icon
fn show_power(app: &AppHandle, s: &Value) {
    let Some(tray) = app.tray_by_id("tray") else { return };
    let power = app.state::<State>().power.lock().unwrap().clone();
    let on = s["menubar_power"].as_bool().unwrap_or(true) && !power.is_empty();
    #[cfg(not(target_os = "windows"))]
    let _ = tray.set_title(on.then_some(power));
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
    let app = tauri::Builder::default();
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
    app.invoke_handler(tauri::generate_handler![
        get_settings, set_settings, tray_title, hide_widget, open_settings, quit, notify, notify_permission, request_notify_permission, open_notify_settings
    ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // no dock icon: it lives in the menu bar
            let file = app.path().app_config_dir()?.join("settings.json");
            let mut settings: Value = serde_json::from_str(DEFAULTS).unwrap();
            #[cfg(not(target_os = "macos"))]
            {
                settings["shortcut"] = json!(SHORTCUT);
            }
            if let Some(saved) = fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                merge(&mut settings, &saved);
            }
            if ![7, 1, 2, 3, 6].contains(&settings["layout"].as_u64().unwrap_or(0)) {
                settings["layout"] = json!(7); // layouts 4 and 5 were dropped
            }
            let main = app.get_webview_window("main").expect("main window");
            let _ = main.set_title(&app.package_info().name);
            let transparent = can_be_transparent(&main);
            app.manage(State { settings: Mutex::new(settings.clone()), file, tray: Mutex::new(None), power: Mutex::new(String::new()), transparent });
            tray(app.handle())?;
            apply(app.handle(), &settings, true);
            if let Err(e) = bind(app.handle(), "", settings["shortcut"].as_str().unwrap_or("")) {
                eprintln!("show/hide shortcut not bound: {e}");
            }
            #[cfg(target_os = "macos")]
            notes::init(app.handle());
            if !wayland(&main) {
                watch_pointer(app.handle().clone());
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
            let _ = (app, e);
        });
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
fn watch_pointer(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(120));
            let Some(w) = app.get_webview_window("main") else { continue };
            if !w.is_visible().unwrap_or(false) {
                continue;
            }
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

    // the scouter is a black template image that only macOS tints; on dark Windows and Linux panels it'd vanish, so the
    // dragon ball goes there
    #[cfg(target_os = "macos")]
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
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
    *app.state::<State>().tray.lock().unwrap() = Some(Tray { show, top, sizes, login });
    Ok(())
}
