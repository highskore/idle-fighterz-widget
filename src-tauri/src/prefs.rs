// Settings logic with no window in it: the size slider (and the old size presets it replaced) and profiles, named
// snapshots of the settings that change how the widget looks and behaves.
use serde_json::{json, Value};

/// the old fixed sizes, now snap points on the slider: (id, tray label, side in points)
pub const SNAPS: [(&str, &str, f64); 3] = [("small", "Small", 300.0), ("medium", "Medium", 400.0), ("large", "Large", 560.0)];
pub const MIN_SIZE: f64 = 300.0;
pub const MAX_SIZE: f64 = 720.0;
// within this many points of a snap point, the slider lands on it
const SNAP_REACH: f64 = 16.0;

/// The window side in points for a `size` setting: an old preset name ("medium") or a number. Clamped to the slider,
/// pulled onto a snap point within reach, and otherwise even: from 520 the pixel art is drawn 2×, and an even side
/// keeps every game pixel a whole number of screen pixels (on the canvas and on the macOS layer alike).
pub fn size(v: &Value) -> f64 {
    let n = match v {
        Value::String(s) => SNAPS.iter().find(|p| p.0 == s).map_or(400.0, |p| p.2),
        Value::Number(n) => n.as_f64().unwrap_or(400.0),
        _ => 400.0,
    };
    let n = if n.is_finite() { n.clamp(MIN_SIZE, MAX_SIZE) } else { 400.0 };
    match SNAPS.iter().find(|p| (p.2 - n).abs() <= SNAP_REACH) {
        Some(p) => p.2,
        None => (n / 2.0).round() * 2.0,
    }
}

/// a saved file from before the slider: its preset name becomes the number it stood for
pub fn migrate(saved: &Value) -> Value {
    let mut saved = saved.clone();
    if let Some(v) = saved.get("size").filter(|v| v.is_string()).cloned() {
        saved["size"] = json!(size(&v) as u64);
    }
    saved
}

/// what a profile keeps: how the widget looks and behaves. Not the shortcut, login, updates, the tray icon's colour,
/// a snooze, or the profiles themselves
pub const PROFILE_KEYS: [&str; 19] = [
    "on_top", "size", "opacity", "layout", "hud", "clouds", "cloud_shape", "cloud_amount", "mode", "fps", "ghost", "auto_hide",
    "battery_auto", "menubar_power", "notify", "quiet", "follow_focus", "recap", "sound",
];
pub const MAX_PROFILES: usize = 12;
pub const MAX_NAME: usize = 24;

/// the settings a profile named now would keep
pub fn snapshot(s: &Value) -> Value {
    Value::Object(PROFILE_KEYS.iter().filter_map(|k| s.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone()))).collect())
}

fn list(s: &Value) -> Vec<Value> {
    s["profiles"].as_array().cloned().unwrap_or_default()
}

fn named<'a>(p: &'a Value, name: &str) -> Option<&'a Value> {
    p["name"].as_str().filter(|n| n.eq_ignore_ascii_case(name)).map(|_| &p["settings"])
}

/// Saves the settings as `name`, replacing a profile of the same name (any case). The new list, or why not.
pub fn save(s: &Value, name: &str) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(format!("a name of 1 to {MAX_NAME} characters"));
    }
    let mut all = list(s);
    let entry = json!({ "name": name, "settings": snapshot(s) });
    match all.iter().position(|p| named(p, name).is_some()) {
        Some(i) => all[i] = entry,
        None if all.len() >= MAX_PROFILES => return Err(format!("at most {MAX_PROFILES} profiles")),
        None => all.push(entry),
    }
    Ok(Value::Array(all))
}

pub fn delete(s: &Value, name: &str) -> Value {
    Value::Array(list(s).into_iter().filter(|p| named(p, name).is_none()).collect())
}

/// the settings profile `name` sets, only the keys a profile may set
pub fn find(s: &Value, name: &str) -> Option<Value> {
    let p = list(s).iter().find_map(|p| named(p, name).cloned())?;
    Some(Value::Object(p.as_object()?.iter().filter(|(k, _)| PROFILE_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()))
}

pub fn names(s: &Value) -> Vec<String> {
    list(s).iter().filter_map(|p| p["name"].as_str().map(String::from)).collect()
}

// equal, with 400 and 400.0 the same number (the pages send integers, the app may write floats)
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Object(x), Value::Object(y)) => x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w))),
        _ => a == b,
    }
}

