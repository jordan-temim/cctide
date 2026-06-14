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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn memory_file_basic_structure() {
        let mf = MemoryFile {
            project: "test-project".to_string(),
            name: "notes.md".to_string(),
            path: "/home/user/.claude/projects/test-project/memory/notes.md".to_string(),
            content: "# Notes\nContent here".to_string(),
        };
        assert_eq!(mf.project, "test-project");
        assert_eq!(mf.name, "notes.md");
        assert!(!mf.content.is_empty());
    }

    #[test]
    fn memory_sort_puts_memory_md_first() {
        let mut files = [
            PathBuf::from("/memory/notes.md"),
            PathBuf::from("/memory/MEMORY.md"),
            PathBuf::from("/memory/archive.md"),
        ]
        .to_vec();

        files.sort_by_key(|p| {
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            (name != "MEMORY.md", name)
        });

        let sorted_names: Vec<String> = files
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect();

        assert_eq!(sorted_names[0], "MEMORY.md");
        assert_eq!(sorted_names[1], "archive.md");
        assert_eq!(sorted_names[2], "notes.md");
    }

    #[test]
    fn memory_sort_alphabetical_when_no_memory_md() {
        let mut files = [
            PathBuf::from("/memory/zebra.md"),
            PathBuf::from("/memory/apple.md"),
            PathBuf::from("/memory/banana.md"),
        ]
        .to_vec();

        files.sort_by_key(|p| {
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            (name != "MEMORY.md", name)
        });

        let sorted_names: Vec<String> = files
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect();

        assert_eq!(sorted_names, vec!["apple.md", "banana.md", "zebra.md"]);
    }

    // --- read_memory integration ---

    #[test]
    fn read_memory_empty_when_no_memory_dir() {
        // project_dir_for_cwd finds the project via the cache key, but the
        // memory/ subdir doesn't exist on disk → read_memory returns empty.
        let mut cache = crate::scan::ScanCache::default();
        let cwd = "/nonexistent-cctide-proj-xyzzy";
        let encoded = crate::scan::encode_cwd(cwd);
        cache.insert_test_transcript(
            PathBuf::from(format!("/tmp/fakeroots-cctide/{encoded}/session.jsonl")),
            100,
        );
        let files = read_memory(&cache, &[cwd.to_string()]);
        assert!(files.is_empty());
    }

    #[test]
    fn read_memory_reads_md_files_and_filters_non_md() {
        use std::fs;
        let cwd = "/cctide-memtest-readmd-proj";
        let encoded = crate::scan::encode_cwd(cwd);
        let base = tempfile::tempdir().unwrap();
        let project_dir = base.path().join(&encoded);
        let memory_dir = project_dir.join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("MEMORY.md"), "# Index").unwrap();
        fs::write(memory_dir.join("notes.md"), "# Notes").unwrap();
        fs::write(memory_dir.join("ignored.txt"), "not md").unwrap();

        let mut cache = crate::scan::ScanCache::default();
        cache.insert_test_transcript(project_dir.join("session.jsonl"), 100);

        let files = read_memory(&cache, &[cwd.to_string()]);
        // Only .md files returned; MEMORY.md first, then alphabetical.
        assert_eq!(files.len(), 2, "only .md files should be returned");
        assert_eq!(files[0].name, "MEMORY.md", "MEMORY.md must come first");
        assert_eq!(files[1].name, "notes.md");
        assert_eq!(files[0].project, encoded);
    }

    #[test]
    fn read_memory_deduplicates_same_project_cwd() {
        use std::fs;
        let cwd = "/cctide-memtest-dedup-proj";
        let encoded = crate::scan::encode_cwd(cwd);
        let base = tempfile::tempdir().unwrap();
        let project_dir = base.path().join(&encoded);
        let memory_dir = project_dir.join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("a.md"), "# A").unwrap();

        let mut cache = crate::scan::ScanCache::default();
        cache.insert_test_transcript(project_dir.join("session.jsonl"), 100);

        // Same cwd passed twice → BTreeSet deduplication → single project read.
        let files = read_memory(&cache, &[cwd.to_string(), cwd.to_string()]);
        assert_eq!(files.len(), 1);
    }
}
