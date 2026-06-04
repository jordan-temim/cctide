//! Persisted application configuration.
//!
//! Stored in `<os-config-dir>/com.cctide/cctide.json`. Holds the calibration
//! anchors (5h session and weekly), the weekly reset date, and settings for
//! weighting / context limits / refresh interval.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Calibration anchor: at `calibrated_at`, the user declared being at
/// `percent`%; from that we derive a `budget` (in weighted tokens) used
/// afterwards to convert local consumption into a percentage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Calibration {
    pub percent: f64,
    pub budget: f64,
    pub calibrated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Calibration for the 5h session window (most recent point).
    #[serde(default)]
    pub session_calibration: Option<Calibration>,
    /// Previous session calibration point; together with `session_calibration`
    /// enables a two-point linear fit (`percent = a·tokens + b`).
    #[serde(default)]
    pub session_calibration_2: Option<Calibration>,
    /// Calibration for the weekly limit (most recent point).
    #[serde(default)]
    pub weekly_calibration: Option<Calibration>,
    /// Previous weekly calibration point for the two-point linear fit.
    #[serde(default)]
    pub weekly_calibration_2: Option<Calibration>,
    /// Weekly reset date in ISO format `YYYY-MM-DD` (entered by the user).
    #[serde(default)]
    pub weekly_reset_date: Option<String>,
    /// Per-model context-limit overrides (substring -> tokens).
    #[serde(default)]
    pub context_limits: HashMap<String, u64>,
    /// Auto-refresh interval for the panel, in seconds.
    #[serde(default)]
    pub refresh_secs: u64,
    /// Whether OS (Mac/Windows) system notifications fire at the alert levels.
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    /// The three global alert levels (%), driving icon tiers, bar colours and
    /// notifications. Sorted ascending, 0–100.
    #[serde(default = "default_levels")]
    pub alert_levels: Vec<f64>,
    /// Whether the tray icon reflects live usage (false = static icon).
    #[serde(default = "default_true")]
    pub dynamic_icon: bool,
    /// Whether usage tracking is active. When false the icon shows a disabled
    /// state (tracks only + diagonal slash) and no data is recomputed.
    #[serde(default = "default_true")]
    pub tracking_enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_levels() -> Vec<f64> {
    vec![33.0, 66.0, 90.0]
}

/// Alert level (0..=3): how many of the sorted `alert_levels` the percentage reached.
/// Shared by the tray icon tiers, bar colours, and system notifications.
pub fn level_for(percent: Option<f64>, levels: &[f64]) -> u8 {
    match percent {
        Some(p) => levels.iter().filter(|&&l| p >= l).count() as u8,
        None => 0,
    }
}

/// Normalises alert levels: keep finite values clamped to 0–100, sorted; pad to
/// the defaults if not exactly three.
pub fn sanitize_levels(levels: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = levels
        .iter()
        .filter(|x| x.is_finite())
        .map(|x| x.clamp(0.0, 100.0))
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.len() != 3 {
        return default_levels();
    }
    v
}

impl Default for Config {
    fn default() -> Self {
        Config {
            session_calibration: None,
            session_calibration_2: None,
            weekly_calibration: None,
            weekly_calibration_2: None,
            weekly_reset_date: None,
            context_limits: HashMap::new(),
            refresh_secs: 30,
            notifications_enabled: true,
            alert_levels: default_levels(),
            dynamic_icon: true,
            tracking_enabled: true,
        }
    }
}

/// The app's own data directory, derived from the bundle identifier — mirrors
/// Tauri's `app_config_dir` without needing an `AppHandle`:
/// macOS `~/Library/Application Support/com.cctide`, Windows
/// `%APPDATA%\com.cctide`, Linux `~/.config/com.cctide`.
const APP_DIR: &str = "com.cctide";

/// Current config path: `<os-config-dir>/com.cctide/cctide.json`.
pub fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(APP_DIR).join("cctide.json"))
}

fn parse_config(text: &str) -> Config {
    match serde_json::from_str::<Config>(text) {
        Ok(mut cfg) => {
            if cfg.refresh_secs == 0 {
                cfg.refresh_secs = 30;
            }
            cfg
        }
        Err(_) => Config::default(),
    }
}

pub fn load() -> Config {
    if let Some(text) = config_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        return parse_config(&text);
    }
    Config::default()
}

