// The game on the Rust core, drawn by planet-render, shown in a CALayer under the HUD page (features rust-core and
// native-renderer). The page stays on top, transparent, and takes every press; the layer sits exactly where the page's
// canvas is, stretched to it with nearest-neighbour as `image-rendering: pixelated` stretches the canvas. Window mode
// keeps the page's canvas: its skies are CSS backgrounds under the canvas, which a layer under the page would cover.
use super::{image, Stats};
use block2::RcBlock;
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreate, CGBlendMode, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo};
use objc2_foundation::NSString;
use objc2_io_surface::{kIOSurfaceBytesPerElement, kIOSurfaceColorSpace, kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth, IOSurfaceLockOptions, IOSurfaceRef};
use objc2_quartz_core::{kCAFilterNearest, kCAGravityResize, CALayer, CATransaction};
use std::{
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::AppHandle;

// Core Animation takes layer changes from any thread inside an explicit transaction; IOSurfaces are shared memory, and
// colour spaces immutable
struct AnyThread<T>(T);
unsafe impl<T> Send for AnyThread<T> {}
unsafe impl<T> Sync for AnyThread<T> {}

type Slot<T> = Arc<Mutex<Option<AnyThread<T>>>>;

// a few surfaces the frames take turns in, so the one on screen is never written to
const POOL: usize = 3;

pub struct Under {
    app: AppHandle,
    // made on the main thread once the web view is there
    layer: Slot<Retained<CALayer>>,
    // the window's colour space, kept up to date on the main thread as the window changes screens
    space: Slot<CFRetained<CGColorSpace>>,
    state: Mutex<Shown>,
    stats: Option<Arc<Stats>>,
    // the tick thread's timer
    kq: i32,
    // a frame couldn't go up: the page draws them from then on
    failed: AtomicBool,
    // IDLE_FIGHTERZ_NATIVE_FAIL=show, to try the fallback
    fail_show: bool,
}

#[derive(Default)]
struct Shown {
    on: bool,
    // the layer shows pool[front]
    up: bool,
    reported: Option<Instant>,
    pool: Vec<Buffer>,
    front: usize,
    colours: Option<AnyThread<Colours>>,
    // where the frames went, last logged
    said: &'static str,
}

struct Buffer {
    surface: AnyThread<CFRetained<IOSurfaceRef>>,
    // the frame it holds, as the tick drew it
    held: Vec<u8>,
}

// the colour space the window's backing is in
unsafe fn window_space(window: &AnyObject) -> Option<CFRetained<CGColorSpace>> {
    let ns: Option<Retained<AnyObject>> = msg_send![window, colorSpace];
    let cg: *mut CGColorSpace = msg_send![&*ns?, CGColorSpace];
    Some(unsafe { CFRetained::retain(NonNull::new(cg)?) })
}

impl Under {
    /// Puts an empty, hidden layer under the main window's web view; call on the main thread. The layer is made on the
    /// main thread later: until it is, and if it never is, `place` leaves the frames to the page.
    pub fn new(app: &AppHandle) -> tauri::Result<Self> {
        use tauri::Manager;
        // setup, layer or show: fail there, to try the fallback
        let fail = std::env::var("IDLE_FIGHTERZ_NATIVE_FAIL").unwrap_or_default();
        if fail == "setup" {
            return Err(std::io::Error::other("IDLE_FIGHTERZ_NATIVE_FAIL=setup").into());
        }
        let win = app.get_webview_window("main").expect("main window");
        let (slot, space): (Slot<Retained<CALayer>>, Slot<CFRetained<CGColorSpace>>) = Default::default();
        let (made, spaced, no_layer) = (slot.clone(), space.clone(), fail == "layer");
        win.with_webview(move |w| unsafe {
            if no_layer {
                return;
            }
            let web = w.inner() as *mut AnyObject;
            let parent: Option<Retained<AnyObject>> = msg_send![web, superview];
            let Some(parent) = parent else { return };
            let frame: CGRect = msg_send![web, frame];
            // a layer-hosting view (its layer set before wantsLayer), so AppKit leaves the layer tree to us
            let host: objc2::rc::Allocated<AnyObject> = msg_send![class!(NSView), alloc];
            let host: Retained<AnyObject> = msg_send![host, initWithFrame: frame];
            let root = CALayer::new();
            root.setGeometryFlipped(true); // top-left origin, as the page's coordinates
            let _: () = msg_send![&*host, setLayer: &*root];
            let _: () = msg_send![&*host, setWantsLayer: true];
            let _: () = msg_send![&*host, setAutoresizingMask: 2usize | 16]; // width and height follow the window
            let below: isize = -1; // NSWindowBelow
            let _: () = msg_send![&*parent, addSubview: &*host, positioned: below, relativeTo: web];
            let layer = CALayer::new();
            layer.setContentsGravity(kCAGravityResize);
            layer.setMagnificationFilter(kCAFilterNearest);
            layer.setMinificationFilter(kCAFilterNearest);
            layer.setOpaque(false);
            layer.setHidden(true);
            root.addSublayer(&layer);
            *made.lock().unwrap() = Some(AnyThread(layer));

            let window: Option<Retained<AnyObject>> = msg_send![web, window];
            let Some(window) = window else { return };
            *spaced.lock().unwrap() = window_space(&window).map(AnyThread);
            let center: Retained<AnyObject> = msg_send![class!(NSNotificationCenter), defaultCenter];
            for name in ["NSWindowDidChangeScreenNotification", "NSWindowDidChangeScreenProfileNotification"] {
                let (spaced, seen) = (spaced.clone(), AnyThread(window.clone()));
                let block = RcBlock::new(move |_: NonNull<AnyObject>| *spaced.lock().unwrap() = window_space(&seen.0).map(AnyThread));
                let nil: *const AnyObject = std::ptr::null();
                // the centre keeps the observer, which lives as long as the app
                let _: Retained<AnyObject> =
                    msg_send![&*center, addObserverForName: &*NSString::from_str(name), object: &*window, queue: nil, usingBlock: &*block];
            }
        })?;
        Ok(Under {
            app: app.clone(),
            layer: slot,
            space,
            state: Mutex::default(),
            stats: std::env::var_os("IDLE_FIGHTERZ_NATIVE_STATS").map(|_| Arc::new(Stats::default())),
            kq: unsafe { libc::kqueue() },
            failed: AtomicBool::new(false),
            fail_show: fail == "show",
        })
    }

    // the page draws from now on; the layer is emptied so nothing stale shows under its canvas
    fn give_up(&self, why: &str) -> bool {
        if !self.failed.swap(true, Ordering::Relaxed) {
            eprintln!("frames: the layer failed ({why}), on the page's canvas from now on");
            self.on_main(|l| {
                quietly(|| {
                    unsafe { l.setContents(None) };
                    l.setHidden(true);
                })
            });
        }
        false
    }

    fn on_main(&self, f: impl FnOnce(&CALayer) + Send + 'static) {
        let layer = self.layer.clone();
        let _ = self.app.run_on_main_thread(move || {
            if let Some(l) = layer.lock().unwrap().as_ref() {
                f(&l.0)
            }
        });
    }
}

// without this, Core Animation animates every change for a quarter second
fn quietly(f: impl FnOnce()) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    f();
    CATransaction::commit();
}

