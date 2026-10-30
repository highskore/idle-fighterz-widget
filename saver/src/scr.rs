// The Windows screensaver (IdleFighterZ.scr is this exe renamed). Windows runs it with /s (full screen), /p <hwnd> (the
// little preview in the settings dialog) or /c (settings: there are none). GDI draws the picture `Saver` composes (the
// planet over the site's sky and cloud sea) scaled up nearest-neighbour; a 33 ms timer polls it, so it never draws more
// than 30 frames a second.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("idle-fighterz-saver is the Windows screensaver; macOS has saver/macos");
}

#[cfg(windows)]
fn main() {
    win::main();
}

#[cfg(windows)]
mod win {
    use ifz_saver::Saver;
    use std::cell::RefCell;
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };

    const TIMER: usize = 1;

    enum Mode {
        Screen,
        Preview(HWND),
        Settings,
    }

    struct App {
        saver: Saver,
        /// the picture as GDI wants it: BGRx rows top to bottom, `w`×`h` game pixels, each `scale` device pixels
        bgr: Vec<u8>,
        w: usize,
        h: usize,
        scale: f64,
        power: String,
        power_y: f64,
        power_px: f64,
        muted: u32,
        ink: u32,
        full: bool,
        /// where the pointer first was (full screen: a move away from it ends the screensaver)
        at: Option<(i32, i32)>,
        font: HFONT,
        font_px: i32,
    }

    thread_local! {
        static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    // "/s", "-S", "/p 1234", "/p:1234", "/c:5678"; nothing is /c, as Windows does
    fn mode() -> Mode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let first = args.first().map(|a| a.to_ascii_lowercase()).unwrap_or_default();
        let (flag, rest) = first.split_at(first.len().min(2));
        let num = |s: &str| s.trim_start_matches(':').trim().parse::<isize>().ok();
        match flag.trim_start_matches(['/', '-']) {
            "s" => Mode::Screen,
            "p" => match num(rest).or_else(|| args.get(1).and_then(|a| num(a))) {
                Some(h) if h != 0 => Mode::Preview(h as HWND),
                _ => Mode::Settings,
            },
            _ => Mode::Settings,
        }
    }

    pub fn main() {
        let mode = mode();
        // SAFETY: plain Win32 calls on this thread's own window; every pointer passed lives across its call
        unsafe {
            if let Mode::Settings = mode {
                let text = wide("The Idle FighterZ screensaver shows your planet as the widget plays it, or a demo run of its own when the widget isn't open.\n\nIt has no settings.");
                MessageBoxW(std::ptr::null_mut(), text.as_ptr(), wide("Idle FighterZ").as_ptr(), MB_OK | MB_ICONINFORMATION);
                return;
            }
            let inst = GetModuleHandleW(std::ptr::null());
            let class = wide("IdleFighterZSaver");
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(proc_),
                hInstance: inst,
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc);
            let full = matches!(mode, Mode::Screen);
            APP.with(|a| {
                *a.borrow_mut() = Some(App {
                    saver: Saver::new(),
                    bgr: Vec::new(),
                    w: 0,
                    h: 0,
                    scale: 1.0,
                    power: String::new(),
                    power_y: 0.0,
                    power_px: 0.0,
                    muted: 0,
                    ink: 0,
                    full,
                    at: None,
                    font: std::ptr::null_mut(),
                    font_px: 0,
                });
            });
            let hwnd = match mode {
                Mode::Preview(parent) => {
                    let mut r: RECT = std::mem::zeroed();
                    GetClientRect(parent, &mut r);
                    CreateWindowExW(
                        0,
                        class.as_ptr(),
                        std::ptr::null(),
                        WS_CHILD | WS_VISIBLE,
                        0,
                        0,
                        r.right - r.left,
                        r.bottom - r.top,
                        parent,
                        std::ptr::null_mut(),
                        inst,
                        std::ptr::null(),
                    )
                }
                _ => {
                    let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
                    let (w, h) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
                    let hwnd = CreateWindowExW(
                        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                        class.as_ptr(),
                        wide("Idle FighterZ").as_ptr(),
                        WS_POPUP | WS_VISIBLE,
                        x,
                        y,
                        w,
                        h,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        inst,
                        std::ptr::null(),
                    );
                    ShowCursor(0);
                    hwnd
                }
            };
            if hwnd.is_null() {
                return;
            }
            SetTimer(hwnd, TIMER, 33, None);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            APP.with(|a| {
                if let Some(app) = a.borrow_mut().take() {
                    if !app.font.is_null() {
                        DeleteObject(app.font);
                    }
                }
            });
        }
    }

    unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let full = APP.with(|a| a.borrow().as_ref().is_some_and(|a| a.full));
        match msg {
            WM_TIMER => {
                // a preview whose dialog went without destroying it first (its window belongs to another process)
                let parent = GetParent(hwnd);
                if !full && (parent.is_null() || IsWindow(parent) == 0) {
                    DestroyWindow(hwnd);
                    return 0;
                }
                let fresh = APP.with(|a| a.borrow_mut().as_mut().is_some_and(|app| take(hwnd, app)));
                if fresh {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
                0
            }
            WM_ERASEBKGND => {
                // the sky, beyond the picture (displays other than the primary)
                let sky = APP.with(|a| a.borrow().as_ref().and_then(|a| a.bgr.get(..3).map(|p| [p[2], p[1], p[0]]))).unwrap_or([0xfc, 0xfc, 0xfb]);
                let mut r: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut r);
                let b = CreateSolidBrush(rgb(sky));
                FillRect(wp as HDC, &r, b);
                DeleteObject(b);
                1
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                APP.with(|a| {
                    if let Some(app) = a.borrow_mut().as_mut() {
                        paint(hwnd, dc, app);
                    }
                });
                EndPaint(hwnd, &ps);
                0
            }
            WM_SETCURSOR if full => {
                SetCursor(std::ptr::null_mut());
                1
            }
            WM_MOUSEMOVE if full => {
                let p = ((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32);
                let moved = APP.with(|a| {
                    let mut a = a.borrow_mut();
                    let Some(app) = a.as_mut() else { return false };
                    match app.at {
                        None => {
                            app.at = Some(p);
                            false
                        }
                        Some(o) => (o.0 - p.0).abs() + (o.1 - p.1).abs() > 8,
                    }
                });
                if moved {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
                0
            }
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN if full => {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
                0
            }
            WM_ACTIVATEAPP if full && wp == 0 => {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
                0
            }
            WM_DESTROY => {
                KillTimer(hwnd, TIMER);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    fn rgb(c: [u8; 3]) -> u32 {
        c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16
    }

    // 0xRRGGBB as a COLORREF
    fn hex(c: u32) -> u32 {
        rgb([(c >> 16) as u8, (c >> 8) as u8, c as u8])
    }

    // where the picture goes: full screen spans every display, so the primary one, which starts at the virtual
    // screen's (0, 0); the preview is the whole window
    unsafe fn area(hwnd: HWND, full: bool) -> (i32, i32, i32, i32) {
        if full {
            return (-GetSystemMetrics(SM_XVIRTUALSCREEN), -GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        }
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut r);
        (0, 0, r.right, r.bottom)
    }

    // polls the saver; on a new picture keeps it as BGRx
    fn take(hwnd: HWND, app: &mut App) -> bool {
        // SAFETY: GetSystemMetrics and GetClientRect on our own window
        let (_, _, cw, ch) = unsafe { area(hwnd, app.full) };
        let shot = app.saver.poll(cw.max(1) as f64, ch.max(1) as f64);
        if !shot.fresh || shot.w == 0 {
            return false;
        }
        app.bgr.resize(shot.w * shot.h * 4, 0);
        for (o, p) in app.bgr.as_chunks_mut::<4>().0.iter_mut().zip(shot.rgba.as_chunks::<4>().0) {
            *o = [p[2], p[1], p[0], 0];
        }
        (app.w, app.h, app.scale) = (shot.w, shot.h, shot.scale);
        (app.power_y, app.power_px, app.muted, app.ink) = (shot.power_y, shot.power_px, shot.muted, shot.ink);
        app.power.clone_from(&shot.power.to_string());
        true
    }

    // the picture from the area's top left, and the power level on it: "power" muted, the number in ink
    unsafe fn paint(hwnd: HWND, dc: HDC, app: &mut App) {
        if app.w == 0 {
            return;
        }
        let (ox, oy, cw, _) = area(hwnd, app.full);
        let (dw, dh) = ((app.w as f64 * app.scale).round() as i32, (app.h as f64 * app.scale).round() as i32);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: app.w as i32,
                biHeight: -(app.h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        SetStretchBltMode(dc, COLORONCOLOR);
        StretchDIBits(dc, ox, oy, dw, dh, 0, 0, app.w as i32, app.h as i32, app.bgr.as_ptr().cast(), &bmi, DIB_RGB_COLORS, SRCCOPY);
        if app.power.is_empty() || app.power_px == 0.0 {
            return;
        }
        let px = app.power_px.round() as i32;
        if px != app.font_px {
            if !app.font.is_null() {
                DeleteObject(app.font);
            }
            let face = wide("Segoe UI");
            app.font = CreateFontW(
                -px,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                face.as_ptr(),
            );
            app.font_px = px;
        }
        let old = SelectObject(dc, app.font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextAlign(dc, TA_LEFT | TA_TOP);
        let label: Vec<u16> = "power ".encode_utf16().collect();
        let value: Vec<u16> = app.power.encode_utf16().collect();
        let mut sl: SIZE = std::mem::zeroed();
        let mut sv: SIZE = std::mem::zeroed();
        GetTextExtentPoint32W(dc, label.as_ptr(), label.len() as i32, &mut sl);
        GetTextExtentPoint32W(dc, value.as_ptr(), value.len() as i32, &mut sv);
        let x = ox + (cw - sl.cx - sv.cx) / 2;
        let y = oy + (app.power_y - px as f64 * 0.6).round() as i32;
        SetTextColor(dc, hex(app.muted));
        TextOutW(dc, x, y, label.as_ptr(), label.len() as i32);
        SetTextColor(dc, hex(app.ink));
        TextOutW(dc, x + sl.cx, y, value.as_ptr(), value.len() as i32);
        SelectObject(dc, old);
    }
}
