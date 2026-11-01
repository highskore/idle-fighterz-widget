# The Rust core and the native renderer

The widget runs the game on `planet-core` and draws it with `planet-render` (highskore/idle-fighterz, `game/rust/`, path dependencies into the submodule). The window's page is only the HUD. On macOS, floating, the frames go into a `CALayer` under that page; everywhere else they go onto the page's canvas.

Two cargo features, both on by default, so `cargo build`, `npm run build` and the release workflow all build the same thing:

- **`rust-core`**: the game in the app, behind a HUD-only page. Without it, game.js runs the game in the web view (0.1's widget, `dist/index.html`, which the build still makes).
- **`native-renderer`**: macOS only, compiles to nothing elsewhere. With `rust-core`, the layer under the HUD page (`src-tauri/src/native/`); it also builds the spectacles' overlay layers.

```sh
npm run page                                   # dist/index.html, then dist/hud.html from it
npx tauri dev                                  # macOS: frames on a layer under the HUD page; elsewhere on its canvas
IDLE_FIGHTERZ_ENGINE=canvas npx tauri dev      # on the HUD page's canvas on macOS too
IDLE_FIGHTERZ_NATIVE_STATS=1 npx tauri dev     # the layer's frame-time counters on stderr every 5 s
```

## Where the frames go

| | floating | window mode |
|---|---|---|
| macOS | the layer under the HUD page | the HUD page's canvas |
| Windows, Linux | the HUD page's canvas | the HUD page's canvas |

- **Fallback.** If `native::Under::new` returns an error at setup, the app logs it and runs without a surface: the canvas path, on the same Rust core. If the layer never gets made (no superview or window when the main thread gets to it), `place` says so and the page draws. If a frame can't go up later (no colour space, colour matching or `IOSurface`), the layer is emptied and hidden for good, that tick's message carries `"px": true`, and `hud.js`, seeing `px` while it thinks the layer has the frames, asks `planet_view` again and switches to its canvas. So a failing layer costs at most one frame, never a blank widget. `a_failing_surface_hands_the_frames_to_the_page` checks the tick's side.
- **Logs** (stderr): `engine: …` once at launch, and `frames: on the layer under the page` / `on the page's canvas (window mode)` / `(no layer)` each time that changes, or `frames: the layer failed (…)`. With `IDLE_FIGHTERZ_SMOKE`, `engine: the Rust core` and `hidden: the core plays on` go to its file too.
- **Escape hatches**, read at launch:
  - `IDLE_FIGHTERZ_ENGINE=canvas`: the Rust core on the page's canvas, on macOS too.
  - `IDLE_FIGHTERZ_ENGINE=web`: game.js in the web view (`dist/index.html`). The Rust core isn't started.
  - From Finder or at login, set it with `launchctl setenv IDLE_FIGHTERZ_ENGINE web` and quit and reopen the app (`launchctl unsetenv` to undo); on Windows, `setx IDLE_FIGHTERZ_ENGINE web`. It's an environment variable rather than a setting because settings.json keeps only the keys in `DEFAULTS`, and changing any setting would drop it.
- **Trying the fallback**: `IDLE_FIGHTERZ_NATIVE_FAIL=setup` makes `Under::new` fail, `=layer` skips making the layer, and `=show` fails the first frame.

## The core (`src-tauri/src/planet.rs`)

- **Commands**: `Core` holds `planet_core::bridge::Bridge` in a `Mutex`. Each row of `game/rust/HUD.md`'s calls table is a `planet_<call>` command, with HUD.md's argument names (`now_ms`, `bx`, …) and its results (numbers, bools, JSON strings). Not included: `new` (the app makes the game, which outlives the page) and `spots`/`probe` (the web build's lockstep twin).
- **The tick**: a thread steps the game at the rate widget.js picks for the page (the frame-rate setting, doubled while it's awake: hover, focus or an open panel; `hud.js` passes it on with `planet_fps`), at most 30 times a second, as the game's fixed step is 1/30 s. It sends each non-empty `frame()` message to the main window as a `planet` event. Nothing else calls `frame()` while the widget shows, so each delta reaches the page exactly once.
- **Hidden**: the page gets nothing. If a notification or the menu-bar power level is on, the thread wakes once a second and catches up every fixed step since the last wake (in pieces under `Bridge::step`'s 8-step cap, so none are dropped; at most a minute's worth, as game.js), then sets the menu-bar number and sends the notifications widget.js would have (its texts, toggles, quiet hours and once a minute per kind). Toasts and sounds are dropped; the cues that leave something on the page (an achievement, the wish box closing, zeno, a tree gift) wait for it, and showing the widget resends the whole HUD. With all of those off the game pauses while hidden, as game.js does. Fixed steps make this exact: `a_hidden_catch_up_plays_the_same_game` plays half an hour hidden in batches of 1 s, 5 s and odd sizes against the same game stepped frame by frame, and gets the same save, HUD, shop and notifications.
- **The save**: `save()`'s text goes to `power.json` in the app data directory every 3 s of play (30 s while hidden) and on exit, and is read back with `load()` at launch. Until planet-core's JSON parser has a depth limit, a save over 1 MB or nested more than 32 deep is never parsed (100 KB of `[` would overflow the stack). Each command runs inside `catch_unwind`, so a panic fails that call only, in debug builds (release builds abort on panic).

## The HUD page

The HUD is the dock pill and the other four layouts, the upgrade and ki panels, achievements, characters and the settings window: text in three web fonts, scrolling lists, hover states, transitions. It stays a web page, transparent, over the frames: one HUD for the site and the widget.

- `hud.js` takes the place of game.js in the widget's page (`scripts/hud.mjs` writes `dist/hud.html`, which the window loads instead of `index.html`). It's `game/rust/planet-web/shim.js` with `invoke` in place of the wasm `Planet`, and without the twin. The time and date come from the page (`Date.now()`, `toDateString()`). page.html reads the menu, angels, ascend, god ki, zeno and card info synchronously, so the page keeps the last copy of each. It refreshes them before applying a message that changed the shop, sent a cue or a new world, at most once a second for a message that only changed the HUD (that's nearly every step), and redraws an open menu after a menu button. Input goes over with HUD.md's pointer ids, buttons and touch; the dragon's offer and instant transmission's picker are cues, as on the site.
- **At rest**: page.html's frame loop (`tick`) steps and draws game.js and the site's title; on the Rust core the widget's page has neither, so it doesn't start (`game.native`, set by `hud.js`). Nothing else on the page runs while nothing changes: its timers are one-shot and event-driven, and its infinite CSS animations are on elements the widget hides.
- A native-drawn HUD (the pill and panels in the framebuffer, or AppKit views over the layer) would drop the WebKit processes altogether, but means re-making every panel, the scroll lists, text layout and the settings window, and keeping two HUDs in step while the site uses the HTML one. The measurements below show the web view's idle cost is small once it isn't drawing the game.

## Drawing

- **`planet::Painter`**: planet-render's `Scene` and `Framebuffer` at the page's `st.W`×`st.H` and `st.scale`, with the view the page has: floating (`BARE`) or window mode, `st.clouds` and `st.cloudAmt` (they change with the pointer, so `hud.js` watches them), then widget.js's rim when floating. `hud.js` reports all of that, reduced motion, where its canvas sits and the page's opacity with `planet_view`; nothing is drawn until it has.
- **Where it draws**: inside the tick, between the step's logic and its timeouts, as `Bridge::step_drawn` does, so the drawing's random draws (particles) and its write-backs (where the hero, the boss and the visitors stand, which taps hit-test) come in game.js's order. A tick that runs several steps (battery saver, or a stall) draws only the last, as page.html draws only the last of its battery-saver steps. Drawing changes the game, so it happens in the step, under the same lock.
- **On the page's canvas** (any OS): a frame the page should show adds `"px": true` to the tick's message; `hud.js` fetches it with `planet_pixels` (raw bytes over IPC, not JSON; one request in flight, a newer frame waits for it) and puts it on its canvas. `st.buf` holds it, so widget.js's empty-sky drag test reads the real alpha. Window mode always draws this way: its skies are CSS backgrounds under the canvas (page.html's `.game` per world, the night stars), which a layer under the page would cover.
- **On a layer under the page** (macOS): `native::Under` puts a layer-hosting `NSView` under the web view, with one `CALayer` placed exactly over the page's canvas and stretched to it nearest-neighbour, as `image-rendering: pixelated` stretches the canvas, at the page's opacity. Floating, `planet_view` says the layer has it, and the page's canvas stays empty; the web view stays on top, transparent, and takes every press. A frame goes up only when it changed, from the tick thread itself (see [Energy](#energy)). For the drag test, `hud.js` fetches the frame's alpha as one bit a pixel (`planet_mask`) at most ten times a second while the pointer is over the widget.
- **Pictures**: `thumb(id)` draws the hero's idle sprite (`planet_sprite`, from planet-core's sprite data) as game.js does; the share card's `spr` is the hero's current one and its `scene` the page's canvas, or on the layer a copy of the frame fetched as the share button is pressed.
- **Windows and Linux** draw on the page's canvas. A native surface under the HUD there would need a transparent web view over a sibling surface: WebView2 under wry is a child window (windowed hosting), and child windows don't blend with what's under them; getting that means WebView2's composition hosting with DirectComposition, which wry doesn't use, or a second top-level window kept behind the widget. WebKitGTK could sit in a `GtkOverlay` over a drawing area, but that's untested on X11 and Wayland compositors and would need its own measurements. The canvas path costs one raw IPC response and one `putImageData` a frame, with no JavaScript game.

### Why a bare CALayer, not wgpu or softbuffer

- **softbuffer** drops alpha on macOS: its CoreGraphics backend makes its images `kCGImageAlphaNoneSkipFirst`, so the planet would sit on a black square. It would also scale on the CPU.
- **wgpu** works (a Metal surface with `CompositeAlphaMode::PostMultiplied`), but it brings about a hundred crates, a Metal device and a render pipeline to do one nearest-neighbour blit of a small texture, which is what a `CALayer` with a nearest filter already is. It's the better choice once the renderer itself moves to the GPU, or for Windows and Linux.
- **CALayer**: the `objc2` crates Idle FighterZ already depends on (a few more of the same family, all already in the lockfile through Tauri), and no work for the window server when the frame doesn't change.

### The pixel proof

`the_painter_draws_what_game_js_draws` (in `planet.rs`, needs node, runs on arm64 macOS: game.js's maths is V8's there) draws 28 scenes with game.js headless (`game/rust/difftest/frame.js` through `scripts/frame.cjs`, which adds window mode: `WIDGET` on, `BARE` off) and with the painter, at the widget's canvas sizes: floating small, medium and large (300×261, 400×356, 560×516 CSS px, the dock's band cut off; 2× at large) and the ring layout's full 400×400; window mode inside its border (398×398, 558×558) and resized freely (640×420). They cover no clouds and every cloud shape at amounts 1-5, the worlds, snake way (the run and the arrival), the arena, shenron, the great ape, a raid, the cooler movie, another hero's form and instant transmission. Every one matches game.js byte for byte, as do planet-render's own widget cases in its framediff. `a_tick_draws_its_last_step` checks the tick: only a tick's last step draws, the message asks for the frame then, `planet_pixels` hands over exactly that frame, and the game and its random draws come out as a game stepped and drawn by hand.

## Energy

The layer's work is all in `native::Under`, macOS-only and only when the layer is on (`planet::Surface`'s `tick_thread` and `sleep` hooks; without a surface the tick takes a plain sleep at the default QoS):

