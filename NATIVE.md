# Native renderer

Stage 5 of the Rust port. `planet-core` + `planet-render` (highskore/planet, `rust/`) will step the game and draw it into an RGBA framebuffer at the game's internal resolution. This is the shell that shows that framebuffer in the Powerrr window without a web view. Until the core lands, a stand-in source feeds it.

Nothing changes for users: the code compiles only with the `native-renderer` cargo feature, and even then only a hidden setting turns it on.

## Try it

```sh
npm run page                                   # the web widget, still the default
npx tauri build --bundles app --features native-renderer
# in ~/Library/Application Support/<bundle id>/settings.json:
#   "native_renderer": "planet"    recorded planet frames, looped at 12fps
#   "native_renderer": "pattern"   an animated test pattern that changes every frame
POWERRR_NATIVE_STATS=1 src-tauri/target/release/bundle/macos/Powerrr.app/Contents/MacOS/powerrr   # frame-time counters on stderr every 5 s
```

The setting is read straight from the file at launch and isn't in `DEFAULTS`, so changing any setting from the UI rewrites the file without it. Put it back to try again.

## How it works

`src-tauri/src/native/`:

- **`FrameSource`** (`source.rs`): `size()`, `step(dt)`, `frame() -> &[u8]` (RGBA8, straight alpha, rows top to bottom). Two stand-ins: `TestPattern` (96×96, a pulsing core and a ring with an alpha sweep; it changes every step) and `PngLoop` (the frames in `src-tauri/native-frames/`, 400×356, the game canvas of the medium widget at 1×, recorded by `node scripts/frames.mjs` from the built page in headless Chrome). `planet-native` implements the same trait over `planet-core` + `planet-render`, and `native::wanted` gets a third arm.
- **The surface** (`mod.rs`): the main window stays the one from `tauri.conf.json`, already frameless, transparent and always on top, with its tray, shortcut, position and settings code untouched. In native mode its web view goes to `about:blank` and is hidden, and a `CALayer` is added to the window's content view.
- **The loop**: a thread steps the source at 60 Hz and compares the frame with the last one. Only when it changed does it premultiply a copy into a `CGImage` and hand that to the main thread, which sets it as the layer's `contents` inside a `CATransaction` with actions off (otherwise Core Animation cross-fades every frame for 0.25 s). At most one present is in flight; a newer frame replaces a waiting one. While the window is hidden the loop stands still.
- **Scaling**: the layer is sized to the largest whole multiple of the frame that fits the window in device pixels, centred and snapped to the device-pixel grid, with `magnificationFilter = nearest`. The window server does the scaling on the GPU; the CPU only ever touches the 1× frame.

### Why a bare CALayer, not wgpu or softbuffer

- **softbuffer** drops alpha on macOS: its CoreGraphics backend makes its images `kCGImageAlphaNoneSkipFirst`, so the planet would sit on a black square. It would also scale on the CPU.
- **wgpu** works (a Metal surface with `CompositeAlphaMode::PostMultiplied`), but it brings about a hundred crates, a Metal device and a render pipeline to do one nearest-neighbour blit of a small texture, which is what a `CALayer` with a nearest filter already is. It's the better choice once the renderer itself moves to the GPU, or for Windows and Linux.
- **CALayer**: about 200 lines with the `objc2` crates Powerrr already depends on (three more of the same family, plus `png` for the stand-in; all already in the lockfile through Tauri), and no work for the window server when the frame doesn't change. macOS only, like the rest of the widget's native code; on other platforms the feature compiles to nothing.

## Measurements

Release builds (`--no-sign`, bundle id `com.highskore.powerrr.agenttest`, built with `CARGO_BUILD_JOBS=2 nice -n 15`), medium size on a Retina display, pointer away from the widget. Each run launches the binary, waits 8 s, then takes the CPU time used over 30 s by the app and every WebKit process it started (`ps` cumulative time), as % of one core. Three rounds, interleaved; another agent's builds were running on the machine, so expect a point or two of noise.

| | round 1 | round 2 | round 3 | mean |
|---|---|---|---|---|
| web view (today's widget, idle at 15fps) | 18.2% | 21.1% | 17.2% | **18.8%** (app 2.3, WebContent 14.4, GPU 2.1) |
| native, planet frames (60 steps/s, 12 new frames/s) | 3.5% | 3.6% | 3.2% | **3.4%** |
| native, test pattern (60 new frames/s) | 4.6% | 4.4% | 1.9% | **3.6%** |

In the native runs the WebKit processes, parked on `about:blank`, used nothing measurable. So the native path costs about a fifth of the web view's CPU while stepping at 60 Hz, against a web view that only draws at 15fps when idle.

Frame time, from `POWERRR_NATIVE_STATS` (5 s windows):

| | presents/s | step + compare + premultiply + CGImage (loop thread) | present (main thread) |
|---|---|---|---|
| planet frames, 400×356 | 12.0 | avg 0.09–0.47 ms, max 0.6–10 ms | avg 0.8–2.6 ms, max 2–6 ms |
| test pattern, 96×96 | 60.0 | avg 0.06–0.62 ms, max 0.1–16 ms | avg 0.17–0.65 ms, max 0.2–5 ms |

Both stay far inside the 16.7 ms budget, and the loop never fell behind (60.0 steps/s in every window). Most of the present time goes on Core Animation copying a new `CGImage` to the GPU on the main thread, so it grows with the frame's size. If `planet-render` runs at a bigger internal size, the next step is to render into an `IOSurface` and set that as `contents`, which avoids the copy. The web view's frame time can't be read from a release build (no inspector), so the CPU totals above are the comparison.

## The HUD with a native surface

The HUD is the dock pill and the other four layouts, the upgrade and ki panels, achievements, characters and the settings window: text in three web fonts, scrolling lists, hover states, transitions. Two ways to keep it:

1. **A transparent web view overlay** (recommended). The same window keeps its `WKWebView` above the `CALayer`, but the page is the HUD only: no game canvas, no game loop. `planet-core` sends it a small state snapshot (power level, form, bar, dragon balls, shop state) a few times a second as an event; the page sends actions back (buy, train, open settings) with `invoke`. Pointer events land on the page first: on a HUD element it handles them as today; anywhere else it forwards the press to the core (`invoke('game_pointer', { x, y })`) or starts a window drag, as `widget.js` already decides. The settings window doesn't change at all.
   - Cost: a `WKWebView` showing a static page uses close to nothing. The native run above already pays for one, parked on `about:blank`, and it's in the totals. The HUD's fades and bar animations cost only while they run, and the HUD is hidden when the pointer is away.
   - Work: split `widget.js`/`page.html` into the HUD page and the IPC glue; the game's DOM-free parts move to Rust anyway.
2. **Native-drawn HUD**. Draw the pill and panels into the framebuffer (pixel font, as the game's sprites) or with AppKit views over the layer. One process, no WebKit at all: the WebContent, GPU and Networking processes go away, which is the remaining idle floor. But it means re-making every panel, the scroll lists, text layout and the settings window, and keeping two HUDs in step while the site still uses the HTML one.

Recommendation: ship 1 first. It keeps one HUD for the site and the widget, and the measurements show the web view's idle cost is small once it isn't drawing the game. Revisit 2 only if the WebKit processes' floor (memory more than CPU) turns out to matter.
