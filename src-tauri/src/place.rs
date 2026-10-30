// Snap and stay: a floating widget let go near a screen edge or corner snaps to it, a small margin in; wherever it's
// left is remembered for that arrangement of displays (places.json, beside settings.json), and when the displays
// change it goes back to its place for the new arrangement, or is pulled back on screen. Wayland tells no app where
// its window is, so there none of this runs.
use crate::{Shown, State};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl R {
    fn right(&self) -> f64 {
        self.x + self.w
    }
    fn bottom(&self) -> f64 {
        self.y + self.h
    }
    fn overlap(&self, o: &R) -> f64 {
        let w = self.right().min(o.right()) - self.x.max(o.x);
        let h = self.bottom().min(o.bottom()) - self.y.max(o.y);
        w.max(0.0) * h.max(0.0)
    }
}

// in points: let go this close to an edge (or past it) and it snaps; and the gap it snaps to
pub const REACH: f64 = 40.0;
pub const MARGIN: f64 = 8.0;
// at most this many arrangements remembered
const KEPT: usize = 16;

// one axis: snapped to the near edge (or its far side, if closer), else left where it is
fn axis(pos: f64, len: f64, lo: f64, hi: f64, reach: f64, margin: f64) -> f64 {
    let (a, b) = (lo + margin, hi - margin - len);
    if b < a {
        return lo + ((hi - lo) - len) / 2.0; // wider than the screen: centred
    }
    let (da, db) = (pos - a, b - pos);
    if da <= reach && (da <= db || db > reach) {
        a
    } else if db <= reach {
        b
    } else {
        pos
    }
}

/// Where a window let go at `win` lands on the display whose work area is `area`: each axis snaps to an edge within
/// `reach` (or past it), so near a corner it snaps to both.
pub fn snap(win: R, area: R, reach: f64, margin: f64) -> (f64, f64) {
    (axis(win.x, win.w, area.x, area.right(), reach, margin), axis(win.y, win.h, area.y, area.bottom(), reach, margin))
}

/// The display `win` is mostly on (or the nearest, if it's on none), then `win` moved the least to be wholly on it.
pub fn clamp(win: R, areas: &[R]) -> Option<(f64, f64)> {
    let centre = (win.x + win.w / 2.0, win.y + win.h / 2.0);
    let dist = |a: &R| {
        let dx = (a.x - centre.0).max(centre.0 - a.right()).max(0.0);
        let dy = (a.y - centre.1).max(centre.1 - a.bottom()).max(0.0);
        dx * dx + dy * dy
    };
    let area = areas.iter().max_by(|a, b| win.overlap(a).total_cmp(&win.overlap(b)).then(dist(b).total_cmp(&dist(a))))?;
    let fit = |pos: f64, len: f64, lo: f64, hi: f64| if len >= hi - lo { lo + ((hi - lo) - len) / 2.0 } else { pos.clamp(lo, hi - len) };
    Some((fit(win.x, win.w, area.x, area.right()), fit(win.y, win.h, area.y, area.bottom())))
}

/// the display that holds most of `win`
pub fn home(win: R, areas: &[R]) -> Option<usize> {
    areas.iter().enumerate().max_by(|a, b| win.overlap(a.1).total_cmp(&win.overlap(b.1))).filter(|(_, a)| win.overlap(a) > 0.0).map(|(i, _)| i)
}

/// one arrangement of displays: each display's frame and scale, in a fixed order
pub fn layout_key(displays: &[(R, f64)]) -> String {
    let mut parts: Vec<String> = displays.iter().map(|(r, s)| format!("{}x{}@{},{}*{}", r.w.round(), r.h.round(), r.x.round(), r.y.round(), s)).collect();
    parts.sort();
    parts.join("|")
}

/// a place remembered: the latest per arrangement, at most KEPT arrangements, the oldest dropped
pub fn remember(places: &mut Value, key: &str, x: f64, y: f64) {
    let mut list: Vec<Value> = places.as_array().cloned().unwrap_or_default();
    list.retain(|p| p["k"] != key);
    list.push(json!({ "k": key, "x": x.round(), "y": y.round() }));
    let extra = list.len().saturating_sub(KEPT);
    list.drain(..extra);
    *places = Value::Array(list);
}

