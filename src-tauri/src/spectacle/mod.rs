// The widget's spectacles (two settings, both off by default): during a raid the villain leaps out onto your windows
// and stands on their top edges for ten seconds, and a full-charge beam blasts on across the screen to its edge. Each
// plays in a click-through overlay that never takes focus and exists only while it plays: a non-activating NSPanel of
// Core Animation layers on macOS, layered tool windows on Windows. Linux has none yet. Neither plays while the OS is in
// Focus / Do Not Disturb (where it can be told), snoozed or in quiet hours, on battery saver, with reduced motion, in
// photo mode or with the widget hidden. Where the windows are comes from the window list any app may read: no Screen
// Recording or Accessibility permission, and nothing is asked for.
// drawn only where there's an overlay to draw in
#[cfg(all(target_os = "macos", feature = "native-renderer"))]
mod mac;
#[cfg_attr(not(any(all(target_os = "macos", feature = "native-renderer"), windows)), allow(dead_code))]
pub mod scene;
#[cfg(windows)]
mod win;

use crate::planet::Moment;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use tauri::{AppHandle, Manager};

/// whether this build can show them
pub const AVAILABLE: bool = cfg!(any(all(target_os = "macos", feature = "native-renderer"), windows));

// one at a time
static PLAYING: AtomicBool = AtomicBool::new(false);

/// What may stop a spectacle, as it is now.
#[derive(Clone, Copy, Debug, Default)]
pub struct Gate {
    pub on: bool,
    pub shown: bool,
    pub follow_focus: bool,
    pub focus: Option<bool>,
    pub snoozed: bool,
    pub quiet: bool,
    pub saving: bool,
    pub calm: bool,
    pub photo: bool,
}

/// It plays only when picked, on a shown widget, and when nothing asks for calm.
pub fn may_play(g: &Gate) -> bool {
    g.on && g.shown && !(g.follow_focus && g.focus == Some(true)) && !g.snoozed && !g.quiet && !g.saving && !g.calm && !g.photo
}

/// the setting each moment needs
pub fn key(m: &Moment) -> &'static str {
    match m {
        Moment::Raid { .. } => "raid_windows",
        Moment::Beam { .. } => "screen_beam",
    }
}

/// A raid or a full beam on the shown widget (from the tick thread): plays its spectacle, on a thread of its own,
/// if it's picked and may.
pub fn moment(app: &AppHandle, m: Moment) {
    let on = app.state::<crate::State>().settings.lock().unwrap()[key(&m)].as_bool().unwrap_or(false);
    if !on || !AVAILABLE || PLAYING.swap(true, Relaxed) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if may_play(&gate(&app)) {
            play(&app, m);
        }
        PLAYING.store(false, Relaxed);
    });
}

fn gate(app: &AppHandle) -> Gate {
    let s = app.state::<crate::State>().settings.lock().unwrap().clone();
    let follow_focus = s["follow_focus"].as_bool().unwrap_or(true);
    Gate {
        on: true,
        shown: app.state::<crate::Shown>().get(),
        follow_focus,
        // the probe may read a file: it's on this thread, not the tick's
        focus: if follow_focus { crate::focus::probe() } else { None },
        snoozed: crate::focus::snoozed(s["snooze_until"].as_f64().unwrap_or(0.0), crate::now_ms()),
        quiet: crate::focus::quiet_now(&s["quiet"]),
        // battery saver: picked, or balanced turned down on battery
        saving: crate::fps(app) <= 15.0,
        calm: app.try_state::<crate::planet::Core>().is_none_or(|c| c.calm()),
        photo: app.try_state::<crate::photo::Photo>().is_some_and(|p| p.on.load(Relaxed)),
    }
}

/// Where the overlay goes, in the screen's units (points on macOS, physical pixels on Windows): the widget's display,
/// the floor to stand on when no window will do (the bottom of its work area), the web view's top left, the units in a
/// CSS pixel, and in one of the game's pixels as the beam is drawn (the villain is drawn his size on the planet).
#[cfg_attr(not(any(all(target_os = "macos", feature = "native-renderer"), windows)), allow(dead_code))]
struct Place {
    display: crate::system::Rect,
    floor: scene::Edge,
    origin: (f64, f64),
    k: f64,
    px: f64,
}

#[cfg_attr(not(any(all(target_os = "macos", feature = "native-renderer"), windows)), allow(dead_code))]
fn place(app: &AppHandle) -> Option<Place> {
    let w = app.get_webview_window("main")?;
    let m = w.current_monitor().ok()??;
    let (pos, ws) = (w.inner_position().ok()?, w.scale_factor().ok()?);
    // macOS's units are points (tauri's physical positions are points × the scale); Windows' are physical pixels
    let (dm, dw, k) = if cfg!(target_os = "macos") { (m.scale_factor(), ws, 1.0) } else { (1.0, 1.0, ws) };
    let (mp, ms, wa) = (m.position(), m.size(), m.work_area());
    let display = crate::system::Rect { x: mp.x as f64 / dm, y: mp.y as f64 / dm, w: ms.width as f64 / dm, h: ms.height as f64 / dm };
    let (fx, fw) = (wa.position.x as f64 / dm, wa.size.width as f64 / dm);
    let floor = scene::Edge { x0: fx + 60.0 * k, x1: fx + fw - 60.0 * k, y: (wa.position.y as f64 + wa.size.height as f64) / dm - 4.0 * k };
    Some(Place { display, floor, origin: (pos.x as f64 / dw, pos.y as f64 / dw), k, px: 3.0 * k })
}

