// In-app updates from the GitHub release `releases/latest` (tags desktop-v*). They're checked as install.sh and
// install.ps1 check a release: SHA256SUMS.minisig must verify with the embedded minisign key, its trusted comment
// must name exactly the tag being installed (no replaying an older signed release), and the package's sha256 must be
// the one SHA256SUMS lists for it. Any failure installs nothing.
use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, Ordering}, Mutex},
    time::{Duration, SystemTime},
};
use tauri::{AppHandle, Manager};

const REPO: &str = "highskore/idle-fighterz-widget";
// the minisign public key that signs SHA256SUMS: the same as install.sh's and install.ps1's
const MINISIGN_PUBKEY: &str = "RWQsYREKR+Av/TIhq7yrFboAERzsd4eoUxepZrikLPg8/hrtXm7MvY1C";
// the Apple team ID of the Developer ID certificate that signs the macOS app; empty until builds are (as install.sh)
#[cfg(target_os = "macos")]
const APPLE_TEAM_ID: &str = "";
const TAG_PREFIX: &str = "desktop-v";
const MAX_JSON: u64 = 2 << 20;
const MAX_SUMS: u64 = 64 << 10;
const MAX_PACKAGE: u64 = 512 << 20;

/// where releases come from and the key that signs them
pub struct Source {
    pub api: String,
    pub pubkey: String,
    pub https_only: bool,
}

impl Source {
    pub fn get() -> Source {
        // debug builds only: a stand-in release and its throwaway key, for trying the whole flow locally
        #[cfg(debug_assertions)]
        if let (Ok(api), Ok(pubkey)) = (std::env::var("IDLE_FIGHTERZ_UPDATE_RELEASE"), std::env::var("IDLE_FIGHTERZ_UPDATE_PUBKEY")) {
            eprintln!("update: the stand-in release at {api} (IDLE_FIGHTERZ_UPDATE_RELEASE, debug builds only)");
            return Source { api, pubkey, https_only: false };
        }
        Source { api: format!("https://api.github.com/repos/{REPO}/releases/latest"), pubkey: MINISIGN_PUBKEY.into(), https_only: true }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub assets: Vec<Asset>,
}

/// desktop-vX.Y.Z's version: a plain release, never a pre-release or one with build metadata
pub fn tag_version(tag: &str) -> Option<Version> {
    let v = Version::parse(tag.strip_prefix(TAG_PREFIX)?).ok()?;
    (v.pre.is_empty() && v.build.is_empty()).then_some(v)
}

/// whether `release` is strictly newer than `current` by semver: never a downgrade or the same version again
pub fn is_newer(current: &str, release: &Version) -> bool {
    Version::parse(current).is_ok_and(|c| *release > c)
}

pub fn parse_release(json: &[u8]) -> Result<Release, String> {
    let v: Value = serde_json::from_slice(json).map_err(|e| format!("GitHub's answer isn't a release ({e})"))?;
    if v["draft"].as_bool() != Some(false) || v["prerelease"].as_bool() != Some(false) {
        return Err("the latest release is a draft or a pre-release".into());
    }
    let tag = v["tag_name"].as_str().ok_or("the release has no tag name")?.to_string();
    let version = tag_version(&tag).ok_or_else(|| format!("{tag} isn't a desktop-vX.Y.Z release"))?;
    let assets = v["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|a| Some(Asset { name: a["name"].as_str()?.into(), url: a["browser_download_url"].as_str()?.into() }))
                .collect()
        })
        .unwrap_or_default();
    Ok(Release { tag, version, assets })
}

/// what this platform installs from a release, by the end of its asset name: the universal .zip on macOS, the per-user
/// NSIS installer on Windows, the AppImage on Linux when running as one. None: the release page instead
pub fn package_suffix(os: &str, arch: &str, appimage: bool) -> Option<&'static str> {
    match (os, arch) {
        ("macos", _) => Some("_universal.zip"),
        ("windows", "x86_64") => Some("_x64-setup.exe"),
        ("linux", "x86_64") if appimage => Some("_amd64.AppImage"),
        _ => None,
    }
}

// an asset name becomes a file name here: nothing that could leave the download folder
fn plain_name(n: &str) -> bool {
    !n.is_empty() && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

pub fn pick<'a>(assets: &'a [Asset], suffix: &str) -> Option<&'a Asset> {
    assets.iter().find(|a| a.name.ends_with(suffix) && plain_name(&a.name))
}

fn named<'a>(assets: &'a [Asset], name: &str) -> Option<&'a Asset> {
    assets.iter().find(|a| a.name == name)
}

// lines split on LF or CRLF; a bare CR or a NUL anywhere is refused, so every reader sees the same lines
fn lines(b: &[u8], what: &str) -> Result<Vec<String>, String> {
    let text = std::str::from_utf8(b).map_err(|_| format!("{what} is malformed"))?.replace("\r\n", "\n");
    if text.contains(['\r', '\0']) {
        return Err(format!("{what} is malformed"));
    }
    Ok(text.strip_suffix('\n').unwrap_or(&text).split('\n').map(String::from).collect())
}

