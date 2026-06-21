//! Auto-update service: check-only detection + user-initiated install.
//!
//! Detection runs at startup and every UPDATE_CHECK_INTERVAL. When a newer
//! version is found it records an UpdateInfo in AppState and fires one OS
//! notification per version. The user installs from the panel banner via the
//! `install_update` command; `restart_app` applies it.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::state::{AppState, UpdateInfo};

pub const UPDATE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2 * 60 * 60);

/// True while a check is in flight (prevents concurrent checks).
pub static UPDATE_CHECKING: AtomicBool = AtomicBool::new(false);
/// True once an update has been downloaded and staged for the next relaunch.
pub static UPDATE_STAGED: AtomicBool = AtomicBool::new(false);
/// True once a newer version has been found (drives the tray "U" indicator).
pub static UPDATE_AVAILABLE: AtomicBool = AtomicBool::new(false);

/// Spawns a check-only update query. Records any newer version in AppState and
/// notifies once per version. Never downloads — the user installs explicitly.
pub fn spawn_update_check(app: &tauri::AppHandle) {
    if UPDATE_STAGED.load(Ordering::SeqCst) {
        return;
    }
    if UPDATE_CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        tauri::async_runtime::block_on(async {
            let Ok(updater) = app.updater() else {
                return;
            };
            match updater.check().await {
                Ok(Some(update)) => {
                    let version = update.version.clone();
                    // Embed the version in the URL only if it looks like a plain
                    // semver string (not from latest.json, which is untrusted).
                    let url = if version
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
                    {
                        format!("https://github.com/jordan-temim/cctide/releases/tag/v{version}")
                    } else {
                        "https://github.com/jordan-temim/cctide/releases/latest".to_string()
                    };
                    let info = UpdateInfo {
                        // Release notes from latest.json — not rendered as HTML.
                        notes: update.body.clone(),
                        url,
                        version: version.clone(),
                    };
                    *app.state::<AppState>()
                        .available_update
                        .lock()
                        .expect("available_update poisoned") = Some(info);
                    UPDATE_AVAILABLE.store(true, Ordering::SeqCst);
                    let _ = app.emit("UPDATE_AVAILABLE", ());
                    // Immediately redraw the tray icon so the "U" glyph appears
                    // without waiting for the next ticker cycle. The OS
                    // notification is fired separately by the ticker loop (see
                    // NotifyState::check_update) — sending it here, in the
                    // first seconds after launch, had it dropped by macOS.
                    let app_tick = app.clone();
                    std::thread::spawn(move || crate::tick::do_tick(&app_tick, &mut None, false));
                }
                // Up to date: clear any previously-found update.
                Ok(None) => {
                    UPDATE_AVAILABLE.store(false, Ordering::SeqCst);
                    *app.state::<AppState>()
                        .available_update
                        .lock()
                        .expect("available_update poisoned") = None;
                }
                Err(_) => {}
            }
        });
        UPDATE_CHECKING.store(false, Ordering::SeqCst);
    });
}

/// Downloads and installs the staged update (user-initiated).
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    if UPDATE_CHECKING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("update check already in progress".into());
    }
    let result = async {
        let updater = app.updater().map_err(|e| e.to_string())?;
        let update = updater
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no update available".to_string())?;
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
        UPDATE_STAGED.store(true, Ordering::SeqCst);
        Ok(())
    }
    .await;
    UPDATE_CHECKING.store(false, Ordering::SeqCst);
    result
}

/// Relaunches the app to apply a staged update.
#[tauri::command]
pub fn restart_app(app: tauri::AppHandle) {
    app.restart();
}
