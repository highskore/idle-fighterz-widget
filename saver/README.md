# Screensaver

The planet as a screensaver on macOS and Windows, playing **your run**: while the widget is open the screensaver mirrors it, frame by frame, and while it isn't the screensaver plays a fresh demo run of its own on the Rust core. The run is never written to disk for it: it comes over a local connection, in memory. The only file is the link's random key (see Security).

```
saver/link     saver-link: the frame protocol and the local link (the widget serves, the screensaver connects)
saver/src      the screensaver's side: the live frames or the demo (lib.rs), and the Windows .scr (scr.rs)
saver/macos    the .saver: a ScreenSaverView in Objective-C over the Rust staticlib, its build, and a test host
```

## How it works

- **The widget** (`src-tauri/src/planet.rs`) starts a `saver_link::Hub` at launch. While no screensaver is connected nothing changes: hidden, the widget draws nothing, as before. When one connects, the hub wakes the tick, which then draws every step at the game's 30 steps a second, hidden or not, and sends each frame with the power level (`short`, as in the menu bar), the world and how far into night it is. When the last one goes, the tick goes back to undrawn catch-ups once a second (or pauses). Shown, the widget already draws; the screensaver gets the same frames.
- **The protocol** (`saver/link/src/lib.rs`): after the handshake (below) the widget sends frames: kind, length, then width, height, encoding, the world and its night, the power level, and the pixels, raw RGBA or run-length encoded (a planet frame is mostly empty sky, so runs shrink it a lot). Everything is checked before use: a version or user mismatch refuses the connection; a length over `MAX_PAYLOAD` is refused before anything is allocated; sizes over 1024×1024, a power level over 32 bytes or not UTF-8, a raw frame of the wrong length and runs that over- or under-fill the frame are errors. Any error closes the connection and the screensaver plays its demo, retrying every 2 s.
- **Detaching**: the screensaver closes the connection when it stops, and also by itself once nothing has polled it for 2 s (so a host that never calls `stopAnimation` still lets go). The widget drops a screensaver whose write fails or has been stuck for 3 s, so a hung one can't keep it drawing.
- **The transport**: TCP on 127.0.0.1 on macOS (four ports from 41000 + 4×(uid mod 2000), below the ephemeral range); a named pipe `\\.\pipe\idle-fighterz-saver-<user SID>` on Windows, refusing remote clients, with a DACL for the user's SID alone, its first instance created with `FILE_FLAG_FIRST_PIPE_INSTANCE`.
- **The handshake**: at launch the widget writes 32 random bytes, a new key each time, where only the user can read them (macOS: `~/Library/Application Support/com.highskore.idlefighterz.saver/key`, 0600 in a 0700 folder; Windows: `%LOCALAPPDATA%\IdleFighterZ\saver.key`). Each side sends a fresh nonce and proves it holds the key with HMAC-SHA256 over both nonces and the port the screensaver dialled (constant-time check) before any frame moves. Both derive a session key, HMAC(key, "IFZS session" ‖ nonces ‖ port), and every message after that carries a tag under it over its sequence number and bytes, so a frame changed, replayed, reordered or injected by a relay ends the connection. The screensaver reads the key only if it's a regular 32-byte file it owns that no one else can read or write. Each half of the handshake has 2 s (TCP timeouts; on Windows, polling the pipe with `PeekNamedPipe`, since a synchronous pipe read can't time out). The key is the link's only file; the run itself is never written for it.

## Security

The threat: another local user, or any process, squatting the port or pipe to feed the screensaver fake frames, or connecting to read the widget's frames. A uid in the hello proves nothing, and loopback TCP says nothing about who is at the other end.