/// checks SHA256SUMS.minisig: four lines, by `pubkey`, over `sums`, with the trusted comment "idle-fighterz <tag>"
pub fn verify_sums(sums: &[u8], minisig: &[u8], pubkey: &str, tag: &str) -> Result<(), String> {
    use minisign_verify::{Error, PublicKey, Signature};
    let l = lines(minisig, "SHA256SUMS.minisig")?;
    if l.len() != 4 || !l[2].starts_with("trusted comment: ") {
        return Err("SHA256SUMS.minisig is malformed".into());
    }
    let key = PublicKey::from_base64(pubkey).map_err(|_| "the update key isn't a minisign public key".to_string())?;
    let sig = Signature::decode(&l.join("\n")).map_err(|_| "SHA256SUMS.minisig is malformed".to_string())?;
    key.verify(sums, &sig, true).map_err(|e| match e {
        Error::UnexpectedKeyId => "SHA256SUMS is signed with another key".to_string(),
        _ => "the signature on SHA256SUMS doesn't verify".to_string(),
    })?;
    // the trusted comment, just verified, names the release it was signed for
    let want = format!("idle-fighterz {tag}");
    if sig.trusted_comment() != want {
        return Err(format!("SHA256SUMS was signed for '{}', not '{want}'", sig.trusted_comment()));
    }
    Ok(())
}

/// the sha256 SHA256SUMS lists for `name` (sha256sum's "<hash>  <name>" or "<hash> *<name>"), lower case. No entry, or
/// more than one, is an error
pub fn listed_sha256(sums: &[u8], name: &str) -> Result<String, String> {
    let mut found = Vec::new();
    for line in lines(sums, "SHA256SUMS")? {
        let Some((hash, rest)) = line.split_once([' ', '\t']) else { continue };
        let file = rest.trim_start_matches([' ', '\t']);
        if file.strip_prefix('*').unwrap_or(file) == name {
            found.push(hash.to_ascii_lowercase());
        }
    }
    match found.as_slice() {
        [h] if h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()) => Ok(h.clone()),
        [_] => Err(format!("SHA256SUMS's entry for {name} isn't a sha256")),
        [] => Err(format!("SHA256SUMS has no entry for {name}")),
        _ => Err(format!("SHA256SUMS lists {name} more than once")),
    }
}

fn client(src: &Source, timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("idle-fighterz-updater/", env!("CARGO_PKG_VERSION")))
        .https_only(src.https_only)
        .connect_timeout(Duration::from_secs(30))
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

async fn get(c: &reqwest::Client, url: &str, max: u64, mut sink: impl FnMut(&[u8]) -> Result<(), String>) -> Result<(), String> {
    let mut r = c.get(url).header("Accept", "application/vnd.github+json, */*").send().await.map_err(|e| format!("couldn't reach {url} ({e})"))?;
    if !r.status().is_success() {
        return Err(format!("{url} answered {}", r.status()));
    }
    let mut n = 0u64;
    while let Some(chunk) = r.chunk().await.map_err(|e| format!("the download from {url} broke off ({e})"))? {
        n += chunk.len() as u64;
        if n > max {
            return Err(format!("{url} is bigger than expected"));
        }
        sink(&chunk)?;
    }
    Ok(())
}

async fn get_bytes(c: &reqwest::Client, url: &str, max: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    get(c, url, max, |b| {
        out.extend_from_slice(b);
        Ok(())
    })
    .await?;
    Ok(out)
}

/// the latest release, if it's strictly newer than `current`
pub async fn check(src: &Source, current: &str) -> Result<Option<Release>, String> {
    let c = client(src, Duration::from_secs(60))?;
    let release = parse_release(&get_bytes(&c, &src.api, MAX_JSON).await?)?;
    Ok(is_newer(current, &release.version).then_some(release))
}

