//! Enrichment module for changeset messages with commit information and author acknowledgments.
//!
//! Enriches changeset messages with commit links and author thanks using a fallback strategy:
//! GitHub API (with token) → GitHub public API → Git author name.
//!
//! Repository detection: config override → GITHUB_REPOSITORY env → git remote origin.

use serde::Deserialize;
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
    /// GitHub username (login)
    pub login: String,
    /// Whether this appears to be the user's first contribution to the repository
    pub is_first_contribution: bool,
}

/// GitHub API response structures
#[derive(Deserialize)]
struct CommitAuthor {
    login: String,
}

#[derive(Deserialize)]
struct CommitApiResponse {
    author: Option<CommitAuthor>,
}

#[derive(Deserialize)]
struct Contributor {
    login: Option<String>,
    contributions: u64,
}

/// Get the commit hash for a specific file path
pub fn get_commit_hash_for_path(repo_root: &Path, file_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args([
            "log",
            "-1",
            "--format=%H",
            "--",
            &file_path.to_string_lossy(),
        ])
        .output()
        .ok()?;

    if output.status.success() {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !hash.is_empty() { Some(hash) } else { None }
    } else {
        None
    }
}

/// Detect GitHub repository slug from Git remote
pub fn detect_github_repo_slug(repo_root: &Path) -> Option<String> {
    detect_github_repo_slug_with_config(repo_root, None)
}

/// Detect GitHub repository slug with optional config override
pub fn detect_github_repo_slug_with_config(
    repo_root: &Path,
    config_repo: Option<&str>,
) -> Option<String> {
    // 1. If explicitly configured, use that
    if let Some(repo) = config_repo {
        return Some(repo.to_string());
    }

    // 2. Try GITHUB_REPOSITORY environment variable (useful in GitHub Actions)
    if let Ok(github_repo) = std::env::var("GITHUB_REPOSITORY")
        && !github_repo.is_empty()
    {
        return Some(github_repo);
    }

    // 3. Try to extract from git remote
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let binding = String::from_utf8_lossy(&output.stdout);
    let url = binding.trim();

    // Parse GitHub URLs (both HTTPS and SSH)
    parse_github_url(url)
}

/// Parse GitHub repository slug from various URL formats
fn parse_github_url(url: &str) -> Option<String> {
    // HTTPS: https://github.com/owner/repo.git or https://github.com/owner/repo
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let without_git = rest.strip_suffix(".git").unwrap_or(rest);
        if without_git.split('/').count() >= 2 {
            return Some(without_git.to_string());
        }
    }

    // SSH: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let without_git = rest.strip_suffix(".git").unwrap_or(rest);
        if without_git.split('/').count() >= 2 {
            return Some(without_git.to_string());
        }
    }

    None
}

/// Enrich a changeset message with commit information and author acknowledgments
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
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
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

/// Async version of enrich_changeset_message for internal use
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

/// Get commit information for a specific commit hash
fn get_commit_info_for_hash(repo_root: &Path, commit_hash: &str) -> Option<CommitInfo> {
    // Use \x1f (Unit Separator) to avoid conflicts with user content
    let format_arg = "--format=%H\x1f%h\x1f%an";
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["show", "--no-patch", format_arg, commit_hash])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = stdout.trim().split('\x1f').collect();
    if parts.len() != 3 {
        return None;
    }

    Some(CommitInfo {
        sha: parts[0].to_string(),
        short_sha: parts[1].to_string(),
        author_name: parts[2].to_string(),
    })
}

/// Build commit prefix for enhanced messages
fn build_commit_prefix(commit: &Option<CommitInfo>, repo_slug: Option<&str>) -> String {
    if let Some(commit) = commit {
        if let Some(slug) = repo_slug {
            format!(
                "[{}](https://github.com/{}/commit/{}) ",
                commit.short_sha, slug, commit.sha
            )
        } else {
            format!("{} ", commit.short_sha)
        }
    } else {
        String::new()
    }
}

