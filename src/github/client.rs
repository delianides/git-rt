use std::path::Path;
use std::process::Command;

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
}
