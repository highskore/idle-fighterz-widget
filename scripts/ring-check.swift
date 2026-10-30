// Measures the first-run card's step 1 in a capture of the widget's window (screencapture -o -l <id>): the planet's
// drawn disc (the opaque pixels round the window's middle) and the gold dashed ring, each as a circle in points.
// Prints both as JSON and fails if the ring isn't centred on the disc within `tolerance` points, or if its gap from the
// disc's edge isn't between `min` and `max` points.
// Run: swift scripts/ring-check.swift <capture.png> <scale> [tolerance=2] [min=4] [max=20]
import CoreGraphics
import Foundation
import ImageIO

let a = CommandLine.arguments
guard a.count >= 3, let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[1]) as CFURL, nil),
      let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else { print("usage: ring-check.swift capture.png scale"); exit(2) }
let scale = Double(a[2])!, tol = a.count > 3 ? Double(a[3])! : 2, gapMin = a.count > 4 ? Double(a[4])! : 4, gapMax = a.count > 5 ? Double(a[5])! : 20
let W = img.width, H = img.height
var px = [UInt8](repeating: 0, count: W * H * 4)
let ctx = CGContext(data: &px, width: W, height: H, bitsPerComponent: 8, bytesPerRow: W * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
ctx.draw(img, in: CGRect(x: 0, y: 0, width: W, height: H))
func at(_ x: Int, _ y: Int) -> (Int, Int, Int, Int) { let k = (y * W + x) * 4; return (Int(px[k]), Int(px[k + 1]), Int(px[k + 2]), Int(px[k + 3])) }
func gold(_ x: Int, _ y: Int) -> Bool { let (r, g, b, al) = at(x, y); return al > 200 && abs(r - 217) < 30 && abs(g - 154) < 22 && abs(b - 28) < 35 }

// the ring: every gold pixel, a least-squares circle, outliers (the card's gold dots) dropped, fitted again
var pts: [(Double, Double)] = []
for y in 0..<H { for x in 0..<W where gold(x, y) { pts.append((Double(x), Double(y))) } }
func fit(_ p: [(Double, Double)]) -> (Double, Double, Double) {
  var sx = 0.0, sy = 0.0, sxx = 0.0, syy = 0.0, sxy = 0.0, sxz = 0.0, syz = 0.0, sz = 0.0
  for (x, y) in p { let z = x * x + y * y; sx += x; sy += y; sxx += x * x; syy += y * y; sxy += x * y; sxz += x * z; syz += y * z; sz += z }
  let n = Double(p.count)
  // [sxx sxy sx; sxy syy sy; sx sy n] [A B C] = [sxz syz sz]  (x²+y² = A x + B y + C)
  let m = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n]], v = [sxz, syz, sz]
  func det(_ m: [[Double]]) -> Double { m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]) }
  let d = det(m)
  func col(_ i: Int) -> Double { var c = m; for r in 0..<3 { c[r][i] = v[r] }; return det(c) / d }
  let (A, B, C) = (col(0), col(1), col(2)), cx = A / 2, cy = B / 2
  return (cx, cy, (C + cx * cx + cy * cy).squareRoot())
}
let noRing = pts.count <= 50
// three points at a time, the circle most gold pixels lie on (the dashes, not a stray gold dot or hut), then refined
func through(_ a: (Double, Double), _ b: (Double, Double), _ c: (Double, Double)) -> (Double, Double, Double)? {
  let d = 2 * (a.0 * (b.1 - c.1) + b.0 * (c.1 - a.1) + c.0 * (a.1 - b.1))
  if abs(d) < 1e-6 { return nil }
  let (a2, b2, c2) = (a.0 * a.0 + a.1 * a.1, b.0 * b.0 + b.1 * b.1, c.0 * c.0 + c.1 * c.1)
  let x = (a2 * (b.1 - c.1) + b2 * (c.1 - a.1) + c2 * (a.1 - b.1)) / d, y = (a2 * (c.0 - b.0) + b2 * (a.0 - c.0) + c2 * (b.0 - a.0)) / d
  return (x, y, ((a.0 - x) * (a.0 - x) + (a.1 - y) * (a.1 - y)).squareRoot())
}
func near(_ c: (Double, Double, Double), _ band: Double) -> [(Double, Double)] { pts.filter { abs((($0.0 - c.0) * ($0.0 - c.0) + ($0.1 - c.1) * ($0.1 - c.1)).squareRoot() - c.2) < band } }
var ring = (Double(W) / 2, Double(H) / 2, 0.0)
if !noRing {
  var rng = SystemRandomNumberGenerator(), best = 0
  let sample = pts.count > 4000 ? (0..<4000).map { _ in pts.randomElement(using: &rng)! } : pts
  for _ in 0..<6000 {
    guard let c = through(sample.randomElement(using: &rng)!, sample.randomElement(using: &rng)!, sample.randomElement(using: &rng)!), c.2 > 20 * scale, c.2 < Double(min(W, H)) / 2, c.0 > 0, c.1 > 0, c.0 < Double(W), c.1 < Double(H) else { continue }
    let n = sample.filter { abs((($0.0 - c.0) * ($0.0 - c.0) + ($0.1 - c.1) * ($0.1 - c.1)).squareRoot() - c.2) < 2 * scale }.count
    if n > best { best = n; ring = c }
  }
  for _ in 0..<3 { ring = fit(near(ring, 2 * scale)) }
}
// the disc: from the ring's centre (or the window's), how far the opaque, non-gold pixels reach in 72 directions; the
// disc's edge is the median reach (trees, the hero and the card only reach further), and opposite directions both at
// that edge move the centre to their middle. Repeated until it settles
func solid(_ x: Double, _ y: Double) -> Bool { let (ix, iy) = (Int(x.rounded()), Int(y.rounded())); guard ix >= 0, iy >= 0, ix < W, iy < H else { return false }; return at(ix, iy).3 > 200 && !gold(ix, iy) }
var (cx, cy) = (ring.0, ring.1), r = 0.0
var reach = [Double](repeating: 0, count: 72)
for _ in 0..<8 {
  // the nearest solid pixel to the start: inside the disc
  for k in 0..<72 {
    let t = Double(k) * Double.pi / 36, (dx, dy) = (cos(t), sin(t))
    var d = 0.0, miss = 0
    while d < Double(max(W, H)) { if solid(cx + d * dx, cy + d * dy) { miss = 0 } else { miss += 1; if miss > Int(2 * scale) { break } }; d += 0.5 }
    reach[k] = d - Double(miss) * 0.5
  }
  let sorted = reach.sorted(); r = sorted[36]
  var mx = 0.0, my = 0.0, n = 0.0
  for k in 0..<36 where abs(reach[k] - r) < 3 * scale && abs(reach[k + 36] - r) < 3 * scale {
    let t = Double(k) * Double.pi / 36, s = (reach[k] - reach[k + 36]) / 2
    mx += s * cos(t); my += s * sin(t); n += 1
  }
  if n > 0 { cx += 2 * mx / n; cy += 2 * my / n }
}
let edges = reach.filter { abs($0 - r) < 3 * scale }.count
let p = { (v: Double) in (v / scale * 10).rounded() / 10 }
let off = (((ring.0 - cx) * (ring.0 - cx) + (ring.1 - cy) * (ring.1 - cy)).squareRoot()) / scale, gap = (ring.2 - r) / scale
print("{\"disc\":{\"x\":\(p(cx)),\"y\":\(p(cy)),\"r\":\(p(r)),\"edges\":\(edges)},\"ring\":{\"x\":\(p(ring.0)),\"y\":\(p(ring.1)),\"r\":\(p(ring.2))},\"off\":\((off * 10).rounded() / 10),\"gap\":\((gap * 10).rounded() / 10)}")
if noRing { print("no ring in the capture"); exit(1) }
if edges < 18 { print("too few clear edges to find the disc"); exit(1) }
if off > tol { print("the ring is \(off) points off the disc's centre"); exit(1) }
if gap < gapMin || gap > gapMax { print("the ring is \(gap) points out from the disc's edge"); exit(1) }
