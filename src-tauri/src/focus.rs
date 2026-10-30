// When notifications hold back: a snooze from the tray, and the OS's own Focus / Do Not Disturb. The Focus probe reads
// what the OS lets any app read, with no permission prompt; where it can't tell (`None`), the OS's own filtering is
// left to do its job.
use chrono::{Duration, NaiveDateTime, NaiveTime};

/// whether a notification is held back: snoozed until after `now_ms`, or the OS in Focus with the setting on
pub fn hold(follow_focus: bool, focus: Option<bool>, snooze_until_ms: f64, now_ms: f64) -> bool {
    snoozed(snooze_until_ms, now_ms) || (follow_focus && focus == Some(true))
}

pub fn snoozed(until_ms: f64, now_ms: f64) -> bool {
    until_ms > now_ms
}

// "until tomorrow" ends at this local time
const MORNING: u32 = 8;

/// When a snooze picked at local time `now` ends: "30", "60" or "180" minutes on, or "tomorrow", the next 8:00
/// (after midnight and before 8 that's the same day's: it's still the night before).
pub fn snooze_end(now: NaiveDateTime, choice: &str) -> Option<NaiveDateTime> {
    match choice {
        "30" | "60" | "180" => Some(now + Duration::minutes(choice.parse().ok()?)),
        "tomorrow" => {
            let morning = NaiveTime::from_hms_opt(MORNING, 0, 0)?;
            let today = now.date().and_time(morning);
            Some(if today > now { today } else { today + Duration::days(1) })
        }
        _ => None,
    }
}

/// the time a snooze has left, as the tray shows it: "45 min", "1 h 12 min", "8 h"
pub fn left(secs: i64) -> String {
    let mins = (secs.max(0) + 59) / 60;
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// Is the OS in Focus / Do Not Disturb? None when it can't be told without a permission.
pub fn probe() -> Option<bool> {
    os::probe()
}

/// macOS keeps the Focus that's on in ~/Library/DoNotDisturb/DB/Assertions.json: a manual Focus (Control Centre,
/// a shortcut) is an assertion record there. The folder is behind Full Disk Access, which macOS never asks for: an
/// app without it just can't read the file, and nothing prompts. Some(on) from its JSON
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn assertions_on(json: &str) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let data = v["data"].as_array()?;
    Some(data.iter().any(|d| d["storeAssertionRecords"].as_array().is_some_and(|r| !r.is_empty())))
}

/// Windows' Focus Assist / Do Not Disturb profile (WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED): 0 off, 1 priority
/// only, 2 alarms only
#[cfg_attr(not(windows), allow(dead_code))]
pub fn quiet_profile(profile: u32) -> bool {
    profile != 0
}

#[cfg(target_os = "macos")]
mod os {
    pub fn probe() -> Option<bool> {
        let home = std::env::var_os("HOME")?;
        let file = std::path::Path::new(&home).join("Library/DoNotDisturb/DB/Assertions.json");
        super::assertions_on(&std::fs::read_to_string(file).ok()?)
    }
}

#[cfg(windows)]
mod os {
    use std::ffi::c_void;
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryWnfStateData(name: *const u64, kind: *const c_void, scope: *const c_void, stamp: *mut u32, buf: *mut c_void, size: *mut u32) -> i32;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHQueryUserNotificationState(state: *mut i32) -> i32;
    }
    const WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED: u64 = 0x0D83_063E_A3BF_1C75;

    // the Focus Assist / Do Not Disturb state (undocumented but stable since Windows 10 1803), else the shell's
    // QUNS_QUIET_TIME
    pub fn probe() -> Option<bool> {
        let (mut stamp, mut value, mut size) = (0u32, 0u32, 4u32);
        let ok = unsafe {
            NtQueryWnfStateData(&WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED, std::ptr::null(), std::ptr::null(), &mut stamp, &mut value as *mut u32 as *mut c_void, &mut size)
        } >= 0;
        if ok && size == 4 {
            return Some(super::quiet_profile(value));
        }
        let mut state = 0;
        (unsafe { SHQueryUserNotificationState(&mut state) } == 0).then_some(state == 6)
    }
}

