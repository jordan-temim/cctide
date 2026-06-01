//! Computes the "5h session" and "weekly limit" bars.
//!
//! Fully offline: we sum the local weighted tokens within the relevant window,
//! then convert to a percentage using the budget derived during manual
//! calibration (the user reports the % shown by `/usage`).

use serde::Serialize;

use crate::config::{Calibration, Config};
use crate::scan::Point;

const FIVE_HOURS_SECS: i64 = 5 * 3600;
const WEEK_SECS: i64 = 7 * 24 * 3600;

#[derive(Debug, Serialize)]
pub struct SessionUsage {
    /// Session anchor: the first prompt of the current 5h block, Unix seconds.
    pub window_start: Option<i64>,
    /// When the current 5h window resets (anchor + 5h), Unix seconds.
    pub reset_at: Option<i64>,
    pub weighted_tokens: f64,
    /// Estimated percentage (None until calibrated).
    pub percent: Option<f64>,
    pub calibrated: bool,
}

#[derive(Debug, Serialize)]
pub struct WeeklyUsage {
    pub weighted_tokens: f64,
    pub percent: Option<f64>,
    pub reset_date: Option<String>,
    pub week_start: Option<i64>,
    pub calibrated: bool,
}

fn percent_from(weighted: f64, cal: &Option<Calibration>) -> (Option<f64>, bool) {
    match cal {
        Some(c) if c.budget > 0.0 => (Some(weighted / c.budget * 100.0), true),
        _ => (None, false),
    }
}

/// Sum of weighted tokens over `[from, now]`.
pub fn weighted_since(points: &[Point], from: i64, now: i64) -> f64 {
    points
        .iter()
        .filter(|p| p.ts >= from && p.ts <= now)
        .map(|p| p.weighted)
        .sum()
}

/// Session window, anchored like Anthropic's model: the window starts at the
/// first prompt and lasts exactly 5h; a prompt at/after `anchor + 5h` starts a
/// fresh window. We detect the current anchor from the timestamps, then sum
/// what was consumed since it. If the window has already elapsed (no prompt in
/// the current window yet), usage is 0 and the window is considered reset.
pub fn session_usage(points: &[Point], cfg: &Config, now: i64) -> SessionUsage {
    let mut pts: Vec<&Point> = points.iter().filter(|p| p.ts <= now).collect();
    pts.sort_by_key(|p| p.ts);

    // Find the anchor of the most recent 5h window.
    let mut anchor: Option<i64> = None;
    for p in &pts {
        match anchor {
            None => anchor = Some(p.ts),
            Some(a) if p.ts >= a + FIVE_HOURS_SECS => anchor = Some(p.ts),
            _ => {}
        }
    }

    // The window is only "live" if now is still within [anchor, anchor + 5h].
    let live_anchor = anchor.filter(|a| now < a + FIVE_HOURS_SECS);
    let window_start = live_anchor;
    let reset_at = live_anchor.map(|a| a + FIVE_HOURS_SECS);
    let weighted: f64 = match live_anchor {
        Some(a) => pts.iter().filter(|p| p.ts >= a).map(|p| p.weighted).sum(),
        None => 0.0,
    };

    let (percent, calibrated) = percent_from(weighted, &cfg.session_calibration);
    SessionUsage {
        window_start,
        reset_at,
        weighted_tokens: weighted,
        percent,
        calibrated,
    }
}

/// Computes the week start from the reset date/time (reset − 7d). If the reset
/// is in the past, step forward by 7d until it covers `now` (rolling weekly
/// window). Accepts a `datetime-local` value (`YYYY-MM-DDTHH:MM`) or a plain
/// date (`YYYY-MM-DD`), interpreted in the machine's local timezone.
pub fn week_start_from_reset(reset_date: &str, now: i64) -> Option<i64> {
    use chrono::TimeZone;

    let naive = chrono::NaiveDateTime::parse_from_str(reset_date, "%Y-%m-%dT%H:%M")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(reset_date, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(reset_date, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
        })
        .ok()?;

    let reset_dt = chrono::Local
        .from_local_datetime(&naive)
        .single()?
        .timestamp();
    let mut reset = reset_dt;
    // Advance the reset while it is in the past relative to now.
    while reset <= now {
        reset += WEEK_SECS;
    }
    Some(reset - WEEK_SECS)
}

/// Weekly limit.
pub fn weekly_usage(points: &[Point], cfg: &Config, now: i64) -> WeeklyUsage {
    let week_start = cfg
        .weekly_reset_date
        .as_deref()
        .and_then(|d| week_start_from_reset(d, now));

    let weighted = match week_start {
        Some(start) => weighted_since(points, start, now),
        None => 0.0,
    };
    let (percent, calibrated) = percent_from(weighted, &cfg.weekly_calibration);

    WeeklyUsage {
        weighted_tokens: weighted,
        percent,
        reset_date: cfg.weekly_reset_date.clone(),
        week_start,
        calibrated,
    }
}