// BGRA, premultiplied, in `space`: Core Animation shows it as it is
fn surface(w: u32, h: u32, space: &CGColorSpace) -> Option<CFRetained<IOSurfaceRef>> {
    let num = |n: i32| CFNumber::new_i32(n);
    let (w, h, bpe, fmt) = (num(w as i32), num(h as i32), num(4), num(i32::from_be_bytes(*b"BGRA")));
    let keys: [&CFString; 4] = unsafe { [kIOSurfaceWidth, kIOSurfaceHeight, kIOSurfaceBytesPerElement, kIOSurfacePixelFormat] };
    let values: [&CFType; 4] = [&w, &h, &bpe, &fmt];
    let props = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
    let s = unsafe { IOSurfaceRef::new(props.as_opaque())? };
    unsafe { s.set_value(kIOSurfaceColorSpace, &*space.property_list()?) };
    // clear, as the buffer's `held` starts
    unsafe {
        s.lock(IOSurfaceLockOptions::empty(), std::ptr::null_mut());
        std::ptr::write_bytes(s.base_address().as_ptr().cast::<u8>(), 0, s.alloc_size());
        s.unlock(IOSurfaceLockOptions::empty(), std::ptr::null_mut());
    }
    Some(s)
}

// Core Animation colour-matches a CGImage into the window's space on the CPU, but an IOSurface in any other space on
// the GPU, which rounds a third of the colours a step apart. So the frame goes in already matched, by CG as the CGImage
// was: each colour once, then from a cache (a frame has a hundred or so)
struct Colours {
    space: CFRetained<CGColorSpace>,
    device: CFRetained<CGColorSpace>,
    ctx: CFRetained<CGContext>,
    one: Box<[u8; 4]>, // the context's pixel; dropped after it
    keys: Vec<u32>,
    vals: Vec<u32>,
}

