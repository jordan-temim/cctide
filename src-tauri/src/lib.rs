//! cctide — offline tracking of Claude Code consumption (tray app).

mod config;
mod context;
mod icon;
mod memory;
mod models;
mod notify;
mod rtk;
mod scan;
mod usage;

use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_positioner::{Position, WindowExt};
use tauri_plugin_updater::UpdaterExt;

use config::{Calibration, Config};
use notify::NotifyState;
use scan::ScanCache;

/// Tray-icon alert state (for the blink-until-acknowledged behaviour).
#[derive(Default)]
struct IconState {
    /// Current max alert tier across session/weekly (0..3).
    current_tier: u8,
    /// Highest tier the user has "seen" (opening the panel acknowledges).
    acknowledged_tier: u8,
}

/// Shared state: cache of parsed transcripts + notification + icon state.
struct AppState {
    cache: Mutex<ScanCache>,
    notify_state: Mutex<NotifyState>,
    icon_state: Mutex<IconState>,
    /// Config cached in memory; refreshed from disk every `refresh_secs`.
    config_cache: Mutex<Config>,
    /// Model table loaded once at startup (embedded JSON, immutable).
    models: models::Models,
    /// Cached sysinfo handle; refreshed only when needed.
    system: Mutex<sysinfo::System>,
}

/// (Re)parses JSONL files that changed on disk.
fn refresh_cache(state: &tauri::State<AppState>) {
    state
        .cache
        .lock()
        .expect("cache poisoned")
        .refresh(&state.models);
}

/// Refreshes process list in the cached sysinfo handle.
fn refresh_system(state: &tauri::State<AppState>) {
    state
        .system
        .lock()
        .expect("system poisoned")
        .refresh_processes(sysinfo::ProcessesToUpdate::All, false);
}

/// Refreshes the cache and returns deduplicated consumption points.
fn refreshed_points(state: &tauri::State<AppState>) -> Vec<scan::Point> {
    refresh_cache(state);
    state.cache.lock().expect("cache poisoned").all_points()
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------------------------------------------------------------------------
// Panel data — single command that refreshes everything once and returns all
// data the UI needs. Avoids 5 separate IPC round-trips per refresh cycle and
// ensures all numbers share the same `now` timestamp.
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct ModelUsage {
    model: String,
    tokens: u64,
}

#[derive(serde::Serialize)]
struct PanelData {
    session: usage::SessionUsage,
    weekly: usage::WeeklyUsage,
    sessions: Vec<context::SessionCtx>,
    models: Vec<ModelUsage>,
    config: Config,
}

#[tauri::command]
fn get_panel_data(state: tauri::State<AppState>) -> PanelData {
    let cfg = state
        .config_cache
        .lock()
        .expect("config_cache poisoned")
        .clone();

    // One refresh of both caches for the entire panel.
    refresh_cache(&state);
    refresh_system(&state);

    let now = now_ts();

    // Hold both locks for the remainder of the reads so data is consistent.
    let cache = state.cache.lock().expect("cache poisoned");
    let sys = state.system.lock().expect("system poisoned");

    let points = cache.all_points();
    let session = usage::session_usage(&points, &cfg, now);
    let weekly = usage::weekly_usage(&points, &cfg, now);

    let active = context::active_sessions(&cache, &cfg, &sys, &state.models);

    let models = cache
        .model_totals(weekly.week_start)
        .into_iter()
        .map(|(model, tokens)| ModelUsage { model, tokens })
        .collect();

    PanelData {
        session,
        weekly,
        sessions: active,
        models,
        config: cfg,
    }
}

// Memory is loaded lazily (on section open), not on every panel refresh.
#[tauri::command]
fn get_memory(state: tauri::State<AppState>) -> Vec<memory::MemoryFile> {
    let cfg = state
        .config_cache
        .lock()
        .expect("config_cache poisoned")
        .clone();
    refresh_cache(&state);
    refresh_system(&state);
    let cache = state.cache.lock().expect("cache poisoned");
    let sys = state.system.lock().expect("system poisoned");
    let cwds: Vec<String> = context::active_sessions(&cache, &cfg, &sys, &state.models)
        .into_iter()
        .map(|s| s.cwd)
        .collect();
    memory::read_memory(&cache, &cwds)
}

#[tauri::command]
fn get_rtk_savings() -> Option<rtk::RtkSavings> {
    rtk::savings()
}

// get_config is kept for the one-time setup calls (calibration, notification,
// tracking toggles) that run at startup and don't need a full panel refresh.
#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Config {
    state
        .config_cache
        .lock()
        .expect("config_cache poisoned")
        .clone()
}

// ---------------------------------------------------------------------------
// Mutations — hold the lock for the full read-modify-write to prevent races.
// ---------------------------------------------------------------------------

#[tauri::command]
fn set_tracking(state: tauri::State<AppState>, enabled: bool) -> Result<(), String> {
    let mut lock = state.config_cache.lock().expect("config_cache poisoned");
    lock.tracking_enabled = enabled;
    config::save(&lock)
}

