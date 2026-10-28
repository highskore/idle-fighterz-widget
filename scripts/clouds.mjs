// Checks the widget's cloud templates (CLOUD_TPL in game/game.js) and draws every variant on one contact sheet.
// For every template, both mirrors, scales 0.7-1.3 at the planet sizes the widget uses (56 and 70 px), lobes a pixel
// smaller, as is and a pixel bigger, with and without the extra top lobe, it asserts:
//   - one connected silhouette (no separate balls), from 2-4 lobes that each overlap a neighbour by at least 40%
//   - a flat base: the bottom row covers at least 60% of the cloud's width
//   - wider than tall, 1.6-3
// Run: node scripts/clouds.mjs [sheet.png]   (exits 1 on any failure). The variant rules mirror cloudLobes() in game.js.
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { deflateSync } from 'node:zlib'

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '..', 'game', 'game.js'), 'utf8')
const TPL = Function('return ' + src.match(/const CLOUD_TPL = (\[[\s\S]*?\n {4}\]);/)[1])()
const NAMES = [...src.match(/const CLOUD_TPL = \[([\s\S]*?)\n {4}\];/)[1].matchAll(/\/\/ (.*)/g)].map(m => m[1])

// the same variant rules as cloudLobes() in game.js, with the seeded choices made explicit
function lobes(tp, { flip, scale, bump, extra }) {
  const s = flip * scale, a = scale
  const out = tp.map(([dx, dy, r]) => [dx * s, dy * a, r * a + bump])
  if (extra) { const [dx, dy, r] = tp.reduce((p, q) => q[1] - q[2] < p[1] - p[2] ? q : p); out.push([(dx + .07) * s, (dy - r * .55) * a, .09 * a]) }
  return out
}
// the same pixels drawCloud() fills: lobes cut at the base row
function raster(ls) {
  const x0 = Math.floor(Math.min(...ls.map(([dx, , r]) => dx - r))) - 2, x1 = Math.ceil(Math.max(...ls.map(([dx, , r]) => dx + r))) + 2
  const top = Math.floor(Math.min(...ls.map(([, dy, r]) => dy - r))) - 2, W = x1 - x0 + 1, H = -top + 3, base = -top
  const g = new Int8Array(W * H).fill(-1)
  ls.forEach(([dx, dy, r], i) => {
    const lx = dx - x0, ly = base + dy
    for (let y = Math.floor(ly - r); y <= base; y++) for (let x = Math.floor(lx - r); x <= lx + r; x++) {
      const ex = x - lx, ey = y - ly; if (ex * ex + ey * ey <= r * r && x >= 0 && y >= 0 && x < W && y < H) g[y * W + x] = i
    }
  })
  return { g, W, H, base, ls, lx0: x0 }
}
function check(tp, v, R) {
  const ls = lobes(tp, { ...v, scale: R * v.z }), { g, W, H, base } = raster(ls), err = []
  const on = []; for (let k = 0; k < g.length; k++) if (g[k] >= 0) on.push(k)
  const seen = new Uint8Array(g.length), st = [on[0]]; seen[on[0]] = 1; let n = 0
  while (st.length) { const k = st.pop(); n++; for (const j of [k - 1, k + 1, k - W, k + W]) if (j >= 0 && j < g.length && !seen[j] && g[j] >= 0 && Math.abs((j % W) - (k % W)) <= 1) { seen[j] = 1; st.push(j) } }
  if (n !== on.length) err.push(`${on.length - n} px apart from the main silhouette`)
  const xs = on.map(k => k % W), ys = on.map(k => (k / W) | 0), bw = Math.max(...xs) - Math.min(...xs) + 1, bh = Math.max(...ys) - Math.min(...ys) + 1
  let row = 0; for (let x = 0; x < W; x++) if (g[base * W + x] >= 0) row++
  if (row / bw < .6) err.push(`base row covers ${(row / bw * 100).toFixed(0)}% of the width`)
  if (bw / bh < 1.6 || bw / bh > 3) err.push(`aspect ${(bw / bh).toFixed(2)}`)
  const L = lobes(tp, { ...v, scale: 1, bump: 0 })
  if (L.length < 2 || L.length > 4) err.push(`${L.length} lobes`)
  L.forEach(([ax, ay, ar], i) => { const best = Math.max(...L.map(([bx, by, br], j) => j === i ? -1 : (ar + br - Math.hypot(ax - bx, ay - by)) / (2 * Math.min(ar, br)))); if (best < .4) err.push(`lobe ${i} overlaps its neighbour by ${(best * 100).toFixed(0)}%`) })
  return err
}