/// downloads the package ending in `suffix` into `dir` and checks it: SHA256SUMS signed for this release's tag, then
/// the package's sha256 against it. Returns the checked file; on any error nothing usable is left behind
pub async fn fetch(src: &Source, release: &Release, suffix: &str, dir: &Path) -> Result<PathBuf, String> {
    let tag = &release.tag;
    let pkg = pick(&release.assets, suffix).ok_or_else(|| format!("release {tag} has no *{suffix}"))?;
    let sums = named(&release.assets, "SHA256SUMS").ok_or_else(|| format!("release {tag} has no SHA256SUMS"))?;
    // every release is signed: deleting the .minisig must not turn one back into an unsigned one
    let sig = named(&release.assets, "SHA256SUMS.minisig").ok_or_else(|| format!("release {tag} has no SHA256SUMS.minisig, though releases are signed"))?;
    let c = client(src, Duration::from_secs(60))?;
    let sums = get_bytes(&c, &sums.url, MAX_SUMS).await?;
    let sig = get_bytes(&c, &sig.url, MAX_SUMS).await?;
    verify_sums(&sums, &sig, &src.pubkey, tag)?;
    let want = listed_sha256(&sums, &pkg.name)?;

    use std::io::Write;
    let path = dir.join(&pkg.name);
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("couldn't save the download ({e})"))?;
    let mut hash = Sha256::new();
    let c = client(src, Duration::from_secs(30 * 60))?;
    let done = get(&c, &pkg.url, MAX_PACKAGE, |b| {
        hash.update(b);
        file.write_all(b).map_err(|e| format!("couldn't save the download ({e})"))
    })
    .await
    .and_then(|_| file.sync_all().map_err(|e| e.to_string()));
    drop(file);
    let got = hex(&hash.finalize());
    if let Err(e) = done {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    if got != want {
        let _ = std::fs::remove_file(&path);
        return Err(format!("sha256 mismatch for {} (expected {want}, got {got})", pkg.name));
    }
    Ok(path)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// a new, empty folder only this user can read, for one update's downloads
fn work_dir() -> Result<PathBuf, String> {
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("idle-fighterz-update-{}-{nanos}", std::process::id()));
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&dir).map_err(|e| format!("couldn't make a download folder ({e})"))?;
    Ok(dir)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".idle-fighterz-update-probe-{}", std::process::id()));
    let ok = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// how this copy of the app updates itself
#[derive(Debug)]
enum How {
    #[cfg(target_os = "macos")]
    Bundle(PathBuf), // replace the .app in place
    #[cfg(target_os = "windows")]
    Setup, // run the per-user installer
    #[cfg(target_os = "linux")]
    AppImage(PathBuf), // swap the AppImage file
    Page, // can't here: the release page
}

fn how() -> How {
    #[cfg(target_os = "macos")]
    {
        // …/Name.app/Contents/MacOS/binary; a bare binary (tauri dev) has no bundle to replace
        let exe = std::env::current_exe().and_then(|p| p.canonicalize()).ok();
        let bundle = exe.as_deref().and_then(|e| e.parent()?.parent()?.parent()).filter(|b| b.extension().is_some_and(|x| x == "app"));
        match bundle {
            Some(b) if b.parent().is_some_and(writable) => How::Bundle(b.to_path_buf()),
            _ => How::Page,
        }
    }
    #[cfg(target_os = "windows")]
    {
        How::Setup
    }
    #[cfg(target_os = "linux")]
    {
        // the AppImage runtime sets $APPIMAGE to the file it runs from; a .deb install has none
        match std::env::var_os("APPIMAGE").map(PathBuf::from) {
            Some(p) if p.is_absolute() && p.is_file() && p.parent().is_some_and(writable) && package_suffix("linux", std::env::consts::ARCH, true).is_some() => How::AppImage(p),
            _ => How::Page,
        }
    }
}

fn suffix_for(how: &How) -> Option<&'static str> {
    package_suffix(std::env::consts::OS, std::env::consts::ARCH, !matches!(how, How::Page))
}

fn release_page(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/{tag}")
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(url).spawn();
}

