// The control centre: fullscreen with the HUD in a window of its own (settings: control, control_layout), on another
// display or floating above the game on this one. Its page is control.html, the HUD page laid out again (control.js);
// it plays through the planet_* calls capabilities/control.json grants it, and never draws or touches the planet.
// Closing it turns it off and the HUD comes back to the game; leaving fullscreen closes it.
use crate::prefs;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, Monitor, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "control";

/// the live planet in layout 3 fetches frames at most this often (the page asks every 300 ms)
const FEED_EVERY: Duration = Duration::from_millis(200);
/// the share card's one frame: the pixels asked for this soon after the card
const CARD_FRAME: Duration = Duration::from_secs(3);
/// the frame is saved once the window has stopped moving this long
const SETTLE: Duration = Duration::from_millis(800);

type Frame = (f64, f64, f64, f64);

#[derive(Default)]
pub struct Control {
    layout: AtomicU64,  // the layout the open window was sized for
    making: AtomicBool, // a window is being made (off the calling thread)
    feed: AtomicBool,   // layout 3: it may fetch frames
    fed: Mutex<Option<Instant>>,
    card: Mutex<Option<Instant>>,  // the share card asked for its frame
    moved: Mutex<Option<Instant>>, // the last move or resize, saved once it settles
    away: Mutex<Option<Frame>>,    // where it was on a display that went away, to go back to
}

/// what the window needs: `want` it (control on, fullscreen, the widget showing), whether one is open or being made,
/// and whether its layout changed
#[derive(Debug, PartialEq)]
pub enum Step {
    Open,
    Close,
    Resize,
    Stay,
}

pub fn step(want: bool, open: bool, making: bool, relayout: bool) -> Step {
    match (want, open) {
        (true, false) if !making => Step::Open,
        (false, true) => Step::Close,
        (true, true) if relayout => Step::Resize,
        _ => Step::Stay,
    }
}

/// Opens, resizes or closes the window to match the settings: open while control is on in fullscreen and the widget shows
pub fn sync(app: &AppHandle, s: &Value, shown: bool) {
    let Some(c) = app.try_state::<Control>() else { return };
    let want = crate::control_on(app, s) && shown;
    let layout = s["control_layout"].as_u64().unwrap_or(1);
    c.feed.store(want && layout == 3, Relaxed);
    let w = app.get_webview_window(LABEL);
    match step(want, w.is_some(), c.making.load(Relaxed), c.layout.load(Relaxed) != layout) {
        Step::Close => {
            keep_frame(app);
            if let Some(w) = w {
                let _ = w.destroy();
            }
        }
        Step::Resize => {
            c.layout.store(layout, Relaxed);
            if let Some(w) = w {
                resize(&w, layout);
            }
        }
        // made off this thread: a sync command (set_settings) that makes a window deadlocks on Windows. The settings
        // may change while it's made: once it's there, it's matched to them again (closed, if they no longer want it)
        Step::Open if !c.making.swap(true, Relaxed) => {
            let app = app.clone();
            std::thread::spawn(move || {
                let made = open(&app);
                app.state::<Control>().making.store(false, Relaxed);
                if let Err(e) = made {
                    eprintln!("control centre: no window ({e})");
                    crate::control_lost(&app);
                    return;
                }
                let s = app.state::<crate::State>().settings.lock().unwrap().clone();
                sync(&app, &s, app.state::<crate::Shown>().get());
            });
        }
        _ => {}
    }
}

/// planet_pixels from the control window: layout 3's live planet, at most every FEED_EVERY; or the share card's one frame
pub fn feed_ok<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(c) = app.try_state::<Control>() else { return false };
    if c.card.lock().unwrap().take().is_some_and(|t| t.elapsed() < CARD_FRAME) {
        return true;
    }
    let mut fed = c.fed.lock().unwrap();
    if !c.feed.load(Relaxed) || fed.is_some_and(|t| t.elapsed() < FEED_EVERY) {
        return false;
    }
    *fed = Some(Instant::now());
    true
}

/// the control window drew the share card: its next planet_pixels is the card's picture
pub fn carded<R: Runtime>(app: &AppHandle<R>) {
    if let Some(c) = app.try_state::<Control>() {
        *c.card.lock().unwrap() = Some(Instant::now());
    }
}

