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

/// A saved file from an older version: a size preset name becomes the number it stood for. Any file at all means the
/// widget has been met already, so the first-run card doesn't show (0.2.x had none, and no `onboarded`)
pub fn migrate(saved: &Value) -> Value {
    let mut saved = saved.clone();
    if let Some(v) = saved.get("size").filter(|v| v.is_string()).cloned() {
        saved["size"] = json!(size(&v) as u64);
    }
    if saved.is_object() && saved.get("onboarded").is_none() {
        saved["onboarded"] = json!(true);
    }
    saved
}

/// The settings window for `settings_style`: its page and size in points. "compact" (the default, and anything
/// unknown) is the tall single column; "full" the wide one with tabs and a live preview
pub fn settings_page(style: &Value) -> (&'static str, f64, f64) {
    match style.as_str() {
        Some("full") => ("settings-full.html", 820.0, 580.0),
        _ => ("settings.html", 400.0, 918.0),
    }
}

/// whether a click on the menu-bar icon opens the popover: `tray_style` "popover", where the tray reports clicks (not
/// Linux, whose AppIndicator only opens menus)
pub fn popover(style: &Value) -> bool {
    style.as_str() == Some("popover") && cfg!(not(target_os = "linux"))
}

/// the HUD skins and themes the widget page knows (game/skins.js); anything else reads as today's look
pub const SKINS: [&str; 4] = ["classic", "scouter", "capsule", "drawer"];
pub const THEMES: [&str; 3] = ["light", "dark", "auto"];
/// the ability buttons' looks (game/widget.css `data-kibtn`): the ki meter (the default) fills as they recharge, classic
/// counts down
pub const KI_BUTTONS: [&str; 2] = ["meter", "classic"];
/// the planet and UI sizes in window mode and fullscreen (game/world.js PLANET_FLOOR, UI_SIZE), smallest first
pub const PLANET_SIZES: [&str; 4] = ["small", "medium", "large", "huge"];
pub const UI_SIZES: [&str; 4] = PLANET_SIZES;

