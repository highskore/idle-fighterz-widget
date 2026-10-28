#!/bin/sh
# Powerrr installer: curl -fsSL https://highskore.com/install.sh | sh
# Downloads the latest release from github.com/highskore/powerrr, checks its sha256 and puts
# the app in /Applications (or ~/Applications if that isn't writable; POWERRR_DIR picks another).
# On Linux (x86_64) it installs the AppImage as ~/.local/bin/powerrr (POWERRR_DIR picks another
# folder) with a menu entry. Windows: irm https://highskore.com/install.ps1 | iex
# POWERRR_REPO picks another repo, and POWERRR_RELEASE (a release's API URL) a release other than the latest.
#
# A release's SHA256SUMS is signed with minisign (SHA256SUMS.minisig) by the release workflow. The signature is
# checked with minisign if it's installed, else OpenSSL 3, else (macOS, whose openssl can't) JavaScriptCore through
# osascript, so nothing needs installing. A signed release that doesn't verify is refused, and so is an unsigned one,
# except the releases from before signing, listed below with their SHA256SUMS pinned. On macOS a Developer ID signed
# app must also pass codesign and Gatekeeper, from the team below.
# This file is served by highskore.com and kept as a copy in powerrr's packaging/install.sh: change both.
set -eu

# the minisign public key that signs SHA256SUMS (minisign.pub's second line)
MINISIGN_PUBKEY='RWQsYREKR+Av/TIhq7yrFboAERzsd4eoUxepZrikLPg8/hrtXm7MvY1C'
# the releases from before SHA256SUMS was signed, each with the sha256 of its SHA256SUMS: only these install unsigned
LEGACY_SUMS='desktop-v0.1.1=dc73874f6220409f6b9443337c6a8f77b38713eb8e4ecb9530908eeaaf735ffd'
# the Apple team ID of the Developer ID certificate that signs the macOS app; empty until builds are signed
APPLE_TEAM_ID=''

say() { printf '%s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }
b64d() { if command -v openssl >/dev/null 2>&1; then openssl base64 -d -A; else base64 -d; fi; }
hex() { od -An -v -tx1 | tr -d ' \n'; }
size() { wc -c < "$1" | tr -d ' '; }
cut_bytes() { dd if="$1" bs=1 skip="$2" count="$3" 2>/dev/null; }   # cut_bytes <file> <offset> <length>
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d ' ' -f 1; }
legacy_sum() { for e in $LEGACY_SUMS; do [ "${e%%=*}" = "$1" ] && { echo "${e#*=}"; return 0; }; done; return 0; }

# an OpenSSL 3 that can verify Ed25519 (macOS's own openssl is LibreSSL, which can't)
openssl3() {
  for o in openssl /opt/homebrew/bin/openssl /usr/local/bin/openssl /opt/homebrew/opt/openssl@3/bin/openssl /usr/local/opt/openssl@3/bin/openssl; do
    if command -v "$o" >/dev/null 2>&1 && "$o" version 2>/dev/null | grep -q '^OpenSSL [3-9]'; then echo "$o"; return 0; fi
  done
  return 1
}

# ed25519_openssl <openssl> <32-byte public key> <64-byte signature> <message>
ed25519_openssl() {
  # the key as SubjectPublicKeyInfo DER: a fixed 12-byte prefix (id-Ed25519) and the raw key
  { printf '\060\052\060\005\006\003\053\145\160\003\041\000'; cat "$2"; } > "$2.der"
  "$1" pkeyutl -verify -pubin -keyform DER -inkey "$2.der" -rawin -in "$4" -sigfile "$3" >/dev/null 2>&1
}

