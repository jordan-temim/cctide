//! Outcomes: classifies each session's quota spend by the fate of its edits.
//!
//! Model: a commit carries every uncommitted change to the files it touches,
//! so for each Edit/Write tool call (file + timestamp, from the transcript)
//! the **first commit touching that file afterwards** is the one that shipped
//! it. Commits slice time per file, so several sessions editing the same file
//! each resolve unambiguously — and two sessions sharing one commit are both
//! legitimately credited. A session is then classified by the majority fate
//! of its edits:
//!
//! - edits landed in commits reachable from main, not reverted → `shipped`
//! - edits in commits not (yet) on main                        → `pending`
//! - edits whose commits were later reverted                   → `reverted`
//! - no later commit touches those files                       → `abandoned`
//! - session outside any git repository                        → `non_repo`
//!
//! Sessions with a repo but no edits (chat/research) fall back to a coarse
//! temporal rule: commits in `[first_ts, last_ts + 1h]`.
//!
//! GIT IS READ-ONLY HERE — invariant of this module: the only subcommands
//! ever run are `rev-parse`, `symbolic-ref` and `log`. Nothing that touches
//! the working tree, refs or config.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use crate::scan::SessionSpan;

/// Margin added before the window when reading git logs, so commits/reverts
/// slightly older than the window are still seen.
const LOG_MARGIN_SECS: i64 = 7 * 86_400;
/// Temporal-fallback margin after a session's last activity.
const FALLBACK_MARGIN_SECS: i64 = 3_600;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Fate {
    Shipped,
    Pending,
    Reverted,
    Abandoned,
}

#[derive(Clone, serde::Serialize)]
pub struct OutcomeCategory {
    pub kind: String,
    pub weighted: f64,
    pub percent: f64,
    pub session_count: u32,
}

#[derive(Clone, serde::Serialize)]
pub struct OutcomeReport {
    pub window_start: i64,
    pub window_end: i64,
    pub categories: Vec<OutcomeCategory>,
}

