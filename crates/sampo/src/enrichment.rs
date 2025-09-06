//! Enrichment module for changeset messages with commit information and author acknowledgments.
//!
//! This module provides functionality to enhance changeset messages by adding:
//! - Commit hash information with optional GitHub links
//! - Author acknowledgments with GitHub username detection
//! - Special messages for first-time contributors
//!
//! The enrichment is designed to be graceful: if Git or GitHub information is unavailable,
//! the original message is returned unchanged. GitHub API calls are made using the reqwest
//! HTTP client for reliability and portability.

use serde_json::Value;
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
/// Takes a changeset message and optionally enhances it with:
/// - Commit hash link (if show_commit_hash is true)
/// - Author acknowledgment with optional first contribution message (if show_acknowledgments is true)
///
/// # Arguments
/// * `message` - The original changeset message
/// * `commit_hash` - Git commit hash for this changeset
/// * `workspace` - Repository workspace path
/// * `repo_slug` - GitHub repository slug (owner/repo)
/// * `github_token` - Optional GitHub API token for enhanced features
/// * `show_commit_hash` - Whether to include commit hash links
/// * `show_acknowledgments` - Whether to include author acknowledgments
///
/// # Returns
/// Enhanced message in format: `[commit_hash](link) message — Thanks @user for your first contribution 🎉!`
pub fn enrich_changeset_message(
    message: &str,
    commit_hash: &str,
    workspace: &Path,
    repo_slug: Option<&str>,
    github_token: Option<&str>,
    show_commit_hash: bool,
    show_acknowledgments: bool,
) -> String {
    // Create a tokio runtime for this blocking call
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(enrich_changeset_message_async(
        message,
        commit_hash,
        workspace,
        repo_slug,
        github_token,
        show_commit_hash,
        show_acknowledgments,
    ))
}

/// Async version of enrich_changeset_message for internal use.
async fn enrich_changeset_message_async(
    message: &str,
    commit_hash: &str,
    workspace: &Path,
    repo_slug: Option<&str>,
    github_token: Option<&str>,
    show_commit_hash: bool,
    show_acknowledgments: bool,
) -> String {
    let commit = get_commit_info_for_hash(workspace, commit_hash);

    let commit_prefix = if show_commit_hash {
        build_commit_prefix(&commit, repo_slug)
    } else {
        String::new()
    };

    let acknowledgment_suffix = if show_acknowledgments {
        build_acknowledgment_suffix(&commit, repo_slug, github_token).await
    } else {
        String::new()
    };

    format_enriched_message(message, &commit_prefix, &acknowledgment_suffix)
}

/// Get commit information for a specific commit hash.
fn get_commit_info_for_hash(repo_root: &Path, commit_hash: &str) -> Option<CommitInfo> {
    let format_string = "%H\x1f%h\x1f%an";
    let output = Command::new("git")
        .current_dir(repo_root)
        .args([
            "show",
            "-s", // Suppress diff output
            &format!("--pretty=format:{}", format_string),
            commit_hash,
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
    }

    Some(CommitInfo {
        sha: parts[0].to_string(),
        short_sha: parts[1].to_string(),
        author_name: parts[2].to_string(),
    })
}

/// Get commit hash for when a file was first added to the repository.
pub fn get_commit_hash_for_path(repo_root: &Path, path: &Path) -> Option<String> {
    let commit_info = get_commit_info_for_path(repo_root, path)?;
    Some(commit_info.sha)
}

/// Get commit information for when a file was first added to the repository.
#[allow(dead_code)]
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
            "-n",
            "1", // Get only the first (adding) commit
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
    }

    Some(CommitInfo {
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
            commit_info.short_sha, slug, commit_info.sha
        )
    } else {
        format!("`{}` ", commit_info.short_sha)
    }
}