# ed25519_jsc <32-byte public key> <64-byte signature> <message>: RFC 8032 verification in JavaScriptCore (macOS 11+
# has BigInt), with SHA-512 from shasum. Checks S < L and that [S]B - [k]A encodes to R
ed25519_jsc() {
  k=$({ cut_bytes "$2" 0 32; cat "$1" "$3"; } | shasum -a 512 | cut -d ' ' -f 1)
  cat > "$1.js" <<'JS'
function run([pk, sig, k]) {
  const P = 2n ** 255n - 19n, L = 2n ** 252n + 27742317777372353535851937790883648493n;
  const mod = a => ((a % P) + P) % P;
  const pow = (b, e) => { let r = 1n; b = mod(b); for (; e > 0n; e >>= 1n) { if (e & 1n) r = r * b % P; b = b * b % P; } return r; };
  const inv = a => pow(a, P - 2n);
  const D = mod(-121665n * inv(121666n)), I = pow(2n, (P - 1n) / 4n);
  const le = h => { let n = 0n; for (let i = h.length - 2; i >= 0; i -= 2) n = (n << 8n) | BigInt(parseInt(h.substr(i, 2), 16)); return n; };
  const hex = n => { let s = ''; for (let i = 0; i < 32; i++, n >>= 8n) s += (n & 255n).toString(16).padStart(2, '0'); return s; };
  const add = ([X1, Y1, Z1, T1], [X2, Y2, Z2, T2]) => {
    const a = mod((Y1 - X1) * (Y2 - X2)), b = mod((Y1 + X1) * (Y2 + X2)), c = mod(2n * D * T1 * T2), d = mod(2n * Z1 * Z2);
    const e = b - a, f = d - c, g = d + c, h = b + a;
    return [mod(e * f), mod(g * h), mod(f * g), mod(e * h)];
  };
  const mul = (s, Q) => { let R = [0n, 1n, 1n, 0n]; for (; s > 0n; s >>= 1n) { if (s & 1n) R = add(R, Q); Q = add(Q, Q); } return R; };
  const point = h => {
    let y = le(h); const sign = y >> 255n; y &= (1n << 255n) - 1n;
    if (y >= P) return null;
    const x2 = mod((y * y - 1n) * inv(D * y * y + 1n));
    let x = pow(x2, (P + 3n) / 8n);
    if (mod(x * x - x2) !== 0n) x = mod(x * I);
    if (mod(x * x - x2) !== 0n || (x === 0n && sign === 1n)) return null;
    if ((x & 1n) !== sign) x = P - x;
    return [x, y, 1n, mod(x * y)];
  };
  const encode = ([X, Y, Z]) => { const z = inv(Z), x = mod(X * z), y = mod(Y * z); return hex(y | ((x & 1n) << 255n)); };
  const A = point(pk), S = le(sig.slice(64)), B = point('58' + '66'.repeat(31));
  if (!A || S >= L) return 'bad';
  const minusA = [mod(-A[0]), A[1], A[2], mod(-A[3])];
  return encode(add(mul(S, B), mul(le(k) % L, minusA))) === sig.slice(0, 64) ? 'ok' : 'bad';
}
JS
  [ "$(osascript -l JavaScript "$1.js" "$(hex < "$1")" "$(hex < "$2")" "$k" 2>/dev/null)" = ok ]
}

# verify_sums <SHA256SUMS> <SHA256SUMS.minisig>: the file signature and the trusted comment's, both by MINISIGN_PUBKEY
verify_sums() {
  W=$(mktemp -d "$TMP/minisig.XXXXXX")
  printf '%s' "$MINISIGN_PUBKEY" | b64d > "$W/key" 2>/dev/null || true
  sed -n 2p "$2" | b64d > "$W/line" 2>/dev/null || true
  sed -n 4p "$2" | b64d > "$W/global" 2>/dev/null || true
  [ "$(size "$W/key")" = 42 ] || die "MINISIGN_PUBKEY isn't a minisign public key"
  [ "$(size "$W/line")" = 74 ] && [ "$(size "$W/global")" = 64 ] || return 1
  # "Ed": Ed25519 over the file itself (minisign -l), which every verifier here can check
  [ "$(cut_bytes "$W/line" 0 2)" = Ed ] || { say "SHA256SUMS.minisig isn't in minisign's legacy format (minisign -l)"; return 1; }
  [ "$(cut_bytes "$W/line" 2 8 | hex)" = "$(cut_bytes "$W/key" 2 8 | hex)" ] || { say "SHA256SUMS is signed with another key"; return 1; }
  comment=$(sed -n 3p "$2")
  case "$comment" in 'trusted comment: '*) ;; *) return 1 ;; esac
  cut_bytes "$W/key" 10 32 > "$W/pk"
  cut_bytes "$W/line" 10 64 > "$W/sig"
  { cat "$W/sig"; printf '%s' "${comment#trusted comment: }"; } > "$W/signed-comment"

  if command -v minisign >/dev/null 2>&1; then
    minisign -V -q -P "$MINISIGN_PUBKEY" -m "$1" -x "$2"
  elif O=$(openssl3); then
    ed25519_openssl "$O" "$W/pk" "$W/sig" "$1" && ed25519_openssl "$O" "$W/pk" "$W/global" "$W/signed-comment"
  elif [ "$(uname -s)" = Darwin ]; then
    ed25519_jsc "$W/pk" "$W/sig" "$1" && ed25519_jsc "$W/pk" "$W/global" "$W/signed-comment"
  else
    die "can't check the signature on SHA256SUMS: install minisign or OpenSSL 3, then run this again"
  fi
}

