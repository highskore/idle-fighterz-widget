// The Windows screensaver (IdleFighterZ.scr is this exe renamed). Windows runs it with /s (full screen), /p <hwnd> (the
// little preview in the settings dialog) or /c (settings: there are none). GDI draws the frame scaled up
// nearest-neighbour; a 33 ms timer polls `Saver`, so it never draws more than 30 frames a second.
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
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };

    // the screensaver's ground: the widget's planet floats on near-black
    const BG: [u8; 3] = [0x07, 0x07, 0x0c];
    const TIMER: usize = 1;

    enum Mode {
        Screen,
        Preview(HWND),
        Settings,
    }

    struct App {
        saver: Saver,
        /// the frame composited over BG, as GDI wants it: BGRx rows top to bottom
        bgr: Vec<u8>,
        w: usize,
        h: usize,
        power: String,
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
                    power: String::new(),
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
                let fresh = APP.with(|a| a.borrow_mut().as_mut().is_some_and(take));
                if fresh {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
                0
            }
            WM_ERASEBKGND => {
                let mut r: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut r);
                let b = CreateSolidBrush(rgb(BG));
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

    // polls the saver; on a new frame composites it over BG into `bgr`
    fn take(app: &mut App) -> bool {
        let shot = app.saver.poll();
        if !shot.fresh || shot.w == 0 {
            return false;
        }
        app.bgr.resize(shot.w * shot.h * 4, 0);
        for (o, p) in app.bgr.as_chunks_mut::<4>().0.iter_mut().zip(shot.rgba.as_chunks::<4>().0) {
            let a = p[3] as u32;
            for (c, (&src, &bg)) in [p[2], p[1], p[0]].iter().zip([BG[2], BG[1], BG[0]].iter()).enumerate() {
                o[c] = ((src as u32 * a + bg as u32 * (255 - a) + 127) / 255) as u8;
            }
            o[3] = 0;
        }
        (app.w, app.h) = (shot.w, shot.h);
        app.power.clone_from(&shot.power.to_string());
        true
    }

    // the frame centred a little above the middle, at the biggest whole scale that leaves a margin (or shrunk to fit
    // the preview), and the power level under it
    unsafe fn paint(hwnd: HWND, dc: HDC, app: &mut App) {
        if app.w == 0 {
            return;
        }
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut r);
        // full screen spans every display: centre on the primary one, which starts at the virtual screen's (0, 0)
        let (ox, oy, cw, ch) = if app.full {
            (-GetSystemMetrics(SM_XVIRTUALSCREEN), -GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
        } else {
            (0, 0, r.right, r.bottom)
        };
        let fit = (cw as f64 * 0.8 / app.w as f64).min(ch as f64 * 0.72 / app.h as f64);
        let k = if fit >= 1.0 { fit.floor() } else { fit };
        let (dw, dh) = ((app.w as f64 * k) as i32, (app.h as f64 * k) as i32);
        let (dx, dy) = (ox + (cw - dw) / 2, oy + (ch - dh) / 2 - ch / 20);
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
        StretchDIBits(dc, dx, dy, dw, dh, 0, 0, app.w as i32, app.h as i32, app.bgr.as_ptr().cast(), &bmi, DIB_RGB_COLORS, SRCCOPY);
        if app.power.is_empty() || !app.full && ch < 200 {
            return;
        }
        let px = (k as i32 * 9).clamp(14, 48);
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
                FW_SEMIBOLD as i32,
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
        SetTextColor(dc, rgb([0xd8, 0xd8, 0xe0]));
        SetBkColor(dc, rgb(BG));
        SetTextAlign(dc, TA_CENTER | TA_TOP);
        let text: Vec<u16> = format!("power {}", app.power).encode_utf16().collect();
        let band = RECT { left: ox, top: dy + dh + px / 2, right: ox + cw, bottom: dy + dh + px * 2 };
        ExtTextOutW(dc, ox + cw / 2, band.top, ETO_OPAQUE, &band, text.as_ptr(), text.len() as u32, std::ptr::null());
        SelectObject(dc, old);
    }
}
