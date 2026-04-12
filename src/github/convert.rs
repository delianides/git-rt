use std::collections::HashMap;

use crate::github::query::{GqlCommitWrap, GqlData, GqlNodes, GqlPullRequest, GqlReview};
use crate::state::{
    CheckInfo, CheckStatus, ChecksInfo, MergeableStatus, PrDisplayInfo, PrStatus, ReviewInfo,
    ReviewState,
};

/// Convert a GraphQL response's `data` field into a `PrDisplayInfo`.
/// Returns `None` when the repository has no matching open PR.
pub fn to_pr_display_info(data: Option<GqlData>, owner: &str, repo: &str) -> Option<PrDisplayInfo> {
    let pr = data?.repository?.pull_requests.nodes.into_iter().next()?;
    Some(pr_to_display_info(pr, owner, repo))
}

fn pr_to_display_info(pr: GqlPullRequest, owner: &str, repo: &str) -> PrDisplayInfo {
    let state = map_pr_state(&pr.state, pr.is_draft, &pr.merge_state_status);
    let mergeable = map_mergeable(&pr.mergeable, &pr.merge_state_status);
    let reviews = dedup_reviews(pr.reviews.nodes);
    let (checks_vec, counts) = flatten_check_runs(&pr.commits);
    let labels = pr.labels.nodes.into_iter().map(|l| l.name).collect();
    let assignees = pr.assignees.nodes.into_iter().map(|a| a.login).collect();

    PrDisplayInfo {
        number: pr.number,
        title: pr.title,
        state,
        reviews,
        checks: ChecksInfo {
            total: checks_vec.len(),
            passed: counts.passed,
            failed: counts.failed,
            pending: counts.pending,
            skipped: counts.skipped,
            checks: checks_vec,
        },
        comment_count: pr.comments.total_count,
        mergeable,
        labels,
        assignees,
        url: format!("https://github.com/{owner}/{repo}/pull/{}", pr.number),
    }
}

#[derive(Default)]
struct ChecksCounts {
    passed: usize,
    failed: usize,
    pending: usize,
    skipped: usize,
}

fn map_pr_state(api_state: &str, is_draft: bool, merge_state_status: &str) -> PrStatus {
    if api_state == "MERGED" {
        PrStatus::Merged
    } else if is_draft || merge_state_status == "DRAFT" {
        PrStatus::Draft
    } else {
        PrStatus::Open
    }
}

fn map_review_state(s: &str) -> ReviewState {
    match s {
        "APPROVED" => ReviewState::Approved,
        "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
        "PENDING" => ReviewState::Pending,
        "COMMENTED" => ReviewState::Commented,
        "DISMISSED" => ReviewState::Dismissed,
        _ => ReviewState::Pending,
    }
}

fn map_mergeable(mergeable: &str, merge_state_status: &str) -> MergeableStatus {
    match merge_state_status {
        "CLEAN" | "UNSTABLE" => MergeableStatus::Clean,
        "DIRTY" => MergeableStatus::Conflicts,
        "BEHIND" => MergeableStatus::Behind,
        "BLOCKED" | "HAS_HOOKS" | "DRAFT" => match mergeable {
            "MERGEABLE" => MergeableStatus::Clean,
            "CONFLICTING" => MergeableStatus::Conflicts,
            _ => MergeableStatus::Unknown,
        },
        _ => MergeableStatus::Unknown,
    }
}

fn map_check_status(status: &str, conclusion: Option<&str>) -> CheckStatus {
    match status {
        "COMPLETED" => match conclusion {
            Some("SUCCESS") => CheckStatus::Passed,
            Some("FAILURE")
            | Some("TIMED_OUT")
            | Some("CANCELLED")
            | Some("STARTUP_FAILURE")
            | Some("ACTION_REQUIRED") => CheckStatus::Failed,
            Some("SKIPPED") | Some("NEUTRAL") | Some("STALE") => CheckStatus::Skipped,
            _ => CheckStatus::Pending,
        },
        "IN_PROGRESS" => CheckStatus::Running,
        _ => CheckStatus::Pending,
    }
}

fn dedup_reviews(reviews: Vec<GqlReview>) -> Vec<ReviewInfo> {
    let mut latest: HashMap<String, ReviewInfo> = HashMap::new();
    for review in reviews {
        let login = match review.author {
            Some(user) => user.login,
            None => continue, // ghost/deleted user — skip
        };
        let state = map_review_state(&review.state);
        latest.insert(
            login.clone(),
            ReviewInfo {
                reviewer: login,
                state,
            },
        );
    }
    latest.into_values().collect()
}