#[tauri::command]
fn set_notifications(
    state: tauri::State<AppState>,
    enabled: bool,
    levels: Vec<f64>,
) -> Result<(), String> {
    let mut lock = state.config_cache.lock().expect("config_cache poisoned");
    lock.notifications_enabled = enabled;
    lock.alert_levels = config::sanitize_levels(&levels);
    config::save(&lock)
}

/// Calibrates the bars from the % reported in Claude Code's `/usage`.
#[tauri::command]
fn set_calibration(
    state: tauri::State<AppState>,
    session_percent: Option<f64>,
    weekly_percent: Option<f64>,
    reset_date: Option<String>,
) -> Result<(), String> {
    // Clone current config so we can release the lock before touching the cache.
    let mut cfg = state
        .config_cache
        .lock()
        .expect("config_cache poisoned")
        .clone();
    let now = now_ts();

    // The reset date must be set before computing the weekly window.
    if let Some(date) = reset_date {
        cfg.weekly_reset_date = Some(date);
    }

    let points = refreshed_points(&state);

    if let Some(pct) = session_percent {
        if !pct.is_finite() {
            return Err("invalid session percent".into());
        }
        let pct = pct.clamp(0.0, 100.0);
        let s = usage::session_usage(&points, &cfg, now);
        // Promote: current cal_1 → cal_2 (keep the two most recent points).
        cfg.session_calibration_2 = cfg.session_calibration.take();
        cfg.session_calibration = Some(Calibration {
            percent: pct,
            budget: usage::budget_from_percent(s.weighted_tokens, pct),
            calibrated_at: now,
        });
    }

    if let Some(pct) = weekly_percent {
        if !pct.is_finite() {
            return Err("invalid weekly percent".into());
        }
        let pct = pct.clamp(0.0, 100.0);
        let w = usage::weekly_usage(&points, &cfg, now);
        cfg.weekly_calibration_2 = cfg.weekly_calibration.take();
        cfg.weekly_calibration = Some(Calibration {
            percent: pct,
            budget: usage::budget_from_percent(w.weighted_tokens, pct),
            calibrated_at: now,
        });
    }

    config::save(&cfg)?;
    *state.config_cache.lock().expect("config_cache poisoned") = cfg;
    Ok(())
}

