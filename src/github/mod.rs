pub mod client;
pub mod convert;
pub mod poller;
pub mod query;
pub mod types;

use std::collections::HashMap;
use std::path::Path;

pub use client::{get_remote_url, parse_remote_url, resolve_auth_token};

use poller::PollManager;

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