- **Why a key file.** The screensaver host's sandbox rules out the ways to learn the peer: no Unix socket outside its container (see Feasibility), so no `getpeereid`/`LOCAL_PEERCRED`; mapping a TCP connection to a pid needs `proc_pidinfo` on other processes, which the sandbox denies, and from the widget's side would only prove the client, not the server. The sandbox can read the user's files (its read-only exception for `/`), and file permissions are enforced by the kernel, so a key only the user can read, proven with a challenge-response and never sent, authenticates both ends.
- **What it stops.** A squatter on the port or pipe can't prove the key, so the screensaver refuses it and plays the demo. A relay that passes a genuine handshake through fails on the port in the proofs (it dialled the relay's port, the widget proved its own), and anything it changes or slips in afterwards fails the per-message tag. Windows gets the same tags (the code is shared), though its pipe already rules a relay out: only this user can create or open it, and the screensaver checks the server runs as this user. A process that can't read the key gets no frames. Another user can't create the pipe name (the widget owns it, and it's named for the user's SID) or open it (the DACL).
- **What it doesn't.** Processes running as the same user (or root) can read the key; they can read the user's screen too, so that's the trust boundary. Squatting all four ports or the pipe before the widget starts is a denial of service: the screensaver plays its demo. On Windows the screensaver also checks, before the handshake, that the pipe's server process (`GetNamedPipeServerProcessId`) runs as the same user SID; a widget started elevated can't be queried that way, and the screensaver then plays its demo.
- **Unchanged**: the version and size checks; any failure means the demo.
- **Drawing**: the screensaver puts the frame over the site's own backdrop (`saver/src/sky.rs`): the page's white sky, going to the site's night under a full moon, and the world's cloud sea scrolling along the bottom, behind and in front of the planet as planet-render's `drawSea` has it. It's composed in game pixels and scaled up nearest-neighbour by the largest whole factor that keeps the planet within 60% of the screen's height (6 at 2560×1600, 4 at 1920×1080; a small preview shrinks it to fit), with `power 1.2M` in plain text between the planet and the clouds. The widget still sends its bare frames (only two bytes more), so live and demo look the same. 30 fps at most.

## Feasibility

### macOS

- **The host.** Since Sonoma every third-party screensaver runs inside `legacyScreenSaver.appex`, an app-sandboxed host. Its entitlements on macOS 15.7 (`codesign -d --entitlements - /System/Library/Frameworks/ScreenSaver.framework/PlugIns/legacyScreenSaver.appex`): `app-sandbox`, `network.client`, `network.server`, read-only access to `/`, `files.user-selected.read-only`, `disable-library-validation`, and Mach lookups only for `com.apple.CARenderServer`, `com.apple.CoreDisplay.master`, `com.apple.nsurlstorage-cache` and `com.apple.ViewBridgeAuxiliary`.
- **What IPC that allows.**
  - TCP on loopback: yes, `network.client` grants it. That's what this uses.
  - Mach ports and XPC to the widget: no. Global Mach lookups are limited to the four Apple services above, so the screensaver can't look up a service of ours.
  - POSIX shared memory: no. The sandbox only allows `shm_open` names under the app's own app-group prefix, and the host has no app group of ours.
  - A Unix socket: not reliably. Connecting to one is a `network-outbound` to its path; the read-only exception on `/` doesn't grant that, and putting the socket in the host's container would mean the widget writing into another app's container (on Sonoma and later that raises a privacy prompt). Not tried in the host itself.
- **The bundle.** `saver/macos/build.sh` builds the Rust staticlib (`saver/src/lib.rs`, with planet-core and planet-render for the demo) for each arch, links it into a `-bundle` with `IdleFighterZView.m` (a `ScreenSaverView`), lipo's the arches together, writes `Info.plist` (`NSPrincipalClass`) and ad-hoc signs it. No Xcode project. The Rust side catches panics at the C boundary, so a bug fails a frame rather than crashing the host.
- **The host's bugs, worked around.**
  - `legacyScreenSaver` keeps running after the screensaver is dismissed, and old views can keep getting `animateOneFrame` unseen: the view listens for `com.apple.screensaver.willstop`, stops, and ends the host process (not in previews). It's what other screensavers do (Aerial among them).
  - `stopAnimation` sometimes never comes: the Rust side closes the link once nothing has polled it for 2 s, so the widget stops drawing for it either way, and the demo only steps when polled.
  - The view also tears down when it leaves its window.
