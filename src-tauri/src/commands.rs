//! Tauri IPC commands — thin translation layer between the frontend and the
//! domain modules. No business logic here; only orchestration and serialization.

use crate::state::{now_ts, refresh_cache, refresh_system, refreshed_points, AppState};
use crate::tick::do_tick;
use crate::{config, context, memory, usage};

// ---------------------------------------------------------------------------
// Panel data — single command that refreshes everything once and returns all
// data the UI needs. Avoids multiple IPC round-trips per refresh cycle and
// ensures all numbers share the same `now` timestamp.
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub(crate) struct ModelSeries {
    model: String,
    weighted: f64,
}

#[derive(serde::Serialize)]
pub(crate) struct DayBucket {
    label: String,
    by_model: Vec<ModelSeries>,
    is_today: bool,
}

#[derive(serde::Serialize)]
pub(crate) struct PanelData {
    session: usage::SessionUsage,
    weekly: usage::WeeklyUsage,
    sessions: Vec<context::SessionCtx>,
    chart: Vec<DayBucket>,
    config: config::Config,
    update: Option<crate::state::UpdateInfo>,
    rtk: Option<crate::rtk::RtkSavings>,
}

#[tauri::command]
pub fn get_panel_data(state: tauri::State<AppState>) -> PanelData {
    let cfg = state
        .config_cache
        .lock()
        .expect("config_cache poisoned")
        .clone();

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

    let today_start = {
        use chrono::{Local, TimeZone};
        let today = Local::now().date_naive();
        Local
            .from_local_datetime(&today.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|d| d.timestamp())
            .unwrap_or(0)
    };
    let chart: Vec<DayBucket> = if let Some(ws) = weekly.week_start {
        use chrono::{Local, TimeZone};
        usage::daily_buckets(&points, ws, now)
            .into_iter()
            .map(|(day_ts, by_model)| {
                let label = Local
                    .timestamp_opt(day_ts, 0)
                    .single()
                    .map(|d| d.format("%a").to_string())
                    .unwrap_or_else(|| "?".to_string());
                let mut series: Vec<ModelSeries> = by_model
                    .into_iter()
                    .map(|(model, weighted)| ModelSeries { model, weighted })
                    .collect();
                series.sort_by(|a, b| a.model.cmp(&b.model));
                DayBucket {
                    label,
                    by_model: series,
                    is_today: day_ts == today_start,
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let update = state
        .available_update
        .lock()
        .expect("available_update poisoned")
        .clone();
    let rtk = state.rtk_cache.lock().expect("rtk_cache poisoned").clone();

    PanelData {
        session,
        weekly,
        sessions: active,
        chart,
        config: cfg,
        update,
        rtk,
    }
}

// Memory is loaded lazily (on section open), not on every panel refresh.
#[tauri::command]
pub fn get_memory(state: tauri::State<AppState>) -> Vec<memory::MemoryFile> {
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

// One-time setup calls (calibration, notifications, tracking toggles) that
// don't need a full panel refresh.
#[tauri::command]
pub fn get_config(state: tauri::State<AppState>) -> config::Config {
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
pub fn set_tracking(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    enabled: bool,
) -> Result<(), String> {
    let mut lock = state.config_cache.lock().expect("config_cache poisoned");
    lock.tracking_enabled = enabled;
    config::save(&lock)?;
    drop(lock);
    std::thread::spawn(move || do_tick(&app, &mut None, true));
    Ok(())
}

#[tauri::command]
pub fn set_notifications(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    enabled: bool,
    levels: Vec<f64>,
) -> Result<(), String> {
    let mut lock = state.config_cache.lock().expect("config_cache poisoned");
    lock.notifications_enabled = enabled;
    lock.alert_levels = config::sanitize_levels(&levels);
    config::save(&lock)?;
    drop(lock);
    std::thread::spawn(move || do_tick(&app, &mut None, true));
    Ok(())
}

#[tauri::command]
pub fn set_calibration(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    session_percent: Option<f64>,
    weekly_percent: Option<f64>,
    reset_date: Option<String>,
) -> Result<(), String> {
    let mut cfg = state
        .config_cache
        .lock()
        .expect("config_cache poisoned")
        .clone();
    let now = now_ts();

    if let Some(date) = reset_date {
        cfg.weekly_reset_date = Some(date);
    }

    let points = refreshed_points(&state);

    if let Some(pct) = session_percent {
        if !pct.is_finite() {
            return Err("invalid session percent".into());
        }
        let pct = pct.clamp(0.0, 100.0);
        if pct == 0.0 {
            return Err("session percent must be greater than 0".into());
        }
        let s = usage::session_usage(&points, &cfg, now);
        cfg.session_calibration = Some(config::Calibration {
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
        if pct == 0.0 {
            return Err("weekly percent must be greater than 0".into());
        }
        let w = usage::weekly_usage(&points, &cfg, now);
        cfg.weekly_calibration = Some(config::Calibration {
            percent: pct,
            budget: usage::budget_from_percent(w.weighted_tokens, pct),
            calibrated_at: now,
        });
    }

    config::save(&cfg)?;
    *state.config_cache.lock().expect("config_cache poisoned") = cfg;
    std::thread::spawn(move || do_tick(&app, &mut None, true));
    Ok(())
}
