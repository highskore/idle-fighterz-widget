// The two spectacles as plain data, with no window in them: where the villain can stand (the visible top edges of your
// windows), where he is at each moment of his ten seconds, the beam's line to the screen's edge and its shape over its
// second and a half, and the pixel art both are drawn with. Units are the screen's own: points on macOS (top-left
// origin, as CGWindowList), physical pixels on Windows. The macOS layers and the Windows windows draw from this, and so
// does the offscreen capture in the tests.
use crate::system::{Rect, Win};

/// how long the villain stays out on your windows
pub const RAID_SECS: f64 = 10.0;
/// how long the beam lasts, from the blast to gone
pub const BEAM_SECS: f64 = 1.6;
// the leap out of the widget onto the first window, and back at the end
const LEAP: f64 = 0.6;
// a hop from one window to the next
const HOP: f64 = 0.55;
// at most this many windows are stood on
const MAX_PERCHES: usize = 4;
// a window's top edge this close to the display's top has no room above it (the menu bar is there on macOS)
const TOP_ROOM: f64 = 40.0;

/// A visible stretch of a window's top edge, `x0..x1` at `y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub x0: f64,
    pub x1: f64,
    pub y: f64,
}

// `[a, b]` less every interval in `cut`
fn subtract(a: f64, b: f64, cut: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![(a, b)];
    for &(c0, c1) in cut {
        out =
            out.into_iter()
                .flat_map(|(s0, s1)| {
                    if c1 <= s0 || c0 >= s1 {
                        vec![(s0, s1)]
                    } else {
                        [(s0, c0.max(s0)), (c1.min(s1), s1)].into_iter().filter(|(x0, x1)| x1 > x0).collect()
                    }
                })
                .collect();
    }
    out
}

/// What can be stood on: for each ordinary window (layer 0, front to back, not ours) on `display`, the widest stretch
/// of its top edge that no window in front of it covers, with `room` clear above it for the villain, and at least
/// `min_w` wide. Front-most first. Windows in front that cover a whole display (an overlay) don't count as covering.
pub fn edges(wins: &[Win], display: Rect, me: i64, room: f64, min_w: f64) -> Vec<Edge> {
    let covers_display = |r: &Rect| r.x <= display.x && r.y <= display.y && r.x + r.w >= display.x + display.w && r.y + r.h >= display.y + display.h;
    let seen = |w: &Win| w.pid != me && w.alpha > 0.0 && w.rect.w > 1.0 && w.rect.h > 1.0 && (0..=8).contains(&w.layer) && !covers_display(&w.rect);
    let mut out = Vec::new();
    for (i, w) in wins.iter().enumerate() {
        let r = w.rect;
        if !seen(w) || w.layer != 0 || r.w < min_w || r.h < 60.0 {
            continue;
        }
        let y = r.y;
        if y < display.y + TOP_ROOM.max(room) || y > display.y + display.h - 40.0 {
            continue;
        }
        let (a, b) = (r.x.max(display.x), (r.x + r.w).min(display.x + display.w));
        if b - a < min_w {
            continue;
        }
        // in front, and over the band the villain stands in (from `room` above the edge down to it)
        let cut: Vec<(f64, f64)> =
            wins[..i].iter().filter(|f| seen(f) && f.rect.y < y + 1.0 && f.rect.y + f.rect.h > y - room).map(|f| (f.rect.x, f.rect.x + f.rect.w)).collect();
        if let Some((x0, x1)) = subtract(a, b, &cut).into_iter().filter(|(x0, x1)| x1 - x0 >= min_w).max_by(|p, q| (p.1 - p.0).total_cmp(&(q.1 - q.0))) {
            out.push(Edge { x0, x1, y });
        }
    }
    out
}

/// Where the villain stands, in order: on up to four of `edges` (the front-most ones), the first the nearest to where
/// he leaps out (`from`), then each the nearest left. With nothing to stand on, the bottom of the display (`floor`).
pub fn perches(edges: &[Edge], from: (f64, f64), floor: Edge) -> Vec<(f64, f64)> {
    let mut left: Vec<(f64, f64)> = edges.iter().take(MAX_PERCHES).map(|e| (((e.x0 + e.x1) / 2.0).round(), e.y)).collect();
    if left.is_empty() {
        return vec![(from.0.clamp(floor.x0, floor.x1).round(), floor.y)];
    }
    let mut out = Vec::new();
    let mut at = from;
    while !left.is_empty() {
        let k = (0..left.len()).min_by(|&i, &j| dist(left[i], at).total_cmp(&dist(left[j], at))).unwrap();
        at = left.remove(k);
        out.push(at);
    }
    out
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The villain at one moment: where his feet are, whether he faces right, and how solid he is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub right: bool,
    pub alpha: f64,
}

