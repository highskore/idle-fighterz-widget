#!/bin/sh
# Tests packaging/install.sh's checks of a release. It makes a throwaway minisign key, signs a SHA256SUMS with it,
# and checks every signature verifier the installer has on this OS (minisign, OpenSSL 3, and JavaScriptCore on
# macOS) accepts it and refuses a tampered file, comment or key. Then it serves a stand-in release of <package> (the
# universal .zip on macOS, the .AppImage on Linux) and runs the installer as users do, signed and unsigned.
#
#   scripts/test-install-sh.sh <minisign binary> <package>
set -eu
MINISIGN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
PKG=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
SERVER=
trap '[ -z "$SERVER" ] || kill "$SERVER" 2>/dev/null; rm -rf "$T"' EXIT
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
ok() { printf 'ok: %s\n' "$*"; }

# ---- a throwaway key, never a real one: made here, unencrypted, deleted on exit
mkdir -p "$T/key" "$T/other" "$T/bin"
"$MINISIGN" -G -W -p "$T/key/k.pub" -s "$T/key/k.key" >/dev/null
"$MINISIGN" -G -W -p "$T/other/k.pub" -s "$T/other/k.key" >/dev/null
KEY=$(sed -n 2p "$T/key/k.pub")
OTHER=$(sed -n 2p "$T/other/k.pub")
sign() { "$MINISIGN" -S -l -s "$T/key/k.key" -m "$1" -t "${2:-powerrr test}" >/dev/null; }   # sign <file> [trusted comment], as release.yml does

# the installer with the throwaway key in place of the real one (and $LEGACY as its pinned unsigned releases), and
# without its last line (main "$@") to source it
with_key() { sed -e "s|^MINISIGN_PUBKEY=.*|MINISIGN_PUBKEY='$1'|" -e "s|^LEGACY_SUMS=.*|LEGACY_SUMS='${LEGACY:-}'|" "$ROOT/packaging/install.sh"; }
with_key "$KEY" | sed '$d' > "$T/lib.sh"
grep -q "^MINISIGN_PUBKEY='$KEY'" "$T/lib.sh" || fail "couldn't set the test key"
# the real key, as shipped: a legacy-capable Ed25519 minisign key (42 bytes: "Ed", key id, key)
REAL=$(sed -n "s/^MINISIGN_PUBKEY='\(.*\)'$/\1/p" "$ROOT/packaging/install.sh")
[ -n "$REAL" ] && [ "$(printf '%s' "$REAL" | openssl base64 -d -A | wc -c | tr -d ' ')" = 42 ] && [ "$(printf '%s' "$REAL" | openssl base64 -d -A | head -c 2)" = Ed ] ||
  fail "packaging/install.sh's MINISIGN_PUBKEY isn't a minisign public key"
ok "the shipped key is a minisign Ed25519 public key"

# ---- the verifiers, one at a time
printf '%s  Powerrr_1.0.0_universal.zip\n' "$(printf x | shasum -a 256 | cut -d ' ' -f 1)" > "$T/SUMS"
sign "$T/SUMS"
# a PATH with no minisign on it, so the installer falls through to the next verifier
NOMS=$(printf '%s' "$PATH" | tr ':' '\n' | while read -r d; do [ -x "$d/minisign" ] || printf '%s:' "$d"; done)
NOMS=${NOMS%:}
verify() {   # verify <verifier> <sums> <minisig> [key]: the installer's verify_sums with only that verifier
  (
    TMP=$T
    . "$T/lib.sh"
    [ -z "${4:-}" ] || MINISIGN_PUBKEY=$4
    case "$1" in
      minisign) PATH="$(dirname "$MINISIGN"):$NOMS" ;;
      openssl) PATH=$NOMS; openssl3 >/dev/null || { echo skip; exit 0; } ;;
      jsc) PATH=$NOMS; openssl3() { return 1; } ;;
    esac
    verify_sums "$2" "$3" >/dev/null 2>&1 && echo pass || echo refuse
  )
}
VERIFIERS="minisign openssl"
[ "$(uname -s)" = Darwin ] && VERIFIERS="$VERIFIERS jsc"
cp "$T/SUMS" "$T/SUMS.bad" && printf 'deadbeef  evil\n' >> "$T/SUMS.bad"
sed '3s/$/!/' "$T/SUMS.minisig" > "$T/comment.minisig"   # the trusted comment changed
for v in $VERIFIERS; do
  r=$(verify "$v" "$T/SUMS" "$T/SUMS.minisig")
  [ "$r" = skip ] && { printf 'skip: %s (not on this system)\n' "$v"; continue; }
  [ "$r" = pass ] || fail "$v: a good signature was refused"
  [ "$(verify "$v" "$T/SUMS.bad" "$T/SUMS.minisig")" = refuse ] || fail "$v: a tampered SHA256SUMS passed"
  [ "$(verify "$v" "$T/SUMS" "$T/comment.minisig")" = refuse ] || fail "$v: a tampered trusted comment passed"
  [ "$(verify "$v" "$T/SUMS" "$T/SUMS.minisig" "$OTHER")" = refuse ] || fail "$v: another key's signature passed"
  ok "$v verifies a good signature and refuses a tampered file, comment and key"