const SLOTS: usize = 1 << 14;

impl Colours {
    fn new(space: CFRetained<CGColorSpace>) -> Option<Self> {
        let mut one = Box::new([0u8; 4]);
        let info = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0; // BGRA in memory
        let ctx = unsafe { CGBitmapContextCreate(one.as_mut_ptr().cast(), 1, 1, 8, 4, Some(&space), info)? };
        CGContext::set_blend_mode(Some(&ctx), CGBlendMode::Copy);
        Some(Colours { space, device: CGColorSpace::new_device_rgb()?, ctx, one, keys: vec![0; SLOTS], vals: vec![0; SLOTS] })
    }

    // a straight RGBA pixel to premultiplied BGRA in the window's space; 0 (clear) is 0 in any space
    fn get(&mut self, p: u32) -> u32 {
        let k = (p.wrapping_mul(0x9E37_79B1) >> (32 - SLOTS.trailing_zeros())) as usize;
        if self.keys[k] != p {
            // as the CGImage was: premultiplied, device RGB
            let img = image(&self.device, 1, 1, &p.to_le_bytes());
            CGContext::draw_image(Some(&self.ctx), CGRect::new(CGPoint::ZERO, CGSize::new(1.0, 1.0)), Some(&img));
            self.keys[k] = p;
            self.vals[k] = u32::from_le_bytes(*self.one);
        }
        self.vals[k]
    }
}

// Writes only what changed since the frame the buffer holds (at medium, about 500 of the 142,400 pixels change a
// frame), 32 bytes at a time
fn fill(b: &mut Buffer, rgba: &[u8], w: u32, h: u32, c: &mut Colours) {
    let s = &b.surface.0;
    unsafe { s.lock(IOSurfaceLockOptions::empty(), std::ptr::null_mut()) };
    let (base, stride, row) = (s.base_address().as_ptr().cast::<u8>(), s.bytes_per_row(), w as usize * 4);
    for (y, (src, held)) in rgba.chunks_exact(row).zip(b.held.chunks_exact_mut(row)).take(h as usize).enumerate() {
        if src == held {
            continue;
        }
        let dst = unsafe { std::slice::from_raw_parts_mut(base.add(y * stride), row) };
        for ((d, p), o) in dst.chunks_mut(32).zip(src.chunks(32)).zip(held.chunks_mut(32)) {
            if p == o {
                continue;
            }
            for ((d, p), o) in d.as_chunks_mut::<4>().0.iter_mut().zip(p.as_chunks::<4>().0).zip(o.as_chunks::<4>().0) {
                if p != o {
                    let v = u32::from_le_bytes(*p);
                    *d = if v == 0 { [0; 4] } else { c.get(v).to_le_bytes() };
                }
            }
            o.copy_from_slice(p);
        }
    }
    unsafe { s.unlock(IOSurfaceLockOptions::empty(), std::ptr::null_mut()) };
}