/// the window's size in points for a layout (prefs::CONTROL_LAYOUTS): tall, wide, or wider for three columns
pub fn control_size(layout: u64) -> (f64, f64) {
    match layout {
        3 | 4 => (900.0, 420.0),
        5 => (1100.0, 520.0),
        _ => (420.0, 900.0),
    }
}

/// its smallest size: what the layout still fits in
pub fn control_min(layout: u64) -> (f64, f64) {
    match layout {
        3 | 4 => (720.0, 360.0),
        5 => (900.0, 440.0),
        _ => (360.0, 560.0),
    }
}

// macOS lays displays out in points; Windows and X11 have one physical desktop (as place.rs)
const MAC: bool = cfg!(target_os = "macos");

fn area(m: &Monitor) -> (f64, f64, f64, f64) {
    let (a, k) = (m.work_area(), if MAC { m.scale_factor() } else { 1.0 });
    (a.position.x as f64 / k, a.position.y as f64 / k, a.size.width as f64 / k, a.size.height as f64 / k)
}

fn inside(a: (f64, f64, f64, f64), x: f64, y: f64) -> bool {
    x >= a.0 && y >= a.1 && x < a.0 + a.2 && y < a.1 + a.3
}

/// Where a new window goes, in points (macOS) or pixels: its saved frame if that's still on a display (the size only if
/// it was this layout's), else the middle of a display the game isn't on, else of the game's
pub fn place(saved: Option<(f64, f64, f64, f64, u64)>, layout: u64, areas: &[(f64, f64, f64, f64)], scales: &[f64], game: Option<usize>) -> Option<(f64, f64, f64, f64)> {
    let size = |i: usize| {
        let (w, h) = control_size(layout);
        let k = if MAC { 1.0 } else { scales.get(i).copied().unwrap_or(1.0) };
        (w * k, h * k)
    };
    let fit = |i: usize, x: f64, y: f64, (w, h): (f64, f64)| {
        let a = areas[i];
        let (w, h) = (w.min(a.2), h.min(a.3));
        (x.clamp(a.0, a.0 + a.2 - w), y.clamp(a.1, a.1 + a.3 - h), w, h)
    };
    if let Some((x, y, w, h, l)) = saved {
        let wh = if l == layout { (w, h) } else { size(0) };
        if let Some(i) = areas.iter().position(|a| inside(*a, x + wh.0.min(80.0), y + 20.0)) {
            let wh = if l == layout { wh } else { size(i) };
            return Some(fit(i, x, y, wh));
        }
    }
    let i = (0..areas.len()).find(|i| Some(*i) != game).or(game).filter(|i| *i < areas.len())?;
    let (a, (w, h)) = (areas[i], size(i));
    Some(fit(i, a.0 + (a.2 - w) / 2.0, a.1 + (a.3 - h) / 2.0, (w, h)))
}

fn open(app: &AppHandle) -> tauri::Result<()> {
    let s = app.state::<crate::State>().settings.lock().unwrap().clone();
    let layout = s["control_layout"].as_u64().unwrap_or(1);
    let main = app.get_webview_window("main");
    let monitors = main.as_ref().and_then(|m| m.available_monitors().ok()).unwrap_or_default();
    let game = main.as_ref().and_then(|m| m.current_monitor().ok().flatten());
    let gi = game.as_ref().and_then(|g| monitors.iter().position(|m| m.name() == g.name() && m.position() == g.position()));
    let areas: Vec<_> = monitors.iter().map(area).collect();
    let scales: Vec<_> = monitors.iter().map(|m| m.scale_factor()).collect();
    let (min_w, min_h) = control_min(layout);
    let (w, h) = control_size(layout);
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("control.html".into()))
        .title(format!("{} — control centre", app.package_info().name))
        .inner_size(w, h)
        .min_inner_size(min_w, min_h)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .focused(false) // the keys stay with the game until the panel is clicked
        .visible(false)
        .build()?;
    app.state::<Control>().layout.store(layout, Relaxed);
    if let Some(to) = place(prefs::control_frame(&s["control_frame"]), layout, &areas, &scales, gi) {
        set_frame(&w, to);
    }
    above(&w);
    w.show()
}

