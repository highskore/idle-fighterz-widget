#!/usr/bin/env bash
# Builds the app (named by productName in tauri.conf.json) with the screensaver inside it, signs both with your
# Developer ID (hardened runtime), notarizes and staples it, and packs a .dmg and a .zip with their sha256 sums into
# release/. See README.md, "Releasing".
#
#   APPLE_SIGNING_IDENTITY   "Developer ID Application: Your Name (TEAMID)", in your keychain. Unset: ad-hoc signed,
#                            not notarized (runs here; elsewhere it needs right-click → Open)
#   notarizing, the first that's set:
#     NOTARY_PROFILE         a keychain profile saved by `xcrun notarytool store-credentials` (local)
#     APPLE_API_KEY, APPLE_API_ISSUER, APPLE_API_KEY_PATH   an App Store Connect API key: key id, issuer id, .p8 file
#     APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID               an Apple ID with an app-specific password
#   TARGET                   universal-apple-darwin (default), aarch64-apple-darwin or x86_64-apple-darwin
#   JOBS                     cargo jobs (default: all cores)
#   APP, OUT                 sign and package this .app (default: the one build made), into OUT (default: release)
#
#   scripts/release.sh [build|sign|package]   all three by default. build only ad-hoc signs; sign uses the Developer
#                                             ID, with codesign alone. CI runs build in a job with no secrets and sign
#                                             and package in another, so neither the certificate nor the notary
#                                             credentials are ever next to npm or the crates' build scripts
set -euo pipefail
cd "$(dirname "$0")/.."
STAGE=${1:-all}
case "$STAGE" in all | build | sign | package) ;; *) echo "usage: scripts/release.sh [build|sign|package]" >&2; exit 2 ;; esac

TARGET=${TARGET:-universal-apple-darwin}
VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
PRODUCT=$(node -p "require('./src-tauri/tauri.conf.json').productName")
ARCH=${TARGET%%-*}
[ "$ARCH" = aarch64 ] && ARCH=arm64
[ "$ARCH" = x86_64 ] && ARCH=x64
NAME="${PRODUCT// /}_${VERSION}_${ARCH}"
OUT=${OUT:-release}

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  security find-identity -v -p codesigning | grep -qF "$APPLE_SIGNING_IDENTITY" || { echo "no signing identity \"$APPLE_SIGNING_IDENTITY\" in the keychain" >&2; exit 1; }
  if [ -n "${NOTARY_PROFILE:-}" ]; then NOTARY=(--keychain-profile "$NOTARY_PROFILE")
  elif [ -n "${APPLE_API_KEY:-}" ]; then NOTARY=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
  elif [ -n "${APPLE_ID:-}" ]; then NOTARY=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
  else echo "signing without notarizing: set NOTARY_PROFILE, APPLE_API_KEY… or APPLE_ID…" >&2; NOTARY=()
  fi
else
  echo "APPLE_SIGNING_IDENTITY unset: ad-hoc signed, not notarized" >&2; NOTARY=()
fi

APP=${APP:-"src-tauri/target/$TARGET/release/bundle/macos/$PRODUCT.app"}
if [ "$STAGE" = all ] || [ "$STAGE" = build ]; then
  case "$TARGET" in
    universal-apple-darwin) rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null ;;
    *) rustup target add "$TARGET" >/dev/null ;;
  esac
  [ -d node_modules ] || npm ci --no-audit --no-fund
  npm run -s page
  case "$TARGET" in
    universal-apple-darwin) SAVER_ARCHS="arm64 x86_64" ;;
    aarch64-*) SAVER_ARCHS=arm64 ;;
    *) SAVER_ARCHS=x86_64 ;;
  esac
  # tauri and build.sh only ad-hoc sign (signingIdentity "-"); the Developer ID signature and notarizing come below,
  # with codesign and notarytool, so no certificate or notary credential is around while the build runs. The
  # screensaver is built first: saver/tauri.macos.json puts it in the app's Resources
  quiet() {
    env -u APPLE_SIGNING_IDENTITY -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD \
      -u APPLE_ID -u APPLE_PASSWORD -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH "$@"
  }
  quiet ARCHS="$SAVER_ARCHS" ${JOBS:+JOBS=$JOBS} saver/macos/build.sh
  quiet ${JOBS:+CARGO_BUILD_JOBS=$JOBS} npx tauri build --bundles app --target "$TARGET" --config saver/tauri.macos.json
  [ -d "$APP/Contents/Resources/Idle FighterZ.saver" ] || { echo "no screensaver in $APP" >&2; exit 1; }
  codesign --verify --strict --deep "$APP"
fi
[ "$STAGE" = build ] && exit 0
[ -d "$APP" ] || { echo "no $APP: run scripts/release.sh build first" >&2; exit 1; }

# inside out, under the hardened runtime: the screensaver bundle in Resources (tauri doesn't sign it), what tauri
# would sign (the binaries in Contents/MacOS), then the app
if [ "$STAGE" != package ] && [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  [ ! -e "$APP/Contents/Frameworks" ] || { echo "$APP has Frameworks: sign them here too" >&2; exit 1; }
  xattr -cr "$APP"
  shopt -s nullglob
  for f in "$APP"/Contents/Resources/*.saver "$APP"/Contents/MacOS/* "$APP"; do
    codesign --force --timestamp --options runtime --sign "$APPLE_SIGNING_IDENTITY" "$f"
  done
  codesign --verify --strict --deep "$APP"
fi
[ "$STAGE" = sign ] && exit 0
rm -rf "$OUT" && mkdir -p "$OUT"

notarize() {   # notarize <file to submit> <file to staple>
  [ ${#NOTARY[@]} -gt 0 ] || return 0
  xcrun notarytool submit "$1" "${NOTARY[@]}" --wait
  xcrun stapler staple "$2"
}

# the app: submitted as a zip, then the ticket is stapled to the app itself so it opens offline too
ditto -c -k --keepParent "$APP" "$OUT/submit.zip"
notarize "$OUT/submit.zip" "$APP"
rm "$OUT/submit.zip"
ditto -c -k --keepParent "$APP" "$OUT/$NAME.zip"

# the dmg: the app and a link to /Applications to drag it onto
STAGE=$(mktemp -d)
ditto "$APP" "$STAGE/$PRODUCT.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "$PRODUCT" -srcfolder "$STAGE" -ov -format UDZO "$OUT/$NAME.dmg" >/dev/null
rm -rf "$STAGE"
[ -n "${APPLE_SIGNING_IDENTITY:-}" ] && codesign --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$OUT/$NAME.dmg"
notarize "$OUT/$NAME.dmg" "$OUT/$NAME.dmg"

(cd "$OUT" && shasum -a 256 "$NAME.dmg" "$NAME.zip" > SHA256SUMS && cat SHA256SUMS)
[ ${#NOTARY[@]} -gt 0 ] && spctl --assess --type execute -vv "$APP"
echo "done: $OUT/"
