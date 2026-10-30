// The away recap (feature rust-core): back at the computer after 10 minutes or more with no input (idle, or locked),
// one of the game's own toasts says what happened meanwhile. Idle time is the OS's, read with no permission; what
// happened is the game before and after, kept in memory for this run only.
use crate::{planet::Core, Shown, State};
use planet_core::big::Big;
use serde_json::Value;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// away is this much idle time, in seconds
pub const AWAY: f64 = 600.0;
// back is input within this many seconds
const BACK: f64 = 10.0;
// the game as it was is kept while there's been input this recently, so what came after the last of it counts
const PRESENT: f64 = 30.0;

/// The game at one moment, as far as a recap goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snap {
    pub pl: Big,
    pub saga: u32,
    pub form: String,
    pub dragon: bool,
    pub ascend: bool,
}

/// What happened while away, in the order the recap tells it.
#[derive(Clone, Debug, PartialEq)]
pub enum Happened {
    Power(Big),
    Beat(String),
    Form(String),
    Dragon,
    Ascend,
}

/// what changed between `before` and `after`; `villain` names the saga villain at each count
pub fn happened(before: &Snap, after: &Snap, villain: impl Fn(u32) -> String) -> Vec<Happened> {
    let mut out = Vec::new();
    if after.pl > before.pl {
        out.push(Happened::Power(after.pl - before.pl));
    }
    out.extend((before.saga..after.saga).map(|i| Happened::Beat(villain(i))));
    if after.form != before.form && after.form != "base" {
        out.push(Happened::Form(after.form.clone()));
    }
    if after.dragon {
        out.push(Happened::Dragon);
    }
    if after.ascend && !before.ascend {
        out.push(Happened::Ascend);
    }
    out
}

/// The recap's one line, or none if nothing happened: "while you were away: +2.1B power, beat frieza, the dragon is
/// waiting". More than two villains beaten read as a count.
pub fn summary(events: &[Happened], short: impl Fn(Big) -> String) -> Option<String> {
    let beat: Vec<&str> = events.iter().filter_map(|e| if let Happened::Beat(n) = e { Some(n.as_str()) } else { None }).collect();
    let (mut parts, mut told) = (Vec::new(), false);
    for e in events {
        match e {
            Happened::Power(n) => parts.push(format!("+{} power", short(*n))),
            Happened::Beat(_) if told => {}
            Happened::Beat(_) => {
                told = true;
                parts.push(match beat[..] {
                    [one] => format!("beat {one}"),
                    [a, b] => format!("beat {a} and {b}"),
                    _ => format!("beat {} villains", beat.len()),
                });
            }
            Happened::Form(f) => parts.push(format!("reached {f}")),
            Happened::Dragon => parts.push("the dragon is waiting".into()),
            Happened::Ascend => parts.push("you can ascend".into()),
        }
    }
    (!parts.is_empty()).then(|| format!("while you were away: {}", parts.join(", ")))
}

/// Seconds since the last keyboard or mouse input, from the OS. None where it can't be read.
pub fn idle() -> Option<f64> {
    // debug builds only: the idle time from a file, to see the recap without leaving the computer for 10 minutes
    #[cfg(debug_assertions)]
    if let Some(f) = std::env::var_os("IDLE_FIGHTERZ_IDLE") {
        return std::fs::read_to_string(f).ok()?.trim().parse().ok();
    }
    os::idle()
}

/// Watches the idle time: every 15 s, every 5 s once away. Before going away, the game as it was at the last look
/// with someone there; back, the recap, if the setting is on and the widget shows.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let core = app.state::<Core>();
        let (mut present, mut away): (Option<Snap>, Option<Snap>) = (None, None);
        loop {
            let on = app.state::<State>().settings.lock().unwrap()["recap"].as_bool().unwrap_or(true);
            let idle = if on { idle() } else { None };
            match idle {
                None => (present, away) = (None, None),
                Some(t) if away.is_none() && t >= AWAY => away = present.take(),
                Some(t) if away.is_some() && t < BACK => {
                    let before = away.take().unwrap();
                    let after = core.snap();
                    if let (Some(after), true) = (&after, app.state::<Shown>().get()) {
                        let events = happened(&before, after, |i| crate::planet::villain(i).to_string());
                        if let Some(text) = summary(&events, crate::planet::short) {
                            crate::smoke(&format!("recap: {text}"));
                            let _ = app.emit_to("main", "notice", Value::String(text));
                        }
                    }
                    present = after;
                }
                Some(t) if t < PRESENT => present = core.snap(),
                Some(_) => {}
            }
            std::thread::sleep(Duration::from_secs(if away.is_some() { 5 } else { 15 }));
        }
    });
}

#[cfg(target_os = "macos")]
mod os {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, kind: u32) -> f64;
    }
    // the combined session state, any input event: no event tap, so no Input Monitoring
    pub fn idle() -> Option<f64> {
        Some(unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) })
    }
}

