//! Reads and aggregates Claude Code JSONL transcripts.
//!
//! Consumed tokens are read from `~/.claude/projects/<project>/<id>.jsonl`.
//! Each `type:"assistant"` line carries a `message.usage`. We extract points
//! (timestamp, weighted tokens) for the session/weekly windows, and the last
//! context state per file for per-session tracking.
//!
//! Large files (several MB) are cached by `(mtime, size)` to avoid re-parsing
//! everything on every refresh.
//!
//! ## Deduplication
//!
//! The same API response can appear in multiple JSONL files (resumed sessions,
//! sidechains) and even multiple times within the same file (interrupted writes).
//! Each `Point` carries a `key` — `fnv1a(message.id + requestId)`. Points are
//! deduplicated within `parse_file` (within-file) and again when building the
//! global deduplicated view in `ScanCache.refresh()` (cross-file). The result
//! is stored in `ScanCache.points` and rebuilt only when a file changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use crate::models::Models;

/// A consumption point: one assistant turn.
#[derive(Debug, Clone)]
pub struct Point {
    pub ts: i64, // Unix seconds
    pub weighted: f64,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64, // cache_5m + cache_1h
    pub model: String,
    /// Dedup key: fnv1a(message.id + requestId). Unique per API response.
    pub key: u64,
}

/// Last known context state of a session file.
#[derive(Debug, Clone)]
pub struct LastCtx {
    pub model: String,
    pub context_tokens: u64,
}

/// One Edit/Write/NotebookEdit tool call recorded in a transcript.
#[derive(Debug, Clone)]
pub struct EditRec {
    pub path: String,
    pub ts: i64,
}

/// A session's activity within a time window, for outcome classification:
/// where it ran, when, how much quota it consumed, and which files it edited.
#[derive(Debug, Clone)]
pub struct SessionSpan {
    pub cwd: Option<String>,
    pub first_ts: i64,
    pub last_ts: i64,
    pub weighted: f64,
    pub edits: Vec<EditRec>,
}

#[derive(Debug, Clone, Default)]
struct CachedFile {
    mtime: i64,
    size: u64,
    /// Points from this file, deduplicated within the file.
    points: Vec<Point>,
    last_ctx: Option<LastCtx>,
    /// First real user prompt — used as a human-readable session title.
    first_prompt: Option<String>,
    /// Working directory of the session (from the transcript's `cwd` field).
    cwd: Option<String>,
    /// Edit/Write tool calls, deduplicated alongside their parent records.
    edits: Vec<EditRec>,
}

/// FNV-1a hash of a string → u64 (for the dedup key).
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Encodes a working directory into its `~/.claude/projects` folder name
/// (every non-alphanumeric char becomes `-`, matching Claude Code).
pub fn encode_cwd(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// Global cache (kept in Tauri state across calls).
#[derive(Default)]
pub struct ScanCache {
    /// Per-file metadata + points. Points are deduped within each file.
    files: HashMap<PathBuf, CachedFile>,
    /// Global deduplicated view: key → Point. Rebuilt by `refresh()` whenever
    /// any file is reparsed or removed. Queried directly by all read methods.
    points: HashMap<u64, Point>,
}

/// Root directory of Claude Code projects.
pub fn projects_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join("projects"))
}

fn usage_u64(usage: &Value, key: &str) -> u64 {
    usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
}

/// Parses an ISO 8601 timestamp (`2026-05-29T17:26:09.467Z`) into Unix seconds.
fn parse_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// First real user prompt in a transcript, trimmed to a title length. Skips tool
/// results, slash-commands and local-command wrappers. Used to title a session.
fn first_user_prompt(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if !line.contains("\"user\"") {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if rec.get("type").and_then(|v| v.as_str()) != Some("user") {
            continue;
        }
        let prompt = match rec.get("message").and_then(|m| m.get("content")) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(" "),
            _ => continue,
        };
        let prompt = prompt.trim();
        // Skip tool results, slash-commands, and local-command wrappers.
        if prompt.is_empty() || prompt.starts_with('<') || prompt.starts_with('/') {
            continue;
        }
        let line1 = prompt.lines().next().unwrap_or(prompt).trim();
        // Cap to a short title length (<=35 chars), with an ellipsis if cut.
        if line1.chars().count() > 35 {
            return Some(format!("{}...", line1.chars().take(32).collect::<String>()));
        }
        return Some(line1.to_string());
    }
    None
}

