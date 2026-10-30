// Photo mode (feature rust-core): from the tray or the P key, the game pauses, the HUD hides and the frame on screen is
// saved as a PNG, twice its size in whole pixels, to the pictures folder and the clipboard. Any key or click carries
// on; a toast then says where it went. The paused time is played on resume, so a photo costs the run nothing.
use crate::{planet::Core, Shown};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct Photo {
    pub on: AtomicBool,
    // what the toast says once photo mode ends, if the photo was saved by then
    told: Mutex<Option<String>>,
}

/// `rgba` (`w`×`h`) with each pixel a `k`×`k` block: the pixel art stays crisp
pub fn upscale(rgba: &[u8], w: usize, h: usize, k: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() * k * k);
    let mut wide = Vec::with_capacity(w * 4 * k);
    for row in rgba.chunks_exact(w * 4).take(h) {
        wide.clear();
        for x in 0..w {
            for _ in 0..k {
                wide.extend_from_slice(&row[x * 4..x * 4 + 4]);
            }
        }
        for _ in 0..k {
            out.extend_from_slice(&wide);
        }
    }
    out
}

pub fn png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())?;
    wr.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// "Idle FighterZ 2026-10-30 at 14.03.07.png", or with " 2", " 3"… if that's taken
pub fn file_in(dir: &Path, name: &str, stamp: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    (1..)
        .map(|n| dir.join(if n == 1 { format!("{name} {stamp}.png") } else { format!("{name} {stamp} {n}.png") }))
        .find(|p| !taken(p))
        .unwrap()
}

pub fn enter(app: &AppHandle) {
    let (Some(core), photo) = (app.try_state::<Core>(), app.state::<Photo>()) else { return };
    if photo.on.swap(true, Relaxed) {
        return;
    }
    core.pause(true);
    *photo.told.lock().unwrap() = None;
    let _ = app.emit_to("main", "photo", true);
    app.state::<Shown>().poke(); // ghost mode lets go of the clicks, so a click can end it
    if let Some(w) = app.get_webview_window("main") {
        if !app.state::<Shown>().get() {
            crate::show_widget(app, true);
        }
        let _ = w.set_focus();
    }
    crate::smoke("photo: on");
    let shot = core.frame();
    let app = app.clone();
    std::thread::spawn(move || {
        let text = match shot.ok_or_else(|| "nothing drawn yet".to_string()).and_then(|(rgba, w, h, scale)| save(&app, &rgba, w, h, scale)) {
            Ok(text) => text,
            Err(e) => format!("couldn't save the photo: {e}"),
        };
        crate::smoke(&format!("photo: {text}"));
        let photo = app.state::<Photo>();
        let mut told = photo.told.lock().unwrap();
        if photo.on.load(Relaxed) {
            *told = Some(text);
        } else {
            let _ = app.emit_to("main", "notice", Value::String(text));
        }
    });
}

pub fn exit(app: &AppHandle) {
    let (Some(core), photo) = (app.try_state::<Core>(), app.state::<Photo>()) else { return };
    let mut told = photo.told.lock().unwrap();
    if !photo.on.swap(false, Relaxed) {
        return;
    }
    core.pause(false);
    let _ = app.emit_to("main", "photo", false);
    app.state::<Shown>().poke();
    crate::smoke("photo: off");
    if let Some(text) = told.take() {
        let _ = app.emit_to("main", "notice", Value::String(text));
    }
}

// the frame at twice its size on screen, to the pictures folder and the clipboard; the toast's text
fn save(app: &AppHandle, rgba: &[u8], w: u32, h: u32, scale: f64) -> Result<String, String> {
    let k = (2.0 * scale).round().max(1.0) as usize;
    let big = upscale(rgba, w as usize, h as usize, k);
    let (bw, bh) = (w * k as u32, h * k as u32);
    let bytes = png(&big, bw, bh)?;
    let dir = app.path().picture_dir().or_else(|_| app.path().home_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y-%m-%d at %H.%M.%S").to_string();
    let file = file_in(&dir, &app.package_info().name, &stamp, |p| p.exists());
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    let (tx, rx) = std::sync::mpsc::channel();
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = tx.send(clip::copy(&a, &bytes, &big, bw, bh));
    });
    let copied = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or(false);
    let folder = dir.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    Ok(if copied { format!("photo saved to {folder} and copied") } else { format!("photo saved to {folder}") })
}

// the PNG on the clipboard (on the main thread)
#[cfg(target_os = "macos")]
mod clip {
    use objc2::{class, msg_send, runtime::AnyObject};
    use std::ffi::c_void;

    pub fn copy(_app: &tauri::AppHandle, png: &[u8], _rgba: &[u8], _w: u32, _h: u32) -> bool {
        unsafe {
            let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            if pb.is_null() {
                return false;
            }
            let _: isize = msg_send![pb, clearContents];
            let data: *mut AnyObject = msg_send![class!(NSData), dataWithBytes: png.as_ptr() as *const c_void, length: png.len()];
            let kind: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"public.png".as_ptr()];
            msg_send![pb, setData: data, forType: kind]
        }
    }
}

// as "PNG" (browsers, Office, chat apps) and as a 32-bit DIB with alpha (Paint and older apps)
#[cfg(windows)]
mod clip {
    use std::ffi::c_void;
    use tauri::Manager;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
        fn CloseClipboard() -> i32;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(mem: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(mem: *mut c_void) -> i32;
        fn GlobalFree(mem: *mut c_void) -> *mut c_void;
    }
    const CF_DIBV5: u32 = 17;

