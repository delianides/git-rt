//! Resolve the base branch for a `base..HEAD` commit-range walk.
//!
//! Uses a fixed fallback chain:
//! 1. The current branch's upstream (from `branch.<name>.remote` + `branch.<name>.merge`)
//! 2. `refs/remotes/origin/main`
//! 3. `refs/remotes/origin/master`
//! 4. `refs/heads/main`
//! 5. `refs/heads/master`
//! 6. `None`

/// Return a short-ref name (e.g. `"origin/main"` or `"main"`) to use as the
/// base for a commits-range walk, or `None` if nothing in the fallback chain
/// resolves.
pub fn resolve_base_branch(repo: &gix::Repository, current_branch: &str) -> Option<String> {
    if let Some(upstream) = upstream_for_branch(repo, current_branch) {
        return Some(upstream);
    }
    const CANDIDATES: &[&str] = &["origin/main", "origin/master", "main", "master"];
    for cand in CANDIDATES {
        if ref_exists(repo, cand) {
            return Some((*cand).to_string());
        }
    }
    None
}

fn upstream_for_branch(repo: &gix::Repository, branch: &str) -> Option<String> {
    let config = repo.config_snapshot();
    let remote_key = format!("branch.{branch}.remote");
    let merge_key = format!("branch.{branch}.merge");

    let remote_cow = config.string(remote_key.as_str())?;
    let merge_cow = config.string(merge_key.as_str())?;

    let remote = String::from_utf8_lossy(remote_cow.as_ref()).into_owned();
    let merge = String::from_utf8_lossy(merge_cow.as_ref()).into_owned();

    // `merge` looks like "refs/heads/main" — strip the prefix.
    let short = merge
        .strip_prefix("refs/heads/")
        .unwrap_or(&merge)
        .to_string();
    Some(format!("{remote}/{short}"))
}

fn ref_exists(repo: &gix::Repository, short_name: &str) -> bool {
    repo.find_reference(short_name).is_ok()
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

    fn init_repo_with_branch(dir: &Path, branch: &str) {
        run_git(dir, &["init", "-q", "-b", branch]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
        run_git(dir, &["config", "commit.gpgsign", "false"]);
        run_git(dir, &["config", "tag.gpgsign", "false"]);
        std::fs::write(dir.join("README.md"), "hello\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-q", "-m", "initial"]);
    }

    #[test]
    fn test_resolves_local_main_when_nothing_else() {
        let dir = tempdir().unwrap();
        init_repo_with_branch(dir.path(), "main");
        let repo = gix::open(dir.path()).unwrap();
        let base = resolve_base_branch(&repo, "main");
        assert_eq!(base.as_deref(), Some("main"));
    }

    #[test]
    fn test_resolves_local_master_when_only_master_exists() {
        let dir = tempdir().unwrap();
        init_repo_with_branch(dir.path(), "master");
        let repo = gix::open(dir.path()).unwrap();
        let base = resolve_base_branch(&repo, "master");
        assert_eq!(base.as_deref(), Some("master"));
    }

    #[test]
    fn test_returns_none_when_no_base_resolvable() {
        let dir = tempdir().unwrap();
        init_repo_with_branch(dir.path(), "develop");
        let repo = gix::open(dir.path()).unwrap();
        let base = resolve_base_branch(&repo, "develop");
        assert!(base.is_none());
    }
}
