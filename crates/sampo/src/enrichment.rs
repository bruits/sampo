//! Enrichment module for changeset messages with commit information and author acknowledgments.
//!
//! This module provides functionality to enhance changeset messages by adding:
//! - Commit hash information with optional GitHub links
//! - Author acknowledgments with GitHub username detection
//! - Special badges for first-time contributors
//!
//! The enrichment is designed to be graceful: if Git or GitHub information is unavailable,
//! the original message is returned unchanged.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub author_name: String,
}

#[derive(Debug, Clone)]
pub struct GitHubUserInfo {
    pub login: String,
    pub is_first_contribution: bool,
}

/// Enrich a changeset message with commit information and author acknowledgments.
///
/// This function adds:
/// - Commit hash with optional GitHub link
/// - Author acknowledgment with optional first contribution badge
///
/// # Arguments
/// * `repo_root` - Path to the git repository root
/// * `changeset_path` - Path to the changeset file
/// * `message` - Original changeset message
/// * `repo_slug` - Optional GitHub repository slug (owner/repo)
/// * `github_token` - Optional GitHub API token for user info lookups
///
/// # Returns
/// Enhanced message in format: `[commit_hash](link) message — Thanks @user for your first contribution 🎉!`
pub fn enrich_changeset_message(
    repo_root: &Path,
    changeset_path: &Path,
    message: &str,
    repo_slug: Option<&str>,
    github_token: Option<&str>,
) -> String {
    let commit = get_commit_info_for_path(repo_root, changeset_path);

    let commit_prefix = build_commit_prefix(&commit, repo_slug);
    let acknowledgment_suffix = build_acknowledgment_suffix(&commit, repo_slug, github_token);

    format_enriched_message(message, &commit_prefix, &acknowledgment_suffix)
}

/// Get commit information for when a file was first added to the repository.
fn get_commit_info_for_path(repo_root: &Path, path: &Path) -> Option<CommitInfo> {
    let relative_path = path.strip_prefix(repo_root).unwrap_or(path);
    let relative_path_str = relative_path.to_string_lossy();

    let format_string = "%H\x1f%h\x1f%an";
    let output = Command::new("git")
        .current_dir(repo_root)
        .args([
            "log",
            "--diff-filter=A", // Only show commits that added the file
            "--follow",
            "-n", "1", // Get only the first (adding) commit
            &format!("--pretty=format:{}", format_string),
            "--",
            &relative_path_str,
        ])
        .output()
        .ok()?;
    
    if !output.status.success() {
        return None;
    }
    
    let output_str = String::from_utf8_lossy(&output.stdout);
    let line = output_str.lines().next()?;
    let parts: Vec<&str> = line.split('\u{001F}').collect();
    
    if parts.len() < 3 {
        return None;
    }    Some(CommitInfo {
        sha: parts[0].to_string(),
        short_sha: parts[1].to_string(),
        author_name: parts[2].to_string(),
    })
}

/// Build the commit prefix for the enriched message.
fn build_commit_prefix(commit: &Option<CommitInfo>, repo_slug: Option<&str>) -> String {
    let Some(commit_info) = commit else {
        return String::new();
    };

    if let Some(slug) = repo_slug {
        format!(
            "[`{}`](https://github.com/{}/commit/{}) ",
            commit_info.short_sha,
            slug,
            commit_info.sha
        )
    } else {
        format!("`{}` ", commit_info.short_sha)
    }
}

/// Build the acknowledgment suffix for the enriched message.
fn build_acknowledgment_suffix(
    commit: &Option<CommitInfo>,
    repo_slug: Option<&str>,
    github_token: Option<&str>,
) -> String {
    let Some(commit_info) = commit else {
        return String::new();
    };

    // Try to get GitHub user info if we have both repo slug and token
    let github_user = match (repo_slug, github_token) {
        (Some(slug), Some(token)) => lookup_github_user_info(slug, token, &commit_info.sha),
        _ => None,
    };

    let (display_name, is_github_login) = match github_user {
        Some(ref user) => (user.login.clone(), true),
        None => (commit_info.author_name.clone(), false),
    };

    if display_name.is_empty() {
        return String::new();
    }

    let mut suffix = String::from(" — Thanks ");
    if is_github_login {
        suffix.push('@');
    }
    suffix.push_str(&display_name);

    if github_user.is_some_and(|u| u.is_first_contribution) {
        suffix.push_str(" for your first contribution 🎉");
    }

    suffix.push('!');
    suffix
}

/// Format the final enriched message by combining all parts.
fn format_enriched_message(message: &str, prefix: &str, suffix: &str) -> String {
    match (prefix.is_empty(), suffix.is_empty()) {
        (true, true) => message.to_string(),
        (false, true) => format!("{}{}", prefix, message),
        (true, false) => format!("{}{}", message, suffix),
        (false, false) => format!("{}{}{}", prefix, message, suffix),
    }
}