    #[repr(C)]
    struct BitmapV5Header {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_ppm: i32,
        y_ppm: i32,
        clr_used: u32,
        clr_important: u32,
        masks: [u32; 4],
        cs_type: u32,
        endpoints: [i32; 9],
        gamma: [u32; 3],
        intent: u32,
        profile_data: u32,
        profile_size: u32,
        reserved: u32,
    }

    // a movable global block holding `parts` one after another, handed to the clipboard (which then owns it)
    unsafe fn put(format: u32, parts: &[&[u8]]) -> bool {
        let len = parts.iter().map(|p| p.len()).sum();
        let mem = GlobalAlloc(0x0002, len); // GMEM_MOVEABLE
        if mem.is_null() {
            return false;
        }
        let mut at = GlobalLock(mem) as *mut u8;
        if at.is_null() {
            GlobalFree(mem);
            return false;
        }
        for p in parts {
            std::ptr::copy_nonoverlapping(p.as_ptr(), at, p.len());
            at = at.add(p.len());
        }
        GlobalUnlock(mem);
        if SetClipboardData(format, mem).is_null() {
            GlobalFree(mem);
            return false;
        }
        true
    }

    pub fn copy(app: &tauri::AppHandle, png: &[u8], rgba: &[u8], w: u32, h: u32) -> bool {
        let Some(hwnd) = app.get_webview_window("main").and_then(|w| w.hwnd().ok()) else { return false };
        // BGRA rows, bottom up
        let mut dib = Vec::with_capacity(rgba.len());
        for row in rgba.chunks_exact(w as usize * 4).rev() {
            for x in 0..w as usize {
                let p = &row[x * 4..x * 4 + 4];
                dib.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        let head = BitmapV5Header {
            size: std::mem::size_of::<BitmapV5Header>() as u32,
            width: w as i32,
            height: h as i32,
            planes: 1,
            bit_count: 32,
            compression: 3, // BI_BITFIELDS
            size_image: dib.len() as u32,
            x_ppm: 0,
            y_ppm: 0,
            clr_used: 0,
            clr_important: 0,
            masks: [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000],
            cs_type: 0x7352_4742, // LCS_sRGB
            endpoints: [0; 9],
            gamma: [0; 3],
            intent: 4, // LCS_GM_IMAGES
            profile_data: 0,
            profile_size: 0,
            reserved: 0,
        };
        let head = unsafe { std::slice::from_raw_parts(&head as *const BitmapV5Header as *const u8, std::mem::size_of::<BitmapV5Header>()) };
        let name: Vec<u16> = "PNG".encode_utf16().chain([0]).collect();
        unsafe {
            if OpenClipboard(hwnd.0 as *mut c_void) == 0 {
                return false;
            }
            EmptyClipboard();
            let fmt = RegisterClipboardFormatW(name.as_ptr());
            let ok = (fmt != 0 && put(fmt, &[png])) | put(CF_DIBV5, &[head, &dib]);
            CloseClipboard();
            ok
        }
    }
}

// GTK's clipboard, which serves the image for as long as the app runs
#[cfg(target_os = "linux")]
mod clip {
    use gtk::{gdk, gdk_pixbuf::{Colorspace, Pixbuf}, glib::Bytes};

    pub fn copy(_app: &tauri::AppHandle, _png: &[u8], rgba: &[u8], w: u32, h: u32) -> bool {
        let pixbuf = Pixbuf::from_bytes(&Bytes::from_owned(rgba.to_vec()), Colorspace::Rgb, true, 8, w as i32, h as i32, w as i32 * 4);
        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        clipboard.set_image(&pixbuf);
        clipboard.store();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pixel_becomes_a_block() {
        // 2×1: red, half-clear blue
        let px = [255, 0, 0, 255, 0, 0, 255, 128];
        let big = upscale(&px, 2, 1, 2);
        assert_eq!(big.len(), 4 * 2 * 4);
        let row = [255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 128, 0, 0, 255, 128];
        assert_eq!(big[..16], row);
        assert_eq!(big[16..], row);
        assert_eq!(upscale(&px, 2, 1, 1), px);
    }

    #[test]
    fn the_png_reads_back() {
        let px = upscale(&[1, 2, 3, 4, 5, 6, 7, 8], 2, 1, 4);
        let bytes = png(&px, 8, 4).unwrap();
        let mut r = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (8, 4));
        assert_eq!(buf, px);
    }

    #[test]
    fn photos_never_overwrite() {
        let dir = Path::new("/p");
        let taken = [PathBuf::from("/p/Idle FighterZ 2026-10-30 at 14.03.07.png"), PathBuf::from("/p/Idle FighterZ 2026-10-30 at 14.03.07 2.png")];
        assert_eq!(file_in(dir, "Idle FighterZ", "2026-10-30 at 14.03.08", |p| taken.iter().any(|t| t == p)), PathBuf::from("/p/Idle FighterZ 2026-10-30 at 14.03.08.png"));
        assert_eq!(file_in(dir, "Idle FighterZ", "2026-10-30 at 14.03.07", |p| taken.iter().any(|t| t == p)), PathBuf::from("/p/Idle FighterZ 2026-10-30 at 14.03.07 3.png"));
    }
}
