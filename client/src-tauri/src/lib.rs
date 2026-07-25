#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use state::AppSyncState;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init());

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        builder = builder.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ));
    }

    builder
        .setup(|app| {
            let sync_state = AppSyncState::new(app.handle())?;
            app.manage(sync_state);

            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let show = MenuItem::with_id(app, "show", "Show Sarca", true, None::<&str>)?;
                let sync_now = MenuItem::with_id(app, "sync_now", "Sync now", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &sync_now, &quit])?;

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
                        "sync_now" => {
                            let state = app.state::<AppSyncState>();
                            let engine = state.engine.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = engine.tick().await;
                            });
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::platform_label,
            commands::get_server_config,
            commands::set_server_config,
            commands::list_bindings,
            commands::add_binding,
            commands::remove_binding,
            commands::sync_now,
            commands::sync_statuses,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Sarca client");
}
