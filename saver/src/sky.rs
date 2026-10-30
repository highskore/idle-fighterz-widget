//! The site's backdrop behind the planet: its sky (the page's white, going to the near-black of the site's night under
//! a full moon) and the cloud sea scrolling along the bottom, as planet-render's `draw_sea` draws it for the site view.

use planet_render::{bayer, h2, pk, scene::hash, tones::tones, Framebuffer};

/// the site's game box by day, and its night (page.html `body.night`)
const DAY: u32 = 0xfcfcfb;
const NIGHT: u32 = 0x07070d;
/// the site's ink and muted text, and their night
pub const INK: [u32; 2] = [0x0b0b0b, 0xe8e8ec];
pub const MUTED: [u32; 2] = [0x8d8d8d, 0x8a8a9a];
/// planet-render's `NIGHT_SEA`
const NIGHT_SEA: [u32; 4] = [0x141a3a, 0x232c5a, 0x3a4a82, 0x6a7ab8];
/// the arena (`top`) has no sea
const NO_SEA: u32 = 9;

/// 0xRRGGBB `a` moved towards `b` by `k`.
pub fn mix(a: u32, b: u32, k: f64) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = (((a >> sh) & 255) as f64, ((b >> sh) & 255) as f64);
        (x + (y - x) * k).round() as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

pub fn sky(night: f64) -> u32 {
    mix(DAY, NIGHT, night)
}

/// `seaPal()`: the world's cloud sea, faded toward moonlit blue by night.
pub fn sea_pal(world: u32, night: f64) -> [u32; 4] {
    let day = tones(world).sea;
    std::array::from_fn(|i| mix(day.get(i).copied().unwrap_or(0xffffff), NIGHT_SEA[i], night))
}

/// The top of the back row of clouds, in rows from the top of an `h`-row canvas.
pub fn sea_top(h: f64) -> f64 {
    h * 0.86 - 6.0 - 7.0 * 0.45 - 7.0 * 1.2
}

/// `drawSea(t, front)` at the site's scale (3), over the whole of `fb`.
pub fn draw_sea(fb: &mut Framebuffer, pal: &[u32; 4], world: u32, t: f64, front: bool) {
    if world == NO_SEA {
        return;
    }
    let (w, h) = (fb.w as f64, fb.h as f64);
    let glint = world == 8;
    let base = h - h * if front { 0.06 } else { 0.14 };
    let sp = if front { 6.0 } else { 3.0 };
    let rr: f64 = if front { 9.0 } else { 7.0 };
    let mut i = -2.0;
    while i < w / (rr * 1.3) + 3.0 {
        let span = w + rr * 4.0;
        let px = ((i * rr * 1.3 - t * sp) % span + span) % span - rr * 2.0;
        let py = base + (i * 1.7).sin() * rr * 0.45 + if front { 0.0 } else { -6.0 };
        let r = rr * (0.8 + 0.4 * hash(i * 3.3 + if front { 9.0 } else { 0.0 }));
        let mut y = (py - r).floor();
        while y < h {
            let mut x = (px - r).floor();
            while x <= px + r {
                let (dx, dy) = (x - px, y - py);
                if !((dy < 0.0 && dx * dx + dy * dy > r * r) || (dy >= 0.0 && dx.abs() > r)) {
                    let l = if dy < 0.0 { -(dx * 0.5 + dy * 0.9) / r } else { -0.6 - dy / (h - py + 1.0) };
                    let v = l + bayer(x as i32, y as i32) * 0.3;
                    let col = if glint && v < -0.2 && ((x as i64 * 3 + y as i64) & 7) == 0 && h2(x * 7.0 + ((t * 4.0) as i64) as f64, y) > 0.94 {
                        0xe0702a
                    } else {
                        pal[if v > 0.65 {
                            3
                        } else if v > 0.2 {
                            2
                        } else if v > -0.4 {
                            1
                        } else {
                            0
                        }]
                    };
                    fb.px(x, y, col, 1.0);
                }
                x += 1.0;
            }
            y += 1.0;
        }
        i += 1.0;
    }
}

/// `rgba` (straight alpha, `w`×`h`) over `fb` with its top left at (`x`, `y`), cut to `fb`.
pub fn blend(fb: &mut Framebuffer, rgba: &[u8], w: usize, h: usize, x: i64, y: i64) {
    for row in 0..h as i64 {
        let fy = y + row;
        if fy < 0 || fy >= fb.h as i64 {
            continue;
        }
        let (x0, x1) = (x.max(0), (x + w as i64).min(fb.w as i64));
        for fx in x0..x1 {
            let s = ((row * w as i64 + fx - x) * 4) as usize;
            let a = rgba[s + 3] as u32;
            if a == 0 {
                continue;
            }
            let d = &mut fb.buf[fy as usize * fb.w + fx as usize];
            if a == 255 {
                *d = u32::from_le_bytes([rgba[s], rgba[s + 1], rgba[s + 2], 255]);
                continue;
            }
            let o = d.to_le_bytes();
            let c = |i: usize| ((rgba[s + i] as u32 * a + o[i] as u32 * (255 - a) + 127) / 255) as u8;
            *d = u32::from_le_bytes([c(0), c(1), c(2), 255]);
        }
    }
}

/// The sky alone, opaque.
pub fn fill(fb: &mut Framebuffer, hex: u32) {
    fb.buf.fill(pk(hex, 255));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_darkens_the_sky_and_the_sea() {
        assert_eq!(sky(0.0), DAY);
        assert_eq!(sky(1.0), NIGHT);
        assert_eq!(sea_pal(0, 1.0), NIGHT_SEA);
        assert_ne!(sea_pal(0, 0.0), sea_pal(1, 0.0), "each world its own sea");
    }

    #[test]
    fn the_sea_fills_the_bottom_and_leaves_the_sky() {
        let mut fb = Framebuffer::new(320, 200);
        fill(&mut fb, 0xffffff);
        draw_sea(&mut fb, &sea_pal(0, 0.0), 0, 12.0, false);
        let sky = pk(0xffffff, 255);
        assert!(fb.buf[..320 * 140].iter().all(|&p| p == sky), "clouds above the sea");
        assert!(fb.buf[320 * 199..].iter().all(|&p| p != sky), "a gap in the sea");
        let mut arena = Framebuffer::new(32, 20);
        draw_sea(&mut arena, &sea_pal(9, 0.0), 9, 0.0, true);
        assert!(arena.buf.iter().all(|&p| p == 0));
    }

    #[test]
    fn blending_cuts_to_the_canvas_and_mixes_half_alpha() {
        let mut fb = Framebuffer::new(4, 4);
        fill(&mut fb, 0x000000);
        let mut px = vec![0u8; 3 * 3 * 4];
        px[..4].copy_from_slice(&[255, 0, 0, 255]);
        px[4..8].copy_from_slice(&[0, 0, 255, 128]);
        blend(&mut fb, &px, 3, 3, -1, -1);
        // (0, 0) of the frame is off the canvas; (1, 0) is too; nothing else is drawn
        assert!(fb.buf.iter().all(|&p| p == pk(0, 255)));
        blend(&mut fb, &px, 3, 3, 2, 3);
        assert_eq!(fb.buf[3 * 4 + 2].to_le_bytes(), [255, 0, 0, 255]);
        assert_eq!(fb.buf[3 * 4 + 3].to_le_bytes(), [0, 0, 128, 255]);
    }
}
