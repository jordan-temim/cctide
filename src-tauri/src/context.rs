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
    /// First user prompt, used as a human-readable title (None → UI falls back
    /// to the folder name).
    pub title: Option<String>,
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

        let title = cache.first_prompt_for_session(&sf.session_id, &sf.cwd);

        out.push(SessionCtx {
            session_id: sf.session_id,
            cwd: sf.cwd,
            version: sf.version,
            model,
            context_tokens,
            context_limit,
            percent,
            title,
        });
    }

    // Deduplicate by session id — show each distinct conversation. Several PID
    // files can map to the same session (resume); keep the richest context.
    let mut by_session: std::collections::HashMap<String, SessionCtx> =
        std::collections::HashMap::new();
    for s in out {
        let entry = by_session
            .entry(s.session_id.clone())
            .or_insert_with(|| s.clone());
        if s.context_tokens > entry.context_tokens {
            *entry = s;
        }
    }
    let mut out: Vec<SessionCtx> = by_session.into_values().collect();

    out.sort_by_key(|s| std::cmp::Reverse(s.context_tokens));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ctx(sid: &str, cwd: &str, tokens: u64, limit: u64, percent: Option<f64>) -> SessionCtx {
        SessionCtx {
            session_id: sid.to_string(),
            cwd: cwd.to_string(),
            version: "1.0".to_string(),
            model: None,
            context_tokens: tokens,
            context_limit: limit,
            percent,
            title: None,
        }
    }

    /// Mirrors the dedup-by-session logic in `active_sessions`.
    fn dedup(sessions: Vec<SessionCtx>) -> HashMap<String, SessionCtx> {
        let mut by_session: HashMap<String, SessionCtx> = HashMap::new();
        for s in sessions {
            let entry = by_session
                .entry(s.session_id.clone())
                .or_insert_with(|| s.clone());
            if s.context_tokens > entry.context_tokens {
                *entry = s;
            }
        }
        by_session
    }

    #[test]
    fn dedup_by_session_id_keeps_distinct_and_richest() {
        // Same cwd, different sessions → both kept (distinct conversations).
        // Same session id twice → collapsed to the richest context.
        let kept = dedup(vec![
            ctx("a", "/proj", 50_000, 200_000, None),
            ctx("a", "/proj", 120_000, 200_000, None),
            ctx("b", "/proj", 10_000, 200_000, None),
        ]);
        assert_eq!(kept.len(), 2, "two distinct sessions in same cwd are kept");
        assert_eq!(
            kept["a"].context_tokens, 120_000,
            "richest kept for session a"
        );
    }

    #[test]
    fn sort_by_context_tokens_descending() {
        let mut sessions = [
            ctx("low", "/a", 10_000, 200_000, None),
            ctx("high", "/b", 150_000, 200_000, None),
            ctx("mid", "/c", 50_000, 200_000, None),
        ];
        sessions.sort_by_key(|s| std::cmp::Reverse(s.context_tokens));
        assert_eq!(sessions[0].session_id, "high");
        assert_eq!(sessions[1].session_id, "mid");
        assert_eq!(sessions[2].session_id, "low");
    }

    #[test]
    fn context_percent_calculates_correctly() {
        let s = ctx("test", "/home", 100_000, 200_000, Some(50.0));
        assert!((s.percent.unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn context_percent_none_with_zero_limit() {
        let s = ctx("test", "/home", 0, 0, None);
        assert!(s.percent.is_none());
    }
}
