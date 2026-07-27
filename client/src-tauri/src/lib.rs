#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(test)]
mod acl_check;
mod commands;
mod folder_picker;
mod remote_ipc;
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
                    if remote_ipc::handle_navigation(&webview, url) {
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

            if let Some(inject) = state.take_inject() {
                let _ = webview.eval(inject.eval_script());
            }
            // After the one-shot session inject (and on every later remote load),
            // mark native + install the invoke / Settings bridge.
            let _ = webview.eval(state::native_chrome_js());
        });

    // process + autostart are desktop-oriented.
    #[cfg(desktop)]
    {
        builder = builder
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

                let tray_show =
                    MenuItem::with_id(app, "show", "Show Sarca", true, None::<&str>)?;
                let tray_sync_now =
                    MenuItem::with_id(app, "sync_now", "Sync now", true, None::<&str>)?;
                let tray_sync_settings =
                    MenuItem::with_id(app, "sync_settings", "Sync settings", true, None::<&str>)?;
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
                    &[
                        &show,
                        &sync_settings,
                        &sync_now,
                        &sep,
                        &disconnect,
                        &quit,
                    ],
                )?;
                let app_menu = Menu::with_items(app, &[&app_submenu])?;
                let _ = app.set_menu(app_menu);

                let tray_menu = Menu::with_items(
                    app,
                    &[
                        &tray_show,
                        &tray_sync_settings,
                        &tray_sync_now,
                        &tray_disconnect,
                        &tray_quit,
                    ],
                )?;

                let _tray = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&tray_menu)
                    .tooltip("Sarca — Sync settings in menu")
                    .show_menu_on_left_click(true)
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Right,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            focus_main_window(app);
                        }
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
            state.start_background_loop();

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
            commands::get_session,
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
            commands::sync_now,
            commands::sync_statuses,
            commands::get_client_prefs,
            commands::set_client_prefs,
            commands::is_on_wifi,
            commands::get_about,
            commands::get_cache_size,
            commands::clear_local_cache,
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
            // Non-macOS: keep the run-event callback so Builder::run stays equivalent.
            #[cfg(not(target_os = "macos"))]
            let _ = (app_handle, &event);
        });
}
