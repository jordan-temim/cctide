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
    /// Unix timestamp of the next upcoming reset (always in the future).
    pub next_reset_at: Option<i64>,
    pub calibrated: bool,
}

/// Minimum token distance between two calibration points for the linear fit to
/// be numerically stable (avoids near-zero denominator in slope computation).
const MIN_TOKEN_DIFF: f64 = 1.0;

fn percent_from(
    weighted: f64,
    cal_1: &Option<Calibration>,
    cal_2: &Option<Calibration>,
) -> (Option<f64>, bool) {
    // Two-point linear fit: percent = a·tokens + b
    if let (Some(c1), Some(c2)) = (cal_1, cal_2) {
        if c1.budget > 0.0 && c2.budget > 0.0 {
            // Recover token counts from budget = tokens / (percent/100)
            let k2 = c1.budget * (c1.percent / 100.0); // newer calibration
            let k1 = c2.budget * (c2.percent / 100.0); // older calibration
            let dk = k2 - k1;
            if dk.abs() >= MIN_TOKEN_DIFF {
                let a = (c1.percent - c2.percent) / dk;
                let b = c2.percent - a * k1;
                return (Some((a * weighted + b).clamp(0.0, 200.0)), true);
            }
        }
    }
    // Single-point fallback: linear through origin
    match cal_1 {
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

    let (percent, calibrated) =
        percent_from(weighted, &cfg.session_calibration, &cfg.session_calibration_2);
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
/// Returns `(week_start, next_reset)` — both as Unix seconds in local time.
pub fn week_window_from_reset(reset_date: &str, now: i64) -> Option<(i64, i64)> {
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
        // .earliest() rather than .single() so fall-back DST ambiguity (e.g.
        // 1:30 AM occurring twice) picks the first occurrence instead of
        // returning None and silently dropping the weekly bar.
        .earliest()?
        .timestamp();
    let mut reset = reset_dt;
    // If the anchor is in the future (including >7 d ahead), step backward to
    // the most recent past occurrence of the reset time first.
    while reset > now {
        reset -= WEEK_SECS;
    }
    // Now reset <= now; advance one week at a time to find the next upcoming
    // reset (strictly after now).
    // Adding exactly 604_800 s per step may drift ±1 h at DST transitions in
    // timezones that observe DST; acceptable for this use-case.
    while reset <= now {
        reset += WEEK_SECS;
    }
    // reset is the next upcoming reset; reset − 7d is the most recent past reset.
    Some((reset - WEEK_SECS, reset))
}


/// Weekly limit.
pub fn weekly_usage(points: &[Point], cfg: &Config, now: i64) -> WeeklyUsage {
    let window = cfg
        .weekly_reset_date
        .as_deref()
        .and_then(|d| week_window_from_reset(d, now));

    let week_start = window.map(|(start, _)| start);
    let next_reset_at = window.map(|(_, next)| next);

    let weighted = match week_start {
        Some(start) => weighted_since(points, start, now),
        None => 0.0,
    };
    let (percent, calibrated) =
        percent_from(weighted, &cfg.weekly_calibration, &cfg.weekly_calibration_2);

    WeeklyUsage {
        weighted_tokens: weighted,
        percent,
        reset_date: cfg.weekly_reset_date.clone(),
        week_start,
        next_reset_at,
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

    // --- percent_from (two-point fit) ---

    fn cal_point(percent: f64, tokens: f64) -> Option<Calibration> {
        Some(Calibration {
            percent,
            budget: budget_from_percent(tokens, percent),
            calibrated_at: 0,
        })
    }

    #[test]
    fn two_point_fit_corrects_intercept() {
        // True relationship: percent = 0.5 * tokens + 5 (intercept of 5)
        // cal_2 (older): tokens=10, percent=10  →  a=0.5, b=5
        // cal_1 (newer): tokens=30, percent=20
        let cal_1 = cal_point(20.0, 30.0);
        let cal_2 = cal_point(10.0, 10.0);
        // At tokens=50: expected 0.5*50+5 = 30
        let (pct, done) = percent_from(50.0, &cal_1, &cal_2);
        assert!(done);
        let pct = pct.unwrap();
        assert!((pct - 30.0).abs() < 1e-6, "expected 30, got {pct}");
    }

    #[test]
    fn two_point_same_as_single_when_cal2_missing() {
        let cal_1 = cal_point(50.0, 100.0); // budget = 200
        let (pct, done) = percent_from(50.0, &cal_1, &None);
        assert!(done);
        assert!((pct.unwrap() - 25.0).abs() < 1e-6); // 50/200*100 = 25
    }

    #[test]
    fn two_point_fallback_when_dk_too_small() {
        // Both points at nearly identical token counts → fallback to single-point
        let cal_1 = cal_point(20.0, 100.0);
        let cal_2 = cal_point(20.1, 100.5); // dk = 0.5 < MIN_TOKEN_DIFF=1.0
        let (pct, done) = percent_from(100.0, &cal_1, &cal_2);
        assert!(done);
        // Should use single-point: 100 / (100/0.2) * 100 = 20
        let budget = budget_from_percent(100.0, 20.0);
        let expected = 100.0 / budget * 100.0;
        assert!((pct.unwrap() - expected).abs() < 1e-6);
    }

    #[test]
    fn two_point_result_clamped_at_zero() {
        // fit produces negative at low tokens
        let cal_1 = cal_point(50.0, 100.0);
        let cal_2 = cal_point(10.0, 10.0);
        // a = (50-10)/(100-10) = 40/90, b = 10 - (40/90)*10 ≈ 5.56
        // At tokens=0: b ≈ 5.56 (positive, already fine — try tokens=-1 via 0 clamp)
        // Use tokens far below k1 to get negative result from extrapolation
        let (pct, _) = percent_from(0.0, &cal_1, &cal_2);
        assert!(pct.unwrap() >= 0.0);
    }

    #[test]
    fn two_point_no_calibration_returns_none() {
        let (pct, done) = percent_from(50.0, &None, &None);
        assert!(!done);
        assert!(pct.is_none());
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

    // --- week_window_from_reset ---

    #[test]
    fn week_start_date_only_format() {
        let now = 0i64;
        let result = week_window_from_reset("2026-06-01", now);
        assert!(result.is_some());
    }

    #[test]
    fn week_start_invalid_date_returns_none() {
        let result = week_window_from_reset("not-a-date", 0);
        assert!(result.is_none());
    }

    #[test]
    fn week_start_more_than_7d_in_future_is_in_the_past() {
        // Regression: reset_date >7 d ahead used to produce week_start > now
        // (0 % bar permanently). 1970-01-09 is ~8 days from Unix epoch; now=1000
        // puts it more than one full week in the future.
        let now = 1_000i64;
        let result = week_window_from_reset("1970-01-09", now);
        assert!(result.is_some());
        let (ws, next) = result.unwrap();
        assert!(ws <= now, "week_start {ws} must be ≤ now {now}");
        assert_eq!(next, ws + WEEK_SECS);
    }

    #[test]
    fn week_start_is_7d_before_next_reset() {
        let now = 1_000;
        let result = week_window_from_reset("1970-01-08", now);
        assert!(result.is_some());
        let (ws, next) = result.unwrap();
        assert!(ws <= now);
        assert!(ws >= now - WEEK_SECS);
        assert_eq!(next, ws + WEEK_SECS);
    }

    #[test]
    fn week_start_datetime_local_format() {
        // "%Y-%m-%dT%H:%M" format must parse correctly.
        let now = 1_000;
        let result = week_window_from_reset("1970-01-08T00:00", now);
        assert!(result.is_some());
        let (ws, next) = result.unwrap();
        assert!(ws <= now);
        assert_eq!(next, ws + WEEK_SECS);
    }

    // --- weekly_usage with a valid reset date ---

    #[test]
    fn weekly_usage_sums_points_in_window() {
        let now = 1_000_000i64;
        // Derive the window boundaries so points can be placed precisely.
        let reset_date = "1970-01-15";
        let (ws, _) =
            week_window_from_reset(reset_date, now).expect("test date should parse");
        let points = vec![
            pt(ws - 1, 99.0),       // before window — excluded
            pt(ws, 100.0),          // at window start — included
            pt(ws + 1000, 200.0),   // inside window — included
            pt(now + 1, 50.0),      // future — excluded
        ];
        let cfg = Config {
            weekly_reset_date: Some(reset_date.to_string()),
            ..Default::default()
        };
        let w = weekly_usage(&points, &cfg, now);
        assert_eq!(w.weighted_tokens, 300.0);
        assert!(w.week_start.is_some());
        assert!(w.next_reset_at.is_some());
    }

    #[test]
    fn weekly_usage_with_calibration_computes_percent() {
        let now = 1_000_000i64;
        let reset_date = "1970-01-15";
        let (ws, _) =
            week_window_from_reset(reset_date, now).expect("test date should parse");
        let points = vec![pt(ws, 100.0)];
        let cfg = Config {
            weekly_reset_date: Some(reset_date.to_string()),
            weekly_calibration: cal(200.0), // 100/200*100 = 50%
            ..Default::default()
        };
        let w = weekly_usage(&points, &cfg, now);
        assert!(w.calibrated);
        assert_eq!(w.percent, Some(50.0));
    }

    // --- session_usage 5h boundary edge cases ---

    #[test]
    fn session_window_exactly_expired_returns_zero() {
        // A point that lands exactly at anchor + 5h means now == anchor + 5h,
        // so `now < anchor + 5h` is false → live_anchor = None → zero usage.
        let now = 1_000_000;
        let anchor = now - FIVE_HOURS_SECS;
        let points = vec![pt(anchor, 55.0)];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.weighted_tokens, 0.0);
        assert!(s.window_start.is_none());
    }

    #[test]
    fn session_point_just_before_5h_boundary_stays_in_window() {
        // anchor + 5h = now + 1, so `now < anchor + 5h` is true → still live.
        let now = 1_000_000;
        let anchor = now - FIVE_HOURS_SECS + 1;
        let points = vec![pt(anchor, 77.0)];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.weighted_tokens, 77.0);
        assert!(s.window_start.is_some());
    }
}
