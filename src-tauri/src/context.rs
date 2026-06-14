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
    /// "interactive" for user-facing sessions; other values (or absent on older
    /// Claude Code versions) for background/sub-agent processes.
    #[serde(default)]
    kind: Option<String>,
    /// How the session was launched: "cli", "claude-vscode", …
    #[serde(default)]
    entrypoint: Option<String>,
    /// Live state written by recent Claude Code versions (e.g. "idle").
    #[serde(default)]
    status: Option<String>,
    /// Last activity, Unix **milliseconds** (absent on older versions).
    #[serde(rename = "updatedAt", default)]
    updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionCtx {
    pub session_id: String,
    pub pid: u32,
    pub cwd: String,
    pub version: String,
    pub model: Option<String>,
    pub context_tokens: u64,
    pub context_limit: u64,
    pub percent: Option<f64>,
    /// First user prompt, used as a human-readable title (None → UI falls back
    /// to the folder name).
    pub title: Option<String>,
    pub entrypoint: Option<String>,
    pub status: Option<String>,
    /// Last activity, Unix seconds (None on older Claude Code versions).
    pub updated_at: Option<i64>,
    /// Weighted tokens this session consumed in the current 5h window (0 when
    /// the window is not live or the transcript is unknown).
    pub weighted_5h: f64,
}

/// A session is shown "active" when its last activity is more recent than this.
const ACTIVE_THRESHOLD_SECS: i64 = 120;

/// Whether an open session is worth surfacing: it must have consumed something,
/// i.e. its transcript holds at least one assistant turn (non-zero context). A
/// fresh tab that has only run local slash-commands has `context_tokens == 0`
/// and is hidden, like a session with no transcript at all — nothing to show or
/// act on, and it never reaches Outcomes (no quota point in any window) either.
fn session_has_activity(context_tokens: u64) -> bool {
    context_tokens > 0
}

/// Status fallback: keep the session file's value when present (CLI sessions
/// write one), otherwise classify from the last-activity timestamp — VSCode
/// sessions often lack `status`/`updatedAt` entirely.
fn status_from(file_status: Option<String>, updated_at: Option<i64>, now: i64) -> Option<String> {
    file_status.or_else(|| {
        updated_at.map(|t| {
            if now - t < ACTIVE_THRESHOLD_SECS {
                "active".to_string()
            } else {
                "idle".to_string()
            }
        })
    })
}

pub fn sessions_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join("sessions"))
}

/// Resolves the transcript a session should display: its own (matched by
/// session id), else the cwd's most recent one (resumed sessions keep writing
/// to the original conversation's file) — unless another live session owns
/// that one, in which case a fresh empty tab must not mirror its sibling's
/// conversation. None → no conversation yet, the session is hidden.
fn resolve_transcript(
    cache: &ScanCache,
    session_id: &str,
    cwd: &str,
    owned: &std::collections::HashSet<PathBuf>,
) -> Option<PathBuf> {
    cache.jsonl_for_session(session_id).or_else(|| {
        cache
            .latest_jsonl_for_cwd(cwd)
            .filter(|p| !owned.contains(p))
    })
}

