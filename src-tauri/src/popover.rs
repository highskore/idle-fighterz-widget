// The menu-bar popover (setting `tray_style`: "popover"): a click on the tray icon opens a small frameless window under
// it (popover.html) in place of the native menu, which right-click (or ⌃-click on macOS) still opens. It hides when it
// loses focus or on Esc, and asks the app for nothing while hidden. Not on Linux: AppIndicator reports no clicks.
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, PhysicalPosition, Rect as TrayRect, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "popover";
/// the popover's size in points
pub const SIZE: (f64, f64) = (320.0, 444.0);
// points between the icon (or the taskbar) and the popover, and kept clear of the screen's edges
const GAP: f64 = 6.0;
const MARGIN: f64 = GAP;
// a click on the icon this soon after the popover closed on losing focus was the click that closed it
const REOPEN: Duration = Duration::from_millis(350);

/// A rectangle in one display's points.
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
    fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    fn holds(&self, (x, y): (f64, f64)) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// which edge the menu bar or taskbar holding the icon is on
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// The bar the icon sits in: the side of the screen the work area gives up that the icon is in. macOS: the menu bar,
/// on top (taller on a display with a notch, which the work area accounts for). Windows: the taskbar, wherever it is.
/// An icon inside the work area (an auto-hiding taskbar, or the tray's overflow flyout): the screen edge it's nearest.
pub fn edge(icon: R, screen: R, work: R) -> Edge {
    let (cx, cy) = icon.centre();
    if cy < work.y {
        return Edge::Top;
    }
    if cy >= work.bottom() {
        return Edge::Bottom;
    }
    if cx < work.x {
        return Edge::Left;
    }
    if cx >= work.right() {
        return Edge::Right;
    }
    let near = [(cy - screen.y, Edge::Top), (screen.bottom() - cy, Edge::Bottom), (cx - screen.x, Edge::Left), (screen.right() - cx, Edge::Right)];
    near.iter().fold(near[0], |a, b| if b.0 < a.0 { *b } else { a }).1
}

/// Where the popover's top-left corner goes: centred on the icon and clear of the bar it's in, then kept inside the
/// work area (an icon near a corner pushes it along, never off the display or under the bar).
pub fn place(icon: R, size: (f64, f64), screen: R, work: R) -> (f64, f64) {
    let (w, h) = size;
    let (cx, cy) = icon.centre();
    let (x, y) = match edge(icon, screen, work) {
        Edge::Top => (cx - w / 2.0, icon.bottom().max(work.y) + GAP),
        Edge::Bottom => (cx - w / 2.0, icon.y.min(work.bottom()) - h - GAP),
        Edge::Left => (icon.right().max(work.x) + GAP, cy - h / 2.0),
        Edge::Right => (icon.x.min(work.right()) - w - GAP, cy - h / 2.0),
    };
    let fit = |v: f64, lo: f64, hi: f64| if hi < lo { lo } else { v.clamp(lo, hi) };
    (fit(x, work.x + MARGIN, work.right() - w - MARGIN), fit(y, work.y + MARGIN, work.bottom() - h - MARGIN))
}

/// A display as the window system reports it, in its own physical pixels, with its scale factor.
#[derive(Clone, Copy, Debug)]
pub struct Display {
    pub screen: R,
    pub work: R,
    pub scale: f64,
}

/// The display the icon is on, from its rect in physical pixels: the one whose points hold the icon's centre (each
/// display's physical coordinates are its points times its own scale on macOS), else the nearest. With it, the icon,
/// screen and work area in that display's points.
pub fn display_of(icon: R, displays: &[Display]) -> Option<(usize, R, R, R)> {
    let pts = |r: R, s: f64| R { x: r.x / s, y: r.y / s, w: r.w / s, h: r.h / s };
    let found = displays.iter().position(|d| pts(d.screen, d.scale).holds(pts(icon, d.scale).centre()));
    let k = found.or_else(|| {
        let (cx, cy) = icon.centre();
        let dist = |d: &Display| {
            let (x, y) = d.screen.centre();
            (x - cx).powi(2) + (y - cy).powi(2)
        };
        (0..displays.len()).min_by(|a, b| dist(&displays[*a]).total_cmp(&dist(&displays[*b])))
    })?;
    let d = displays[k];
    Some((k, pts(icon, d.scale), pts(d.screen, d.scale), pts(d.work, d.scale)))
}

/// when the popover last closed on losing focus
pub struct Popover {
    closed: std::sync::Mutex<Option<Instant>>,
}

pub fn init(app: &AppHandle) {
    app.manage(Popover { closed: std::sync::Mutex::new(None) });
}

fn window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Ok(w);
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("popover.html".into()))
        .title("popover")
        .inner_size(SIZE.0, SIZE.1)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .visible(false)
        .focused(true)
        .build()
}

