# Releasing Powerrr


The game is a public submodule, so anyone can build the widget from source and the release workflow (`.github/workflows/release.yml`) needs no deploy key.

A notarized build is what makes installing smooth: downloaded from a browser, or through Homebrew, an app that isn't notarized is stopped by Gatekeeper, and people have to right-click → Open (or allow it in System Settings → Privacy & Security). A plain `npm run build` is ad-hoc signed: fine on your own Mac, not for handing out.

## What it does

`scripts/release.sh` builds the universal app (Apple silicon and Intel), signs it with your Developer ID under the hardened runtime, notarizes it with `notarytool`, staples the ticket, and packs `Powerrr_<version>_universal.dmg`, `…universal.zip` and `SHA256SUMS` into `release/`. Without `APPLE_SIGNING_IDENTITY` it makes the same files, ad-hoc signed and not notarized.

No entitlements file: the app isn't sandboxed, the transparent window uses a private AppKit API that needs no entitlement outside the App Store, notifications need none, and WebKit's JIT runs in WebKit's own processes. The signing identity in `tauri.conf.json` is `"-"` (ad-hoc), which `APPLE_SIGNING_IDENTITY` overrides.

## One-time setup (your Apple Developer account)

1. **Developer ID Application certificate.** In Xcode → Settings → Accounts → your team → Manage Certificates → + → Developer ID Application (or at developer.apple.com → Certificates → +, with a CSR from Keychain Access). It lands in your login keychain. Its name is the signing identity:
   ```sh
   security find-identity -v -p codesigning     # "Developer ID Application: Your Name (TEAMID)"
   ```
2. **App Store Connect API key** (preferred for notarizing). appstoreconnect.apple.com → Users and Access → Integrations → App Store Connect API → Team Keys → +, access "Developer". Note the **Key ID** and the **Issuer ID** above the list, and download `AuthKey_<KEYID>.p8` (only once). The alternative is your Apple ID with an app-specific password from account.apple.com → Sign-In and Security → App-Specific Passwords, plus your Team ID.

## Releasing from your Mac

Save the notary credentials in your keychain once, then point the script at the profile:

```sh
xcrun notarytool store-credentials powerrr \
  --key ~/keys/AuthKey_KEYID.p8 --key-id KEYID --issuer ISSUER-UUID
# or: --apple-id you@example.com --team-id TEAMID   (it asks for the app-specific password)

APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)" NOTARY_PROFILE=powerrr \
  scripts/release.sh
```

It checks the result with `spctl`, which should say `source=Notarized Developer ID`. The script also takes `APPLE_API_KEY` (key id), `APPLE_API_ISSUER` and `APPLE_API_KEY_PATH`, or `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`, instead of a profile. `TARGET=aarch64-apple-darwin` builds for Apple silicon only, which is quicker.

## Releasing from GitHub Actions

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

## Homebrew

`packaging/powerrr.rb` is a cask for a tap repo, `highskore/homebrew-tap`, so the install is `brew install --cask highskore/tap/powerrr`. After each release:

```sh
packaging/bump-cask.sh 0.1.0 ~/code/homebrew-tap   # version + sha256 from the release's SHA256SUMS, copied to Casks/
cd ~/code/homebrew-tap && git commit -am "powerrr 0.1.0" && git push
```

## winget and Scoop

`packaging/winget/` holds a manifest set for [winget-pkgs](https://github.com/microsoft/winget-pkgs) (`highskore.Powerrr`, the per-user installer) and `packaging/scoop/powerrr.json` a Scoop manifest for a bucket (it unpacks the same installer; `checkver` and `autoupdate` follow new releases). After a release, fill in the version, date and installer sha256 from `SHA256SUMS`:

```sh
V=0.2.0; SHA=$(curl -fsSL https://github.com/highskore/powerrr/releases/download/desktop-v$V/SHA256SUMS | awk -v f=Powerrr_${V}_x64-setup.exe '$2 == f { print $1 }')
mkdir -p out && for f in packaging/winget/*.yaml; do sed "s/{{VERSION}}/$V/g; s/{{SHA256}}/$SHA/; s/{{DATE}}/$(date +%F)/" "$f" > out/$(basename "$f"); done
winget validate out          # on Windows; then open a PR to winget-pkgs with manifests/h/highskore/Powerrr/$V/
```

For Scoop, set `version` and `hash` in `powerrr.json` the same way and commit it to a bucket repo (e.g. `highskore/scoop-bucket`), so the install is `scoop bucket add highskore https://github.com/highskore/scoop-bucket` then `scoop install powerrr`.

## curl and PowerShell

The installers are served from the highskore.com repo as `https://highskore.com/install.sh` (macOS and Linux) and `https://highskore.com/install.ps1` (Windows):

```sh
curl -fsSL https://highskore.com/install.sh | sh
```

On macOS it fetches the latest release of `highskore/powerrr`, checks the zip against `SHA256SUMS`, copies the app to `/Applications` (or `~/Applications`) and opens it. curl doesn't quarantine what it downloads, so even a build that isn't notarized opens; it says so, and that a browser download of such a build needs right-click → Open. On Linux it checks the AppImage the same way and installs it as `~/.local/bin/powerrr`, with a menu entry and icon. `install.ps1` checks the `-setup.exe` and runs it silently (per user, no admin).

## First release, in order

1. Create the public repo `highskore/homebrew-tap` (empty, with a `Casks/` folder).
2. Add the secrets above to `highskore/powerrr`.
3. Set `version` in `src-tauri/tauri.conf.json` (and `package.json`, `Cargo.toml`), commit, tag `desktop-v<version>`, push the tag. Watch the workflow; download the dmg and check `spctl -a -vv "/Volumes/Powerrr/Powerrr.app"`.
4. Run `packaging/bump-cask.sh <version> <tap checkout>`, push the tap, then try `brew install --cask highskore/tap/powerrr` and the curl line.