const variants = []
for (const flip of [1, -1]) for (const z of [.7, .85, 1, 1.15, 1.3]) for (const bump of [-1, 0, 1]) for (const extra of [false, true]) variants.push({ flip, z, bump, extra })
let bad = 0
TPL.forEach((tp, t) => {
  for (const R of [56, 70]) for (const v of variants) {
    if (v.extra && tp.length >= 4) continue
    const err = check(tp, v, R)
    if (err.length) { bad++; console.log(`✗ ${t} ${NAMES[t]}  R${R} z${v.z} ${v.flip < 0 ? 'mirrored' : ''} bump${v.bump}${v.extra ? ' +lobe' : ''}: ${err.join('; ')}`) }
  }
})
console.log(bad ? `${bad} variants fail` : `all ${TPL.length} templates pass in every variant`)

// contact sheet: a row per template; mirrors x scales 0.7 / 1 / 1.3 x extra lobe, at R 70, drawn 3x with the game's
// shading and the widget's dark rim over a sky blue
const out = process.argv[2]
if (out) {
  const PAL = [0xe2a93a, 0xf3c55a, 0xf9dd8c, 0xfff2c8], RIM = 0x140f0c, SKY = 0x9cc4e4, K = 3
  const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map(v => (v + .5) / 16), bayer = (x, y) => BAYER[(y & 3) * 4 + (x & 3)]
  const cells = TPL.map(tp => [1, -1].flatMap(flip => [.7, 1, 1.3].flatMap(z => [false, true].filter(e => !e || tp.length < 4).map(extra => raster(lobes(tp, { flip, scale: 70 * z, bump: 0, extra }))))))
  const cw = Math.max(...cells.flat().map(c => c.W)) + 6, ch = Math.max(...cells.flat().map(c => c.H)) + 6, cols = Math.max(...cells.map(r => r.length))
  const W = cw * cols, H = ch * cells.length, px = new Uint32Array(W * H).fill(SKY)
  cells.forEach((row, ri) => row.forEach((c, ci) => {
    const ox = ci * cw + ((cw - c.W) >> 1), oy = ri * ch + ch - c.H - 3, band = c.base * .35
    for (let y = 0; y < c.H; y++) for (let x = 0; x < c.W; x++) {
      const i = c.g[y * c.W + x]
      if (i >= 0) {
        // the lobe drawn last at this pixel is the highest one covering it, as in drawCloud()
        const order = c.ls.map((l, k) => k).sort((p, q) => c.ls[q][1] - c.ls[p][1]); let li = -1
        for (const k of order) { const [dx, dy, r] = c.ls[k], ex = x - (dx - c.lx0), ey = y - (c.base + dy); if (ex * ex + ey * ey <= r * r) li = k }
        const [dx, dy, r] = c.ls[li], ex = x - (dx - c.lx0), ey = y - (c.base + dy)
        const v = -(ex * .5 + ey * .9) / r + bayer(x, y) * .3 - Math.max(0, y - (c.base - band)) / band * .9
        px[(oy + y) * W + ox + x] = PAL[v > .65 ? 3 : v > .2 ? 2 : v > -.4 ? 1 : 0]
      } else if ([[1, 0], [-1, 0], [0, 1], [0, -1]].some(([a, b]) => { const X = x + a, Y = y + b; return X >= 0 && Y >= 0 && X < c.W && Y < c.H && c.g[Y * c.W + X] >= 0 })) px[(oy + y) * W + ox + x] = RIM
    }
  }))
  const big = new Uint32Array(W * K * H * K); for (let y = 0; y < H * K; y++) for (let x = 0; x < W * K; x++) big[y * W * K + x] = px[((y / K) | 0) * W + ((x / K) | 0)]
  writeFileSync(out, png(W * K, H * K, big))
  console.log('contact sheet:', out)
}
function png(w, h, px) {
  const CRC = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0 })
  const crc = b => { let c = 0xffffffff; for (const v of b) c = CRC[(c ^ v) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0 }
  const raw = Buffer.alloc(h * (w * 3 + 1))
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const c = px[y * w + x], o = y * (w * 3 + 1) + 1 + x * 3; raw[o] = c >> 16 & 255; raw[o + 1] = c >> 8 & 255; raw[o + 2] = c & 255 }
  const chunk = (t, d) => { const b = Buffer.alloc(12 + d.length); b.writeUInt32BE(d.length, 0); b.write(t, 4, 'ascii'); d.copy(b, 8); b.writeUInt32BE(crc(b.subarray(4, 8 + d.length)), 8 + d.length); return b }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))])
}
process.exitCode = bad ? 1 : 0