impl crate::planet::Surface for Under {
    fn place(&self, [x, y, w, h]: [f64; 4], opacity: f64, bare: bool) -> bool {
        let mut s = self.state.lock().unwrap();
        let ready = !self.failed.load(Ordering::Relaxed) && self.layer.lock().unwrap().is_some();
        s.on = ready && bare && w > 0.0 && h > 0.0;
        s.up = false; // the next frame goes up, whatever it is
        let on = s.on;
        let path = match (on, ready) {
            (true, _) => "on the layer under the page",
            (false, true) => "on the page's canvas (window mode)",
            (false, false) => "on the page's canvas (no layer)",
        };
        if s.said != path {
            eprintln!("frames: {path}");
            s.said = path;
        }
        self.on_main(move |l| {
            quietly(|| {
                if on {
                    // the page's y runs down from the top. AppKit gives a layer-hosting view's root layer the view's own
                    // geometry (not flipped) whatever it was set to, so measured from the bottom it's the page's y
                    // flipped: otherwise the frames sit lower than the canvas by the band the canvas leaves under it
                    let flip = l.superlayer().filter(|p| !p.isGeometryFlipped()).map(|p| p.bounds().size.height);
                    #[cfg(debug_assertions)]
                    eprintln!("layer: page rect {x},{y} {w}x{h}, root flipped: {}", flip.is_none());
                    let y = flip.map_or(y, |hh| hh - y - h);
                    l.setFrame(CGRect::new(CGPoint::new(x, y), CGSize::new(w, h)));
                    l.setOpacity(opacity.clamp(0.0, 1.0) as f32);
                } else {
                    unsafe { l.setContents(None) };
                }
                l.setHidden(!on);
            })
        });
        on
    }

    // on the tick thread, with no hop to the main thread: the frame goes into a surface the window server isn't
    // showing, and the layer takes that surface as its contents
    fn show(&self, rgba: &[u8], w: u32, h: u32) -> bool {
        let t0 = Instant::now();
        let mut s = self.state.lock().unwrap();
        if !s.on || (s.up && s.pool.get(s.front).is_some_and(|b| b.held == rgba)) {
            return true;
        }
        if self.fail_show {
            return self.give_up("IDLE_FIGHTERZ_NATIVE_FAIL=show");
        }
        let layer = self.layer.lock().unwrap();
        let Some(layer) = layer.as_ref() else { return self.give_up("no layer") };
        let Some(space) = self.space.lock().unwrap().as_ref().map(|c| c.0.clone()) else { return self.give_up("no colour space") };
        // a new size or a new colour space: new surfaces, and the colours matched again
        if s.colours.as_ref().is_none_or(|c| c.0.space != space) || s.pool.first().is_some_and(|b| b.held.len() != rgba.len() || b.surface.0.width() != w as usize) {
            s.pool.clear();
            s.colours = Colours::new(space).map(AnyThread);
        }
        let front = s.front;
        let free = (0..s.pool.len()).find(|&k| k != front && !s.pool[k].surface.0.is_in_use());
        let k = match free {
            Some(k) => k,
            None if s.pool.len() < POOL => {
                let Some(c) = s.colours.as_ref() else { return self.give_up("no colour matching") };
                let Some(n) = surface(w, h, &c.0.space) else { return self.give_up("no IOSurface") };
                s.pool.push(Buffer { surface: AnyThread(n), held: vec![0; rgba.len()] });
                s.pool.len() - 1
            }
            None => (front + 1) % s.pool.len(), // the window server holds them all: the oldest
        };
        let Shown { pool, colours, .. } = &mut *s;
        fill(&mut pool[k], rgba, w, h, &mut colours.as_mut().expect("made with the pool").0);
        let t = Instant::now();
        let contents: &AnyObject = unsafe { &*(&*s.pool[k].surface.0 as *const IOSurfaceRef).cast::<AnyObject>() };
        quietly(|| unsafe { layer.0.setContents(Some(contents)) });
        (s.front, s.up) = (k, true);
        if let Some(st) = &self.stats {
            st.add(&st.present, t.elapsed());
            st.add(&st.tick, t0.elapsed());
            st.changed.fetch_add(1, Ordering::Relaxed);
            let at = *s.reported.get_or_insert(t0);
            if at.elapsed() >= Duration::from_secs(5) {
                s.reported = Some(Instant::now());
                st.report();
            }
        }
        true
    }