fn run(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<std::process::Output, String> {
    std::process::Command::new(cmd).args(args).output().map_err(|e| format!("couldn't run {cmd} ({e})"))
}

/// the checked app in an unpacked .zip must be signed as install.sh's check_app wants: a Developer ID signature must
/// verify, come from `team` and pass Gatekeeper, and with a team set every build must be signed so
#[cfg(target_os = "macos")]
fn check_app(app: &Path, team: &str) -> Result<(), String> {
    let out = run("codesign", &["-dvv".as_ref(), app.as_os_str()])?;
    let info = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    if !info.contains("Authority=Developer ID Application:") {
        return if team.is_empty() { Ok(()) } else { Err(format!("the app isn't signed with a Developer ID (expected team {team})")) };
    }
    if !run("codesign", &["--verify".as_ref(), "--deep".as_ref(), "--strict".as_ref(), app.as_os_str()])?.status.success() {
        return Err("the app's code signature doesn't verify".into());
    }
    let signer = info.lines().find_map(|l| l.strip_prefix("TeamIdentifier=")).unwrap_or("").to_string();
    if team.is_empty() {
        return Err(format!("the app is signed by team {signer}, but this build has no Apple team ID to check it against"));
    }
    if signer != team {
        return Err(format!("the app is signed by team {signer}, not {team}"));
    }
    if !run("spctl", &["--assess".as_ref(), "--type".as_ref(), "execute".as_ref(), app.as_os_str()])?.status.success() {
        return Err("Gatekeeper rejects the app".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn plist(app: &Path, key: &str) -> String {
    let info = app.join("Contents/Info.plist");
    run("/usr/libexec/PlistBuddy", &["-c".as_ref(), format!("Print :{key}").as_ref(), info.as_os_str()])
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// unpacks the checked .zip, checks the app in it is this app at `version`, and swaps it in for `bundle`
#[cfg(target_os = "macos")]
fn install_bundle(zip: &Path, bundle: &Path, identifier: &str, version: &Version, work: &Path) -> Result<(), String> {
    let out = work.join("app");
    if !run("ditto", &["-x".as_ref(), "-k".as_ref(), zip.as_os_str(), out.as_os_str()])?.status.success() {
        return Err("couldn't unpack the update".into());
    }
    let apps: Vec<PathBuf> = std::fs::read_dir(&out).map_err(|e| e.to_string())?.filter_map(|e| Some(e.ok()?.path())).filter(|p| p.extension().is_some_and(|x| x == "app")).collect();
    let [app] = apps.as_slice() else { return Err("the update's .zip doesn't hold exactly one app".into()) };
    let (id, v) = (plist(app, "CFBundleIdentifier"), plist(app, "CFBundleShortVersionString"));
    if id != identifier || v != version.to_string() {
        return Err(format!("the update holds {id} {v}, not {identifier} {version}"));
    }
    check_app(app, APPLE_TEAM_ID)?;
    // not notarized: a downloaded app carries no quarantine from here (nor does install.sh's curl), and none it might
    // have picked up stays, so Gatekeeper doesn't stop the relaunch. The bytes were checked above
    let _ = run("xattr", &["-dr".as_ref(), "com.apple.quarantine".as_ref(), app.as_os_str()]);
    // copied next to the old app first (ditto keeps the signature's extended attributes), so the swap is two renames
    let parent = bundle.parent().ok_or("the app has no folder")?;
    let name = bundle.file_name().ok_or("the app has no name")?.to_string_lossy().to_string();
    let staged = parent.join(format!(".{name}.update-{}", std::process::id()));
    if !run("ditto", &[app.as_os_str(), staged.as_os_str()])?.status.success() {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("couldn't copy the update into {}", parent.display()));
    }
    swap(&staged, bundle)
}

/// puts `staged` where `dest` is, keeping `dest` until that worked and putting it back if it didn't
#[cfg(unix)]
fn swap(staged: &Path, dest: &Path) -> Result<(), String> {
    let old = dest.with_file_name(format!(".{}.old-{}", dest.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    let fail = |e: std::io::Error| {
        let _ = if staged.is_dir() { std::fs::remove_dir_all(staged) } else { std::fs::remove_file(staged) };
        format!("couldn't replace {} ({e})", dest.display())
    };
    std::fs::rename(dest, &old).map_err(fail)?;
    if let Err(e) = std::fs::rename(staged, dest) {
        let _ = std::fs::rename(&old, dest);
        return Err(fail(e));
    }
    let _ = if old.is_dir() { std::fs::remove_dir_all(&old) } else { std::fs::remove_file(&old) };
    Ok(())
}

/// copies the checked AppImage next to the running one and swaps it in
#[cfg(target_os = "linux")]
fn install_appimage(file: &Path, dest: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let staged = dest.with_file_name(format!(".{}.update-{}", dest.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    std::fs::copy(file, &staged)
        .and_then(|_| std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)))
        .and_then(|_| std::fs::File::open(&staged)?.sync_all())
        .map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            format!("couldn't copy the update next to {} ({e})", dest.display())
        })?;
    swap(&staged, dest)
}

/// starts `target` once this process has exited (a second copy would only hand over to this one)
#[cfg(unix)]
fn relaunch_after_exit(open: &str, target: &Path) -> Result<(), String> {
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"while kill -0 "$0" 2>/dev/null; do sleep 0.2; done; exec "$1" "$2""#)
        .arg(std::process::id().to_string())
        .arg(open)
        .arg(target)
        .spawn()
        .map(drop)
        .map_err(|e| format!("couldn't relaunch ({e})"))
}

/// the update's tray item and the found release; one check or install at a time
#[derive(Default)]
pub struct Updater {
    found: Mutex<Option<Release>>,
    told: Mutex<Option<Version>>, // the version the automatic check last announced
    busy: AtomicBool,
    last: Mutex<Option<SystemTime>>,
}

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// checks on launch and then once a day while the setting is on (manual checks always work)
pub fn start(app: &AppHandle) {
    app.manage(Updater::default());
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        loop {
            let on = app.state::<crate::State>().settings.lock().unwrap()["update_check"].as_bool().unwrap_or(true);
            let due = app.state::<Updater>().last.lock().unwrap().is_none_or(|t| !t.elapsed().is_ok_and(|e| e < DAY));
            if on && due {
                check_now(&app, false);
            }
            // wall-clock, a nap at a time: a sleeping laptop doesn't stretch the day
            std::thread::sleep(Duration::from_secs(60 * 60));
        }
    });
}

fn current(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// a toast in the widget while it shows the HUD, else a notification
fn notice(app: &AppHandle, title: &str, body: &str) {
    #[cfg(feature = "rust-core")]
    if app.state::<crate::Shown>().get() && app.try_state::<crate::planet::Core>().is_some() {
        use tauri::Emitter;
        let _ = app.emit_to("main", "notice", if body.is_empty() { title.to_string() } else { format!("{title}. {body}") });
        return;
    }
    crate::notify(app.clone(), title.into(), body.into());
}

fn tray_items(app: &AppHandle) -> Option<crate::Tray> {
    app.state::<crate::State>().tray.lock().unwrap().clone()
}

// "Update to X.Y.Z" just above "Check for Updates…", or gone
fn show_item(app: &AppHandle, version: Option<&Version>) {
    let Some(t) = tray_items(app) else { return };
    let listed = t.menu.get(t.update.id()).is_some();
    if let Some(v) = version {
        let _ = t.update.set_text(format!("Update to {v}"));
        let _ = t.update.set_enabled(true);
        if !listed {
            let at = t.menu.items().ok().and_then(|i| i.iter().position(|i| i.id() == t.check.id())).unwrap_or(0);
            let _ = t.menu.insert(&t.update, at);
        }
    } else if listed {
        let _ = t.menu.remove(&t.update);
    }
}

/// asks GitHub now. `manual` (the tray item) always reports what it found
pub fn check_now(app: &AppHandle, manual: bool) {
    let u = app.state::<Updater>();
    if u.busy.swap(true, Ordering::SeqCst) {
        if manual {
            notice(app, "Already checking for updates", "");
        }
        return;
    }
    let result = tauri::async_runtime::block_on(check(&Source::get(), &current(app)));
    *u.last.lock().unwrap() = Some(SystemTime::now());
    match result {
        Ok(Some(r)) => {
            let v = r.version.clone();
            *u.found.lock().unwrap() = Some(r);
            show_item(app, Some(&v));
            let mut told = u.told.lock().unwrap();
            if manual || told.as_ref() != Some(&v) {
                *told = Some(v.clone());
                drop(told);
                let place = if cfg!(target_os = "macos") { "the menu bar" } else { "the tray" };
                notice(app, &format!("{} {v} is out", app.package_info().name), &format!("Update to {v} from {place}"));
            }
        }
        Ok(None) => {
            *u.found.lock().unwrap() = None;
            show_item(app, None);
            if manual {
                notice(app, "You're on the latest version", &format!("{} {}", app.package_info().name, current(app)));
            }
        }
        Err(e) => {
            eprintln!("update check: {e}");
            if manual {
                notice(app, "Couldn't check for updates", &e);
            }
        }
    }
    u.busy.store(false, Ordering::SeqCst);
}

/// the tray's "Update to X.Y.Z": download, verify, install, restart, or the release page where this copy can't update
/// itself. Fails closed: on any error nothing is installed and it says why
pub fn install_now(app: &AppHandle) {
    let u = app.state::<Updater>();
    let Some(release) = u.found.lock().unwrap().clone() else { return };
    if u.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let how = how();
    let suffix = match (&how, suffix_for(&how)) {
        (How::Page, _) | (_, None) => {
            open_url(&release_page(&release.tag));
            u.busy.store(false, Ordering::SeqCst);
            return;
        }
        (_, Some(s)) => s,
    };
    if let Some(t) = tray_items(app) {
        let _ = t.update.set_text(format!("Updating to {}…", release.version));
        let _ = t.update.set_enabled(false);
    }
    match install(app, &release, &how, suffix) {
        Ok(()) => crate::quit(app.clone()),
        Err(e) => {
            eprintln!("update: {e}");
            notice(app, "Update failed: nothing was installed", &e);
            show_item(app, Some(&release.version));
            u.busy.store(false, Ordering::SeqCst);
        }
    }
}

// everything is checked before anything is replaced; Ok means the app should exit now
fn install(app: &AppHandle, release: &Release, how: &How, suffix: &str) -> Result<(), String> {
    let work = work_dir()?;
    let file = tauri::async_runtime::block_on(fetch(&Source::get(), release, suffix, &work));
    let done = file.and_then(|file| match how {
        #[cfg(target_os = "macos")]
        How::Bundle(bundle) => {
            install_bundle(&file, bundle, &app.config().identifier, &release.version, &work)?;
            relaunch_after_exit("open", bundle)
        }
        #[cfg(target_os = "windows")]
        How::Setup => {
            // the per-user NSIS installer: passive (a progress bar, no questions); it closes this app if it's still
            // running and starts the new one when done (/R). The file stays for it to run from
            let _ = app;
            std::process::Command::new(&file).args(["/P", "/R", "/UPDATE"]).spawn().map(drop).map_err(|e| format!("couldn't start the installer ({e})"))
        }
        #[cfg(target_os = "linux")]
        How::AppImage(dest) => {
            let _ = app;
            install_appimage(&file, dest)?;
            relaunch_after_exit("env", dest)
        }
        How::Page => Err("this copy can't update itself".into()),
    });
    if cfg!(not(target_os = "windows")) || done.is_err() {
        let _ = std::fs::remove_dir_all(&work);
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMS: &[u8] = include_bytes!("../testdata/update/SHA256SUMS");
    const SIG: &[u8] = include_bytes!("../testdata/update/SHA256SUMS.minisig");
    const TAG: &str = "desktop-v9.9.9";
    fn key(f: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/update").join(f)).unwrap().trim().to_string()
    }
    fn fixture(f: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/update").join(f)).unwrap()
    }

    #[test]
    fn only_strictly_newer_plain_releases_are_offered() {
        let v = |t: &str| tag_version(t);
        assert_eq!(v("desktop-v0.2.2"), Some(Version::new(0, 2, 2)));
        assert_eq!(v("desktop-v0.3.0-beta.1"), None, "a pre-release");
        assert_eq!(v("desktop-v0.3.0+build"), None, "build metadata");
        assert_eq!(v("v0.3.0"), None, "another tag scheme");
        assert_eq!(v("desktop-v0.3"), None);
        assert_eq!(v("desktop-v01.2.3"), None, "a leading zero");
        assert!(is_newer("0.2.1", &Version::new(0, 2, 2)));
        assert!(is_newer("0.2.1", &Version::new(0, 10, 0)), "numeric, not text, order");
        assert!(is_newer("0.9.9", &Version::new(1, 0, 0)));
        assert!(!is_newer("0.2.1", &Version::new(0, 2, 1)), "the same version");
        assert!(!is_newer("0.2.1", &Version::new(0, 2, 0)), "a downgrade");
        assert!(!is_newer("0.10.0", &Version::new(0, 9, 9)), "a downgrade");
        assert!(!is_newer("not a version", &Version::new(9, 9, 9)));
    }

    #[test]
    fn a_release_must_be_a_published_desktop_release() {
        let json = |tag: &str, pre: bool, draft: bool| {
            serde_json::to_vec(&serde_json::json!({ "tag_name": tag, "prerelease": pre, "draft": draft, "assets": [
                { "name": "SHA256SUMS", "browser_download_url": "https://x/SHA256SUMS" }, { "name": "broken" }
            ] }))
            .unwrap()
        };
        let r = parse_release(&json("desktop-v0.2.2", false, false)).unwrap();
        assert_eq!((r.tag.as_str(), r.version.clone()), ("desktop-v0.2.2", Version::new(0, 2, 2)));
        assert_eq!(r.assets, vec![Asset { name: "SHA256SUMS".into(), url: "https://x/SHA256SUMS".into() }]);
        assert!(parse_release(&json("desktop-v0.2.2", true, false)).is_err());
        assert!(parse_release(&json("desktop-v0.2.2", false, true)).is_err());
        assert!(parse_release(&json("desktop-v0.3.0-rc.1", false, false)).is_err());
        assert!(parse_release(&json("site-v1.0.0", false, false)).is_err());
        assert!(parse_release(br#"{"tag_name": "desktop-v0.2.2"}"#).is_err(), "no draft/prerelease flags");
        assert!(parse_release(b"<html>").is_err());
    }

    #[test]
    fn each_platform_picks_its_own_package() {
        let assets: Vec<Asset> = ["IdleFighterZ_0.2.2_amd64.AppImage", "IdleFighterZ_0.2.2_amd64.deb", "IdleFighterZ_0.2.2_universal.dmg", "IdleFighterZ_0.2.2_universal.zip", "IdleFighterZ_0.2.2_x64-setup.exe", "IdleFighterZ_0.2.2_x64_en-US.msi", "SHA256SUMS", "SHA256SUMS.minisig"]
            .iter()
            .map(|n| Asset { name: n.to_string(), url: format!("https://x/{n}") })
            .collect();
        let pick_for = |os, arch, appimage| package_suffix(os, arch, appimage).and_then(|s| pick(&assets, s)).map(|a| a.name.as_str());
        assert_eq!(pick_for("macos", "aarch64", false), Some("IdleFighterZ_0.2.2_universal.zip"));
        assert_eq!(pick_for("macos", "x86_64", false), Some("IdleFighterZ_0.2.2_universal.zip"));
        assert_eq!(pick_for("windows", "x86_64", false), Some("IdleFighterZ_0.2.2_x64-setup.exe"));
        assert_eq!(pick_for("linux", "x86_64", true), Some("IdleFighterZ_0.2.2_amd64.AppImage"));
        assert_eq!(pick_for("linux", "x86_64", false), None, "a .deb install gets the release page");
        assert_eq!(pick_for("linux", "aarch64", true), None, "there's only an x86_64 build");
        assert_eq!(pick_for("windows", "aarch64", false), None);
        // a name that would leave the download folder is never picked
        let bad = vec![Asset { name: "../x_universal.zip".into(), url: "u".into() }, Asset { name: ".x_universal.zip".into(), url: "u".into() }];
        assert_eq!(pick(&bad, "_universal.zip"), None);
    }

    #[test]
    fn sha256sums_parsing() {
        let n = "IdleFighterZ_9.9.9_universal.zip";
        let want = "a3022c7e8e73e9199eb034e4cbda98d3fc97e34957b2387dabea83c3aa985622";
        assert_eq!(listed_sha256(SUMS, n).unwrap(), want);
        let h = want.to_uppercase();
        assert_eq!(listed_sha256(format!("{h} *{n}\n").as_bytes(), n).unwrap(), want, "binary-mode entry, upper case");
        assert_eq!(listed_sha256(format!("{h}  {n}\r\nffff  other\r\n").as_bytes(), n).unwrap(), want, "CRLF");
        assert_eq!(listed_sha256(format!("{h}  {n}").as_bytes(), n).unwrap(), want, "no final newline");
        // a bare CR could hide a second entry from one reader and not another
        assert!(listed_sha256(format!("{h}  {n}\r{h}  x\n").as_bytes(), n).unwrap_err().contains("malformed"));
        assert!(listed_sha256(format!("{h}  {n}\0\n").as_bytes(), n).unwrap_err().contains("malformed"));
        assert!(listed_sha256(format!("{h}  {n}\n{h}  {n}\n").as_bytes(), n).unwrap_err().contains("more than once"));
        assert!(listed_sha256(format!("{h}  {n}\n{}  {n}\n", "0".repeat(64)).as_bytes(), n).unwrap_err().contains("more than once"));
        assert!(listed_sha256(format!("{h}  {n}.sig\n{h}  x{n}\n").as_bytes(), n).unwrap_err().contains("no entry"), "exact names only");
        assert!(listed_sha256(format!("abc  {n}\n").as_bytes(), n).unwrap_err().contains("isn't a sha256"));
        assert!(listed_sha256(b"", n).is_err());
    }

    #[test]
    fn signature_verification() {
        let k = key("test.pub");
        assert_eq!(verify_sums(SUMS, SIG, &k, TAG), Ok(()));
        // minisign's default (prehashed) format verifies too
        assert_eq!(verify_sums(SUMS, &fixture("prehashed.minisig"), &k, TAG), Ok(()));
        // CRLF line endings are the same four lines
        let crlf = String::from_utf8(SIG.to_vec()).unwrap().replace('\n', "\r\n");
        assert_eq!(verify_sums(SUMS, crlf.as_bytes(), &k, TAG), Ok(()));

        let err = |sums: &[u8], sig: &[u8], key: &str, tag: &str| verify_sums(sums, sig, key, tag).unwrap_err();
        assert!(err(SUMS, SIG, &key("other.pub"), TAG).contains("another key"), "the wrong key");
        assert!(err(SUMS, &fixture("other-key.minisig"), &k, TAG).contains("another key"), "signed by another key");
        let mut tampered = SUMS.to_vec();
        tampered[0] ^= 1;
        assert!(err(&tampered, SIG, &k, TAG).contains("doesn't verify"), "a SHA256SUMS changed after signing");
        // the trusted comment is signed too: changing it breaks the signature
        let relabeled = String::from_utf8(SIG.to_vec()).unwrap().replace(TAG, "desktop-v9.9.8");
        assert!(err(SUMS, relabeled.as_bytes(), &k, "desktop-v9.9.8").contains("doesn't verify"));
        // an older release's signed SHA256SUMS, served as this one: a good signature, for another tag
        assert_eq!(err(SUMS, &fixture("other-tag.minisig"), &k, TAG), "SHA256SUMS was signed for 'idle-fighterz desktop-v0.2.0', not 'idle-fighterz desktop-v9.9.9'");
        assert!(err(SUMS, SIG, &k, "desktop-v9.9.99").contains("not 'idle-fighterz desktop-v9.9.99'"), "exactly the tag");
        // a second trusted comment after a bare CR, as install.sh's crtrick test
        let crtrick = String::from_utf8(SIG.to_vec()).unwrap().replace(&format!("idle-fighterz {TAG}\n"), &format!("idle-fighterz {TAG}\rtrusted comment: idle-fighterz desktop-v10.0.0\n"));
        assert!(err(SUMS, crtrick.as_bytes(), &k, "desktop-v10.0.0").contains("malformed"));
        assert!(verify_sums(SUMS, &SIG[..SIG.len() - 20], &k, TAG).is_err(), "a cut-off signature");
        assert!(err(SUMS, format!("{}\nextra\n", String::from_utf8_lossy(SIG).trim_end()).as_bytes(), &k, TAG).contains("malformed"), "five lines");
        assert!(err(SUMS, b"", &k, TAG).contains("malformed"));
        assert!(err(SUMS, SIG, "not a key", TAG).contains("isn't a minisign public key"));
    }

    #[test]
    fn the_shipped_key_is_a_minisign_key() {
        assert!(minisign_verify::PublicKey::from_base64(MINISIGN_PUBKEY).is_ok());
        assert_eq!(Source::get().pubkey, MINISIGN_PUBKEY);
    }

    #[cfg(unix)]
    #[test]
    fn a_swap_replaces_or_leaves_the_old_copy() {
        let dir = work_dir().unwrap();
        let (dest, staged) = (dir.join("Idle FighterZ.app"), dir.join(".staged"));
        std::fs::create_dir_all(dest.join("Contents")).unwrap();
        std::fs::write(dest.join("Contents/v"), "old").unwrap();
        std::fs::create_dir_all(staged.join("Contents")).unwrap();
        std::fs::write(staged.join("Contents/v"), "new").unwrap();
        swap(&staged, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("Contents/v")).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "nothing left beside it");
        // nothing staged: the old copy stays where it was
        assert!(swap(&dir.join("missing"), &dest).is_err());
        assert_eq!(std::fs::read_to_string(dest.join("Contents/v")).unwrap(), "new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // a stand-in GitHub on localhost: release JSON, SHA256SUMS, its signature and the packages
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        use std::io::{BufRead, BufReader, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut first = String::new();
                let _ = r.read_line(&mut first);
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let path = first.split(' ').nth(1).unwrap_or("").to_string();
                let mut s = s;
                match files.iter().find(|(p, _)| *p == path) {
                    Some((_, b)) => {
                        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
                        let _ = s.write_all(b);
                    }
                    None => {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        base
    }

    fn mock_release(tag: &str, sig: Option<&[u8]>, package: &[u8]) -> (Source, String) {
        let name = "IdleFighterZ_9.9.9_universal.zip";
        let mut files = vec![("/SHA256SUMS".to_string(), SUMS.to_vec()), (format!("/{name}"), package.to_vec())];
        if let Some(sig) = sig {
            files.push(("/SHA256SUMS.minisig".into(), sig.to_vec()));
        }
        let names: Vec<String> = files.iter().map(|(p, _)| p[1..].to_string()).collect();
        // the JSON needs the files' server's address, so it comes from a second server
        let base = serve(files);
        let assets: Vec<Value> = names.iter().map(|n| serde_json::json!({ "name": n, "browser_download_url": format!("{base}/{n}") })).collect();
        let json = serde_json::to_vec(&serde_json::json!({ "tag_name": tag, "draft": false, "prerelease": false, "assets": assets })).unwrap();
        let api = format!("{}/latest", serve(vec![("/latest".into(), json)]));
        (Source { api, pubkey: key("test.pub"), https_only: false }, name.into())
    }

    #[test]
    fn a_stand_in_release_downloads_and_verifies() {
        use tauri::async_runtime::block_on as block;
        let good = fixture("IdleFighterZ_9.9.9_universal.zip");
        let (src, name) = mock_release(TAG, Some(SIG), &good);
        assert!(block(check(&src, "9.9.9")).unwrap().is_none(), "already on it");
        assert!(block(check(&src, "10.0.0")).unwrap().is_none(), "never a downgrade");
        let release = block(check(&src, "0.2.1")).unwrap().expect("9.9.9 is newer");
        let dir = work_dir().unwrap();
        let file = block(fetch(&src, &release, "_universal.zip", &dir)).unwrap();
        assert_eq!((file.file_name().unwrap().to_str().unwrap(), std::fs::read(&file).unwrap()), (name.as_str(), good.clone()));
        std::fs::remove_dir_all(&dir).unwrap();

        // each of these fails closed, with nothing left in the download folder
        let refused = |src: &Source, why: &str| {
            let release = block(check(src, "0.2.1")).unwrap().unwrap();
            let dir = work_dir().unwrap();
            let e = block(fetch(src, &release, "_universal.zip", &dir)).unwrap_err();
            assert!(e.contains(why), "{e}");
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "{why}: a download was left behind");
            std::fs::remove_dir_all(&dir).unwrap();
        };
        refused(&mock_release(TAG, Some(SIG), b"a tampered package").0, "sha256 mismatch");
        refused(&mock_release(TAG, None, &good).0, "no SHA256SUMS.minisig");
        refused(&mock_release(TAG, Some(&fixture("other-tag.minisig")), &good).0, "was signed for 'idle-fighterz desktop-v0.2.0'");
        refused(&mock_release(TAG, Some(&fixture("other-key.minisig")), &good).0, "another key");
        // a newer tag replaying 9.9.9's signed SHA256SUMS
        refused(&mock_release("desktop-v10.0.0", Some(SIG), &good).0, "not 'idle-fighterz desktop-v10.0.0'");
        let (mut https, _) = mock_release(TAG, Some(SIG), &good);
        https.https_only = true;
        assert!(block(check(&https, "0.2.1")).is_err(), "release builds only speak https");
    }
}