/// A step along `sizes` from `cur` (a size it doesn't know is medium), held at either end; 0 is medium
pub fn step(sizes: &[&'static str], cur: &Value, by: i64) -> &'static str {
    let medium = sizes.iter().position(|s| *s == "medium").unwrap_or(0);
    if by == 0 {
        return sizes[medium];
    }
    let at = sizes.iter().position(|s| cur.as_str() == Some(*s)).unwrap_or(medium) as i64;
    sizes[(at + by.signum()).clamp(0, sizes.len() as i64 - 1) as usize]
}

/// a skin, theme, ki buttons look, mode, planet or UI size this version doesn't know (a newer one's, or a typo) goes back to the default
pub fn known_look(s: &mut Value) {
    let known = [("skin", &SKINS[..], "classic"), ("theme", &THEMES[..], "light"), ("ki_buttons", &KI_BUTTONS[..], "meter"), ("mode", &MODES[..], "floating"), ("mode_prev", &MODES[..2], "floating"), ("planet_size", &PLANET_SIZES[..], "medium"), ("ui_size", &UI_SIZES[..], "medium")];
    for (key, known, fallback) in known {
        if !s[key].as_str().is_some_and(|v| known.contains(&v)) {
            s[key] = json!(fallback);
        }
    }
}

/// the widget's modes: floating over the wallpaper, an opaque window, or that window filling the display. `mode_prev`,
/// the mode leaving fullscreen goes back to, is one of the first two
pub const MODES: [&str; 3] = ["floating", "window", "fullscreen"];

/// the widget layouts there are (4 and 5 were dropped)
pub const LAYOUTS: [u64; 5] = [7, 1, 2, 3, 6];

/// an update picking fullscreen keeps the mode it's picked from, `mode_prev`, for leaving it
pub fn keep_prev(cur: &Value, update: &mut Value) {
    if update.get("mode") == Some(&json!("fullscreen")) && cur["mode"] != "fullscreen" {
        update["mode_prev"] = cur["mode"].clone();
    }
}

/// an update (a page's, or a profile's) without a layout, skin or theme this version doesn't have: those keep their value
pub fn vet(update: &mut Value) {
    let Some(o) = update.as_object_mut() else { return };
    let ok = |k: &str, v: &Value| match k {
        "layout" => v.as_u64().is_some_and(|l| LAYOUTS.contains(&l)),
        "skin" => v.as_str().is_some_and(|v| SKINS.contains(&v)),
        "theme" => v.as_str().is_some_and(|v| THEMES.contains(&v)),
        "ki_buttons" => v.as_str().is_some_and(|v| KI_BUTTONS.contains(&v)),
        "mode" => v.as_str().is_some_and(|v| MODES.contains(&v)),
        "mode_prev" => v.as_str().is_some_and(|v| MODES[..2].contains(&v)),
        "planet_size" => v.as_str().is_some_and(|v| PLANET_SIZES.contains(&v)),
        "ui_size" => v.as_str().is_some_and(|v| UI_SIZES.contains(&v)),
        _ => true,
    };
    o.retain(|k, v| ok(k, v));
}

/// what a profile keeps: how the widget looks and behaves, and which settings window and menu-bar click it has. Not the
/// shortcut, login, updates, the tray icon's colour, a snooze, the first-run card, or the profiles themselves
pub const PROFILE_KEYS: [&str; 28] = [
    "on_top", "size", "opacity", "layout", "hud", "clouds", "cloud_shape", "cloud_amount", "mode", "fps", "ghost", "auto_hide",
    "battery_auto", "menubar_power", "notify", "quiet", "follow_focus", "recap", "sound", "skin", "theme", "settings_style", "tray_style", "raid_windows", "screen_beam",
    "ki_buttons", "planet_size", "ui_size",
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
        assert_eq!(migrate(&json!({ "size": "large", "on_top": false }))["size"], json!(560));
        assert_eq!(migrate(&json!({ "size": 450 }))["size"], json!(450), "a number stays");
        assert_eq!(migrate(&json!({ "on_top": true })), json!({ "on_top": true, "onboarded": true }));
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

    #[test]
    fn any_old_file_has_met_the_widget_and_a_saved_answer_stays() {
        assert_eq!(migrate(&json!({}))["onboarded"], json!(true));
        assert_eq!(migrate(&json!({ "onboarded": false }))["onboarded"], json!(false), "asked for the intro again");
        assert_eq!(migrate(&json!({ "onboarded": true }))["onboarded"], json!(true));
        assert_eq!(migrate(&json!("junk")), json!("junk"), "not an object: left for merge to ignore");
    }

    #[test]
    fn the_settings_style_picks_the_window() {
        assert_eq!(settings_page(&json!("compact")), ("settings.html", 400.0, 918.0));
        assert_eq!(settings_page(&json!("full")), ("settings-full.html", 820.0, 580.0));
        assert_eq!(settings_page(&json!("fancy")).0, "settings.html", "unknown: compact");
        assert_eq!(settings_page(&Value::Null).0, "settings.html");
    }

    #[test]
    fn the_popover_is_only_where_the_tray_reports_clicks() {
        assert_eq!(popover(&json!("popover")), cfg!(not(target_os = "linux")));
        assert!(!popover(&json!("menu")) && !popover(&json!("other")) && !popover(&Value::Null));
    }

    fn settings() -> Value {
        json!({
            "on_top": true, "size": 400, "opacity": 1, "layout": 7, "hud": "hover", "clouds": "hover", "cloud_shape": "c", "cloud_amount": 2,
            "mode": "floating", "fps": 30, "ghost": false, "auto_hide": true, "battery_auto": true, "menubar_power": true,
            "notify": { "raid": true, "pod": false }, "quiet": { "on": false, "from": "22:00", "to": "08:00" }, "follow_focus": true, "recap": true,
            "sound": null, "shortcut": "Alt+Cmd+KeyP", "login": false, "profiles": [], "snooze_until": 0, "skin": "classic", "theme": "light",
            "ki_buttons": "classic", "settings_style": "compact", "tray_style": "menu", "onboarded": true,
        })
    }

    #[test]
    fn a_profile_saves_and_applies_back() {
        let mut s = settings();
        s["size"] = json!(300);
        s["ghost"] = json!(true);
        s["notify"]["raid"] = json!(false);
        s["skin"] = json!("drawer");
        s["theme"] = json!("dark");
        s["ki_buttons"] = json!("meter");
        s["profiles"] = save(&s, " Work ").unwrap();
        let work = s.clone();
        s["size"] = json!(560.0);
        s["sound"] = json!(true);
        s["ghost"] = json!(false);
        s["skin"] = json!("scouter");
        s["theme"] = json!("auto");
        s["ki_buttons"] = json!("classic");
        s["shortcut"] = json!("Alt+Cmd+KeyO");
        s["profiles"] = save(&s, "Play").unwrap();
        assert_eq!(names(&s), ["Work", "Play"]);
        assert_eq!(active(&s).as_deref(), Some("Play"));

        let p = find(&s, "work").unwrap();
        assert!(p.get("shortcut").is_none() && p.get("profiles").is_none() && p.get("sound").is_none() && p.get("onboarded").is_none(), "{p}");
        crate::merge(&mut s, &p);
        assert_eq!(active(&s).as_deref(), Some("Work"));
        assert_eq!((s["skin"].clone(), s["theme"].clone(), s["ki_buttons"].clone()), (json!("drawer"), json!("dark"), json!("meter")), "a profile carries the look");
        for k in PROFILE_KEYS.iter().filter(|k| **k != "sound") {
            assert!(same(&s[*k], &work[*k]), "{k}: {} vs {}", s[*k], work[*k]);
        }
        assert_eq!(s["shortcut"], json!("Alt+Cmd+KeyO"), "not a profile's");
        s["opacity"] = json!(0.5);
        assert_eq!(active(&s), None);
    }

    #[test]
    fn an_unknown_skin_theme_ki_buttons_mode_or_size_is_the_default() {
        let defaults = json!({ "skin": "classic", "theme": "light", "ki_buttons": "meter", "mode": "floating", "mode_prev": "floating", "planet_size": "medium", "ui_size": "medium" });
        for (skin, theme, ki, mode) in [(json!("manga"), json!("sepia"), json!("gauge"), json!("kiosk")), (json!(3), json!(null), json!(2), json!(1)), (json!(""), json!(true), json!(null), json!(null))] {
            let mut s = json!({ "skin": skin, "theme": theme, "ki_buttons": ki, "mode": mode, "mode_prev": mode, "planet_size": mode, "ui_size": mode });
            known_look(&mut s);
            assert_eq!(s, defaults);
        }
        let mut s = json!({ "mode": "fullscreen", "mode_prev": "fullscreen" });
        known_look(&mut s);
        assert_eq!((&s["mode"], &s["mode_prev"]), (&json!("fullscreen"), &json!("floating")), "fullscreen leaves for floating or window");
        for skin in SKINS {
            for theme in THEMES {
                for ki in KI_BUTTONS {
                    for (mode, prev) in [("floating", "window"), ("window", "floating"), ("fullscreen", "window")] {
                        let kept = json!({ "skin": skin, "theme": theme, "ki_buttons": ki, "mode": mode, "mode_prev": prev, "planet_size": "huge", "ui_size": "small" });
                        let mut s = kept.clone();
                        known_look(&mut s);
                        assert_eq!(s, kept, "kept");
                    }
                }
            }
        }
    }

    // a profile saved by a newer version (or edited by hand) can't set what this one doesn't have
    #[test]
    fn an_update_keeps_only_layouts_and_looks_there_are() {
        let mut p = json!({ "layout": 4, "skin": "manga", "theme": 3, "ki_buttons": "gauge", "mode": "kiosk", "mode_prev": "fullscreen", "planet_size": "giant", "ui_size": 2, "size": 300 });
        vet(&mut p);
        assert_eq!(p, json!({ "size": 300 }));
        for mode in MODES {
            let mut p = json!({ "mode": mode });
            vet(&mut p);
            assert_eq!(p, json!({ "mode": mode }), "kept");
        }
        let mut p = json!({ "mode_prev": "window" });
        vet(&mut p);
        assert_eq!(p, json!({ "mode_prev": "window" }), "kept");
        for size in PLANET_SIZES {
            let mut p = json!({ "planet_size": size, "ui_size": size });
            vet(&mut p);
            assert_eq!(p, json!({ "planet_size": size, "ui_size": size }), "kept");
        }
        for layout in LAYOUTS {
            let mut p = json!({ "layout": layout, "skin": "drawer", "theme": "auto", "ki_buttons": "meter" });
            vet(&mut p);
            assert_eq!(p, json!({ "layout": layout, "skin": "drawer", "theme": "auto", "ki_buttons": "meter" }), "kept");
        }
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

    #[test]
    fn fullscreen_remembers_the_mode_it_was_picked_from() {
        for from in ["floating", "window"] {
            let mut u = json!({ "mode": "fullscreen" });
            keep_prev(&json!({ "mode": from, "mode_prev": "floating" }), &mut u);
            assert_eq!(u, json!({ "mode": "fullscreen", "mode_prev": from }));
        }
        let mut u = json!({ "mode": "fullscreen", "size": 560 });
        keep_prev(&json!({ "mode": "fullscreen", "mode_prev": "window" }), &mut u);
        assert_eq!(u, json!({ "mode": "fullscreen", "size": 560 }), "already fullscreen: what it came from stays");
        for u in [json!({ "mode": "window" }), json!({ "size": 300 })] {
            let mut v = u.clone();
            keep_prev(&json!({ "mode": "floating" }), &mut v);
            assert_eq!(v, u, "not picking fullscreen");
        }
    }

    // the widget's ⌘/ctrl +, −, 0 and ⌥/alt +, −, 0
    #[test]
    fn sizes_step_and_hold_at_the_ends() {
        let s = |cur: &str, by| step(&UI_SIZES, &json!(cur), by);
        assert_eq!([s("small", 1), s("medium", 1), s("large", 1), s("huge", 1)], ["medium", "large", "huge", "huge"]);
        assert_eq!([s("huge", -1), s("large", -1), s("medium", -1), s("small", -1)], ["large", "medium", "small", "small"]);
        assert_eq!([s("small", 0), s("huge", 0)], ["medium", "medium"], "0 is medium");
        assert_eq!(s("giant", 1), "large", "one it doesn't know steps from medium");
        assert_eq!(step(&PLANET_SIZES, &json!(null), -3), "small", "one step at a time");
    }
}
