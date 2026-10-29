// The native renderer (cargo feature `native-renderer`, then `"native_renderer": "planet"` or `"pattern"` in
// settings.json): the main window's web view is emptied and hidden, and a CALayer in its place shows a FrameSource's
// framebuffer. Core Animation scales it on the GPU, nearest-neighbour by a whole factor, with its alpha over the
// wallpaper; a new image goes up only when the frame changed. See NATIVE.md.
mod source;

pub use source::{FrameSource, PngLoop, TestPattern};

use objc2::{msg_send, rc::Retained, runtime::AnyObject};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_quartz_core::{kCAFilterNearest, kCAGravityResize, CALayer, CATransaction};
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

const FPS: f64 = 60.0;

// Core Animation objects that only the main thread touches; the loop thread just carries them there
struct MainOnly<T>(T);
unsafe impl<T> Send for MainOnly<T> {}
unsafe impl<T> Sync for MainOnly<T> {}

/// the source named by the hidden setting, if any
pub fn wanted(saved: &serde_json::Value) -> Option<Box<dyn FrameSource>> {
    match saved.get("native_renderer")?.as_str()? {
        "planet" => Some(Box::new(PngLoop::planet())),
        "pattern" => Some(Box::new(TestPattern::new())),
        _ => None,
    }
}

/// takes over the main window; call on the main thread (tauri's setup)
pub fn start(app: &AppHandle, mut src: Box<dyn FrameSource>) -> tauri::Result<()> {
    let win = app.get_webview_window("main").expect("main window");
    let _ = win.navigate("about:blank".parse().unwrap()); // the game page stops: nothing runs behind the layer
    win.with_webview(|w| unsafe {
        let view = w.inner() as *mut AnyObject;
        let _: () = msg_send![view, setHidden: true];
    })?;
    let ns_window = win.ns_window()? as *mut AnyObject;
    let (layer, view) = unsafe {
        let view: Retained<AnyObject> = msg_send![ns_window, contentView];
        let _: () = msg_send![&*view, setWantsLayer: true];
        let root: Retained<CALayer> = msg_send![&*view, layer];
        let layer = CALayer::new();
        layer.setContentsGravity(kCAGravityResize);
        layer.setMagnificationFilter(kCAFilterNearest);
        layer.setMinificationFilter(kCAFilterNearest);
        layer.setOpaque(false);
        root.addSublayer(&layer);
        (layer, view)
    };

    let stats = std::env::var_os("IDLE_FIGHTERZ_NATIVE_STATS").map(|_| Arc::new(Stats::default()));
    let slot: Arc<Mutex<Option<MainOnly<CFRetained<CGImage>>>>> = Arc::default();
    let shared = MainOnly((layer, view, ns_window));
    let app = app.clone();
    std::thread::spawn(move || {
        let shared = Arc::new(shared);
        let space = CGColorSpace::new_device_rgb().expect("device RGB");
        let (w, h) = src.size();
        let mut last: Vec<u8> = Vec::new();
        let tick = Duration::from_secs_f64(1.0 / FPS);
        let mut next = Instant::now();
        let mut visible = true;
        for n in 0u64..=u64::MAX {
            next += tick;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now; // fell behind: don't try to catch up
            }
            // hidden, the window shows nothing: stand still, and ask again 4 times a second
            if n % 15 == 0 {
                visible = win.is_visible().unwrap_or(true);
            }
            if !visible {
                continue;
            }
            let t0 = Instant::now();
            src.step(1.0 / FPS as f32);
            let frame = src.frame();
            let changed = frame != last.as_slice();
            if changed {
                last.clear();
                last.extend_from_slice(frame);
                let img = MainOnly(image(&space, w, h, frame));
                // one present in flight at most: a later frame replaces the waiting one
                if slot.lock().unwrap().replace(img).is_none() {
                    let (slot, shared, stats) = (slot.clone(), shared.clone(), stats.clone());
                    let _ = app.run_on_main_thread(move || {
                        let t = Instant::now();
                        if let Some(img) = slot.lock().unwrap().take() {
                            let (layer, view, ns_window) = &shared.0;
                            present(layer, view, *ns_window, &img.0, w, h);
                        }
                        if let Some(s) = stats {
                            s.add(&s.present, t.elapsed());
                        }
                    });
                }
            }
            if let Some(s) = &stats {
                s.add(&s.tick, t0.elapsed());
                if changed {
                    s.changed.fetch_add(1, Ordering::Relaxed);
                }
                if n % (FPS as u64 * 5) == 0 && n > 0 {
                    s.report();
                }
            }
        }
    });
    Ok(())
}

