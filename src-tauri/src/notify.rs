//! System (macOS/Windows) notifications at the global alert levels.
//!
//! Fires once each time the session or weekly bar reaches a *new, higher* alert
//! level (e.g. 33 → 66 → 90%). Re-arms when the bar drops back below a level.
//! The level state is tracked even when notifications are disabled, so toggling
//! them on later doesn't replay levels already passed.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::config::{level_for, Config};
use crate::usage::{SessionUsage, WeeklyUsage};

/// Highest alert level already signalled per bar.
#[derive(Default)]
pub struct NotifyState {
    session_level: u8,
    weekly_level: u8,
    /// Whether the "please calibrate a second time" notification was already sent.
    recal_session_sent: bool,
    recal_weekly_sent: bool,
    /// Window anchors seen on the previous check; used to detect window rollovers
    /// so the recal nudge can re-arm for the new window.
    last_session_window: Option<i64>,
    last_weekly_week: Option<i64>,
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

impl NotifyState {
    /// Evaluates the alert levels and fires notifications on new crossings.
    pub fn check(
        &mut self,
        app: &AppHandle,
        cfg: &Config,
        session: &SessionUsage,
        weekly: &WeeklyUsage,
    ) {
        Self::check_bar(
            app,
            cfg,
            "cctide - session (5h)",
            "Session usage",
            session.percent,
            &mut self.session_level,
        );
        Self::check_bar(
            app,
            cfg,
            "cctide - weekly limit",
            "Weekly usage",
            weekly.percent,
            &mut self.weekly_level,
        );

        // Recalibration nudge: fire once when enough tokens have been consumed
        // since the first calibration to make a second point meaningful (≥25% of
        // budget). Re-arms when the window rolls over.
        self.check_recal(
            app,
            cfg,
            session.weighted_tokens,
            session.window_start,
            weekly.weighted_tokens,
            weekly.week_start,
        );
    }

    fn check_recal(
        &mut self,
        app: &AppHandle,
        cfg: &Config,
        session_tokens: f64,
        session_window_start: Option<i64>,
        weekly_tokens: f64,
        weekly_week_start: Option<i64>,
    ) {
        if !cfg.notifications_enabled {
            return;
        }

        // Re-arm when the 5h window rolls over while the second point is still missing,
        // so the nudge can fire again in the new window.
        if session_window_start != self.last_session_window {
            if cfg.session_calibration_2.is_none() {
                self.recal_session_sent = false;
            }
            self.last_session_window = session_window_start;
        }
        if weekly_week_start != self.last_weekly_week {
            if cfg.weekly_calibration_2.is_none() {
                self.recal_weekly_sent = false;
            }
            self.last_weekly_week = weekly_week_start;
        }

        // Session
        if cfg.session_calibration_2.is_some() {
            self.recal_session_sent = true; // second point exists, no longer needed
        } else if let Some(c1) = &cfg.session_calibration {
            if !self.recal_session_sent && c1.budget > 0.0 {
                let k1 = c1.budget * (c1.percent / 100.0);
                if (session_tokens - k1) / c1.budget >= 0.25 {
                    notify(
                        app,
                        "cctide",
                        "Calibrate one final time for better accuracy.",
                    );
                    self.recal_session_sent = true;
                }
            }
        }
        // Weekly
        if cfg.weekly_calibration_2.is_some() {
            self.recal_weekly_sent = true;
        } else if let Some(c1) = &cfg.weekly_calibration {
            if !self.recal_weekly_sent && c1.budget > 0.0 {
                let k1 = c1.budget * (c1.percent / 100.0);
                if (weekly_tokens - k1) / c1.budget >= 0.25 {
                    notify(
                        app,
                        "cctide",
                        "Calibrate one final time for better accuracy.",
                    );
                    self.recal_weekly_sent = true;
                }
            }
        }
    }

    fn check_bar(
        app: &AppHandle,
        cfg: &Config,
        title: &str,
        what: &str,
        pct: Option<f64>,
        stored: &mut u8,
    ) {
        let lvl = level_for(pct, &cfg.alert_levels);
        if lvl > *stored {
            if cfg.notifications_enabled {
                let reached = cfg
                    .alert_levels
                    .get(lvl as usize - 1)
                    .copied()
                    .unwrap_or(0.0);
                notify(app, title, &format!("{what} reached {reached:.0}%"));
            }
            *stored = lvl; // advance even if disabled, to avoid replaying later
        } else if lvl < *stored {
            *stored = lvl; // dropped below a level → re-arm
        }
    }
}