- **Install, no admin.** Builds that carry the screensaver have **Install Screensaver…** in the tray menu. It copies `Idle FighterZ.app/Contents/Resources/Idle FighterZ.saver` to `~/Library/Screen Savers/` (replacing an older copy, via a temporary copy so a failed copy leaves the old one) and opens System Settings → Screen Saver, where you pick it. It never changes the selected screensaver itself.
- **Signing.** `build.sh` signs ad-hoc, which runs locally and in CI. A notarized release needs the `.saver` signed with the Developer ID and hardened runtime before Tauri bundles it, or notarization rejects the app; the release workflow doesn't do that yet, so release builds don't carry the screensaver yet (see below).

### Windows

- **The .scr** is `idle-fighterz-saver.exe` renamed (`saver/src/scr.rs`, plain Win32 and GDI through `windows-sys`): `/s` covers the virtual screen with a topmost window and centres the planet on the primary display, hides the pointer, and ends on a key, a click, a pointer move of more than 8 px or losing activation. `/p <hwnd>` (also `/p:<hwnd>`) draws into a child of the settings dialog's preview and ends when Windows destroys it. `/c` and no argument show a box: there are no settings.
- **IPC**: the named pipe above. A pipe read and a write on the same synchronous handle would serialize, so after the hellos only the widget writes; the screensaver detaches by closing.
- **Install, no admin.** **Install Screensaver…** copies the bundled `IdleFighterZ.scr` to `%LOCALAPPDATA%\IdleFighterZ\`, sets `HKCU\Control Panel\Desktop\SCRNSAVE.EXE` to it (its 8.3 path where the volume has them, as the screensaver launcher splits on spaces) and `ScreenSaveActive` to 1, tells Windows (`SPI_SETSCREENSAVEACTIVE`), and opens the screensaver settings to set the wait. This one does select it: that's what installing a screensaver means on Windows, and the dialog shows it. A group policy that pins a screensaver (`HKCU\Software\Policies\Microsoft\Windows\Control Panel\Desktop`) overrides it.
- **Untested here** beyond CI: built, unit-tested (including the pipe) and smoke-run (`/s`, and `/p` into a window of the job's own) on `windows-latest`. Not tried against a running widget or in the real settings dialog.

### Linux

Skipped. There's no common screensaver API: GNOME and KDE have none for third-party savers (they blank or lock), and XScreenSaver hacks only run under XScreenSaver on X11 (not under xsecurelock's default saver, nor on Wayland). A hack would be a small program drawing into `$XSCREENSAVER_WINDOW`, but few desktops run XScreenSaver now, so it isn't worth a third build. The widget doesn't start the link there.

## Build and try

```sh
cd saver
cargo test --workspace
macos/build.sh                                    # build/Idle FighterZ.saver (ARCHS="arm64 x86_64" for universal)
macos/try.sh "build/Idle FighterZ.saver" 20 /tmp/shots          # in a test host, off screen; PNGs every 5 s
SANDBOX=1 macos/try.sh "build/Idle FighterZ.saver" 20 /tmp/shots  # the same, under legacyScreenSaver's sandbox
cargo build --release --bin idle-fighterz-saver   # Windows: target/release/idle-fighterz-saver.exe → IdleFighterZ.scr
```

A test copy beside an installed one needs its own name, bundle id and class: `SAVER_NAME="Idle FighterZ Test" SAVER_ID=com.example.test.saver SAVER_CLASS=IdleFighterZTestView macos/build.sh`.

To bundle it with the app, build it first, then add its Tauri config:

```sh
saver/macos/build.sh && npx tauri build --config saver/tauri.macos.json        # macOS
npx tauri build --config saver/tauri.windows.json                              # Windows, after building the .scr into saver/build
```

The release workflow doesn't do this yet: on macOS the `.saver` has to be signed with the Developer ID first (see Signing), which is a change to the signed release path for a separate PR.
