#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(test)]
mod acl_check;
mod client_log;
mod commands;
mod folder_picker;
mod mediastore;
mod paths;
mod remote_ipc;
mod startup;
mod state;

use std::time::Duration;

use state::{is_shell_url, navigate_to_server, navigate_to_sync_settings, AppSyncState};
use tauri::{plugin::Builder as PluginBuilder, webview::PageLoadEvent, Manager};

#[cfg(desktop)]
use state::{navigate_to_shell, ServerConfig};
#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

#[cfg(desktop)]
fn focus_main_window(app: &impl Manager<tauri::Wry>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("sarca"),
        );
    }

    // WebKitGTK's DMA-BUF renderer (default since 2.42) drives GPU usage far
    // higher than the page content warrants on many Linux GPU/driver combos
    // (NVIDIA proprietary, some Mesa+Wayland setups) — see
    // https://v2.tauri.app/develop/debug/linux-graphics/. Must be set before
    // the webview is created. Respect an explicit user override either way.
    #[cfg(all(desktop, target_os = "linux"))]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            focus_main_window(app);
        }));
    }

    builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(folder_picker::init())
        .plugin(startup::init())
        .plugin(mediastore::init())
        .plugin(
            PluginBuilder::<tauri::Wry, ()>::new("sarca-nav")
                // Mark every navigation as native *before* page scripts when the
                // platform supports document-start JS (desktop). On Android remote
                // URLs this may run late — native_chrome_js + UI polling cover that.
                .js_init_script(format!(
                    "{}\n{}",
                    state::NATIVE_MARK_JS,
                    state::OPEN_SYNC_JS
                ))
                .on_navigation(|webview, url| {
                    // Remote Settings UI → Rust command channel (cancel navigation).
                    if remote_ipc::handle_navigation(webview, url) {
                        return false;
                    }
                    // Custom scheme and legacy query: open in-app Settings → Sync.
                    let open_sync = url.scheme() == "sarca-sync"
                        || url
                            .query_pairs()
                            .any(|(k, v)| k == "__sarca_open_sync" && v == "1");
                    if open_sync {
                        let app = webview.app_handle().clone();
                        let _ = navigate_to_sync_settings(&app);
                        return false;
                    }
                    true
                })
                .build(),
        )
        .on_page_load(|webview, payload| {
            let Some(state) = webview.try_state::<AppSyncState>() else {
                return;
            };
            state.remember_shell_url(payload.url().clone());

            if payload.event() != PageLoadEvent::Finished {
                return;
            }

            // Backup if `on_navigation` missed a same-document query change:
            // never leave the user on a remote URL that requested legacy Sync open.
            let wants_legacy_sync = payload
                .url()
                .query_pairs()
                .any(|(k, v)| k == "__sarca_open_sync" && v == "1");
            if wants_legacy_sync {
                let app = webview.app_handle().clone();
                let _ = navigate_to_sync_settings(&app);
                return;
            }
            // `__sarca_open_settings=sync` stays on the remote page; UI opens Settings.

            if is_shell_url(payload.url()) {
                return;
            }

            if let Some(inject) = state.take_inject_for(payload.url()) {
                let _ = webview.eval(inject.eval_script());
            }
            // After the one-shot session inject (and on every later remote load),
            // mark native + install the invoke / Settings bridge.
            let _ = webview.eval(state::native_chrome_js());
        });

    // e2e driver. Compiled only under `--features pilot` (never in a release
    // bundle) and further gated on a debug build so a `--release --features
    // pilot` slip cannot open the control socket in a shipped binary.
    #[cfg(all(desktop, debug_assertions, feature = "pilot"))]
    {
        builder = builder.plugin(tauri_plugin_pilot::init());
    }

    // process + autostart are desktop-oriented.
    #[cfg(desktop)]
    {
        builder =
            builder
                .plugin(tauri_plugin_process::init())
                .plugin(tauri_plugin_autostart::init(
                    tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                    Some(vec!["--minimized"]),
                ));
    }

    builder
        .register_asynchronous_uri_scheme_protocol("sarca-ipc", |ctx, request, responder| {
            remote_ipc::handle_protocol(ctx, request, responder);
        })
        .setup(|app| {
            // Kept out of capabilities/ on purpose: that directory is scanned at
            // build time, and `pilot:default` does not exist unless the optional
            // plugin is compiled in. Registering at runtime keeps release builds
            // buildable without the git dependency.
            #[cfg(all(desktop, debug_assertions, feature = "pilot"))]
            app.add_capability(include_str!("../e2e/pilot-capability.json"))?;

            let sync_state = AppSyncState::new(app.handle())?;
            let reconnect = {
                let cfg = tauri::async_runtime::block_on(sync_state.server.lock()).clone();
                cfg.is_connected().then_some(cfg)
            };
            app.manage(sync_state);

            #[cfg(desktop)]
            {
                // Separate menu items for window vs tray (items cannot belong to two menus).
                // Same ids so one on_menu_event handler covers both.
                let show = MenuItem::with_id(app, "show", "Show Sarca", true, None::<&str>)?;
                let sync_now = MenuItem::with_id(app, "sync_now", "Sync now", true, None::<&str>)?;
                let sync_settings =
                    MenuItem::with_id(app, "sync_settings", "Sync settings", true, None::<&str>)?;
                let disconnect =
                    MenuItem::with_id(app, "disconnect", "Disconnect", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let sep = PredefinedMenuItem::separator(app)?;

                let tray_show = MenuItem::with_id(app, "show", "Show Sarca", true, None::<&str>)?;
                let tray_disconnect =
                    MenuItem::with_id(app, "disconnect", "Disconnect", true, None::<&str>)?;
                let tray_quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

                app.on_menu_event(|app, event| match event.id().as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        focus_main_window(app);
                    }
                    "disconnect" => {
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let state = handle.state::<AppSyncState>();
                            let cfg = ServerConfig::default();
                            let _ = state.save_server(&cfg).await;
                            if let Ok(mut guard) = state.pending_inject.lock() {
                                *guard = None;
                            }
                            let _ = navigate_to_shell(&handle);
                        });
                    }
                    "sync_now" => {
                        let state = app.state::<AppSyncState>();
                        let engine = state.engine.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = engine.tick().await;
                        });
                    }
                    "sync_settings" => {
                        let _ = navigate_to_sync_settings(app);
                    }
                    _ => {}
                });

                // Window menu bar — Sync reachable when the system tray is hidden (Linux/GNOME).
                let app_submenu = Submenu::with_items(
                    app,
                    "Sarca",
                    true,
                    &[&show, &sync_settings, &sync_now, &sep, &disconnect, &quit],
                )?;
                let app_menu = Menu::with_items(app, &[&app_submenu])?;
                let _ = app.set_menu(app_menu);

                let tray_menu = Menu::with_items(app, &[&tray_show, &tray_disconnect, &tray_quit])?;

                let _tray = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&tray_menu)
                    .show_menu_on_left_click(false)
                    .tooltip("Sarca")
                    .on_tray_icon_event(|tray, event| match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => {
                            focus_main_window(tray.app_handle());
                        }
                        TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        } => {
                            focus_main_window(tray.app_handle());
                        }
                        _ => {}
                    })
                    .build(app)?;

                if let Some(window) = app.get_webview_window("main") {
                    let window_ = window.clone();
                    window.on_window_event(move |event| {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                            let _ = window_.hide();
                        }
                    });
                }
            }

            let state = app.state::<AppSyncState>();
            {
                let prefs = commands::load_prefs(&state);
                client_log::set_enabled(prefs.enable_logs, state.data_dir());
            }
            // Resolve device identity once early so Sync UI / Camera remote_root
            // never wait on a later Android plugin IPC round-trip.
            commands::ensure_device_label_cached(app.handle());
            state.start_background_loop();

            #[cfg(target_os = "android")]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Let the activity settle before system permission dialogs.
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    match startup::ensure_runtime_access(&handle).await {
                        Ok(()) => {
                            // First background tick may have raced the dialog; re-scan now.
                            handle.state::<AppSyncState>().request_sync_wake();
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "android runtime access prompt failed");
                            client_log::write_line(
                                handle.state::<AppSyncState>().data_dir(),
                                &format!("ensure_runtime_access failed: {e}"),
                            );
                        }
                    }
                });
            }

            if let Some(cfg) = reconnect {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let _ = navigate_to_server(&handle, &cfg);
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::platform_label,
            commands::device_label,
            commands::get_session,
            commands::get_url_history,
            commands::update_session,
            commands::connect,
            commands::disconnect,
            commands::open_app,
            commands::open_sync_settings,
            commands::pick_local_folder,
            commands::default_gallery_path,
            commands::list_storages,
            commands::ensure_remote_folder,
            commands::list_bindings,
            commands::add_binding,
            commands::remove_binding,
            commands::set_binding_enabled,
            commands::update_binding_local_path,
            commands::update_binding_remote_root,
            commands::sync_now,
            commands::sync_statuses,
            commands::sync_transfer_queue,
            commands::get_client_prefs,
            commands::set_client_prefs,
            commands::verify_app_lock_pin,
            commands::export_logs,
            commands::is_on_wifi,
            commands::get_about,
            commands::get_cache_size,
            commands::clear_local_cache,
            commands::cache_get_preview,
            commands::cache_put_preview,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Sarca client")
        .run(|app_handle, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } = event
            {
                if !has_visible_windows {
                    focus_main_window(app_handle);
                }
            }
            // Mobile: when the activity returns to foreground, wake the sync
            // loop immediately so auto-upload does not wait up to 30s (or stay
            // idle until the user opens Sync settings and polls).
            #[cfg(any(target_os = "android", target_os = "ios"))]
            if let tauri::RunEvent::Resumed = event {
                let state = app_handle.state::<AppSyncState>();
                // Only wake when a session exists — otherwise MediaStore discovery
                // on resume races the connect shell paint for no benefit.
                let connected = state::load_server_config(state.data_dir()).is_connected();
                if connected {
                    state.request_sync_wake();
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "android", target_os = "ios")))]
            let _ = (app_handle, &event);
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                let _ = &event;
            }
        });
}
