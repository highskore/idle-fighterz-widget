// game.js drawing one frame headless, for planet.rs's pixel test: the game's rust/difftest/frame.js, plus --windowed
// for the widget's window mode (WIDGET on, BARE off: the site's cloud sea, no rim), which frame.js has no flag for.
// Usage: node scripts/frame.cjs <out-prefix> [frame.js's options] [--windowed]
const path = require('path')
const dir = path.join(__dirname, '..', 'game', 'rust', 'difftest')
const file = require.resolve(path.join(dir, 'harness.js')), harness = require(file)
if (process.argv.includes('--windowed')) require.cache[file].exports = o => { const g = harness({ ...o, widget: true }); g.ctx.BARE = false; return g }
require(path.join(dir, 'frame.js'))
