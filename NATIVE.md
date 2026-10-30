# Native renderer

Stage 5 of the Rust port. `planet-core` + `planet-render` (highskore/idle-fighterz, `rust/`) will step the game and draw it into an RGBA framebuffer at the game's internal resolution. This is the shell that shows that framebuffer in the Idle FighterZ window without a web view. Until the core lands, a stand-in source feeds it.

Since 0.2 the shipped widget runs on the Rust core with this layer under its HUD on macOS: see [What ships](#what-ships-02). The stand-in sources below still sit behind a hidden setting.

## Try it

```sh
npm run page
npx tauri build --bundles app
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

Stage 5a and 5b. `planet-core` (`game/rust/planet-core`, a path dependency into the submodule) runs the game in the app, `planet-render` draws it, and the window's page is only its HUD. A default feature since 0.2; `--no-default-features` builds the web-view game.

```sh
npm run page                                   # dist/index.html, then dist/hud.html from it
npx tauri dev                                  # macOS: frames on a layer under the HUD page; elsewhere on its canvas
IDLE_FIGHTERZ_ENGINE=canvas npx tauri dev      # on the HUD page's canvas on macOS too
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
- **On a layer under the page** (macOS, with `native-renderer` too): `native::Under` puts a layer-hosting `NSView` under the web view, with one `CALayer` placed exactly over the page's canvas and stretched to it nearest-neighbour, as `image-rendering: pixelated` stretches the canvas, at the page's opacity. Floating, `planet_view` says the layer has it, and the page's canvas stays empty; the web view stays on top, transparent, and takes every press. A frame goes up only when it changed, from the tick thread itself (no hop to the main thread): into one of three `IOSurface`s the window server isn't showing, only the pixels that differ from the frame that surface last held, then set as the layer's `contents` in a transaction with actions off. See [Energy](#energy-the-layer-on-the-efficiency-cores). For the drag test, `hud.js` fetches the frame's alpha as one bit a pixel (`planet_mask`) at most ten times a second while the pointer is over the widget.
- **Pictures**: `thumb(id)` draws the hero's idle sprite (`planet_sprite`, from planet-core's sprite data) as game.js does; the share card's `spr` is the hero's current one and its `scene` the page's canvas, or on the layer a copy of the frame fetched as the share button is pressed.
- **Windows and Linux** draw on the page's canvas. A native surface under the HUD there would need a transparent web view over a sibling surface: WebView2 under wry is a child window (windowed hosting), and child windows don't blend with what's under them; getting that means WebView2's composition hosting with DirectComposition, which wry doesn't use, or a second top-level window kept behind the widget. WebKitGTK could sit in a `GtkOverlay` over a drawing area, but that's untested on X11 and Wayland compositors and would need its own measurements. The canvas path costs one raw IPC response and one `putImageData` a frame, with no JavaScript game.

### What ships (0.2)

`rust-core` and `native-renderer` are the default features, so `cargo build`, `npm run build` and the release workflow all build the same thing (`native-renderer` compiles to nothing outside macOS). Where the frames go:

| | floating | window mode |
|---|---|---|
| macOS | the layer under the HUD page | the HUD page's canvas |
| Windows, Linux | the HUD page's canvas | the HUD page's canvas |

- **Fallback.** If `native::Under::new` returns an error at setup, the app logs it and runs without a surface: the canvas path, on the same Rust core. If the layer never gets made (no superview or window when the main thread gets to it), `place` says so and the page draws. If a frame can't go up later (no colour space, colour matching or `IOSurface`), the layer is emptied and hidden for good, that tick's message carries `"px": true`, and `hud.js`, seeing `px` while it thinks the layer has the frames, asks `planet_view` again and switches to its canvas. So a failing layer costs at most one frame, never a blank widget. `a_failing_surface_hands_the_frames_to_the_page` checks the tick's side.
- **Logs** (stderr): `engine: …` once at launch, and `frames: on the layer under the page` / `on the page's canvas (window mode)` / `(no layer)` each time that changes, or `frames: the layer failed (…)`. With `IDLE_FIGHTERZ_SMOKE`, `engine: the Rust core` and `hidden: the core plays on` go to its file too.
- **Escape hatches**, read at launch:
  - `IDLE_FIGHTERZ_ENGINE=canvas`: the Rust core on the page's canvas, on macOS too.
  - `IDLE_FIGHTERZ_ENGINE=web`: 0.1's game, game.js in the web view (`dist/index.html`, which the build still makes). The Rust core isn't started.
  - From Finder or at login, set it with `launchctl setenv IDLE_FIGHTERZ_ENGINE web` and quit and reopen the app (`launchctl unsetenv` to undo); on Windows, `setx IDLE_FIGHTERZ_ENGINE web`. It's an environment variable rather than a setting because settings.json keeps only the keys in `DEFAULTS`, and changing any setting would drop it.
