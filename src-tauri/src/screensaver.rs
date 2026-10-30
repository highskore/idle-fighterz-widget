// The screensaver (saver/): the tray's "Install Screensaver…" puts the copy bundled in the app where the system looks
// for it, for this user only (no admin), and the app serves it frames of the run while it shows (planet.rs).
// Linux has no screensaver of ours (saver/README.md): there only the tests use the copying
#![cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[cfg(target_os = "macos")]
const FILE: &str = "Idle FighterZ.saver";
#[cfg(windows)]
const FILE: &str = "IdleFighterZ.scr";

/// The screensaver bundled with the app, if this build has one (the tray offers to install it only then).
#[cfg(any(target_os = "macos", windows))]
pub fn bundled(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().resource_dir().ok()?.join(FILE)).filter(|p| p.exists())
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn bundled(_: &AppHandle) -> Option<PathBuf> {
    None
}

/// Installs the bundled screensaver and opens the system's screensaver settings to pick it. What happened, for a
/// notification.
pub fn install(app: &AppHandle) -> Result<&'static str, String> {
    let src = bundled(app).ok_or("this build has no screensaver")?;
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or("no home folder")?;
        copy_into(&src, &PathBuf::from(home).join("Library/Screen Savers"))?;
        let _ = std::process::Command::new("open").arg("x-apple.systempreferences:com.apple.ScreenSaver-Settings.extension").spawn();
        Ok("pick Idle FighterZ in Screen Saver settings")
    }
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").ok_or("no local app data folder")?;
        let scr = copy_into(&src, &PathBuf::from(local).join("IdleFighterZ"))?;
        win::select(&scr)?;
        let _ = std::process::Command::new("control.exe").arg("desk.cpl,,@screensaver").spawn();
        Ok("it's your screensaver now")
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = src;
        Err("no screensaver on this system".into())
    }
}

/// Copies `src` (a file, or a bundle folder) into `dir`, replacing an older copy; the copy's path.
pub fn copy_into(src: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = src.file_name().ok_or("no file name")?;
    let dest = dir.join(name);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // a new copy beside the old one, then swapped in, so a failed copy leaves the old one working
    let tmp = dir.join(format!(".{}.new", name.to_string_lossy()));
    let _ = remove(&tmp);
    copy(src, &tmp).and_then(|_| remove(&dest)).and_then(|_| std::fs::rename(&tmp, &dest)).map_err(|e| {
        let _ = remove(&tmp);
        format!("{}: {e}", dest.display())
    })?;
    Ok(dest)
}

fn copy(src: &Path, dest: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.is_dir() {
        std::fs::create_dir_all(dest)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy(&e.path(), &dest.join(e.file_name()))?;
        }
        Ok(())
    } else if meta.file_type().is_symlink() {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(std::fs::read_link(src)?, dest);
        #[cfg(not(unix))]
        Ok(())
    } else {
        std::fs::copy(src, dest).map(|_| ())
    }
}

fn remove(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;
    use windows_sys::Win32::{
        Storage::FileSystem::GetShortPathNameW,
        UI::WindowsAndMessaging::{SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETSCREENSAVEACTIVE},
    };
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    // the 8.3 form of a path with spaces in it, where the volume keeps them: the screensaver launcher splits on spaces
    fn short(p: &Path) -> std::ffi::OsString {
        let wide: Vec<u16> = p.as_os_str().encode_wide().chain([0]).collect();
        let mut buf = vec![0u16; 1024];
        // SAFETY: `wide` is NUL-terminated and `buf` holds the length passed
        let n = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 || n >= buf.len() {
            return p.as_os_str().to_owned();
        }
        std::ffi::OsString::from_wide(&buf[..n])
    }

    /// Makes `scr` this user's screensaver (HKCU, as the settings dialog does), and turns screensavers on.
    pub fn select(scr: &Path) -> Result<(), String> {
        let (desk, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(r"Control Panel\Desktop").map_err(|e| e.to_string())?;
        desk.set_value("SCRNSAVE.EXE", &short(scr)).map_err(|e| e.to_string())?;
        desk.set_value("ScreenSaveActive", &"1").map_err(|e| e.to_string())?;
        // SAFETY: SPI_SETSCREENSAVEACTIVE takes its value in uiParam and no pointer
        unsafe { SystemParametersInfoW(SPI_SETSCREENSAVEACTIVE, 1, std::ptr::null_mut(), SPIF_UPDATEINIFILE | SPIF_SENDCHANGE) };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_is_copied_whole_and_replaces_the_old_one() {
        let t = std::env::temp_dir().join(format!("ifz-saver-install-{}", std::process::id()));
        let _ = remove(&t);
        let src = t.join("app/Idle FighterZ.saver");
        std::fs::create_dir_all(src.join("Contents/MacOS")).unwrap();
        std::fs::write(src.join("Contents/Info.plist"), "new").unwrap();
        std::fs::write(src.join("Contents/MacOS/IdleFighterZ"), "bin").unwrap();
        let dir = t.join("Screen Savers");
        std::fs::create_dir_all(dir.join("Idle FighterZ.saver/Contents")).unwrap();
        std::fs::write(dir.join("Idle FighterZ.saver/Contents/stale"), "old").unwrap();
        let dest = copy_into(&src, &dir).unwrap();
        assert_eq!(dest, dir.join("Idle FighterZ.saver"));
        assert_eq!(std::fs::read_to_string(dest.join("Contents/Info.plist")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(dest.join("Contents/MacOS/IdleFighterZ")).unwrap(), "bin");
        assert!(!dest.join("Contents/stale").exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp copy left behind");
        remove(&t).unwrap();
    }

    #[test]
    fn a_missing_source_leaves_the_installed_copy() {
        let t = std::env::temp_dir().join(format!("ifz-saver-keep-{}", std::process::id()));
        let _ = remove(&t);
        std::fs::create_dir_all(t.join("IdleFighterZ")).unwrap();
        std::fs::write(t.join("IdleFighterZ/IdleFighterZ.scr"), "old").unwrap();
        assert!(copy_into(&t.join("gone/IdleFighterZ.scr"), &t.join("IdleFighterZ")).is_err());
        assert_eq!(std::fs::read_to_string(t.join("IdleFighterZ/IdleFighterZ.scr")).unwrap(), "old");
        remove(&t).unwrap();
    }
}
