#!/usr/bin/env bash
# Runs a built .saver in the test harness (harness.m) and keeps its frames.
#
#   saver/macos/try.sh <bundle.saver> <seconds> <shots dir> [width height]
#   SANDBOX=1   sign the harness with legacyScreenSaver's sandbox entitlements (harness.entitlements) first; its shots
#               then land in the sandbox's container (~/Library/Containers/com.highskore.idlefighterz.saverharness),
#               and are moved to <shots dir>, and the container emptied, afterwards
set -euo pipefail
SAVER=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
mkdir -p "$3" && SHOTS=$(cd "$3" && pwd)
cd "$(dirname "$0")"
H=../build/harness
mkdir -p ../build
clang -fobjc-arc -O2 -Wall -Werror harness.m -framework Cocoa -framework ScreenSaver -sectcreate __TEXT __info_plist harness-Info.plist -o "$H"
if [ "${SANDBOX:-}" = 1 ]; then
  ID=com.highskore.idlefighterz.saverharness
  codesign --force --sign - --identifier "$ID" --entitlements harness.entitlements "$H"
  C="$HOME/Library/Containers/$ID"
  "$H" "$SAVER" "$2" "$C/Data/tmp/shots" "${@:4}"
  mv "$C/Data/tmp/shots/"* "$SHOTS/" 2>/dev/null || true
  # macOS keeps the container's metadata file from the shell: what's left is that, empty otherwise
  rm -rf "$C" 2>/dev/null || echo "left the empty container $C" >&2
else
  codesign --force --sign - "$H"
  "$H" "$SAVER" "$2" "$3" "${@:4}"
fi
