// The spectacles on Windows: layered popup windows that take no clicks (WS_EX_TRANSPARENT), never activate
// (WS_EX_NOACTIVATE, shown with SW_SHOWNOACTIVATE), stay off the taskbar and Alt+Tab (WS_EX_TOOLWINDOW) and sit just
// under the widget. The villain is a window his own size, moved; the beam one window over its line, drawn once, and
// faded by the window's own alpha. Made on this thread when a spectacle starts, destroyed when it ends.
use super::{scene, Place};
use std::{ffi::c_void, sync::Once};
use tauri::{AppHandle, Manager};

type H = *mut c_void;

#[repr(C)]
struct WndClassExW {
    size: u32,
    style: u32,
    proc_: unsafe extern "system" fn(H, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: H,
    icon: H,
    cursor: H,
    background: H,
    menu: *const u16,
    class: *const u16,
    icon_sm: H,
}
#[repr(C)]
struct BitmapInfo {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bits: u16,
    compression: u32,
    size_image: u32,
    xppm: i32,
    yppm: i32,
    used: u32,
    important: u32,
    colours: [u32; 1],
}
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}
#[repr(C)]
struct Blend {
    op: u8,
    flags: u8,
    alpha: u8,
    format: u8,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(c: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: H,
        menu: H,
        inst: H,
        param: *const c_void,
    ) -> H;
    fn DestroyWindow(h: H) -> i32;
    fn DefWindowProcW(h: H, msg: u32, w: usize, l: isize) -> isize;
    fn ShowWindow(h: H, cmd: i32) -> i32;
    fn SetWindowPos(h: H, after: H, x: i32, y: i32, w: i32, ht: i32, flags: u32) -> i32;
    fn UpdateLayeredWindow(h: H, dst: H, pos: *const Point, size: *const Point, src: H, at: *const Point, key: u32, blend: *const Blend, flags: u32) -> i32;
    fn PeekMessageW(msg: *mut [usize; 8], h: H, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const [usize; 8]) -> i32;
    fn DispatchMessageW(msg: *const [usize; 8]) -> isize;
    fn GetDC(h: H) -> H;
    fn ReleaseDC(h: H, dc: H) -> i32;
}
#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(dc: H) -> H;
    fn CreateDIBSection(dc: H, info: *const BitmapInfo, usage: u32, bits: *mut *mut c_void, section: H, offset: u32) -> H;
    fn SelectObject(dc: H, obj: H) -> H;
    fn DeleteObject(obj: H) -> i32;
    fn DeleteDC(dc: H) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> H;
}

const CLASS: &[u16] = &[73, 70, 90, 83, 112, 101, 99, 116, 97, 99, 108, 101, 0]; // "IFZSpectacle"
                                                                                 // layered, click-through, never activated, not on the taskbar, topmost
const EX: u32 = 0x0008_0000 | 0x20 | 0x0800_0000 | 0x80 | 0x8;
const WS_POPUP: u32 = 0x8000_0000;
const SWP_NOSIZE: u32 = 0x1;
const SWP_NOZORDER: u32 = 0x4;
const SWP_NOACTIVATE: u32 = 0x10;
const ULW_ALPHA: u32 = 2;

unsafe extern "system" fn proc_(h: H, msg: u32, w: usize, l: isize) -> isize {
    DefWindowProcW(h, msg, w, l)
}

// one layered window with a 32-bit top-down DIB of its size
struct Layered {
    hwnd: H,
    dc: H,
    bmp: H,
    old: H,
    bits: *mut u8,
    w: i32,
    h: i32,
}

impl Layered {
    // hidden, at (x, y), just under `widget` (or topmost, if there's none)
    fn new(x: i32, y: i32, w: i32, h: i32, widget: H) -> Option<Layered> {
        static CLASSED: Once = Once::new();
        unsafe {
            let inst = GetModuleHandleW(std::ptr::null());
            CLASSED.call_once(|| {
                let c = WndClassExW {
                    size: std::mem::size_of::<WndClassExW>() as u32,
                    style: 0,
                    proc_,
                    cls_extra: 0,
                    wnd_extra: 0,
                    instance: inst,
                    icon: std::ptr::null_mut(),
                    cursor: std::ptr::null_mut(),
                    background: std::ptr::null_mut(),
                    menu: std::ptr::null(),
                    class: CLASS.as_ptr(),
                    icon_sm: std::ptr::null_mut(),
                };
                RegisterClassExW(&c);
            });
            let (w, h) = (w.max(1), h.max(1));
            let null = std::ptr::null_mut();
            let hwnd = CreateWindowExW(EX, CLASS.as_ptr(), std::ptr::null(), WS_POPUP, x, y, w, h, null, null, inst, std::ptr::null());
            if hwnd.is_null() {
                return None;
            }
            let screen = GetDC(null);
            let dc = CreateCompatibleDC(screen);
            ReleaseDC(null, screen);
            let info = BitmapInfo {
                size: 40,
                width: w,
                height: -h,
                planes: 1,
                bits: 32,
                compression: 0,
                size_image: 0,
                xppm: 0,
                yppm: 0,
                used: 0,
                important: 0,
                colours: [0],
            };
            let mut bits: *mut c_void = null;
            let bmp = CreateDIBSection(dc, &info, 0, &mut bits, null, 0);
            if dc.is_null() || bmp.is_null() || bits.is_null() {
                if !bmp.is_null() {
                    DeleteObject(bmp);
                }
                if !dc.is_null() {
                    DeleteDC(dc);
                }
                DestroyWindow(hwnd);
                return None;
            }
            let old = SelectObject(dc, bmp);
            let after = if widget.is_null() { -1isize as H } else { widget }; // HWND_TOPMOST
            SetWindowPos(hwnd, after, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
            Some(Layered { hwnd, dc, bmp, old, bits: bits.cast(), w, h })
        }
    }

    // a premultiplied RGBA picture of the window's size, at `alpha`
    fn put(&self, c: &scene::Canvas, alpha: f64) {
        let n = (self.w * self.h) as usize;
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, n * 4) };
        for (d, s) in dst.chunks_exact_mut(4).zip(c.rgba.chunks_exact(4)) {
            d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
        }
        let size = Point { x: self.w, y: self.h };
        let zero = Point { x: 0, y: 0 };
        let blend = Blend { op: 0, flags: 0, alpha: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, format: 1 };
        unsafe { UpdateLayeredWindow(self.hwnd, std::ptr::null_mut(), std::ptr::null(), &size, self.dc, &zero, 0, &blend, ULW_ALPHA) };
    }

    // the same picture, at another alpha
    fn alpha(&self, alpha: f64) {
        let blend = Blend { op: 0, flags: 0, alpha: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, format: 1 };
        let null = std::ptr::null_mut();
        unsafe { UpdateLayeredWindow(self.hwnd, null, std::ptr::null(), std::ptr::null(), null, std::ptr::null(), 0, &blend, ULW_ALPHA) };
    }

    fn move_to(&self, x: i32, y: i32) {
        unsafe { SetWindowPos(self.hwnd, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
    }

    fn show(&self) {
        unsafe { ShowWindow(self.hwnd, 4) }; // SW_SHOWNOACTIVATE
    }
}

impl Drop for Layered {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
            DestroyWindow(self.hwnd);
        }
    }
}

