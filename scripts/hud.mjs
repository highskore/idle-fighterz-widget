// Builds dist/hud.html, the page the window shows with the rust-core feature: the widget's page (npm run page first)
// with game.js swapped for hud.js (and control.js), so the HUD runs on the app's planet-core and nothing in the page plays
// the game; and dist/control.html, the same page for the control centre's window.
// Run: node scripts/hud.mjs
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const read = f => readFileSync(join(root, f), 'utf8').replace(/\r\n/g, '\n')   // as game/build.mjs: CRLF checkouts match too
const fence = s => s.replace(/\/\*widget\*\/([\s\S]*?)\/\*\/widget\*\//g, '$1')   // as game/build.mjs --widget
const page = read('dist/index.html'), game = read('game/shenron.js') + fence(read('game/game.js'))
if (page.split(game).length !== 2) throw new Error('game.js not found once in dist/index.html: run npm run page first')
const hud = page.replace(game, () => read('control.js') + read('hud.js'))
writeFileSync(join(root, 'dist', 'hud.html'), hud)
// the control centre's window (control.js): the same page, marked as the panel
const tag = '<html lang="en" class="widget">'
if (hud.split(tag).length !== 2) throw new Error('the widget page has no <html class="widget">')
writeFileSync(join(root, 'dist', 'control.html'), hud.replace(tag, '<html lang="en" class="widget control">').replace('<script src="onboard.js"></script>\n', ''))   // no first-run card there
console.log(join(root, 'dist', 'hud.html'), 'and control.html written')
