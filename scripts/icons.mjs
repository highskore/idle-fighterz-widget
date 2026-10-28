// Renders the app icon (a four-star dragon ball, 32×32 pixel art, dithered like the game) and the menu-bar
// template icon, scaled up pixel-exact to every size the bundle needs. Run: npm run icons
import { writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { deflateSync } from 'node:zlib'

const out = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'icons')
const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map(v => v / 16)
const bayer = (x, y) => BAYER[(y & 3) * 4 + (x & 3)]

// a five-pointed star stamp, 5×5
const STAR = ['..#..', '.###.', '#####', '.###.', '#.#.#'].map(r => [...r].map(c => c === '#'))

function ball(N = 32) {
  const px = new Uint32Array(N * N)   // 0xRRGGBBAA
  const c = (N - 1) / 2, r = N / 2 - 1
  const TONES = [0xa8400eff, 0xdc6a12ff, 0xf3901cff, 0xffb43aff, 0xffd770ff]
  for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) {
    const dx = (x - c) / r, dy = (y - c) / r, d2 = dx * dx + dy * dy
    if (d2 > 1) continue
    const z = Math.sqrt(1 - d2), lit = Math.max(0, -dx * .45 - dy * .55 + z * .7)   // lit from the top left
    const t = Math.min(TONES.length - 1, Math.floor(lit * TONES.length * .95 + bayer(x, y) * .9))
    px[y * N + x] = d2 > .82 && lit < .35 ? 0x7a2a08ff : TONES[t]
  }
  for (const [sx, sy] of [[8, 9], [17, 8], [9, 17], [18, 17]]) STAR.forEach((row, j) => row.forEach((on, i) => { if (on) px[(sy + j) * N + sx + i] = 0xd4281aff }))
  for (const [x, y] of [[8, 5], [9, 5], [10, 5], [7, 6], [8, 6], [6, 7]]) px[y * N + x] = 0xfff6d8ff   // the shine
  return { N, px }
}

// the menu-bar icon: a scouter, lens over the eye and the earpiece behind, as 18×18 pixel art doubled to 36px
// (the tray shows it at 18pt). black on clear so macOS tints it as a template image; the glass is see-through
const SCOUTER = [
  '..................',
  '..................',
  '..................',
  '..................',
  '..............##..',
  '...#########.####.',
  '...#ggggggg#.####.',
  '..#ggggggg#######.',
  '..#ggggggg#######.',
  '.#ggggggg#...####.',
  '.#########...#..#.',
  '.............#..#.',
  '.............####.',
  '..............##..',
  '..................',
  '..................',
  '..................',
  '..................',
]
function tray() {
  const N = SCOUTER.length, px = new Uint32Array(N * N)
  SCOUTER.forEach((row, y) => [...row].forEach((c, x) => { px[y * N + x] = c === '#' ? 0xff : c === 'g' ? 0x60 : 0 }))   // black, alpha only
  return scale({ N, px }, 36)
}

const scale = ({ N, px }, S) => { const k = S / N, o = new Uint32Array(S * S); for (let y = 0; y < S; y++) for (let x = 0; x < S; x++) o[y * S + x] = px[Math.floor(y / k) * N + Math.floor(x / k)]; return { N: S, px: o } }

const CRC = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0 })
const crc32 = b => { let c = 0xffffffff; for (const v of b) c = CRC[(c ^ v) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0 }
function png({ N, px }) {
  const raw = Buffer.alloc(N * (N * 4 + 1))
  for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) raw.writeUInt32BE(px[y * N + x], y * (N * 4 + 1) + 1 + x * 4)
  const chunk = (type, data) => { const b = Buffer.alloc(12 + data.length); b.writeUInt32BE(data.length, 0); b.write(type, 4, 'ascii'); data.copy(b, 8); b.writeUInt32BE(crc32(b.subarray(4, 8 + data.length)), 8 + data.length); return b }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(N, 0); ihdr.writeUInt32BE(N, 4); ihdr[8] = 8; ihdr[9] = 6
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))])
}
// .icns is a list of typed PNG entries
function icns(icon) {
  const parts = [['ic11', 32], ['ic12', 64], ['ic07', 128], ['ic13', 256], ['ic08', 256], ['ic14', 512], ['ic09', 512], ['ic10', 1024]].map(([type, s]) => {
    const data = png(scale(icon, s)), head = Buffer.alloc(8); head.write(type, 0, 'ascii'); head.writeUInt32BE(data.length + 8, 4); return Buffer.concat([head, data])
  })
  const head = Buffer.alloc(8); head.write('icns', 0, 'ascii'); head.writeUInt32BE(8 + parts.reduce((a, p) => a + p.length, 0), 4)
  return Buffer.concat([head, ...parts])
}

const icon = ball()
for (const [name, s] of [['32x32.png', 32], ['128x128.png', 128], ['128x128@2x.png', 256], ['icon.png', 1024]]) writeFileSync(join(out, name), png(scale(icon, s)))
writeFileSync(join(out, 'icon.icns'), icns(icon))
writeFileSync(join(out, 'tray.png'), png(tray()))
console.log('icons written to', out)