    // utility QoS puts the drawing on the efficiency cores, at about half the energy
    fn tick_thread(&self) {
        unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
    }

    // a utility thread's sleep is coalesced by up to 150 ms; a critical timer with no leeway isn't (p99 5 ms late)
    fn sleep(&self, d: Duration) {
        let ev = libc::kevent64_s {
            ident: 1,
            filter: libc::EVFILT_TIMER,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_NSECONDS | libc::NOTE_CRITICAL | libc::NOTE_LEEWAY,
            data: d.as_nanos() as i64,
            udata: 0,
            ext: [0, 0], // ext[1]: the leeway
        };
        let mut out = ev;
        if d.is_zero() || unsafe { libc::kevent64(self.kq, &ev, 1, &mut out, 1, 0, std::ptr::null()) } < 0 {
            std::thread::sleep(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_graphics::{kCGColorSpaceDisplayP3, kCGColorSpaceSRGB, CGBitmapContextGetData};

    // the frame as Core Animation would have shown the CGImage: the whole frame matched by CG at once
    fn matched(space: &CGColorSpace, rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
        let info = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;
        let ctx = unsafe { CGBitmapContextCreate(std::ptr::null_mut(), w as usize, h as usize, 8, w as usize * 4, Some(space), info) }.unwrap();
        CGContext::set_blend_mode(Some(&ctx), CGBlendMode::Copy);
        let img = image(&CGColorSpace::new_device_rgb().unwrap(), w, h, rgba);
        CGContext::draw_image(Some(&ctx), CGRect::new(CGPoint::ZERO, CGSize::new(w as f64, h as f64)), Some(&img));
        let data = CGBitmapContextGetData(Some(&ctx)).cast::<u8>();
        unsafe { std::slice::from_raw_parts(data, (w * h * 4) as usize) }.to_vec()
    }

    fn contents(s: &IOSurfaceRef, w: u32, h: u32) -> Vec<u8> {
        let (base, stride) = (s.base_address().as_ptr().cast::<u8>(), s.bytes_per_row());
        (0..h as usize).flat_map(|y| unsafe { std::slice::from_raw_parts(base.add(y * stride), w as usize * 4) }.to_vec()).collect()
    }

    // frames that change a little each time (opaque, translucent and clear pixels), taking turns in three buffers as the
    // pool does: every buffer ends up exactly what CG matches the frame to, in sRGB and in Display P3
    #[test]
    fn the_surfaces_hold_what_core_animation_showed() {
        let (w, h) = (37u32, 23u32);
        let mut rng = 0x2545_f491u32;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng
        };
        let px = |n: u32| match n % 4 {
            0 => 0,
            1 => n | 0xff00_0000,
            _ => n,
        };
        for name in [unsafe { kCGColorSpaceSRGB }, unsafe { kCGColorSpaceDisplayP3 }] {
            let space = CGColorSpace::with_name(Some(name)).unwrap();
            let mut c = Colours::new(space.clone()).unwrap();
            let mut pool: Vec<Buffer> = (0..3).map(|_| Buffer { surface: AnyThread(surface(w, h, &space).unwrap()), held: vec![0; (w * h * 4) as usize] }).collect();
            let mut frame: Vec<u32> = (0..w * h).map(|_| px(next())).collect();
            for f in 0..40 {
                for _ in 0..(1 + next() % 30) {
                    let k = (next() % (w * h)) as usize;
                    frame[k] = px(next());
                }
                let rgba: Vec<u8> = frame.iter().flat_map(|p| p.to_le_bytes()).collect();
                let b = &mut pool[f % 3];
                fill(b, &rgba, w, h, &mut c);
                assert!(contents(&b.surface.0, w, h) == matched(&space, &rgba, w, h), "frame {f}");
            }
        }
    }
}