// GNOME: org.gnome.desktop.notifications show-banners off is its Do Not Disturb, read through the settings portal or
// gsettings. KDE Plasma: its notification server's Inhibited property
#[cfg(target_os = "linux")]
mod os {
    use zbus::zvariant::{OwnedValue, Value};

    fn as_bool(v: &Value) -> Option<bool> {
        match v {
            Value::Bool(b) => Some(*b),
            Value::Value(inner) => as_bool(inner),
            _ => None,
        }
    }

    pub fn probe() -> Option<bool> {
        let bus = zbus::blocking::Connection::session().ok();
        if let Some(bus) = &bus {
            for method in ["ReadOne", "Read"] {
                let reply = bus.call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.Settings"),
                    method,
                    &("org.gnome.desktop.notifications", "show-banners"),
                );
                if let Some(banners) = reply.ok().and_then(|r| r.body().deserialize::<OwnedValue>().ok()).and_then(|v| as_bool(&v)) {
                    return Some(!banners);
                }
            }
        }
        if let Ok(out) = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.notifications", "show-banners"]).output() {
            match String::from_utf8_lossy(&out.stdout).trim() {
                "false" => return Some(true),
                "true" => return Some(false),
                _ => {}
            }
        }
        let bus = bus?;
        let reply = bus
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.freedesktop.Notifications", "Inhibited"),
            )
            .ok()?;
        let inhibited: OwnedValue = reply.body().deserialize().ok()?;
        as_bool(&inhibited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn snoozes_end_when_picked() {
        assert_eq!(snooze_end(at(30, 14, 10), "30"), Some(at(30, 14, 40)));
        assert_eq!(snooze_end(at(30, 14, 10), "60"), Some(at(30, 15, 10)));
        assert_eq!(snooze_end(at(30, 23, 10), "180"), Some(at(31, 2, 10)));
        assert_eq!(snooze_end(at(30, 14, 10), "tomorrow"), Some(at(31, 8, 0)));
        assert_eq!(snooze_end(at(30, 23, 59), "tomorrow"), Some(at(31, 8, 0)));
        assert_eq!(snooze_end(at(31, 1, 30), "tomorrow"), Some(at(31, 8, 0)), "after midnight it's still the night before");
        assert_eq!(snooze_end(at(31, 8, 0), "tomorrow"), Some(at(31, 8, 0) + Duration::days(1)), "8:00 sharp: the next one");
        assert_eq!(snooze_end(at(30, 14, 10), "forever"), None);
    }

    #[test]
    fn the_time_left_reads_short() {
        assert_eq!(left(30 * 60), "30 min");
        assert_eq!(left(30 * 60 - 20), "30 min", "a part minute counts");
        assert_eq!(left(72 * 60), "1 h 12 min");
        assert_eq!(left(3 * 3600), "3 h");
        assert_eq!(left(5), "1 min");
        assert_eq!(left(-5), "0 min");
    }

    #[test]
    fn notifications_hold_for_a_snooze_or_focus() {
        let now = 1_000_000.0;
        assert!(!hold(true, Some(false), 0.0, now));
        assert!(hold(true, Some(true), 0.0, now), "focus on, followed");
        assert!(!hold(false, Some(true), 0.0, now), "focus on, not followed");
        assert!(!hold(true, None, 0.0, now), "focus unknown: the OS filters on its own");
        assert!(hold(false, None, now + 1.0, now), "snoozed");
        assert!(!hold(false, None, now, now), "a snooze ending now is over");
    }

    #[test]
    fn focus_states_read() {
        assert_eq!(assertions_on(r#"{"data":[{"storeAssertionRecords":[{"assertionDetails":{"assertionDetailsModeIdentifier":"com.apple.donotdisturb.mode.default"}}]}]}"#), Some(true));
        assert_eq!(assertions_on(r#"{"data":[{}]}"#), Some(false));
        assert_eq!(assertions_on(r#"{"data":[{"storeAssertionRecords":[]}]}"#), Some(false));
        assert_eq!(assertions_on("not json"), None);
        assert!(!quiet_profile(0));
        assert!(quiet_profile(1) && quiet_profile(2));
    }
}
