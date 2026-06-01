//! Optional surfacing of RTK (Rust Token Killer) stats.
//!
//! If the `rtk` binary is present on the machine, we read its analytics via
//! `rtk gain --format json` (fully local, no network). Otherwise we return None
//! and the UI section stays hidden.
//!
//! `rtk` is resolved via PATH. It is a user-installed tool; no path injection
//! is possible since we pass only static string arguments and spawn without a
//! shell. A 5-second timeout prevents hangs if the binary is slow or stuck.

use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::Duration;

const RTK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RtkSummary {
    #[serde(default)]
    pub total_commands: u64,
    #[serde(default)]
    pub total_input: u64,
    #[serde(default)]
    pub total_output: u64,
    #[serde(default)]
    pub total_saved: u64,
    #[serde(default)]
    pub avg_savings_pct: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RtkWeek {
    #[serde(default)]
    pub week_start: String,
    #[serde(default)]
    pub week_end: String,
    #[serde(default)]
    pub saved_tokens: u64,
    #[serde(default)]
    pub savings_pct: f64,
    #[serde(default)]
    pub commands: u64,
}

#[derive(Debug, Serialize)]
pub struct RtkSavings {
    pub summary: RtkSummary,
    pub weekly: Vec<RtkWeek>,
}

#[derive(Debug, Deserialize)]
struct GainOutput {
    summary: RtkSummary,
    #[serde(default)]
    weekly: Vec<RtkWeek>,
}

fn run_gain(extra: &[&str]) -> Option<GainOutput> {
    // Tauri apps do not inherit the login shell's PATH, so common locations
    // (/usr/local/bin on Intel Macs, /opt/homebrew/bin on Apple Silicon) are
    // missing. Prepend them so `rtk` is found regardless of install prefix.
    let path_env = std::env::var("PATH").unwrap_or_default();
    let extended_path = format!(
        "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:{}",
        path_env
    );

    let mut child = Command::new("rtk")
        .args(["gain", "--format", "json"])
        .args(extra)
        .env("PATH", extended_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Poll until done or timeout, then kill + reap to avoid zombie processes.
    let deadline = std::time::Instant::now() + RTK_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return child
                        .wait_with_output()
                        .ok()
                        .and_then(|o| serde_json::from_slice::<GainOutput>(&o.stdout).ok());
                }
                return None;
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                // Timeout or error: kill and reap the child before returning.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Returns RTK stats, or None if `rtk` is absent / unreadable.
pub fn savings() -> Option<RtkSavings> {
    // `--weekly` returns both `summary` and `weekly`.
    let weekly_out = run_gain(&["--weekly"]);
    match weekly_out {
        Some(o) => Some(RtkSavings {
            summary: o.summary,
            weekly: o.weekly,
        }),
        None => {
            // Fallback: at least the global summary.
            let o = run_gain(&[])?;
            Some(RtkSavings {
                summary: o.summary,
                weekly: Vec::new(),
            })
        }
    }
}