fn displays(app: &AppHandle) -> Vec<Display> {
    let r = |p: tauri::PhysicalPosition<i32>, s: tauri::PhysicalSize<u32>| R { x: p.x as f64, y: p.y as f64, w: s.width as f64, h: s.height as f64 };
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Display { screen: r(*m.position(), *m.size()), work: r(m.work_area().position, m.work_area().size), scale: m.scale_factor() })
        .collect()
}

/// The icon's rect from a tray event, in physical pixels.
pub fn icon_rect(rect: &TrayRect) -> R {
    let p = rect.position.to_physical::<f64>(1.0);
    let s = rect.size.to_physical::<f64>(1.0);
    R { x: p.x, y: p.y, w: s.width, h: s.height }
}

/// Opens the popover under the icon at `icon` (physical pixels), or closes it if it's open (or just closed by this
/// very click taking the focus from it).
pub fn toggle(app: &AppHandle, icon: R) {
    if let Some(w) = app.get_webview_window(LABEL) {
        if w.is_visible().unwrap_or(false) {
            hide(app);
            return;
        }
    }
    let just = app.state::<Popover>().closed.lock().unwrap().take().is_some_and(|t| t.elapsed() < REOPEN);
    if !just {
        show(app, icon);
    }
}

pub fn show(app: &AppHandle, icon: R) {
    let Ok(w) = window(app) else { return };
    #[cfg(debug_assertions)]
    eprintln!("popover: show under {icon:?}");
    let all = displays(app);
    if let Some((k, icon, screen, work)) = display_of(icon, &all) {
        let (x, y) = place(icon, SIZE, screen, work);
        // in the target display's pixels. macOS converts them with the scale of the display the window is on now, so
        // a window coming from a display of another scale is placed again once there
        let scale = all[k].scale;
        let at = PhysicalPosition::new((x * scale).round() as i32, (y * scale).round() as i32);
        let _ = w.set_position(at);
        if (w.scale_factor().unwrap_or(scale) - scale).abs() > f64::EPSILON {
            let _ = w.set_position(at);
        }
    }
    let _ = w.show();
    let _ = w.set_focus();
    #[cfg(target_os = "windows")]
    let _ = w.with_webview(|v| unsafe {
        let _ = v.controller().SetIsVisible(true);
    });
    let _ = tauri::Emitter::emit_to(&w, LABEL, "popover", true);
}

/// The (hidden) popover window moved to just under the icon, for the native menu to pop up from at its corner.
#[cfg(target_os = "macos")]
pub fn anchor_for_menu(app: &AppHandle, icon: R) -> Option<tauri::WebviewWindow> {
    let w = window(app).ok()?;
    let all = displays(app);
    let (k, icon_pts, _, work) = display_of(icon, &all)?;
    let scale = all[k].scale;
    let (x, y) = (icon_pts.x, icon_pts.bottom().max(work.y));
    let _ = w.set_position(PhysicalPosition::new((x * scale).round() as i32, (y * scale).round() as i32));
    Some(w)
}

/// whether ⌃ is held now (the session's modifier state: no event tap, no permission)
#[cfg(target_os = "macos")]
pub fn control_held() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceFlagsState(state: i32) -> u64;
    }
    unsafe { CGEventSourceFlagsState(0) & 0x0004_0000 != 0 } // combined session state, kCGEventFlagMaskControl
}

/// Hides the popover; its page stops asking for anything until it shows again.
pub fn hide(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    let _ = tauri::Emitter::emit_to(&w, LABEL, "popover", false);
    let _ = w.hide();
    #[cfg(target_os = "windows")]
    let _ = w.with_webview(|v| unsafe {
        let _ = v.controller().SetIsVisible(false);
    });
}

