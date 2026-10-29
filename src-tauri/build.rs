// every command the pages may call; the capabilities grant each window only the ones it uses
const COMMANDS: &[&str] = &[
    "get_settings", "set_settings", "tray_title", "hide_widget", "open_settings", "quit", "notify", "notify_permission",
    "request_notify_permission", "open_notify_settings", "page_state",
    // the HUD page's (feature rust-core)
    "planet_start", "planet_resend", "planet_resize", "planet_pointer_down", "planet_pointer_move", "planet_pointer_up",
    "planet_pointer_cancel", "planet_key", "planet_train", "planet_kaioken", "planet_buy", "planet_learn", "planet_respec",
    "planet_learn_angel", "planet_auto_next", "planet_pick_wish", "planet_pick_hero", "planet_menu", "planet_angels",
    "planet_ascend_info", "planet_god_info", "planet_zeno_info", "planet_card", "planet_fps",
];

fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)))
        .expect("tauri build script failed")
}
