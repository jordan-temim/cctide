//! cctide — offline tracking of Claude Code consumption (tray app).
//!
//! Entry point: wires together Tauri plugins, the tray icon, the popup window,
//! and the background services. No business logic here.

mod config;
mod context;
mod icon;
mod memory;
mod models;
mod notify;
mod outcome;
mod rtk;
mod scan;
mod usage;

mod commands;
mod state;
mod tick;
mod update_svc;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_positioner::{Position, WindowExt};

use notify::NotifyState;
use scan::ScanCache;
use state::AppState;

fn toggle_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            #[cfg(target_os = "macos")]
            let pos = Position::TrayBottomCenter;
            #[cfg(not(target_os = "macos"))]
            let pos = Position::TrayBottomRight;
            let _ = win.move_window(pos);
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Surface panics from background threads (ticker, update check) on stderr —
    // visible in the `tauri dev` terminal; release builds have no console.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[cctide] PANIC: {info}");
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            cache: std::sync::Mutex::new(ScanCache::default()),
            notify_state: std::sync::Mutex::new(NotifyState::default()),
            config_cache: std::sync::Mutex::new(config::load()),
            models: models::load(),
            system: std::sync::Mutex::new(sysinfo::System::new()),
            available_update: std::sync::Mutex::new(None),
            rtk_cache: std::sync::Mutex::new(None),
            outcome_cache: std::sync::Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_panel_data,
            commands::get_outcomes,
            commands::get_memory,
            commands::get_config,
            commands::set_calibration,
            commands::set_notifications,
            commands::set_tracking,
            commands::kill_session,
            commands::delete_session_transcript,
            commands::cleanup_stale_sessions,
            commands::delete_memory_file,
            update_svc::install_update,
            update_svc::restart_app,
        ])
        .setup(|app| {
            // Menu-bar app: no Dock icon (macOS).
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Dedicated monochrome template for the menu bar.
            let tray_icon =
                tauri::image::Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?;

            let quit = MenuItem::with_id(app, "quit", "Quit cctide", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            TrayIconBuilder::with_id("cctide-tray")
                .icon(tray_icon)
                .icon_as_template(true)
                .tooltip("cctide")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    let app = tray.app_handle();
                    tauri_plugin_positioner::on_tray_event(app, &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_window(app);
                    }
                })
                .build(app)?;
            eprintln!("[cctide] tray icon created");

            // Hide the popup when it loses focus.
            if let Some(win) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                apply_macos_rounded_corners(&win);
                let win_clone = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let _ = win_clone.hide();
                    }
                });
            }

            // Request OS notification permission (macOS: required for .show() to work).
            if !matches!(
                app.notification().permission_state(),
                Ok(tauri_plugin_notification::PermissionState::Granted)
            ) {
                let _ = app.notification().request_permission();
            }

            // Update service: check at startup, then every UPDATE_CHECK_INTERVAL.
            let uh = app.handle().clone();
            update_svc::spawn_update_check(&uh);
            std::thread::spawn(move || loop {
                std::thread::sleep(update_svc::UPDATE_CHECK_INTERVAL);
                update_svc::spawn_update_check(&uh);
            });

            // Ticker service: recomputes usage + icon every refresh_secs.
            tick::start_ticker(app.handle().clone());
            eprintln!("[cctide] setup complete, ticker started");

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(target_os = "macos")]
fn apply_macos_rounded_corners(win: &tauri::WebviewWindow) {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSColor, NSWindow};

    let Ok(ptr) = win.ns_window() else { return };
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr is a valid, non-null NSWindow* provided by Tauri's ns_window().
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    unsafe {
        let ns_win = &*(ptr as *const NSWindow);
        ns_win.setOpaque(false);
        let clear: Retained<NSColor> = NSColor::clearColor();
        ns_win.setBackgroundColor(Some(&*clear));
        drop(clear);
        let Some(content_view) = ns_win.contentView() else {
            return;
        };
        content_view.setWantsLayer(true);
        if let Some(layer) = content_view.layer() {
            layer.setCornerRadius(12.0);
            layer.setMasksToBounds(true);
        }
    }
}
