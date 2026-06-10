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
    /// Predicted Unix timestamp when the session bar hits 100%, at current velocity.
    /// None if uncalibrated, no activity, already over 100%, or ETA is after the window reset.
    pub eta_secs: Option<i64>,
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

/// Single-point calibration: linear through the origin. With the empirical quota
/// weights (`models.rs`), the relationship passes through 0, so one point
/// (`budget = K / (percent/100)`) is enough — no second point / intercept needed.
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

/// Predicts when weighted consumption will reach 100% of budget, given a linear
/// velocity over `[window_start, now]`. Returns None if uncalibrated, already
/// over budget, if there isn't enough elapsed time/consumption for a stable
/// velocity estimate (at least 20 min elapsed and at least 5% consumed), or if
/// the predicted time falls after the window reset (the window will reset
/// before 100% is reached, so there is nothing to warn about).
fn compute_eta(
    weighted: f64,
    cal: &Option<Calibration>,
    window_start: Option<i64>,
    now: i64,
) -> Option<i64> {
    let budget = cal.as_ref().filter(|c| c.budget > 0.0)?.budget;
    let remaining = budget - weighted;
    if remaining <= 0.0 {
        return None;
    }
    let start = window_start?;
    let elapsed = now - start;
    let pct = weighted / budget * 100.0;
    // Require at least 20 minutes elapsed and 5% consumed so the velocity
    // estimate is based on enough data to be meaningful.
    if elapsed < 1200 || pct < 5.0 {
        return None;
    }
    let velocity = weighted / elapsed as f64; // weighted tokens per second
    let eta = now + (remaining / velocity) as i64;
    (eta < start + FIVE_HOURS_SECS).then_some(eta)
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

    let eta_secs = compute_eta(weighted, &cfg.session_calibration, live_anchor, now);

    SessionUsage {
        window_start,
        reset_at,
        weighted_tokens: weighted,
        percent,
        calibrated,
        eta_secs,
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
    let (percent, calibrated) = percent_from(weighted, &cfg.weekly_calibration);

    WeeklyUsage {
        weighted_tokens: weighted,
        percent,
        reset_date: cfg.weekly_reset_date.clone(),
        week_start,
        next_reset_at,
        calibrated,
    }
}

/// Returns 7 daily buckets, one per calendar day starting from `week_start`.
/// Each entry is `(midnight_local_ts, per_model_weighted_sums)`.
pub fn daily_buckets(
    points: &[Point],
    week_start: i64,
    now: i64,
) -> Vec<(i64, std::collections::HashMap<String, f64>)> {
    use chrono::{Duration, Local, TimeZone};
    use std::collections::HashMap;

    let ws_dt = Local
        .timestamp_opt(week_start, 0)
        .single()
        .unwrap_or_else(Local::now);
    let ws_date = ws_dt.date_naive();

    let mut buckets = Vec::with_capacity(7);
    for i in 0i64..7 {
        let day_date = ws_date + Duration::days(i);
        let day_start = Local
            .from_local_datetime(&day_date.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|d| d.timestamp())
            .unwrap_or(week_start + i * 86400);
        let day_end = day_start + 86400;

        let mut by_model: HashMap<String, f64> = HashMap::new();
        for p in points
            .iter()
            .filter(|p| p.ts >= day_start && p.ts < day_end && p.ts <= now)
            .filter(|p| !p.model.is_empty() && !p.model.starts_with('<'))
        {
            *by_model.entry(p.model.clone()).or_insert(0.0) += p.weighted;
        }
        buckets.push((day_start, by_model));
    }
    buckets
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

    // --- compute_eta ---

    #[test]
    fn eta_none_when_uncalibrated() {
        assert_eq!(compute_eta(500.0, &None, Some(0), 3600), None);
    }

    #[test]
    fn eta_none_with_zero_budget() {
        assert_eq!(compute_eta(500.0, &cal(0.0), Some(0), 3600), None);
    }

    #[test]
    fn eta_none_at_or_over_budget() {
        assert_eq!(compute_eta(1000.0, &cal(1000.0), Some(0), 3600), None);
        assert_eq!(compute_eta(1500.0, &cal(1000.0), Some(0), 3600), None);
    }

    #[test]
    fn eta_none_without_live_window() {
        assert_eq!(compute_eta(500.0, &cal(1000.0), None, 3600), None);
    }

    #[test]
    fn eta_none_before_20_min_elapsed() {
        // 50% consumed but only 1199s elapsed → velocity not trusted yet.
        assert_eq!(compute_eta(500.0, &cal(1000.0), Some(0), 1199), None);
    }

    #[test]
    fn eta_none_below_5_pct_consumed() {
        // 4.9% consumed after 1h → velocity not trusted yet.
        assert_eq!(compute_eta(49.0, &cal(1000.0), Some(0), 3600), None);
    }

    #[test]
    fn eta_linear_extrapolation() {
        // 500/1000 consumed in 3600s → velocity 500/3600; the remaining 500
        // take another 3600s → ETA at t=7200, well before the 5h reset.
        assert_eq!(compute_eta(500.0, &cal(1000.0), Some(0), 3600), Some(7200));
    }

    #[test]
    fn eta_none_when_past_window_reset() {
        // 10% in 2000s → 100% would land at t=20_000, after the reset at
        // t=18_000: the window resets first, no ETA.
        assert_eq!(compute_eta(100.0, &cal(1000.0), Some(0), 2000), None);
    }

    #[test]
    fn session_usage_exposes_eta() {
        // Anchor at t=0 (first point), 500/1000 consumed, now=3600 → ETA 7200.
        let now = 3600;
        let points = vec![pt(0, 400.0), pt(1800, 100.0)];
        let cfg = Config {
            session_calibration: cal(1000.0),
            ..Default::default()
        };
        let s = session_usage(&points, &cfg, now);
        assert_eq!(s.eta_secs, Some(7200));
    }

    #[test]
    fn session_usage_no_eta_when_uncalibrated() {
        let points = vec![pt(0, 500.0)];
        let s = session_usage(&points, &Config::default(), 3600);
        assert_eq!(s.eta_secs, None);
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

    // --- percent_from (single-point) ---

    #[test]
    fn percent_from_single_point_through_origin() {
        let c = cal(200.0); // budget 200
        let (pct, done) = percent_from(50.0, &c);
        assert!(done);
        assert!((pct.unwrap() - 25.0).abs() < 1e-6); // 50/200*100
    }

    #[test]
    fn percent_from_no_calibration_returns_none() {
        let (pct, done) = percent_from(50.0, &None);
        assert!(!done);
        assert!(pct.is_none());
    }

    #[test]
    fn percent_from_zero_budget_returns_none() {
        // A calibration with budget 0 is treated as uncalibrated (no divide-by-zero).
        let c = Some(Calibration {
            percent: 50.0,
            budget: 0.0,
            calibrated_at: 0,
        });
        let (pct, done) = percent_from(100.0, &c);
        assert!(!done);
        assert!(pct.is_none());
    }

    #[test]
    fn percent_from_can_exceed_100() {
        // Past the calibrated budget the bar goes over 100% (no clamp) — expected
        // before a reset.
        let c = cal(200.0);
        let (pct, _) = percent_from(300.0, &c); // 300 / 200 * 100
        assert!((pct.unwrap() - 150.0).abs() < 1e-6);
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
        let (ws, _) = week_window_from_reset(reset_date, now).expect("test date should parse");
        let points = vec![
            pt(ws - 1, 99.0),     // before window — excluded
            pt(ws, 100.0),        // at window start — included
            pt(ws + 1000, 200.0), // inside window — included
            pt(now + 1, 50.0),    // future — excluded
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

    // --- daily_buckets ---

    #[test]
    fn daily_buckets_returns_seven_entries() {
        // week_start = 2026-01-05 noon UTC → stable across UTC±12 timezones
        let week_start = 1_736_078_400i64; // 2026-01-05 12:00:00 UTC
        let now = week_start + 6 * 86400 + 3600; // 6 days + 1 h later
        let buckets = daily_buckets(&[], week_start, now);
        assert_eq!(buckets.len(), 7);
    }

    #[test]
    fn daily_buckets_sums_points_in_correct_day() {
        // Anchor on a known date; we'll query local-midnight boundaries ourselves.
        let week_start = 1_736_078_400i64; // 2026-01-05 12:00:00 UTC
        let now = week_start + 4 * 86400;

        // Build one bucket via daily_buckets to learn what day_start[0] actually is.
        let empty_buckets = daily_buckets(&[], week_start, now);
        let day0_start = empty_buckets[0].0;
        let day1_start = empty_buckets[1].0;
        let day2_start = empty_buckets[2].0;

        // Place points at noon of days 0, 1, 2 (guaranteed inside those buckets).
        let p0 = pt(day0_start + 43200, 10.0); // day 0 noon
        let p1a = pt(day1_start + 21600, 20.0); // day 1 morning
        let p1b = pt(day1_start + 64800, 30.0); // day 1 evening
        let p2 = pt(day2_start + 43200, 5.0); // day 2 noon

        let points = vec![p0, p1a, p1b, p2];
        let buckets = daily_buckets(&points, week_start, now);

        let total = |i: usize| -> f64 { buckets[i].1.values().sum() };
        assert!((total(0) - 10.0).abs() < 1e-9, "day 0");
        assert!((total(1) - 50.0).abs() < 1e-9, "day 1 sum");
        assert!((total(2) - 5.0).abs() < 1e-9, "day 2");
        assert_eq!(total(3), 0.0, "day 3 empty");
    }

    #[test]
    fn daily_buckets_filters_empty_and_synthetic_models() {
        let week_start = 1_736_078_400i64;
        let now = week_start + 7 * 86400;
        let empty_buckets = daily_buckets(&[], week_start, now);
        let day0_start = empty_buckets[0].0;

        let points = vec![
            Point {
                ts: day0_start + 43200,
                weighted: 100.0,
                model: "<synthetic>".to_string(),
                key: 1,
            },
            Point {
                ts: day0_start + 43200,
                weighted: 200.0,
                model: String::new(),
                key: 2,
            },
            Point {
                ts: day0_start + 43200,
                weighted: 50.0,
                model: "claude-sonnet-4-6".to_string(),
                key: 3,
            },
        ];
        let buckets = daily_buckets(&points, week_start, now);
        assert_eq!(buckets[0].1.len(), 1, "only real models");
        assert!((buckets[0].1["claude-sonnet-4-6"] - 50.0).abs() < 1e-9);
    }

    #[test]
    fn daily_buckets_excludes_future_points() {
        let week_start = 1_736_078_400i64;
        let empty_buckets = daily_buckets(&[], week_start, week_start + 7 * 86400);
        let day3_start = empty_buckets[3].0;

        // now = noon of day 2 → day 3 point is in the future
        let now = day3_start - 3600;
        let future_pt = pt(day3_start + 43200, 99.0);
        let buckets = daily_buckets(&[future_pt], week_start, now);
        assert_eq!(
            buckets[3].1.values().sum::<f64>(),
            0.0,
            "future point must not appear in day 3"
        );
    }

    #[test]
    fn daily_buckets_point_on_day_boundary_goes_to_correct_day() {
        let week_start = 1_736_078_400i64;
        let empty_buckets = daily_buckets(&[], week_start, week_start + 7 * 86400);
        let day1_start = empty_buckets[1].0;

        // Point exactly at day1_start belongs to day 1, not day 0.
        let now = week_start + 7 * 86400;
        let boundary_pt = pt(day1_start, 42.0);
        let buckets = daily_buckets(&[boundary_pt], week_start, now);
        assert_eq!(
            buckets[0].1.values().sum::<f64>(),
            0.0,
            "day 0 must be empty"
        );
        assert!(
            (buckets[1].1.values().sum::<f64>() - 42.0).abs() < 1e-9,
            "day 1 must contain boundary point"
        );
    }

    #[test]
    fn weekly_usage_with_calibration_computes_percent() {
        let now = 1_000_000i64;
        let reset_date = "1970-01-15";
        let (ws, _) = week_window_from_reset(reset_date, now).expect("test date should parse");
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

    // --- session multiple boundaries ---

    #[test]
    fn session_multiple_5h_windows_only_counts_latest() {
        let now = 1_000_000;
        let w1_anchor = now - 2 * FIVE_HOURS_SECS;
        let w2_anchor = now - FIVE_HOURS_SECS + 3600;
        let points = vec![
            pt(w1_anchor, 100.0),
            pt(w1_anchor + 1000, 50.0), // w1: 150 total (but window expired)
            pt(w2_anchor, 200.0),       // w2: only this window is live
            pt(w2_anchor + 1000, 100.0),
        ];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        // Only w2 window (most recent) is live: 200 + 100 = 300
        assert_eq!(s.weighted_tokens, 300.0);
    }

    #[test]
    fn session_reset_at_is_future() {
        let now = 1_000_000;
        let anchor = now - 1000;
        let points = vec![pt(anchor, 50.0)];
        let cfg = Config::default();
        let s = session_usage(&points, &cfg, now);
        assert!(s.reset_at.is_some());
        let reset = s.reset_at.unwrap();
        assert!(reset > now);
        assert_eq!(reset, anchor + FIVE_HOURS_SECS);
    }

    // --- weekly boundary edge cases ---

    #[test]
    fn week_window_reset_date_exactly_now() {
        // reset_date is exactly at now: step backward by 7d, then forward to get next.
        let now = 1_000_000i64;
        let reset_dt_str = "1970-01-08"; // This will be parsed relative to local tz
        let result = week_window_from_reset(reset_dt_str, now);
        assert!(result.is_some());
        let (ws, next) = result.unwrap();
        assert!(ws <= now);
        assert!(next > now);
        assert_eq!(next - ws, WEEK_SECS);
    }

    #[test]
    fn week_window_with_datetime_including_seconds() {
        // Test parsing of "%Y-%m-%dT%H:%M:%S" format
        let now = 1_000_000i64;
        let result = week_window_from_reset("1970-01-08T12:00:00", now);
        assert!(result.is_some());
        let (ws, next) = result.unwrap();
        assert!(ws <= now);
        assert_eq!(next - ws, WEEK_SECS);
    }

    // --- weighted_since boundary tests ---

    #[test]
    fn weighted_since_includes_boundaries() {
        let from = 1_000i64;
        let now = 2_000i64;
        let points = vec![
            pt(from, 10.0), // boundary inclusive
            pt(from + 500, 20.0),
            pt(now, 30.0), // boundary inclusive
            pt(now + 1, 99.0),
        ];
        let w = weighted_since(&points, from, now);
        assert_eq!(w, 60.0); // 10 + 20 + 30
    }

    #[test]
    fn weighted_since_empty_window_returns_zero() {
        let points = vec![pt(100, 10.0), pt(200, 20.0)];
        let w = weighted_since(&points, 300, 400);
        assert_eq!(w, 0.0);
    }

    // --- daily_buckets with mixed models ---

    #[test]
    fn daily_buckets_per_model_breakdown() {
        let week_start = 1_736_078_400i64;
        let now = week_start + 86400;
        let empty_buckets = daily_buckets(&[], week_start, now);
        let day0_start = empty_buckets[0].0;

        let points = vec![
            Point {
                ts: day0_start + 43200,
                weighted: 100.0,
                model: "claude-opus-4-8".to_string(),
                key: 1,
            },
            Point {
                ts: day0_start + 43200,
                weighted: 50.0,
                model: "claude-sonnet-4-6".to_string(),
                key: 2,
            },
            Point {
                ts: day0_start + 50000,
                weighted: 30.0,
                model: "claude-opus-4-8".to_string(),
                key: 3,
            },
        ];

        let buckets = daily_buckets(&points, week_start, now);
        let day0_models = &buckets[0].1;

        assert_eq!(day0_models.len(), 2, "should have 2 models");
        assert!(
            (day0_models.get("claude-opus-4-8").unwrap_or(&0.0) - 130.0).abs() < 1e-9,
            "opus total"
        );
        assert!(
            (day0_models.get("claude-sonnet-4-6").unwrap_or(&0.0) - 50.0).abs() < 1e-9,
            "sonnet total"
        );
    }
}
