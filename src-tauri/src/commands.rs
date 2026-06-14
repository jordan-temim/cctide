//! Tauri IPC commands — thin translation layer between the frontend and the
//! domain modules. No business logic here; only orchestration and serialization.

use crate::state::{now_ts, refresh_cache, refresh_system, refreshed_points, AppState};
use crate::tick::do_tick;
use crate::{config, context, memory, outcome, usage};

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
        .expect("config_cache poisoned")
        .clone();

    refresh_cache(&state);
    refresh_system(&state);

    let now = now_ts();

    // Hold both locks for the remainder of the reads so data is consistent.
    let cache = state.cache.lock().expect("cache poisoned");
    let sys = state.system.lock().expect("system poisoned");

    // Gauges always use all projects; chart respects the filter.
    let all_points = cache.all_points();
    let session = usage::session_usage(&all_points, &cfg, now);
    let weekly = usage::weekly_usage(&all_points, &cfg, now);
    let active =
        context::active_sessions(&cache, &cfg, &sys, &state.models, session.window_start, now);

    let weekly_start = weekly.week_start.unwrap_or(now - 7 * 86_400);
    let projects = cache.project_cwds_in_window(weekly_start, now);

    let chart_points = match &project_filter {
        Some(cwd) => cache.points_for_project(cwd),
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
            .map(|(day_ts, by_model, cost_usd, breakdown)| {
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
                    cost_usd,
                    breakdown,
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
            .expect("outcome_cache poisoned")
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
        .expect("config_cache poisoned")
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
        .expect("cache poisoned")
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
        *state.outcome_cache.lock().expect("outcome_cache poisoned") = Some((now, report.clone()));
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
        .expect("config_cache poisoned")
        .clone();
    refresh_cache(&state);
    refresh_system(&state);
    let cache = state.cache.lock().expect("cache poisoned");
    let sys = state.system.lock().expect("system poisoned");
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
        .expect("config_cache poisoned")
        .clone()
}

// ---------------------------------------------------------------------------
// Session management (Sessions tab) — kill / delete / clean up. Every path is
// resolved server-side from ids, never taken verbatim from the frontend, except
// memory files whose paths are validated against the projects tree.
// ---------------------------------------------------------------------------

/// True if `pid` is declared by a `~/.claude/sessions/<pid>.json` file — the
/// only processes this app is allowed to terminate.
fn is_session_pid(pid: u32) -> bool {
    let Some(dir) = context::sessions_dir() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .any(|v| v.get("pid").and_then(|p| p.as_u64()) == Some(pid as u64))
}

/// Terminates a running Claude Code session process (graceful TERM where
/// supported, hard kill otherwise). The pid must belong to a declared session.
#[tauri::command]
pub fn kill_session(state: tauri::State<AppState>, pid: u32) -> Result<(), String> {
    if !is_session_pid(pid) {
        return Err("pid does not belong to a Claude Code session".into());
    }
    refresh_system(&state);
    let sys = state.system.lock().expect("system poisoned");
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
        let cache = state.cache.lock().expect("cache poisoned");
        cache
            .jsonl_for_session(&session_id)
            .ok_or("transcript not found")?
    };
    std::fs::remove_file(&path).map_err(|e| format!("delete failed: {e}"))?;
    refresh_cache(&state);
    std::thread::spawn(move || do_tick(&app, &mut None, true));
    Ok(())
}

/// Removes `~/.claude/sessions/<pid>.json` files whose process is gone.
/// Returns the number of files removed.
#[tauri::command]
pub fn cleanup_stale_sessions(state: tauri::State<AppState>) -> Result<u32, String> {
    let dir = context::sessions_dir().ok_or("no home directory")?;
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("read failed: {e}"))?;
    refresh_system(&state);
    let sys = state.system.lock().expect("system poisoned");

    let mut removed = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(pid) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("pid").and_then(|p| p.as_u64()))
        else {
            continue;
        };
        if sys.process(sysinfo::Pid::from_u32(pid as u32)).is_none()
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Deletes one project memory file. The path must resolve to a real `.md` file
/// inside `~/.claude/projects/<project>/memory/`. When an index (`MEMORY.md`)
/// sits next to it, its line referencing the file is dropped (best effort).
#[tauri::command]
pub fn delete_memory_file(path: String) -> Result<(), String> {
    let canon = std::path::Path::new(&path)
        .canonicalize()
        .map_err(|_| "file not found")?;
    if canon.extension().and_then(|x| x.to_str()) != Some("md") {
        return Err("not a memory file".into());
    }
    let root = crate::scan::projects_dir()
        .ok_or("no home directory")?
        .canonicalize()
        .map_err(|_| "projects dir not found")?;
    let parent = canon.parent().ok_or("invalid path")?;
    if !canon.starts_with(&root) || parent.file_name().and_then(|n| n.to_str()) != Some("memory") {
        return Err("path is outside the memory directories".into());
    }

    let name = canon
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("invalid path")?
        .to_string();
    std::fs::remove_file(&canon).map_err(|e| format!("delete failed: {e}"))?;

    // Drop the deleted file's line from the MEMORY.md index, if present.
    if name != "MEMORY.md" {
        let index = parent.join("MEMORY.md");
        if let Ok(text) = std::fs::read_to_string(&index) {
            let _ = std::fs::write(&index, drop_index_lines(&text, &name));
        }
    }
    Ok(())
}

/// Removes from a MEMORY.md index the lines whose markdown link targets the
/// deleted file — i.e. lines containing `(<name>)`, the link-target part of
/// `- [Title](<name>) — hook`.
fn drop_index_lines(index: &str, name: &str) -> String {
    let target = format!("({name})");
    let kept: Vec<&str> = index.lines().filter(|l| !l.contains(&target)).collect();
    kept.join("\n") + "\n"
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

#[cfg(test)]
mod tests {
    use super::drop_index_lines;

    #[test]
    fn drop_index_lines_removes_only_the_target_link() {
        let index = "# Memory Index\n\n\
            - [Foo](foo.md) — about foo\n\
            - [Bar](bar.md) — about bar\n";
        let out = drop_index_lines(index, "foo.md");
        assert!(!out.contains("foo.md"));
        assert!(out.contains("(bar.md)"));
        assert!(out.contains("# Memory Index"));
    }

    #[test]
    fn drop_index_lines_does_not_match_suffixed_names() {
        // `(foo.md)` must not match `(bar-foo.md)` — the opening paren anchors
        // the link target's start.
        let index = "- [Bar foo](bar-foo.md) — composite name\n";
        let out = drop_index_lines(index, "foo.md");
        assert!(out.contains("(bar-foo.md)"));
    }

    #[test]
    fn drop_index_lines_no_match_keeps_text_intact() {
        let index = "- [Foo](foo.md) — hook\n";
        assert_eq!(drop_index_lines(index, "missing.md"), index);
    }

    #[test]
    fn drop_index_lines_mention_without_link_is_kept() {
        // A plain-text mention of the name is not a link target → kept.
        let index = "- [Other](other.md) — see also foo.md\n";
        let out = drop_index_lines(index, "foo.md");
        assert!(out.contains("(other.md)"));
    }
}