// above the game: the game's window is on top too, and a click on it would bring it in front of the panel
#[cfg(target_os = "macos")]
fn above<R: Runtime>(w: &WebviewWindow<R>) {
    struct Ptr(usize);
    unsafe impl Send for Ptr {}
    let Ok(ns) = w.ns_window() else { return };
    let ns = Ptr(ns as usize);
    let _ = w.run_on_main_thread(move || unsafe {
        let ns = ns;
        let _: () = objc2::msg_send![ns.0 as *mut objc2::runtime::AnyObject, setLevel: 4isize]; // floating is 3
    });
}
#[cfg(not(target_os = "macos"))]
fn above<R: Runtime>(_: &WebviewWindow<R>) {}

fn set_frame<R: Runtime>(w: &WebviewWindow<R>, (x, y, width, height): (f64, f64, f64, f64)) {
    if MAC {
        let _ = w.set_position(tauri::LogicalPosition::new(x, y));
        let _ = w.set_size(tauri::LogicalSize::new(width, height));
    } else {
        let _ = w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32));
        let _ = w.set_size(tauri::PhysicalSize::new(width.round() as u32, height.round() as u32));
    }
}

// a new layout: its size, where the window is
fn resize<R: Runtime>(w: &WebviewWindow<R>, layout: u64) {
    let ((width, height), (min_w, min_h)) = (control_size(layout), control_min(layout));
    let _ = w.set_min_size(Some(tauri::LogicalSize::new(min_w, min_h)));
    let _ = w.set_size(tauri::LogicalSize::new(width, height));
    if let Some(m) = w.current_monitor().ok().flatten() {
        if let Some((x, y, ..)) = frame(w) {
            let a = area(&m);
            let k = if MAC { 1.0 } else { m.scale_factor() };
            let (wd, ht) = ((width * k).min(a.2), (height * k).min(a.3));
            set_frame(w, (x.clamp(a.0, a.0 + a.2 - wd), y.clamp(a.1, a.1 + a.3 - ht), wd, ht));
        }
    }
}

fn frame<R: Runtime>(w: &WebviewWindow<R>) -> Option<(f64, f64, f64, f64)> {
    let (p, s) = (w.outer_position().ok()?, w.inner_size().ok()?);
    let k = if MAC { w.scale_factor().ok()? } else { 1.0 };
    Some((p.x as f64 / k, p.y as f64 / k, s.width as f64 / k, s.height as f64 / k))
}

/// The window's frame, into the settings: it opens there next time. While it's away from its display, the frame it had there
pub fn keep_frame(app: &AppHandle) {
    let (Some(w), Some(c)) = (app.get_webview_window(LABEL), app.try_state::<Control>()) else { return };
    let away = *c.away.lock().unwrap();
    if let Some((x, y, width, height)) = away.or_else(|| frame(&w)) {
        let f = json!({ "x": x.round(), "y": y.round(), "w": width.round(), "h": height.round(), "layout": c.layout.load(Relaxed) });
        if prefs::control_frame(&f).is_some() {
            crate::remember(app, json!({ "control_frame": f }));
        }
    }
}

/// The window moved or was resized: its frame is saved once it settles
pub fn moved(app: &AppHandle) {
    let Some(c) = app.try_state::<Control>() else { return };
    if c.moved.lock().unwrap().replace(Instant::now()).is_some() {
        return; // a save is already waiting
    }
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(SETTLE / 2);
        let c = app.state::<Control>();
        let mut at = c.moved.lock().unwrap();
        if at.is_some_and(|t| t.elapsed() >= SETTLE) {
            *at = None;
            drop(at);
            keep_frame(&app);
            return;
        }
    });
}

