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

/// Deletes one project memory file. The path must resolve to a real `.md`
/// file inside `~/.claude/projects/<project>/memory/`. When an index
/// (`MEMORY.md`) sits next to it, its line referencing the file is dropped
/// (best effort).
pub fn delete_memory_file(path: &str) -> Result<(), String> {
    let canon = std::path::Path::new(path)
        .canonicalize()
        .map_err(|_| "file not found")?;
    if canon.extension().and_then(|x| x.to_str()) != Some("md") {
        return Err("not a memory file".into());
    }
    let root = crate::scan::projects_dir()
        .ok_or("no home directory")?
        .canonicalize()
        .map_err(|_| "projects dir not found")?;
    let parent = canon.parent().ok_or("invalid path")?;
    if !canon.starts_with(&root) || parent.file_name().and_then(|n| n.to_str()) != Some("memory") {
        return Err("path is outside the memory directories".into());
    }

    let name = canon
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("invalid path")?
        .to_string();
    std::fs::remove_file(&canon).map_err(|e| format!("delete failed: {e}"))?;

    // Drop the deleted file's line from the MEMORY.md index, if present.
    if name != "MEMORY.md" {
        let index = parent.join("MEMORY.md");
        if let Ok(text) = std::fs::read_to_string(&index) {
            let _ = std::fs::write(&index, drop_index_lines(&text, &name));
        }
    }
    Ok(())
}

/// Removes from a MEMORY.md index the lines whose markdown link targets the
/// deleted file — i.e. lines containing `(<name>)`, the link-target part of
/// `- [Title](<name>) — hook`.
fn drop_index_lines(index: &str, name: &str) -> String {
    let target = format!("({name})");
    let kept: Vec<&str> = index.lines().filter(|l| !l.contains(&target)).collect();
    kept.join("\n") + "\n"
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

    // --- drop_index_lines ---

    #[test]
    fn drop_index_lines_removes_only_the_target_link() {
        let index = "# Memory Index\n\n\
            - [Foo](foo.md) — about foo\n\
            - [Bar](bar.md) — about bar\n";
        let out = drop_index_lines(index, "foo.md");
        assert!(!out.contains("foo.md"));
        assert!(out.contains("(bar.md)"));
        assert!(out.contains("# Memory Index"));
    }

    #[test]
    fn drop_index_lines_does_not_match_suffixed_names() {
        // `(foo.md)` must not match `(bar-foo.md)` — the opening paren anchors
        // the link target's start.
        let index = "- [Bar foo](bar-foo.md) — composite name\n";
        let out = drop_index_lines(index, "foo.md");
        assert!(out.contains("(bar-foo.md)"));
    }

    #[test]
    fn drop_index_lines_no_match_keeps_text_intact() {
        let index = "- [Foo](foo.md) — hook\n";
        assert_eq!(drop_index_lines(index, "missing.md"), index);
    }

    #[test]
    fn drop_index_lines_mention_without_link_is_kept() {
        // A plain-text mention of the name is not a link target → kept.
        let index = "- [Other](other.md) — see also foo.md\n";
        let out = drop_index_lines(index, "foo.md");
        assert!(out.contains("(other.md)"));
    }
}