# fetch_sums <SHA256SUMS url> <SHA256SUMS.minisig url, or empty>: downloads SHA256SUMS to $TMP and checks its signature,
# or for a release from before signing, that it's the SHA256SUMS that release was published with
fetch_sums() {
  curl -fsSL "$1" -o "$TMP/SHA256SUMS"
  if [ -n "$2" ]; then
    [ -n "$MINISIGN_PUBKEY" ] || die "this release's SHA256SUMS is signed, but this installer has no key to check it with: not installing"
    curl -fsSL "$2" -o "$TMP/SHA256SUMS.minisig"
    verify_sums "$TMP/SHA256SUMS" "$TMP/SHA256SUMS.minisig" || die "the signature on SHA256SUMS doesn't verify: not installing"
    # the (signed) trusted comment names the release it was made for: an older release's SHA256SUMS and signature,
    # served as a newer one, are refused
    comment=$(sed -n 3p "$TMP/SHA256SUMS.minisig")
    [ "$comment" = "trusted comment: powerrr $TAG" ] || die "SHA256SUMS was signed for '${comment#trusted comment: }', not 'powerrr $TAG': not installing"
    say "SHA256SUMS signature ok ($TAG)"
  else
    # without this, deleting the .minisig would turn any release back into an unsigned one
    pinned=$(legacy_sum "$TAG")
    [ -n "$pinned" ] || die "release $TAG has no SHA256SUMS.minisig, though releases are signed: not installing"
    [ "$(sha256 "$TMP/SHA256SUMS")" = "$pinned" ] || die "SHA256SUMS of $TAG isn't the one it was released with: not installing"
    say "note: unsigned checksums: $TAG is from before releases were signed; its SHA256SUMS matches the copy pinned in this installer"
  fi
}

# a Developer ID signed app must verify, come from APPLE_TEAM_ID and pass Gatekeeper (SIGNED=1). An ad-hoc signed or
# unsigned one installs as before (SIGNED=0), until APPLE_TEAM_ID is set: from then on every build must be signed
check_app() {
  SIGNED=0
  info=$(codesign -dvv "$1" 2>&1 || true)
  case "$info" in
    *'Authority=Developer ID Application:'*) ;;
    *) [ -z "$APPLE_TEAM_ID" ] || die "the app isn't signed with a Developer ID (this installer expects team $APPLE_TEAM_ID): not installing"; return 0 ;;
  esac
  codesign --verify --deep --strict "$1" >&2 || die "the app's code signature doesn't verify: not installing"
  team=$(printf '%s\n' "$info" | sed -n 's/^TeamIdentifier=//p')
  [ -n "$APPLE_TEAM_ID" ] || die "the app is signed by team $team, but this installer has no APPLE_TEAM_ID to check it against"
  [ "$team" = "$APPLE_TEAM_ID" ] || die "the app is signed by team $team, not $APPLE_TEAM_ID: not installing"
  spctl --assess --type execute "$1" 2>/dev/null || die "Gatekeeper rejects the app (spctl --assess): not installing"
  SIGNED=1
  say "code signature ok: Developer ID, team $team, accepted by Gatekeeper"
}

