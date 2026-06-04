//! Read-only access to the memory files of the active sessions' projects.
//!
//! A project's memory lives in `<project-dir>/memory/*.md`, where the project
//! dir is the one holding the session's JSONL transcript.

use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::scan::ScanCache;

#[derive(Debug, Serialize)]
pub struct MemoryFile {
    /// Project folder name (encoded), used to group the display.
    pub project: String,
    pub name: String,
    pub path: String,
    pub content: String,
}

/// Reads the `.md` files in the `memory/` folders of the given working dirs.
pub fn read_memory(cache: &ScanCache, cwds: &[String]) -> Vec<MemoryFile> {
    // Unique `memory/` folders to walk.
    let mut memory_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for cwd in cwds {
        if let Some(project_dir) = cache.project_dir_for_cwd(cwd) {
            memory_dirs.insert(project_dir.join("memory"));
        }
    }

    let mut out = Vec::new();
    for dir in memory_dirs {
        if !dir.is_dir() {
            continue;
        }
        let project = dir
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        // MEMORY.md first, then alphabetical order.
        files.sort_by_key(|p| {
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            (name != "MEMORY.md", name)
        });

        for path in files {
            // Resolve the real path and verify it stays inside the memory dir.
            // This blocks symlinks that point outside `~/.claude/projects/*/memory/`.
            let Ok(canon) = path.canonicalize() else {
                continue;
            };
            let Ok(canon_dir) = dir.canonicalize() else {
                continue;
            };
            if !canon.starts_with(&canon_dir) {
                continue;
            }
            let content = std::fs::read_to_string(&canon).unwrap_or_default();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            // Return the canonicalized path so openPath on the frontend always
            // resolves to a file that is confirmed to be within the memory dir.
            out.push(MemoryFile {
                project: project.clone(),
                name,
                path: canon.to_string_lossy().to_string(),
                content,
            });
        }
    }

    out
}
