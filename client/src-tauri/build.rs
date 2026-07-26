fn main() {
    // Remote-origin Settings (http/https) go through Tauri invoke when
    // `__TAURI_INTERNALS__` is injected via capabilities `remote.urls`.
    // Those calls require explicit ACL allow-* permissions for every command.
    const COMMANDS: &[&str] = &[
        "platform_label",
        "get_session",
        "update_session",
        "connect",
        "disconnect",
        "open_app",
        "open_sync_settings",
        "pick_local_folder",
        "default_gallery_path",
        "list_storages",
        "ensure_remote_folder",
        "list_bindings",
        "add_binding",
        "remove_binding",
        "sync_now",
        "sync_statuses",
        "get_client_prefs",
        "set_client_prefs",
        "is_on_wifi",
        "get_about",
        "get_cache_size",
        "clear_local_cache",
    ];

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