- **The tick on the efficiency cores, on time.** The tick thread runs at the utility QoS class. A utility thread's `nanosleep` is coalesced: in a test at load 10 with 2 ms of work every 33 ms, its wakes were p50 47 ms late and 235 of 300 missed by more than half a period, so it drew 12-14 frames a second. The tick sleeps on a kqueue timer with `NOTE_CRITICAL` and no leeway instead: p99 5 ms late, none missed, at half the energy of the default class for the same work. It draws 30.0 frames a second (15.0 in battery saver), checked with `IDLE_FIGHTERZ_NATIVE_STATS`.
- **No `CGImage`, no main thread.** The tick writes the frame into one of three `IOSurface`s the window server isn't showing and sets it as the layer's `contents` itself, in a transaction with actions off. Core Animation takes layer changes from any thread inside an explicit transaction. The present takes about 0.1 ms on the tick thread, against 0.8-1.4 ms on the main thread for a `CGImage`.
- **The same colours.** Core Animation colour-matches a `CGImage` into the window's colour space on the CPU, but an `IOSurface` on the GPU: untagged, the frame comes out in the display's own colours (greener on a P3 screen); tagged sRGB, a third of its colours land a step apart. So the frame goes in already matched, by CG the way a `CGImage` would be: each colour is converted once (a 1×1 draw, about 3 µs) and then comes from a cache, and the surfaces are tagged with the window's space, which Core Animation then shows as it is. The window's space follows it to other screens (`NSWindowDidChangeScreen`, `…ScreenProfile`). Checked on screen: 65,536 opaque and translucent colours, and game screenshots, come out byte for byte as a `CGImage` and the web view's canvas showed them. `the_surfaces_hold_what_core_animation_showed` checks the surfaces against CG matching each whole frame, in sRGB and Display P3.
- **Only what changed.** Each surface keeps the frame it last held, and the tick writes only the pixels that differ from it, 32 bytes compared at a time. At medium about 500 of the 142,400 pixels change a frame; the fill and present take about 0.18 ms. The drawing itself still redraws the whole frame: planet-render reports no damage, and a frame is never identical to the last in practice (0 in 600), so skipping identical frames, which the layer still does, rarely helps.