#[cfg(windows)]
mod os {
    #[repr(C)]
    struct LastInputInfo {
        size: u32,
        time: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }
    // locked, input goes to the secure desktop, so the session's last input stays put and the idle time grows
    pub fn idle() -> Option<f64> {
        let mut info = LastInputInfo { size: std::mem::size_of::<LastInputInfo>() as u32, time: 0 };
        (unsafe { GetLastInputInfo(&mut info) } != 0).then(|| unsafe { GetTickCount() }.wrapping_sub(info.time) as f64 / 1000.0)
    }
}

// X11: the MIT-SCREEN-SAVER extension's idle time. Wayland: GNOME's Mutter idle monitor, else the freedesktop
// screensaver's session idle time (KDE)
#[cfg(target_os = "linux")]
mod os {
    use std::{ptr, sync::Mutex};
    use x11_dl::{xlib::Xlib, xss::Xss};

    struct X(Xlib, Xss, *mut x11_dl::xlib::Display);
    unsafe impl Send for X {}
    static X11: Mutex<Option<Option<X>>> = Mutex::new(None);

    fn x11() -> Option<f64> {
        let mut x = X11.lock().unwrap();
        let x = x.get_or_insert_with(|| {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                return None;
            }
            let (lib, xss) = (Xlib::open().ok()?, Xss::open().ok()?);
            let dpy = unsafe { (lib.XOpenDisplay)(ptr::null()) };
            (!dpy.is_null()).then_some(X(lib, xss, dpy))
        });
        let X(lib, xss, dpy) = x.as_ref()?;
        unsafe {
            let info = (xss.XScreenSaverAllocInfo)();
            if info.is_null() {
                return None;
            }
            let ok = (xss.XScreenSaverQueryInfo)(*dpy, (lib.XDefaultRootWindow)(*dpy), info) != 0;
            let idle = (*info).idle as f64 / 1000.0;
            (lib.XFree)(info as *mut _);
            ok.then_some(idle)
        }
    }

    fn dbus() -> Option<f64> {
        let bus = zbus::blocking::Connection::session().ok()?;
        let mutter = bus.call_method(Some("org.gnome.Mutter.IdleMonitor"), "/org/gnome/Mutter/IdleMonitor/Core", Some("org.gnome.Mutter.IdleMonitor"), "GetIdletime", &());
        if let Some(ms) = mutter.ok().and_then(|r| r.body().deserialize::<u64>().ok()) {
            return Some(ms as f64 / 1000.0);
        }
        let saver = bus.call_method(Some("org.freedesktop.ScreenSaver"), "/org/freedesktop/ScreenSaver", Some("org.freedesktop.ScreenSaver"), "GetSessionIdleTime", &()).ok()?;
        Some(saver.body().deserialize::<u32>().ok()? as f64)
    }

    pub fn idle() -> Option<f64> {
        x11().or_else(dbus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(pl: f64, saga: u32, form: &str, dragon: bool, ascend: bool) -> Snap {
        Snap { pl: pl.into(), saga, form: form.into(), dragon, ascend }
    }
    fn short(n: Big) -> String {
        format!("{:.1}B", n.to_f64() / 1e9)
    }
    const SAGAS: [&str; 5] = ["raditz", "nappa", "vegeta", "frieza", "cell"];

    #[test]
    fn the_recap_tells_what_happened() {
        let events = happened(&snap(1e9, 3, "super saiyan", false, false), &snap(3.1e9, 4, "super saiyan", true, false), |i| SAGAS[i as usize].into());
        assert_eq!(events, [Happened::Power(2.1e9.into()), Happened::Beat("frieza".into()), Happened::Dragon]);
        assert_eq!(summary(&events, short).as_deref(), Some("while you were away: +2.1B power, beat frieza, the dragon is waiting"));
    }

    #[test]
    fn several_villains_and_a_form() {
        let events = [Happened::Beat("raditz".into()), Happened::Beat("nappa".into()), Happened::Form("super saiyan".into()), Happened::Ascend];
        assert_eq!(summary(&events, short).as_deref(), Some("while you were away: beat raditz and nappa, reached super saiyan, you can ascend"));
        let three = [Happened::Power(5e8.into()), Happened::Beat("a".into()), Happened::Beat("b".into()), Happened::Beat("c".into())];
        assert_eq!(summary(&three, short).as_deref(), Some("while you were away: +0.5B power, beat 3 villains"));
    }

    #[test]
    fn nothing_happened_no_recap() {
        let same = snap(1e9, 3, "base", false, true);
        assert_eq!(happened(&same, &same, |_| String::new()), []);
        assert_eq!(summary(&[], short), None);
        // power spent, back to base after an ascend, ascend already open: none of it news
        let after = snap(2e8, 3, "base", false, true);
        assert_eq!(happened(&snap(1e9, 3, "super saiyan", false, true), &after, |_| String::new()), []);
    }
}