- **Trying the fallback**: `IDLE_FIGHTERZ_NATIVE_FAIL=setup` makes `Under::new` fail, `=layer` skips making the layer, and `=show` fails the first frame.

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
- Next steps for energy: done below, in [Energy](#energy-the-layer-on-the-efficiency-cores).

### Energy (the layer on the efficiency cores)

After 5b the layer used less CPU than the web view but no less energy. Four changes in `native::Under`, all macOS-only and only when the layer is on (`planet::Surface`'s `tick_thread` and `sleep` hooks; without a surface the tick runs exactly as before):

- **The tick on the efficiency cores, on time.** The tick thread runs at the utility QoS class. 5b's attempt at that drew 12-14 frames a second because a utility thread's `nanosleep` is coalesced: in a test at load 10 with 2 ms of work every 33 ms, its wakes were p50 47 ms late and 235 of 300 missed by more than half a period. The tick now sleeps on a kqueue timer with `NOTE_CRITICAL` and no leeway: p99 5 ms late, none missed, at half the energy of the default class for the same work. It draws 30.0 frames a second (15.0 in battery saver), checked with `IDLE_FIGHTERZ_NATIVE_STATS`.
- **No `CGImage`, no main thread.** The tick writes the frame into one of three `IOSurface`s the window server isn't showing and sets it as the layer's `contents` itself, in a transaction with actions off. Core Animation takes layer changes from any thread inside an explicit transaction. The present went from 0.8-1.4 ms on the main thread to about 0.1 ms on the tick thread, and the main thread from 4.3% to 1.5% CPU.
- **The same colours.** Core Animation colour-matches a `CGImage` into the window's colour space on the CPU, but an `IOSurface` on the GPU: untagged, the frame comes out in the display's own colours (greener on a P3 screen); tagged sRGB, a third of its colours land a step apart. So the frame goes in already matched, by CG the way the `CGImage` was: each colour is converted once (a 1×1 draw, about 3 µs) and then comes from a cache, and the surfaces are tagged with the window's space, which Core Animation then shows as it is. The window's space follows it to other screens (`NSWindowDidChangeScreen`, `…ScreenProfile`). Checked on screen: 65,536 opaque and translucent colours, and game screenshots, come out byte for byte as the `CGImage` and the web view's canvas showed them. `the_surfaces_hold_what_core_animation_showed` checks the surfaces against CG matching each whole frame, in sRGB and Display P3.
- **Only what changed.** Each surface keeps the frame it last held, and the tick writes only the pixels that differ from it, 32 bytes compared at a time. At medium about 500 of the 142,400 pixels change a frame; the fill and present take about 0.18 ms. The drawing itself still redraws the whole frame: planet-render reports no damage, and a frame is never identical to the last in practice (0 in 600), so skipping identical frames, which the layer still does, rarely helps.

What each step bought (release builds, medium, default settings, measured as in 5b: 60 s windows of `proc_pid_rusage` over the app and the WebKit processes it started, interleaved rounds; another agent's builds kept the load at 7-14 on 10 cores throughout, so the absolute CPU figures run higher than 5b's for every build, the web view's too):

| | runs | CPU, whole widget (median) | energy, mean (median) | wakeups/s | of which the app | of which WebContent |
|---|---|---|---|---|---|---|
| web view, idle | 6 | 21.6% | 123 mW (113) | 344 | 4.0% / 18 mW | 13.0% / 85 mW |
| canvas, idle | 3 | 24.9% | 137 mW (128) | 357 | 10.3% / 73 mW | 8.9% / 42 mW |
| layer, 5b, idle | 5 | 19.7% | 129 mW (123) | 326 | 13.7% / 112 mW | 4.6% / 15 mW |
| + utility QoS and the critical timer | 4 | 18.1% | 143 mW (143) | 324 | 13.1% / 124 mW | 4.2% / 17 mW |
| + IOSurface off the main thread, colour-matched | 4 | 15.0% | 84 mW (83) | 325 | 9.5% / 66 mW | 4.8% / 16 mW |
| **+ only the changed pixels (now), idle** | 6 | 16.0% | **75 mW (68)** | 328 | 9.5% / 58 mW | 5.0% / 15 mW |
| web view, hovered | 6 | 32.6% | 278 mW (256) | 380 | 3.4% / 23 mW | 20.2% / 200 mW |
| canvas, hovered | 3 | 23.7% | 142 mW (150) | 361 | 10.9% / 73 mW | 9.4% / 43 mW |
| layer, 5b, hovered | 5 | 21.3% | 157 mW (138) | 326 | 13.3% / 130 mW | 5.0% / 22 mW |
| + utility QoS and the critical timer | 4 | 18.3% | 152 mW (144) | 320 | 13.4% / 131 mW | 3.5% / 17 mW |
| + IOSurface off the main thread, colour-matched | 4 | 15.2% | 94 mW (88) | 321 | 10.0% / 73 mW | 4.0% / 17 mW |
| **+ only the changed pixels (now), hovered** | 6 | 16.0% | **83 mW (80)** | 325 | 10.1% / 63 mW | 4.7% / 16 mW |
| web view, hidden | 2 | 1.0% | 2 mW | 32 | | |
| layer, 5b, hidden | 2 | 0.4% | 2 mW | 3 | | |
| layer now, hidden | 2 | 0.4% | 2 mW | 3 | | |

- Idle, the layer now takes 39% less energy than the web view (75 against 123 mW) and 42% less than in 5b; hovered, 70% less than the web view (83 against 278 mW, which draws at 60fps there) and 47% less than 5b. It also comes in under the canvas path, idle and hovered. Hidden, nothing changed.

- The step that moved energy is the present: utility QoS alone gained nothing while every frame still went to the main thread on a performance core.
- Wakeups hardly change: the app wakes about 300 times a second in every build, the web view's too. They came from the HUD page's `requestAnimationFrame` loop, which ran with nothing to draw: WebKit drives rAF from a display link in the app process. The page no longer starts it when the app runs the game (idle 343 → 141 wakeups/s, hovered 341 → 173; see [The HUD page at rest](#the-hud-page-at-rest)).

### The HUD page at rest

page.html's frame loop (`tick`) steps and draws game.js and the site's title; on the Rust core the widget's page has neither, so it doesn't start (`game.native`, set by `hud.js`). Nothing else on the page runs while nothing changes: its timers are one-shot and event-driven, and its infinite CSS animations are on elements the widget hides. Measured as above (four interleaved rounds, medium, default settings, load 12-20):

| | wakeups/s | CPU, whole widget (median) | energy, mean (median) | of which the app | of which WebContent |
|---|---|---|---|---|---|
| 0.2.1, idle | 343 | 8.5% | 164 mW (164) | 5.5% / 124 mW | 2.7% / 35 mW |
| no rAF loop, idle | 141 | 5.2% | 109 mW (119) | 4.4% / 96 mW | 0.9% / 12 mW |
| 0.2.1, hovered | 341 | 9.9% | 183 mW (185) | 5.8% / 131 mW | 3.1% / 41 mW |
| no rAF loop, hovered | 173 | 6.9% | 145 mW (145) | 4.7% / 115 mW | 1.4% / 21 mW |

The ~115 wakeups a second left idle are the app's own: the 30 fps tick, its `planet` events and the layer's present.
