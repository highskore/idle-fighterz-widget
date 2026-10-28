// What the native surface shows. planet-core + planet-render plug in here later; until then, a test pattern and a
// loop of recorded planet frames (scripts/frames.mjs).

/// A framebuffer that advances in time. `frame()` is RGBA8, straight alpha, rows top to bottom, `size()` pixels.
pub trait FrameSource: Send {
    fn size(&self) -> (u32, u32);
    fn step(&mut self, dt: f32);
    fn frame(&self) -> &[u8];
}

/// A ring of pixels orbiting a pulsing core on a transparent ground: changes every step, so it presents every frame.
pub struct TestPattern {
    t: f32,
    buf: Vec<u8>,
}

const PW: u32 = 96;
const PH: u32 = 96;

impl TestPattern {
    pub fn new() -> Self {
        let mut p = Self { t: 0.0, buf: vec![0; (PW * PH * 4) as usize] };
        p.draw();
        p
    }

    fn draw(&mut self) {
        self.buf.fill(0);
        let (cx, cy) = (PW as f32 / 2.0, PH as f32 / 2.0);
        let core = 14.0 + 4.0 * (self.t * 3.0).sin();
        for y in 0..PH {
            for x in 0..PW {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let d = (dx * dx + dy * dy).sqrt();
                let px = if d < core {
                    Some([255, 196, 64, 255]) // opaque core
                } else if (d - 34.0).abs() < 2.0 {
                    let a = (dy.atan2(dx) - self.t * 2.0).rem_euclid(std::f32::consts::TAU);
                    Some([90, 200, 255, (64.0 + 191.0 * a / std::f32::consts::TAU) as u8]) // ring with an alpha sweep
                } else {
                    None
                };
                if let Some(c) = px {
                    let k = ((y * PW + x) * 4) as usize;
                    self.buf[k..k + 4].copy_from_slice(&c);
                }
            }
        }
    }
}

impl FrameSource for TestPattern {
    fn size(&self) -> (u32, u32) {
        (PW, PH)
    }
    fn step(&mut self, dt: f32) {
        self.t += dt;
        self.draw();
    }
    fn frame(&self) -> &[u8] {
        &self.buf
    }
}

/// Recorded game frames played in a loop at their own rate: most 60fps steps leave the frame as it was.
pub struct PngLoop {
    size: (u32, u32),
    frames: Vec<Vec<u8>>,
    fps: f32,
    t: f32,
}

impl PngLoop {
    pub fn new(pngs: &[&[u8]], fps: f32) -> Result<Self, png::DecodingError> {
        let mut size = (0, 0);
        let mut frames = Vec::with_capacity(pngs.len());
        for bytes in pngs {
            let mut dec = png::Decoder::new(std::io::Cursor::new(*bytes));
            dec.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
            let mut reader = dec.read_info()?;
            let mut buf = vec![0; reader.output_buffer_size().expect("frame fits in memory")];
            let info = reader.next_frame(&mut buf)?;
            buf.truncate(info.buffer_size());
            assert_eq!(info.color_type, png::ColorType::Rgba, "frames are RGB(A)");
            size = (info.width, info.height);
            frames.push(buf);
        }
        Ok(Self { size, frames, fps, t: 0.0 })
    }

    /// the frames recorded by scripts/frames.mjs, at the 12fps they were taken
    pub fn planet() -> Self {
        macro_rules! frames {
            ($($n:literal)*) => { [$(include_bytes!(concat!("../../native-frames/0", $n, ".png")).as_slice()),*] };
        }
        Self::new(&frames!("00" "01" "02" "03" "04" "05" "06" "07" "08" "09" "10" "11"), 12.0).expect("bundled frames decode")
    }
}

impl FrameSource for PngLoop {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn step(&mut self, dt: f32) {
        self.t = (self.t + dt) % (self.frames.len() as f32 / self.fps);
    }
    fn frame(&self) -> &[u8] {
        &self.frames[(self.t * self.fps) as usize % self.frames.len()]
    }
}
