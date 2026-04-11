use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};

use crate::github::client::{get_remote_url, parse_remote_url};
use crate::github::convert;
use crate::github::query::{self, GqlResponse};
use crate::state::PrDisplayInfo;

/// Events sent from the GitHub polling thread to the main event loop.
#[derive(Debug)]
pub enum GitHubEvent {
    /// PR data was fetched and converted to display info.
    PrUpdate(PrDisplayInfo),
    /// No PR found for the current branch.
    NoPr,
    /// An error occurred during fetch.
    Error(String),
}

/// Fetch PR data via a single GraphQL request.
///
/// Returns `Ok(None)` when the repository has no matching open PR.
/// Returns `Err` for network failures, JSON parse failures, or any
/// non-empty `errors` array in the GraphQL response.
pub(super) fn fetch_pr_data(
    agent: &ureq::Agent,
    owner: &str,
    repo: &str,
    branch: &str,
    token: &str,
) -> Result<Option<PrDisplayInfo>> {
    let body = serde_json::json!({
        "query": query::PR_QUERY,
        "variables": { "owner": owner, "repo": repo, "branch": branch },
    });

    let resp: GqlResponse = agent
        .post("https://api.github.com/graphql")
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "git-rt")
        .send_json(body)
        .context("GraphQL request failed")?
        .body_mut()
        .read_json()
        .context("Failed to parse GraphQL response")?;

    if let Some(errors) = resp.errors.as_ref() {
        if !errors.is_empty() {
            anyhow::bail!("GraphQL error: {}", errors[0].message);
        }
    }

    Ok(convert::to_pr_display_info(resp.data))
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

            // One agent, reused for every poll.
            let agent = ureq::Agent::new_with_defaults();
            let mut poll_manager = PollManager::new();

            loop {
                let event = match fetch_pr_data(&agent, &owner, &repo, &branch, &token) {
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

/// Manages adaptive polling intervals for GitHub API requests.
/// Starts at an idle interval and switches to a faster active interval
/// when changes are detected, backing off after consecutive unchanged responses.
pub(super) struct PollManager {
    idle_interval: Duration,
    active_interval: Duration,
    current_interval: Duration,
    unchanged_count: u32,
    backoff_threshold: u32,
    last_hash: u64,
}

impl PollManager {
    pub(super) fn new() -> Self {
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
    pub(super) fn report(&mut self, info: &PrDisplayInfo) -> Duration {
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
    pub(super) fn go_active(&mut self) {
        self.unchanged_count = 0;
        self.current_interval = self.active_interval;
    }

    /// The current polling interval.
    pub(super) fn interval(&self) -> Duration {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ChecksInfo, MergeableStatus, PrStatus};

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
