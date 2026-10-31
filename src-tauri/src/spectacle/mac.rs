// The spectacles on macOS: a borderless, non-activating NSPanel over the widget's display, transparent, ignoring the
// mouse, on every Space and beside fullscreen apps, left out of screen sharing, just under the widget. It holds Core
// Animation layers of the pixel art, which this thread moves inside explicit transactions; the window server
// composites them. Made when a spectacle starts, closed when it ends.
use super::{scene, Place};
use crate::native::image;
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGImage};
use objc2_quartz_core::{kCAFilterNearest, kCAGravityResize, CALayer, CATransaction};
use std::{sync::mpsc, time::Duration};
use tauri::{AppHandle, Manager};

// Core Animation takes layer changes from any thread inside an explicit transaction; the panel is only touched on the
// main thread
struct AnyThread<T>(T);
unsafe impl<T> Send for AnyThread<T> {}
unsafe impl<T> Sync for AnyThread<T> {}

fn quietly(f: impl FnOnce()) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    f();
    CATransaction::commit();
}

/// A layer showing `img`, stretched to its bounds with nearest-neighbour: whole game pixels.
pub fn picture(img: &scene::Image, space: &CGColorSpace) -> (Retained<CALayer>, CFRetained<CGImage>) {
    let l = CALayer::new();
    l.setContentsGravity(unsafe { kCAGravityResize });
    l.setMagnificationFilter(unsafe { kCAFilterNearest });
    l.setMinificationFilter(unsafe { kCAFilterNearest });
    let cg = image(space, img.w as u32, img.h as u32, &img.rgba);
    set_contents(&l, &cg);
    (l, cg)
}

fn set_contents(l: &CALayer, cg: &CGImage) {
    let obj: &AnyObject = unsafe { &*(cg as *const CGImage).cast::<AnyObject>() };
    unsafe { l.setContents(Some(obj)) };
}

/// The villain's layer: facing left and right, his feet at the layer's position.
pub struct Villain {
    layer: Retained<CALayer>,
    faces: [CFRetained<CGImage>; 2],
    right: Option<bool>,
}

impl Villain {
    /// `imgs`: facing left, then right; `px` units a game pixel.
    pub fn new(root: &CALayer, imgs: &[scene::Image; 2], px: f64, space: &CGColorSpace) -> Self {
        let (layer, left) = picture(&imgs[0], space);
        let right = image(space, imgs[1].w as u32, imgs[1].h as u32, &imgs[1].rgba);
        quietly(|| {
            layer.setBounds(CGRect::new(CGPoint::ZERO, CGSize::new(imgs[0].w as f64 * px, imgs[0].h as f64 * px)));
            layer.setAnchorPoint(CGPoint::new(0.5, 1.0));
            layer.setOpacity(0.0);
            root.addSublayer(&layer);
        });
        Villain { layer, faces: [left, right], right: None }
    }

    pub fn pose(&mut self, p: &scene::Pose) {
        quietly(|| {
            self.layer.setPosition(CGPoint::new(p.x.round(), p.y.round()));
            self.layer.setOpacity(p.alpha as f32);
            if self.right != Some(p.right) {
                set_contents(&self.layer, &self.faces[p.right as usize]);
                self.right = Some(p.right);
            }
        });
    }

    pub fn fade(&self, alpha: f64) {
        quietly(|| self.layer.setOpacity(alpha as f32));
    }
}

/// The beam's layers: its body (one column of bands stretched along it), the head at its tip and a flare at its root.
pub struct Beam {
    body: Retained<CALayer>,
    head: Retained<CALayer>,
    flare: Retained<CALayer>,
    _imgs: [CFRetained<CGImage>; 2],
}

impl Beam {
    pub fn new(root: &CALayer, look: &scene::Look, space: &CGColorSpace) -> Self {
        let (body, col) = picture(&scene::column(look, 16), space);
        let disc = scene::disc(look, 16);
        let (head, d) = picture(&disc, space);
        let (flare, _) = picture(&disc, space);
        quietly(|| {
            body.setAnchorPoint(CGPoint::new(0.0, 0.5));
            for l in [&body, &head, &flare] {
                l.setOpacity(0.0);
                root.addSublayer(l);
            }
        });
        Beam { body, head, flare, _imgs: [col, d] }
    }