done
# the prehashed format (minisign without -l) isn't one the fallbacks can check: refused everywhere alike
"$MINISIGN" -S -s "$T/key/k.key" -m "$T/SUMS" -x "$T/hashed.minisig" >/dev/null
[ "$(verify minisign "$T/SUMS" "$T/hashed.minisig")" = refuse ] || fail "a prehashed signature passed"
ok "a prehashed (non -l) signature is refused"

# ---- the installer end to end, against a stand-in release on localhost
case "$(uname -s)" in
  Darwin) KIND=mac ;;
  *) KIND=linux ;;
esac
SRV=$T/srv
mkdir -p "$SRV"
NAME=$(basename "$PKG")
cp "$PKG" "$SRV/"
# one folder per release, each with its SHA256SUMS (and SHA256SUMS.minisig)
mkdir -p "$SRV/unsigned" "$SRV/unsigned-new" "$SRV/legacy-changed" "$SRV/signed" "$SRV/tampered" "$SRV/signed-bad" "$SRV/replay"
(cd "$SRV" && shasum -a 256 "$NAME" > unsigned/SHA256SUMS)
# ci-unsigned stands for a release from before signing (pinned in the installer); ci-unsigned-new for a later one whose
# .minisig was deleted; legacy-changed is ci-unsigned with its SHA256SUMS swapped
LEGACY="ci-unsigned=$(shasum -a 256 "$SRV/unsigned/SHA256SUMS" | cut -d ' ' -f 1)"
cp "$SRV/unsigned/SHA256SUMS" "$SRV/unsigned-new/"
printf '%064d  %s\n' 0 "$NAME" > "$SRV/legacy-changed/SHA256SUMS"
cp "$SRV/unsigned/SHA256SUMS" "$SRV/signed/" && sign "$SRV/signed/SHA256SUMS" 'powerrr ci-signed'
cp "$SRV/signed/SHA256SUMS" "$SRV/signed/SHA256SUMS.minisig" "$SRV/tampered/" && printf '%064d  extra\n' 0 >> "$SRV/tampered/SHA256SUMS"
cp "$SRV/signed/SHA256SUMS" "$SRV/signed/SHA256SUMS.minisig" "$SRV/replay/"   # ci-signed's, served as ci-replay
printf '%064d  %s\n' 0 "$NAME" > "$SRV/signed-bad/SHA256SUMS" && sign "$SRV/signed-bad/SHA256SUMS" 'powerrr ci-signed-bad'   # a wrong hash
PORT=8766
URL=http://127.0.0.1:$PORT
for r in unsigned unsigned-new legacy-changed signed tampered signed-bad replay; do
  tag=ci-$r; [ $r != legacy-changed ] || tag=ci-unsigned
  sig=; [ ! -f "$SRV/$r/SHA256SUMS.minisig" ] || sig=", {\"name\": \"SHA256SUMS.minisig\", \"browser_download_url\": \"$URL/$r/SHA256SUMS.minisig\"}"
  printf '{"tag_name": "%s", "assets": [{"name": "%s", "browser_download_url": "%s/%s"}, {"name": "SHA256SUMS", "browser_download_url": "%s/%s/SHA256SUMS"}%s]}\n' \
    "$tag" "$NAME" "$URL" "$NAME" "$URL" "$r" "$sig" > "$SRV/$r.json"