// a premultiplied RGBA copy of the frame, owned by the image
fn image(space: &CGColorSpace, w: u32, h: u32, rgba: &[u8]) -> CFRetained<CGImage> {
    let mut px = rgba.to_vec().into_boxed_slice();
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u16;
        if a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u16 * a + 127) / 255) as u8;
            }
        }
    }
    unsafe extern "C-unwind" fn release(_: *mut c_void, data: NonNull<c_void>, len: usize) {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(data.as_ptr().cast::<u8>(), len)) });
    }
    let len = px.len();
    let data = Box::into_raw(px).cast::<c_void>();
    unsafe {
        let provider = CGDataProvider::with_data(std::ptr::null_mut(), data, len, Some(release)).expect("data provider");
        let info = CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::OrderDefault.0);
        CGImage::new(
            w as usize,
            h as usize,
            8,
            32,
            w as usize * 4,
            Some(space),
            info,
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
        .expect("image")
    }
}

// shows the image at the biggest whole-number scale that fits, centred, on the device-pixel grid
fn present(layer: &CALayer, view: &AnyObject, ns_window: *mut AnyObject, img: &CGImage, w: u32, h: u32) {
    let (bounds, scale): (CGRect, f64) = unsafe { (msg_send![view, bounds], msg_send![ns_window, backingScaleFactor]) };
    let k = ((bounds.size.width * scale / w as f64).min(bounds.size.height * scale / h as f64)).floor().max(1.0);
    let size = CGSize::new(w as f64 * k / scale, h as f64 * k / scale);
    let snap = |v: f64| (v * scale).round() / scale;
    let origin = CGPoint::new(snap((bounds.size.width - size.width) / 2.0), snap((bounds.size.height - size.height) / 2.0));
    // without this, Core Animation cross-fades every new image for a quarter second
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    layer.setContentsScale(scale);
    layer.setFrame(CGRect::new(origin, size));
    unsafe { layer.setContents(Some(img.as_ref())) };
    CATransaction::commit();
}

// frame-time counters, printed to stderr every 5 s with IDLE_FIGHTERZ_NATIVE_STATS set
#[derive(Default)]
struct Stats {
    tick: Timing,
    present: Timing,
    changed: AtomicU64,
}

#[derive(Default)]
struct Timing {
    n: AtomicU64,
    ns: AtomicU64,
    max: AtomicU64,
}

impl Stats {
    fn add(&self, t: &Timing, d: Duration) {
        let ns = d.as_nanos() as u64;
        t.n.fetch_add(1, Ordering::Relaxed);
        t.ns.fetch_add(ns, Ordering::Relaxed);
        t.max.fetch_max(ns, Ordering::Relaxed);
    }

    fn report(&self) {
        let take = |t: &Timing| {
            let (n, ns, max) = (t.n.swap(0, Ordering::Relaxed), t.ns.swap(0, Ordering::Relaxed), t.max.swap(0, Ordering::Relaxed));
            (n, if n > 0 { ns as f64 / n as f64 / 1e3 } else { 0.0 }, max as f64 / 1e3)
        };
        let (ticks, tick_avg, tick_max) = take(&self.tick);
        let (presents, pres_avg, pres_max) = take(&self.present);
        let changed = self.changed.swap(0, Ordering::Relaxed);
        eprintln!(
            "native: {:.1} steps/s, {:.1} changed/s, {:.1} presents/s | step+upload avg {tick_avg:.0}µs max {tick_max:.0}µs | present avg {pres_avg:.0}µs max {pres_max:.0}µs",
            ticks as f64 / 5.0,
            changed as f64 / 5.0,
            presents as f64 / 5.0
        );
    }
}