/// (Re)reads a JSONL file into a `CachedFile` (mtime/size left for the caller):
/// points deduplicated within the file, last context state, first user prompt,
/// session cwd and Edit/Write tool calls.
fn parse_file(path: &Path, pricing: &Models) -> CachedFile {
    let mut parsed = CachedFile::default();
    let mut seen_keys: HashSet<u64> = HashSet::new();

    let Ok(text) = std::fs::read_to_string(path) else {
        return parsed;
    };
    parsed.first_prompt = first_user_prompt(&text);
    let path_str = path.to_string_lossy();

    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // The session cwd lives on most records, usage-bearing or not; grab it
        // from the first one that carries it, then stop paying the extra parse.
        if parsed.cwd.is_none() && line.contains("\"cwd\"") {
            if let Ok(rec) = serde_json::from_str::<Value>(line) {
                if let Some(c) = rec.get("cwd").and_then(|v| v.as_str()) {
                    parsed.cwd = Some(c.to_string());
                }
            }
        }
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if rec.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let Some(msg) = rec.get("message") else {
            continue;
        };
        let Some(usage) = msg.get("usage") else {
            continue;
        };

        let model = msg
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let input = usage_u64(usage, "input_tokens");
        let output = usage_u64(usage, "output_tokens");
        let cache_creation = usage_u64(usage, "cache_creation_input_tokens");
        let cache_read = usage_u64(usage, "cache_read_input_tokens");

        // Cache writes are split 5m / 1h (different price multipliers). Fall
        // back to treating the lump total as a 5m write if the split is absent.
        let (cache_5m, cache_1h) = match usage.get("cache_creation") {
            Some(c) => (
                c.get("ephemeral_5m_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                c.get("ephemeral_1h_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            ),
            None => (cache_creation, 0),
        };

        let ts = rec
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(parse_ts)
            .unwrap_or(0);

        // Dedup key: fnv1a(message.id + requestId). Fallback to a per-line
        // key when both are absent so distinct records are never collapsed.
        let mid = msg.get("id").and_then(|v| v.as_str());
        let rid = rec.get("requestId").and_then(|v| v.as_str());
        let key = match (mid, rid) {
            (None, None) => fnv1a(&format!("{path_str}#{line_no}")),
            (m, r) => fnv1a(&format!("{}\0{}", m.unwrap_or(""), r.unwrap_or(""))),
        };

        // Context occupancy = all tokens present in the window (cache-read included).
        let context_tokens = input + output + cache_creation + cache_read;

        parsed.last_ctx = Some(LastCtx {
            model: model.clone(),
            context_tokens,
        });

        // Skip duplicate entries within this file.
        if !seen_keys.insert(key) {
            continue;
        }

        if ts == 0 {
            continue;
        }

        // Edit/Write tool calls ride in the same assistant records as usage;
        // collect them behind the same dedup gate.
        if let Some(Value::Array(blocks)) = msg.get("content") {
            for b in blocks {
                if b.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                    continue;
                }
                let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if !matches!(name, "Edit" | "Write" | "NotebookEdit") {
                    continue;
                }
                let file = b
                    .get("input")
                    .and_then(|i| i.get("file_path").or_else(|| i.get("notebook_path")))
                    .and_then(|v| v.as_str());
                if let Some(p) = file {
                    parsed.edits.push(EditRec {
                        path: p.to_string(),
                        ts,
                    });
                }
            }
        }

        // Quota weighting excludes cache reads — see models.rs for rationale.
        let weighted = pricing.quota_units(&model, input, output, cache_5m, cache_1h);
        let cost_usd = pricing.cost_usd(&model, input, output, cache_5m, cache_1h);
        parsed.points.push(Point {
            ts,
            weighted,
            cost_usd,
            input_tokens: input,
            output_tokens: output,
            cache_write_tokens: cache_5m + cache_1h,
            model,
            key,
        });
    }

    parsed
}

