// Puts the app's own pages in dist/ (npm run page first): the full settings window, the menu-bar popover and the
// first-run card, with fonts.css (their fonts inline: the CSP loads fonts from nowhere else), and adds the card to the
// widget's pages (index.html and hud.html).
// Run: node scripts/pages.mjs
import { copyFileSync, existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..'), dist = join(root, 'dist')
const face = (family, pkg, style) => {
  const file = join(root, 'node_modules', '@fontsource', pkg, 'files', `${pkg}-latin-400-${style}.woff2`)
  return `@font-face { font-family: "${family}"; font-style: ${style}; font-weight: 400; font-display: block; src: url(data:font/woff2;base64,${readFileSync(file).toString('base64')}) format("woff2"); }`
}
writeFileSync(join(dist, 'fonts.css'), [face('Fragment Mono', 'fragment-mono', 'normal'), face('Instrument Serif', 'instrument-serif', 'normal'), face('Instrument Serif', 'instrument-serif', 'italic')].join('\n') + '\n')
for (const f of ['settings-full.html', 'popover.html', 'onboard.js']) copyFileSync(join(root, f), join(dist, f))
copyFileSync(join(root, 'src-tauri', 'icons', '128x128.png'), join(dist, 'icon.png'))
const tag = '<script src="onboard.js"></script>'
for (const f of ['index.html', 'hud.html']) {
  const p = join(dist, f)
  if (!existsSync(p)) continue
  const page = readFileSync(p, 'utf8')
  const end = page.lastIndexOf('</body>')
  if (!page.includes(tag) && end > 0) writeFileSync(p, page.slice(0, end) + tag + '\n' + page.slice(end))
}
console.log(dist, 'pages written')
