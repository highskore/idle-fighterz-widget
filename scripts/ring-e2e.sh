#!/bin/bash
# The first-run ring on screen (macOS): runs a built bundle with a fresh first-run settings file at 300, 400 and 720 pt
# (720 draws the pixel art 2×) in the dock, pill and strip layouts, with the frames on the macOS layer under the page and
# on the page's canvas (IDLE_FIGHTERZ_ENGINE=canvas), captures only the widget's own window
# (no desktop, no other app), and checks the gold ring against the planet disc drawn in that capture
# (scripts/ring-check.swift: centred within 2 pt, 4 to 14 pt clear of the outline).
# Use a test identifier: its settings file is overwritten. Waits until there's been no keyboard or mouse input for 30 s
# before each launch, as the widget may take the focus.
# Run: sh scripts/ring-e2e.sh "<path>/Idle FighterZ.app" com.example.test [out-dir]
set -u -o pipefail
APP="$1/Contents/MacOS/idle-fighterz"; ID=$2; OUT=${3:-$(mktemp -d)}
DIR="$HOME/Library/Application Support/$ID"; HERE=$(cd "$(dirname "$0")" && pwd)
[ -f "$DIR/settings.json" ] && cp "$DIR/settings.json" "$OUT/settings.json.bak"
mkdir -p "$DIR" "$OUT"
idle() { ioreg -c IOHIDSystem | awk '/HIDIdleTime/ {print int($NF/1000000000); exit}'; }
wins="$OUT/wins.swift"
cat > "$wins" <<'SWIFT'
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
for w in CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]] where (w[kCGWindowOwnerPID as String] as? Int) == pid && (w[kCGWindowName as String] as? String) == "Idle FighterZ" { print(w[kCGWindowNumber as String]!) }
SWIFT
fail=0
for case in "300 7 layer" "400 7 layer" "720 7 layer" "400 6 layer" "400 2 layer" "400 7 canvas" "720 6 canvas"; do
  set -- $case
  until [ "$(idle)" -ge 30 ]; do sleep 5; done
  echo "{\"size\":$1,\"layout\":$2,\"onboarded\":false}" > "$DIR/settings.json"
  if [ "$3" = canvas ]; then IDLE_FIGHTERZ_ENGINE=canvas "$APP" > "$OUT/run.log" 2>&1 & else "$APP" > "$OUT/run.log" 2>&1 & fi; pid=$!
  sleep 7
  id=$(swift "$wins" $pid | head -1)
  shot="$OUT/ring-$1-$2-$3.png"
  screencapture -x -o -l "$id" "$shot"
  kill $pid; wait $pid 2>/dev/null
  printf '%s pt, layout %s, on the %s: ' "$1" "$2" "$3"
  swift "$HERE/ring-check.swift" "$shot" 2 2 4 14 | tr '\n' ' ' || fail=1
  echo
done
[ -f "$OUT/settings.json.bak" ] && cp "$OUT/settings.json.bak" "$DIR/settings.json"
echo "captures in $OUT"
exit $fail
