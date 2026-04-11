#![allow(dead_code)] // Removed in Task 5 once poller.rs uses these types.

use serde::Deserialize;

/// The single GraphQL query that fetches everything the PR widget needs.
/// Variables: `$owner: String!`, `$repo: String!`, `$branch: String!`.
pub const PR_QUERY: &str = r#"
query($owner: String!, $repo: String!, $branch: String!) {
  repository(owner: $owner, name: $repo) {
    pullRequests(headRefName: $branch, states: OPEN, first: 1) {
      nodes {
        number
        title
        isDraft
        mergeable
        mergeStateStatus
        comments { totalCount }
        labels(first: 20) { nodes { name } }
        assignees(first: 10) { nodes { login } }
        reviews(last: 50) {
          nodes { author { login } state }
        }
        commits(last: 1) {
          nodes {
            commit {
              checkSuites(first: 10) {
                nodes {
                  checkRuns(first: 50) {
                    nodes { name status conclusion }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
"#;

/// Top-level GraphQL response envelope.
#[derive(Debug, Deserialize)]
pub struct GqlResponse {
    pub data: Option<GqlData>,
    pub errors: Option<Vec<GqlError>>,
}

/// A single GraphQL error entry.
#[derive(Debug, Deserialize)]
pub struct GqlError {
    pub message: String,
}

/// The `data` field of the response.
#[derive(Debug, Deserialize)]
pub struct GqlData {
    pub repository: Option<GqlRepository>,
}

/// The `repository` field.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GqlRepository {
    pub pull_requests: GqlNodes<GqlPullRequest>,
}

/// Generic wrapper for any GraphQL connection that exposes `nodes`.
#[derive(Debug, Deserialize)]
pub struct GqlNodes<T> {
    pub nodes: Vec<T>,
}

/// A pull request node from the query.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GqlPullRequest {
    pub number: u64,
    pub title: String,
    pub is_draft: bool,
    /// `MERGEABLE` | `CONFLICTING` | `UNKNOWN`
    pub mergeable: String,
    /// `CLEAN` | `DIRTY` | `BEHIND` | `UNSTABLE` | `BLOCKED` | `HAS_HOOKS` | `DRAFT` | `UNKNOWN`
    pub merge_state_status: String,
    pub comments: GqlCount,
    pub labels: GqlNodes<GqlLabel>,
    pub assignees: GqlNodes<GqlUser>,
    pub reviews: GqlNodes<GqlReview>,
    pub commits: GqlNodes<GqlCommitWrap>,
}

/// A connection that only exposes `totalCount`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GqlCount {
    pub total_count: u64,
}

#[derive(Debug, Deserialize)]
pub struct GqlLabel {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct GqlUser {
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub struct GqlReview {
    /// Null when the reviewer is a ghost/deleted user.
    pub author: Option<GqlUser>,
    /// `PENDING` | `COMMENTED` | `APPROVED` | `CHANGES_REQUESTED` | `DISMISSED`
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct GqlCommitWrap {
    pub commit: GqlCommit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GqlCommit {
    pub check_suites: GqlNodes<GqlCheckSuite>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GqlCheckSuite {
    pub check_runs: GqlNodes<GqlCheckRun>,
}

#[derive(Debug, Deserialize)]
pub struct GqlCheckRun {
    pub name: String,
    /// `QUEUED` | `IN_PROGRESS` | `COMPLETED` | `WAITING` | `PENDING` | `REQUESTED`
    pub status: String,
    /// `ACTION_REQUIRED` | `TIMED_OUT` | `CANCELLED` | `FAILURE` | `SUCCESS`
    /// | `NEUTRAL` | `SKIPPED` | `STARTUP_FAILURE` | `STALE`
    pub conclusion: Option<String>,
}
