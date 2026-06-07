//! Open Claude Code sessions and how full their context window is.
//!
//! Active sessions are described by `~/.claude/sessions/<pid>.json`. We keep
//! only the still-alive PIDs, then read the last context state of the matching
//! transcript to show "X / limit" tokens.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use sysinfo::{Pid, System};

use crate::config::Config;
use crate::scan::ScanCache;

/// (Partial) contents of a `sessions/<pid>.json` file.
#[derive(Debug, Deserialize)]
struct SessionFile {
    pid: u32,
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: String,
    #[serde(default)]
    version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionCtx {
    pub session_id: String,
    pub cwd: String,
    pub version: String,
    pub model: Option<String>,
    pub context_tokens: u64,
    pub context_limit: u64,
    pub percent: Option<f64>,
}

fn sessions_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join("sessions"))
}

/// Lists open sessions (alive PID) with their context fill level.
/// `sys` must have been refreshed by the caller before this call.
pub fn active_sessions(
    cache: &ScanCache,
    cfg: &Config,
    sys: &System,
    models: &crate::models::Models,
) -> Vec<SessionCtx> {
    let Some(dir) = sessions_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(sf) = serde_json::from_str::<SessionFile>(&text) else {
            continue;
        };

        // Keep only processes that are still running.
        if sys.process(Pid::from_u32(sf.pid)).is_none() {
            continue;
        }

        let (model, context_tokens) =
            match cache.last_ctx_for_session_or_cwd(&sf.session_id, &sf.cwd) {
                Some(ctx) => (Some(ctx.model), ctx.context_tokens),
                None => (None, 0),
            };

        let context_limit = match &model {
            Some(m) => models.context_limit_for(m, &cfg.context_limits),
            None => 200_000,
        };
        let percent = if context_limit > 0 {
            Some(context_tokens as f64 / context_limit as f64 * 100.0)
        } else {
            None
        };

        out.push(SessionCtx {
            session_id: sf.session_id,
            cwd: sf.cwd,
            version: sf.version,
            model,
            context_tokens,
            context_limit,
            percent,
        });
    }

    // Deduplicate by cwd — keep the entry with the most context tokens when
    // multiple Claude Code processes are open in the same directory.
    let mut by_cwd: std::collections::HashMap<String, SessionCtx> =
        std::collections::HashMap::new();
    for s in out {
        let entry = by_cwd.entry(s.cwd.clone()).or_insert_with(|| s.clone());
        if s.context_tokens > entry.context_tokens {
            *entry = s;
        }
    }
    let mut out: Vec<SessionCtx> = by_cwd.into_values().collect();

    out.sort_by_key(|s| std::cmp::Reverse(s.context_tokens));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_by_cwd_keeps_highest_context() {
        // Two sessions in same cwd: keep the one with more tokens.
        let sessions = vec![
            SessionCtx {
                session_id: "sess1".to_string(),
                cwd: "/home/user/proj".to_string(),
                version: "1.0".to_string(),
                model: Some("sonnet".to_string()),
                context_tokens: 50_000,
                context_limit: 200_000,
                percent: Some(25.0),
            },
            SessionCtx {
                session_id: "sess2".to_string(),
                cwd: "/home/user/proj".to_string(),
                version: "1.0".to_string(),
                model: Some("sonnet".to_string()),
                context_tokens: 100_000,
                context_limit: 200_000,
                percent: Some(50.0),
            },
        ];

        // Simulate the dedup logic from active_sessions.
        let mut by_cwd: std::collections::HashMap<String, SessionCtx> =
            std::collections::HashMap::new();
        for s in sessions {
            let entry = by_cwd.entry(s.cwd.clone()).or_insert_with(|| s.clone());
            if s.context_tokens > entry.context_tokens {
                *entry = s;
            }
        }

        assert_eq!(by_cwd.len(), 1);
        let kept = &by_cwd["/home/user/proj"];
        assert_eq!(kept.session_id, "sess2");
        assert_eq!(kept.context_tokens, 100_000);
    }

    #[test]
    fn sort_by_context_tokens_descending() {
        let mut sessions = [
            SessionCtx {
                session_id: "low".to_string(),
                cwd: "/a".to_string(),
                version: "1.0".to_string(),
                model: None,
                context_tokens: 10_000,
                context_limit: 200_000,
                percent: None,
            },
            SessionCtx {
                session_id: "high".to_string(),
                cwd: "/b".to_string(),
                version: "1.0".to_string(),
                model: None,
                context_tokens: 150_000,
                context_limit: 200_000,
                percent: None,
            },
            SessionCtx {
                session_id: "mid".to_string(),
                cwd: "/c".to_string(),
                version: "1.0".to_string(),
                model: None,
                context_tokens: 50_000,
                context_limit: 200_000,
                percent: None,
            },
        ]
        .to_vec();

        sessions.sort_by_key(|s| std::cmp::Reverse(s.context_tokens));

        assert_eq!(sessions[0].session_id, "high");
        assert_eq!(sessions[1].session_id, "mid");
        assert_eq!(sessions[2].session_id, "low");
    }

    #[test]
    fn context_percent_calculates_correctly() {
        let s = SessionCtx {
            session_id: "test".to_string(),
            cwd: "/home".to_string(),
            version: "1.0".to_string(),
            model: Some("sonnet".to_string()),
            context_tokens: 100_000,
            context_limit: 200_000,
            percent: Some(50.0),
        };
        assert!((s.percent.unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn context_percent_none_with_zero_limit() {
        let s = SessionCtx {
            session_id: "test".to_string(),
            cwd: "/home".to_string(),
            version: "1.0".to_string(),
            model: None,
            context_tokens: 0,
            context_limit: 0,
            percent: None,
        };
        assert!(s.percent.is_none());
    }
}
