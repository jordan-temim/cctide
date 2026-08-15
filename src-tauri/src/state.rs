//! Shared application state and lightweight helpers used across all layers.

use std::sync::Mutex;

use crate::{config::Config, models, notify::NotifyState, outcome, rtk, scan::ScanCache};

/// The tray icon's id, shared between where it's created (`lib.rs`) and where
/// it's redrawn (`tick.rs`) so a rename can't silently break icon updates.
pub const TRAY_ICON_ID: &str = "cctide-tray";

/// An available update surfaced to the panel banner.
#[derive(Clone, serde::Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    pub url: String,
}

/// All mutable state shared across Tauri commands and background services.
///
/// Lock ordering: when a call site needs more than one of these locks at once
/// (e.g. `get_panel_data`), acquire `cache` before `system` — no code path
/// currently needs the reverse order, and keeping this order everywhere rules
/// out a deadlock between them.
pub struct AppState {
    pub cache: Mutex<ScanCache>,
    pub notify_state: Mutex<NotifyState>,
    /// Config cached in memory; updated by set_* commands and read by the icon thread.
    pub config_cache: Mutex<Config>,
    /// Model table loaded once at startup (embedded JSON, immutable).
    pub models: models::Models,
    pub system: Mutex<sysinfo::System>,
    /// Latest update found by the background check, if any.
    pub available_update: Mutex<Option<UpdateInfo>>,
    /// RTK savings cached by the refresh loop.
    pub rtk_cache: Mutex<Option<rtk::RtkSavings>>,
    /// Outcome report cached on demand: (computed_at, report). Git work is
    /// only done when the panel asks and the cache is stale.
    pub outcome_cache: Mutex<Option<(i64, outcome::OutcomeReport)>>,
}

pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn refresh_cache(state: &tauri::State<AppState>) {
    state
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .refresh(&state.models);
}

pub fn refresh_system(state: &tauri::State<AppState>) {
    state
        .system
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .refresh_processes(sysinfo::ProcessesToUpdate::All, false);
}

pub fn refreshed_points(state: &tauri::State<AppState>) -> std::sync::Arc<[crate::scan::Point]> {
    refresh_cache(state);
    state
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .all_points()
}
