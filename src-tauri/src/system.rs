// What the computer is up to, for three settings: ghost mode (is ⌥/Alt held?), auto-hide (is a fullscreen app or a
// slideshow in front?) and the battery saver (on battery, or in low power mode?). Every probe reads state the OS gives
// any app, with no permission prompt; where a platform can't tell, the probe says so and its setting does nothing.
use crate::{show_widget, smoke, Shown, State};
use serde_json::Value;
use std::{
    sync::atomic::{AtomicBool, Ordering::Relaxed},
    time::Duration,
};
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct Sys {
    pub ghosted: AtomicBool,     // the widget lets clicks through right now
    pub saving: AtomicBool,      // on battery, or in low power mode / battery saver
    pub auto_hidden: AtomicBool, // auto-hide hid the widget, and brings it back
}

/// the frame rate to draw at: battery saver (15) while saving power, but only in place of the default, balanced (30).
/// smooth or battery saver picked in settings is the user's own choice and stays
pub fn frame_rate(pick: f64, auto: bool, saving: bool) -> f64 {
    if auto && saving && pick == 30.0 {
        15.0
    } else {
        pick
    }
}

/// whether the widget lets clicks through: ghost mode on a floating widget, except while ⌥ is held
pub fn click_through(ghost: bool, floating: bool, alt: bool) -> bool {
    ghost && floating && !alt
}

#[derive(Debug, PartialEq)]
pub enum Act {
    Hide,
    Show,
    Stay,
}

/// one auto-hide step. It acts only when the screen changes, so a widget shown by hand during a slideshow stays, and
/// one hidden by hand isn't brought back
pub fn auto_hide(on: bool, was_busy: bool, busy: bool, visible: bool, auto_hidden: bool) -> Act {
    if auto_hidden && (!on || !busy) {
        Act::Show
    } else if on && busy && !was_busy && visible {
        Act::Hide
    } else {
        Act::Stay
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct Win {
    pub pid: i64,
    pub layer: i64,
    pub alpha: f64,
    pub rect: Rect,
    pub presenter: bool, // Keynote or PowerPoint
}

/// macOS: the frontmost ordinary window (layer 0) fills a whole display, as in a fullscreen Space or a borderless
/// game; or Keynote or PowerPoint fills one at any layer (a slideshow). Windows come front to back; ours don't count
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn fullscreen(wins: &[Win], displays: &[Rect], me: i64) -> bool {
    let covers = |r: &Rect| displays.iter().any(|d| r.x <= d.x && r.y <= d.y && r.x + r.w >= d.x + d.w && r.y + r.h >= d.y + d.h);
    let mut theirs = wins.iter().filter(|w| w.pid != me && w.alpha > 0.0 && w.rect.w > 1.0 && w.rect.h > 1.0);
    theirs.clone().find(|w| w.layer == 0).is_some_and(|w| covers(&w.rect)) || theirs.any(|w| w.presenter && w.layer >= 0 && covers(&w.rect))
}

/// Windows: SHQueryUserNotificationState's QUNS_BUSY (a fullscreen app), QUNS_RUNNING_D3D_FULL_SCREEN (a game) or
/// QUNS_PRESENTATION_MODE
#[cfg_attr(not(windows), allow(dead_code))]
pub fn quns_busy(state: i32) -> bool {
    matches!(state, 2..=4)
}

/// one entry of /sys/class/power_supply
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct Supply {
    pub kind: String,
    pub online: bool,
    pub status: String,
    pub scope: String,
}

/// Linux: a system battery discharging, with no charger online. A mouse's or a headset's battery (scope Device) isn't
/// the computer's
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn on_battery(supplies: &[Supply]) -> bool {
    let charger = supplies.iter().any(|s| s.kind != "Battery" && s.online);
    !charger && supplies.iter().any(|s| s.kind == "Battery" && s.scope != "Device" && s.status == "Discharging")
}

fn setting<T>(app: &AppHandle, f: impl FnOnce(&Value) -> T) -> T {
    f(&app.state::<State>().settings.lock().unwrap())
}

// blocks until the widget shows or hides or the settings change, or for `d`
fn pause(app: &AppHandle, d: Option<Duration>) {
    let shown = app.state::<Shown>();
    let s = shown.on.lock().unwrap();
    let n = s.1;
    match d {
        Some(d) => drop(shown.changed.wait_timeout_while(s, d, |s| s.1 == n)),
        None => drop(shown.changed.wait_while(s, |s| s.1 == n)),
    }
}

pub fn watch(app: &AppHandle, wayland: bool) {
    watch_power(app.clone());
    watch_screen(app.clone(), wayland);
    watch_alt(app.clone(), wayland);
}

// on battery or low power: every 20 s while the widget shows (hidden, the frame rate doesn't matter), and the pages
// hear the new frame rate when it changes
fn watch_power(app: AppHandle) {
    std::thread::spawn(move || {
        let mut power = os::Power::new();
        loop {
            app.state::<Shown>().wait();
            if setting(&app, |s| s["battery_auto"].as_bool().unwrap_or(true)) {
                let saving = power.saving();
                if app.state::<Sys>().saving.swap(saving, Relaxed) != saving {
                    smoke(&format!("power: saving={saving}"));
                    crate::publish(&app);
                }
            }
            pause(&app, Some(Duration::from_secs(20)));
        }
    });
}

// auto-hide: every 2 s while the widget shows, or while it's hidden for a fullscreen app (to bring it back). The first
// look after a pause only notes how the screen is, so it acts on changes alone
fn watch_screen(app: AppHandle, wayland: bool) {
    std::thread::spawn(move || {
        let mut screen = os::Screen::new(wayland);
        let mut was = None;
        loop {
            let on = setting(&app, |s| s["auto_hide"].as_bool().unwrap_or(false));
            let ours = app.state::<Sys>().auto_hidden.load(Relaxed);
            let visible = app.state::<Shown>().get();
            if !ours && !(on && visible) {
                was = None;
                pause(&app, None);
                continue;
            }
            let busy = screen.busy().unwrap_or(false);
            match auto_hide(on, was.unwrap_or(busy), busy, visible, ours) {
                Act::Hide => {
                    smoke("auto-hide: hid the widget");
                    let a = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        show_widget(&a, false);
                        a.state::<Sys>().auto_hidden.store(true, Relaxed);
                    });
                }
                Act::Show => {
                    smoke("auto-hide: brought the widget back");
                    app.state::<Sys>().auto_hidden.store(false, Relaxed);
                    let a = app.clone();
                    let _ = app.run_on_main_thread(move || show_widget(&a, true));
                }
                Act::Stay => {}
            }
            was = Some(busy);
            pause(&app, Some(Duration::from_secs(2)));
        }
    });
}