/// Writes the config to disk atomically (write temp → rename) so concurrent
/// readers never see a partially-written file.
pub fn save(cfg: &Config) -> Result<(), String> {
    let path = config_path().ok_or("home dir not found")?;
    let parent = path.parent().ok_or("config path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- level_for ---

    #[test]
    fn level_for_none_returns_zero() {
        assert_eq!(level_for(None, &[33.0, 66.0, 90.0]), 0);
    }

    #[test]
    fn level_for_below_all_thresholds() {
        assert_eq!(level_for(Some(10.0), &[33.0, 66.0, 90.0]), 0);
    }

    #[test]
    fn level_for_crosses_one_threshold() {
        assert_eq!(level_for(Some(50.0), &[33.0, 66.0, 90.0]), 1);
    }

    #[test]
    fn level_for_at_threshold_counts_it() {
        assert_eq!(level_for(Some(66.0), &[33.0, 66.0, 90.0]), 2);
    }

    #[test]
    fn level_for_all_thresholds() {
        assert_eq!(level_for(Some(100.0), &[33.0, 66.0, 90.0]), 3);
    }

    // --- sanitize_levels ---

    #[test]
    fn sanitize_sorts_ascending() {
        assert_eq!(sanitize_levels(&[90.0, 33.0, 66.0]), vec![33.0, 66.0, 90.0]);
    }

    #[test]
    fn sanitize_clamps_to_0_100() {
        let v = sanitize_levels(&[0.0, 50.0, 110.0]);
        assert_eq!(v[2], 100.0);
        let v2 = sanitize_levels(&[-5.0, 50.0, 80.0]);
        assert_eq!(v2[0], 0.0);
    }

    #[test]
    fn sanitize_wrong_count_returns_default() {
        assert_eq!(sanitize_levels(&[50.0]), vec![33.0, 66.0, 90.0]);
        assert_eq!(sanitize_levels(&[]), vec![33.0, 66.0, 90.0]);
        assert_eq!(
            sanitize_levels(&[10.0, 20.0, 30.0, 40.0]),
            vec![33.0, 66.0, 90.0]
        );
    }

    #[test]
    fn sanitize_filters_nan_and_inf() {
        // NaN and inf are filtered → less than 3 values → default
        let v = sanitize_levels(&[f64::NAN, 50.0, 80.0]);
        assert_eq!(v, vec![33.0, 66.0, 90.0]);
        let v2 = sanitize_levels(&[f64::INFINITY, 50.0, 80.0]);
        assert_eq!(v2, vec![33.0, 66.0, 90.0]);
    }

    #[test]
    fn sanitize_valid_three_passthrough() {
        assert_eq!(sanitize_levels(&[10.0, 50.0, 80.0]), vec![10.0, 50.0, 80.0]);
    }

    // --- parse_config / defaults ---

    #[test]
    fn default_refresh_secs_is_30() {
        assert_eq!(Config::default().refresh_secs, 30);
    }

    #[test]
    fn default_notifications_enabled() {
        assert!(Config::default().notifications_enabled);
    }

    #[test]
    fn default_tracking_enabled() {
        assert!(Config::default().tracking_enabled);
    }

    #[test]
    fn default_alert_levels() {
        assert_eq!(Config::default().alert_levels, vec![33.0, 66.0, 90.0]);
    }

    #[test]
    fn parse_config_unknown_fields_ignored() {
        let json = r#"{"refresh_secs": 60, "unknown_field": true}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.refresh_secs, 60);
    }

    #[test]
    fn parse_config_zero_refresh_falls_back_to_30() {
        let json = r#"{"refresh_secs": 0}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        // parse_config normalises 0 → 30
        let text = serde_json::to_string(&cfg).unwrap();
        let parsed = super::parse_config(&text);
        assert_eq!(parsed.refresh_secs, 30);
    }

    #[test]
    fn parse_config_invalid_json_returns_default() {
        let cfg = super::parse_config("not valid json {{{");
        assert_eq!(cfg.refresh_secs, 30);
        assert_eq!(cfg.alert_levels, vec![33.0, 66.0, 90.0]);
        assert!(cfg.notifications_enabled);
    }

    #[test]
    fn parse_config_calibration_round_trips() {
        let original = Config {
            session_calibration: Some(Calibration {
                percent: 42.0,
                budget: 1234.5,
                calibrated_at: 1_000_000,
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed = super::parse_config(&json);
        let cal = parsed.session_calibration.unwrap();
        assert!((cal.percent - 42.0).abs() < 1e-9);
        assert!((cal.budget - 1234.5).abs() < 1e-9);
        assert_eq!(cal.calibrated_at, 1_000_000);
    }
}
