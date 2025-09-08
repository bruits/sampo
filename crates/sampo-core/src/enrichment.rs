//! Enrichment module for changeset messages with commit information and author acknowledgments.

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
    // If explicitly configured, use that
    if let Some(repo) = config_repo {
        return Some(repo.to_string());
    }

    // Try to extract from git remote
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

    // If we have a GitHub repo and token, try to get GitHub user info
    if let (Some(slug), Some(token)) = (repo_slug, github_token)
        && let Some(github_user) = get_github_user_for_commit(slug, &commit.sha, token).await
    {
        if github_user.is_first_contribution {
            return format!(
                " — Thanks @{} for your first contribution 🎉!",
                github_user.login
            );
        } else {
            return format!(" — Thanks @{}!", github_user.login);
        }
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
    _repo_slug: &str,
    _commit_sha: &str,
    _token: &str,
) -> Option<GitHubUserInfo> {
    // This is a simplified version - in a real implementation you'd:
    // 1. Get commit info from GitHub API
    // 2. Get author's GitHub login
    // 3. Check if it's their first contribution
    // For now, we'll return None to avoid API calls in tests
    None
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
}