// ghost mode: the floating widget ignores the mouse, and ⌥ held makes it take clicks again. ⌥ is read 10 times a second,
// only while ghost mode is on and the widget shows
fn watch_alt(app: AppHandle, wayland: bool) {
    std::thread::spawn(move || {
        let mut keys = os::Keys::new(wayland);
        let mut through = false;
        loop {
            let ghost = setting(&app, |s| s["ghost"].as_bool().unwrap_or(false));
            let floating = setting(&app, |s| s["mode"] != "window") && app.state::<State>().transparent;
            let watch = ghost && floating && app.state::<Shown>().get();
            let alt = if watch { keys.alt() } else { None };
            let want = click_through(ghost, floating, alt.unwrap_or(false));
            if want != through {
                if let Some(w) = app.get_webview_window("main") {
                    if w.set_ignore_cursor_events(want).is_ok() {
                        through = want;
                        app.state::<Sys>().ghosted.store(want, Relaxed);
                    }
                }
            }
            if alt.is_some() {
                std::thread::sleep(Duration::from_millis(100));
            } else {
                pause(&app, None);
            }
        }
    });
}

#[cfg(target_os = "macos")]
mod os {
    use super::{Rect, Win};
    use objc2::{class, msg_send, runtime::AnyObject, sel};
    use std::ffi::{c_char, c_void, CStr};

