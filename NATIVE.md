# Native renderer

Stage 5 of the Rust port. `planet-core` + `planet-render` (highskore/idle-fighterz, `rust/`) will step the game and draw it into an RGBA framebuffer at the game's internal resolution. This is the shell that shows that framebuffer in the Idle FighterZ window without a web view. Until the core lands, a stand-in source feeds it.

Nothing changes for users: the code compiles only with the `native-renderer` cargo feature, and even then only a hidden setting turns it on.

## Try it

```sh
npm run page                                   # the web widget, still the default
npx tauri build --bundles app --features native-renderer
# in ~/Library/Application Support/<bundle id>/settings.json:
#   "native_renderer": "planet"    recorded planet frames, looped at 12fps
#   "native_renderer": "pattern"   an animated test pattern that changes every frame
IDLE_FIGHTERZ_NATIVE_STATS=1 "src-tauri/target/release/bundle/macos/Idle FighterZ.app/Contents/MacOS/idle-fighterz"   # frame-time counters on stderr every 5 s
```

The setting is read straight from the file at launch and isn't in `DEFAULTS`, so changing any setting from the UI rewrites the file without it. Put it back to try again.

## How it works

`src-tauri/src/native/`:

- **`FrameSource`** (`source.rs`): `size()`, `step(dt)`, `frame() -> &[u8]` (RGBA8, straight alpha, rows top to bottom). Two stand-ins: `TestPattern` (96×96, a pulsing core and a ring with an alpha sweep; it changes every step) and `PngLoop` (the frames in `src-tauri/native-frames/`, 400×356, the game canvas of the medium widget at 1×, recorded by `node scripts/frames.mjs` from the built page in headless Chrome). `planet-native` implements the same trait over `planet-core` + `planet-render`, and `native::wanted` gets a third arm.
- **The surface** (`mod.rs`): the main window stays the one from `tauri.conf.json`, already frameless, transparent and always on top, with its tray, shortcut, position and settings code untouched. In native mode its web view goes to `about:blank` and is hidden, and a `CALayer` is added to the window's content view.
- **The loop**: a thread steps the source at 60 Hz and compares the frame with the last one. Only when it changed does it premultiply a copy into a `CGImage` and hand that to the main thread, which sets it as the layer's `contents` inside a `CATransaction` with actions off (otherwise Core Animation cross-fades every frame for 0.25 s). At most one present is in flight; a newer frame replaces a waiting one. It steps at the frame-rate setting (60 in the measurements below). While the window is hidden the loop waits, with no timer, until it shows.
- **Scaling**: the layer is sized to the largest whole multiple of the frame that fits the window in device pixels, centred and snapped to the device-pixel grid, with `magnificationFilter = nearest`. The window server does the scaling on the GPU; the CPU only ever touches the 1× frame.

### Why a bare CALayer, not wgpu or softbuffer

- **softbuffer** drops alpha on macOS: its CoreGraphics backend makes its images `kCGImageAlphaNoneSkipFirst`, so the planet would sit on a black square. It would also scale on the CPU.
- **wgpu** works (a Metal surface with `CompositeAlphaMode::PostMultiplied`), but it brings about a hundred crates, a Metal device and a render pipeline to do one nearest-neighbour blit of a small texture, which is what a `CALayer` with a nearest filter already is. It's the better choice once the renderer itself moves to the GPU, or for Windows and Linux.
- **CALayer**: about 200 lines with the `objc2` crates Idle FighterZ already depends on (three more of the same family, plus `png` for the stand-in; all already in the lockfile through Tauri), and no work for the window server when the frame doesn't change. macOS only, like the rest of the widget's native code; on other platforms the feature compiles to nothing.

## Measurements

Release builds (`--no-sign`, bundle id `com.highskore.idlefighterz.agenttest`, built with `CARGO_BUILD_JOBS=2 nice -n 15`), medium size on a Retina display, pointer away from the widget. Each run launches the binary, waits 8 s, then takes the CPU time used over 30 s by the app and every WebKit process it started (`ps` cumulative time), as % of one core. Three rounds, interleaved; another agent's builds were running on the machine, so expect a point or two of noise.

