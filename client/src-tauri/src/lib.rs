#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use std::time::Duration;

use state::{is_shell_url, navigate_to_server, navigate_to_sync_settings, AppSyncState};
use tauri::{plugin::Builder as PluginBuilder, webview::PageLoadEvent, Manager};

#[cfg(desktop)]
use state::{navigate_to_shell, ServerConfig};
#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(
            PluginBuilder::<tauri::Wry, ()>::new("sarca-nav")
                .on_navigation(|webview, url| {
                    if url.scheme() == "sarca-sync" {
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

            if is_shell_url(payload.url()) {
                return;
            }

            if let Some(inject) = state.take_inject() {
                let _ = webview.eval(inject.eval_script());
            }
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
        .setup(|app| {
            let sync_state = AppSyncState::new(app.handle())?;
            let reconnect = {
                let cfg = tauri::async_runtime::block_on(sync_state.server.lock()).clone();
                cfg.is_connected().then_some(cfg)
            };
            app.manage(sync_state);

            #[cfg(desktop)]
            {
                let show = MenuItem::with_id(app, "show", "Show Sarca", true, None::<&str>)?;
                let sync_now = MenuItem::with_id(app, "sync_now", "Sync now", true, None::<&str>)?;
                let sync_settings =
                    MenuItem::with_id(app, "sync_settings", "Sync settings", true, None::<&str>)?;
                let disconnect =
                    MenuItem::with_id(app, "disconnect", "Disconnect", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(
                    app,
                    &[&show, &sync_now, &sync_settings, &disconnect, &quit],
                )?;

                let _tray = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "quit" => app.exit(0),
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
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
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
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
            commands::connect,
            commands::disconnect,
            commands::open_app,
            commands::open_sync_settings,
            commands::pick_local_folder,
            commands::list_storages,
            commands::ensure_remote_folder,
            commands::list_bindings,
            commands::add_binding,
            commands::remove_binding,
            commands::sync_now,
            commands::sync_statuses,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Sarca client");
}