// ---------------------------------------------------------------------------
// Window management
// ---------------------------------------------------------------------------

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
            // Opening the panel acknowledges the current alert (stops blinking).
            if let Some(state) = app.try_state::<AppState>() {
                let mut ist = state.icon_state.lock().expect("icon_state poisoned");
                ist.acknowledged_tier = ist.current_tier;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            cache: Mutex::new(ScanCache::default()),
            notify_state: Mutex::new(NotifyState::default()),
            icon_state: Mutex::new(IconState::default()),
            config_cache: Mutex::new(config::load()),
            models: models::load(),
            system: Mutex::new(sysinfo::System::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_panel_data,
            get_memory,
            get_rtk_savings,
            get_config,
            set_calibration,
            set_notifications,
            set_tracking,
        ])
        .setup(|app| {
            // Menu bar app: no Dock icon (macOS).
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Dedicated monochrome template for the menu bar (auto-tinted by
            // macOS for light/dark); the colour app icon would render as a
            // solid blob under template mode.
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

            // Request OS notification permission once (without it, .show() is a
            // silent no-op on macOS).
            if !matches!(
                app.notification().permission_state(),
                Ok(tauri_plugin_notification::PermissionState::Granted)
            ) {
                let _ = app.notification().request_permission();
            }

            // Background update check: download silently, notify when ready.
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let Ok(updater) = update_handle.updater() else {
                    return;
                };
                let Ok(Some(update)) = updater.check().await else {
                    return;
                };
                let version = update.version.clone();
                if update.download_and_install(|_, _| {}, || {}).await.is_ok() {
                    let _ = update_handle
                        .notification()
                        .builder()
                        .title("cctide updated")
                        .body(format!("v{version} ready — quit and reopen to apply"))
                        .show();
                }
            });

            // Icon thread: renders the live CC-gauge tray icon and (macOS) drives
            // the blink-until-acknowledged alert. Ticks fast for smooth blinking;
            // recomputes usage only every `refresh_secs`.
            let ih = app.handle().clone();
            std::thread::spawn(move || {
                const TICK_MS: u64 = 400;
                // Shimmer: a small notch sweeps both C arcs for SHIMMER_TICKS after each recompute.
                const SHIMMER_TICKS: u64 = 5; // ~2 s
                let mut tick: u64 = 0;
                let mut fills = (0.0_f64, 0.0_f64);
                let mut tiers = (0u8, 0u8);
                let mut last_sig: Option<(i32, i32, u8, u8, bool, i32)> = None;
                let mut last_disabled: Option<bool> = None;
                let mut shimmer_start: Option<u64> = None;

                loop {
                    let mut cfg = ih
                        .state::<AppState>()
                        .config_cache
                        .lock()
                        .expect("config_cache poisoned")
                        .clone();

                    if !cfg.tracking_enabled {
                        if last_disabled != Some(true) {
                            let rendered = icon::render(&icon::IconParams {
                                session_fill: 0.0,
                                weekly_fill: 0.0,
                                session_tier: 0,
                                weekly_tier: 0,
                                blink_off: false,
                                disabled: true,
                                shimmer_pos: None,
                            });
                            let img = tauri::image::Image::new_owned(
                                rendered.rgba,
                                rendered.width,
                                rendered.height,
                            );
                            if let Some(tray) = ih.tray_by_id("cctide-tray") {
                                let _ = tray.set_icon(Some(img));
                            }
                            last_disabled = Some(true);
                            last_sig = None;
                        }
                        tick = tick.wrapping_add(1);
                        std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
                        continue;
                    }
                    last_disabled = Some(false);

                    let recompute_every = (cfg.refresh_secs.max(5) * 1000 / TICK_MS).max(1);
                    if tick.is_multiple_of(recompute_every) {
                        shimmer_start = Some(tick);
                        let state = ih.state::<AppState>();
                        // Reload config from disk once per recompute cycle to pick
                        // up any external edits to cctide.json.
                        cfg = config::load();
                        *state.config_cache.lock().expect("config_cache poisoned") = cfg.clone();
                        let now = now_ts();
                        let points = refreshed_points(&state);
                        let session = usage::session_usage(&points, &cfg, now);
                        let weekly = usage::weekly_usage(&points, &cfg, now);
                        fills = (
                            session
                                .percent
                                .map(|p| (p / 100.0).clamp(0.0, 1.0))
                                .unwrap_or(0.0),
                            weekly
                                .percent
                                .map(|p| (p / 100.0).clamp(0.0, 1.0))
                                .unwrap_or(0.0),
                        );
                        tiers = (
                            config::level_for(session.percent, &cfg.alert_levels),
                            config::level_for(weekly.percent, &cfg.alert_levels),
                        );
                        {
                            let mut ist = state.icon_state.lock().expect("icon_state poisoned");
                            ist.current_tier = tiers.0.max(tiers.1);
                            // Dropping below the acknowledged tier re-arms future climbs.
                            if ist.current_tier < ist.acknowledged_tier {
                                ist.acknowledged_tier = ist.current_tier;
                            }
                        }
                        // System notifications at level crossings (independent of
                        // the icon; gated by `notifications_enabled` inside).
                        {
                            let mut ns = state.notify_state.lock().expect("notify_state poisoned");
                            ns.check(&ih, &cfg, &session, &weekly);
                        }
                    }

                    // Render/blink the icon only when the dynamic icon is on;
                    // notifications above run regardless.
                    if cfg.dynamic_icon {
                        // Shimmer: small notch sweeping both C arcs after each recompute.
                        let shimmer_pos = shimmer_start.and_then(|start| {
                            let elapsed = tick.wrapping_sub(start);
                            if elapsed < SHIMMER_TICKS {
                                Some(elapsed as f64 / SHIMMER_TICKS as f64)
                            } else {
                                None
                            }
                        });
                        let shimmer_sig = shimmer_pos.map(|p| (p * 100.0) as i32).unwrap_or(-1);

                        let max_tier = tiers.0.max(tiers.1);
                        let blink_off = {
                            #[cfg(target_os = "macos")]
                            {
                                let state = ih.state::<AppState>();
                                let ist = state.icon_state.lock().expect("icon_state poisoned");
                                if max_tier > 0 && ist.current_tier > ist.acknowledged_tier {
                                    let cadence = match max_tier {
                                        3 => 1, // ~0.8s period (strong)
                                        2 => 3, // ~2.4s
                                        _ => 8, // ~6.4s (slow, subtle)
                                    };
                                    (tick / cadence) % 2 == 1
                                } else {
                                    false
                                }
                            }
                            #[cfg(not(target_os = "macos"))]
                            {
                                let _ = max_tier;
                                false
                            }
                        };

                        let sig = (
                            (fills.0 * 200.0) as i32,
                            (fills.1 * 200.0) as i32,
                            tiers.0,
                            tiers.1,
                            blink_off,
                            shimmer_sig,
                        );
                        if last_sig != Some(sig) {
                            let rendered = icon::render(&icon::IconParams {
                                session_fill: fills.0,
                                weekly_fill: fills.1,
                                session_tier: tiers.0,
                                weekly_tier: tiers.1,
                                blink_off,
                                disabled: false,
                                shimmer_pos,
                            });
                            if let Some(tray) = ih.tray_by_id("cctide-tray") {
                                let img = tauri::image::Image::new_owned(
                                    rendered.rgba,
                                    rendered.width,
                                    rendered.height,
                                );
                                let _ = tray.set_icon(Some(img));
                                #[cfg(target_os = "macos")]
                                let _ = tray.set_icon_as_template(true);
                            }
                            last_sig = Some(sig);
                        }
                    }

                    tick = tick.wrapping_add(1);
                    std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
                }
            });

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
    // `clear` is kept alive for the duration of the setBackgroundColor call.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    unsafe {
        let ns_win = &*(ptr as *const NSWindow);
        ns_win.setOpaque(false);
        let clear: Retained<NSColor> = NSColor::clearColor();
        ns_win.setBackgroundColor(Some(&*clear));
        drop(clear); // explicit: NSWindow retains it internally, we release our ref
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
