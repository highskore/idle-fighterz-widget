#!/usr/bin/env bash
# Builds the app (named by productName in tauri.conf.json), signs it with your Developer ID (hardened runtime), notarizes and staples it, and packs a
# .dmg and a .zip with their sha256 sums into release/. See README.md, "Releasing".
#
#   APPLE_SIGNING_IDENTITY   "Developer ID Application: Your Name (TEAMID)", in your keychain. Unset: ad-hoc signed,
#                            not notarized (runs here; elsewhere it needs right-click → Open)
#   notarizing, the first that's set:
#     NOTARY_PROFILE         a keychain profile saved by `xcrun notarytool store-credentials` (local)
#     APPLE_API_KEY, APPLE_API_ISSUER, APPLE_API_KEY_PATH   an App Store Connect API key: key id, issuer id, .p8 file
#     APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID               an Apple ID with an app-specific password
#   TARGET                   universal-apple-darwin (default), aarch64-apple-darwin or x86_64-apple-darwin
#   JOBS                     cargo jobs (default: all cores)
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=${TARGET:-universal-apple-darwin}
VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
PRODUCT=$(node -p "require('./src-tauri/tauri.conf.json').productName")
ARCH=${TARGET%%-*}
[ "$ARCH" = aarch64 ] && ARCH=arm64
[ "$ARCH" = x86_64 ] && ARCH=x64
NAME="${PRODUCT// /-}_${VERSION}_${ARCH}"
OUT=release

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

case "$TARGET" in
  universal-apple-darwin) rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null ;;
  *) rustup target add "$TARGET" >/dev/null ;;
esac
[ -d node_modules ] || npm ci --no-audit --no-fund
# an empty secret still exports the variable in CI, and tauri then tries to import a certificate that isn't there
for v in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY; do [ -n "${!v:-}" ] || unset "$v"; done
npm run -s page
# notarizing happens below, with notarytool, for the app and the dmg alike: keep tauri from doing it as well
env -u APPLE_ID -u APPLE_PASSWORD -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
  ${JOBS:+CARGO_BUILD_JOBS=$JOBS} npx tauri build --bundles app --target "$TARGET"

APP="src-tauri/target/$TARGET/release/bundle/macos/$PRODUCT.app"
codesign --verify --strict --deep "$APP"
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
