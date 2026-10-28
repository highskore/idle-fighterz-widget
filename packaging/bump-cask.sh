#!/usr/bin/env bash
# Points the cask at a published release: sets version and sha256 from the release's SHA256SUMS.
#   packaging/bump-cask.sh 0.2.0                       updates packaging/powerrr.rb
#   packaging/bump-cask.sh 0.2.0 ~/code/homebrew-tap   …and copies it to Casks/ in a tap checkout
# REPO defaults to highskore/powerrr.
set -euo pipefail
VERSION=${1:?usage: bump-cask.sh <version> [tap checkout]}
REPO=${REPO:-highskore/powerrr}
CASK="$(dirname "$0")/powerrr.rb"
PRODUCT=$(node -p "require(require('path').resolve(process.argv[1])).productName" "$(dirname "$0")/../src-tauri/tauri.conf.json")
DMG="${PRODUCT// /-}_${VERSION}_universal.dmg"

SUMS=$(curl -fsSL "https://github.com/$REPO/releases/download/desktop-v$VERSION/SHA256SUMS")
SHA=$(printf '%s\n' "$SUMS" | awk -v f="$DMG" '$2 == f { print $1 }')
[ ${#SHA} -eq 64 ] || { echo "no sha256 for $DMG in desktop-v$VERSION's SHA256SUMS" >&2; exit 1; }

sed -i '' -E "s/^  version \".*\"/  version \"$VERSION\"/; s/^  sha256 \".*\"/  sha256 \"$SHA\"/" "$CASK"
echo "powerrr $VERSION  $SHA"
if [ -n "${2:-}" ]; then
  mkdir -p "$2/Casks" && cp "$CASK" "$2/Casks/powerrr.rb"
  echo "copied to $2/Casks/powerrr.rb: commit and push the tap"
fi