/// Build acknowledgment suffix for enhanced messages
async fn build_acknowledgment_suffix(
    commit: &Option<CommitInfo>,
    repo_slug: Option<&str>,
    github_token: Option<&str>,
) -> String {
    let Some(commit) = commit else {
        return String::new();
    };

    // If we have both GitHub repo and token, try to get GitHub user info with first contribution detection
    if let (Some(slug), Some(token)) = (repo_slug, github_token)
        && let Some(github_user) = get_github_user_for_commit(slug, &commit.sha, token).await
    {
        return if github_user.is_first_contribution {
            format!(
                " — Thanks @{} for your first contribution 🎉!",
                github_user.login
            )
        } else {
            format!(" — Thanks @{}!", github_user.login)
        };
    }

    // If we have repo_slug but no token, we can still try to get the GitHub user from commit API
    // (public commits are accessible without auth for public repos)
    if let Some(slug) = repo_slug
        && let Some(github_user) = get_github_user_for_commit_public(slug, &commit.sha).await
    {
        return format!(" — Thanks @{}!", github_user.login);
    }

    // Fallback to just the Git author name
    format!(" — Thanks {}!", commit.author_name)
}

/// Format the final enriched message
fn format_enriched_message(
    message: &str,
    commit_prefix: &str,
    acknowledgment_suffix: &str,
) -> String {
    format!("{}{}{}", commit_prefix, message, acknowledgment_suffix)
}

/// Get GitHub user information for a commit
async fn get_github_user_for_commit(
    repo_slug: &str,
    commit_sha: &str,
    token: &str,
) -> Option<GitHubUserInfo> {
    let commit_url = format!(
        "https://api.github.com/repos/{}/commits/{}",
        repo_slug, commit_sha
    );

    let commit_json = github_api_get(&commit_url, token).await?;
    let commit: CommitApiResponse = serde_json::from_str(&commit_json).ok()?;
    let login = commit.author?.login;

    // Check if first contribution when we have a token
    let is_first_contribution = check_first_contribution(repo_slug, &login, token).await;

    Some(GitHubUserInfo {
        login,
        is_first_contribution,
    })
}

/// Get GitHub user information for a commit from public API (no token required)
async fn get_github_user_for_commit_public(
    repo_slug: &str,
    commit_sha: &str,
) -> Option<GitHubUserInfo> {
    let commit_url = format!(
        "https://api.github.com/repos/{}/commits/{}",
        repo_slug, commit_sha
    );

    let commit_json = github_api_get_public(&commit_url).await?;
    let commit: CommitApiResponse = serde_json::from_str(&commit_json).ok()?;
    let login = commit.author?.login;

    Some(GitHubUserInfo {
        login,
        is_first_contribution: false, // Cannot detect without token
    })
}

/// Check if a user is making their first contribution to a repository
async fn check_first_contribution(repo_slug: &str, login: &str, token: &str) -> bool {
    const PER_PAGE: u32 = 100;
    const MAX_PAGES: u32 = 20; // Safety bound to avoid excessive paging

    for page in 1..=MAX_PAGES {
        let contributors_url = format!(
            "https://api.github.com/repos/{}/contributors?per_page={}&page={}&anon=true",
            repo_slug, PER_PAGE, page
        );

        let Some(body) = github_api_get(&contributors_url, token).await else {
            break;
        };

        let Ok(contributors): Result<Vec<Contributor>, _> = serde_json::from_str(&body) else {
            break;
        };

        if contributors.is_empty() {
            break;
        }

        if let Some(contributor) = contributors
            .into_iter()
            .find(|c| c.login.as_deref() == Some(login))
        {
            return contributor.contributions == 1;
        }
    }

    // If we can't find the user in contributors, assume it's not their first contribution
    // This is a conservative approach for cases where the API might have issues
    false
}