    /// `l` as `b` shapes it, in the root's coordinates (top left, y down).
    pub fn blast(&self, l: &scene::Line, b: &scene::Blast) {
        let (c, s) = (l.d.0, l.d.1);
        let tip = CGPoint::new(l.o.0 + c * b.len, l.o.1 + s * b.len);
        let square = |k: f64| CGRect::new(CGPoint::ZERO, CGSize::new(b.width * k, b.width * k));
        quietly(|| {
            self.body.setAffineTransform(CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 });
            self.body.setBounds(CGRect::new(CGPoint::ZERO, CGSize::new(b.len.max(0.0), b.width)));
            self.body.setPosition(CGPoint::new(l.o.0, l.o.1));
            self.body.setAffineTransform(CGAffineTransform { a: c, b: s, c: -s, d: c, tx: 0.0, ty: 0.0 });
            self.head.setBounds(square(1.3));
            self.head.setPosition(tip);
            self.flare.setBounds(square(1.8));
            self.flare.setPosition(CGPoint::new(l.o.0, l.o.1));
            for x in [&self.body, &self.head, &self.flare] {
                x.setOpacity(b.alpha as f32);
            }
        });
    }
}

// the panel and its root layer, made on the main thread
struct Stage {
    panel: AnyThread<Retained<AnyObject>>,
    root: AnyThread<Retained<CALayer>>,
}

// Opens the overlay over `area` (top-left points); None if the main thread doesn't make it within a second
fn open(app: &AppHandle, area: crate::system::Rect) -> Option<Stage> {
    let (tx, rx) = mpsc::channel();
    let main = app.get_webview_window("main")?;
    let widget = AnyThread(main.ns_window().ok()? as usize);
    app.run_on_main_thread(move || unsafe {
        let widget = widget.0 as *mut AnyObject;
        let screens: Retained<AnyObject> = msg_send![class!(NSScreen), screens];
        let first: Option<Retained<AnyObject>> = msg_send![&*screens, firstObject];
        let Some(first) = first else { return };
        // AppKit's y runs up from the bottom of the first screen
        let f0: CGRect = msg_send![&*first, frame];
        let frame = CGRect::new(CGPoint::new(area.x, f0.size.height - area.y - area.h), CGSize::new(area.w, area.h));
        let panel: objc2::rc::Allocated<AnyObject> = msg_send![class!(NSPanel), alloc];
        // borderless (0) and non-activating (1 << 7); buffered (2)
        let panel: Retained<AnyObject> = msg_send![panel, initWithContentRect: frame, styleMask: 1usize << 7, backing: 2usize, defer: false];
        let clear: Retained<AnyObject> = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![&*panel, setReleasedWhenClosed: false];
        let _: () = msg_send![&*panel, setOpaque: false];
        let _: () = msg_send![&*panel, setBackgroundColor: &*clear];
        let _: () = msg_send![&*panel, setHasShadow: false];
        let _: () = msg_send![&*panel, setIgnoresMouseEvents: true];
        let _: () = msg_send![&*panel, setHidesOnDeactivate: false];
        let _: () = msg_send![&*panel, setLevel: 3isize]; // floating, the widget's own when on top
                                                          // every Space, beside fullscreen apps, not in the window cycle, and still on Exposé
        let _: () = msg_send![&*panel, setCollectionBehavior: 1usize | 16 | 64 | 256];
        let _: () = msg_send![&*panel, setAnimationBehavior: 2isize]; // none
        let _: () = msg_send![&*panel, setSharingType: 0usize]; // left out of screen sharing and recordings
        let host: objc2::rc::Allocated<AnyObject> = msg_send![class!(NSView), alloc];
        let host: Retained<AnyObject> = msg_send![host, initWithFrame: CGRect::new(CGPoint::ZERO, frame.size)];
        let root = CALayer::new();
        root.setGeometryFlipped(true);
        let _: () = msg_send![&*host, setLayer: &*root];
        let _: () = msg_send![&*host, setWantsLayer: true];
        let _: () = msg_send![&*panel, setContentView: &*host];
        // under the widget when they share a level (it's on top), so the beam comes out from behind the hero
        let (level, number): (isize, isize) = (msg_send![widget, level], msg_send![widget, windowNumber]);
        if level >= 3 {
            let _: () = msg_send![&*panel, orderWindow: -1isize, relativeTo: number];
        } else {
            let _: () = msg_send![&*panel, orderWindow: 1isize, relativeTo: 0isize];
        }
        let _ = tx.send((AnyThread(panel), AnyThread(root)));
    })
    .ok()?;
    let (panel, root) = rx.recv_timeout(Duration::from_secs(1)).ok()?;
    Some(Stage { panel, root })
}