main() {
  REPO=${POWERRR_REPO:-highskore/powerrr}
  OS=$(uname -s)
  case "$OS" in
    Darwin) ;;
    Linux) [ "$(uname -m)" = x86_64 ] || die "there's only an x86_64 build for Linux" ;;
    *) die "this installs on macOS and Linux; on Windows: irm https://highskore.com/install.ps1 | iex" ;;
  esac

  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT

  API=${POWERRR_RELEASE:-https://api.github.com/repos/$REPO/releases/latest}
  say "finding the release at ${API}…"
  curl -fsSL "$API" -o "$TMP/release.json" || die "couldn't reach GitHub"
  asset() { grep -o "\"browser_download_url\": *\"[^\"]*$1\"" "$TMP/release.json" | head -n 1 | sed 's/.*"\(https*:[^"]*\)"/\1/'; }
  SIG_URL=$(asset '/SHA256SUMS\.minisig')
  TAG=$(grep -o '"tag_name": *"[^"]*"' "$TMP/release.json" | head -n 1 | sed 's/.*"\([^"]*\)"$/\1/')
  [ -n "$TAG" ] || die "the release has no tag name"

  if [ "$OS" = Linux ]; then
    IMG_URL=$(asset '_amd64\.AppImage')
    SUMS_URL=$(asset '/SHA256SUMS')
    [ -n "$IMG_URL" ] && [ -n "$SUMS_URL" ] || die "the latest release has no Linux AppImage or SHA256SUMS"
    IMG=$(basename "$IMG_URL")
    say "downloading ${IMG}…"
    curl -fsSL "$IMG_URL" -o "$TMP/$IMG"
    fetch_sums "$SUMS_URL" "$SIG_URL"
    WANT=$(awk -v f="$IMG" '$2 == f { print $1 }' "$TMP/SHA256SUMS")
    GOT=$(sha256sum "$TMP/$IMG" | awk '{ print $1 }')
    [ -n "$WANT" ] && [ "$WANT" = "$GOT" ] || die "sha256 mismatch for $IMG (expected ${WANT:-nothing}, got $GOT)"
    say "sha256 ok"

    DEST=${POWERRR_DIR:-$HOME/.local/bin}
    BIN="$DEST/powerrr"
    DATA=${XDG_DATA_HOME:-$HOME/.local/share}
    mkdir -p "$DEST" "$DATA/applications" "$DATA/icons/hicolor/128x128/apps"
    pkill -f "^$BIN" 2>/dev/null && sleep 1 || true
    install -m 755 "$TMP/$IMG" "$BIN"
    # the icon from the package just checked (extracting needs no FUSE)
    (cd "$TMP" && chmod +x "$IMG" && "./$IMG" --appimage-extract 'usr/share/icons/hicolor/128x128/apps/*' >/dev/null 2>&1) || true
    ICON=$(find "$TMP/squashfs-root/usr/share/icons/hicolor/128x128/apps" -name '*.png' 2>/dev/null | head -n 1)
    [ -n "$ICON" ] && cp "$ICON" "$DATA/icons/hicolor/128x128/apps/powerrr.png"
    cat > "$DATA/applications/powerrr.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Powerrr
Comment=The highskore planet game as a desktop widget
Exec=$BIN
Icon=powerrr
Categories=Game;
Terminal=false
DESKTOP
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" 2>/dev/null || true
    say "installed $BIN"

    case ":$PATH:" in *":$DEST:"*) ;; *) say "note: $DEST isn't on your PATH; the menu entry works anyway" ;; esac
    # AppImages mount themselves with FUSE 2, which newer distros (Ubuntu 22.04+) leave out
    if ! { ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; }; then
      say "note: AppImages need FUSE 2. if it won't start: sudo apt install libfuse2 (or libfuse2t64), or use the .deb from the release"
    fi
    if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
      nohup "$BIN" >/dev/null 2>&1 &
    fi
    say "it's in the tray, and Ctrl+Alt+P shows or hides it"
    return 0
  fi

  ZIP_URL=$(asset '_universal\.zip')
  SUMS_URL=$(asset '/SHA256SUMS')
  [ -n "$ZIP_URL" ] && [ -n "$SUMS_URL" ] || die "the latest release has no universal .zip or SHA256SUMS"
  ZIP=$(basename "$ZIP_URL")

  say "downloading ${ZIP}…"
  curl -fsSL "$ZIP_URL" -o "$TMP/$ZIP"
  fetch_sums "$SUMS_URL" "$SIG_URL"
  WANT=$(awk -v f="$ZIP" '$2 == f { print $1 }' "$TMP/SHA256SUMS")
  GOT=$(shasum -a 256 "$TMP/$ZIP" | awk '{ print $1 }')
  [ -n "$WANT" ] && [ "$WANT" = "$GOT" ] || die "sha256 mismatch for $ZIP (expected ${WANT:-nothing}, got $GOT)"
  say "sha256 ok"

  ditto -x -k "$TMP/$ZIP" "$TMP/app"
  APP=$(find "$TMP/app" -maxdepth 1 -name '*.app' | head -n 1)
  [ -n "$APP" ] || die "the zip has no app in it"
  NAME=$(basename "$APP")
  check_app "$APP"

  DEST=${POWERRR_DIR:-/Applications}
  [ -w "$DEST" ] || { DEST="$HOME/Applications"; mkdir -p "$DEST"; }
  pkill -f "$NAME/Contents/MacOS/" 2>/dev/null && sleep 1 || true
  rm -rf "$DEST/$NAME"
  ditto "$APP" "$DEST/$NAME"
  say "installed $DEST/$NAME"

  if [ "$SIGNED" = 0 ]; then
    say "note: this build isn't signed with a Developer ID or notarized. if macOS won't open it, right-click $NAME in $DEST → Open, once."
  fi
  open "$DEST/$NAME"
  say "it's in the menu bar, and ⌥⌘P shows or hides it"
}

# last, so a download cut short runs nothing
main "$@"
