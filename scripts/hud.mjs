// Builds dist/hud.html, the page the window shows with the rust-core feature: the widget's page (npm run page first)
// with game.js swapped for hud.js, so the HUD runs on the app's planet-core and nothing in the page plays the game.
// Run: node scripts/hud.mjs
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const read = f => readFileSync(join(root, f), 'utf8')
const fence = s => s.replace(/\/\*widget\*\/([\s\S]*?)\/\*\/widget\*\//g, '$1')   // as game/build.mjs --widget
const page = read('dist/index.html'), game = read('game/shenron.js') + fence(read('game/game.js'))
if (page.split(game).length !== 2) throw new Error('game.js not found once in dist/index.html: run npm run page first')
writeFileSync(join(root, 'dist', 'hud.html'), page.replace(game, () => read('hud.js')))
console.log(join(root, 'dist', 'hud.html'), 'written')
