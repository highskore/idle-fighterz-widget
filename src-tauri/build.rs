// every command the pages may call; the capabilities grant each window only the ones it uses
const COMMANDS: &[&str] = &[
    "get_settings", "set_settings", "tray_title", "hide_widget", "open_settings", "quit", "notify", "notify_permission",
    "request_notify_permission", "open_notify_settings", "page_state", "focus_status", "snooze", "save_profile", "delete_profile",
    "apply_profile", "set_sound", "photo", "photo_exit",
    // the full settings window's preview, and the menu-bar popover's
    "glance", "preview_frame", "tray_action", "hide_popover", "intro_done",
    // the HUD page's (feature rust-core)
    "planet_start", "planet_resend", "planet_resize", "planet_pointer_down", "planet_pointer_move", "planet_pointer_up",
    "planet_pointer_cancel", "planet_key", "planet_train", "planet_kaioken", "planet_buy", "planet_learn", "planet_respec",
    "planet_learn_angel", "planet_auto_next", "planet_pick_wish", "planet_pick_hero", "planet_menu", "planet_angels",
    "planet_ascend_info", "planet_god_info", "planet_zeno_info", "planet_card", "planet_fps", "planet_itx", "planet_itx_jump",
    "planet_view", "planet_pixels", "planet_mask", "planet_sprite", "planet_spot", "planet_buy_n",
    "planet_shop_info", "planet_bulk",
];

fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)))
        .expect("tauri build script failed")
}
