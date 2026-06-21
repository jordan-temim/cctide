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
    /// Newest app version already announced via OS notification.
    update_version: Option<String>,
}

/// Shows an OS notification. Returns whether the send succeeded — callers that
/// must retry on failure (e.g. the update notice) rely on this.
fn notify(app: &AppHandle, title: &str, body: &str) -> bool {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .is_ok()
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
    }

    /// Announces an available update once per version. Called from the ticker
    /// loop (app fully initialized) rather than inline in the network check,
    /// where a too-early send was silently dropped by the OS. The version
    /// marker only advances on a successful send, so a dropped notification is
    /// retried on the next ticker cycle instead of being lost. Pass `None` when
    /// no update is available, to re-arm for a future version.
    pub fn check_update(&mut self, app: &AppHandle, version: Option<&str>) {
        match version {
            None => self.update_version = None,
            Some(v) => {
                if self.update_version.as_deref() == Some(v) {
                    return;
                }
                let body = format!("v{v} — open cctide to install");
                if notify(app, "cctide update available", &body) {
                    self.update_version = Some(v.to_string());
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
