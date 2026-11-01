// The native renderer (cargo features `native-renderer` and `rust-core`, macOS): the game planet-render draws goes in a
// CALayer under the HUD page (`Under`); the spectacles' layers take their images from `image` too. See NATIVE.md.
mod under;

pub use under::Under;

use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

// a premultiplied RGBA copy of the frame, owned by the image
pub(crate) fn image(space: &CGColorSpace, w: u32, h: u32, rgba: &[u8]) -> CFRetained<CGImage> {
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