### Measurements

Release builds (`--no-sign`, bundle id `com.highskore.idlefighterz.agenttest`), medium, default settings (30fps, clouds on hover, notifications and the menu-bar number on), on a Retina display. Each run launches the app, waits 25 s, then reads `proc_pid_rusage` (CPU time, wakeups, energy) of the app and the WebKit processes it started over 60 s. Idle: the pointer away. Hovered: the pointer kept over the widget (the web view draws at 60fps there; the Rust core steps and draws at 30, its fixed step). Hidden: × after 8 s. Interleaved rounds; another agent's builds kept the load at 7-14 on 10 cores throughout, so the absolute CPU figures run high. "Web view" is game.js in the web view, "canvas" the Rust core on the page's canvas, "layer" the first version of the layer (a `CGImage` presented on the main thread), and each row after it adds one of the changes above.

| | runs | CPU, whole widget (median) | energy, mean (median) | wakeups/s | of which the app | of which WebContent |
|---|---|---|---|---|---|---|
| web view, idle | 6 | 21.6% | 123 mW (113) | 344 | 4.0% / 18 mW | 13.0% / 85 mW |
| canvas, idle | 3 | 24.9% | 137 mW (128) | 357 | 10.3% / 73 mW | 8.9% / 42 mW |
| layer, idle | 5 | 19.7% | 129 mW (123) | 326 | 13.7% / 112 mW | 4.6% / 15 mW |
| + utility QoS and the critical timer | 4 | 18.1% | 143 mW (143) | 324 | 13.1% / 124 mW | 4.2% / 17 mW |
| + IOSurface off the main thread, colour-matched | 4 | 15.0% | 84 mW (83) | 325 | 9.5% / 66 mW | 4.8% / 16 mW |
| **+ only the changed pixels, idle** | 6 | 16.0% | **75 mW (68)** | 328 | 9.5% / 58 mW | 5.0% / 15 mW |
| web view, hovered | 6 | 32.6% | 278 mW (256) | 380 | 3.4% / 23 mW | 20.2% / 200 mW |
| canvas, hovered | 3 | 23.7% | 142 mW (150) | 361 | 10.9% / 73 mW | 9.4% / 43 mW |
| layer, hovered | 5 | 21.3% | 157 mW (138) | 326 | 13.3% / 130 mW | 5.0% / 22 mW |
| + utility QoS and the critical timer | 4 | 18.3% | 152 mW (144) | 320 | 13.4% / 131 mW | 3.5% / 17 mW |
| + IOSurface off the main thread, colour-matched | 4 | 15.2% | 94 mW (88) | 321 | 10.0% / 73 mW | 4.0% / 17 mW |
| **+ only the changed pixels, hovered** | 6 | 16.0% | **83 mW (80)** | 325 | 10.1% / 63 mW | 4.7% / 16 mW |
| web view, hidden | 2 | 1.0% | 2 mW | 32 | | |
| layer, hidden | 2 | 0.4% | 2 mW | 3 | | |

