use serde::Deserialize;

/// A pull request from the GitHub API
#[derive(Debug, Deserialize)]
pub struct GitHubPr {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub draft: Option<bool>,
    pub mergeable: Option<bool>,
    pub mergeable_state: Option<String>,
    pub comments: Option<u64>,
    pub labels: Option<Vec<GitHubLabel>>,
    pub assignees: Option<Vec<GitHubUser>>,
    pub head: GitHubRef,
}

/// A label on a GitHub PR
#[derive(Debug, Deserialize)]
pub struct GitHubLabel {
    pub name: String,
}

/// A GitHub user
#[derive(Debug, Deserialize)]
pub struct GitHubUser {
    pub login: String,
}

/// A git ref (branch) in a GitHub PR
#[derive(Debug, Deserialize)]
pub struct GitHubRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
}

/// A review on a GitHub PR
#[derive(Debug, Deserialize)]
pub struct GitHubReview {
    pub user: GitHubUser,
    pub state: String,
}

/// Response from the check-runs API endpoint
#[derive(Debug, Deserialize)]
pub struct GitHubCheckRuns {
    pub total_count: u64,
    pub check_runs: Vec<GitHubCheckRun>,
}

/// A single check run from GitHub Actions / status checks
#[derive(Debug, Deserialize)]
pub struct GitHubCheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

/// A comment on a GitHub issue/PR
#[derive(Debug, Deserialize)]
pub struct GitHubComment {
    pub id: u64,
}
