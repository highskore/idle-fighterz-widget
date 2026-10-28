# Powerrr

The [highskore.com](https://highskore.com) idle planet game as a macOS desktop pet: a frameless [Tauri v2](https://v2.tauri.app) window with the planet and its runner floating over the wallpaper, and a menu-bar icon. The name is set once, `productName` in `src-tauri/tauri.conf.json` ("Powerrr"; the UI writes it lowercase), and everything else reads it from there. The bundle id is `com.highskore.powerrr`.

The game is [highskore/planet](https://github.com/highskore/planet), a submodule at `game/`. You need Node, Rust and Xcode's command line tools.

```sh
git clone --recurse-submodules https://github.com/highskore/powerrr.git && cd powerrr
npm ci
npm run build               # widget page, then the release .app (ad-hoc signed)
# → src-tauri/target/release/bundle/macos/Powerrr.app
```

In an existing clone, `git submodule update --init` fetches the game; `git submodule update --remote game` moves it to the game's latest commit (commit the new pointer).

- **Page**: `node game/build.mjs --widget` (`npm run page`) writes `dist/index.html` under the directory it runs from (here the repo root). It's the same page with `<html class="widget">`, the code fenced by `/*widget*/…/*/widget*/` in `game/page.html`, `game/game.js` and `game/world.js` kept, `game/widget.css` and `game/widget.js` filled in, no CDN three.js (only the title used it) and the fonts inlined from `@fontsource` in the local `node_modules`. The site build drops the fenced code, so the homepage doesn't change.
- **Floating** (the default mode): the game starts at once, with no title, links or card, and no sky or backdrop in any scene. Solid sprites get a 1px dark rim so they read on any wallpaper. Nothing fades at the window edges; instead the scene has room: the planet keeps the site's size against the sprites (56–70 px across its radius, as on phones and desktops), the pixel art is 1× at small and medium and 2× at large, and the window is big enough round it for the giant ape, Nappa and the raids. Clouds, if on (always, while the pointer is over the widget, or never), are a finite cloud of whole outlined puffs in one of these shapes: drifting puffs (the default), whose cloud amount slider (1–5, default 2) adds clouds from a ranked list of spots clear of the planet and the hero's path, from three clouds up to a full scattered sea; every cloud is one of seven hand-made cumulus templates (checked by `npm run clouds`, which also draws them all on a contact sheet), mirrored and scaled per spot; or dock clouds (staggered rows along the dock like the site's cloud sea), a bank under the planet, a halo round it, or a plate it rests on.
- **Window mode** (a setting, live): an opaque rounded card with the site's skies, cloud sea and world backdrops, a shadow, and a free resize.
- **HUD**: five layouts. Dock (the default) is one pill under the planet with the power level, the form and a thin bar to the next one, dragon balls and wishes once there are any, and round icon buttons; floating, the game canvas stops above it so nothing runs under it. Also ring, pill bar, corner chips and top strip. It shows while the pointer is over the window and fades after a few idle seconds, or it can stay up. Idle, the game draws at 15fps instead of 30. The ki training tree, angels, achievements and characters scroll down in one column, never sideways.
- **Settings** (the gear in the widget, or Settings… in the menu bar): always on top, size, layout, HUD, mode, clouds, their shape and amount, opacity, the show/hide shortcut, launch at login, and quit. Notifications for villain raids, the dragon, trips, space pods and ascend each get their own toggle, plus quiet hours. Settings are saved to `settings.json` in the app's config folder.
- **No menu bar needed**: on a notched Mac, or behind a menu-bar manager, the icon may be out of sight. Everything works without it: the gear opens settings, × hides the widget, **⌥⌘P** (a global shortcut, changeable or off in settings) shows and hides it, and opening the app again (Spotlight, Finder, `open -a Powerrr`) brings it back. A click on a notification shows the widget too. The menu-bar icon is a template image (a ball with a star cut out, drawn at 36px for 18pt), so macOS tints it for light and dark menu bars.
- **Notifications** go through `UNUserNotificationCenter`. macOS asks once: when a notification toggle is first switched on in settings, or when the first one is due. Settings shows the answer (allowed, not asked yet, blocked) with a button to ask or to open System Settings. None go out in quiet hours, none while the pointer is over the widget (you're looking at it), and each kind at most once a minute. They need the app to run from its `.app` bundle and to be signed, ad-hoc at least; `tauri dev` runs a bare binary and shows none.
- **Hidden**: × hides the widget. With any notification on, the game keeps playing while hidden, in quarter-second steps on a once-a-second timer with no drawing, so raids, trips and the dragon still arrive (about 1% CPU). With all notifications off it pauses.
- **Moving it**: press on empty sky and drag, or hold ⌥/⌘ and drag from anywhere. A click on the planet trains as usual.
- **Menu bar**: show/hide, settings, always on top, size (small 300, medium 400, large 560 pt), launch at login, quit. Position and settings are kept between launches. The game isn't saved, same as on the site.
- **Icons**: `npm run icons` renders the pixel-art dragon ball app icon and the menu-bar template icon into `src-tauri/icons/`.

## Releasing

The game is a public submodule, so anyone can build the widget from source and the release workflow (`.github/workflows/release.yml`) needs no deploy key.

A notarized build is what makes installing smooth: downloaded from a browser, or through Homebrew, an app that isn't notarized is stopped by Gatekeeper, and people have to right-click → Open (or allow it in System Settings → Privacy & Security). A plain `npm run build` is ad-hoc signed: fine on your own Mac, not for handing out.

### What it does

`scripts/release.sh` builds the universal app (Apple silicon and Intel), signs it with your Developer ID under the hardened runtime, notarizes it with `notarytool`, staples the ticket, and packs `Powerrr_<version>_universal.dmg`, `…universal.zip` and `SHA256SUMS` into `release/`. Without `APPLE_SIGNING_IDENTITY` it makes the same files, ad-hoc signed and not notarized.

No entitlements file: the app isn't sandboxed, the transparent window uses a private AppKit API that needs no entitlement outside the App Store, notifications need none, and WebKit's JIT runs in WebKit's own processes. The signing identity in `tauri.conf.json` is `"-"` (ad-hoc), which `APPLE_SIGNING_IDENTITY` overrides.

### One-time setup (your Apple Developer account)

1. **Developer ID Application certificate.** In Xcode → Settings → Accounts → your team → Manage Certificates → + → Developer ID Application (or at developer.apple.com → Certificates → +, with a CSR from Keychain Access). It lands in your login keychain. Its name is the signing identity:
   ```sh
   security find-identity -v -p codesigning     # "Developer ID Application: Your Name (TEAMID)"
   ```
2. **App Store Connect API key** (preferred for notarizing). appstoreconnect.apple.com → Users and Access → Integrations → App Store Connect API → Team Keys → +, access "Developer". Note the **Key ID** and the **Issuer ID** above the list, and download `AuthKey_<KEYID>.p8` (only once). The alternative is your Apple ID with an app-specific password from account.apple.com → Sign-In and Security → App-Specific Passwords, plus your Team ID.

### Releasing from your Mac

Save the notary credentials in your keychain once, then point the script at the profile:

```sh
xcrun notarytool store-credentials powerrr \
  --key ~/keys/AuthKey_KEYID.p8 --key-id KEYID --issuer ISSUER-UUID
# or: --apple-id you@example.com --team-id TEAMID   (it asks for the app-specific password)

APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)" NOTARY_PROFILE=powerrr \
  scripts/release.sh
```

It checks the result with `spctl`, which should say `source=Notarized Developer ID`. The script also takes `APPLE_API_KEY` (key id), `APPLE_API_ISSUER` and `APPLE_API_KEY_PATH`, or `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`, instead of a profile. `TARGET=aarch64-apple-darwin` builds for Apple silicon only, which is quicker.

### Releasing from GitHub Actions

Push a tag like `desktop-v0.1.0` (matching `version` in `tauri.conf.json`). The workflow builds on macOS, signs and notarizes when the secrets are there (ad-hoc signed otherwise, with a note in the release), and creates a GitHub Release with the dmg, zip and `SHA256SUMS`.

Secrets to add yourself (repo → Settings → Secrets and variables → Actions):

- [ ] `APPLE_CERTIFICATE`: the Developer ID Application certificate and its private key as a base64 `.p12`. In Keychain Access → login → My Certificates, right-click "Developer ID Application: …" → Export → `.p12` with a password, then `base64 -i cert.p12 | pbcopy`.
- [ ] `APPLE_CERTIFICATE_PASSWORD`: the password you gave the `.p12`.
- [ ] `APPLE_SIGNING_IDENTITY`: `Developer ID Application: Your Name (TEAMID)`.
- [ ] `KEYCHAIN_PASSWORD`: any random string; it locks the throwaway keychain on the runner.
- [ ] Notarizing, either
  - [ ] `APPLE_API_KEY` (the contents of `AuthKey_<KEYID>.p8`), `APPLE_API_KEY_ID` and `APPLE_API_ISSUER` (preferred), or
  - [ ] `APPLE_ID`, `APPLE_PASSWORD` (app-specific) and `APPLE_TEAM_ID`.

Delete the exported `.p12` and keep the `.p8` somewhere safe once they're in.

### Homebrew

`packaging/powerrr.rb` is a cask for a tap repo, `highskore/homebrew-tap`, so the install is `brew install --cask highskore/tap/powerrr`. After each release:

```sh
packaging/bump-cask.sh 0.1.0 ~/code/homebrew-tap   # version + sha256 from the release's SHA256SUMS, copied to Casks/
cd ~/code/homebrew-tap && git commit -am "powerrr 0.1.0" && git push
```

### curl

The installer is served from the highskore.com repo as `https://highskore.com/install.sh`:

```sh
curl -fsSL https://highskore.com/install.sh | sh
```

It fetches the latest release of `highskore/powerrr`, checks the zip against `SHA256SUMS`, copies the app to `/Applications` (or `~/Applications`) and opens it. curl doesn't quarantine what it downloads, so even a build that isn't notarized opens; it says so, and that a browser download of such a build needs right-click → Open.

### First release, in order

1. Create the public repo `highskore/homebrew-tap` (empty, with a `Casks/` folder).
2. Add the secrets above to `highskore/powerrr`.
3. Set `version` in `src-tauri/tauri.conf.json` (and `package.json`, `Cargo.toml`), commit, tag `desktop-v<version>`, push the tag. Watch the workflow; download the dmg and check `spctl -a -vv "/Volumes/Powerrr/Powerrr.app"`.
4. Run `packaging/bump-cask.sh <version> <tap checkout>`, push the tap, then try `brew install --cask highskore/tap/powerrr` and the curl line.