// along an arc from a to b, `k` from 0 to 1, rising `lift` at the top
fn arc(a: (f64, f64), b: (f64, f64), k: f64, lift: f64) -> (f64, f64) {
    let k = k.clamp(0.0, 1.0);
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k - lift * 4.0 * k * (1.0 - k))
}

/// Where the villain is `t` seconds in: he leaps from the widget (`from`) to the first perch, stands on each in turn
/// (hovering a pixel, `px` units, every other moment), hops to the next, and at the end leaps back into the widget,
/// fading. His feet never go above `top`, so his arcs keep him on screen. None outside `0..RAID_SECS`.
pub fn raid_pose(from: (f64, f64), perches: &[(f64, f64)], px: f64, top: f64, t: f64) -> Option<Pose> {
    if !(0.0..RAID_SECS).contains(&t) || perches.is_empty() {
        return None;
    }
    let n = perches.len();
    let hold = (RAID_SECS - 2.0 * LEAP - HOP * (n - 1) as f64) / n as f64;
    let face = |a: (f64, f64), b: (f64, f64)| b.0 >= a.0;
    if t < LEAP {
        let (x, y) = arc(from, perches[0], t / LEAP, 40.0 * px);
        return Some(Pose { x, y: y.max(top), right: face(from, perches[0]), alpha: (t / 0.2).min(1.0) });
    }
    let mut s = t - LEAP;
    for i in 0..n {
        let p = perches[i];
        if s < hold {
            // standing: face the widget, and hover
            let bob = if (s / 0.45).floor() as i64 % 2 == 1 { px } else { 0.0 };
            return Some(Pose { x: p.0, y: p.1 - bob, right: from.0 >= p.0, alpha: 1.0 });
        }
        s -= hold;
        if i + 1 < n {
            if s < HOP {
                let q = perches[i + 1];
                let (x, y) = arc(p, q, s / HOP, (24.0 * px).max((q.0 - p.0).abs() * 0.15));
                return Some(Pose { x, y: y.max(top), right: face(p, q), alpha: 1.0 });
            }
            s -= HOP;
        }
    }
    let last = perches[n - 1];
    let k = (s / LEAP).min(1.0);
    let (x, y) = arc(last, from, k, 40.0 * px);
    Some(Pose { x, y: y.max(top), right: face(last, from), alpha: ((1.0 - k) / 0.4).min(1.0) })
}

/// The beam's colours, edge to core, as the game draws a beam: kamehameha blue, galick gun violet, final flash gold,
/// kaioken red.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub edge: u32,
    pub glow: u32,
    pub mid: u32,
}

pub const KAME: Look = Look { edge: 0x1f6ad8, glow: 0x7ac8ff, mid: 0xcdeeff };
pub const GALICK: Look = Look { edge: 0x7a3ae0, glow: 0xb98af8, mid: 0xe2cbff };
pub const FLASH: Look = Look { edge: 0xc8601a, glow: 0xffc21a, mid: 0xfff3a0 };
pub const KAIO: Look = Look { edge: 0xc8201a, glow: 0xff6a5a, mid: 0xffc8bf };

/// The beam: from `o` along the unit vector `d` to the display's edge (`len`), `width` across at its fullest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub o: (f64, f64),
    pub d: (f64, f64),
    pub len: f64,
    pub width: f64,
}

/// From `o` toward (`dx`, `dy`) to where it leaves `display`, plus the head's reach so it goes right off the edge.
pub fn line(o: (f64, f64), dx: f64, dy: f64, display: Rect, width: f64) -> Line {
    let l = dx.hypot(dy);
    let d = if l > 1e-9 { (dx / l, dy / l) } else { (1.0, 0.0) };
    // the ray's exit from the display: the nearest of the four sides it heads for
    let mut len = f64::INFINITY;
    for (dc, lo, hi, oc) in [(d.0, display.x, display.x + display.w, o.0), (d.1, display.y, display.y + display.h, o.1)] {
        if dc > 1e-9 {
            len = len.min((hi - oc) / dc);
        } else if dc < -1e-9 {
            len = len.min((lo - oc) / dc);
        }
    }
    let len = if len.is_finite() { len.max(0.0) } else { 0.0 };
    Line { o, d, len: len + width, width }
}