impl Stage {
    fn close(self, app: &AppHandle) {
        let panel = self.panel;
        let _ = app.run_on_main_thread(move || unsafe {
            let nil: *const AnyObject = std::ptr::null();
            let _: () = msg_send![&*panel.0, orderOut: nil];
            let _: () = msg_send![&*panel.0, close];
            drop(panel);
        });
    }

    // positions in the overlay's own coordinates
    fn local(&self, area: &crate::system::Rect, p: (f64, f64)) -> (f64, f64) {
        (p.0 - area.x, p.1 - area.y)
    }
}

pub fn raid(app: &AppHandle, p: &Place, from: (f64, f64), perches: &[(f64, f64)], imgs: [scene::Image; 2], go: impl Fn(&AppHandle) -> bool) {
    let Some(space) = CGColorSpace::new_device_rgb() else { return };
    let Some(stage) = open(app, p.display) else { return };
    let mut v = Villain::new(&stage.root.0, &imgs, p.px, &space);
    let from = stage.local(&p.display, from);
    let perches: Vec<(f64, f64)> = perches.iter().map(|q| stage.local(&p.display, *q)).collect();
    let mut last = None;
    let done = super::frames(
        scene::RAID_SECS,
        super::RAID_FPS,
        || go(app),
        |t| {
            // standing still, nothing to send
            if let Some(pose) = scene::raid_pose(from, &perches, p.px, imgs[0].h as f64 * p.px, t).filter(|q| last != Some(*q)) {
                v.pose(&pose);
                last = Some(pose);
            }
        },
    );
    // beaten or gone: he blinks out
    if !done {
        let a = last.map_or(0.0, |q| q.alpha);
        super::frames(0.25, super::RAID_FPS, || true, |t| v.fade(a * (1.0 - t / 0.25)));
    }
    stage.close(app);
}

