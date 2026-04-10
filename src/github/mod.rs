pub mod types;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};

use crate::state::{
    CheckInfo, CheckStatus, ChecksInfo, MergeableStatus, PrDisplayInfo, PrStatus, ReviewInfo,
    ReviewState,
};
use types::{GitHubCheckRuns, GitHubComment, GitHubPr, GitHubReview};

/// Events sent from the GitHub polling thread to the main event loop
#[derive(Debug)]
pub enum GitHubEvent {
    /// PR data was fetched and converted to display info
    PrUpdate(PrDisplayInfo),
    /// No PR found for the current branch
    NoPr,
    /// An error occurred during fetch
    Error(String),
}

/// Resolve a GitHub auth token. Tries `gh auth token` first, then
/// falls back to the `GIT_RT_GITHUB_TOKEN` environment variable.
pub fn resolve_auth_token() -> Option<String> {
    // Try `gh auth token` first
    if let Ok(output) = Command::new("gh").args(["auth", "token"]).output() {
        if output.status.success() {
            let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !token.is_empty() {
                return Some(token);
            }
        }
    }

    // Fall back to environment variable
    std::env::var("GIT_RT_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
}

/// Parse owner and repo from a GitHub remote URL.
/// Supports SSH (`git@github.com:owner/repo.git`) and HTTPS
/// (`https://github.com/owner/repo.git`) formats, with or without `.git` suffix.
/// Returns `None` for non-GitHub URLs.
pub fn parse_remote_url(url: &str) -> Option<(String, String)> {
    // SSH format: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let rest = rest.strip_suffix(".git").unwrap_or(rest);
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Some((parts[0].to_string(), parts[1].to_string()));
        }
        return None;
    }

    // HTTPS format: https://github.com/owner/repo.git
    if url.starts_with("https://github.com/") || url.starts_with("http://github.com/") {
        let path = url
            .trim_start_matches("https://github.com/")
            .trim_start_matches("http://github.com/");
        let path = path.strip_suffix(".git").unwrap_or(path);
        let parts: Vec<&str> = path.splitn(2, '/').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Some((parts[0].to_string(), parts[1].to_string()));
        }
        return None;
    }

    None
}

/// Get the remote URL for "origin" by running `git remote get-url origin`.
pub fn get_remote_url(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .ok()?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !url.is_empty() {
            return Some(url);
        }
    }
    None
}

/// Build a ureq agent and set common headers for a request.
fn github_get(
    agent: &ureq::Agent,
    url: &str,
    token: &str,
) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
    agent
        .get(url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "git-rt")
}