/// Derives the budget (weighted tokens) from a declared % and the window's
/// current consumption: `budget = K_now / (percent/100)`.
pub fn budget_from_percent(weighted_now: f64, percent: f64) -> f64 {
    if percent <= 0.0 {
        0.0
    } else {
        weighted_now / (percent / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Calibration, Config};
    use crate::scan::Point;

    fn pt(ts: i64, weighted: f64) -> Point {
        Point {
            ts,
            weighted,
            tokens: 1000,
            model: "claude-sonnet-4-6".to_string(),
            key: ts as u64,
        }
    }

    fn cal(budget: f64) -> Option<Calibration> {
        Some(Calibration {
            percent: 50.0,
            budget,
            calibrated_at: 0,
        })
    }

    // --- budget_from_percent ---

    #[test]
    fn budget_roundtrip() {
        let b = budget_from_percent(75.0, 37.5);
        assert!((b - 200.0).abs() < 1e-9);
    }

    #[test]
    fn budget_zero_percent_returns_zero() {
        assert_eq!(budget_from_percent(100.0, 0.0), 0.0);
    }

    #[test]
    fn budget_at_100_pct() {
        assert!((budget_from_percent(500.0, 100.0) - 500.0).abs() < 1e-9);
    }

    // --- session_usage ---

    #[test]
    fn session_single_point_in_window() {
        let now = 1_000_000;
        let points = vec![pt(now - 100, 50.0)];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.weighted_tokens, 50.0);
        assert!(s.window_start.is_some());
        assert_eq!(s.window_start.unwrap(), now - 100);
    }

    #[test]
    fn session_expired_window_returns_zero() {
        let now = 1_000_000;
        // Only point is 6h old → window expired
        let points = vec![pt(now - 6 * 3600, 50.0)];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.weighted_tokens, 0.0);
        assert!(s.window_start.is_none());
    }

    #[test]
    fn session_resets_after_5h_gap() {
        let now = 1_000_000;
        let points = vec![
            pt(now - 6 * 3600, 999.0), // old session — excluded
            pt(now - 3600, 30.0),      // new session — counted
        ];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.weighted_tokens, 30.0);
    }

    #[test]
    fn session_sums_multiple_points_in_window() {
        let now = 1_000_000;
        let anchor = now - 2 * 3600;
        let points = vec![
            pt(anchor, 10.0),
            pt(anchor + 100, 20.0),
            pt(anchor + 200, 30.0),
        ];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.weighted_tokens, 60.0);
    }

    #[test]
    fn session_percent_with_calibration() {
        let now = 1_000_000;
        let points = vec![pt(now - 100, 50.0)];
        let cfg = Config {
            session_calibration: cal(200.0), // 50/200 = 25%
            ..Default::default()
        };
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.percent, Some(25.0));
        assert!(s.calibrated);
    }

    #[test]
    fn session_no_calibration_returns_none_pct() {
        let now = 1_000_000;
        let points = vec![pt(now - 100, 50.0)];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.percent, None);
        assert!(!s.calibrated);
    }

    // --- weighted_since ---

    #[test]
    fn weighted_since_excludes_outside_window() {
        let from = 1_000;
        let now = 2_000;
        let points = vec![
            pt(999, 100.0),
            pt(1000, 10.0),
            pt(1500, 20.0),
            pt(2001, 999.0),
        ];
        let w = weighted_since(&points, from, now);
        assert_eq!(w, 30.0); // only ts=1000 and ts=1500
    }

    // --- weekly_usage ---

    #[test]
    fn weekly_usage_no_reset_date_returns_zero() {
        let now = 1_000_000;
        let points = vec![pt(now - 100, 50.0)];
        let cfg = Config::default();
        let w = weekly_usage(&points, &cfg, now);
        assert_eq!(w.weighted_tokens, 0.0);
        assert!(!w.calibrated);
    }

    // --- week_start_from_reset ---

    #[test]
    fn week_start_date_only_format() {
        // Any valid future date in ISO format should parse
        let now = 0i64; // epoch
        let result = week_start_from_reset("2026-06-01", now);
        assert!(result.is_some());
    }

    #[test]
    fn week_start_invalid_date_returns_none() {
        let result = week_start_from_reset("not-a-date", 0);
        assert!(result.is_none());
    }

    #[test]
    fn week_start_is_7d_before_next_reset() {
        // reset 7 days from epoch → week_start should be at epoch (≈0)
        let now = 1_000; // just after epoch
        let result = week_start_from_reset("1970-01-08", now);
        // week_start = Jan 1 00:00 local time (≈0 UTC, may vary by TZ)
        assert!(result.is_some());
        let ws = result.unwrap();
        assert!(ws <= now);
        assert!(ws >= now - WEEK_SECS);
    }
}