pub fn beam(app: &AppHandle, p: &Place, line: scene::Line, look: scene::Look, go: impl Fn(&AppHandle) -> bool) {
    let Some(space) = CGColorSpace::new_device_rgb() else { return };
    let Some(stage) = open(app, p.display) else { return };
    let b = Beam::new(&stage.root.0, &look, &space);
    let local = scene::Line { o: stage.local(&p.display, line.o), ..line };
    super::frames(
        scene::BEAM_SECS,
        super::BEAM_FPS,
        || go(app),
        |t| {
            if let Some(s) = scene::blast(&local, t) {
                b.blast(&local, &s);
            }
        },
    );
    stage.close(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{Rect, Win};
    use objc2_core_graphics::{CGBitmapContextCreate, CGBitmapContextGetData, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo};

    // the layers drawn by Core Animation's own renderer into an offscreen bitmap (no window, nothing on screen): RGBA
    fn render(root: &CALayer, w: usize, h: usize) -> Vec<u8> {
        let space = CGColorSpace::new_device_rgb().unwrap();
        let info = CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::OrderDefault.0;
        let ctx = unsafe { CGBitmapContextCreate(std::ptr::null_mut(), w, h, 8, w * 4, Some(&space), info) }.unwrap();
        // the bitmap's y runs up: flipped, so row 0 is the top as on screen
        CGContext::translate_ctm(Some(&ctx), 0.0, h as f64);
        CGContext::scale_ctm(Some(&ctx), 1.0, -1.0);
        root.renderInContext(&ctx);
        let data = CGBitmapContextGetData(Some(&ctx)).cast::<u8>();
        unsafe { std::slice::from_raw_parts(data, w * h * 4) }.to_vec()
    }

    fn alpha(px: &[u8], w: usize, x: usize, y: usize) -> u8 {
        px[(y * w + x) * 4 + 3]
    }

    fn png(path: &std::path::Path, w: usize, h: usize, rgba: &[u8]) {
        let mut e = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).unwrap()), w as u32, h as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(rgba).unwrap();
    }

    // The real layer tree, rendered offscreen by Core Animation: the villain stands on a window's edge facing the
    // widget, and the beam runs from the hero to the screen's edge. SPECTACLE_CAPTURE=<dir> keeps the frames as PNGs.
    #[test]
    fn the_layers_draw_the_villain_on_an_edge_and_the_beam_to_the_edge() {
        let (w, h) = (900usize, 520usize);
        let space = CGColorSpace::new_device_rgb().unwrap();
        let root = CALayer::new();
        root.setGeometryFlipped(true);
        root.setBounds(CGRect::new(CGPoint::ZERO, CGSize::new(w as f64, h as f64)));
        let screen = Rect { x: 0.0, y: 0.0, w: w as f64, h: h as f64 };
        let wins = [Win { pid: 2, layer: 0, alpha: 1.0, rect: Rect { x: 120.0, y: 300.0, w: 420.0, h: 200.0 }, presenter: false }];
        let spr = planet_core::sprite::get("VILLAINS.3.spr"); // frieza
        let imgs = scene::faces(spr.w, spr.h, &spr.col);
        let px = 3.0;
        let edges = scene::edges(&wins, screen, 1, imgs[0].h as f64 * px + 8.0, imgs[0].w as f64 * px + 16.0);
        let from = (780.0, 140.0);
        let perches = scene::perches(&edges, from, scene::Edge { x0: 0.0, x1: w as f64, y: h as f64 - 4.0 });
        assert_eq!(perches, [(330.0, 300.0)]);
        let mut v = Villain::new(&root, &imgs, px, &space);
        v.pose(&scene::raid_pose(from, &perches, px, 0.0, 3.0).unwrap());
        let raid = render(&root, w, h);
        assert!(alpha(&raid, w, 330, 290) > 0, "he stands just above the edge");
        assert_eq!(alpha(&raid, w, 330, 310), 0, "and not below it");
        assert_eq!(alpha(&raid, w, 330, 290 - imgs[0].h * 3 - 6), 0, "nor floats above it");

        let beam = Beam::new(&root, &scene::KAME, &space);
        v.fade(0.0);
        let line = scene::line(from, -1.0, 0.3, screen, 48.0);
        beam.blast(&line, &scene::blast(&line, 0.5).unwrap());
        let shot = render(&root, w, h);
        let on = |k: f64| {
            let (x, y) = (from.0 + line.d.0 * k, from.1 + line.d.1 * k);
            alpha(&shot, w, x as usize, y as usize)
        };
        assert!(on(100.0) == 255 && on(400.0) == 255, "solid down its line");
        assert_eq!(alpha(&shot, w, 780, 20), 0, "and nothing far off it");
        let edge = from.0 / -line.d.0; // where it reaches x = 0
        assert!(on(edge - 30.0) == 255, "right to the screen's edge");

        // the software drawing (Windows, and the capture) agrees with Core Animation on where the beam is
        let mut cv = scene::Canvas::new(w, h);
        cv.beam(&line, &scene::blast(&line, 0.5).unwrap(), &scene::KAME, (0.0, 0.0));
        let both = (0..w * h).filter(|&k| (shot[k * 4 + 3] > 0) == (cv.rgba[k * 4 + 3] > 0)).count();
        assert!(both as f64 > 0.995 * (w * h) as f64, "{both} of {} agree", w * h);

        if let Some(dir) = std::env::var_os("SPECTACLE_CAPTURE") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            png(&dir.join("mac-layers-raid.png"), w, h, &raid);
            png(&dir.join("mac-layers-beam.png"), w, h, &shot);
        }
    }

    // The panel is made only on screen, which a test mustn't do: instead every AppKit call `open` and `close` make is
    // checked against the runtime's own signature, as objc2 checks it at run time (a wrong integer type would panic)
    #[test]
    fn the_panel_calls_match_appkit() {
        use objc2::{runtime::AnyClass, sel};
        let check = |cls: &AnyClass, s: objc2::runtime::Sel, args: &[&str], ret: &str| {
            let m = cls.instance_method(s).unwrap_or_else(|| panic!("{} has no {s:?}", cls.name().to_str().unwrap()));
            assert_eq!(m.arguments_count(), args.len() + 2, "{s:?}");
            for (i, want) in args.iter().enumerate().filter(|(_, w)| !w.is_empty()) {
                assert_eq!(m.argument_type(i + 2).unwrap().to_str().unwrap(), *want, "{s:?} argument {i}");
            }
            if !ret.is_empty() {
                assert_eq!(m.return_type().to_str().unwrap(), ret, "{s:?} returns");
            }
        };
        let (panel, view) = (class!(NSPanel), class!(NSView));
        check(panel, sel!(initWithContentRect:styleMask:backing:defer:), &["", "Q", "Q", ""], "@");
        for s in [sel!(setReleasedWhenClosed:), sel!(setOpaque:), sel!(setHasShadow:), sel!(setIgnoresMouseEvents:), sel!(setHidesOnDeactivate:)] {
            check(panel, s, &[""], "v");
        }
        check(panel, sel!(setBackgroundColor:), &["@"], "v");
        check(panel, sel!(setLevel:), &["q"], "v");
        check(panel, sel!(setCollectionBehavior:), &["Q"], "v");
        check(panel, sel!(setAnimationBehavior:), &["q"], "v");
        check(panel, sel!(setSharingType:), &["Q"], "v");
        check(panel, sel!(setContentView:), &["@"], "v");
        check(panel, sel!(orderWindow:relativeTo:), &["q", "q"], "v");
        check(panel, sel!(level), &[], "q");
        check(panel, sel!(windowNumber), &[], "q");
        check(panel, sel!(orderOut:), &["@"], "v");
        check(panel, sel!(close), &[], "v");
        check(view, sel!(initWithFrame:), &[""], "@");
        check(view, sel!(setLayer:), &["@"], "v");
        check(view, sel!(setWantsLayer:), &[""], "v");
    }

    // A raid's ten seconds and a beam's second and a half, frame by frame, on the layers: what a frame costs this
    // thread (Core Animation draws them in the window server)
    #[test]
    fn a_frame_costs_the_thread_little() {
        let space = CGColorSpace::new_device_rgb().unwrap();
        let root = CALayer::new();
        let spr = planet_core::sprite::get("VILLAINS.4.spr");
        let imgs = scene::faces(spr.w, spr.h, &spr.col);
        let mut v = Villain::new(&root, &imgs, 3.0, &space);
        let perches = [(200.0, 300.0), (700.0, 500.0), (1200.0, 250.0)];
        let t0 = std::time::Instant::now();
        let mut n = 0;
        for k in 0..(scene::RAID_SECS * 30.0) as usize {
            v.pose(&scene::raid_pose((1400.0, 100.0), &perches, 3.0, 0.0, k as f64 / 30.0).unwrap());
            n += 1;
        }
        let b = Beam::new(&root, &scene::GALICK, &space);
        let line = scene::line((1400.0, 100.0), -1.0, 0.2, Rect { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }, 48.0);
        for k in 0..(scene::BEAM_SECS * 60.0) as usize {
            b.blast(&line, &scene::blast(&line, k as f64 / 60.0).unwrap());
            n += 1;
        }
        let per = t0.elapsed() / n;
        eprintln!("spectacle: {n} frames, {per:?} each on the thread");
        assert!(per < Duration::from_millis(2), "{per:?} a frame");
    }
}