/// The beam at one moment: how far it has reached, how wide it is and how solid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    pub len: f64,
    pub width: f64,
    pub alpha: f64,
}

/// `t` seconds after the blast: it shoots out to the edge in a fifth of a second, holds at full width with a pulse,
/// then thins and fades. None outside `0..BEAM_SECS`.
pub fn blast(l: &Line, t: f64) -> Option<Blast> {
    if !(0.0..BEAM_SECS).contains(&t) {
        return None;
    }
    let grow = 0.2;
    let reach = 1.0 - (1.0 - (t / grow).min(1.0)).powi(3);
    let (width, alpha) = if t < 0.9 {
        (l.width * if (t * 16.0).floor() as i64 % 2 == 0 { 1.0 } else { 0.9 }, 1.0)
    } else {
        let k = (t - 0.9) / (BEAM_SECS - 0.9);
        (l.width * (1.0 - 0.6 * k), 1.0 - k)
    };
    Some(Blast { len: l.len * reach, width, alpha })
}

// ── pixel art: straight RGBA8, rows top to bottom

/// An image: `w`×`h` straight RGBA8.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

const RIM: u32 = 0x14141e;

/// A game sprite (`col`: 0xRRGGBB or negative for none) with the widget's 1px dark rim round it, so it reads on any
/// window, mirrored if `flip`.
pub fn sprite(w: usize, h: usize, col: &[i32], flip: bool) -> Image {
    let (iw, ih) = (w + 2, h + 2);
    let at = |x: i64, y: i64| -> Option<u32> {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            return None;
        }
        let x = if flip { w as i64 - 1 - x } else { x };
        let c = col[y as usize * w + x as usize];
        (c >= 0).then_some(c as u32)
    };
    let mut rgba = vec![0u8; iw * ih * 4];
    for y in 0..ih as i64 {
        for x in 0..iw as i64 {
            let c = at(x - 1, y - 1).or_else(|| {
                let near = [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| at(x - 1 + dx, y - 1 + dy).is_some());
                near.then_some(RIM)
            });
            if let Some(c) = c {
                let k = (y as usize * iw + x as usize) * 4;
                rgba[k..k + 4].copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
            }
        }
    }
    Image { w: iw, h: ih, rgba }
}

/// A villain facing left, then right. The game's villain sprites face right (the widget mirrors them to face the hero).
pub fn faces(w: usize, h: usize, col: &[i32]) -> [Image; 2] {
    [sprite(w, h, col, true), sprite(w, h, col, false)]
}

// the beam's bands across it, from its middle (0) to its edge (1): white, then the look's mid, glow and edge, in a
// faint halo of its glow
fn band(look: &Look, r: f64) -> Option<(u32, u8)> {
    match r {
        r if r < 0.25 => Some((0xffffff, 255)),
        r if r < 0.42 => Some((look.mid, 255)),
        r if r < 0.6 => Some((look.glow, 245)),
        r if r < 0.75 => Some((look.edge, 230)),
        r if r <= 1.0 => Some((look.glow, 90)),
        _ => None,
    }
}

fn put(rgba: &mut [u8], k: usize, (c, a): (u32, u8)) {
    rgba[k * 4..k * 4 + 4].copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, a]);
}

/// The beam's body, `n` pixels across and 1 along (it's the same all along, so it's stretched to length).
pub fn column(look: &Look, n: usize) -> Image {
    let mut rgba = vec![0u8; n * 4];
    for i in 0..n {
        let r = ((i as f64 + 0.5) / n as f64 * 2.0 - 1.0).abs();
        if let Some(b) = band(look, r) {
            put(&mut rgba, i, b);
        }
    }
    Image { w: 1, h: n, rgba }
}

/// The beam's head (and the flare at its root): a disc `n` pixels across in the same bands.
pub fn disc(look: &Look, n: usize) -> Image {
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let r = ((x as f64 + 0.5) / n as f64 * 2.0 - 1.0).hypot((y as f64 + 0.5) / n as f64 * 2.0 - 1.0);
            if let Some(b) = band(look, r) {
                put(&mut rgba, y * n + x, b);
            }
        }
    }
    Image { w: n, h: n, rgba }
}