impl ScanCache {
    /// Refreshes the cache: only re-parses files that changed, then rebuilds
    /// the global deduplicated point map if anything changed.
    pub fn refresh(&mut self, pricing: &Models) {
        let Some(root) = projects_dir() else { return };
        if !root.exists() {
            return;
        }

        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut dirty = false;

        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let size = meta.len();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let pb = path.to_path_buf();
            seen.insert(pb.clone());

            let needs_parse = match self.files.get(&pb) {
                Some(c) => c.mtime != mtime || c.size != size,
                None => true,
            };
            if needs_parse {
                let mut parsed = parse_file(path, pricing);
                parsed.mtime = mtime;
                parsed.size = size;
                self.files.insert(pb, parsed);
                dirty = true;
            }
        }

        // Drop files that disappeared from disk.
        let before = self.files.len();
        self.files.retain(|k, _| seen.contains(k));
        if self.files.len() != before {
            dirty = true;
        }

        // Rebuild the global deduplicated map only when something changed.
        if dirty {
            self.rebuild_points();
        }
    }

    /// Rebuilds the deduplicated global point map from all cached files.
    /// Cross-file dedup: first file encountered for a given key wins.
    fn rebuild_points(&mut self) {
        self.points.clear();
        for file in self.files.values() {
            for p in &file.points {
                self.points.entry(p.key).or_insert_with(|| p.clone());
            }
        }
    }

    /// All deduplicated consumption points across every project.
    pub fn all_points(&self) -> Vec<Point> {
        self.points.values().cloned().collect()
    }

    /// Last known context for a given session file.
    pub fn last_ctx_for(&self, path: &Path) -> Option<LastCtx> {
        self.files.get(path).and_then(|f| f.last_ctx.clone())
    }

    /// Finds a session's JSONL file by its id (`<sessionId>.jsonl`), across all
    /// projects — robust to how the working directory is encoded in the
    /// project folder name.
    pub fn jsonl_for_session(&self, session_id: &str) -> Option<PathBuf> {
        let target = format!("{session_id}.jsonl");
        self.files
            .keys()
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(target.as_str()))
            .cloned()
    }

    /// Project folder (`~/.claude/projects/<encoded-cwd>`) for a working dir,
    /// if any of its transcripts are cached.
    pub fn project_dir_for_cwd(&self, cwd: &str) -> Option<PathBuf> {
        let enc = encode_cwd(cwd);
        self.files
            .keys()
            .filter_map(|p| p.parent())
            .find(|d| d.file_name().and_then(|n| n.to_str()) == Some(enc.as_str()))
            .map(|d| d.to_path_buf())
    }

    /// Most recently modified transcript in the project folder of `cwd` — the
    /// fallback when a session has no transcript under its own id (resumed
    /// sessions keep writing to the original conversation's file).
    pub fn latest_jsonl_for_cwd(&self, cwd: &str) -> Option<PathBuf> {
        let enc = encode_cwd(cwd);
        self.files
            .iter()
            .filter(|(p, _)| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .and_then(|n| n.to_str())
                    == Some(enc.as_str())
            })
            .max_by_key(|(_, f)| f.mtime)
            .map(|(p, _)| p.clone())
    }

    /// First user prompt of a transcript (used as the session title).
    pub fn first_prompt_for(&self, path: &Path) -> Option<String> {
        self.files.get(path).and_then(|f| f.first_prompt.clone())
    }

    /// Last-modified time of a transcript, Unix seconds. The transcript is
    /// written on every turn, so this is a reliable last-activity signal.
    pub fn mtime_for(&self, path: &Path) -> Option<i64> {
        self.files.get(path).map(|f| f.mtime).filter(|m| *m > 0)
    }

    /// Test-only: registers a bare transcript (mtime only) so other modules'
    /// tests can shape the cache (`files` is private).
    #[cfg(test)]
    pub(crate) fn insert_test_transcript(&mut self, path: PathBuf, mtime: i64) {
        self.files.insert(
            path,
            CachedFile {
                mtime,
                ..Default::default()
            },
        );
    }

    /// One span per transcript with activity in `[from, now]`: session cwd,
    /// activity bounds, weighted consumption and file edits, all clamped to the
    /// window. Same within-file-dedup approximation as `weighted_for_file`.
    pub fn session_edit_spans(&self, from: i64, now: i64) -> Vec<SessionSpan> {
        let mut spans: Vec<SessionSpan> = Vec::new();
        for file in self.files.values() {
            let in_window: Vec<&Point> = file
                .points
                .iter()
                .filter(|p| p.ts >= from && p.ts <= now)
                .collect();
            if in_window.is_empty() {
                continue;
            }
            spans.push(SessionSpan {
                cwd: file.cwd.clone(),
                first_ts: in_window.iter().map(|p| p.ts).min().unwrap_or(from),
                last_ts: in_window.iter().map(|p| p.ts).max().unwrap_or(now),
                weighted: in_window.iter().map(|p| p.weighted).sum(),
                edits: file
                    .edits
                    .iter()
                    .filter(|e| e.ts >= from && e.ts <= now)
                    .cloned()
                    .collect(),
            });
        }
        spans
    }

    /// Sum of a transcript's weighted tokens within `[from, now]`. Uses the
    /// file's own within-file-deduped points: cross-file duplicates are not
    /// subtracted, which is acceptable for a per-session breakdown (the global
    /// gauges stay fully deduped).
    pub fn weighted_for_file(&self, path: &Path, from: i64, now: i64) -> f64 {
        match self.files.get(path) {
            Some(f) => f
                .points
                .iter()
                .filter(|p| p.ts >= from && p.ts <= now)
                .map(|p| p.weighted)
                .sum(),
            None => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- deduplication via all_points ---

    #[test]
    fn dedup_by_key() {
        // Two points with the same key in different files → counted only once.
        let mut cache = ScanCache::default();
        let shared_key = 42u64;
        let p1 = Point {
            ts: 1000,
            weighted: 50.0,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            cache_write_tokens: 0,
            model: "claude-sonnet-4-6".to_string(),
            key: shared_key,
        };
        let p2 = Point {
            ts: 2000,
            weighted: 50.0,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            cache_write_tokens: 0,
            model: "claude-sonnet-4-6".to_string(),
            key: shared_key,
        };
        cache.files.insert(
            PathBuf::from("file1"),
            CachedFile {
                mtime: 0,
                size: 0,
                points: vec![p1],
                last_ctx: None,
                first_prompt: None,
                ..Default::default()
            },
        );
        cache.files.insert(
            PathBuf::from("file2"),
            CachedFile {
                mtime: 0,
                size: 0,
                points: vec![p2],
                last_ctx: None,
                first_prompt: None,
                ..Default::default()
            },
        );
        cache.rebuild_points();
        let pts = cache.all_points();
        assert_eq!(pts.len(), 1);
        assert!((pts[0].weighted - 50.0).abs() < 1e-9);
    }

    // --- encode_cwd ---

    #[test]
    fn encode_cwd_replaces_non_alphanumeric() {
        assert_eq!(
            encode_cwd("/Users/alice/my project"),
            "-Users-alice-my-project"
        );
    }

    #[test]
    fn encode_cwd_preserves_alphanumeric() {
        assert_eq!(encode_cwd("abc123"), "abc123");
    }

    // --- fnv1a ---

    #[test]
    fn fnv1a_deterministic() {
        assert_eq!(fnv1a("hello|world"), fnv1a("hello|world"));
    }

    #[test]
    fn fnv1a_different_inputs_differ() {
        assert_ne!(fnv1a("abc|def"), fnv1a("def|abc"));
        assert_ne!(fnv1a("msg1|req1"), fnv1a("msg2|req1"));
    }

    #[test]
    fn fnv1a_empty_string_is_offset_basis() {
        assert_eq!(fnv1a(""), 0xcbf29ce484222325);
    }

    // --- parse_ts ---

    #[test]
    fn parse_ts_valid_rfc3339() {
        let ts = parse_ts("2026-05-29T17:26:09.467Z");
        assert!(ts.is_some());
        assert!(ts.unwrap() > 0);
    }

    #[test]
    fn parse_ts_invalid_returns_none() {
        assert!(parse_ts("not-a-date").is_none());
        assert!(parse_ts("").is_none());
    }

    // --- rebuild_points (cross-file dedup) ---

    #[test]
    fn rebuild_points_dedup_across_files() {
        // Multiple files, some with same keys.
        let mut cache = ScanCache::default();

        // File 1: points with keys 1, 2
        cache.files.insert(
            PathBuf::from("file1"),
            CachedFile {
                mtime: 100,
                size: 1000,
                points: vec![
                    Point {
                        ts: 1000,
                        weighted: 10.0,
                        cost_usd: 0.0,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_write_tokens: 0,
                        model: "sonnet".to_string(),
                        key: 1,
                    },
                    Point {
                        ts: 1100,
                        weighted: 20.0,
                        cost_usd: 0.0,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_write_tokens: 0,
                        model: "sonnet".to_string(),
                        key: 2,
                    },
                ],
                last_ctx: None,
                first_prompt: None,
                ..Default::default()
            },
        );

        // File 2: keys 2 (dup), 3 (new). Key 2 appears in both; whichever file is
        // iterated first by the HashMap wins (order is non-deterministic).
        cache.files.insert(
            PathBuf::from("file2"),
            CachedFile {
                mtime: 200,
                size: 2000,
                points: vec![
                    Point {
                        ts: 2000,
                        weighted: 99.0,
                        cost_usd: 0.0,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_write_tokens: 0,
                        model: "sonnet".to_string(),
                        key: 2,
                    },
                    Point {
                        ts: 2100,
                        weighted: 30.0,
                        cost_usd: 0.0,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_write_tokens: 0,
                        model: "sonnet".to_string(),
                        key: 3,
                    },
                ],
                last_ctx: None,
                first_prompt: None,
                ..Default::default()
            },
        );

        cache.rebuild_points();

        // Should have 3 unique keys: 1, 2, 3.
        assert_eq!(cache.points.len(), 3);
        // Key 2 must be exactly one of the two candidates (20.0 from file1 or 99.0 from
        // file2). Which one wins depends on HashMap iteration order.
        let key2 = cache.points[&2].weighted;
        assert!(
            (key2 - 20.0).abs() < 1e-9 || (key2 - 99.0).abs() < 1e-9,
            "key 2 should be 20.0 or 99.0, got {key2}"
        );
    }

    // --- latest_jsonl_for_cwd ---

    #[test]
    fn latest_jsonl_for_cwd_picks_most_recent() {
        let mut cache = ScanCache::default();
        let cwd = "/home/user/proj";
        let encoded_cwd = encode_cwd(cwd);

        for (name, mtime) in [("old", 100), ("new", 200)] {
            cache.files.insert(
                PathBuf::from(format!("/root/.claude/projects/{encoded_cwd}/{name}.jsonl")),
                CachedFile {
                    mtime,
                    size: 500,
                    points: vec![],
                    last_ctx: Some(LastCtx {
                        model: "opus".to_string(),
                        context_tokens: 50_000,
                    }),
                    first_prompt: None,
                    ..Default::default()
                },
            );
        }

        let path = cache.latest_jsonl_for_cwd(cwd);
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains("new.jsonl"));
        // The resolved path feeds the path-based getters.
        assert_eq!(cache.last_ctx_for(&path).unwrap().context_tokens, 50_000);
        assert_eq!(cache.mtime_for(&path), Some(200));
    }

    #[test]
    fn latest_jsonl_for_cwd_none_for_unknown_project() {
        let cache = ScanCache::default();
        assert!(cache.latest_jsonl_for_cwd("/nowhere").is_none());
    }

    #[test]
    fn jsonl_for_session_finds_by_filename() {
        let mut cache = ScanCache::default();
        let session_id = "abc123";

        cache.files.insert(
            PathBuf::from(format!("/root/.claude/projects/proj1/{session_id}.jsonl")),
            CachedFile {
                mtime: 100,
                size: 500,
                points: vec![],
                last_ctx: None,
                first_prompt: None,
                ..Default::default()
            },
        );
        cache.files.insert(
            PathBuf::from("/root/.claude/projects/proj2/other.jsonl"),
            CachedFile {
                mtime: 100,
                size: 500,
                points: vec![],
                last_ctx: None,
                first_prompt: None,
                ..Default::default()
            },
        );

        let path = cache.jsonl_for_session(session_id);
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().contains(session_id));
    }

    #[test]
    fn project_dir_for_cwd_finds_by_encoded_name() {
        let mut cache = ScanCache::default();
        let cwd = "/Users/alice/my project/src";
        let encoded = encode_cwd(cwd);

        let project_dir = PathBuf::from(format!("/root/.claude/projects/{encoded}"));
        let file_path = project_dir.join("session1.jsonl");

        cache.files.insert(
            file_path,
            CachedFile {
                mtime: 100,
                size: 500,
                points: vec![],
                last_ctx: None,
                first_prompt: None,
                ..Default::default()
            },
        );

        let found = cache.project_dir_for_cwd(cwd);
        assert!(found.is_some());
        let found_path = found.unwrap();
        assert!(found_path.to_string_lossy().contains(&encoded));
    }

    // --- weighted_for_file ---

    #[test]
    fn weighted_for_file_sums_window_only() {
        let mut cache = ScanCache::default();
        let path = PathBuf::from("/root/.claude/projects/proj/sess1.jsonl");
        cache.files.insert(
            path.clone(),
            CachedFile {
                mtime: 100,
                size: 500,
                points: vec![
                    pt_for(1_000, 10.0),
                    pt_for(2_000, 20.0),
                    pt_for(9_000, 99.0),
                ],
                last_ctx: None,
                first_prompt: None,
                ..Default::default()
            },
        );
        // Window [1_000, 5_000] keeps the first two points only.
        let w = cache.weighted_for_file(&path, 1_000, 5_000);
        assert!((w - 30.0).abs() < 1e-9);
    }

    #[test]
    fn weighted_for_file_unknown_returns_zero() {
        let cache = ScanCache::default();
        assert_eq!(
            cache.weighted_for_file(Path::new("/none.jsonl"), 0, 10_000),
            0.0
        );
    }

    fn pt_for(ts: i64, weighted: f64) -> Point {
        Point {
            ts,
            weighted,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            cache_write_tokens: 0,
            model: "claude-sonnet-4-6".to_string(),
            key: ts as u64,
        }
    }

    // --- first_user_prompt ---

    #[test]
    fn first_user_prompt_takes_first_text_message() {
        let text = concat!(
            r#"{"type":"user","message":{"content":"hello there"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"usage":{}}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"second"}}"#,
        );
        assert_eq!(first_user_prompt(text).as_deref(), Some("hello there"));
    }

    #[test]
    fn first_user_prompt_from_text_blocks() {
        let text = r#"{"type":"user","message":{"content":[{"type":"text","text":"from array"}]}}"#;
        assert_eq!(first_user_prompt(text).as_deref(), Some("from array"));
    }

    #[test]
    fn first_user_prompt_skips_tool_results_and_commands() {
        let text = concat!(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"<command-name>/m</command-name>"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"/clear"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"real prompt"}}"#,
        );
        assert_eq!(first_user_prompt(text).as_deref(), Some("real prompt"));
    }

    #[test]
    fn first_user_prompt_truncates_to_35_with_ellipsis() {
        let long = "x".repeat(100);
        let text = format!(r#"{{"type":"user","message":{{"content":"{long}"}}}}"#);
        let t = first_user_prompt(&text).unwrap();
        assert_eq!(t.chars().count(), 35);
        assert!(t.ends_with("..."));
    }

    #[test]
    fn first_user_prompt_none_without_prompt() {
        let text = r#"{"type":"assistant","message":{"usage":{}}}"#;
        assert_eq!(first_user_prompt(text), None);
    }

    #[test]
    fn first_user_prompt_takes_first_line_of_multiline_content() {
        // JSON \n escape → actual newline; first_user_prompt must take first line only.
        let text =
            "{\"type\":\"user\",\"message\":{\"content\":\"line one\\nline two\\nline three\"}}";
        let t = first_user_prompt(text).unwrap();
        assert_eq!(t, "line one");
    }

    // --- parse_file: cwd + edit extraction ---

    #[test]
    fn parse_file_extracts_cwd_and_edits() {
        let text = concat!(
            r#"{"type":"user","cwd":"/home/u/proj","message":{"content":"do it"}}"#,
            "\n",
            r#"{"type":"assistant","timestamp":"2026-06-01T10:00:00Z","requestId":"r1","message":{"id":"m1","model":"claude-sonnet-4-6","usage":{"input_tokens":1,"output_tokens":1},"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/home/u/proj/src/a.rs"}},{"type":"tool_use","name":"Read","input":{"file_path":"/home/u/proj/src/ignored.rs"}}]}}"#,
            "\n",
            r#"{"type":"assistant","timestamp":"2026-06-01T11:00:00Z","requestId":"r2","message":{"id":"m2","model":"claude-sonnet-4-6","usage":{"input_tokens":1,"output_tokens":1},"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/home/u/proj/b.md"}}]}}"#,
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edits.jsonl");
        std::fs::write(&path, text).unwrap();

        let parsed = parse_file(&path, &Models::default());

        assert_eq!(parsed.cwd.as_deref(), Some("/home/u/proj"));
        // Read tool calls are not edits.
        assert_eq!(parsed.edits.len(), 2);
        assert_eq!(parsed.edits[0].path, "/home/u/proj/src/a.rs");
        assert_eq!(parsed.edits[1].path, "/home/u/proj/b.md");
        assert!(parsed.edits[0].ts < parsed.edits[1].ts);
        assert_eq!(parsed.points.len(), 2);
    }

    // --- parse_file: cache_creation format variants ---

    #[test]
    fn parse_file_cache_creation_split_5m_1h() {
        // cache_creation object → 5m and 1h split used separately for quota weighting.
        let text = concat!(
            "{\"type\":\"user\",\"cwd\":\"/proj\",\"message\":{\"content\":\"go\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-01T10:00:00Z\",",
            "\"requestId\":\"r1\",\"message\":{\"id\":\"m1\",\"model\":\"claude-sonnet-4-6\",",
            "\"usage\":{\"input_tokens\":0,\"output_tokens\":0,",
            "\"cache_creation\":{\"ephemeral_5m_input_tokens\":500,\"ephemeral_1h_input_tokens\":1000}}}}",
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("split.jsonl");
        std::fs::write(&path, text).unwrap();

        let parsed = parse_file(&path, &Models::default());

        assert_eq!(parsed.points.len(), 1);
        assert_eq!(
            parsed.points[0].cache_write_tokens, 1500,
            "cache_write_tokens = 5m + 1h"
        );
        // sonnet quota: output=1.0, cw5m=0, cw1h=0.11 → 1000 * 0.11 = 110
        let expected = 1000.0 * 0.11;
        assert!(
            (parsed.points[0].weighted - expected).abs() < 1e-6,
            "got {}",
            parsed.points[0].weighted
        );
    }

    #[test]
    fn parse_file_cache_creation_flat_fallback() {
        // No cache_creation split → lump total treated as 5m (sonnet cw5m quota = 0).
        let text = concat!(
            "{\"type\":\"user\",\"cwd\":\"/proj\",\"message\":{\"content\":\"go\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-01T10:00:00Z\",",
            "\"requestId\":\"r2\",\"message\":{\"id\":\"m2\",\"model\":\"claude-sonnet-4-6\",",
            "\"usage\":{\"input_tokens\":0,\"output_tokens\":0,",
            "\"cache_creation_input_tokens\":1000}}}",
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flat.jsonl");
        std::fs::write(&path, text).unwrap();

        let parsed = parse_file(&path, &Models::default());

        assert_eq!(parsed.points.len(), 1);
        // flat → (5m=1000, 1h=0) → cache_write_tokens=1000
        assert_eq!(parsed.points[0].cache_write_tokens, 1000);
        // sonnet cw5m quota weight = 0; cw1h = 0 → weighted = 0
        assert!(
            parsed.points[0].weighted.abs() < 1e-6,
            "flat fallback has no quota contribution for sonnet"
        );
    }

    // --- session_edit_spans ---

    #[test]
    fn session_edit_spans_windows_points_and_edits() {
        let mut cache = ScanCache::default();
        cache.files.insert(
            PathBuf::from("/p/s1.jsonl"),
            CachedFile {
                points: vec![pt_for(1_000, 10.0), pt_for(3_000, 20.0), pt_for(9_000, 5.0)],
                cwd: Some("/home/u/proj".to_string()),
                edits: vec![
                    EditRec {
                        path: "a.rs".to_string(),
                        ts: 1_500,
                    },
                    EditRec {
                        path: "b.rs".to_string(),
                        ts: 8_000,
                    },
                ],
                ..Default::default()
            },
        );
        // No activity in window → no span.
        cache.files.insert(
            PathBuf::from("/p/s2.jsonl"),
            CachedFile {
                points: vec![pt_for(20_000, 99.0)],
                ..Default::default()
            },
        );

        let spans = cache.session_edit_spans(500, 5_000);
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.cwd.as_deref(), Some("/home/u/proj"));
        assert_eq!((s.first_ts, s.last_ts), (1_000, 3_000));
        assert!((s.weighted - 30.0).abs() < 1e-9);
        // Edits outside the window are dropped too.
        assert_eq!(s.edits.len(), 1);
        assert_eq!(s.edits[0].path, "a.rs");
    }
}