/// it lost focus: closes, and remembers when, so the icon click that took the focus doesn't open it again
pub fn blurred(app: &AppHandle) {
    let open = app.get_webview_window(LABEL).is_some_and(|w| w.is_visible().unwrap_or(false));
    #[cfg(debug_assertions)]
    eprintln!("popover: lost focus (open: {open})");
    if open {
        *app.state::<Popover>().closed.lock().unwrap() = Some(Instant::now());
        hide(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: R = R { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 };
    // a notched MacBook's menu bar is 37 points tall, the icon's button a little shorter
    const MAC_WORK: R = R { x: 0.0, y: 37.0, w: 1512.0, h: 945.0 };
    const POP: (f64, f64) = (320.0, 466.0);

    #[test]
    fn under_a_menu_bar_icon_centred_and_below_the_bar() {
        let icon = R { x: 1100.0, y: 0.0, w: 30.0, h: 34.0 };
        assert_eq!(edge(icon, MAC, MAC_WORK), Edge::Top);
        let (x, y) = place(icon, POP, MAC, MAC_WORK);
        assert_eq!(x, 1115.0 - 160.0);
        assert_eq!(y, 37.0 + GAP, "below the notch-height menu bar, not just the icon");
    }

    #[test]
    fn an_icon_near_the_corner_keeps_the_popover_on_the_display() {
        let icon = R { x: 1490.0, y: 0.0, w: 22.0, h: 24.0 };
        let (x, _) = place(icon, POP, MAC, MAC_WORK);
        assert_eq!(x, 1512.0 - 320.0 - MARGIN);
        let left = R { x: 2.0, y: 0.0, w: 22.0, h: 24.0 };
        assert_eq!(place(left, POP, MAC, MAC_WORK).0, MARGIN);
    }

    #[test]
    fn windows_taskbars_on_each_edge() {
        let screen = R { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };
        // bottom (the default): above the taskbar, centred on the icon
        let work = R { x: 0.0, y: 0.0, w: 1920.0, h: 1032.0 };
        let icon = R { x: 1700.0, y: 1040.0, w: 32.0, h: 40.0 };
        assert_eq!(edge(icon, screen, work), Edge::Bottom);
        assert_eq!(place(icon, POP, screen, work), (1716.0 - 160.0, 1032.0 - 466.0 - GAP));
        // in the bottom-right corner: pushed left, still above the taskbar
        let corner = R { x: 1880.0, y: 1040.0, w: 32.0, h: 40.0 };
        assert_eq!(place(corner, POP, screen, work), (1920.0 - 320.0 - MARGIN, 1032.0 - 466.0 - GAP));
        // top
        let work = R { x: 0.0, y: 48.0, w: 1920.0, h: 1032.0 };
        let icon = R { x: 1700.0, y: 8.0, w: 32.0, h: 32.0 };
        assert_eq!(place(icon, POP, screen, work), (1716.0 - 160.0, 48.0 + GAP));
        // left: beside it, centred on the icon's height, kept on screen
        let work = R { x: 60.0, y: 0.0, w: 1860.0, h: 1080.0 };
        let icon = R { x: 14.0, y: 1000.0, w: 32.0, h: 32.0 };
        assert_eq!(edge(icon, screen, work), Edge::Left);
        assert_eq!(place(icon, POP, screen, work), (60.0 + GAP, 1080.0 - 466.0 - MARGIN));
        // right
        let work = R { x: 0.0, y: 0.0, w: 1860.0, h: 1080.0 };
        let icon = R { x: 1874.0, y: 500.0, w: 32.0, h: 32.0 };
        assert_eq!(place(icon, POP, screen, work), (1860.0 - 320.0 - GAP, 516.0 - 233.0));
    }

    #[test]
    fn an_icon_inside_the_work_area_goes_by_the_nearest_edge() {
        // an auto-hiding taskbar gives up no work area; the overflow flyout sits above the taskbar
        let screen = R { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };
        let icon = R { x: 1700.0, y: 1000.0, w: 32.0, h: 32.0 };
        assert_eq!(edge(icon, screen, screen), Edge::Bottom);
        let (_, y) = place(icon, POP, screen, screen);
        assert_eq!(y, 1000.0 - 466.0 - GAP);
    }

    #[test]
    fn a_popover_bigger_than_the_work_area_starts_at_its_corner() {
        let tiny = R { x: 0.0, y: 24.0, w: 300.0, h: 300.0 };
        let icon = R { x: 150.0, y: 0.0, w: 20.0, h: 24.0 };
        assert_eq!(place(icon, POP, R { x: 0.0, y: 0.0, w: 300.0, h: 324.0 }, tiny), (MARGIN, 24.0 + MARGIN));
    }

    #[test]
    fn the_icon_finds_its_display_among_several() {
        // a 2× laptop and, to its right, a 1× external display; macOS reports each in its own scale's pixels
        let laptop = Display { screen: R { x: 0.0, y: 0.0, w: 3024.0, h: 1964.0 }, work: R { x: 0.0, y: 74.0, w: 3024.0, h: 1890.0 }, scale: 2.0 };
        let external = Display { screen: R { x: 1512.0, y: 0.0, w: 2560.0, h: 1440.0 }, work: R { x: 1512.0, y: 25.0, w: 2560.0, h: 1415.0 }, scale: 1.0 };
        let all = [laptop, external];
        // the icon in the external display's menu bar, in its pixels
        let (k, icon, screen, work) = display_of(R { x: 3900.0, y: 0.0, w: 30.0, h: 24.0 }, &all).unwrap();
        assert_eq!(k, 1);
        assert_eq!((icon.x, screen.x, work.y), (3900.0, 1512.0, 25.0));
        let (x, y) = place(icon, POP, screen, work);
        assert!(x + POP.0 <= screen.right() - MARGIN && x >= 1512.0, "{x}");
        assert_eq!(y, 25.0 + GAP);
        // the icon in the laptop's (notched) menu bar, in its 2× pixels
        let (k, icon, _, work) = display_of(R { x: 2400.0, y: 0.0, w: 60.0, h: 68.0 }, &all).unwrap();
        assert_eq!((k, icon.x, work.y), (0, 1200.0, 37.0));
        // off every display (a stale rect): the nearest one
        assert_eq!(display_of(R { x: 9000.0, y: 0.0, w: 30.0, h: 24.0 }, &all).unwrap().0, 1);
        assert!(display_of(R { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, &[]).is_none());
    }
}
