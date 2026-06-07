//! Background ticker: recomputes usage, updates the tray icon, emits the
//! "refresh" event every `refresh_secs`. Also plays the shimmer animation.

use std::sync::atomic::Ordering;

use tauri::Emitter;

use tauri::Manager;

use crate::state::{now_ts, refreshed_points, AppState};
use crate::update_svc::UPDATE_AVAILABLE;
use crate::{config, icon, rtk, usage};

pub const SHIMMER_FRAMES: usize = 5;
pub const SHIMMER_MS: u64 = 400;

/// One recompute cycle: reads config_cache, refreshes scan, renders icon,
/// fires OS notifications, emits "refresh". No sleep.
///
/// `last_disabled_sig` avoids redundant icon redraws when tracking is off;
/// pass `&mut None` for one-shot calls that always want a fresh render.
pub fn do_tick(app: &tauri::AppHandle, last_disabled_sig: &mut Option<(bool, bool)>, shimmer: bool) {
    let state = app.state::<AppState>();
    let cfg = state.config_cache.lock().expect("config_cache poisoned").clone();

    if !cfg.tracking_enabled {
        let upd = UPDATE_AVAILABLE.load(Ordering::SeqCst);
        let sig = (true, upd);
        if last_disabled_sig.as_ref() != Some(&sig) {
            let rendered = icon::render(&icon::IconParams {
                session_fill: 0.0,
                weekly_fill: 0.0,
                session_tier: 0,
                weekly_tier: 0,
                disabled: true,
                shimmer_pos: None,
                update_available: upd,
            });
            if let Some(tray) = app.tray_by_id("cctide-tray") {
                let _ = tray.set_icon(Some(tauri::image::Image::new_owned(
                    rendered.rgba,
                    rendered.width,
                    rendered.height,
                )));
            }
            *last_disabled_sig = Some(sig);
        }
        return;
    }
    *last_disabled_sig = Some((false, false));

    let now = now_ts();
    let points = refreshed_points(&state);
    let session = usage::session_usage(&points, &cfg, now);
    let weekly = usage::weekly_usage(&points, &cfg, now);
    let fills = (
        session.percent.map(|p| (p / 100.0).clamp(0.0, 1.0)).unwrap_or(0.0),
        weekly.percent.map(|p| (p / 100.0).clamp(0.0, 1.0)).unwrap_or(0.0),
    );
    let tiers = (
        config::level_for(session.percent, &cfg.alert_levels),
        config::level_for(weekly.percent, &cfg.alert_levels),
    );
    state
        .notify_state
        .lock()
        .expect("notify_state poisoned")
        .check(app, &cfg, &session, &weekly);
    *state.rtk_cache.lock().expect("rtk_cache poisoned") = rtk::savings();
    app.emit("refresh", ()).ok();

    if cfg.dynamic_icon {
        let update_available = UPDATE_AVAILABLE.load(Ordering::SeqCst);
        let frames = if shimmer { SHIMMER_FRAMES } else { 0 };
        for frame in 0..=frames {
            let shimmer_pos = if frame < frames {
                Some(frame as f64 / frames as f64)
            } else {
                None
            };
            let rendered = icon::render(&icon::IconParams {
                session_fill: fills.0,
                weekly_fill: fills.1,
                session_tier: tiers.0,
                weekly_tier: tiers.1,
                disabled: false,
                shimmer_pos,
                update_available,
            });
            if let Some(tray) = app.tray_by_id("cctide-tray") {
                let img = tauri::image::Image::new_owned(
                    rendered.rgba,
                    rendered.width,
                    rendered.height,
                );
                let _ = tray.set_icon(Some(img));
                #[cfg(target_os = "macos")]
                let _ = tray.set_icon_as_template(true);
            }
            if frame < frames {
                std::thread::sleep(std::time::Duration::from_millis(SHIMMER_MS));
                // Abort shimmer if tracking was disabled while we were sleeping.
                if !state
                    .config_cache
                    .lock()
                    .expect("config_cache poisoned")
                    .tracking_enabled
                {
                    break;
                }
            }
        }
    }
}

/// Spawns the background ticker thread. Reloads config from disk each cycle
/// so external edits are picked up without restarting the app.
pub fn start_ticker(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut last_disabled_sig: Option<(bool, bool)> = None;
        loop {
            {
                let state = app.state::<AppState>();
                *state.config_cache.lock().expect("config_cache poisoned") = config::load();
            }
            do_tick(&app, &mut last_disabled_sig, true);
            let shimmer_elapsed_ms = SHIMMER_FRAMES as u64 * SHIMMER_MS;
            let state = app.state::<AppState>();
            let refresh_ms = state
                .config_cache
                .lock()
                .expect("config_cache poisoned")
                .refresh_secs
                .max(5)
                * 1000;
            std::thread::sleep(std::time::Duration::from_millis(
                refresh_ms.saturating_sub(shimmer_elapsed_ms),
            ));
        }
    });
}