- Idle, the layer takes 39% less energy than the web view (75 against 123 mW); hovered, 70% less (83 against 278 mW, where the web view draws at 60fps). It also comes in under the canvas path, idle and hovered. Hidden, all are near zero.
- The step that moved energy is the present: utility QoS alone gained nothing while every frame still went to the main thread on a performance core.
- Wakeups came from the HUD page's `requestAnimationFrame` loop, which ran with nothing to draw: WebKit drives rAF from a display link in the app process. With the page's frame loop off on the Rust core (see [The HUD page](#the-hud-page)), measured the same way (four interleaved rounds, load 12-20):

| | wakeups/s | CPU, whole widget (median) | energy, mean (median) | of which the app | of which WebContent |
|---|---|---|---|---|---|
| rAF loop running, idle | 343 | 8.5% | 164 mW (164) | 5.5% / 124 mW | 2.7% / 35 mW |
| no rAF loop, idle | 141 | 5.2% | 109 mW (119) | 4.4% / 96 mW | 0.9% / 12 mW |
| rAF loop running, hovered | 341 | 9.9% | 183 mW (185) | 5.8% / 131 mW | 3.1% / 41 mW |
| no rAF loop, hovered | 173 | 6.9% | 145 mW (145) | 4.7% / 115 mW | 1.4% / 21 mW |

The ~115 wakeups a second left idle are the app's own: the 30 fps tick, its `planet` events and the layer's present.