/// Runs a read-only git subcommand in `dir`; `None` on any failure.
fn git(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Repo root for a working directory, if it is inside a git repo.
fn toplevel(cwd: &str) -> Option<String> {
    git(cwd, &["rev-parse", "--show-toplevel"]).map(|s| s.trim().to_string())
}

/// Name of the repo's main branch: the remote's default branch if a remote
/// exists (whatever it is called), else local `main`, else `master`.
fn main_ref(repo: &str) -> Option<String> {
    if let Some(head) = git(repo, &["symbolic-ref", "refs/remotes/origin/HEAD"]) {
        let head = head.trim();
        if let Some(short) = head.strip_prefix("refs/remotes/origin/") {
            // Prefer the local branch (it sees unpushed commits), fall back to
            // the remote-tracking ref.
            if git(repo, &["rev-parse", "--verify", "--quiet", short]).is_some() {
                return Some(short.to_string());
            }
            return Some(format!("origin/{short}"));
        }
    }
    for cand in ["main", "master"] {
        if git(repo, &["rev-parse", "--verify", "--quiet", cand]).is_some() {
            return Some(cand.to_string());
        }
    }
    None
}

struct CommitInfo {
    ts: i64,
    in_main: bool,
    reverted: bool,
}

/// Everything needed to resolve edit fates in one repo, read in two `git log`
/// passes (all branches + main only).
struct RepoLog {
    commits: Vec<CommitInfo>,
    /// file (repo-relative) → indices into `commits`, sorted by commit time.
    file_index: HashMap<String, Vec<usize>>,
}

/// Extracts the SHAs targeted by "This reverts commit <sha>" lines.
fn revert_targets(body: &str) -> Vec<String> {
    const MARKER: &str = "This reverts commit ";
    let mut targets = Vec::new();
    for (pos, _) in body.match_indices(MARKER) {
        let hex: String = body[pos + MARKER.len()..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if hex.len() >= 7 {
            targets.push(hex);
        }
    }
    targets
}

fn load_repo(repo: &str, since: i64) -> Option<RepoLog> {
    let main = main_ref(repo);
    let since_arg = format!(
        "--since={}",
        chrono::DateTime::from_timestamp(since, 0)?.to_rfc3339()
    );

    // SHAs reachable from main within the horizon.
    let main_shas: HashSet<String> = match &main {
        Some(m) => git(repo, &["log", m, since_arg.as_str(), "--format=%H"])
            .map(|out| out.lines().map(|l| l.trim().to_string()).collect())
            .unwrap_or_default(),
        None => HashSet::new(),
    };

    // All commits on all branches within the horizon, with their files.
    // Layout per commit: \x1e SHA \x1f ts \x1f body \x1d, then --name-only
    // appends the touched files.
    let out = git(
        repo,
        &[
            "log",
            "--all",
            since_arg.as_str(),
            "--name-only",
            "--format=%x1e%H%x1f%ct%x1f%B%x1d",
        ],
    )?;

    let mut shas: Vec<String> = Vec::new();
    let mut commits: Vec<CommitInfo> = Vec::new();
    let mut files_per_commit: Vec<Vec<String>> = Vec::new();
    let mut reverted_targets: Vec<String> = Vec::new();

    for chunk in out.split('\x1e').skip(1) {
        let mut parts = chunk.split('\x1f');
        let sha = parts.next().unwrap_or("").trim().to_string();
        let ts: i64 = parts.next().unwrap_or("").trim().parse().unwrap_or(0);
        let rest = parts.next().unwrap_or("");
        let (body, files_part) = rest.split_once('\x1d').unwrap_or((rest, ""));
        reverted_targets.extend(revert_targets(body));
        let files: Vec<String> = files_part
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        commits.push(CommitInfo {
            ts,
            in_main: main_shas.contains(&sha),
            reverted: false,
        });
        shas.push(sha);
        files_per_commit.push(files);
    }

    // Mark reverted commits by targeted SHA (prefix match — revert messages
    // may carry abbreviated hashes).
    for target in &reverted_targets {
        for (i, sha) in shas.iter().enumerate() {
            if sha.starts_with(target.as_str()) {
                commits[i].reverted = true;
            }
        }
    }

    // file → commit indices, ordered by commit time.
    let mut file_index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, files) in files_per_commit.iter().enumerate() {
        for f in files {
            file_index.entry(f.clone()).or_default().push(i);
        }
    }
    for idxs in file_index.values_mut() {
        idxs.sort_by_key(|&i| commits[i].ts);
    }

    Some(RepoLog {
        commits,
        file_index,
    })
}

impl RepoLog {
    /// Fate of one edit: the first commit touching the file after the edit.
    fn edit_fate(&self, rel_path: &str, ts: i64) -> Fate {
        if let Some(idxs) = self.file_index.get(rel_path) {
            for &i in idxs {
                let c = &self.commits[i];
                if c.ts >= ts {
                    return match (c.in_main, c.reverted) {
                        (true, true) => Fate::Reverted,
                        (true, false) => Fate::Shipped,
                        (false, _) => Fate::Pending,
                    };
                }
            }
        }
        Fate::Abandoned
    }

    /// Coarse fallback for sessions without edits: best status among commits
    /// made while the session was active (+1h).
    fn temporal_fate(&self, first_ts: i64, last_ts: i64) -> Fate {
        let window = first_ts..=(last_ts + FALLBACK_MARGIN_SECS);
        let mut best = Fate::Abandoned;
        for c in &self.commits {
            if !window.contains(&c.ts) {
                continue;
            }
            let fate = match (c.in_main, c.reverted) {
                (true, true) => Fate::Reverted,
                (true, false) => Fate::Shipped,
                (false, _) => Fate::Pending,
            };
            best = match (best, fate) {
                (_, Fate::Shipped) | (Fate::Shipped, _) => Fate::Shipped,
                (_, Fate::Pending) | (Fate::Pending, _) => Fate::Pending,
                (_, Fate::Reverted) | (Fate::Reverted, _) => Fate::Reverted,
                _ => Fate::Abandoned,
            };
        }
        best
    }
}

/// Majority fate of a session's edits inside its repo; ties break toward the
/// better outcome (Shipped > Pending > Reverted > Abandoned).
fn classify_span(span: &SessionSpan, repo_root: &str, log: &RepoLog) -> Fate {
    let root = Path::new(repo_root);
    let mut counts: HashMap<Fate, usize> = HashMap::new();
    for edit in &span.edits {
        // Only edits inside the repo can ever be committed; ignore the rest
        // (e.g. ~/.claude memory files).
        let Ok(rel) = Path::new(&edit.path).strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        *counts.entry(log.edit_fate(&rel, edit.ts)).or_insert(0) += 1;
    }
    if counts.is_empty() {
        return log.temporal_fate(span.first_ts, span.last_ts);
    }
    // Worst-to-best order: `max_by_key` keeps the LAST maximum, so a tie
    // resolves toward the better outcome (Shipped > Pending > Reverted >
    // Abandoned).
    [
        Fate::Abandoned,
        Fate::Reverted,
        Fate::Pending,
        Fate::Shipped,
    ]
    .into_iter()
    .max_by_key(|f| counts.get(f).copied().unwrap_or(0))
    .unwrap_or(Fate::Abandoned)
}

/// Builds the report: classify every span, aggregate weighted quota and
/// session counts per category. `percent` is each category's share of the
/// total weighted consumption across all spans.
pub fn outcome_report(spans: &[SessionSpan], window_start: i64, window_end: i64) -> OutcomeReport {
    // cwd → toplevel and toplevel → log, resolved once per repo.
    let mut toplevels: HashMap<String, Option<String>> = HashMap::new();
    let mut logs: HashMap<String, Option<RepoLog>> = HashMap::new();
    let since = window_start - LOG_MARGIN_SECS;

    // kind → (weighted, count), keyed by final category label.
    let mut agg: HashMap<&'static str, (f64, u32)> = HashMap::new();
    let total: f64 = spans.iter().map(|s| s.weighted).sum();

    for span in spans {
        let root = span.cwd.as_ref().and_then(|cwd| {
            toplevels
                .entry(cwd.clone())
                .or_insert_with(|| toplevel(cwd))
                .clone()
        });
        let kind = match root {
            None => "non_repo",
            Some(root) => {
                let log = logs
                    .entry(root.clone())
                    .or_insert_with(|| load_repo(&root, since));
                match log {
                    None => "non_repo",
                    Some(log) => match classify_span(span, &root, log) {
                        Fate::Shipped => "shipped",
                        Fate::Pending => "pending",
                        Fate::Reverted => "reverted",
                        Fate::Abandoned => "abandoned",
                    },
                }
            }
        };
        let e = agg.entry(kind).or_insert((0.0, 0));
        e.0 += span.weighted;
        e.1 += 1;
    }

    let categories = ["shipped", "pending", "reverted", "abandoned", "non_repo"]
        .into_iter()
        .map(|kind| {
            let (weighted, session_count) = agg.get(kind).copied().unwrap_or((0.0, 0));
            OutcomeCategory {
                kind: kind.to_string(),
                weighted,
                percent: if total > 0.0 {
                    weighted / total * 100.0
                } else {
                    0.0
                },
                session_count,
            }
        })
        .collect();

    OutcomeReport {
        window_start,
        window_end,
        categories,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{EditRec, SessionSpan};
    use std::path::PathBuf;

    fn have_git() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Runs a git command in `dir` with deterministic identity and dates.
    fn git_test(dir: &Path, ts: i64, args: &[&str]) {
        let date = format!("@{ts} +0000");
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .expect("git run");
        assert!(
            status.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// Fresh repo in a unique temp dir, default branch `branch`. Returns the
    /// canonicalized root (macOS tmp is a symlink; git reports the real path).
    fn init_repo(name: &str, branch: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cctide-outcome-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let status = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["-c", &format!("init.defaultBranch={branch}"), "init", "-q"])
            .output()
            .expect("git init");
        assert!(status.status.success());
        dir
    }

    fn commit_file(dir: &Path, file: &str, content: &str, msg: &str, ts: i64) {
        std::fs::write(dir.join(file), content).unwrap();
        git_test(dir, ts, &["add", "-A"]);
        git_test(dir, ts, &["commit", "-q", "-m", msg]);
    }

    fn span(dir: &Path, first_ts: i64, last_ts: i64, edits: Vec<EditRec>) -> SessionSpan {
        SessionSpan {
            cwd: Some(dir.to_string_lossy().into_owned()),
            first_ts,
            last_ts,
            weighted: 100.0,
            edits,
        }
    }

    fn edit(dir: &Path, file: &str, ts: i64) -> EditRec {
        EditRec {
            path: dir.join(file).to_string_lossy().into_owned(),
            ts,
        }
    }

    fn kind_of(report: &OutcomeReport, kind: &str) -> (f64, u32) {
        let c = report
            .categories
            .iter()
            .find(|c| c.kind == kind)
            .expect("category");
        (c.weighted, c.session_count)
    }

    fn base_ts() -> i64 {
        chrono::Utc::now().timestamp() - 86_400
    }

    #[test]
    fn shipped_when_commit_follows_edit() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("shipped", "main");
        commit_file(&repo, "a.rs", "v1", "init", t);
        // Session edits a.rs, commit lands 7h later (evening commit for a
        // morning session): still shipped — no time margin involved.
        let s = span(&repo, t + 100, t + 200, vec![edit(&repo, "a.rs", t + 150)]);
        commit_file(&repo, "a.rs", "v2", "ship it", t + 150 + 7 * 3600);
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "shipped"), (100.0, 1));
    }

    #[test]
    fn pending_when_commit_is_on_unmerged_branch() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("pending", "main");
        commit_file(&repo, "a.rs", "v1", "init", t);
        git_test(&repo, t, &["checkout", "-q", "-b", "feat"]);
        let s = span(&repo, t + 100, t + 200, vec![edit(&repo, "a.rs", t + 150)]);
        commit_file(&repo, "a.rs", "v2", "wip", t + 300);
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "pending"), (100.0, 1));
    }

    #[test]
    fn reverted_when_commit_was_reverted() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("reverted", "main");
        commit_file(&repo, "a.rs", "v1", "init", t);
        let s = span(&repo, t + 100, t + 200, vec![edit(&repo, "a.rs", t + 150)]);
        commit_file(&repo, "a.rs", "v2", "bad idea", t + 300);
        git_test(&repo, t + 400, &["revert", "--no-edit", "HEAD"]);
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "reverted"), (100.0, 1));
    }

    #[test]
    fn abandoned_when_no_later_commit_touches_file() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("abandoned", "main");
        commit_file(&repo, "a.rs", "v1", "init", t);
        let s = span(&repo, t + 100, t + 200, vec![edit(&repo, "a.rs", t + 150)]);
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "abandoned"), (100.0, 1));
    }

    #[test]
    fn two_sessions_same_file_commits_in_between() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("two-sessions", "main");
        commit_file(&repo, "a.rs", "v1", "init", t);
        // Session A edits, commit lands → shipped.
        let a = span(&repo, t + 100, t + 200, vec![edit(&repo, "a.rs", t + 150)]);
        commit_file(&repo, "a.rs", "v2", "ship A", t + 300);
        // Session B edits the same file after that commit, never committed.
        let b = span(&repo, t + 400, t + 500, vec![edit(&repo, "a.rs", t + 450)]);
        let report = outcome_report(&[a, b], t, t + 86_400);
        assert_eq!(kind_of(&report, "shipped"), (100.0, 1));
        assert_eq!(kind_of(&report, "abandoned"), (100.0, 1));
    }

    #[test]
    fn master_repo_ships_too() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("master", "master");
        commit_file(&repo, "a.rs", "v1", "init", t);
        let s = span(&repo, t + 100, t + 200, vec![edit(&repo, "a.rs", t + 150)]);
        commit_file(&repo, "a.rs", "v2", "ship", t + 300);
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "shipped"), (100.0, 1));
    }

    #[test]
    fn non_repo_and_missing_cwd() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let plain = std::env::temp_dir().join("cctide-outcome-plain");
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        let s1 = span(&plain, t, t + 100, vec![]);
        let s2 = SessionSpan {
            cwd: Some("/definitely/not/a/dir".to_string()),
            first_ts: t,
            last_ts: t + 100,
            weighted: 50.0,
            edits: vec![],
        };
        let s3 = SessionSpan {
            cwd: None,
            first_ts: t,
            last_ts: t + 100,
            weighted: 25.0,
            edits: vec![],
        };
        let report = outcome_report(&[s1, s2, s3], t, t + 86_400);
        assert_eq!(kind_of(&report, "non_repo"), (175.0, 3));
    }

    #[test]
    fn no_edit_session_falls_back_to_temporal() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("temporal", "main");
        commit_file(&repo, "a.rs", "v1", "init", t - 10_000);
        // No edits, but a main commit lands while the session is active.
        let s = span(&repo, t + 100, t + 500, vec![]);
        commit_file(&repo, "a.rs", "v2", "manual ship", t + 300);
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "shipped"), (100.0, 1));
    }

    #[test]
    fn edits_outside_repo_are_ignored() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("outside", "main");
        commit_file(&repo, "a.rs", "v1", "init", t);
        // Only edit is a memory file outside the repo → temporal fallback,
        // no commit during the session → abandoned.
        let s = span(
            &repo,
            t + 100,
            t + 200,
            vec![EditRec {
                path: "/elsewhere/memory.md".to_string(),
                ts: t + 150,
            }],
        );
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "abandoned"), (100.0, 1));
    }

    #[test]
    fn percentages_share_total_weight() {
        let t = 1_000_000;
        let s1 = SessionSpan {
            cwd: None,
            first_ts: t,
            last_ts: t + 10,
            weighted: 75.0,
            edits: vec![],
        };
        let s2 = SessionSpan {
            cwd: None,
            first_ts: t,
            last_ts: t + 10,
            weighted: 25.0,
            edits: vec![],
        };
        let report = outcome_report(&[s1, s2], t, t + 100);
        let nr = report
            .categories
            .iter()
            .find(|c| c.kind == "non_repo")
            .unwrap();
        assert!((nr.percent - 100.0).abs() < 1e-9);
        assert_eq!(report.categories.len(), 5);
        // Empty input → all-zero percentages, no NaN.
        let empty = outcome_report(&[], t, t + 100);
        assert!(empty.categories.iter().all(|c| c.percent == 0.0));
    }

    #[test]
    fn revert_targets_parses_sha_lines() {
        let body =
            "Revert \"feat\"\n\nThis reverts commit 0123456789abcdef0123456789abcdef01234567.\n";
        assert_eq!(
            revert_targets(body),
            vec!["0123456789abcdef0123456789abcdef01234567".to_string()]
        );
        // Word "revert" alone never matches.
        assert!(revert_targets("revert the thing").is_empty());
        // Too-short hex is rejected.
        assert!(revert_targets("This reverts commit 0123.").is_empty());
    }

    #[test]
    fn classify_majority_and_tie_break() {
        // No git needed: hand-built RepoLog.
        // commit 0: in main, clean; commit 1: never reached main.
        let log = RepoLog {
            commits: vec![
                CommitInfo {
                    ts: 200,
                    in_main: true,
                    reverted: false,
                },
                CommitInfo {
                    ts: 200,
                    in_main: false,
                    reverted: false,
                },
            ],
            file_index: HashMap::from([
                ("shipped.rs".to_string(), vec![0]),
                ("pending.rs".to_string(), vec![1]),
            ]),
        };
        let mk = |edits: Vec<EditRec>| SessionSpan {
            cwd: Some("/repo".to_string()),
            first_ts: 100,
            last_ts: 150,
            weighted: 1.0,
            edits,
        };
        let e = |file: &str| EditRec {
            path: format!("/repo/{file}"),
            ts: 100,
        };
        // Majority wins: 2 pending vs 1 shipped → Pending.
        let span = mk(vec![e("pending.rs"), e("pending.rs"), e("shipped.rs")]);
        assert_eq!(classify_span(&span, "/repo", &log), Fate::Pending);
        // Tie breaks toward the better outcome: 1 shipped vs 1 abandoned → Shipped.
        let span = mk(vec![e("shipped.rs"), e("never-committed.rs")]);
        assert_eq!(classify_span(&span, "/repo", &log), Fate::Shipped);
    }

    #[test]
    fn merged_branch_flips_pending_to_shipped() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("merge", "main");
        commit_file(&repo, "a.rs", "v1", "init", t);
        git_test(&repo, t, &["checkout", "-q", "-b", "feat"]);
        let s = span(&repo, t + 100, t + 200, vec![edit(&repo, "a.rs", t + 150)]);
        commit_file(&repo, "a.rs", "v2", "feat work", t + 300);
        // Before the merge the session is pending…
        let report = outcome_report(&[s.clone()], t, t + 86_400);
        assert_eq!(kind_of(&report, "pending"), (100.0, 1));
        // …and shipped once the branch lands on main (SHA reachability).
        git_test(&repo, t + 400, &["checkout", "-q", "main"]);
        git_test(
            &repo,
            t + 400,
            &["merge", "-q", "--no-ff", "--no-edit", "feat"],
        );
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "shipped"), (100.0, 1));
    }

    #[test]
    fn remote_default_branch_drives_main_ref() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        // Upstream repo whose default branch is "trunk", cloned locally:
        // the clone's origin/HEAD points at trunk, whatever it is called.
        let upstream = init_repo("trunk-upstream", "trunk");
        commit_file(&upstream, "a.rs", "v1", "init", t);
        let clone_dir = std::env::temp_dir().join("cctide-outcome-trunk-clone");
        let _ = std::fs::remove_dir_all(&clone_dir);
        let out = Command::new("git")
            .args(["clone", "-q"])
            .arg(format!("file://{}", upstream.display()))
            .arg(&clone_dir)
            .output()
            .expect("git clone");
        assert!(out.status.success());
        let clone_dir = clone_dir.canonicalize().unwrap();
        assert_eq!(
            main_ref(clone_dir.to_str().unwrap()).as_deref(),
            Some("trunk")
        );
        // Local commits on trunk (not pushed) still count as shipped.
        let s = span(
            &clone_dir,
            t + 100,
            t + 200,
            vec![edit(&clone_dir, "a.rs", t + 150)],
        );
        commit_file(&clone_dir, "a.rs", "v2", "ship", t + 300);
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "shipped"), (100.0, 1));
    }

    #[test]
    fn cwd_in_subdirectory_resolves_toplevel() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("subdir", "main");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        commit_file(&repo, "sub/a.rs", "v1", "init", t);
        // Session ran in repo/sub; edit paths are absolute as Claude writes them.
        let mut s = span(
            &repo,
            t + 100,
            t + 200,
            vec![edit(&repo, "sub/a.rs", t + 150)],
        );
        s.cwd = Some(repo.join("sub").to_string_lossy().into_owned());
        commit_file(&repo, "sub/a.rs", "v2", "ship", t + 300);
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "shipped"), (100.0, 1));
    }

    #[test]
    fn revert_with_abbreviated_sha_is_detected() {
        if !have_git() {
            return;
        }
        let t = base_ts();
        let repo = init_repo("short-revert", "main");
        commit_file(&repo, "a.rs", "v1", "init", t);
        let s = span(&repo, t + 100, t + 200, vec![edit(&repo, "a.rs", t + 150)]);
        commit_file(&repo, "a.rs", "v2", "bad", t + 300);
        let short = git(repo.to_str().unwrap(), &["rev-parse", "--short", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let msg = format!("Revert \"bad\"\n\nThis reverts commit {short}.");
        git_test(
            &repo,
            t + 400,
            &["commit", "-q", "--allow-empty", "-m", &msg],
        );
        let report = outcome_report(&[s], t, t + 86_400);
        assert_eq!(kind_of(&report, "reverted"), (100.0, 1));
    }
}