pub fn recall(places: &Value, key: &str) -> Option<(f64, f64)> {
    let p = places.as_array()?.iter().find(|p| p["k"] == key)?;
    Some((p["x"].as_f64()?, p["y"].as_f64()?))
}

// ── the window. macOS lays displays out in points, and a window's physical position is in its own display's scale,
// so there everything is in points; Windows and X11 have one physical desktop, so there it's physical pixels

struct Desk {
    areas: Vec<R>,
    scales: Vec<f64>,
    key: String,
}

fn desk<T: Runtime>(w: &WebviewWindow<T>) -> Option<Desk> {
    let ms = w.available_monitors().ok()?;
    if ms.is_empty() {
        return None;
    }
    let mac = cfg!(target_os = "macos");
    let mut displays = Vec::new();
    let mut areas = Vec::new();
    let mut scales = Vec::new();
    for m in &ms {
        let s = m.scale_factor();
        let k = if mac { s } else { 1.0 };
        let (p, z, a) = (m.position(), m.size(), m.work_area());
        displays.push((R { x: p.x as f64 / k, y: p.y as f64 / k, w: z.width as f64 / k, h: z.height as f64 / k }, s));
        areas.push(R { x: a.position.x as f64 / k, y: a.position.y as f64 / k, w: a.size.width as f64 / k, h: a.size.height as f64 / k });
        scales.push(s);
    }
    Some(Desk { key: layout_key(&displays), areas, scales })
}

fn frame<T: Runtime>(w: &WebviewWindow<T>) -> Option<R> {
    let (p, s) = (w.outer_position().ok()?, w.outer_size().ok()?);
    let k = if cfg!(target_os = "macos") { w.scale_factor().ok()? } else { 1.0 };
    Some(R { x: p.x as f64 / k, y: p.y as f64 / k, w: s.width as f64 / k, h: s.height as f64 / k })
}

fn put<T: Runtime>(w: &WebviewWindow<T>, at: R, x: f64, y: f64) {
    if (x - at.x).abs() < 1.0 && (y - at.y).abs() < 1.0 {
        return;
    }
    let _ = if cfg!(target_os = "macos") {
        w.set_position(tauri::LogicalPosition::new(x, y))
    } else {
        w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32))
    };
}

/// the move watcher: woken by every move of the main window
#[derive(Default)]
pub struct Mover {
    moved: Mutex<Option<Instant>>,
    wake: Condvar,
}

impl Mover {
    pub fn moved(&self) {
        *self.moved.lock().unwrap() = Some(Instant::now());
        self.wake.notify_all();
    }
}

pub struct Places {
    file: PathBuf,
    list: Mutex<Value>,
}

impl Places {
    pub fn load(file: PathBuf) -> Self {
        let list = fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(Value::is_array).unwrap_or(json!([]));
        Places { file, list: Mutex::new(list) }
    }
    fn keep(&self, key: &str, x: f64, y: f64) {
        let mut list = self.list.lock().unwrap();
        if recall(&list, key) == Some((x.round(), y.round())) {
            return;
        }
        remember(&mut list, key, x, y);
        let _ = fs::create_dir_all(self.file.parent().unwrap());
        let _ = fs::write(&self.file, serde_json::to_vec(&*list).unwrap());
    }
}

// the main button, held: the drag isn't over. Read from state any app may read (no event tap, no permission)
fn button_down() -> bool {
    os::button_down()
}

