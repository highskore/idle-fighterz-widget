# Power Planet (desktop widget)

The homepage's idle planet game as a macOS desktop pet: a frameless, transparent [Tauri v2](https://v2.tauri.app) window with only the planet and its runner, and a menu-bar icon.

```sh
pnpm build:desktop          # from the repo root: widget page, then the release .app
# → desktop/src-tauri/target/release/bundle/macos/Power Planet.app
```

- **Page**: `node planet/build.mjs --widget` writes `desktop/dist/index.html`. It's the same page with `<html class="widget">`, the code fenced by `/*widget*/…/*/widget*/` in `planet/page.html` and `planet/game.js` kept, `planet/widget.css` and `planet/widget.js` filled in, no CDN three.js (only the title used it) and the fonts inlined from `@fontsource`. The site build drops the fenced code, so `public/planet.html` doesn't change.
- **Widget mode**: the game starts at once. There's no title, links or card, and no sky, cloud sea or backdrop in any scene (the planet, trips, Snake Way, HFIL). Solid sprites get a 1px dark rim so they read on any wallpaper. The canvas feathers out toward the window edges, so nothing ends in a hard line. The HUD comes in four layouts, picked in settings: ring (the default, with the ring drawn behind the planet), pill bar, corner chips and top strip. It shows while the pointer is over the window and fades after a few idle seconds, or it can stay up. While nobody interacts with it, the game draws at 15fps instead of 30.
- **Settings** (the gear in the widget, or Settings… in the menu bar): always on top, size, layout, HUD, opacity, launch at login, and quit. Notifications for villain raids, the dragon, trips, space pods and ascend each get their own toggle, plus quiet hours. Settings are saved to `settings.json` in the app's config folder. The × on the widget hides it to the menu bar.
- **Moving it**: press on empty sky and drag, or hold ⌥/⌘ and drag from anywhere. A click on the planet trains as usual.
- **Menu bar**: show/hide, settings, always on top, size (small 240, medium 340, large 480 pt), launch at login, quit. Position and settings are kept between launches. The game isn't saved, same as on the site.
- **Icons**: `npm run icons` renders the pixel-art dragon ball app icon and the menu-bar template icon into `src-tauri/icons/`.

This uses npm, not pnpm, because the repo's pnpm 7.9 fails to fetch from the registry on Node 25 (`ERR_INVALID_THIS`).
The app is unsigned. On another Mac, right-click → Open the first time, or sign and notarise it (see the PR).