done
python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$SRV" >/dev/null 2>&1 &
SERVER=$!
i=0; until curl -fsS "$URL/unsigned.json" >/dev/null 2>&1; do i=$((i + 1)); [ $i -lt 30 ] || fail "the test server didn't start"; sleep 1; done

with_key "$KEY" > "$T/install.sh"
with_key "$OTHER" > "$T/install-other.sh"
# stand-ins, so the test doesn't launch the app or kill a running one
printf '#!/bin/sh\nexit 0\n' > "$T/bin/open" && cp "$T/bin/open" "$T/bin/pkill" && chmod +x "$T/bin/open" "$T/bin/pkill"
run() {   # run <installer> <release>: its exit code, and everything it printed in $T/out
  rm -rf "$T/dest" "$T/home"; mkdir -p "$T/dest" "$T/home"
  env -u DISPLAY -u WAYLAND_DISPLAY PATH="$T/bin:$NOMS" HOME="$T/home" XDG_DATA_HOME="$T/home/share" POWERRR_DIR="$T/dest" \
    POWERRR_RELEASE="$URL/$2.json" sh "$1" > "$T/out" 2>&1 && code=0 || code=$?
  sed 's/^/    /' "$T/out"
  return 0
}
installed() { if [ $KIND = mac ]; then ls -d "$T"/dest/*.app >/dev/null 2>&1; else [ -x "$T/dest/powerrr" ]; fi; }

run "$T/install.sh" unsigned
[ $code = 0 ] && installed && grep -q 'unsigned checksums' "$T/out" || fail "a pinned unsigned release didn't install with the unsigned checksums note"
ok "a release from before signing installs, with the note"
if [ $KIND = linux ]; then
  [ -s "$T/home/share/icons/hicolor/128x128/apps/powerrr.png" ] || fail "no icon from the AppImage"
  ok "the menu icon comes from the AppImage"
fi

run "$T/install.sh" signed
[ $code = 0 ] && installed && grep -q 'SHA256SUMS signature ok' "$T/out" || fail "a signed release didn't install"
ok "a signed release installs"

if [ $KIND = mac ]; then
  sed "s|^APPLE_TEAM_ID=.*|APPLE_TEAM_ID='ABCDE12345'|" "$T/install.sh" > "$T/install-team.sh"
  run "$T/install-team.sh" signed
  [ $code != 0 ] && ! installed && grep -q "isn't signed with a Developer ID" "$T/out" || fail "an app without a Developer ID passed an installer with APPLE_TEAM_ID set"
  ok "with APPLE_TEAM_ID set, an app without a Developer ID signature is refused"
fi

run "$T/install.sh" unsigned-new
[ $code != 0 ] && ! installed && grep -q 'no SHA256SUMS.minisig' "$T/out" || fail "a release with its .minisig removed wasn't refused"
ok "a later release without a .minisig is refused"

run "$T/install.sh" legacy-changed
[ $code != 0 ] && ! installed && grep -q "isn't the one it was released with" "$T/out" || fail "a changed SHA256SUMS on a pinned release wasn't refused"
ok "a release from before signing with a changed SHA256SUMS is refused"

run "$T/install.sh" replay
[ $code != 0 ] && ! installed && grep -q "was signed for 'powerrr ci-signed', not 'powerrr ci-replay'" "$T/out" || fail "another release's signed SHA256SUMS wasn't refused"
ok "another release's signed SHA256SUMS is refused"

run "$T/install.sh" tampered
[ $code != 0 ] && ! installed && grep -q "doesn't verify" "$T/out" || fail "a tampered SHA256SUMS wasn't refused"
ok "a tampered SHA256SUMS is refused, and nothing is installed"

with_key '' > "$T/install-nokey.sh"
run "$T/install-nokey.sh" signed
[ $code != 0 ] && ! installed && grep -q 'no key to check it with' "$T/out" || fail "a signed release passed an installer with no key"
ok "a signed release is refused by an installer with no key"

run "$T/install-other.sh" signed
[ $code != 0 ] && ! installed || fail "a release signed with another key wasn't refused"
ok "a release signed with another key is refused"

run "$T/install.sh" signed-bad
[ $code != 0 ] && ! installed && grep -q 'sha256 mismatch' "$T/out" || fail "a wrong sha256 under a good signature wasn't refused"
ok "a wrong sha256 is refused even when SHA256SUMS is signed"
echo "all install.sh tests passed"
