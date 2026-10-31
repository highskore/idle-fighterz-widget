#!/usr/bin/env bash
# Points the cask at a published release: sets version and sha256 from the release's SHA256SUMS, once its signature
# (SHA256SUMS.minisig) verifies with install.sh's key and names this release. Needs minisign.
#   packaging/bump-cask.sh 0.2.0                       updates packaging/idle-fighterz.rb
#   packaging/bump-cask.sh 0.2.0 ~/code/homebrew-tap   …and copies it to Casks/ in a tap checkout
# REPO defaults to highskore/idle-fighterz-widget.
set -euo pipefail
VERSION=${1:?usage: bump-cask.sh <version> [tap checkout]}
[[ $VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $VERSION" >&2; exit 1; }
REPO=${REPO:-highskore/idle-fighterz-widget}
CASK="$(dirname "$0")/idle-fighterz.rb"
PRODUCT=$(node -p "require(require('path').resolve(process.argv[1])).productName" "$(dirname "$0")/../src-tauri/tauri.conf.json")
DMG="${PRODUCT// /}_${VERSION}_universal.dmg"

TAG="desktop-v$VERSION"
PUB=$(sed -n "s/^MINISIGN_PUBKEY='\(.*\)'$/\1/p" "$(dirname "$0")/install.sh")
[ -n "$PUB" ] || { echo "no MINISIGN_PUBKEY in install.sh" >&2; exit 1; }
command -v minisign >/dev/null || { echo "minisign is needed to check SHA256SUMS (brew install minisign)" >&2; exit 1; }
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
curl -fsSL "https://github.com/$REPO/releases/download/$TAG/SHA256SUMS" -o "$W/SHA256SUMS"
curl -fsSL "https://github.com/$REPO/releases/download/$TAG/SHA256SUMS.minisig" -o "$W/SHA256SUMS.minisig"
SIGNED=$(minisign -V -Q -P "$PUB" -m "$W/SHA256SUMS" -x "$W/SHA256SUMS.minisig") || { echo "the signature on $TAG's SHA256SUMS doesn't verify" >&2; exit 1; }
# an older release's SHA256SUMS and signature, served as this one, are refused
[ "$SIGNED" = "idle-fighterz $TAG" ] || { echo "SHA256SUMS was signed for '$SIGNED', not 'idle-fighterz $TAG'" >&2; exit 1; }
SUMS=$(cat "$W/SHA256SUMS")
SHA=$(printf '%s\n' "$SUMS" | awk -v f="$DMG" '$2 == f { print $1 }')
[ ${#SHA} -eq 64 ] || { echo "no sha256 for $DMG in desktop-v$VERSION's SHA256SUMS" >&2; exit 1; }

sed -i '' -E "s/^  version \".*\"/  version \"$VERSION\"/; s/^  sha256 \".*\"/  sha256 \"$SHA\"/" "$CASK"
echo "idle-fighterz $VERSION  $SHA"
if [ -n "${2:-}" ]; then
  mkdir -p "$2/Casks" && cp "$CASK" "$2/Casks/idle-fighterz.rb"
  echo "copied to $2/Casks/idle-fighterz.rb: commit and push the tap"
fi
