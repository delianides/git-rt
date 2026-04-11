//! Commit range walking for the Commits tab. Types here are referenced by
//! `CommitsTabState` in `src/state/mod.rs`.

use gix::bstr::ByteSlice;

use crate::git::GitFailure;

/// A single commit entry for the Commits tab list.
#[derive(Debug, Clone)]
pub struct CommitEntry {
    pub sha_full: String,
    pub sha_short: String,
    pub title: String,
}

/// Result of walking a commit range.
#[derive(Debug, Clone)]
pub struct CommitsRangeResult {
    pub commits: Vec<CommitEntry>,
    pub truncated_count: usize,
    pub base_ref: String,
}

/// Hard ceiling on counting "extra" commits beyond `limit` to avoid
/// pathological walks on very long branches.
const COUNT_CEILING_EXTRA: usize = 500;

/// Walk commits reachable from HEAD but not reachable from the merge-base of
/// `(HEAD, base_ref)`. Returns up to `limit` entries plus a count of
/// additional commits (up to [`COUNT_CEILING_EXTRA`]).
///
/// Commits are returned newest-first (reverse chronological order).
pub fn commit_range(
    repo: &gix::Repository,
    base_ref: &str,
    limit: usize,
) -> Result<CommitsRangeResult, GitFailure> {
    // Resolve HEAD id
    let head_id = repo
        .head_id()
        .map_err(|e| GitFailure::Failed(format!("commit_range head_id: {e}")))?
        .detach();

    // Resolve base_ref id (peel to commit)
    let base_id = {
        let mut r = repo.find_reference(base_ref).map_err(|e| {
            GitFailure::Failed(format!("commit_range find_reference({base_ref}): {e}"))
        })?;
        r.peel_to_id()
            .map_err(|e| GitFailure::Failed(format!("commit_range peel_to_id({base_ref}): {e}")))?
            .detach()
    };

    // If HEAD == base, the branch is fully caught up: nothing to show.
    if head_id == base_id {
        return Ok(CommitsRangeResult {
            commits: Vec::new(),
            truncated_count: 0,
            base_ref: base_ref.to_string(),
        });
    }

    // Compute merge base of HEAD and base_ref.
    let merge_base_id = repo
        .merge_base(head_id, base_id)
        .map_err(|e| GitFailure::Failed(format!("commit_range merge_base: {e}")))?
        .detach();

    // Build a set of all commits reachable from merge_base (inclusive).
    // Commits in this set are excluded from the walk (they are in `base`
    // already). This is the fallback hashset approach — equivalent to
    // `git log merge_base..HEAD`.
    let excluded: std::collections::HashSet<gix::ObjectId> = {
        let walk = repo
            .rev_walk([merge_base_id])
            .all()
            .map_err(|e| GitFailure::Failed(format!("commit_range rev_walk(base): {e}")))?;
        let mut set = std::collections::HashSet::new();
        for info in walk {
            let info =
                info.map_err(|e| GitFailure::Failed(format!("commit_range walk(base) item: {e}")))?;
            set.insert(info.id);
        }
        set
    };

    // Walk HEAD ancestry, skipping commits in `excluded`.
    let walk = repo
        .rev_walk([head_id])
        .all()
        .map_err(|e| GitFailure::Failed(format!("commit_range rev_walk(head): {e}")))?;

    let mut commits: Vec<CommitEntry> = Vec::new();
    let mut truncated_count: usize = 0;
    let count_ceiling = limit + COUNT_CEILING_EXTRA;

    for info in walk {
        let info =
            info.map_err(|e| GitFailure::Failed(format!("commit_range walk(head) item: {e}")))?;

        if excluded.contains(&info.id) {
            continue;
        }

        if commits.len() < limit {
            // Parse the commit title.
            let obj = repo
                .find_object(info.id)
                .map_err(|e| GitFailure::Failed(format!("commit_range find_object: {e}")))?;
            let commit = obj
                .try_into_commit()
                .map_err(|e| GitFailure::Failed(format!("commit_range into_commit: {e}")))?;
            let raw_msg = commit.message_raw_sloppy();
            let title = raw_msg
                .to_str_lossy()
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();

            let sha_full = info.id.to_hex().to_string();
            let sha_short = sha_full.chars().take(7).collect();

            commits.push(CommitEntry {
                sha_full,
                sha_short,
                title,
            });
        } else {
            // Past the display limit — count extras up to the ceiling.
            truncated_count += 1;
            let total_seen = commits.len() + truncated_count;
            if total_seen >= count_ceiling {
                break;
            }
        }
    }

    Ok(CommitsRangeResult {
        commits,
        truncated_count,
        base_ref: base_ref.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;
    use tempfile::tempdir;

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git invocation failed");
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    fn init_repo(dir: &Path) {
        run_git(dir, &["init", "-q", "-b", "main"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
        run_git(dir, &["config", "commit.gpgsign", "false"]);
        run_git(dir, &["config", "tag.gpgsign", "false"]);
    }

    fn commit_file(dir: &Path, name: &str, content: &str, msg: &str) {
        std::fs::write(dir.join(name), content).unwrap();
        run_git(dir, &["add", name]);
        run_git(dir, &["commit", "-q", "-m", msg]);
    }

    #[test]
    fn test_empty_range_when_head_equals_base() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        commit_file(dir.path(), "a.txt", "a\n", "initial");
        let repo = gix::open(dir.path()).unwrap();
        let result = commit_range(&repo, "main", 100).unwrap();
        assert!(result.commits.is_empty());
        assert_eq!(result.truncated_count, 0);
        assert_eq!(result.base_ref, "main");
    }

    #[test]
    fn test_range_returns_commits_on_branch_newest_first() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        commit_file(dir.path(), "a.txt", "a\n", "initial");
        run_git(dir.path(), &["checkout", "-q", "-b", "feature"]);
        commit_file(dir.path(), "b.txt", "b\n", "second on feature");
        commit_file(dir.path(), "c.txt", "c\n", "third on feature");
        let repo = gix::open(dir.path()).unwrap();
        let result = commit_range(&repo, "main", 100).unwrap();
        assert_eq!(result.commits.len(), 2);
        assert_eq!(result.truncated_count, 0);
        assert_eq!(result.commits[0].title, "third on feature");
        assert_eq!(result.commits[1].title, "second on feature");
    }

    #[test]
    fn test_short_sha_is_seven_chars_and_full_is_forty() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        commit_file(dir.path(), "a.txt", "a\n", "initial");
        run_git(dir.path(), &["checkout", "-q", "-b", "feature"]);
        commit_file(dir.path(), "b.txt", "b\n", "second");
        let repo = gix::open(dir.path()).unwrap();
        let result = commit_range(&repo, "main", 100).unwrap();
        assert_eq!(result.commits[0].sha_short.len(), 7);
        assert!(result.commits[0].sha_full.len() >= 40);
        assert!(result.commits[0]
            .sha_full
            .starts_with(&result.commits[0].sha_short));
    }

    #[test]
    fn test_truncated_count_when_over_limit() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        commit_file(dir.path(), "base.txt", "base\n", "initial");
        run_git(dir.path(), &["checkout", "-q", "-b", "feature"]);
        for i in 0..5 {
            commit_file(
                dir.path(),
                &format!("f{i}.txt"),
                "x\n",
                &format!("feat {i}"),
            );
        }
        let repo = gix::open(dir.path()).unwrap();
        // Cap at 2 → 5 commits total on branch → 3 truncated
        let result = commit_range(&repo, "main", 2).unwrap();
        assert_eq!(result.commits.len(), 2);
        assert_eq!(result.truncated_count, 3);
    }
}
