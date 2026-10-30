#!/usr/bin/env bash
# Builds the macOS screensaver bundle, ad-hoc signed: saver/build/<name>.saver.
#
#   ARCHS        arm64 and/or x86_64 (default: this Mac's); both makes a universal bundle
#   SAVER_NAME   the bundle's and the screensaver list's name (default "Idle FighterZ")
#   SAVER_ID     its bundle id (default com.highskore.idlefighterz.saver)
#   SAVER_CLASS  the view's class name: a second copy loaded beside the first needs its own (default IdleFighterZView)
#   JOBS         cargo jobs
set -euo pipefail
cd "$(dirname "$0")/.."
ARCHS=${ARCHS:-$(uname -m)}
NAME=${SAVER_NAME:-Idle FighterZ}
ID=${SAVER_ID:-com.highskore.idlefighterz.saver}
CLASS=${SAVER_CLASS:-IdleFighterZView}
VERSION=$(node -p "require('../src-tauri/tauri.conf.json').version" 2>/dev/null || echo 0.0.0)
OUT=build
B="$OUT/$NAME.saver"
export MACOSX_DEPLOYMENT_TARGET=11.0

rm -rf "$B" && mkdir -p "$B/Contents/MacOS" "$B/Contents/Resources" "$OUT/obj"
objs=()
for a in $ARCHS; do
  case $a in arm64) t=aarch64-apple-darwin ;; x86_64) t=x86_64-apple-darwin ;; *) echo "unknown arch $a" >&2; exit 2 ;; esac
  cargo build --release --lib --target "$t" ${JOBS:+-j "$JOBS"}
  clang -arch "$a" -mmacosx-version-min=11.0 -fobjc-arc -O2 -Wall -Werror -bundle -DIFZ_VIEW="$CLASS" \
    macos/IdleFighterZView.m "target/$t/release/libifz_saver.a" \
    -framework ScreenSaver -framework Cocoa -framework QuartzCore -o "$OUT/obj/IdleFighterZ-$a"
  objs+=("$OUT/obj/IdleFighterZ-$a")
done
lipo -create "${objs[@]}" -output "$B/Contents/MacOS/IdleFighterZ"
sed -e "s|@ID@|$ID|" -e "s|@NAME@|$NAME|" -e "s|@CLASS@|$CLASS|" -e "s|@VERSION@|$VERSION|g" macos/Info.plist > "$B/Contents/Info.plist"
cp macos/thumbnail.png macos/thumbnail@2x.png "$B/Contents/Resources/" 2>/dev/null || true
codesign --force --sign - "$B"
echo "$B"
