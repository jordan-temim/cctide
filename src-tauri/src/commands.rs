//! Tauri IPC commands — thin translation layer between the frontend and the
//! domain modules. No business logic here; only orchestration and serialization.

use crate::state::{now_ts, refresh_cache, refresh_system, refreshed_points, AppState};
use crate::{config, context, memory, outcome, tick, usage};

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
    cost_usd: f64,
    breakdown: usage::DayBreakdown,
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
    /// Unique project cwds with activity in the weekly window (for the filter dropdown).
    projects: Vec<String>,
}

#[tauri::command]
pub fn get_panel_data(state: tauri::State<AppState>, project_filter: Option<String>) -> PanelData {
    let cfg = state
        .config_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    refresh_cache(&state);
    refresh_system(&state);

    let now = now_ts();

    // Hold both locks for the remainder of the reads so data is consistent.
    let cache = state.cache.lock().unwrap_or_else(|e| e.into_inner());
    let sys = state.system.lock().unwrap_or_else(|e| e.into_inner());

    // Gauges always use all projects; chart respects the filter.
    let all_points = cache.all_points();
    let session = usage::session_usage(&all_points, &cfg, now);
    let weekly = usage::weekly_usage(&all_points, &cfg, now);
    let active =
        context::active_sessions(&cache, &cfg, &sys, &state.models, session.window_start, now);

    let weekly_start = weekly.week_start.unwrap_or(now - 7 * 86_400);
    let projects = cache.project_cwds_in_window(weekly_start, now);

    let chart_points: std::sync::Arc<[crate::scan::Point]> = match &project_filter {
        Some(cwd) => cache.points_for_project(cwd).into(),
        None => all_points,
    };

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
        usage::daily_buckets(&chart_points, ws, now)
            .into_iter()
            .map(|bucket| {
                let label = Local
                    .timestamp_opt(bucket.day_start, 0)
                    .single()
                    .map(|d| d.format("%a").to_string())
                    .unwrap_or_else(|| "?".to_string());
                let mut series: Vec<ModelSeries> = bucket
                    .by_model
                    .into_iter()
                    .map(|(model, weighted)| ModelSeries { model, weighted })
                    .collect();
                series.sort_by(|a, b| a.model.cmp(&b.model));
                DayBucket {
                    label,
                    by_model: series,
                    is_today: bucket.day_start == today_start,
                    cost_usd: bucket.cost_usd,
                    breakdown: bucket.breakdown,
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let update = state
        .available_update
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let rtk = state
        .rtk_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    PanelData {
        session,
        weekly,
        sessions: active,
        chart,
        config: cfg,
        update,
        rtk,
        projects,
    }
}

// Outcomes are computed lazily (on section open) and cached: classification
// shells out to `git log` per repo, far too heavy for the refresh poll.
const OUTCOME_TTL_SECS: i64 = 300;

#[tauri::command]
pub fn get_outcomes(
    state: tauri::State<AppState>,
    project_filter: Option<String>,
) -> outcome::OutcomeReport {
    let now = now_ts();
    // Cache is only used for unfiltered queries (filter results vary per cwd).
    if project_filter.is_none() {
        if let Some((computed_at, report)) = state
            .outcome_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if now - computed_at < OUTCOME_TTL_SECS {
                return report.clone();
            }
        }
    }

    let cfg = state
        .config_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    refresh_cache(&state);
    // Same window as the rest of the Analytics tab; fall back to a rolling
    // 7 days when no weekly reset date is configured.
    let (window_start, window_end) = cfg
        .weekly_reset_date
        .as_deref()
        .and_then(|d| usage::week_window_from_reset(d, now))
        .unwrap_or((now - 7 * 86_400, now));
    let all_spans = state
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .session_edit_spans(window_start, now);
    let spans: Vec<_> = match &project_filter {
        Some(cwd) => all_spans
            .into_iter()
            .filter(|s| s.cwd.as_deref() == Some(cwd.as_str()))
            .collect(),
        None => all_spans,
    };
    let report = outcome::outcome_report(&spans, window_start, window_end.min(now));
    if project_filter.is_none() {
        *state
            .outcome_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((now, report.clone()));
    }
    report
}

// Memory is loaded lazily (on section open), not on every panel refresh.
#[tauri::command]
pub fn get_memory(
    state: tauri::State<AppState>,
    project_filter: Option<String>,
) -> Vec<memory::MemoryFile> {
    let cfg = state
        .config_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    refresh_cache(&state);
    refresh_system(&state);
    let cache = state.cache.lock().unwrap_or_else(|e| e.into_inner());
    let sys = state.system.lock().unwrap_or_else(|e| e.into_inner());
    let mut cwds: Vec<String> =
        context::active_sessions(&cache, &cfg, &sys, &state.models, None, now_ts())
            .into_iter()
            .map(|s| s.cwd)
            .collect();
    // Narrow to the selected project when the Sessions tab's filter is active,
    // so Memory mirrors the same cwd as the open-sessions list above it.
    if let Some(filter) = project_filter.as_deref() {
        cwds.retain(|c| c == filter);
    }
    memory::read_memory(&cache, &cwds)
}

// One-time setup calls (calibration, notifications, tracking toggles) that
// don't need a full panel refresh.
#[tauri::command]
pub fn get_config(state: tauri::State<AppState>) -> config::Config {
    state
        .config_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

// ---------------------------------------------------------------------------
// Session management (Sessions tab) — kill / delete / clean up. Every path is
// resolved server-side from ids, never taken verbatim from the frontend, except
// memory files whose paths are validated against the projects tree.
// ---------------------------------------------------------------------------

/// Terminates a running Claude Code session process (graceful TERM where
/// supported, hard kill otherwise). The pid must belong to a declared session.
#[tauri::command]
pub fn kill_session(state: tauri::State<AppState>, pid: u32) -> Result<(), String> {
    if !context::is_session_pid(pid) {
        return Err("pid does not belong to a Claude Code session".into());
    }
    refresh_system(&state);
    let sys = state.system.lock().unwrap_or_else(|e| e.into_inner());
    let proc = sys
        .process(sysinfo::Pid::from_u32(pid))
        .ok_or("process already gone")?;
    let killed = proc
        .kill_with(sysinfo::Signal::Term)
        .unwrap_or_else(|| proc.kill());
    if killed {
        Ok(())
    } else {
        Err("failed to terminate the process".into())
    }
}

/// Deletes a session's transcript (`<sessionId>.jsonl`). The file is resolved
/// from the scan cache, so only files inside `~/.claude/projects` can match.
/// The caller is responsible for having warned the user when the session has
/// activity in the current 5h window (the gauge will under-count until reset).
#[tauri::command]
pub fn delete_session_transcript(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    session_id: String,
) -> Result<(), String> {
    refresh_cache(&state);
    let path = {
        let cache = state.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .jsonl_for_session(&session_id)
            .ok_or("transcript not found")?
    };
    std::fs::remove_file(&path).map_err(|e| format!("delete failed: {e}"))?;
    // do_tick refreshes the cache itself, so this single spawned call both
    // picks up the deletion and re-renders the icon — no need for a second
    // explicit refresh_cache() here.
    tick::trigger_tick(app);
    Ok(())
}

/// Removes `~/.claude/sessions/<pid>.json` files whose process is gone.
/// Returns the number of files removed.
#[tauri::command]
pub fn cleanup_stale_sessions(state: tauri::State<AppState>) -> Result<u32, String> {
    refresh_system(&state);
    let sys = state.system.lock().unwrap_or_else(|e| e.into_inner());
    context::cleanup_stale_sessions(&sys)
}

/// Deletes one project memory file. The path must resolve to a real `.md` file
/// inside `~/.claude/projects/<project>/memory/`. When an index (`MEMORY.md`)
/// sits next to it, its line referencing the file is dropped (best effort).
#[tauri::command]
pub fn delete_memory_file(path: String) -> Result<(), String> {
    memory::delete_memory_file(&path)
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
    let mut lock = state.config_cache.lock().unwrap_or_else(|e| e.into_inner());
    lock.tracking_enabled = enabled;
    config::save(&lock)?;
    drop(lock);
    tick::trigger_tick(app);
    Ok(())
}

#[tauri::command]
pub fn set_notifications(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    enabled: bool,
    levels: Vec<f64>,
) -> Result<(), String> {
    let mut lock = state.config_cache.lock().unwrap_or_else(|e| e.into_inner());
    lock.notifications_enabled = enabled;
    lock.alert_levels = config::sanitize_levels(&levels);
    config::save(&lock)?;
    drop(lock);
    tick::trigger_tick(app);
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
        .unwrap_or_else(|e| e.into_inner())
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
    *state.config_cache.lock().unwrap_or_else(|e| e.into_inner()) = cfg;
    tick::trigger_tick(app);
    Ok(())
}