/// Fetch PR data from the GitHub API and convert it to `PrDisplayInfo`.
/// Returns `Ok(None)` if no open PR exists for the given branch.
pub fn fetch_pr_data(
    owner: &str,
    repo: &str,
    branch: &str,
    token: &str,
) -> Result<Option<PrDisplayInfo>> {
    let agent = ureq::Agent::new_with_defaults();

    // Find open PR for this branch
    let search_url = format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls?head={owner}:{branch}&state=open"
    );
    let prs: Vec<GitHubPr> = github_get(&agent, &search_url, token)
        .call()
        .context("Failed to search for PRs")?
        .body_mut()
        .read_json()
        .context("Failed to parse PR search response")?;

    let number = match prs.first() {
        Some(pr) => pr.number,
        None => return Ok(None),
    };

    // Fetch the individual PR endpoint — the list endpoint returns
    // mergeable/mergeable_state as null since GitHub computes them async.
    let pr_url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let pr: GitHubPr = github_get(&agent, &pr_url, token)
        .call()
        .context("Failed to fetch PR details")?
        .body_mut()
        .read_json()
        .context("Failed to parse PR details response")?;

    // Fetch reviews
    let reviews_url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews");
    let reviews: Vec<GitHubReview> = github_get(&agent, &reviews_url, token)
        .call()
        .context("Failed to fetch reviews")?
        .body_mut()
        .read_json()
        .context("Failed to parse reviews response")?;

    // Fetch check runs
    let checks_url =
        format!("https://api.github.com/repos/{owner}/{repo}/commits/{branch}/check-runs");
    let check_runs: GitHubCheckRuns = github_get(&agent, &checks_url, token)
        .call()
        .context("Failed to fetch check runs")?
        .body_mut()
        .read_json()
        .context("Failed to parse check runs response")?;

    // Fetch issue comments (for accurate count beyond PR description comments)
    let comments_url =
        format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}/comments");
    let comments: Vec<GitHubComment> = github_get(&agent, &comments_url, token)
        .call()
        .context("Failed to fetch comments")?
        .body_mut()
        .read_json()
        .context("Failed to parse comments response")?;

    // Convert PR state
    let state = if pr.draft.unwrap_or(false) {
        PrStatus::Draft
    } else {
        match pr.state.as_str() {
            "open" => PrStatus::Open,
            "closed" => PrStatus::Closed,
            _ => PrStatus::Open,
        }
    };

    // Deduplicate reviews: keep latest per reviewer
    let mut latest_reviews: HashMap<String, ReviewInfo> = HashMap::new();
    for review in reviews {
        let review_state = match review.state.as_str() {
            "APPROVED" => ReviewState::Approved,
            "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
            "PENDING" => ReviewState::Pending,
            "COMMENTED" => ReviewState::Commented,
            "DISMISSED" => ReviewState::Dismissed,
            _ => ReviewState::Pending,
        };
        latest_reviews.insert(
            review.user.login.clone(),
            ReviewInfo {
                reviewer: review.user.login,
                state: review_state,
            },
        );
    }

    // Convert check runs
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut pending = 0usize;
    let mut skipped = 0usize;
    let checks: Vec<CheckInfo> = check_runs
        .check_runs
        .iter()
        .map(|cr| {
            let status = if cr.status == "completed" {
                match cr.conclusion.as_deref() {
                    Some("success") => {
                        passed += 1;
                        CheckStatus::Passed
                    }
                    Some("failure") | Some("timed_out") | Some("cancelled") => {
                        failed += 1;
                        CheckStatus::Failed
                    }
                    Some("skipped") => {
                        skipped += 1;
                        CheckStatus::Skipped
                    }
                    _ => {
                        pending += 1;
                        CheckStatus::Pending
                    }
                }
            } else if cr.status == "in_progress" {
                pending += 1;
                CheckStatus::Running
            } else {
                pending += 1;
                CheckStatus::Pending
            };
            CheckInfo {
                name: cr.name.clone(),
                status,
            }
        })
        .collect();

    let total = checks.len();

    // Convert mergeable status
    let mergeable = match pr.mergeable_state.as_deref() {
        Some("clean") => MergeableStatus::Clean,
        Some("unstable") => MergeableStatus::Clean, // checks failing but no conflicts
        Some("dirty") => MergeableStatus::Conflicts,
        Some("behind") => MergeableStatus::Behind,
        _ => match pr.mergeable {
            Some(true) => MergeableStatus::Clean,
            Some(false) => MergeableStatus::Conflicts,
            None => MergeableStatus::Unknown,
        },
    };

    let labels = pr
        .labels
        .unwrap_or_default()
        .into_iter()
        .map(|l| l.name)
        .collect();

    let assignees = pr
        .assignees
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.login)
        .collect();

    Ok(Some(PrDisplayInfo {
        number,
        title: pr.title,
        state,
        reviews: latest_reviews.into_values().collect(),
        checks: ChecksInfo {
            total,
            passed,
            failed,
            pending,
            skipped,
            checks,
        },
        comment_count: comments.len() as u64,
        mergeable,
        labels,
        assignees,
    }))
}

/// Manages adaptive polling intervals for GitHub API requests.
/// Starts at an idle interval and switches to a faster active interval
/// when changes are detected, backing off after consecutive unchanged responses.
struct PollManager {
    idle_interval: Duration,
    active_interval: Duration,
    current_interval: Duration,
    unchanged_count: u32,
    backoff_threshold: u32,
    last_hash: u64,
}

impl PollManager {
    fn new() -> Self {
        let idle = Duration::from_secs(30);
        Self {
            idle_interval: idle,
            active_interval: Duration::from_secs(10),
            current_interval: idle,
            unchanged_count: 0,
            backoff_threshold: 3,
            last_hash: 0,
        }
    }

    /// Report new PR data. Returns the interval to use for the next poll.
    fn report(&mut self, info: &PrDisplayInfo) -> Duration {
        let hash = simple_hash(info);
        if hash == self.last_hash {
            self.unchanged_count += 1;
            if self.unchanged_count >= self.backoff_threshold {
                self.current_interval = self.idle_interval;
            }
        } else {
            self.last_hash = hash;
            self.unchanged_count = 0;
            self.current_interval = self.active_interval;
        }
        self.current_interval
    }

    /// Switch to active polling (e.g., after a filesystem change).
    #[allow(dead_code)]
    fn go_active(&mut self) {
        self.unchanged_count = 0;
        self.current_interval = self.active_interval;
    }

    /// The current polling interval.
    fn interval(&self) -> Duration {
        self.current_interval
    }
}