// ── the same drawn in software, into a canvas: the offscreen capture, and the Windows windows

/// Premultiplied RGBA8 to draw into.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        Canvas { w, h, rgba: vec![0; w * h * 4] }
    }

    // straight colour `c` at alpha `a` over the pixel
    fn over(&mut self, x: i64, y: i64, c: &[u8], a: f64) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let a = (c[3] as f64 / 255.0 * a).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let k = (y as usize * self.w + x as usize) * 4;
        for (d, s) in self.rgba[k..k + 3].iter_mut().zip(c) {
            *d = (*s as f64 * a + *d as f64 * (1.0 - a)).round() as u8;
        }
        self.rgba[k + 3] = (255.0 * a + self.rgba[k + 3] as f64 * (1.0 - a)).round() as u8;
    }

    /// `img` scaled `k` (nearest), its bottom middle at (`x`, `y`), at `alpha`.
    pub fn feet(&mut self, img: &Image, x: f64, y: f64, k: f64, alpha: f64) {
        let (w, h) = (img.w as f64 * k, img.h as f64 * k);
        let (x0, y0) = ((x - w / 2.0).round() as i64, (y - h).round() as i64);
        for py in 0..h.ceil() as i64 {
            for px in 0..w.ceil() as i64 {
                let (sx, sy) = ((px as f64 / k) as usize, (py as f64 / k) as usize);
                if sx < img.w && sy < img.h {
                    let s = (sy * img.w + sx) * 4;
                    let c = [img.rgba[s], img.rgba[s + 1], img.rgba[s + 2], img.rgba[s + 3]];
                    self.over(x0 + px, y0 + py, &c, alpha);
                }
            }
        }
    }

    /// The beam `l` as `b` shapes it, `look`'s bands, drawn as the layers draw it: the body from the root out, the
    /// head at its tip and a flare at its root. `at` is the canvas's own origin in screen units.
    pub fn beam(&mut self, l: &Line, b: &Blast, look: &Look, at: (f64, f64)) {
        let (col, dsc) = (column(look, 16), disc(look, 16));
        let (ox, oy) = (l.o.0 - at.0, l.o.1 - at.1);
        let (tx, ty) = (ox + l.d.0 * b.len, oy + l.d.1 * b.len);
        let half = b.width / 2.0;
        let (head, flare) = (b.width * 1.3 / 2.0, b.width * 1.8 / 2.0);
        // only the rows and columns the beam's box covers
        let reach = flare.max(head);
        let (x0, x1) = (ox.min(tx) - reach, ox.max(tx) + reach);
        let (y0, y1) = (oy.min(ty) - reach, oy.max(ty) + reach);
        let sample = |img: &Image, u: f64, v: f64| -> Option<[u8; 4]> {
            let (sx, sy) = ((u * img.w as f64) as i64, (v * img.h as f64) as i64);
            (sx >= 0 && sy >= 0 && sx < img.w as i64 && sy < img.h as i64).then(|| {
                let s = (sy as usize * img.w + sx as usize) * 4;
                [img.rgba[s], img.rgba[s + 1], img.rgba[s + 2], img.rgba[s + 3]]
            })
        };
        for y in (y0.floor().max(0.0) as i64)..(y1.ceil().min(self.h as f64) as i64) {
            for x in (x0.floor().max(0.0) as i64)..(x1.ceil().min(self.w as f64) as i64) {
                let (rx, ry) = (x as f64 + 0.5 - ox, y as f64 + 0.5 - oy);
                let (along, across) = (rx * l.d.0 + ry * l.d.1, rx * l.d.1 - ry * l.d.0);
                // body, then head and flare over it, as the layers stack
                if (0.0..=b.len).contains(&along) && across.abs() < half {
                    if let Some(c) = sample(&col, 0.0, (across + half) / b.width) {
                        self.over(x, y, &c, b.alpha);
                    }
                }
                for (cx, cy, r) in [(tx, ty, head), (ox, oy, flare)] {
                    let (u, v) = ((x as f64 + 0.5 - cx + r) / (2.0 * r), (y as f64 + 0.5 - cy + r) / (2.0 * r));
                    if let Some(c) = sample(&dsc, u, v) {
                        self.over(x, y, &c, b.alpha);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(pid: i64, x: f64, y: f64, w: f64, h: f64) -> Win {
        Win { pid, layer: 0, alpha: 1.0, rect: Rect { x, y, w, h }, presenter: false }
    }
    const SCREEN: Rect = Rect { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 };

    #[test]
    fn the_villain_stands_only_on_edges_he_can_be_seen_on() {
        let wins = [
            win(5, 100.0, 300.0, 600.0, 400.0), // front
            win(6, 400.0, 200.0, 800.0, 500.0), // behind it: its edge is clear, the front one is below
            win(7, 50.0, 320.0, 300.0, 300.0),  // its edge is under the front window
            win(1, 900.0, 100.0, 400.0, 300.0), // ours
        ];
        let e = edges(&wins, SCREEN, 1, 110.0, 90.0);
        assert_eq!(e, [Edge { x0: 100.0, x1: 700.0, y: 300.0 }, Edge { x0: 400.0, x1: 1200.0, y: 200.0 }]);
    }

    #[test]
    fn edges_skip_the_menu_bar_tiny_windows_overlays_and_other_layers() {
        let mut floating = win(5, 0.0, 400.0, 600.0, 100.0);
        floating.layer = 3;
        let mut clear = win(9, 600.0, 500.0, 300.0, 300.0);
        clear.alpha = 0.0;
        let wins = [
            win(8, 0.0, 0.0, 1512.0, 982.0), // a window filling the screen: an overlay, never covers
            floating,                        // floating panels aren't stood on, but they do cover
            win(2, 0.0, 25.0, 300.0, 200.0), // under the menu bar: no room
            win(3, 1000.0, 300.0, 60.0, 400.0),
            win(4, 100.0, 450.0, 400.0, 300.0), // covered by the floating panel
            clear,
            win(6, 650.0, 520.0, 200.0, 200.0),
        ];
        let e = edges(&wins, SCREEN, 1, 110.0, 90.0);
        assert_eq!(e, [Edge { x0: 650.0, x1: 850.0, y: 520.0 }], "an invisible window covers nothing either");
    }

    #[test]
    fn a_window_cut_in_two_gives_its_wider_side() {
        let wins = [win(5, 300.0, 100.0, 200.0, 800.0), win(6, 0.0, 400.0, 1400.0, 300.0)];
        assert_eq!(edges(&wins, SCREEN, 1, 110.0, 90.0), [Edge { x0: 500.0, x1: 1400.0, y: 400.0 }]);
    }

    #[test]
    fn perches_go_nearest_first_and_fall_back_to_the_floor() {
        let e = [Edge { x0: 0.0, x1: 200.0, y: 300.0 }, Edge { x0: 1000.0, x1: 1200.0, y: 200.0 }, Edge { x0: 500.0, x1: 700.0, y: 600.0 }];
        assert_eq!(perches(&e, (1300.0, 100.0), Edge { x0: 0.0, x1: 1512.0, y: 900.0 }), [(1100.0, 200.0), (600.0, 600.0), (100.0, 300.0)]);
        let floor = Edge { x0: 10.0, x1: 1500.0, y: 900.0 };
        assert_eq!(perches(&[], (2000.0, 100.0), floor), [(1500.0, 900.0)]);
        let many: Vec<Edge> = (0..9).map(|i| Edge { x0: i as f64 * 150.0, x1: i as f64 * 150.0 + 100.0, y: 500.0 }).collect();
        assert_eq!(perches(&many, (0.0, 0.0), floor).len(), MAX_PERCHES);
    }

    #[test]
    fn the_raid_lasts_ten_seconds_and_ends_back_at_the_widget() {
        let from = (1300.0, 150.0);
        let p = [(1100.0, 200.0), (600.0, 600.0), (100.0, 300.0)];
        assert_eq!(raid_pose(from, &p, 3.0, 0.0, -0.1), None);
        assert_eq!(raid_pose(from, &p, 3.0, 0.0, RAID_SECS), None);
        assert_eq!(raid_pose(from, &[], 3.0, 0.0, 1.0), None);
        let start = raid_pose(from, &p, 3.0, 0.0, 0.0).unwrap();
        assert_eq!((start.x, start.y, start.alpha), (from.0, from.1, 0.0));
        // he stands on every perch for a while, and is solid there
        let hold = (RAID_SECS - 2.0 * LEAP - HOP * 2.0) / 3.0;
        for (i, q) in p.iter().enumerate() {
            let t = LEAP + i as f64 * (hold + HOP) + 0.1;
            let s = raid_pose(from, &p, 3.0, 0.0, t).unwrap();
            assert_eq!((s.x, s.y, s.alpha), (q.0, q.1, 1.0), "perch {i}");
            assert_eq!(s.right, from.0 >= q.0, "he faces the widget");
        }
        let end = raid_pose(from, &p, 3.0, 0.0, RAID_SECS - 1e-6).unwrap();
        assert!((end.x - from.0).abs() < 0.1 && (end.y - from.1).abs() < 0.1 && end.alpha < 0.01, "{end:?}");
        // and moves smoothly: no jump bigger than a hop could make in a 30th of a second
        let mut last = start;
        for k in 1..300 {
            let s = raid_pose(from, &p, 3.0, 0.0, k as f64 / 30.0).unwrap();
            assert!(dist((s.x, s.y), (last.x, last.y)) < 120.0, "t={} {last:?} → {s:?}", k as f64 / 30.0);
            last = s;
        }
    }

    #[test]
    fn the_beam_runs_to_the_screen_edge() {
        let l = line((1300.0, 150.0), -1.0, 0.0, SCREEN, 48.0);
        assert_eq!((l.d, l.len), ((-1.0, 0.0), 1300.0 + 48.0));
        let l = line((100.0, 100.0), 3.0, 4.0, SCREEN, 48.0);
        assert!((l.len - 48.0 - (882.0 / 0.8)).abs() < 1e-9, "the bottom first: {}", l.len);
        assert_eq!(line((10.0, 10.0), 0.0, 0.0, SCREEN, 10.0).d, (1.0, 0.0), "no direction: to the right");
        let b = |t| blast(&l, t);
        assert_eq!(b(-0.01), None);
        assert_eq!(b(BEAM_SECS), None);
        assert!(b(0.05).unwrap().len < l.len && b(0.2).unwrap().len == l.len, "it shoots out, then holds");
        assert_eq!(b(0.5).unwrap().alpha, 1.0);
        let late = b(1.5).unwrap();
        assert!(late.alpha < 0.2 && late.width < l.width * 0.5, "{late:?}");
    }

    #[test]
    fn sprites_get_a_rim_and_mirror() {
        let col = [0x112233, -1, -1, -1];
        let img = sprite(2, 2, &col, false);
        assert_eq!((img.w, img.h), (4, 4));
        let px = |i: &Image, x: usize, y: usize| i.rgba[(y * i.w + x) * 4..(y * i.w + x) * 4 + 4].to_vec();
        assert_eq!(px(&img, 1, 1), [0x11, 0x22, 0x33, 255]);
        assert_eq!(px(&img, 2, 1), [0x14, 0x14, 0x1e, 255], "the rim beside it");
        assert_eq!(px(&img, 2, 2), [0, 0, 0, 0], "not diagonally");
        assert_eq!(px(&sprite(2, 2, &col, true), 2, 1), [0x11, 0x22, 0x33, 255], "mirrored");
    }

    // A made-up desktop (a wallpaper and three app windows; never the real screen) with both spectacles drawn over it
    // in software, frame by frame. SPECTACLE_CAPTURE=<dir> keeps the frames as PNGs and one contact sheet
    #[test]
    fn capture_both_over_a_made_up_desktop() {
        let (w, h) = (1200usize, 720usize);
        let screen = Rect { x: 0.0, y: 0.0, w: w as f64, h: h as f64 };
        let wins = [win(5, 140.0, 360.0, 460.0, 300.0), win(6, 520.0, 220.0, 560.0, 380.0), win(7, 60.0, 170.0, 380.0, 170.0)];
        let desktop = || {
            let mut c = Canvas::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let k = (y * w + x) * 4;
                    let t = y as f64 / h as f64;
                    c.rgba[k..k + 4].copy_from_slice(&[(40.0 + 60.0 * t) as u8, (60.0 + 40.0 * t) as u8, (110.0 - 30.0 * t) as u8, 255]);
                }
            }
            // back to front, each a body and a title bar
            for r in wins.iter().rev().map(|v| v.rect) {
                for y in r.y as i64..(r.y + r.h) as i64 {
                    for x in r.x as i64..(r.x + r.w) as i64 {
                        let bar = (y as f64) < r.y + 28.0;
                        c.over(x, y, &if bar { [214, 214, 220, 255] } else { [246, 246, 248, 255] }, 1.0);
                    }
                }
            }
            c
        };
        // the widget sits top right; the villain leaps from his spot on its planet
        let (px, from) = (3.0, (1110.0, 110.0));
        let spr = planet_core::sprite::get("VILLAINS.4.spr"); // cell
        let imgs = faces(spr.w, spr.h, &spr.col);
        let e = edges(&wins, screen, 1, imgs[0].h as f64 * px + 8.0, imgs[0].w as f64 * px + 16.0);
        let p = perches(&e, from, Edge { x0: 60.0, x1: w as f64 - 60.0, y: h as f64 - 4.0 });
        assert_eq!(p.len(), 3, "{e:?}");
        let mut frames = Vec::new();
        for t in [0.3, 1.5, 4.0, 6.5, 9.0, 9.8] {
            let mut c = desktop();
            let pose = raid_pose(from, &p, px, imgs[0].h as f64 * px, t).unwrap();
            c.feet(&imgs[pose.right as usize], pose.x, pose.y, px, pose.alpha);
            frames.push((format!("raid-{t:.1}s"), c));
        }
        let line = line((1060.0, 130.0), -1.0, 0.35, screen, 22.0 * px);
        for t in [0.06, 0.3, 0.8, 1.3] {
            let mut c = desktop();
            c.beam(&line, &blast(&line, t).unwrap(), &KAME, (0.0, 0.0));
            frames.push((format!("beam-{t:.2}s"), c));
        }
        // standing, his feet are on a window's top edge
        let stand = raid_pose(from, &p, px, imgs[0].h as f64 * px, 4.0).unwrap();
        assert!(wins.iter().any(|v| (v.rect.y - stand.y).abs() <= px), "{stand:?}");

        let Some(dir) = std::env::var_os("SPECTACLE_CAPTURE") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, w: usize, h: usize, rgba: &[u8]| {
            let mut e = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(dir.join(name)).unwrap()), w as u32, h as u32);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header().unwrap().write_image_data(rgba).unwrap();
        };
        // the sheet: every frame at half size, two across
        let (hw, hh) = (w / 2, h / 2);
        let rows = frames.len().div_ceil(2);
        let mut sheet = vec![0u8; hw * 2 * hh * rows * 4];
        for (i, (name, c)) in frames.iter().enumerate() {
            save(&format!("{name}.png"), w, h, &c.rgba);
            let (ox, oy) = ((i % 2) * hw, (i / 2) * hh);
            for y in 0..hh {
                for x in 0..hw {
                    let (s, d) = (((y * 2) * w + x * 2) * 4, ((oy + y) * hw * 2 + ox + x) * 4);
                    sheet[d..d + 4].copy_from_slice(&c.rgba[s..s + 4]);
                }
            }
        }
        save("sheet.png", hw * 2, hh * rows, &sheet);
    }

    #[test]
    fn the_beam_is_white_in_the_middle_and_its_colour_at_the_edge() {
        let c = column(&KAME, 16);
        let at = |i: usize| c.rgba[i * 4..i * 4 + 4].to_vec();
        assert_eq!(at(8), [255, 255, 255, 255]);
        assert_eq!(at(0), [0x7a, 0xc8, 0xff, 90], "a faint halo");
        assert_eq!(at(2), [0x1f, 0x6a, 0xd8, 230], "the edge");
        let d = disc(&KAME, 16);
        assert_eq!(d.rgba[3], 0, "a disc's corner is clear");
        let mut cv = Canvas::new(200, 60);
        let l = line((10.0, 30.0), 1.0, 0.0, Rect { x: 0.0, y: 0.0, w: 200.0, h: 60.0 }, 32.0);
        cv.beam(&l, &blast(&l, 0.5).unwrap(), &KAME, (0.0, 0.0));
        let p = |x: usize, y: usize| cv.rgba[(y * 200 + x) * 4..(y * 200 + x) * 4 + 4].to_vec();
        assert_eq!(p(100, 30), [255, 255, 255, 255], "white down the middle");
        assert_eq!(p(100, 2), [0, 0, 0, 0], "nothing past its edge");
        assert_eq!(p(100, 16)[3], 90, "its halo");
    }
}