    type Cf = *const c_void;
    #[repr(C)]
    #[derive(Default)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: Cf);
        fn CFArrayGetCount(a: Cf) -> isize;
        fn CFArrayGetValueAtIndex(a: Cf, i: isize) -> Cf;
        fn CFDictionaryGetValue(d: Cf, key: Cf) -> Cf;
        fn CFNumberGetValue(n: Cf, kind: isize, out: *mut c_void) -> bool;
        fn CFStringGetCString(s: Cf, buf: *mut c_char, size: isize, encoding: u32) -> bool;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    #[allow(non_upper_case_globals)]
    extern "C" {
        static kCGWindowOwnerPID: Cf;
        static kCGWindowOwnerName: Cf;
        static kCGWindowLayer: Cf;
        static kCGWindowAlpha: Cf;
        static kCGWindowBounds: Cf;
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> Cf;
        fn CGRectMakeWithDictionaryRepresentation(dict: Cf, rect: *mut CGRect) -> bool;
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGEventSourceFlagsState(state: i32) -> u64;
    }
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> Cf;
        fn IOPSGetProvidingPowerSourceType(blob: Cf) -> Cf;
    }

    unsafe fn string(s: Cf) -> Option<String> {
        let mut buf = [0 as c_char; 128];
        (!s.is_null() && CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, 0x0800_0100)).then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
    unsafe fn number(n: Cf) -> Option<f64> {
        let mut v = 0f64;
        (!n.is_null() && CFNumberGetValue(n, 13, &mut v as *mut f64 as *mut c_void)).then_some(v) // kCFNumberDoubleType
    }

    pub struct Keys;
    impl Keys {
        pub fn new(_wayland: bool) -> Self {
            Keys
        }
        // the modifier state of the session, as NSEvent.modifierFlags reads it: no event tap, so no Input Monitoring
        pub fn alt(&mut self) -> Option<bool> {
            Some(unsafe { CGEventSourceFlagsState(0) } & 0x0008_0000 != 0) // combined session state, the ⌥ flag
        }
    }

    pub struct Screen;
    impl Screen {
        pub fn new(_wayland: bool) -> Self {
            Screen
        }
        // the window list's bounds, owners and layers need no Screen Recording permission (only window titles do)
        pub fn busy(&mut self) -> Option<bool> {
            unsafe {
                let mut ids = [0u32; 16];
                let mut n = 0;
                if CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut n) != 0 {
                    return None;
                }
                let displays: Vec<Rect> = ids[..n as usize]
                    .iter()
                    .map(|&d| {
                        let r = CGDisplayBounds(d);
                        Rect { x: r.x, y: r.y, w: r.w, h: r.h }
                    })
                    .collect();
                let list = CGWindowListCopyWindowInfo(1 | 16, 0); // on screen only, no desktop elements
                if list.is_null() {
                    return None;
                }
                let wins: Vec<Win> = (0..CFArrayGetCount(list))
                    .map(|i| {
                        let d = CFArrayGetValueAtIndex(list, i);
                        let mut r = CGRect::default();
                        CGRectMakeWithDictionaryRepresentation(CFDictionaryGetValue(d, kCGWindowBounds), &mut r);
                        let owner = string(CFDictionaryGetValue(d, kCGWindowOwnerName)).unwrap_or_default();
                        Win {
                            pid: number(CFDictionaryGetValue(d, kCGWindowOwnerPID)).unwrap_or(-1.0) as i64,
                            layer: number(CFDictionaryGetValue(d, kCGWindowLayer)).unwrap_or(-1.0) as i64,
                            alpha: number(CFDictionaryGetValue(d, kCGWindowAlpha)).unwrap_or(1.0),
                            rect: Rect { x: r.x, y: r.y, w: r.w, h: r.h },
                            presenter: owner == "Keynote" || owner == "Microsoft PowerPoint",
                        }
                    })
                    .collect();
                CFRelease(list);
                Some(super::fullscreen(&wins, &displays, std::process::id() as i64))
            }
        }
    }

    pub struct Power;
    impl Power {
        pub fn new() -> Self {
            Power
        }
        pub fn saving(&mut self) -> bool {
            unsafe {
                let blob = IOPSCopyPowerSourcesInfo();
                let battery = !blob.is_null() && string(IOPSGetProvidingPowerSourceType(blob)).as_deref() == Some("Battery Power");
                if !blob.is_null() {
                    CFRelease(blob);
                }
                // Low Power Mode came to the Mac in macOS 12
                let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
                let has: bool = msg_send![info, respondsToSelector: sel!(isLowPowerModeEnabled)];
                battery || (has && msg_send![info, isLowPowerModeEnabled])
            }
        }
    }
}

#[cfg(windows)]
mod os {
    #[repr(C)]
    #[derive(Default)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        _battery_flag: u8,
        _battery_life_percent: u8,
        system_status_flag: u8,
        _battery_life_time: u32,
        _battery_full_life_time: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHQueryUserNotificationState(state: *mut i32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
    }

    pub struct Keys;
    impl Keys {
        pub fn new(_wayland: bool) -> Self {
            Keys
        }
        pub fn alt(&mut self) -> Option<bool> {
            Some(unsafe { GetAsyncKeyState(0x12) } < 0) // VK_MENU, down
        }
    }

    pub struct Screen;
    impl Screen {
        pub fn new(_wayland: bool) -> Self {
            Screen
        }
        pub fn busy(&mut self) -> Option<bool> {
            let mut state = 0;
            (unsafe { SHQueryUserNotificationState(&mut state) } == 0).then(|| super::quns_busy(state))
        }
    }