#[cfg_attr(not(any(all(target_os = "macos", feature = "native-renderer"), windows)), allow(dead_code))]
fn play(app: &AppHandle, m: Moment) {
    let Some(p) = place(app) else { return };
    let at = |x: f64, y: f64| (p.origin.0 + x * p.k, p.origin.1 + y * p.k);
    match m {
        Moment::Raid { id, sprite, x, y, scale } => {
            let from = at(x, y);
            // as big as he is on the planet: the widget's own scale, whatever its size
            let p = Place { px: scale * p.k, ..p };
            let imgs = scene::faces(sprite.w, sprite.h, &sprite.col);
            let (sw, sh) = (imgs[0].w as f64 * p.px, imgs[0].h as f64 * p.px);
            let wins = crate::system::windows().unwrap_or_default();
            let edges = scene::edges(&wins, p.display, std::process::id() as i64, sh + 8.0 * p.k, sw + 16.0 * p.k);
            let perches = scene::perches(&edges, from, p.floor);
            crate::smoke(&format!("spectacle: the villain on {} windows", perches.len()));
            let go = |app: &AppHandle| app.state::<crate::Shown>().get() && app.try_state::<crate::planet::Core>().is_some_and(|c| c.raid_on(id));
            // he leaves the widget while he's out, and is back on the planet once the overlay is gone
            let core = app.try_state::<crate::planet::Core>();
            core.as_ref().inspect(|c| c.boss_away(true));
            backend::raid(app, &p, from, &perches, imgs, go);
            core.as_ref().inspect(|c| c.boss_away(false));
        }
        Moment::Beam { x, y, dx, dy, kame, ff, kaio, ape } => {
            // as the game colours a beam: kaioken red, vegeta's final flash gold, a kamehameha blue, a galick gun violet
            let look = if kaio {
                scene::KAIO
            } else if ff && !kame {
                scene::FLASH
            } else if kame {
                scene::KAME
            } else {
                scene::GALICK
            };
            let width = 22.0 * p.px * if ape { 1.5 } else { 1.0 } * if ff { 1.25 } else { 1.0 };
            let line = scene::line(at(x, y), dx, dy, p.display, width);
            crate::smoke(&format!("spectacle: beam {:.0} long", line.len));
            backend::beam(app, &p, line, look, |app: &AppHandle| app.state::<crate::Shown>().get());
        }
    }
}

#[cfg(all(target_os = "macos", feature = "native-renderer"))]
use mac as backend;
#[cfg(windows)]
use win as backend;

// no overlay here yet: Linux (X11 could, Wayland lets no app place a window or see the others)
#[cfg(not(any(all(target_os = "macos", feature = "native-renderer"), windows)))]
mod backend {
    pub fn raid(_: &tauri::AppHandle, _: &super::Place, _: (f64, f64), _: &[(f64, f64)], _: [super::scene::Image; 2], _: impl Fn(&tauri::AppHandle) -> bool) {}
    pub fn beam(_: &tauri::AppHandle, _: &super::Place, _: super::scene::Line, _: super::scene::Look, _: impl Fn(&tauri::AppHandle) -> bool) {}
}

// the frames' pace
#[cfg(any(all(target_os = "macos", feature = "native-renderer"), windows))]
const RAID_FPS: f64 = 30.0;
#[cfg(any(all(target_os = "macos", feature = "native-renderer"), windows))]
const BEAM_FPS: f64 = 60.0;

// shared by the backends: a clock that ticks at `fps` until `secs` or `go` says stop; f gets the time
#[cfg(any(all(target_os = "macos", feature = "native-renderer"), windows))]
fn frames(secs: f64, fps: f64, mut go: impl FnMut() -> bool, mut f: impl FnMut(f64)) -> bool {
    let t0 = std::time::Instant::now();
    loop {
        let t = t0.elapsed().as_secs_f64();
        if t >= secs {
            return true;
        }
        if !go() {
            return false;
        }
        f(t);
        let next = ((t * fps).floor() + 1.0) / fps;
        std::thread::sleep(std::time::Duration::from_secs_f64((next - t0.elapsed().as_secs_f64()).max(0.002)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_plays_unless_picked_shown_and_nothing_asks_for_calm() {
        let ok = Gate { on: true, shown: true, follow_focus: true, ..Default::default() };
        assert!(may_play(&ok));
        assert!(!may_play(&Gate { on: false, ..ok }), "off by default");
        assert!(!may_play(&Gate { shown: false, ..ok }), "hidden");
        assert!(!may_play(&Gate { focus: Some(true), ..ok }), "in focus / do not disturb");
        assert!(may_play(&Gate { focus: Some(true), follow_focus: false, ..ok }), "focus not followed");
        assert!(may_play(&Gate { focus: None, ..ok }), "focus can't be told (macOS without full disk access)");
        assert!(!may_play(&Gate { snoozed: true, ..ok }));
        assert!(!may_play(&Gate { quiet: true, ..ok }), "quiet hours");
        assert!(!may_play(&Gate { saving: true, ..ok }), "battery saver");
        assert!(!may_play(&Gate { calm: true, ..ok }), "reduced motion");
        assert!(!may_play(&Gate { photo: true, ..ok }), "photo mode");
    }

    #[test]
    fn both_are_off_by_default() {
        let s: serde_json::Value = serde_json::from_str(crate::DEFAULTS).unwrap();
        assert_eq!((s["raid_windows"].as_bool(), s["screen_beam"].as_bool()), (Some(false), Some(false)));
    }
}