/// the profile the settings match right now, if any: the first whose every setting is the current one
pub fn active(s: &Value) -> Option<String> {
    list(s).iter().find(|p| p["settings"].as_object().is_some_and(|o| o.iter().all(|(k, v)| s.get(k).is_some_and(|c| same(v, c))))).and_then(|p| p["name"].as_str().map(String::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_sizes_become_their_points() {
        assert_eq!([size(&json!("small")), size(&json!("medium")), size(&json!("large"))], [300.0, 400.0, 560.0]);
        assert_eq!(size(&json!("huge")), 400.0, "an unknown name is medium");
        assert_eq!(size(&Value::Null), 400.0);
        assert_eq!(migrate(&json!({ "size": "large", "on_top": false })), json!({ "size": 560, "on_top": false }));
        assert_eq!(migrate(&json!({ "size": 450 })), json!({ "size": 450 }), "a number stays");
        assert_eq!(migrate(&json!({ "on_top": true })), json!({ "on_top": true }));
    }

    #[test]
    fn the_slider_snaps_clamps_and_keeps_pixels_whole() {
        assert_eq!(size(&json!(390)), 400.0);
        assert_eq!(size(&json!(416)), 400.0);
        assert_eq!(size(&json!(417)), 418.0, "out of reach: free, made even");
        assert_eq!(size(&json!(548)), 560.0);
        assert_eq!(size(&json!(311)), 300.0);
        assert_eq!(size(&json!(100)), 300.0);
        assert_eq!(size(&json!(5000)), 720.0);
        assert_eq!(size(&json!(f64::NAN.to_string())), 400.0);
        for n in (300..=720).step_by(1) {
            let s = size(&json!(n));
            assert!(s % 2.0 == 0.0 && (MIN_SIZE..=MAX_SIZE).contains(&s), "{n} → {s}");
        }
    }

    fn settings() -> Value {
        json!({
            "on_top": true, "size": 400, "opacity": 1, "layout": 7, "hud": "hover", "clouds": "hover", "cloud_shape": "c", "cloud_amount": 2,
            "mode": "floating", "fps": 30, "ghost": false, "auto_hide": true, "battery_auto": true, "menubar_power": true,
            "notify": { "raid": true, "pod": false }, "quiet": { "on": false, "from": "22:00", "to": "08:00" }, "follow_focus": true, "recap": true,
            "sound": null, "shortcut": "Alt+Cmd+KeyP", "login": false, "profiles": [], "snooze_until": 0,
        })
    }

    #[test]
    fn a_profile_saves_and_applies_back() {
        let mut s = settings();
        s["size"] = json!(300);
        s["ghost"] = json!(true);
        s["notify"]["raid"] = json!(false);
        s["profiles"] = save(&s, " Work ").unwrap();
        let work = s.clone();
        s["size"] = json!(560.0);
        s["sound"] = json!(true);
        s["ghost"] = json!(false);
        s["shortcut"] = json!("Alt+Cmd+KeyO");
        s["profiles"] = save(&s, "Play").unwrap();
        assert_eq!(names(&s), ["Work", "Play"]);
        assert_eq!(active(&s).as_deref(), Some("Play"));

        let p = find(&s, "work").unwrap();
        assert!(p.get("shortcut").is_none() && p.get("profiles").is_none() && p.get("sound").is_none(), "{p}");
        crate::merge(&mut s, &p);
        assert_eq!(active(&s).as_deref(), Some("Work"));
        for k in PROFILE_KEYS.iter().filter(|k| **k != "sound") {
            assert!(same(&s[*k], &work[*k]), "{k}: {} vs {}", s[*k], work[*k]);
        }
        assert_eq!(s["shortcut"], json!("Alt+Cmd+KeyO"), "not a profile's");
        s["opacity"] = json!(0.5);
        assert_eq!(active(&s), None);
    }

    #[test]
    fn profile_names_and_limits() {
        let mut s = settings();
        s["profiles"] = save(&s, "Work").unwrap();
        s["size"] = json!(560);
        s["profiles"] = save(&s, "WORK").unwrap();
        assert_eq!(names(&s), ["WORK"], "the same name in another case replaces it");
        assert_eq!(find(&s, "work").unwrap()["size"], json!(560));
        assert!(save(&s, "  ").is_err());
        assert!(save(&s, &"x".repeat(MAX_NAME + 1)).is_err());
        assert!(save(&s, "a\nb").is_err());
        for k in 1..MAX_PROFILES {
            s["profiles"] = save(&s, &format!("p{k}")).unwrap();
        }
        assert!(save(&s, "one more").is_err());
        assert!(save(&s, "p3").is_ok(), "replacing one is fine when full");
        s["profiles"] = delete(&s, "P3");
        assert_eq!(names(&s).len(), MAX_PROFILES - 1);
        assert_eq!(find(&s, "nope"), None);
    }
}
