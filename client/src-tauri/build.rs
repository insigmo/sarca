fn main() {
    // Remote-origin Settings (http/https) go through Tauri invoke when
    // `__TAURI_INTERNALS__` is injected via a capability's `remote.urls`.
    // Those calls require explicit ACL allow-* permissions for every command.
    // No capability file grants remote access any more: the connected server
    // origin gets one built at runtime (`remote_ipc::grant_remote_capability`),
    // which resolves these same identifiers.
    const COMMANDS: &[&str] = &[
        "platform_label",
        "device_label",
        "get_session",
        "get_url_history",
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
        "set_binding_enabled",
        "update_binding_local_path",
        "update_binding_remote_root",
        "sync_now",
        "sync_statuses",
        "sync_transfer_queue",
        "get_client_prefs",
        "set_client_prefs",
        "verify_app_lock_pin",
        "export_logs",
        "is_on_wifi",
        "get_about",
        "get_cache_size",
        "clear_local_cache",
        "cache_get_preview",
        "cache_put_preview",
    ];

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