    pub struct Power;
    impl Power {
        pub fn new() -> Self {
            Power
        }
        // unplugged (ACLineStatus 0), or battery saver on (SystemStatusFlag 1)
        pub fn saving(&mut self) -> bool {
            let mut s = SystemPowerStatus::default();
            let ok = unsafe { GetSystemPowerStatus(&mut s) } != 0;
            ok && (s.ac_line_status == 0 || s.system_status_flag == 1)
        }
    }
}

// X11 through its own connection on each watcher's thread (x11-dl loads libX11 at run time). Wayland shows no app the
// keyboard or other apps' windows, so there ⌥ and fullscreen go unseen
#[cfg(target_os = "linux")]
mod os {
    use std::{ffi::CString, fs, os::raw::c_ulong, ptr};
    use x11_dl::xlib::{self, Xlib};

    struct X {
        lib: Xlib,
        dpy: *mut xlib::Display,
        root: c_ulong,
    }
    impl X {
        fn open(wayland: bool) -> Option<X> {
            if wayland {
                return None;
            }
            let lib = Xlib::open().ok()?;
            let dpy = unsafe { (lib.XOpenDisplay)(ptr::null()) };
            if dpy.is_null() {
                return None;
            }
            let root = unsafe { (lib.XDefaultRootWindow)(dpy) };
            Some(X { lib, dpy, root })
        }
        fn atom(&self, name: &str) -> c_ulong {
            let c = CString::new(name).unwrap();
            unsafe { (self.lib.XInternAtom)(self.dpy, c.as_ptr(), xlib::False) }
        }
        // a 32-bit list property (windows, atoms)
        fn list(&self, win: c_ulong, prop: c_ulong, kind: c_ulong) -> Vec<c_ulong> {
            let (mut actual, mut format, mut n, mut after, mut data) = (0, 0, 0, 0, ptr::null_mut());
            let ok = unsafe { (self.lib.XGetWindowProperty)(self.dpy, win, prop, 0, 64, xlib::False, kind, &mut actual, &mut format, &mut n, &mut after, &mut data) };
            let mut out = Vec::new();
            if ok == 0 && !data.is_null() {
                if format == 32 {
                    out = unsafe { std::slice::from_raw_parts(data as *const c_ulong, n as usize) }.to_vec();
                }
                unsafe { (self.lib.XFree)(data as *mut _) };
            }
            out
        }
    }
    impl Drop for X {
        fn drop(&mut self) {
            unsafe { (self.lib.XCloseDisplay)(self.dpy) };
        }
    }

    pub struct Keys(Option<X>);
    impl Keys {
        pub fn new(wayland: bool) -> Self {
            Keys(X::open(wayland))
        }
        pub fn alt(&mut self) -> Option<bool> {
            let x = self.0.as_ref()?;
            let (mut r, mut c, mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0, 0, 0);
            unsafe { (x.lib.XQueryPointer)(x.dpy, x.root, &mut r, &mut c, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask) };
            Some(mask & xlib::Mod1Mask != 0)
        }
    }

    pub struct Screen(Option<X>);
    impl Screen {
        pub fn new(wayland: bool) -> Self {
            Screen(X::open(wayland))
        }
        // the active window's _NET_WM_STATE has _NET_WM_STATE_FULLSCREEN
        pub fn busy(&mut self) -> Option<bool> {
            let x = self.0.as_ref()?;
            let active = *x.list(x.root, x.atom("_NET_ACTIVE_WINDOW"), xlib::XA_WINDOW).first()?;
            if active == 0 {
                return Some(false);
            }
            Some(x.list(active, x.atom("_NET_WM_STATE"), xlib::XA_ATOM).contains(&x.atom("_NET_WM_STATE_FULLSCREEN")))
        }
    }