fn flatten_check_runs(commits: &GqlNodes<GqlCommitWrap>) -> (Vec<CheckInfo>, ChecksCounts) {
    let mut out: Vec<CheckInfo> = Vec::new();
    let mut counts = ChecksCounts::default();

    let Some(commit_wrap) = commits.nodes.first() else {
        return (out, counts);
    };

    for suite in &commit_wrap.commit.check_suites.nodes {
        for run in &suite.check_runs.nodes {
            let status = map_check_status(&run.status, run.conclusion.as_deref());
            match status {
                CheckStatus::Passed => counts.passed += 1,
                CheckStatus::Failed => counts.failed += 1,
                CheckStatus::Skipped => counts.skipped += 1,
                CheckStatus::Pending | CheckStatus::Running => counts.pending += 1,
            }
            out.push(CheckInfo {
                name: run.name.clone(),
                status,
            });
        }
    }

    (out, counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::query::GqlResponse;

    /// Parse a GraphQL response JSON string into `GqlResponse` for testing.
    fn parse(json: &str) -> GqlResponse {
        serde_json::from_str(json).expect("test fixture JSON failed to parse")
    }

    #[test]
    fn happy_path_open_pr() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 42,
            "title": "Add feature X",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "comments": { "totalCount": 3 },
            "labels": { "nodes": [{ "name": "bug" }, { "name": "enhancement" }] },
            "assignees": { "nodes": [{ "login": "alice" }] },
            "reviews": {
              "nodes": [
                { "author": { "login": "bob" }, "state": "APPROVED" }
              ]
            },
            "commits": {
              "nodes": [
                {
                  "commit": {
                    "checkSuites": {
                      "nodes": [
                        {
                          "checkRuns": {
                            "nodes": [
                              { "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" },
                              { "name": "test",  "status": "COMPLETED", "conclusion": "SUCCESS" }
                            ]
                          }
                        }
                      ]
                    }
                  }
                }
              ]
            }
          }
        ]
      }
    }
  }
}"#,
        );

        let info = to_pr_display_info(resp.data, "test-owner", "test-repo")
            .expect("expected Some PrDisplayInfo");
        assert_eq!(info.number, 42);
        assert_eq!(info.title, "Add feature X");
        assert_eq!(info.state, PrStatus::Open);
        assert_eq!(info.comment_count, 3);
        assert_eq!(info.mergeable, MergeableStatus::Clean);
        assert_eq!(
            info.labels,
            vec!["bug".to_string(), "enhancement".to_string()]
        );
        assert_eq!(info.assignees, vec!["alice".to_string()]);
        assert_eq!(info.reviews.len(), 1);
        assert_eq!(info.reviews[0].reviewer, "bob");
        assert_eq!(info.reviews[0].state, ReviewState::Approved);
        assert_eq!(info.checks.total, 2);
        assert_eq!(info.checks.passed, 2);
        assert_eq!(info.checks.failed, 0);
        assert_eq!(info.url, "https://github.com/test-owner/test-repo/pull/42");
    }

    #[test]
    fn no_pr_for_branch() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": { "nodes": [] }
    }
  }
}"#,
        );
        assert!(to_pr_display_info(resp.data, "test-owner", "test-repo").is_none());
    }

    #[test]
    fn draft_pr_maps_to_draft_status() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "wip",
            "state": "OPEN",
            "isDraft": true,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "DRAFT",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": { "nodes": [] },
            "commits": { "nodes": [] }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.state, PrStatus::Draft);
    }

    #[test]
    fn dirty_merge_state_maps_to_conflicts() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "CONFLICTING",
            "mergeStateStatus": "DIRTY",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": { "nodes": [] },
            "commits": { "nodes": [] }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.mergeable, MergeableStatus::Conflicts);
    }

    #[test]
    fn behind_merge_state_maps_to_behind() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "BEHIND",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": { "nodes": [] },
            "commits": { "nodes": [] }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.mergeable, MergeableStatus::Behind);
    }

    #[test]
    fn mixed_check_states_count_correctly() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "UNSTABLE",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": { "nodes": [] },
            "commits": {
              "nodes": [
                {
                  "commit": {
                    "checkSuites": {
                      "nodes": [
                        {
                          "checkRuns": {
                            "nodes": [
                              { "name": "a", "status": "COMPLETED",  "conclusion": "SUCCESS" },
                              { "name": "b", "status": "COMPLETED",  "conclusion": "FAILURE" },
                              { "name": "c", "status": "IN_PROGRESS","conclusion": null      },
                              { "name": "d", "status": "COMPLETED",  "conclusion": "SKIPPED" }
                            ]
                          }
                        }
                      ]
                    }
                  }
                }
              ]
            }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.checks.total, 4);
        assert_eq!(info.checks.passed, 1);
        assert_eq!(info.checks.failed, 1);
        assert_eq!(info.checks.pending, 1);
        assert_eq!(info.checks.skipped, 1);
        assert_eq!(info.mergeable, MergeableStatus::Clean); // UNSTABLE -> Clean
    }

    #[test]
    fn completed_check_with_null_conclusion_is_pending() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": { "nodes": [] },
            "commits": {
              "nodes": [
                {
                  "commit": {
                    "checkSuites": {
                      "nodes": [
                        {
                          "checkRuns": {
                            "nodes": [
                              { "name": "x", "status": "COMPLETED", "conclusion": null }
                            ]
                          }
                        }
                      ]
                    }
                  }
                }
              ]
            }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.checks.pending, 1);
        assert_eq!(info.checks.passed, 0);
    }

    #[test]
    fn action_required_check_maps_to_failed() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": { "nodes": [] },
            "commits": {
              "nodes": [
                {
                  "commit": {
                    "checkSuites": {
                      "nodes": [
                        {
                          "checkRuns": {
                            "nodes": [
                              { "name": "e2e", "status": "COMPLETED", "conclusion": "ACTION_REQUIRED" }
                            ]
                          }
                        }
                      ]
                    }
                  }
                }
              ]
            }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.checks.total, 1);
        assert_eq!(info.checks.failed, 1);
        assert_eq!(info.checks.passed, 0);
        assert_eq!(info.checks.pending, 0);
    }

    #[test]
    fn neutral_and_stale_checks_map_to_skipped() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": { "nodes": [] },
            "commits": {
              "nodes": [
                {
                  "commit": {
                    "checkSuites": {
                      "nodes": [
                        {
                          "checkRuns": {
                            "nodes": [
                              { "name": "lint",  "status": "COMPLETED", "conclusion": "NEUTRAL" },

                              { "name": "retry", "status": "COMPLETED", "conclusion": "STALE" }
                            ]
                          }
                        }
                      ]
                    }
                  }
                }
              ]
            }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.checks.total, 2);
        assert_eq!(info.checks.skipped, 2);
        assert_eq!(info.checks.passed, 0);
        assert_eq!(info.checks.failed, 0);
        assert_eq!(info.checks.pending, 0);
    }

    #[test]
    fn empty_reviews_checks_labels_assignees() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 9,
            "title": "empty",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": { "nodes": [] },
            "commits": { "nodes": [] }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert!(info.reviews.is_empty());
        assert!(info.labels.is_empty());
        assert!(info.assignees.is_empty());
        assert_eq!(info.checks.total, 0);
    }

    #[test]
    fn ghost_reviewer_is_skipped() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": {
              "nodes": [
                { "author": null, "state": "COMMENTED" },
                { "author": { "login": "alice" }, "state": "APPROVED" }
              ]
            },
            "commits": { "nodes": [] }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.reviews.len(), 1);
        assert_eq!(info.reviews[0].reviewer, "alice");
    }

    #[test]
    fn dedup_reviews_keeps_latest_per_reviewer() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "comments": { "totalCount": 0 },
            "labels": { "nodes": [] },
            "assignees": { "nodes": [] },
            "reviews": {
              "nodes": [
                { "author": { "login": "bob" }, "state": "CHANGES_REQUESTED" },
                { "author": { "login": "bob" }, "state": "APPROVED" }
              ]
            },
            "commits": { "nodes": [] }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo").unwrap();
        assert_eq!(info.reviews.len(), 1);
        assert_eq!(info.reviews[0].reviewer, "bob");
        assert_eq!(info.reviews[0].state, ReviewState::Approved);
    }

    #[test]
    fn merged_pr_maps_to_merged_status() {
        let resp = parse(
            r#"{
  "data": {
    "repository": {
      "pullRequests": {
        "nodes": [
          {
            "number": 99,
            "title": "Ship it",
            "state": "MERGED",
            "isDraft": false,
            "mergeable": "UNKNOWN",
            "mergeStateStatus": "UNKNOWN",
            "comments": { "totalCount": 5 },
            "labels": { "nodes": [{ "name": "shipped" }] },
            "assignees": { "nodes": [] },
            "reviews": {
              "nodes": [
                { "author": { "login": "reviewer" }, "state": "APPROVED" }
              ]
            },
            "commits": { "nodes": [] }
          }
        ]
      }
    }
  }
}"#,
        );
        let info = to_pr_display_info(resp.data, "test-owner", "test-repo")
            .expect("expected Some PrDisplayInfo");
        assert_eq!(info.number, 99);
        assert_eq!(info.title, "Ship it");
        assert_eq!(info.state, PrStatus::Merged);
        assert_eq!(info.comment_count, 5);
    }
}
