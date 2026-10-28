# Powerrr

The [highskore.com](https://highskore.com) idle planet game as a desktop pet for macOS, Windows and Linux: a frameless [Tauri v2](https://v2.tauri.app) window with the planet and its runner floating over the wallpaper, and a menu-bar (tray) icon. The name is set once, `productName` in `src-tauri/tauri.conf.json` ("Powerrr"; the UI writes it lowercase), and everything else reads it from there. The bundle id (and app identifier everywhere) is `com.highskore.powerrr`.

## Install

### macOS (11+, Apple silicon and Intel)

```sh
curl -fsSL https://highskore.com/install.sh | sh
```

or with Homebrew:

```sh
brew install --cask highskore/tap/powerrr
```

Builds aren't notarized yet. The curl install opens straight away; a Homebrew or browser download is stopped by macOS on first launch, so run `xattr -dr com.apple.quarantine /Applications/Powerrr.app` once (or allow it in System Settings → Privacy & Security).

### Windows (10 and 11, x64)

In PowerShell:

```powershell
irm https://highskore.com/install.ps1 | iex
```

It checks the installer against the release's `SHA256SUMS`, installs for your user only (no admin) to `%LOCALAPPDATA%\Powerrr` and starts it. Or download `Powerrr_<version>_x64-setup.exe` (per user) or the `.msi` (per machine) from the [latest release](https://github.com/highskore/powerrr/releases/latest). Uninstall from Settings → Apps.

**SmartScreen:** builds aren't code-signed yet, so a browser download of the installer gets "Windows protected your PC". Click **More info → Run anyway**, once. The PowerShell install isn't stopped: files it downloads carry no mark of the web.

### Linux (x86_64)

```sh
curl -fsSL https://highskore.com/install.sh | sh
```

It installs the AppImage as `~/.local/bin/powerrr` with a menu entry and starts it. AppImages need FUSE 2 (`sudo apt install libfuse2`, `libfuse2t64` on Ubuntu 24.04). On Debian and Ubuntu you can instead take the `.deb` from the [latest release](https://github.com/highskore/powerrr/releases/latest): `sudo apt install ./Powerrr_<version>_amd64.deb`.

- **Floating needs a compositor.** Every Wayland desktop has one, as do GNOME, KDE and Xfce on X11 with compositing on. On a bare X11 window manager the transparent window would come out black, so the widget sees that and stays in window mode (floating is greyed out in settings).
- **The tray icon** needs AppIndicator support: built into KDE, Xfce, Cinnamon and Ubuntu's GNOME; on stock GNOME install the "AppIndicator and KStatusNotifierItem Support" extension. Without it the widget's gear and × do everything the menu does. The power level shows as a label beside the icon where the panel supports it.
- **The show/hide shortcut** (Ctrl+Alt+P) is an X11 key grab: it works on X11, but on Wayland only while an X11 (XWayland) window has focus. Set a shortcut in your desktop's settings that runs `powerrr` instead: a second launch shows the running widget.

## Build from source

The game is [highskore/planet](https://github.com/highskore/planet), a submodule at `game/`. You need Node and Rust, plus Xcode's command line tools on macOS, the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) on Windows (MSVC build tools; WebView2 ships with Windows 10 and 11), and on Debian/Ubuntu `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev patchelf`.

```sh
git clone --recurse-submodules https://github.com/highskore/powerrr.git && cd powerrr
npm ci
npm run build               # widget page, then the release bundles for this OS
# macOS   → src-tauri/target/release/bundle/macos/Powerrr.app (ad-hoc signed)
# Windows → src-tauri/target/release/bundle/nsis/*-setup.exe and bundle/msi/*.msi
# Linux   → src-tauri/target/release/bundle/appimage/*.AppImage and bundle/deb/*.deb
```

On Windows, clone with `git -c core.autocrlf=false clone …`: the page build matches the game's sources on LF line endings. Which bundles each OS makes is set in `src-tauri/tauri.windows.conf.json` and `tauri.linux.conf.json`, which tauri merges over `tauri.conf.json`.

In an existing clone, `git submodule update --init` fetches the game; `git submodule update --remote game` moves it to the game's latest commit (commit the new pointer).

- **Page**: `node game/build.mjs --widget` (`npm run page`) writes `dist/index.html` under the directory it runs from (here the repo root). It's the same page with `<html class="widget">`, the code fenced by `/*widget*/…/*/widget*/` in `game/page.html`, `game/game.js` and `game/world.js` kept, `game/widget.css` and `game/widget.js` filled in, no CDN three.js (only the title used it) and the fonts inlined from `@fontsource` in the local `node_modules`. The site build drops the fenced code, so the homepage doesn't change.
- **Floating** (the default mode): the game starts at once, with no title, links or card, and no sky or backdrop in any scene. Solid sprites get a 1px dark rim so they read on any wallpaper. Nothing fades at the window edges; instead the scene has room: the planet keeps the site's size against the sprites (56–70 px across its radius, as on phones and desktops), the pixel art is 1× at small and medium and 2× at large, and the window is big enough round it for the giant ape, Nappa and the raids. Clouds, if on (always, while the pointer is over the widget, or never), are a finite cloud of whole outlined puffs in one of these shapes: drifting puffs (the default), whose cloud amount slider (1–5, default 2) adds clouds from a ranked list of spots clear of the planet and the hero's path, from three clouds up to a full scattered sea; every cloud is one of seven hand-made cumulus templates (checked by `npm run clouds`, which also draws them all on a contact sheet), mirrored and scaled per spot; or dock clouds (staggered rows along the dock like the site's cloud sea), a bank under the planet, a halo round it, or a plate it rests on.
- **Window mode** (a setting, live): an opaque rounded card with the site's skies, cloud sea and world backdrops, a shadow, and a free resize.
- **HUD**: five layouts. Dock (the default) is one pill under the planet with the power level, the form and a thin bar to the next one, dragon balls and wishes once there are any, and round icon buttons; floating, the game canvas stops above it so nothing runs under it. Also ring, pill bar, corner chips and top strip. It shows while the pointer is over the window and fades after a few idle seconds, or it can stay up. Idle, the game draws at 15fps instead of 30. The ki training tree, angels, achievements and characters scroll down in one column, never sideways.
- **Settings** (the gear in the widget, or Settings… in the menu bar): always on top, size, layout, HUD, mode, clouds, their shape and amount, opacity, the show/hide shortcut, the power level in the menu bar, launch at login, and quit. Notifications for villain raids, the dragon, trips, space pods and ascend each get their own toggle, plus quiet hours. Settings are saved to `settings.json` in the app's config folder: `~/Library/Application Support/com.highskore.powerrr/` on macOS, `%APPDATA%\com.highskore.powerrr\` on Windows, `~/.config/com.highskore.powerrr/` on Linux.
- **Windows and Linux**: the same widget, settings and menu, from a tray icon (the dragon ball: the black scouter is a macOS template image and would vanish on a dark taskbar). The shortcut is **Ctrl+Alt+P**, and settings spell keys out (Ctrl+Alt+P) where macOS shows ⌥⌘P. The power level goes in the tray icon's tooltip on Windows, which has no tray titles, and in an AppIndicator label on Linux. Notifications go through the system's own (Windows toasts, the freedesktop notification service), which ask no permission up front. Launch at login is a `Run` registry entry on Windows and `~/.config/autostart/` on Linux (pointing at the AppImage itself, not its temporary mount). Opening the app again shows the running widget instead of starting a second one. Hover: Windows and X11 report the pointer as on macOS; Wayland tells apps neither where the pointer nor where their window is, so there the widget relies on the pointer events its page gets anyway.
- **No menu bar needed**: on a notched Mac, or behind a menu-bar manager, the icon may be out of sight. Everything works without it: the gear opens settings, × hides the widget, **⌥⌘P** (a global shortcut, changeable or off in settings) shows and hides it, and opening the app again (Spotlight, Finder, `open -a Powerrr`) brings it back. A click on a notification shows the widget too. The menu-bar icon is a template image (a pixel-art scouter, 18×18 doubled to 36px for 18pt), so macOS tints it for light and dark menu bars.
- **Notifications** go through `UNUserNotificationCenter`. macOS asks once: when a notification toggle is first switched on in settings, or when the first one is due. Settings shows the answer (allowed, not asked yet, blocked) with a button to ask or to open System Settings. None go out in quiet hours, none while the pointer is over the widget (you're looking at it), and each kind at most once a minute. They need the app to run from its `.app` bundle and to be signed, ad-hoc at least; `tauri dev` runs a bare binary and shows none.
- **Hidden**: × hides the widget. With any notification on, or the power level in the menu bar, the game keeps playing while hidden, in quarter-second steps on a once-a-second timer with no drawing, so raids, trips and the dragon still arrive (about 1% CPU). With all of those off it pauses.
- **Moving it**: press on empty sky and drag, or hold ⌥/⌘ and drag from anywhere. A click on the planet trains as usual.
- **Menu bar**: the scouter icon with your power level beside it (1.2K, 3.4M, 5.6B, then 7.8e12), updated about once a second and only when the number changes; a setting turns the number off and leaves the icon. Its menu: show/hide, settings, always on top, size (small 300, medium 400, large 560 pt), launch at login, quit. Position and settings are kept between launches. The game isn't saved, same as on the site.
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

Push a tag like `desktop-v0.1.0` (matching `version` in `tauri.conf.json`). The workflow builds on three runners: `macos-14` (the universal app; signed and notarized when the secrets are there, ad-hoc signed otherwise), `windows-latest` (`Powerrr_<version>_x64-setup.exe` and `Powerrr_<version>_x64_en-US.msi`, Authenticode-signed when the Windows secrets are there) and `ubuntu-22.04` (`Powerrr_<version>_amd64.AppImage` and `.deb`). A last job puts all of them and one combined `SHA256SUMS` in a GitHub Release, with a note for whatever went out unsigned.

Every pull request (and a manual run from the Actions tab) builds all three, unsigned, and smoke-tests two: on Windows it runs the installer silently, starts the app and checks a second launch hands over and exits; on Linux it installs the `.deb` and runs the AppImage under Xvfb, which has no compositor, so it checks the fallback to window mode too. The packages are attached to the run as artifacts. Signing secrets are only read for `desktop-v*` tags.

Secrets to add yourself (repo → Settings → Secrets and variables → Actions):

- [ ] `APPLE_CERTIFICATE`: the Developer ID Application certificate and its private key as a base64 `.p12`. In Keychain Access → login → My Certificates, right-click "Developer ID Application: …" → Export → `.p12` with a password, then `base64 -i cert.p12 | pbcopy`.
- [ ] `APPLE_CERTIFICATE_PASSWORD`: the password you gave the `.p12`.
- [ ] `APPLE_SIGNING_IDENTITY`: `Developer ID Application: Your Name (TEAMID)`.
- [ ] `KEYCHAIN_PASSWORD`: any random string; it locks the throwaway keychain on the runner.
- [ ] Notarizing, either
  - [ ] `APPLE_API_KEY` (the contents of `AuthKey_<KEYID>.p8`), `APPLE_API_KEY_ID` and `APPLE_API_ISSUER` (preferred), or
  - [ ] `APPLE_ID`, `APPLE_PASSWORD` (app-specific) and `APPLE_TEAM_ID`.

Delete the exported `.p12` and keep the `.p8` somewhere safe once they're in.

Windows code signing (optional: without them the Windows packages are unsigned and SmartScreen warns, see Install):

- [ ] `WINDOWS_CERTIFICATE`: an Authenticode code-signing certificate and its private key as a base64 `.pfx` (`base64 -i cert.pfx | pbcopy`, or `[Convert]::ToBase64String([IO.File]::ReadAllBytes('cert.pfx'))` in PowerShell).
- [ ] `WINDOWS_CERTIFICATE_PASSWORD`: the `.pfx` password (leave unset if it has none).

The workflow imports it into the runner's certificate store and tauri signs the app, the installer and the msi by thumbprint, timestamped. A certificate kept on a hardware token or in a cloud HSM (the norm for new certificates) needs a `signCommand` instead; see [tauri's Windows signing guide](https://v2.tauri.app/distribute/sign/windows/).

### Homebrew

`packaging/powerrr.rb` is a cask for a tap repo, `highskore/homebrew-tap`, so the install is `brew install --cask highskore/tap/powerrr`. After each release:

```sh
packaging/bump-cask.sh 0.1.0 ~/code/homebrew-tap   # version + sha256 from the release's SHA256SUMS, copied to Casks/
cd ~/code/homebrew-tap && git commit -am "powerrr 0.1.0" && git push
```

### winget and Scoop

`packaging/winget/` holds a manifest set for [winget-pkgs](https://github.com/microsoft/winget-pkgs) (`highskore.Powerrr`, the per-user installer) and `packaging/scoop/powerrr.json` a Scoop manifest for a bucket (it unpacks the same installer; `checkver` and `autoupdate` follow new releases). After a release, fill in the version, date and installer sha256 from `SHA256SUMS`:

```sh
V=0.2.0; SHA=$(curl -fsSL https://github.com/highskore/powerrr/releases/download/desktop-v$V/SHA256SUMS | awk -v f=Powerrr_${V}_x64-setup.exe '$2 == f { print $1 }')
mkdir -p out && for f in packaging/winget/*.yaml; do sed "s/{{VERSION}}/$V/g; s/{{SHA256}}/$SHA/; s/{{DATE}}/$(date +%F)/" "$f" > out/$(basename "$f"); done
winget validate out          # on Windows; then open a PR to winget-pkgs with manifests/h/highskore/Powerrr/$V/
```

For Scoop, set `version` and `hash` in `powerrr.json` the same way and commit it to a bucket repo (e.g. `highskore/scoop-bucket`), so the install is `scoop bucket add highskore https://github.com/highskore/scoop-bucket` then `scoop install powerrr`.

### curl and PowerShell

The installers are served from the highskore.com repo as `https://highskore.com/install.sh` (macOS and Linux) and `https://highskore.com/install.ps1` (Windows):

```sh
curl -fsSL https://highskore.com/install.sh | sh
```

On macOS it fetches the latest release of `highskore/powerrr`, checks the zip against `SHA256SUMS`, copies the app to `/Applications` (or `~/Applications`) and opens it. curl doesn't quarantine what it downloads, so even a build that isn't notarized opens; it says so, and that a browser download of such a build needs right-click → Open. On Linux it checks the AppImage the same way and installs it as `~/.local/bin/powerrr`, with a menu entry and icon. `install.ps1` checks the `-setup.exe` and runs it silently (per user, no admin).

### First release, in order

1. Create the public repo `highskore/homebrew-tap` (empty, with a `Casks/` folder).
2. Add the secrets above to `highskore/powerrr`.
3. Set `version` in `src-tauri/tauri.conf.json` (and `package.json`, `Cargo.toml`), commit, tag `desktop-v<version>`, push the tag. Watch the workflow; download the dmg and check `spctl -a -vv "/Volumes/Powerrr/Powerrr.app"`.
4. Run `packaging/bump-cask.sh <version> <tap checkout>`, push the tap, then try `brew install --cask highskore/tap/powerrr` and the curl line.