/// The displays changed: a window left off every display goes to the middle of the game's, and back to where it was
/// when that display returns
pub fn rehome(app: &AppHandle) {
    let (Some(w), Some(main), Some(c)) = (app.get_webview_window(LABEL), app.get_webview_window("main"), app.try_state::<Control>()) else { return };
    let (Some(now), Ok(ms)) = (frame(&w), w.available_monitors()) else { return };
    let on = |(x, y, ..): Frame| ms.iter().any(|m| inside(area(m), x + 40.0, y + 20.0));
    let mut away = c.away.lock().unwrap();
    match *away {
        Some(was) if on(was) => {
            *away = None;
            set_frame(&w, was);
        }
        _ if !on(now) => {
            let Some(g) = main.current_monitor().ok().flatten() else { return };
            away.get_or_insert(now);
            if let Some(to) = place(None, c.layout.load(Relaxed), &[area(&g)], &[g.scale_factor()], None) {
                set_frame(&w, to);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::CONTROL_LAYOUTS;

    const LEFT: (f64, f64, f64, f64) = (-1920.0, 0.0, 1920.0, 1080.0);
    const LAPTOP: (f64, f64, f64, f64) = (0.0, 25.0, 1512.0, 920.0);

    #[test]
    fn each_control_layout_has_its_window() {
        let sizes: Vec<_> = CONTROL_LAYOUTS.map(control_size).collect();
        assert_eq!(sizes, [(420.0, 900.0), (420.0, 900.0), (900.0, 420.0), (900.0, 420.0), (1100.0, 520.0)]);
        for l in CONTROL_LAYOUTS {
            let ((w, h), (mw, mh)) = (control_size(l), control_min(l));
            assert!(mw <= w && mh <= h, "layout {l} fits its own minimum");
        }
    }

    // the window follows the settings, and one still being made when they stop wanting it is closed once it's there
    #[test]
    fn the_window_follows_the_settings() {
        assert_eq!(step(true, false, false, false), Step::Open);
        assert_eq!(step(true, false, true, false), Step::Stay, "already being made");
        assert_eq!(step(false, true, false, false), Step::Close);
        assert_eq!(step(false, false, true, false), Step::Stay, "being made: matched again once it's there");
        assert_eq!(step(false, true, false, true), Step::Close, "made after the settings turned it off");
        assert_eq!(step(true, true, false, true), Step::Resize);
        assert_eq!(step(true, true, false, false), Step::Stay);
        assert_eq!(step(false, false, false, false), Step::Stay);
    }

    #[test]
    fn a_new_window_goes_where_the_game_isnt() {
        let k = if MAC { 1.0 } else { 2.0 };
        let (w, h) = (420.0 * k, 900.0 * k);
        assert_eq!(place(None, 1, &[LAPTOP, LEFT], &[2.0, 2.0], Some(0)), Some((LEFT.0 + (1920.0 - w) / 2.0, (1080.0 - h).max(0.0) / 2.0, w, h.min(1080.0))));
        let (w, h) = (900.0 * k, 420.0 * k);
        let at = place(None, 4, &[LAPTOP, LEFT], &[2.0, 2.0], Some(1)).unwrap();
        assert_eq!((at.2, at.3), (w.min(1512.0), h.min(920.0)), "on the laptop, the game on the left");
        assert!(inside(LAPTOP, at.0, at.1));
        let one = place(None, 5, &[LAPTOP], &[1.0], Some(0)).unwrap();
        assert!(inside(LAPTOP, one.0, one.1) && one.2 <= 1512.0 && one.3 <= 920.0, "one display: floats over the game");
        assert_eq!(place(None, 1, &[], &[], None), None);
    }

    #[test]
    fn a_saved_frame_comes_back_while_its_display_is_there() {
        let saved = Some((-1500.0, 100.0, 500.0, 800.0, 1));
        assert_eq!(place(saved, 1, &[LAPTOP, LEFT], &[1.0, 1.0], Some(0)), Some((-1500.0, 100.0, 500.0, 800.0)));
        let (w, h) = control_size(3);
        assert_eq!(place(Some((-1500.0, 100.0, 500.0, 800.0, 1)), 3, &[LAPTOP, LEFT], &[1.0, 1.0], Some(0)), Some((-1500.0, 100.0, w, h)), "another layout: its own size, same place");
        let gone = place(saved, 1, &[LAPTOP], &[1.0], Some(0)).unwrap();
        assert!(inside(LAPTOP, gone.0, gone.1), "its display is gone: on the game's");
        let big = place(Some((0.0, 25.0, 3000.0, 3000.0, 2)), 2, &[LAPTOP], &[1.0], Some(0)).unwrap();
        assert_eq!((big.2, big.3), (1512.0, 920.0), "never bigger than the display");
    }
}