/// Lists open sessions (alive PID) with their context fill level.
/// `sys` must have been refreshed by the caller before this call.
/// `window_start`/`now` bound the current live 5h window for the per-session
/// quota attribution (`window_start: None` → `weighted_5h` is 0).
pub fn active_sessions(
    cache: &ScanCache,
    cfg: &Config,
    sys: &System,
    models: &crate::models::Models,
    window_start: Option<i64>,
    now: i64,
) -> Vec<SessionCtx> {
    let Some(dir) = sessions_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    // Pass 1: parse the session files of live, interactive processes.
    let mut session_files: Vec<SessionFile> = Vec::new();
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

        // Keep only user-facing sessions. Older Claude Code versions don't
        // write `kind`, so an absent field is treated as interactive.
        if sf.kind.as_deref().is_some_and(|k| k != "interactive") {
            continue;
        }

        session_files.push(sf);
    }

    // Transcripts owned (by id) by one of the live sessions. A session without
    // its own transcript may borrow the cwd's most recent one (resume case),
    // but never one of these — a fresh empty tab would otherwise mirror
    // another live session's conversation and show up as a duplicate.
    let owned: std::collections::HashSet<PathBuf> = session_files
        .iter()
        .filter_map(|sf| cache.jsonl_for_session(&sf.session_id))
        .collect();

    // Pass 2: resolve each session's transcript and build the view. Sessions
    // with no transcript at all (fresh tab, no conversation yet) are hidden.
    let mut out = Vec::new();
    for sf in session_files {
        let Some(transcript) = resolve_transcript(cache, &sf.session_id, &sf.cwd, &owned) else {
            continue;
        };

        let (model, context_tokens) = match cache.last_ctx_for(&transcript) {
            Some(ctx) => (Some(ctx.model), ctx.context_tokens),
            None => (None, 0),
        };

        // Hide sessions that have consumed nothing yet — a freshly-opened tab
        // whose transcript has no assistant turn. Like the no-transcript case
        // above, there is nothing to show (0 / limit) or act on, and it would
        // never appear in Outcomes either (no quota point in any window).
        if !session_has_activity(context_tokens) {
            continue;
        }

        let context_limit = match &model {
            Some(m) => models.context_limit_for(m, &cfg.context_limits),
            None => 200_000,
        };
        let percent = if context_limit > 0 {
            Some(context_tokens as f64 / context_limit as f64 * 100.0)
        } else {
            None
        };

        let title = cache.first_prompt_for(&transcript);

        let weighted_5h = match window_start {
            Some(ws) => cache.weighted_for_file(&transcript, ws, now),
            None => 0.0,
        };

        // Last activity: the session file's `updatedAt` when present, else the
        // transcript's mtime (written on every turn).
        let updated_at = sf
            .updated_at
            .map(|ms| ms / 1000)
            .or_else(|| cache.mtime_for(&transcript));
        let status = status_from(sf.status, updated_at, now);

        out.push(SessionCtx {
            session_id: sf.session_id,
            pid: sf.pid,
            cwd: sf.cwd,
            version: sf.version,
            model,
            context_tokens,
            context_limit,
            percent,
            title,
            entrypoint: sf.entrypoint,
            status,
            updated_at,
            weighted_5h,
        });
    }

    // Deduplicate by session id (resume → multiple PIDs, same session). Distinct
    // interactive sessions in the same cwd are kept separate: several tabs or
    // terminals on one project are all real sessions the user wants to see.
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
            pid: 1,
            cwd: cwd.to_string(),
            version: "1.0".to_string(),
            model: None,
            context_tokens: tokens,
            context_limit: limit,
            percent,
            title: None,
            entrypoint: None,
            status: None,
            updated_at: None,
            weighted_5h: 0.0,
        }
    }

    /// Mirrors the dedup logic in `active_sessions`.
    fn dedup(sessions: Vec<SessionCtx>) -> Vec<SessionCtx> {
        let mut by_session: HashMap<String, SessionCtx> = HashMap::new();
        for s in sessions {
            let entry = by_session
                .entry(s.session_id.clone())
                .or_insert_with(|| s.clone());
            if s.context_tokens > entry.context_tokens {
                *entry = s;
            }
        }
        by_session.into_values().collect()
    }

    #[test]
    fn dedup_by_session_id_keeps_richest() {
        // Same session id twice (resumed session) → collapsed to richest context.
        let kept = dedup(vec![
            ctx("a", "/proj", 50_000, 200_000, None),
            ctx("a", "/proj", 120_000, 200_000, None),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].context_tokens, 120_000);
    }

    #[test]
    fn same_cwd_distinct_sessions_kept_separate() {
        // Two interactive sessions in the same project (e.g. two editor tabs)
        // have distinct session ids → both shown.
        let kept = dedup(vec![
            ctx("tab1", "/proj", 80_000, 200_000, None),
            ctx("tab2", "/proj", 10_000, 200_000, None),
        ]);
        assert_eq!(kept.len(), 2, "independent sessions in same cwd kept");
    }

    #[test]
    fn different_cwds_kept_separate() {
        // Two sessions in different cwds → both shown.
        let kept = dedup(vec![
            ctx("a", "/proj-a", 50_000, 200_000, None),
            ctx("b", "/proj-b", 10_000, 200_000, None),
        ]);
        assert_eq!(kept.len(), 2);
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

    // --- resolve_transcript ---

    use crate::scan::encode_cwd;
    use std::collections::HashSet;

    /// Cache with transcripts under `/proj`'s project folder: `own.jsonl`
    /// (mtime 100) and `other.jsonl` (mtime 200, the most recent).
    fn two_transcript_cache() -> (ScanCache, PathBuf, PathBuf) {
        let mut cache = ScanCache::default();
        let dir = format!("/root/.claude/projects/{}", encode_cwd("/proj"));
        let own = PathBuf::from(format!("{dir}/own.jsonl"));
        let other = PathBuf::from(format!("{dir}/other.jsonl"));
        cache.insert_test_transcript(own.clone(), 100);
        cache.insert_test_transcript(other.clone(), 200);
        (cache, own, other)
    }

    #[test]
    fn resolve_prefers_own_transcript_over_newer_sibling() {
        // The session has its own file → used even though a more recently
        // modified transcript exists in the same cwd.
        let (cache, own, _) = two_transcript_cache();
        let got = resolve_transcript(&cache, "own", "/proj", &HashSet::new());
        assert_eq!(got, Some(own));
    }

    #[test]
    fn resolve_borrows_latest_unowned_cwd_transcript() {
        // Resume case: no file under the live session's id → borrow the cwd's
        // most recent transcript when no other live session owns it.
        let (cache, _, other) = two_transcript_cache();
        let got = resolve_transcript(&cache, "resumed-id", "/proj", &HashSet::new());
        assert_eq!(got, Some(other));
    }

    #[test]
    fn resolve_never_borrows_a_transcript_owned_by_a_live_session() {
        // Fresh empty tab next to a live session: the cwd's latest transcript
        // belongs to that sibling → not borrowed, the tab stays hidden.
        let (cache, _, other) = two_transcript_cache();
        let owned: HashSet<PathBuf> = [other].into_iter().collect();
        let got = resolve_transcript(&cache, "fresh-tab", "/proj", &owned);
        assert_eq!(got, None);
    }

    #[test]
    fn resolve_none_for_unknown_cwd() {
        let (cache, _, _) = two_transcript_cache();
        let got = resolve_transcript(&cache, "any", "/elsewhere", &HashSet::new());
        assert_eq!(got, None);
    }

    // --- status_from ---

    #[test]
    fn status_from_keeps_file_value() {
        let s = status_from(Some("idle".to_string()), Some(1_000_000), 1_000_010);
        assert_eq!(s.as_deref(), Some("idle"));
    }

    #[test]
    fn status_from_recent_activity_is_active() {
        let s = status_from(None, Some(1_000_000), 1_000_000 + ACTIVE_THRESHOLD_SECS - 1);
        assert_eq!(s.as_deref(), Some("active"));
    }

    #[test]
    fn status_from_old_activity_is_idle() {
        let s = status_from(None, Some(1_000_000), 1_000_000 + ACTIVE_THRESHOLD_SECS);
        assert_eq!(s.as_deref(), Some("idle"));
    }

    #[test]
    fn status_from_nothing_is_none() {
        assert_eq!(status_from(None, None, 1_000_000), None);
    }

    // --- session_has_activity (0-token hiding) ---

    #[test]
    fn zero_context_session_is_hidden() {
        // A fresh tab with no assistant turn (context 0) is not surfaced…
        assert!(!session_has_activity(0));
        // …but any real consumption makes it visible.
        assert!(session_has_activity(1));
        assert!(session_has_activity(150_000));
    }
}