/// Simple hash of PrDisplayInfo for change detection.
fn simple_hash(info: &PrDisplayInfo) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    info.number.hash(&mut hasher);
    info.title.hash(&mut hasher);
    info.comment_count.hash(&mut hasher);
    info.checks.total.hash(&mut hasher);
    info.checks.passed.hash(&mut hasher);
    info.checks.failed.hash(&mut hasher);
    info.checks.pending.hash(&mut hasher);
    info.labels.hash(&mut hasher);
    info.assignees.hash(&mut hasher);
    hasher.finish()
}

/// Start a background thread that polls the GitHub API for PR data.
/// Returns a receiver that yields `GitHubEvent` messages.
pub fn start_polling(repo_path: &Path, branch: &str, token: &str) -> Receiver<GitHubEvent> {
    let (tx, rx): (Sender<GitHubEvent>, Receiver<GitHubEvent>) = bounded(8);

    let remote_url = get_remote_url(repo_path);
    let branch = branch.to_string();
    let token = token.to_string();

    std::thread::Builder::new()
        .name("github-poller".into())
        .spawn(move || {
            let (owner, repo) = match remote_url.as_deref().and_then(parse_remote_url) {
                Some(pair) => pair,
                None => {
                    let _ = tx.send(GitHubEvent::Error(
                        "Could not parse GitHub owner/repo from remote URL".into(),
                    ));
                    return;
                }
            };

            let mut poll_manager = PollManager::new();

            loop {
                let event = match fetch_pr_data(&owner, &repo, &branch, &token) {
                    Ok(Some(info)) => {
                        poll_manager.report(&info);
                        GitHubEvent::PrUpdate(info)
                    }
                    Ok(None) => GitHubEvent::NoPr,
                    Err(e) => GitHubEvent::Error(format!("{e:#}")),
                };

                if tx.send(event).is_err() {
                    // Receiver dropped, stop polling
                    break;
                }

                std::thread::sleep(poll_manager.interval());
            }
        })
        .expect("Failed to spawn GitHub poller thread");

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_remote_ssh() {
        let result = parse_remote_url("git@github.com:delianides/git-rt.git");
        assert_eq!(
            result,
            Some(("delianides".to_string(), "git-rt".to_string()))
        );
    }

    #[test]
    fn test_parse_remote_https() {
        let result = parse_remote_url("https://github.com/delianides/git-rt.git");
        assert_eq!(
            result,
            Some(("delianides".to_string(), "git-rt".to_string()))
        );
    }

    #[test]
    fn test_parse_remote_https_no_suffix() {
        let result = parse_remote_url("https://github.com/delianides/git-rt");
        assert_eq!(
            result,
            Some(("delianides".to_string(), "git-rt".to_string()))
        );
    }

    #[test]
    fn test_parse_remote_invalid() {
        assert!(parse_remote_url("https://gitlab.com/user/repo.git").is_none());
    }

    #[test]
    fn test_parse_remote_empty() {
        assert!(parse_remote_url("").is_none());
    }

    #[test]
    fn test_parse_remote_ssh_no_suffix() {
        let result = parse_remote_url("git@github.com:user/repo");
        assert_eq!(result, Some(("user".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_poll_manager_starts_idle() {
        let pm = PollManager::new();
        assert_eq!(pm.interval(), Duration::from_secs(30));
    }

    #[test]
    fn test_poll_manager_goes_active_on_change() {
        let mut pm = PollManager::new();
        let info = PrDisplayInfo {
            number: 1,
            title: "test".to_string(),
            state: PrStatus::Open,
            reviews: vec![],
            checks: ChecksInfo {
                total: 0,
                passed: 0,
                failed: 0,
                pending: 0,
                skipped: 0,
                checks: vec![],
            },
            comment_count: 0,
            mergeable: MergeableStatus::Clean,
            labels: vec![],
            assignees: vec![],
        };
        let interval = pm.report(&info);
        assert_eq!(interval, Duration::from_secs(10));
    }

    #[test]
    fn test_poll_manager_backs_off_after_unchanged() {
        let mut pm = PollManager::new();
        let info = PrDisplayInfo {
            number: 1,
            title: "test".to_string(),
            state: PrStatus::Open,
            reviews: vec![],
            checks: ChecksInfo {
                total: 0,
                passed: 0,
                failed: 0,
                pending: 0,
                skipped: 0,
                checks: vec![],
            },
            comment_count: 0,
            mergeable: MergeableStatus::Clean,
            labels: vec![],
            assignees: vec![],
        };
        pm.report(&info); // first time — active
        pm.report(&info); // unchanged 1
        pm.report(&info); // unchanged 2
        let interval = pm.report(&info); // unchanged 3 — backs off
        assert_eq!(interval, Duration::from_secs(30));
    }
}
