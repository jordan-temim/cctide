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

#[derive(Debug, Clone, Default)]
struct CachedFile {
    mtime: i64,
    size: u64,
    /// Points from this file, deduplicated within the file.
    points: Vec<Point>,
    last_ctx: Option<LastCtx>,
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

/// (Re)reads a JSONL file and returns its points (deduplicated within the file)
/// and the last known context state.
fn parse_file(path: &Path, pricing: &Models) -> (Vec<Point>, Option<LastCtx>) {
    let mut points: Vec<Point> = Vec::new();
    let mut last_ctx: Option<LastCtx> = None;
    let mut seen_keys: HashSet<u64> = HashSet::new();

    let Ok(text) = std::fs::read_to_string(path) else {
        return (points, last_ctx);
    };
    let path_str = path.to_string_lossy();

    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || !line.contains("\"usage\"") {
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

        last_ctx = Some(LastCtx {
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

        // Quota weighting excludes cache reads — see models.rs for rationale.
        let weighted = pricing.quota_units(&model, input, output, cache_5m, cache_1h);
        points.push(Point {
            ts,
            weighted,
            model,
            key,
        });
    }

    (points, last_ctx)
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
                let (points, last_ctx) = parse_file(path, pricing);
                self.files.insert(
                    pb,
                    CachedFile {
                        mtime,
                        size,
                        points,
                        last_ctx,
                    },
                );
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

    /// Resolves a session's context: by session id, else the most recently
    /// modified transcript in the session's `cwd` project folder (handles
    /// resumed sessions whose live transcript keeps a different id).
    pub fn last_ctx_for_session_or_cwd(&self, session_id: &str, cwd: &str) -> Option<LastCtx> {
        if let Some(ctx) = self
            .jsonl_for_session(session_id)
            .and_then(|p| self.last_ctx_for(&p))
        {
            return Some(ctx);
        }
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
            .and_then(|(_, f)| f.last_ctx.clone())
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
            model: "claude-sonnet-4-6".to_string(),
            key: shared_key,
        };
        let p2 = Point {
            ts: 2000,
            weighted: 50.0,
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
            },
        );
        cache.files.insert(
            PathBuf::from("file2"),
            CachedFile {
                mtime: 0,
                size: 0,
                points: vec![p2],
                last_ctx: None,
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
                        model: "sonnet".to_string(),
                        key: 1,
                    },
                    Point {
                        ts: 1100,
                        weighted: 20.0,
                        model: "sonnet".to_string(),
                        key: 2,
                    },
                ],
                last_ctx: None,
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
                        model: "sonnet".to_string(),
                        key: 2,
                    },
                    Point {
                        ts: 2100,
                        weighted: 30.0,
                        model: "sonnet".to_string(),
                        key: 3,
                    },
                ],
                last_ctx: None,
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

    // --- last_ctx_for_session_or_cwd ---

    #[test]
    fn last_ctx_fallback_to_cwd_when_session_not_found() {
        let mut cache = ScanCache::default();
        let session_id = "unknown-sess";
        let cwd = "/home/user/proj";
        let encoded_cwd = encode_cwd(cwd);

        // Set up a file in the project folder for the cwd.
        let project_path = PathBuf::from(format!("/root/.claude/projects/{encoded_cwd}/session1.jsonl"));
        cache.files.insert(
            project_path,
            CachedFile {
                mtime: 123,
                size: 500,
                points: vec![],
                last_ctx: Some(LastCtx {
                    model: "opus".to_string(),
                    context_tokens: 50_000,
                }),
            },
        );

        let ctx = cache.last_ctx_for_session_or_cwd(session_id, cwd);
        assert!(ctx.is_some());
        let ctx_unwrapped = ctx.unwrap();
        assert_eq!(ctx_unwrapped.model, "opus");
        assert_eq!(ctx_unwrapped.context_tokens, 50_000);
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
            },
        );
        cache.files.insert(
            PathBuf::from("/root/.claude/projects/proj2/other.jsonl"),
            CachedFile {
                mtime: 100,
                size: 500,
                points: vec![],
                last_ctx: None,
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
            },
        );

        let found = cache.project_dir_for_cwd(cwd);
        assert!(found.is_some());
        let found_path = found.unwrap();
        assert!(found_path.to_string_lossy().contains(&encoded));
    }
}
