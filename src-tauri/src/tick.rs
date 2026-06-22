//! Background ticker: recomputes usage, updates the tray icon and title, emits
//! the "refresh" event every `refresh_secs`. Also plays the shimmer animation.
//!
//! **Tray title**: when a 5h session is live, `do_tick` sets the macOS menubar
//! title (text to the right of the CC icon) to the session's reset time in
//! `HH:MM` local format (`tray.set_title`). The title is cleared when no
//! session is active or when tracking is disabled.

use std::sync::atomic::Ordering;

use chrono::{Local, TimeZone, Timelike};
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
pub fn do_tick(
    app: &tauri::AppHandle,
    last_disabled_sig: &mut Option<(bool, bool)>,
    shimmer: bool,
) {
    let state = app.state::<AppState>();
    let cfg = state
        .config_cache
        .lock()
        .expect("config_cache poisoned")
        .clone();

    if !cfg.tracking_enabled {
        let upd = UPDATE_AVAILABLE.load(Ordering::SeqCst);
        let sig = (true, upd);
        if last_disabled_sig.as_ref() != Some(&sig) {
            let rendered = icon::render(&icon::IconParams {
                session_fill: 0.0,
                weekly_fill: 0.0,
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
                let _ = tray.set_title(None::<&str>);
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
        session
            .percent
            .map(|p| (p / 100.0).clamp(0.0, 1.0))
            .unwrap_or(0.0),
        weekly
            .percent
            .map(|p| (p / 100.0).clamp(0.0, 1.0))
            .unwrap_or(0.0),
    );
    state
        .notify_state
        .lock()
        .expect("notify_state poisoned")
        .check(app, &cfg, &session, &weekly);
    *state.rtk_cache.lock().expect("rtk_cache poisoned") = rtk::savings();

    if let Some(tray) = app.tray_by_id("cctide-tray") {
        let _ = tray.set_title(reset_time_label(session.reset_at).as_deref());
    }

    app.emit("refresh", ()).ok();

    if cfg.dynamic_icon {
        let frames = if shimmer { SHIMMER_FRAMES } else { 0 };
        for frame in 0..=frames {
            // Re-read every frame: an update can be detected (and its tray redraw
            // fired) mid-shimmer. If we captured this once before the loop, the
            // trailing frames would overwrite the "U" glyph with the stale value,
            // hiding it until the next full ticker cycle (~refresh_secs later).
            let update_available = UPDATE_AVAILABLE.load(Ordering::SeqCst);
            let shimmer_pos = if frame < frames {
                Some(frame as f64 / frames as f64)
            } else {
                None
            };
            let rendered = icon::render(&icon::IconParams {
                session_fill: fills.0,
                weekly_fill: fills.1,
                disabled: false,
                shimmer_pos,
                update_available,
            });
            if let Some(tray) = app.tray_by_id("cctide-tray") {
                let img =
                    tauri::image::Image::new_owned(rendered.rgba, rendered.width, rendered.height);
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

/// Formats a 5h-window reset timestamp as `HH:MM` in the machine's local
/// timezone, or `None` when there is no live session.
fn reset_time_label(reset_at: Option<i64>) -> Option<String> {
    reset_at.and_then(|ts| {
        Local
            .timestamp_opt(ts, 0)
            .single()
            .map(|dt| format!("{:02}:{:02}", dt.hour(), dt.minute()))
    })
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
            // Fire the "update available" notification from here — the app is
            // fully initialized by the time the ticker runs, and a send dropped
            // by the OS is retried next cycle (check_update only marks a version
            // as announced on success).
            {
                let state = app.state::<AppState>();
                let version = if UPDATE_AVAILABLE.load(Ordering::SeqCst) {
                    state
                        .available_update
                        .lock()
                        .expect("available_update poisoned")
                        .as_ref()
                        .map(|u| u.version.clone())
                } else {
                    None
                };
                state
                    .notify_state
                    .lock()
                    .expect("notify_state poisoned")
                    .check_update(&app, version.as_deref());
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- reset_time_label ---

    #[test]
    fn reset_time_label_none_when_no_session() {
        assert_eq!(reset_time_label(None), None);
    }

    #[test]
    fn reset_time_label_returns_some_for_valid_timestamp() {
        // Any valid Unix timestamp must yield a label (not None).
        assert!(reset_time_label(Some(1_700_000_000)).is_some());
        assert!(reset_time_label(Some(0)).is_some());
    }

    #[test]
    fn reset_time_label_format_is_hh_mm() {
        // Output must be exactly "HH:MM" — 5 chars, colon at position 2,
        // both parts numeric and in valid range. Checked across several timestamps
        // to hit different hours/minutes regardless of the local timezone.
        for ts in [0i64, 3_600, 7_261, 1_700_000_000, 1_748_000_000] {
            let label = reset_time_label(Some(ts)).unwrap();
            assert_eq!(label.len(), 5, "wrong length for ts={ts}: '{label}'");
            assert_eq!(
                &label[2..3],
                ":",
                "no colon at position 2 for ts={ts}: '{label}'"
            );
            let hour: u32 = label[..2].parse().unwrap_or(99);
            let minute: u32 = label[3..].parse().unwrap_or(99);
            assert!(hour <= 23, "hour out of range for ts={ts}: '{label}'");
            assert!(minute <= 59, "minute out of range for ts={ts}: '{label}'");
        }
    }

    #[test]
    fn reset_time_label_zero_pads_single_digits() {
        // The format must always emit two digits for each part.
        // We can't know the local time for a given ts, but we can verify that
        // whatever the output is, it never looks like "9:05" or "14:5".
        for ts in [60i64, 61, 3_661, 7_200, 86_399] {
            let label = reset_time_label(Some(ts)).unwrap();
            // Both parts before and after the colon must be exactly 2 chars.
            let parts: Vec<&str> = label.splitn(2, ':').collect();
            assert_eq!(parts.len(), 2);
            assert_eq!(
                parts[0].len(),
                2,
                "hour not zero-padded for ts={ts}: '{label}'"
            );
            assert_eq!(
                parts[1].len(),
                2,
                "minute not zero-padded for ts={ts}: '{label}'"
            );
        }
    }
}