/// Starts the watchers: one for the end of each drag, one for display changes (every 3 s while the widget shows).
pub fn watch(app: &AppHandle, file: PathBuf) {
    app.manage(Places::load(file));
    let mover = Arc::new(Mover::default());
    app.manage(mover.clone());
    let a = app.clone();
    std::thread::spawn(move || loop {
        // a drag is over once the button is up and nothing has moved for a moment
        {
            let mut m = mover.moved.lock().unwrap();
            while m.is_none() {
                m = mover.wake.wait(m).unwrap();
            }
        }
        loop {
            std::thread::sleep(Duration::from_millis(150));
            let quiet = mover.moved.lock().unwrap().is_some_and(|t| t.elapsed() >= Duration::from_millis(250));
            if quiet && !button_down() {
                break;
            }
        }
        *mover.moved.lock().unwrap() = None;
        let h = a.clone();
        let _ = a.run_on_main_thread(move || settle(&h, true));
    });
    let a = app.clone();
    std::thread::spawn(move || {
        let mut last = String::new();
        loop {
            a.state::<Shown>().wait();
            let (tx, rx) = std::sync::mpsc::channel();
            let h = a.clone();
            let _ = a.run_on_main_thread(move || {
                let _ = tx.send(h.get_webview_window("main").and_then(|w| desk(&w)).map(|d| d.key));
            });
            if let Ok(Some(key)) = rx.recv_timeout(Duration::from_secs(5)) {
                if key != last {
                    let first = last.is_empty();
                    last = key;
                    let h = a.clone();
                    let _ = a.run_on_main_thread(move || arrive(&h, first));
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

fn floating(app: &AppHandle) -> bool {
    let s = app.state::<State>();
    s.transparent && s.settings.lock().unwrap()["mode"] != "window"
}

/// After a drag (`dragged`) or a resize: snapped if floating and let go near an edge, kept on screen, remembered.
pub fn settle(app: &AppHandle, dragged: bool) {
    let (Some(w), Some(places)) = (app.get_webview_window("main"), app.try_state::<Places>()) else { return };
    let (Some(d), Some(at)) = (desk(&w), frame(&w)) else { return };
    let Some((mut x, mut y)) = clamp(at, &d.areas) else { return };
    if dragged && floating(app) {
        if let Some(i) = home(at, &d.areas) {
            let k = if cfg!(target_os = "macos") { 1.0 } else { d.scales[i] };
            (x, y) = snap(R { x, y, ..at }, d.areas[i], REACH * k, MARGIN * k);
            // a snap can't leave the display it snapped to
            (x, y) = clamp(R { x, y, ..at }, &d.areas[i..=i]).unwrap_or((x, y));
        }
    }
    put(&w, at, x, y);
    places.keep(&d.key, x, y);
}

// the displays changed (or this is the first look): the place kept for this arrangement, else on screen where it is
fn arrive(app: &AppHandle, first: bool) {
    let Some(w) = app.get_webview_window("main") else { return };
    let (Some(d), Some(at)) = (desk(&w), frame(&w)) else { return };
    let kept = recall(&app.state::<Places>().list.lock().unwrap(), &d.key);
    let want = match kept {
        Some((x, y)) => R { x, y, ..at },
        None => at,
    };
    if let Some((x, y)) = clamp(want, &d.areas) {
        put(&w, at, x, y);
        if kept.is_none() && !first {
            app.state::<Places>().keep(&d.key, x, y);
        }
    }
}

#[cfg(target_os = "macos")]
mod os {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    }
    pub fn button_down() -> bool {
        unsafe { CGEventSourceButtonState(0, 0) } // the combined session state, the left button
    }
}

#[cfg(windows)]
mod os {
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    // either button: with the buttons swapped, the primary is the physical right one
    pub fn button_down() -> bool {
        unsafe { GetAsyncKeyState(0x01) < 0 || GetAsyncKeyState(0x02) < 0 }
    }
}

#[cfg(target_os = "linux")]
mod os {
    use std::{ptr, sync::Mutex};
    use x11_dl::xlib::{self, Xlib};

    struct X(Xlib, *mut xlib::Display);
    unsafe impl Send for X {}
    static X11: Mutex<Option<Option<X>>> = Mutex::new(None);

    pub fn button_down() -> bool {
        let mut x = X11.lock().unwrap();
        let x = x.get_or_insert_with(|| {
            let lib = Xlib::open().ok()?;
            let dpy = unsafe { (lib.XOpenDisplay)(ptr::null()) };
            (!dpy.is_null()).then_some(X(lib, dpy))
        });
        let Some(X(lib, dpy)) = x.as_ref() else { return false };
        let (mut r, mut c, mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0, 0, 0);
        unsafe { (lib.XQueryPointer)(*dpy, (lib.XDefaultRootWindow)(*dpy), &mut r, &mut c, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask) };
        mask & (xlib::Button1Mask | xlib::Button3Mask) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: R = R { x: 0.0, y: 25.0, w: 1512.0, h: 920.0 }; // a work area under the menu bar, over the dock
    const WIN: R = R { x: 600.0, y: 300.0, w: 400.0, h: 400.0 };

    #[test]
    fn let_go_near_an_edge_it_snaps_to_it() {
        assert_eq!(snap(WIN, SCREEN, REACH, MARGIN), (600.0, 300.0), "the middle: left where it is");
        assert_eq!(snap(R { x: 30.0, ..WIN }, SCREEN, REACH, MARGIN), (8.0, 300.0), "near the left edge");
        assert_eq!(snap(R { x: -120.0, ..WIN }, SCREEN, REACH, MARGIN), (8.0, 300.0), "past it");
        assert_eq!(snap(R { x: 1090.0, ..WIN }, SCREEN, REACH, MARGIN), (1104.0, 300.0), "near the right edge");
        assert_eq!(snap(R { x: 1090.0, y: 520.0, ..WIN }, SCREEN, REACH, MARGIN), (1104.0, 537.0), "near a corner: both");
        assert_eq!(snap(R { x: 20.0, y: 40.0, ..WIN }, SCREEN, REACH, MARGIN), (8.0, 33.0), "the top left corner");
        let narrow = R { x: 0.0, y: 0.0, w: 450.0, h: 900.0 };
        assert_eq!(snap(R { x: 20.0, ..WIN }, narrow, REACH, MARGIN).0, 8.0, "both edges in reach: the nearer");
        assert_eq!(snap(R { x: 40.0, ..WIN }, narrow, REACH, MARGIN).0, 42.0, "the right edge was nearer");
        let tiny = R { x: 0.0, y: 0.0, w: 300.0, h: 300.0 };
        assert_eq!(snap(WIN, tiny, REACH, MARGIN), (-50.0, -50.0), "bigger than the screen: centred");
    }

    #[test]
    fn a_window_off_screen_comes_back() {
        let laptop = R { x: 0.0, y: 25.0, w: 1512.0, h: 920.0 };
        let external = R { x: 1512.0, y: 0.0, w: 2560.0, h: 1415.0 };
        assert_eq!(clamp(R { x: 3000.0, y: 900.0, ..WIN }, &[laptop, external]), Some((3000.0, 900.0)), "on the external: stays");
        // the external unplugged: back onto the laptop, at its nearest edge
        assert_eq!(clamp(R { x: 3000.0, y: 900.0, ..WIN }, &[laptop]), Some((1112.0, 545.0)));
        assert_eq!(clamp(R { x: 1400.0, y: 100.0, ..WIN }, &[laptop, external]), Some((1512.0, 100.0)), "mostly on the external");
        assert_eq!(clamp(R { x: -500.0, y: -500.0, ..WIN }, &[laptop, external]), Some((0.0, 25.0)));
        assert_eq!(clamp(WIN, &[]), None);
    }

    #[test]
    fn places_are_kept_per_arrangement() {
        let one = layout_key(&[(R { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }, 2.0)]);
        let two = layout_key(&[(R { x: 1512.0, y: 0.0, w: 2560.0, h: 1440.0 }, 1.0), (R { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }, 2.0)]);
        let same = layout_key(&[(R { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }, 2.0), (R { x: 1512.0, y: 0.0, w: 2560.0, h: 1440.0 }, 1.0)]);
        assert_ne!(one, two);
        assert_eq!(two, same, "the order displays are listed in doesn't matter");
        let mut places = json!([]);
        remember(&mut places, &one, 8.4, 33.0);
        remember(&mut places, &two, 3000.0, 900.0);
        remember(&mut places, &one, 1104.0, 33.0);
        assert_eq!(recall(&places, &one), Some((1104.0, 33.0)));
        assert_eq!(recall(&places, &two), Some((3000.0, 900.0)));
        assert_eq!(recall(&places, "nope"), None);
        for k in 0..40 {
            remember(&mut places, &format!("k{k}"), 0.0, 0.0);
        }
        assert_eq!(places.as_array().unwrap().len(), KEPT);
        assert_eq!(recall(&places, &one), None, "the oldest go");
    }
}
