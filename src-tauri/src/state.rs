//! Shared application state and lightweight helpers used across all layers.

use std::sync::Mutex;

use crate::{config::Config, models, notify::NotifyState, outcome, rtk, scan::ScanCache};

/// An available update surfaced to the panel banner.
#[derive(Clone, serde::Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    pub url: String,
}

/// All mutable state shared across Tauri commands and background services.
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
        .expect("cache poisoned")
        .refresh(&state.models);
}

pub fn refresh_system(state: &tauri::State<AppState>) {
    state
        .system
        .lock()
        .expect("system poisoned")
        .refresh_processes(sysinfo::ProcessesToUpdate::All, false);
}

pub fn refreshed_points(state: &tauri::State<AppState>) -> Vec<crate::scan::Point> {
    refresh_cache(state);
    state.cache.lock().expect("cache poisoned").all_points()
}