| | round 1 | round 2 | round 3 | mean |
|---|---|---|---|---|
| web view (today's widget, idle at 15fps) | 18.2% | 21.1% | 17.2% | **18.8%** (app 2.3, WebContent 14.4, GPU 2.1) |
| native, planet frames (60 steps/s, 12 new frames/s) | 3.5% | 3.6% | 3.2% | **3.4%** |
| native, test pattern (60 new frames/s) | 4.6% | 4.4% | 1.9% | **3.6%** |

In the native runs the WebKit processes, parked on `about:blank`, used nothing measurable. So the native path costs about a fifth of the web view's CPU while stepping at 60 Hz, against a web view that only draws at 15fps when idle.

Frame time, from `IDLE_FIGHTERZ_NATIVE_STATS` (5 s windows):

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

## The Rust core (feature `rust-core`)

Stage 5a and 5b. `planet-core` (`game/rust/planet-core`, a path dependency into the submodule) runs the game in the app, `planet-render` draws it, and the window's page is only its HUD. Without the feature, the build is unchanged.

```sh
npm run page:hud                               # dist/index.html, then dist/hud.html from it
npx tauri dev --features rust-core                       # frames on the HUD page's canvas
npx tauri dev --features rust-core,native-renderer      # macOS: with "native_renderer": "core" in settings.json, on a layer under it
```

- **`src-tauri/src/planet.rs`**: `Core` holds `planet_core::bridge::Bridge` in a `Mutex`. Each row of `game/rust/HUD.md`'s calls table is a `planet_<call>` command, with HUD.md's argument names (`now_ms`, `bx`, …) and its results (numbers, bools, JSON strings). Not included: `new` (the app makes the game, which outlives the page) and `spots`/`probe` (the web build's lockstep twin).
- **The tick**: a thread steps the game at the rate widget.js picks for the page (the frame-rate setting, doubled while it's awake: hover, focus or an open panel; `hud.js` passes it on with `planet_fps`), at most 30 times a second, as the game's fixed step is 1/30 s. It sends each non-empty `frame()` message to the main window as a `planet` event. Nothing else calls `frame()` while the widget shows, so each delta reaches the page exactly once.
- **Hidden**: the page gets nothing. If a notification or the menu-bar power level is on, the thread wakes once a second and catches up every fixed step since the last wake (in pieces under `Bridge::step`'s 8-step cap, so none are dropped; at most a minute's worth, as game.js), then sets the menu-bar number and sends the notifications widget.js would have (its texts, toggles, quiet hours and once a minute per kind). Toasts and sounds are dropped; the cues that leave something on the page (an achievement, the wish box closing, zeno, a tree gift) wait for it, and showing the widget resends the whole HUD. With all of those off the game pauses while hidden, as game.js does. Fixed steps make this exact: `a_hidden_catch_up_plays_the_same_game` plays half an hour hidden in batches of 1 s, 5 s and odd sizes against the same game stepped frame by frame, and gets the same save, HUD, shop and notifications.
- **The save**: `save()`'s text goes to `power.json` in the app data directory every 3 s of play (30 s while hidden) and on exit, and is read back with `load()` at launch. Until planet-core's JSON parser has a depth limit, a save over 1 MB or nested more than 32 deep is never parsed (100 KB of `[` would overflow the stack). Each command runs inside `catch_unwind`, so a panic fails that call only, in debug builds (release builds abort on panic).
- **The page**: `hud.js` takes the place of game.js in the widget's page (`scripts/hud.mjs` writes `dist/hud.html`, which the window loads instead of `index.html`). It's `game/rust/planet-web/shim.js` with `invoke` in place of the wasm `Planet`, and without the twin. The time and date come from the page (`Date.now()`, `toDateString()`). page.html reads the menu, angels, ascend, god ki, zeno and card info synchronously, so the page keeps the last copy of each. It refreshes them before applying a message that changed the shop, sent a cue or a new world, at most once a second for a message that only changed the HUD (that's nearly every step), and redraws an open menu after a menu button. Input goes over with HUD.md's pointer ids, buttons and touch; the dragon's offer and instant transmission's picker are cues, as on the site.

### Drawing (stage 5b)

- **`planet::Painter`**: planet-render's `Scene` and `Framebuffer` at the page's `st.W`×`st.H` and `st.scale`, with the view the page has: floating (`BARE`) or window mode, `st.clouds` and `st.cloudAmt` (they change with the pointer, so `hud.js` watches them), then widget.js's rim when floating. `hud.js` reports all of that, reduced motion, where its canvas sits and the page's opacity with `planet_view`; nothing is drawn until it has.
- **Where it draws**: inside the tick, between the step's logic and its timeouts, as `Bridge::step_drawn` does, so the drawing's random draws (particles) and its write-backs (where the hero, the boss and the visitors stand, which taps hit-test) come in game.js's order. A tick that runs several steps (battery saver, or a stall) draws only the last, as page.html draws only the last of its battery-saver steps. The 5a seam (`Renderer::draw(&Game)` called from the layer's own 60 Hz loop) couldn't do either: drawing changes the game, so it has to happen in the step, under the same lock. It's gone, and so is `Frames`.
- **On the page's canvas** (any OS): a frame the page should show adds `"px": true` to the tick's message; `hud.js` fetches it with `planet_pixels` (raw bytes over IPC, not JSON; one request in flight, a newer frame waits for it) and puts it on its canvas. `st.buf` holds it, so widget.js's empty-sky drag test reads the real alpha. Window mode always draws this way: its skies are CSS backgrounds under the canvas (page.html's `.game` per world, the night stars), which a layer under the page would cover.
- **On a layer under the page** (macOS, with `native-renderer` too, and `"native_renderer": "core"` in settings.json): `native::Under` puts a layer-hosting `NSView` under the web view, with one `CALayer` placed exactly over the page's canvas and stretched to it nearest-neighbour, as `image-rendering: pixelated` stretches the canvas, at the page's opacity. Floating, `planet_view` says the layer has it, and the page's canvas stays empty; the web view stays on top, transparent, and takes every press. A frame goes up only when it changed (premultiplied into a `CGImage`, set as `contents` on the main thread with actions off, one in flight). For the drag test, `hud.js` fetches the frame's alpha as one bit a pixel (`planet_mask`) at most ten times a second while the pointer is over the widget.
- **Pictures**: `thumb(id)` draws the hero's idle sprite (`planet_sprite`, from planet-core's sprite data) as game.js does; the share card's `spr` is the hero's current one and its `scene` the page's canvas, or on the layer a copy of the frame fetched as the share button is pressed.
- **Windows and Linux** draw on the page's canvas. A native surface under the HUD there would need a transparent web view over a sibling surface: WebView2 under wry is a child window (windowed hosting), and child windows don't blend with what's under them; getting that means WebView2's composition hosting with DirectComposition, which wry doesn't use, or a second top-level window kept behind the widget. WebKitGTK could sit in a `GtkOverlay` over a drawing area, but that's untested on X11 and Wayland compositors and would need its own measurements. The canvas path costs one raw IPC response and one `putImageData` a frame, with no JavaScript game.

### The pixel proof

`the_painter_draws_what_game_js_draws` (in `planet.rs`, needs node, runs on arm64 macOS: game.js's maths is V8's there) draws 28 scenes with game.js headless (`game/rust/difftest/frame.js` through `scripts/frame.cjs`, which adds window mode: `WIDGET` on, `BARE` off) and with the painter, at the widget's canvas sizes: floating small, medium and large (300×261, 400×356, 560×516 CSS px, the dock's band cut off; 2× at large) and the ring layout's full 400×400; window mode inside its border (398×398, 558×558) and resized freely (640×420). They cover no clouds and every cloud shape at amounts 1-5, the worlds, snake way (the run and the arrival), the arena, shenron, the great ape, a raid, the cooler movie, another hero's form and instant transmission. Every one matches game.js byte for byte, as do planet-render's own widget cases in its framediff. `a_tick_draws_its_last_step` checks the tick: only a tick's last step draws, the message asks for the frame then, `planet_pixels` hands over exactly that frame, and the game and its random draws come out as a game stepped and drawn by hand.