    pub struct Power(Option<Option<zbus::blocking::Connection>>);
    impl Power {
        pub fn new() -> Self {
            Power(None)
        }
        pub fn saving(&mut self) -> bool {
            let read = |dir: &std::path::Path, f: &str| fs::read_to_string(dir.join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
            let supplies: Vec<super::Supply> = fs::read_dir("/sys/class/power_supply")
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    let d = e.path();
                    super::Supply { kind: read(&d, "type"), online: read(&d, "online") == "1", status: read(&d, "status"), scope: read(&d, "scope") }
                })
                .collect();
            super::on_battery(&supplies) || self.power_saver()
        }
        // power-profiles-daemon's power-saver profile, where it runs
        fn power_saver(&mut self) -> bool {
            use zbus::zvariant::{OwnedValue, Value};
            let Some(bus) = self.0.get_or_insert_with(|| zbus::blocking::Connection::system().ok()).as_ref() else { return false };
            [("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"), ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles")].iter().any(|(name, path)| {
                bus.call_method(Some(*name), *path, Some("org.freedesktop.DBus.Properties"), "Get", &(*name, "ActiveProfile"))
                    .ok()
                    .and_then(|r| r.body().deserialize::<OwnedValue>().ok())
                    .is_some_and(|v| matches!(&*v, Value::Str(s) if s.as_str() == "power-saver"))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_saver_takes_over_balanced_only() {
        assert_eq!(frame_rate(30.0, true, true), 15.0);
        assert_eq!(frame_rate(30.0, true, false), 30.0);
        assert_eq!(frame_rate(30.0, false, true), 30.0);
        // a manual pick wins, plugged in or not
        assert_eq!(frame_rate(60.0, true, true), 60.0);
        assert_eq!(frame_rate(15.0, true, false), 15.0);
        assert_eq!(frame_rate(15.0, false, false), 15.0);
    }

    #[test]
    fn ghost_mode_lets_clicks_through_a_floating_widget_unless_alt_is_held() {
        assert!(click_through(true, true, false));
        assert!(!click_through(true, true, true));
        assert!(!click_through(true, false, false), "window mode is unaffected");
        assert!(!click_through(false, true, false));
    }

    #[test]
    fn auto_hide_acts_on_changes_only() {
        assert_eq!(auto_hide(true, false, true, true, false), Act::Hide);
        assert_eq!(auto_hide(true, true, false, false, true), Act::Show);
        assert_eq!(auto_hide(true, true, true, false, true), Act::Stay, "still fullscreen");
        // shown by hand mid-slideshow: it stays
        assert_eq!(auto_hide(true, true, true, true, false), Act::Stay);
        // hidden by hand: not brought back
        assert_eq!(auto_hide(true, true, false, false, false), Act::Stay);
        // turned off while it had hidden the widget: back it comes
        assert_eq!(auto_hide(false, true, true, false, true), Act::Show);
        assert_eq!(auto_hide(false, false, true, true, false), Act::Stay);
    }

    #[test]
    fn a_window_filling_a_display_is_fullscreen() {
        let screen = [Rect { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }, Rect { x: 1512.0, y: 0.0, w: 1920.0, h: 1080.0 }];
        let win = |pid, layer, rect, presenter| Win { pid, layer, alpha: 1.0, rect, presenter };
        let full = Rect { x: 1512.0, y: 0.0, w: 1920.0, h: 1080.0 };
        let zoomed = Rect { x: 0.0, y: 38.0, w: 1512.0, h: 944.0 }; // a maximized window leaves the menu bar
        assert!(fullscreen(&[win(7, 0, full, false), win(8, 0, zoomed, false)], &screen, 1));
        assert!(!fullscreen(&[win(8, 0, zoomed, false), win(7, 0, full, false)], &screen, 1), "only the frontmost counts");
        assert!(!fullscreen(&[win(1, 0, full, false)], &screen, 1), "our own window");
        assert!(!fullscreen(&[win(9, 24, full, false), win(8, 0, zoomed, false)], &screen, 1), "a system overlay");
        assert!(fullscreen(&[win(9, 24, full, true), win(8, 0, zoomed, false)], &screen, 1), "a slideshow above the rest");
        assert!(fullscreen(&[win(5, 3, zoomed, false), win(7, 0, full, false)], &screen, 1), "a floating panel over a fullscreen app");
        let mut clear = win(7, 0, full, false);
        clear.alpha = 0.0;
        assert!(!fullscreen(&[clear], &screen, 1), "an invisible overlay");
    }

    #[test]
    fn busy_notification_states_hide() {
        // QUNS_NOT_PRESENT, BUSY, RUNNING_D3D_FULL_SCREEN, PRESENTATION_MODE, ACCEPTS_NOTIFICATIONS, QUIET_TIME, APP
        let busy: Vec<bool> = (1..=7).map(quns_busy).collect();
        assert_eq!(busy, [false, true, true, true, false, false, false]);
    }

    #[test]
    fn linux_power_supplies() {
        let s = |kind: &str, online, status: &str, scope: &str| Supply { kind: kind.into(), online, status: status.into(), scope: scope.into() };
        assert!(on_battery(&[s("Mains", false, "", ""), s("Battery", false, "Discharging", "")]));
        assert!(!on_battery(&[s("Mains", true, "", ""), s("Battery", false, "Charging", "")]));
        assert!(!on_battery(&[s("Mains", true, "", ""), s("Battery", false, "Discharging", "")]), "a charger online");
        assert!(!on_battery(&[s("Battery", false, "Discharging", "Device")]), "a wireless mouse");
        assert!(!on_battery(&[]), "a desktop");
    }
}