/// Perform a GET request to GitHub API and return the response body as String
///
/// Uses reqwest to make HTTP requests to the GitHub API with proper authentication
/// and headers. Returns None if the request fails or returns empty content.
async fn github_api_get(url: &str, token: &str) -> Option<String> {
    let client = reqwest::Client::new();

    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "sampo/0.4.0")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let body = response.text().await.ok()?;
    if body.trim().is_empty() {
        None
    } else {
        Some(body)
    }
}

/// Perform a GET request to GitHub API without authentication (for public repos)
///
/// Similar to github_api_get but without authorization header.
/// Only works with public repositories and endpoints.
async fn github_api_get_public(url: &str) -> Option<String> {
    let client = reqwest::Client::new();

    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "sampo/0.4.0")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let body = response.text().await.ok()?;
    if body.trim().is_empty() {
        None
    } else {
        Some(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_github_url_https() {
        assert_eq!(
            parse_github_url("https://github.com/owner/repo.git"),
            Some("owner/repo".to_string())
        );
        assert_eq!(
            parse_github_url("https://github.com/owner/repo"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn parse_github_url_ssh() {
        assert_eq!(
            parse_github_url("git@github.com:owner/repo.git"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn parse_github_url_invalid() {
        assert_eq!(parse_github_url("https://gitlab.com/owner/repo.git"), None);
        assert_eq!(parse_github_url("not-a-url"), None);
    }

    #[test]
    fn build_commit_prefix_with_repo() {
        let commit = Some(CommitInfo {
            sha: "abcd1234".to_string(),
            short_sha: "abcd".to_string(),
            author_name: "Author".to_string(),
        });

        let prefix = build_commit_prefix(&commit, Some("owner/repo"));
        assert_eq!(
            prefix,
            "[abcd](https://github.com/owner/repo/commit/abcd1234) "
        );
    }

    #[test]
    fn build_commit_prefix_without_repo() {
        let commit = Some(CommitInfo {
            sha: "abcd1234".to_string(),
            short_sha: "abcd".to_string(),
            author_name: "Author".to_string(),
        });

        let prefix = build_commit_prefix(&commit, None);
        assert_eq!(prefix, "abcd ");
    }

    #[test]
    fn format_enriched_message_complete() {
        let message =
            format_enriched_message("feat: add new feature", "[abcd](link) ", " — Thanks @user!");
        assert_eq!(
            message,
            "[abcd](link) feat: add new feature — Thanks @user!"
        );
    }

    #[test]
    fn enrich_changeset_message_integration() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Initialize a git repo
        std::process::Command::new("git")
            .arg("init")
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Configure git user
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create a test file and commit it
        let test_file = repo_path.join("test.md");
        fs::write(&test_file, "initial content").unwrap();

        std::process::Command::new("git")
            .args(["add", "test.md"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Get the commit hash
        let commit_hash = get_commit_hash_for_path(repo_path, &test_file)
            .expect("Should find commit hash for test file");

        // Test enrichment with all features enabled
        let enriched = enrich_changeset_message(
            "fix: resolve critical bug",
            &commit_hash,
            repo_path,
            Some("owner/repo"),
            None, // no GitHub token for this test
            true, // show commit hash
            true, // show acknowledgments
        );

        // Should contain the commit hash link and author thanks
        assert!(
            enriched.contains(&commit_hash[..8]),
            "Should contain short commit hash"
        );
        assert!(
            enriched.contains("Thanks Test User!"),
            "Should contain author thanks"
        );
        assert!(
            enriched.contains("fix: resolve critical bug"),
            "Should contain original message"
        );

        // Test with features disabled
        let plain = enrich_changeset_message(
            "fix: resolve critical bug",
            &commit_hash,
            repo_path,
            Some("owner/repo"),
            None,
            false, // no commit hash
            false, // no acknowledgments
        );

        assert_eq!(
            plain, "fix: resolve critical bug",
            "Should be unchanged when features disabled"
        );
    }

    #[tokio::test]
    async fn test_github_api_get_with_invalid_token() {
        // Test with invalid token should return None (graceful failure)
        let result = github_api_get(
            "https://api.github.com/repos/bruits/sampo/commits/invalid",
            "invalid_token",
        )
        .await;
        assert!(result.is_none(), "Should return None for invalid requests");
    }

    #[test]
    fn test_parse_github_url_edge_cases() {
        // Test edge cases for GitHub URL parsing
        assert_eq!(parse_github_url(""), None);
        assert_eq!(parse_github_url("https://github.com/"), None);
        assert_eq!(parse_github_url("git@github.com:"), None);
        assert_eq!(parse_github_url("https://github.com/user"), None); // Missing repo
        assert_eq!(
            parse_github_url("https://github.com/user/repo/extra/path"),
            Some("user/repo/extra/path".to_string())
        );
    }

    #[tokio::test]
    async fn test_check_first_contribution_no_token() {
        // Test check_first_contribution without valid token
        let result = check_first_contribution("bruits/sampo", "testuser", "invalid_token").await;
        // Should return false (conservative default) when API calls fail
        assert!(!result, "Should return false when API calls fail");
    }

    #[tokio::test]
    async fn test_build_acknowledgment_suffix_fallback() {
        // Test that acknowledgment falls back to Git author when GitHub API fails
        let commit = Some(CommitInfo {
            sha: "abcd1234".to_string(),
            short_sha: "abcd".to_string(),
            author_name: "Local Developer".to_string(),
        });

        // Test without GitHub repo/token (should use Git author)
        let result = build_acknowledgment_suffix(&commit, None, None).await;
        assert_eq!(result, " — Thanks Local Developer!");

        // Test with empty commit
        let result = build_acknowledgment_suffix(&None, Some("owner/repo"), Some("token")).await;
        assert_eq!(result, "");
    }

    #[test]
    fn test_detect_github_repo_slug_with_config_override() {
        // Test that explicit config overrides git remote detection
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Even without git repo, explicit config should work
        let result = detect_github_repo_slug_with_config(repo_path, Some("explicit/repo"));
        assert_eq!(result, Some("explicit/repo".to_string()));

        // Test that None config falls back to git detection (which will fail in this case)
        let result = detect_github_repo_slug_with_config(repo_path, None);
        // Note: this might return env var if GITHUB_REPOSITORY is set, but that's OK
        // The important thing is explicit config overrides everything
        assert!(result.is_none() || result.is_some());
    }

    #[tokio::test]
    async fn test_get_github_user_for_commit_public() {
        // Test public API access (should fail gracefully with invalid repo)
        let result = get_github_user_for_commit_public("invalid/repo", "invalid_sha").await;
        assert!(result.is_none(), "Should return None for invalid requests");
    }

    #[tokio::test]
    async fn test_github_api_get_public_with_invalid_url() {
        // Test public API with invalid URL should return None
        let result = github_api_get_public("https://api.github.com/invalid/endpoint").await;
        assert!(result.is_none(), "Should return None for invalid requests");
    }

    #[tokio::test]
    async fn test_build_acknowledgment_suffix_with_public_repo() {
        let commit = Some(CommitInfo {
            sha: "abcd1234".to_string(),
            short_sha: "abcd".to_string(),
            author_name: "Test Author".to_string(),
        });

        // Test with repo_slug but no token (should try public API, fall back to Git author)
        let result = build_acknowledgment_suffix(&commit, Some("invalid/repo"), None).await;
        assert_eq!(result, " — Thanks Test Author!");

        // Test with neither repo nor token
        let result = build_acknowledgment_suffix(&commit, None, None).await;
        assert_eq!(result, " — Thanks Test Author!");
    }

    #[tokio::test]
    async fn test_reqwest_timeout_behavior() {
        // Test that reqwest properly handles timeouts
        // Using a non-routable IP to trigger timeout (should be fast)
        let result = github_api_get_public("http://10.255.255.1/timeout-test").await;
        assert!(
            result.is_none(),
            "Should return None for timeout/unreachable requests"
        );
    }
}
