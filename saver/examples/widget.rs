// A stand-in for the widget's side of the link, to try the screensaver without building the app: serves this user's
// endpoint and, while a screensaver is attached, sends it a run of its own at 30 fps with "stand-in" as the power level.
//
//   cargo run --release --example widget [seconds]
use ifz_saver::{Demo, DEMO_H, DEMO_W};
use std::time::{Duration, Instant};

fn main() {
    let secs: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let hub = saver_link::Hub::start(|| eprintln!("widget: a screensaver came or went")).expect("the endpoint is taken");
    let mut demo = Demo::new(42);
    let (t0, mut last, mut sent) = (Instant::now(), Instant::now(), 0u32);
    while t0.elapsed().as_secs_f64() < secs {
        std::thread::sleep(Duration::from_millis(33));
        if !hub.attached() {
            continue;
        }
        let now = Instant::now();
        if demo.step((now - last).as_secs_f64(), 0.0) {
            hub.publish(demo.fb.rgba(), DEMO_W, DEMO_H, "stand-in");
            sent += 1;
        }
        last = now;
    }
    eprintln!("widget: sent {sent} frames");
}
