// The game on the Rust core, drawn by planet-render, shown in a CALayer under the HUD page (features rust-core and
// native-renderer). The page stays on top, transparent, and takes every press; the layer sits exactly where the page's
// canvas is, stretched to it with nearest-neighbour as `image-rendering: pixelated` stretches the canvas. Window mode
// keeps the page's canvas: its skies are CSS backgrounds under the canvas, which a layer under the page would cover.
use super::{image, MainOnly, Stats};
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGImage};
use objc2_quartz_core::{kCAFilterNearest, kCAGravityResize, CALayer, CATransaction};
use std::{
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::AppHandle;

type Pending = Arc<Mutex<Option<MainOnly<CFRetained<CGImage>>>>>;

pub struct Under {
    app: AppHandle,
    // made on the main thread once the web view is there
    layer: Arc<Mutex<Option<MainOnly<Retained<CALayer>>>>>,
    space: MainOnly<CFRetained<CGColorSpace>>,
    state: Mutex<Shown>,
    pending: Pending,
    stats: Option<Arc<Stats>>,
}

#[derive(Default)]
struct Shown {
    on: bool,
    last: Vec<u8>,
    reported: Option<Instant>,
}

impl Under {
    /// Puts an empty, hidden layer under the main window's web view; call on the main thread.
    pub fn new(app: &AppHandle) -> tauri::Result<Self> {
        use tauri::Manager;
        let win = app.get_webview_window("main").expect("main window");
        let slot: Arc<Mutex<Option<MainOnly<Retained<CALayer>>>>> = Arc::default();
        let made = slot.clone();
        win.with_webview(move |w| unsafe {
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
            *made.lock().unwrap() = Some(MainOnly(layer));
        })?;
        Ok(Under {
            app: app.clone(),
            layer: slot,
            space: MainOnly(CGColorSpace::new_device_rgb().expect("device RGB")),
            state: Mutex::default(),
            pending: Arc::default(),
            stats: std::env::var_os("IDLE_FIGHTERZ_NATIVE_STATS").map(|_| Arc::new(Stats::default())),
        })
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

impl crate::planet::Surface for Under {
    fn place(&self, [x, y, w, h]: [f64; 4], opacity: f64, bare: bool) -> bool {
        let mut s = self.state.lock().unwrap();
        s.on = bare && w > 0.0 && h > 0.0;
        s.last.clear(); // the next frame goes up, whatever it is
        let on = s.on;
        self.on_main(move |l| {
            quietly(|| {
                if on {
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

    fn show(&self, rgba: &[u8], w: u32, h: u32) {
        let t0 = Instant::now();
        let mut s = self.state.lock().unwrap();
        if !s.on || rgba == s.last.as_slice() {
            return;
        }
        s.last.clear();
        s.last.extend_from_slice(rgba);
        let img = MainOnly(image(&self.space.0, w, h, rgba));
        // one present in flight at most: a later frame replaces the waiting one
        if self.pending.lock().unwrap().replace(img).is_none() {
            let (pending, stats) = (self.pending.clone(), self.stats.clone());
            self.on_main(move |l| {
                let t = Instant::now();
                if let Some(img) = pending.lock().unwrap().take() {
                    quietly(|| unsafe { l.setContents(Some(img.0.as_ref())) });
                }
                if let Some(s) = stats {
                    s.add(&s.present, t.elapsed());
                }
            });
        }
        if let Some(st) = &self.stats {
            st.add(&st.tick, t0.elapsed());
            st.changed.fetch_add(1, Ordering::Relaxed);
            let at = *s.reported.get_or_insert(t0);
            if at.elapsed() >= Duration::from_secs(5) {
                s.reported = Some(Instant::now());
                st.report();
            }
        }
    }
}