// this thread's windows answer their messages, so Windows never takes them for hung
fn pump() {
    let mut m = [0usize; 8];
    unsafe {
        while PeekMessageW(&mut m, std::ptr::null_mut(), 0, 0, 1) != 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}

fn widget(app: &AppHandle) -> H {
    let top = app.state::<crate::State>().settings.lock().unwrap()["on_top"].as_bool().unwrap_or(true);
    match app.get_webview_window("main").and_then(|w| w.hwnd().ok()) {
        Some(h) if top => h.0,
        _ => std::ptr::null_mut(),
    }
}

pub fn raid(app: &AppHandle, p: &Place, from: (f64, f64), perches: &[(f64, f64)], imgs: [scene::Image; 2], go: impl Fn(&AppHandle) -> bool) {
    let (w, h) = ((imgs[0].w as f64 * p.px).ceil() as usize, (imgs[0].h as f64 * p.px).ceil() as usize);
    let faces: Vec<scene::Canvas> = imgs
        .iter()
        .map(|img| {
            let mut c = scene::Canvas::new(w, h);
            c.feet(img, w as f64 / 2.0, h as f64, p.px, 1.0);
            c
        })
        .collect();
    let Some(win) = Layered::new(from.0 as i32, from.1 as i32, w as i32, h as i32, widget(app)) else { return };
    let top = p.display.y + h as f64;
    let mut last: Option<scene::Pose> = None;
    let done = super::frames(
        scene::RAID_SECS,
        super::RAID_FPS,
        || go(app),
        |t| {
            pump();
            let Some(pose) = scene::raid_pose(from, perches, p.px, top, t) else { return };
            if last == Some(pose) {
                return;
            }
            if last.is_none_or(|l| l.right != pose.right) {
                win.put(&faces[pose.right as usize], pose.alpha);
            } else if last.is_some_and(|l| l.alpha != pose.alpha) {
                win.alpha(pose.alpha);
            }
            win.move_to((pose.x - w as f64 / 2.0).round() as i32, (pose.y - h as f64).round() as i32);
            if last.is_none() {
                win.show();
            }
            last = Some(pose);
        },
    );
    if !done {
        let a = last.map_or(0.0, |q| q.alpha);
        super::frames(
            0.25,
            super::RAID_FPS,
            || true,
            |t| {
                pump();
                win.alpha(a * (1.0 - t / 0.25));
            },
        );
    }
}

pub fn beam(app: &AppHandle, p: &Place, line: scene::Line, look: scene::Look, go: impl Fn(&AppHandle) -> bool) {
    // the window covers the beam at its fullest, on its display: drawn once, then only faded
    let full = scene::Blast { len: line.len, width: line.width, alpha: 1.0 };
    let tip = (line.o.0 + line.d.0 * line.len, line.o.1 + line.d.1 * line.len);
    let reach = line.width;
    let d = p.display;
    let x0 = (line.o.0.min(tip.0) - reach).max(d.x).floor();
    let y0 = (line.o.1.min(tip.1) - reach).max(d.y).floor();
    let x1 = (line.o.0.max(tip.0) + reach).min(d.x + d.w).ceil();
    let y1 = (line.o.1.max(tip.1) + reach).min(d.y + d.h).ceil();
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut c = scene::Canvas::new(w, h);
    c.beam(&line, &full, &look, (x0, y0));
    let Some(win) = Layered::new(x0 as i32, y0 as i32, w as i32, h as i32, widget(app)) else { return };
    win.put(&c, 0.0);
    drop(c);
    win.show();
    super::frames(
        scene::BEAM_SECS,
        super::BEAM_FPS,
        || go(app),
        |t| {
            pump();
            if let Some(b) = scene::blast(&line, t) {
                // it bursts on rather than growing out: one picture, faded
                win.alpha(b.alpha * (t / 0.08).min(1.0));
            }
        },
    );
}