### What it costs (stage 5b)

Release builds (`--no-sign`, bundle id `com.highskore.idlefighterz.agenttest`), medium, default settings (30fps, clouds on hover, notifications and the menu-bar number on), on a Retina display. Each run launches the app, waits 25 s, then reads `proc_pid_rusage` (CPU time, wakeups, energy) of the app and the WebKit processes it started over 60 s. Idle: the pointer away. Hovered: the pointer kept over the widget (so the web view draws at 60fps; the Rust core steps and draws at 30, its fixed step). Hidden: × after 8 s. The builds ran interleaved, four rounds of idle and hovered and two hidden; the machine was busy throughout (load 9-16 on 10 cores, from other work), so expect a few points of noise. "Canvas" is `rust-core` alone (the same binary as the layer, without the setting).

| | CPU, whole widget | energy | wakeups/s | of which the app | of which WebContent |
|---|---|---|---|---|---|
| web view, idle | 12.7% | 190 mW | 339 | 2.4% | 7.7% |
| canvas, idle | 16.3% | 207 mW | 364 | 7.2% | 6.3% |
| layer, idle | 10.8% | 217 mW | 329 | 7.9% | 2.6% |
| web view, hovered | 24.4% | 366 mW | 374 | 2.7% | 15.2% |
| canvas, hovered | 17.9% | 207 mW | 359 | 7.6% | 6.8% |
| layer, hovered | 10.5% | 246 mW | 327 | 7.2% | 2.7% |
| web view, hidden | 0.4% | 1.7 mW | 17 | | |
| canvas, hidden | 0.4% | 2.8 mW | 12 | | |
| layer, hidden | 0.3% | 2.7 mW | 3 | | |

- The layer takes the drawing out of WebKit: WebContent drops from 8-15% to under 3%, and the widget's CPU is the lowest of the three, idle and hovered. Its energy isn't lower yet: the app's tick thread draws on the performance cores, where game.js in WebContent ran cheaper per CPU-second. Moving the tick to the utility QoS class looked like a win (7% and 90 mW) until the frame counters showed why: its sleeps ran long, and it drew 12-14 frames a second instead of 30. That's reverted; the tick now wakes half a step after its step is due, so it draws 30 (15 in battery saver) a second, checked with `IDLE_FIGHTERZ_NATIVE_STATS`.
- The canvas path costs more CPU than the web view idle (a frame copied over IPC and put on the canvas, 30 times a second), and less hovered, where the web view draws twice as often.
- Hidden, all three are near zero; the layer build wakes least.
- Frame time (`IDLE_FIGHTERZ_NATIVE_STATS`, 400×356): the layer's compare, premultiply and `CGImage` 0.3-0.7 ms; the present on the main thread 0.8-1.4 ms.
- Next steps for energy, if wanted: draw only the box that changed (px.wasm already tracks it), and hand the layer an `IOSurface` rather than a new `CGImage` each frame.