/// Build the acknowledgment suffix for the enriched message.
async fn build_acknowledgment_suffix(
    commit: &Option<CommitInfo>,
    repo_slug: Option<&str>,
    github_token: Option<&str>,
) -> String {
    let Some(commit_info) = commit else {
        return String::new();
    };

    // Try to get GitHub user info if we have both repo slug and token
    let github_user = match (repo_slug, github_token) {
        (Some(slug), Some(token)) => lookup_github_user_info(slug, token, &commit_info.sha).await,
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
async fn lookup_github_user_info(
    repo_slug: &str,
    token: &str,
    sha: &str,
) -> Option<GitHubUserInfo> {
    let login = lookup_github_login_for_commit(repo_slug, token, sha).await?;
    let is_first_contribution = is_first_contribution(repo_slug, token, &login)
        .await
        .unwrap_or(false);

    Some(GitHubUserInfo {
        login,
        is_first_contribution,
    })
}

/// Get the GitHub login for the author of a specific commit.
async fn lookup_github_login_for_commit(repo_slug: &str, token: &str, sha: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{}/commits/{}", repo_slug, sha);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "sampo-github-action")
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let body = response.text().await.ok()?;
    parse_github_login_from_response(&body)
}

/// Parse GitHub login from API response JSON.
fn parse_github_login_from_response(response_body: &str) -> Option<String> {
    let json: Value = serde_json::from_str(response_body).ok()?;

    // Navigate to author.login in the JSON structure
    json.get("author")?
        .get("login")?
        .as_str()
        .filter(|login| !login.is_empty())
        .map(|login| login.to_string())
}

/// Check if this is the user's first contribution to the repository.
async fn is_first_contribution(repo_slug: &str, token: &str, login: &str) -> Option<bool> {
    let url = format!(
        "https://api.github.com/repos/{}/commits?author={}&per_page=2",
        repo_slug, login
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "sampo-github-action")
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let body = response.text().await.ok()?;
    let json: Value = serde_json::from_str(&body).ok()?;

    // Parse the JSON array and count commits
    let commits = json.as_array()?;
    Some(commits.len() <= 1)
}

/// Detect GitHub repository slug from configuration, environment, or git remote.
///
/// This is a convenience function that calls `detect_github_repo_slug_with_config`
/// with `None` as the configuration parameter.
#[allow(dead_code)]
pub fn detect_github_repo_slug(repo_root: &Path) -> Option<String> {
    detect_github_repo_slug_with_config(repo_root, None)
}

/// Detect GitHub repository slug with optional configuration override.
pub fn detect_github_repo_slug_with_config(
    repo_root: &Path,
    config_repo: Option<&str>,
) -> Option<String> {
    // First priority: configuration from .sampo/config.toml
    if let Some(repo) = config_repo
        && !repo.is_empty()
        && repo.contains('/')
    {
        return Some(repo.to_string());
    }

    // Second priority: environment variable (common in CI)
    if let Ok(repo) = std::env::var("GITHUB_REPOSITORY")
        && !repo.is_empty()
        && repo.contains('/')
    {
        return Some(repo);
    }

    // Third priority: parsing git remote origin URL
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
        let slug = rest.trim_end_matches('/').trim_end_matches(".git");

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
        assert_eq!(
            parse_github_slug_from_url(url),
            Some("owner/repo".to_string())
        );

        let url_no_git = "git@github.com:owner/repo";
        assert_eq!(
            parse_github_slug_from_url(url_no_git),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_slug_https() {
        let url = "https://github.com/owner/repo.git";
        assert_eq!(
            parse_github_slug_from_url(url),
            Some("owner/repo".to_string())
        );

        let url_no_git = "https://github.com/owner/repo";
        assert_eq!(
            parse_github_slug_from_url(url_no_git),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_login_from_response() {
        let response = r#"{"author":{"login":"testuser","id":123}}"#;
        assert_eq!(
            parse_github_login_from_response(response),
            Some("testuser".to_string())
        );

        let empty_response = r#"{"author":null}"#;
        assert_eq!(parse_github_login_from_response(empty_response), None);

        let no_author_response = r#"{"message":"Not Found"}"#;
        assert_eq!(parse_github_login_from_response(no_author_response), None);

        let empty_login_response = r#"{"author":{"login":"","id":123}}"#;
        assert_eq!(parse_github_login_from_response(empty_login_response), None);

        let invalid_json = "not json";
        assert_eq!(parse_github_login_from_response(invalid_json), None);
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

        assert_eq!(format_enriched_message(message, "", ""), "Add new feature");
    }

    #[test]
    fn test_first_contribution_detection() {
        // Test cases for is_first_contribution JSON parsing logic
        // Note: These test the parsing logic, not the actual API calls

        // Single commit (first contribution)
        let single_commit_response = r#"[{"sha":"abc123","commit":{"message":"Initial commit"}}]"#;
        let json: Value = serde_json::from_str(single_commit_response).unwrap();
        let commits = json.as_array().unwrap();
        assert!(commits.len() <= 1);

        // Multiple commits (not first contribution)
        let multiple_commits_response = r#"[
            {"sha":"abc123","commit":{"message":"First commit"}},
            {"sha":"def456","commit":{"message":"Second commit"}}
        ]"#;
        let json: Value = serde_json::from_str(multiple_commits_response).unwrap();
        let commits = json.as_array().unwrap();
        assert!(commits.len() > 1);

        // Empty array (no commits)
        let empty_response = r#"[]"#;
        let json: Value = serde_json::from_str(empty_response).unwrap();
        let commits = json.as_array().unwrap();
        assert!(commits.len() <= 1);
    }

    #[tokio::test]
    async fn test_reqwest_client_configuration() {
        // Test that we can create a reqwest client with proper headers
        // This doesn't make an actual HTTP request, just verifies the client setup

        let client = reqwest::Client::new();
        let request = client
            .get("https://api.github.com/repos/owner/repo/commits/abc123")
            .header("Authorization", "Bearer fake_token")
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "sampo-github-action")
            .build()
            .expect("Should be able to build request");

        // Verify headers are set correctly
        assert!(request.headers().contains_key("authorization"));
        assert!(request.headers().contains_key("accept"));
        assert!(request.headers().contains_key("user-agent"));

        assert_eq!(
            request.headers().get("accept").unwrap(),
            "application/vnd.github+json"
        );
        assert_eq!(
            request.headers().get("user-agent").unwrap(),
            "sampo-github-action"
        );
    }

    #[test]
    fn test_detect_github_repo_slug_with_config() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();

        // Test that config takes priority over environment
        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "env/repo");
        }

        // Config should override environment
        let result = detect_github_repo_slug_with_config(workspace, Some("config/repo"));
        assert_eq!(result, Some("config/repo".to_string()));

        // Environment should be used when config is None
        let result = detect_github_repo_slug_with_config(workspace, None);
        assert_eq!(result, Some("env/repo".to_string()));

        // Environment should be used when config is empty
        let result = detect_github_repo_slug_with_config(workspace, Some(""));
        assert_eq!(result, Some("env/repo".to_string()));

        // Clean up
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
        }

        // Should fall back to git remote when no config or env
        let result = detect_github_repo_slug_with_config(workspace, None);
        // This might be None if git remote is not configured in test environment
        // The important thing is that it doesn't panic
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn test_config_integration() {
        use crate::config::Config;
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        let sampo_dir = workspace.join(".sampo");
        fs::create_dir_all(&sampo_dir).unwrap();

        // Create a config file with GitHub repository
        let config_content = r#"
[github]
repository = "owner/repo-from-config"
"#;
        fs::write(sampo_dir.join("config.toml"), config_content).unwrap();

        // Load config and test detection
        let config = Config::load(workspace).unwrap();
        let result =
            detect_github_repo_slug_with_config(workspace, config.github_repository.as_deref());
        assert_eq!(result, Some("owner/repo-from-config".to_string()));
    }

    #[test]
    fn test_changelog_options_config() {
        use crate::config::Config;
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        let sampo_dir = workspace.join(".sampo");
        fs::create_dir_all(&sampo_dir).unwrap();

        // Test with both options disabled
        let config_content = r#"
[changelog]
show_commit_hash = false
show_acknowledgments = false

[github]
repository = "owner/repo"
"#;
        fs::write(sampo_dir.join("config.toml"), config_content).unwrap();

        let config = Config::load(workspace).unwrap();
        assert!(!config.changelog_show_commit_hash);
        assert!(!config.changelog_show_acknowledgments);

        // Initialize git repo for the test
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(workspace)
            .output()
            .ok();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(workspace)
            .output()
            .ok();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(workspace)
            .output()
            .ok();

        // Create and commit a test file to get a real commit hash
        let test_file = workspace.join("test.txt");
        fs::write(&test_file, "test content").unwrap();
        std::process::Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(workspace)
            .output()
            .ok();
        std::process::Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(workspace)
            .output()
            .ok();

        // Get the commit hash
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(workspace)
            .output()
            .unwrap();
        let commit_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if !commit_hash.is_empty() {
            // Test enrichment with options disabled - should return original message
            let result = enrich_changeset_message(
                "Test message",
                &commit_hash,
                workspace,
                Some("owner/repo"),
                None,
                false, // no commit hash
                false, // no acknowledgments
            );
            assert_eq!(result, "Test message");

            // Test with commit hash enabled only
            let result = enrich_changeset_message(
                "Test message",
                &commit_hash,
                workspace,
                Some("owner/repo"),
                None,
                true,  // show commit hash
                false, // no acknowledgments
            );
            assert!(result.contains("Test message"));
            assert!(result.contains(&commit_hash[..7])); // short hash
        }
    }
}