/// Lookup GitHub user information for a specific commit.
fn lookup_github_user_info(repo_slug: &str, token: &str, sha: &str) -> Option<GitHubUserInfo> {
    let login = lookup_github_login_for_commit(repo_slug, token, sha)?;
    let is_first_contribution = is_first_contribution(repo_slug, token, &login).unwrap_or(false);

    Some(GitHubUserInfo {
        login,
        is_first_contribution,
    })
}

/// Get the GitHub login for the author of a specific commit.
fn lookup_github_login_for_commit(repo_slug: &str, token: &str, sha: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{}/commits/{}", repo_slug, sha);
    let output = Command::new("curl")
        .args([
            "-sS",
            "-H", &format!("Authorization: Bearer {}", token),
            "-H", "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let body = String::from_utf8_lossy(&output.stdout);
    parse_github_login_from_response(&body)
}

/// Parse GitHub login from API response JSON.
fn parse_github_login_from_response(response_body: &str) -> Option<String> {
    // Simple JSON parsing: look for "login": "username"
    let start_pattern = "\"login\":\"";
    let start_pos = response_body.find(start_pattern)? + start_pattern.len();
    let end_pos = response_body[start_pos..].find('"')?;
    let login = &response_body[start_pos..start_pos + end_pos];

    if login.is_empty() {
        None
    } else {
        Some(login.to_string())
    }
}

/// Check if this is the user's first contribution to the repository.
fn is_first_contribution(repo_slug: &str, token: &str, login: &str) -> Option<bool> {
    let url = format!(
        "https://api.github.com/repos/{}/commits?author={}&per_page=2",
        repo_slug, login
    );
    let output = Command::new("curl")
        .args([
            "-sS",
            "-H", &format!("Authorization: Bearer {}", token),
            "-H", "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let body = String::from_utf8_lossy(&output.stdout);
    // Count SHA occurrences to determine commit count
    let commit_count = body.matches("\"sha\"").count();
    Some(commit_count <= 1)
}

/// Detect GitHub repository slug from environment or git remote.
pub fn detect_github_repo_slug(repo_root: &Path) -> Option<String> {
    // First, try environment variable (common in CI)
    if let Ok(repo) = std::env::var("GITHUB_REPOSITORY")
        && !repo.is_empty()
        && repo.contains('/')
    {
        return Some(repo);
    }

    // Fallback to parsing git remote origin URL
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_github_slug_from_url(&url)
}

/// Parse GitHub repository slug from various URL formats.
fn parse_github_slug_from_url(url: &str) -> Option<String> {
    // Handle SSH format: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let slug = rest.trim_end_matches(".git");
        if slug.split('/').count() == 2 {
            return Some(slug.to_string());
        }
    }

    // Handle HTTPS format: https://github.com/owner/repo.git
    if let Some(pos) = url.find("github.com/") {
        let rest = &url[pos + "github.com/".len()..];
        let slug = rest
            .trim_end_matches('/')
            .trim_end_matches(".git");

        let parts: Vec<&str> = slug.split('/').collect();
        if parts.len() >= 2 {
            return Some(format!("{}/{}", parts[0], parts[1]));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_slug_ssh() {
        let url = "git@github.com:owner/repo.git";
        assert_eq!(parse_github_slug_from_url(url), Some("owner/repo".to_string()));

        let url_no_git = "git@github.com:owner/repo";
        assert_eq!(parse_github_slug_from_url(url_no_git), Some("owner/repo".to_string()));
    }

    #[test]
    fn test_parse_github_slug_https() {
        let url = "https://github.com/owner/repo.git";
        assert_eq!(parse_github_slug_from_url(url), Some("owner/repo".to_string()));

        let url_no_git = "https://github.com/owner/repo";
        assert_eq!(parse_github_slug_from_url(url_no_git), Some("owner/repo".to_string()));
    }

    #[test]
    fn test_parse_github_login_from_response() {
        let response = r#"{"author":{"login":"testuser","id":123}}"#;
        assert_eq!(parse_github_login_from_response(response), Some("testuser".to_string()));

        let empty_response = r#"{"author":null}"#;
        assert_eq!(parse_github_login_from_response(empty_response), None);
    }

    #[test]
    fn test_format_enriched_message() {
        let message = "Add new feature";
        let prefix = "`abc123` ";
        let suffix = " — Thanks @user!";

        assert_eq!(
            format_enriched_message(message, prefix, suffix),
            "`abc123` Add new feature — Thanks @user!"
        );

        assert_eq!(
            format_enriched_message(message, "", ""),
            "Add new feature"
        );
    }
}
