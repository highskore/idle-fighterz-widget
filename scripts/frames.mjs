// Records the widget's game canvas, at its internal resolution, as PNG frames for the native renderer's stand-in
// source (src-tauri/native-frames/). Loads dist/index.html (npm run page first) in headless Chrome over the DevTools
// protocol and reads the canvas every 1/fps seconds.
// Run: node scripts/frames.mjs [count=12] [fps=12]   (CHROME=/path/to/chrome to override)
import { spawn } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const [count = 12, fps = 12] = process.argv.slice(2).map(Number)
const chrome = process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
const out = join(root, 'src-tauri', 'native-frames')
const profile = mkdtempSync(join(tmpdir(), 'powerrr-frames-'))
const proc = spawn(chrome, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--hide-scrollbars', 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] })
const wsUrl = await new Promise((res, rej) => {
  let log = ''
  proc.stderr.on('data', d => { log += d; const m = log.match(/DevTools listening on (ws:\S+)/); if (m) res(m[1]) })
  proc.on('exit', () => rej(new Error('chrome exited:\n' + log)))
})

const browser = new WebSocket(wsUrl)
await new Promise(r => browser.addEventListener('open', r))
let seq = 0
const pending = new Map()
browser.addEventListener('message', e => { const m = JSON.parse(e.data); pending.get(m.id)?.(m); pending.delete(m.id) })
const send = (method, params = {}, sessionId) => new Promise((res, rej) => {
  const id = ++seq
  pending.set(id, m => (m.error ? rej(new Error(method + ': ' + m.error.message)) : res(m.result)))
  browser.send(JSON.stringify({ id, method, params, sessionId }))
})
const { targetId } = await send('Target.createTarget', { url: 'about:blank' })
const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true })
const page = (m, p) => send(m, p, sessionId)
await page('Page.enable')
// the medium widget: 400pt square, where the game draws at 1×
await page('Emulation.setDeviceMetricsOverride', { width: 400, height: 400, deviceScaleFactor: 1, mobile: false })
await page('Page.navigate', { url: pathToFileURL(join(root, 'dist', 'index.html')).href })
const evaluate = async expr => (await page('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true })).result.value

// the game needs a moment to lay out and start its loop
await new Promise(r => setTimeout(r, 2500))
rmSync(out, { recursive: true, force: true })
mkdirSync(out, { recursive: true })
for (let i = 0; i < count; i++) {
  const url = await evaluate(`document.querySelector('.stage canvas').toDataURL('image/png')`)
  writeFileSync(join(out, `${String(i).padStart(3, '0')}.png`), Buffer.from(url.split(',')[1], 'base64'))
  await new Promise(r => setTimeout(r, 1000 / fps))
}
const size = await evaluate(`(c => c.width + 'x' + c.height)(document.querySelector('.stage canvas'))`)
console.log(`${count} frames at ${size} → ${out}`)
browser.close()
proc.kill()
rmSync(profile, { recursive: true, force: true })
