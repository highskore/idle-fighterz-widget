# Releasing Idle FighterZ


The game is a public submodule, so anyone can build the widget from source and the release workflow (`.github/workflows/release.yml`) needs no deploy key.

A notarized build is what makes installing smooth: downloaded from a browser, or through Homebrew, an app that isn't notarized is stopped by Gatekeeper, and people have to right-click → Open (or allow it in System Settings → Privacy & Security). A plain `npm run build` is ad-hoc signed: fine on your own Mac, not for handing out.

## What it does

`scripts/release.sh` builds the universal app (Apple silicon and Intel), signs it with your Developer ID under the hardened runtime, notarizes it with `notarytool`, staples the ticket, and packs `IdleFighterZ_<version>_universal.dmg`, `…universal.zip` and `SHA256SUMS` into `release/`. Without `APPLE_SIGNING_IDENTITY` it makes the same files, ad-hoc signed and not notarized.

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
xcrun notarytool store-credentials idle-fighterz \
  --key ~/keys/AuthKey_KEYID.p8 --key-id KEYID --issuer ISSUER-UUID
# or: --apple-id you@example.com --team-id TEAMID   (it asks for the app-specific password)

APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)" NOTARY_PROFILE=idle-fighterz \
  scripts/release.sh
```

It checks the result with `spctl`, which should say `source=Notarized Developer ID`. The script also takes `APPLE_API_KEY` (key id), `APPLE_API_ISSUER` and `APPLE_API_KEY_PATH`, or `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`, instead of a profile. `TARGET=aarch64-apple-darwin` builds for Apple silicon only, which is quicker.

## Releasing from GitHub Actions

Push a tag like `desktop-v0.1.0` (matching `version` in `tauri.conf.json`). The workflow builds on three runners: `macos-14` (the universal app; signed and notarized when the secrets are there, ad-hoc signed otherwise), `windows-latest` (`IdleFighterZ_<version>_x64-setup.exe` and `IdleFighterZ_<version>_x64_en-US.msi`, Authenticode-signed when the Windows secrets are there) and `ubuntu-22.04` (`IdleFighterZ_<version>_amd64.AppImage` and `.deb`). A last job puts all of them and one combined `SHA256SUMS` in a GitHub Release, with a note for whatever went out unsigned.

Every pull request (and a manual run from the Actions tab) builds all three, unsigned, and smoke-tests two: on Windows it runs the installer silently, starts the app and checks a second launch hands over and exits; on Linux it installs the `.deb` and runs the AppImage under Xvfb, which has no compositor, so it checks the fallback to window mode too. The packages are attached to the run as artifacts. Signing secrets are only read for `desktop-v*` tags.

### Where the secrets live

The signing secrets are kept in a GitHub **environment** called `release`, not at repo level. Only jobs that name it can read them, and only runs from a `desktop-v*` tag can use it: the `build` and `release` jobs name it on a tag run alone, so pull requests and manual runs never get them. Within a job each secret is passed to the one step that needs it, never to the whole job: npm's install scripts, the crates' build scripts and third-party actions don't run next to any of them. The certificate import gets the certificate and its password; the macOS build gets only the identity's name; the notarizing step gets the notary credentials after the build has finished; the Windows build signs by thumbprint from the certificate store. Every third-party action is pinned to a commit.

One-time setup, with `gh` (repo admin):

1. Create the environment, open only to `desktop-v*` tags:
   ```sh
   gh api -X PUT repos/highskore/idle-fighterz-widget/environments/release \
     -F 'deployment_branch_policy[protected_branches]=false' -F 'deployment_branch_policy[custom_branch_policies]=true'
   gh api -X POST repos/highskore/idle-fighterz-widget/environments/release/deployment-branch-policies -f name='desktop-v*' -f type=tag
   gh api repos/highskore/idle-fighterz-widget/environments/release/deployment-branch-policies   # one policy: desktop-v*, type tag
   ```
2. Move the secrets into it: set each one on the environment (below), then delete the repo-level copy. `gh` asks for each value, so nothing lands in your shell history:
   ```sh
   gh secret list -R highskore/idle-fighterz-widget                         # the repo-level ones to move
   gh secret set APPLE_CERTIFICATE -R highskore/idle-fighterz-widget --env release   # …and so on for each secret below
   gh secret delete APPLE_CERTIFICATE -R highskore/idle-fighterz-widget     # the repo-level copy, once the environment has it
   gh secret list -R highskore/idle-fighterz-widget --env release
   ```
3. Protect the tags, so only you can create, move or delete a `desktop-v*` tag (anyone who could push one could run the release with its secrets):
   ```sh
   gh api -X POST repos/highskore/idle-fighterz-widget/rulesets --input - <<'JSON'
   {
     "name": "release tags",
     "target": "tag",
     "enforcement": "active",
     "conditions": { "ref_name": { "include": ["refs/tags/desktop-v*"], "exclude": [] } },
     "rules": [{ "type": "creation" }, { "type": "update" }, { "type": "deletion" }, { "type": "non_fast_forward" }],
     "bypass_actors": [{ "actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always" }]
   }
   JSON
   ```
   `actor_id: 5` is the repository admin role: admins (you) can still push release tags, nobody else can.

Secrets to add yourself, to the `release` environment (repo → Settings → Environments → release, or `gh secret set NAME --env release`):

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

`packaging/idle-fighterz.rb` is a cask for a tap repo, `highskore/homebrew-tap`, so the install is `brew install --cask highskore/tap/idle-fighterz`. After each release:

```sh
packaging/bump-cask.sh 0.1.0 ~/code/homebrew-tap   # version + sha256 from the release's SHA256SUMS, copied to Casks/
cd ~/code/homebrew-tap && git commit -am "idle-fighterz 0.1.0" && git push
```

## winget and Scoop

`packaging/winget/` holds a manifest set for [winget-pkgs](https://github.com/microsoft/winget-pkgs) (`highskore.IdleFighterZ`, the per-user installer) and `packaging/scoop/idle-fighterz.json` a Scoop manifest for a bucket (it unpacks the same installer; `checkver` and `autoupdate` follow new releases). After a release, fill in the version, date and installer sha256 from `SHA256SUMS`:

```sh
V=0.2.0; SHA=$(curl -fsSL https://github.com/highskore/idle-fighterz-widget/releases/download/desktop-v$V/SHA256SUMS | awk -v f=IdleFighterZ_${V}_x64-setup.exe '$2 == f { print $1 }')
mkdir -p out && for f in packaging/winget/*.yaml; do sed "s/{{VERSION}}/$V/g; s/{{SHA256}}/$SHA/; s/{{DATE}}/$(date +%F)/" "$f" > out/$(basename "$f"); done
winget validate out          # on Windows; then open a PR to winget-pkgs with manifests/h/highskore/IdleFighterZ/$V/
```

For Scoop, set `version` and `hash` in `idle-fighterz.json` the same way and commit it to a bucket repo (e.g. `highskore/scoop-bucket`), so the install is `scoop bucket add highskore https://github.com/highskore/scoop-bucket` then `scoop install idle-fighterz`.

## curl and PowerShell

The installers are served from the highskore.com repo as `https://highskore.com/install.sh` (macOS and Linux) and `https://highskore.com/install.ps1` (Windows):

```sh
curl -fsSL https://highskore.com/install.sh | sh
```

On macOS it fetches the latest release of `highskore/idle-fighterz-widget`, checks the zip against `SHA256SUMS`, copies the app to `/Applications` (or `~/Applications`) and opens it. curl doesn't quarantine what it downloads, so even a build that isn't notarized opens; it says so, and that a browser download of such a build needs right-click → Open. On Linux it checks the AppImage the same way, installs it as `~/.local/bin/idle-fighterz` and adds a menu entry, with the icon taken from the AppImage it just checked. `install.ps1` checks the `-setup.exe`, runs it silently (per user, no admin) and starts `idle-fighterz.exe`; it refuses an elevated (admin) PowerShell unless `IDLE_FIGHTERZ_ALLOW_ADMIN=1`. `IDLE_FIGHTERZ_RELEASE`, a release's API URL, points either at a release other than the latest.

`packaging/install.sh` and `packaging/install.ps1` are copies of the two, and must stay byte-identical to the site's: CI warns when they differ, and on a `desktop-v*` tag the difference fails the run, before anything is released. Change both repos together. CI runs `install.ps1` in Windows PowerShell on every build (the `install-ps1` job), and `install.sh` on macOS and Linux (the `install-sh` job, `scripts/test-install-sh.sh`).

### Signed checksums

A `SHA256SUMS` from the same release as the download only catches a broken download: whoever can change the release can change both. So the release workflow signs `SHA256SUMS` with [minisign](https://jedisct1.github.io/minisign/) into `SHA256SUMS.minisig`, and the installers check it against the public key they carry, which an attacker on GitHub can't change. A signature that doesn't verify is refused, and so is a release with no `SHA256SUMS.minisig` (deleting it would otherwise turn a release back into an unsigned one). There are no exceptions, so every release must be signed: the `release` job fails without `MINISIGN_SECRET_KEY`.

- **The key.** It exists: the private half is `~/.minisign/powerrr.key` on your Mac (named before the rename), and it is in the `release` environment as `MINISIGN_SECRET_KEY` (the key file, or just its second line) with `MINISIGN_PASSWORD` (its password, if it has one). Only the `sign SHA256SUMS` step of the `release` job gets them. The public key is `RWQsYREKR+Av/TIhq7yrFboAERzsd4eoUxepZrikLPg8/hrtXm7MvY1C`, in `MINISIGN_PUBKEY` at the top of `install.sh` (line 18) and `$MinisignPublicKey` in `install.ps1` (line 19), in both repos. Keep a backup of the private key (and its password) offline: without it no release can be signed for these installers.
- **The format.** The workflow signs with `minisign -S -l` (the legacy format: Ed25519 over the file itself, not over its BLAKE2b hash), so the installers can verify it without minisign. `install.sh` uses `minisign` if it's installed, else OpenSSL 3 (`pkeyutl -verify -rawin` on the raw key), else, on macOS, whose `openssl` is LibreSSL without Ed25519, a short RFC 8032 verifier run by JavaScriptCore through `osascript` (always there, BigInt since macOS 11); with none of them (a Linux without OpenSSL 3) it stops and says to install minisign. `install.ps1` verifies Ed25519 itself in PowerShell (`System.Numerics.BigInteger` and .NET's SHA-512). Both check the trusted comment's signature too, and that the signature's key ID is the embedded key's. The workflow signs with the trusted comment `idle-fighterz <tag>` (e.g. `idle-fighterz desktop-v0.2.0`), and the installers refuse a signature whose comment doesn't name exactly the release they're installing: an older release's genuinely signed `SHA256SUMS`, served as a newer one, would otherwise install the older build. `scripts/test-install-sh.sh` and the `install-ps1` job make a throwaway key, sign a stand-in `SHA256SUMS` with it, and check each verifier accepts it and refuses a changed file, comment or key.
- **A new key** (if the old one is lost or leaks): `minisign -G -p idle-fighterz.pub -s ~/.minisign/idle-fighterz.key`, then `gh secret set MINISIGN_SECRET_KEY -R highskore/idle-fighterz-widget --env release < ~/.minisign/idle-fighterz.key` and `gh secret set MINISIGN_PASSWORD -R highskore/idle-fighterz-widget --env release`, then paste the second line of `idle-fighterz.pub` into the four installer copies (the lines above), and ship the installers before the next release.

### Code signatures

Once builds are signed, the installers check that too, before installing:

- **macOS**: an app signed with a Developer ID must pass `codesign --verify --deep --strict`, be signed by the team in `APPLE_TEAM_ID` (`install.sh`, line 22) and pass `spctl --assess` (Gatekeeper, so it must be notarized), or it's refused. An app with a Developer ID signature and no `APPLE_TEAM_ID` in the installer is refused too, so fill it in (your 10-character Team ID, as in `Developer ID Application: Your Name (TEAMID)`) before the first signed release. Until it's set, ad-hoc signed builds install as before, with the note; once it's set, a build without a Developer ID signature is refused, so set it when the macOS builds are signed for good.
- **Windows**: with `$SignerThumbprint` and/or `$SignerSubject` set (`install.ps1`, lines 24 and 25), the `-setup.exe` must have a `Valid` Authenticode signature from that certificate (`Get-AuthenticodeSignature`), or it's refused: an unsigned one too. The thumbprint changes when the certificate is renewed and the subject doesn't, so the subject alone is the easier one to keep (`(Get-PfxCertificate cert.pfx).Subject` prints it). Unset, a valid signature's signer is printed and nothing is required.

The scoop manifest's `hash` is all zeros until a release fills it in; that fails closed (Scoop refuses the download), so it's fine to leave as is.

## First release, in order

1. Create the public repo `highskore/homebrew-tap` (empty, with a `Casks/` folder).
2. Set up the `release` environment and the tag ruleset, and add the secrets above to the environment (see "Where the secrets live").
3. Set `version` in `src-tauri/tauri.conf.json` (and `package.json`, `Cargo.toml`), commit, tag `desktop-v<version>`, push the tag. Watch the workflow; download the dmg and check `spctl -a -vv "/Volumes/Idle FighterZ/Idle FighterZ.app"`.
4. Run `packaging/bump-cask.sh <version> <tap checkout>`, push the tap, then try `brew install --cask highskore/tap/idle-fighterz` and the curl line.
