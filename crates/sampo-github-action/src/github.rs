use crate::error::{ActionError, Result};
use reqwest::Url;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Serialize)]
struct CreatePullRequestPayload {
    title: String,
    head: String,
    base: String,
    body: String,
}

#[derive(Debug, Serialize)]
struct UpdatePullRequestPayload {
    title: String,
    body: String,
}

#[derive(Debug, Serialize)]
struct CreateReleasePayload {
    tag_name: String,
    name: String,
    body: String,
    draft: bool,
    prerelease: bool,
}

#[derive(Debug, Serialize)]
struct CreateDiscussionPayload {
    title: String,
    body: String,
    category_id: u64,
}

#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u64,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    #[serde(rename = "upload_url")]
    upload_url: String,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct DiscussionCategory {
    id: u64,
    slug: String,
}

pub struct GitHubClient {
    client: Client,
    repo: String,
    token: String,
}

impl GitHubClient {
    pub fn new(repo: String, token: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(format!("sampo-github-action/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "http-client-init".to_string(),
                message: format!("Failed to create HTTP client: {}", e),
            })?;

        Ok(Self {
            client,
            repo,
            token,
        })
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// Create or update a GitHub Pull Request
    pub fn ensure_pull_request(
        &self,
        head_branch: &str,
        base_branch: &str,
        title: &str,
        body: &str,
    ) -> Result<()> {
        let api_url = format!("https://api.github.com/repos/{}/pulls", self.repo);

        println!("Creating/updating PR: {} <- {}", base_branch, head_branch);

        let payload = CreatePullRequestPayload {
            title: title.to_string(),
            head: head_branch.to_string(),
            base: base_branch.to_string(),
            body: body.to_string(),
        };

        // Try to create the PR
        let response = self
            .client
            .post(&api_url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&payload)
            .send()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "github-create-pr".to_string(),
                message: format!("HTTP request to {} failed: {}", api_url, e),
            })?;

        if response.status().is_success() {
            let pr: PullRequest = response
                .json()
                .map_err(|e| ActionError::SampoCommandFailed {
                    operation: "github-create-pr".to_string(),
                    message: format!("Failed to parse successful PR response: {}", e),
                })?;

            println!("PR created successfully: {}", pr.html_url);
            return Ok(());
        }

        // Handle error responses
        let status = response.status();
        let error_text = response
            .text()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "github-create-pr".to_string(),
                message: format!("Failed to read error response body: {}", e),
            })?;

        // Check if PR already exists
        if status == 422 && error_text.contains("A pull request already exists") {
            println!("PR already exists, attempting to update...");
            return self.find_and_update_existing_pr(head_branch, title, body);
        }

        // Return the GitHub API error with context
        Err(ActionError::SampoCommandFailed {
            operation: "github-create-pr".to_string(),
            message: format!(
                "GitHub API error for {}: {} (status {})",
                self.repo, error_text, status
            ),
        })
    }

    fn find_and_update_existing_pr(
        &self,
        head_branch: &str,
        title: &str,
        body: &str,
    ) -> Result<()> {
        let owner = self.repo.split('/').next().unwrap_or("");
        let list_url = format!(
            "https://api.github.com/repos/{}/pulls?state=open&head={}:{}",
            self.repo, owner, head_branch
        );

        let response = self
            .client
            .get(&list_url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "github-list-prs".to_string(),
                message: format!("HTTP request to {} failed: {}", list_url, e),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().unwrap_or_default();
            return Err(ActionError::SampoCommandFailed {
                operation: "github-list-prs".to_string(),
                message: format!("Failed to list PRs ({}): {}", status, error_text),
            });
        }

        let prs: Vec<PullRequest> =
            response
                .json()
                .map_err(|e| ActionError::SampoCommandFailed {
                    operation: "github-list-prs".to_string(),
                    message: format!("Failed to parse PR list response: {}", e),
                })?;

        if let Some(pr) = prs.first() {
            println!("Found existing PR #{}, updating...", pr.number);
            self.update_pull_request(pr.number, title, body)
        } else {
            println!("No open PR found for {}:{}", self.repo, head_branch);
            Ok(())
        }
    }

    /// Update an existing Pull Request
    fn update_pull_request(&self, pr_number: u64, title: &str, body: &str) -> Result<()> {
        let api_url = format!(
            "https://api.github.com/repos/{}/pulls/{}",
            self.repo, pr_number
        );

        let payload = UpdatePullRequestPayload {
            title: title.to_string(),
            body: body.to_string(),
        };

        let response = self
            .client
            .patch(&api_url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&payload)
            .send()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "github-update-pr".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        if response.status().is_success() {
            let pr: PullRequest = response
                .json()
                .map_err(|e| ActionError::SampoCommandFailed {
                    operation: "github-update-pr".to_string(),
                    message: format!("Failed to parse PR response: {}", e),
                })?;

            println!("PR updated successfully: {}", pr.html_url);
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().unwrap_or_default();
            Err(ActionError::SampoCommandFailed {
                operation: "github-update-pr".to_string(),
                message: format!(
                    "Failed to update PR #{} ({}): {}",
                    pr_number, status, error_text
                ),
            })
        }
    }

    /// Create a GitHub Release
    pub fn create_release(&self, tag: &str, body: &str, prerelease: bool) -> Result<String> {
        let api_url = format!("https://api.github.com/repos/{}/releases", self.repo);

        let payload = CreateReleasePayload {
            tag_name: tag.to_string(),
            name: tag.to_string(),
            body: body.to_string(),
            draft: false,
            prerelease,
        };

        let response = self
            .client
            .post(&api_url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&payload)
            .send()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "github-create-release".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        if response.status().is_success() {
            let release: Release =
                response
                    .json()
                    .map_err(|e| ActionError::SampoCommandFailed {
                        operation: "github-create-release".to_string(),
                        message: format!("Failed to parse release response: {}", e),
                    })?;

            println!("Created GitHub release for {}: {}", tag, release.html_url);

            // Return the upload URL without template parameters
            let upload_url = release
                .upload_url
                .split('{')
                .next()
                .unwrap_or("")
                .to_string();
            Ok(upload_url)
        } else {
            let status = response.status();
            let error_text = response.text().unwrap_or_default();
            Err(ActionError::SampoCommandFailed {
                operation: "github-create-release".to_string(),
                message: format!("Failed to create release ({}): {}", status, error_text),
            })
        }
    }

    /// Get discussion categories for the repository
    fn get_discussion_categories(&self) -> Result<Vec<DiscussionCategory>> {
        let api_url = format!(
            "https://api.github.com/repos/{}/discussions/categories",
            self.repo
        );

        let response = self
            .client
            .get(&api_url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "github-list-discussion-categories".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        if response.status().is_success() {
            let categories: Vec<DiscussionCategory> =
                response
                    .json()
                    .map_err(|e| ActionError::SampoCommandFailed {
                        operation: "github-list-discussion-categories".to_string(),
                        message: format!("Failed to parse categories response: {}", e),
                    })?;
            Ok(categories)
        } else {
            let status = response.status();
            let error_text = response.text().unwrap_or_default();
            // Provide actionable context for common failure modes
            let hint = if status.as_u16() == 404 {
                "Hint: Discussions may be disabled on this repository, or the token lacks discussions permissions. Enable Discussions in Settings and grant `permissions: discussions: write` in the workflow."
            } else if status.as_u16() == 410 {
                "Hint: Discussions are disabled for this repository. Enable Discussions in Settings > Features."
            } else if status.as_u16() == 403 {
                "Hint: Missing permissions. Grant `permissions: discussions: write` in the workflow."
            } else {
                ""
            };
            Err(ActionError::SampoCommandFailed {
                operation: "github-list-discussion-categories".to_string(),
                message: format!(
                    "Failed to get categories ({}): {}{}{}",
                    status,
                    error_text,
                    if hint.is_empty() { "" } else { " — " },
                    hint
                ),
            })
        }
    }

    /// Create a GitHub Discussion
    pub fn create_discussion(
        &self,
        tag: &str,
        body: &str,
        preferred_category: Option<&str>,
    ) -> Result<()> {
        let categories = self.get_discussion_categories()?;

        let desired_slug = preferred_category
            .and_then(|s| if s.trim().is_empty() { None } else { Some(s) })
            .unwrap_or("announcements");

        // Find category by slug, with fallbacks
        let category_id = categories
            .iter()
            .find(|cat| cat.slug == desired_slug)
            .or_else(|| categories.iter().find(|cat| cat.slug == "announcements"))
            .or_else(|| categories.first())
            .map(|cat| cat.id)
            .ok_or_else(|| ActionError::SampoCommandFailed {
                operation: "github-find-discussion-category".to_string(),
                message: "No discussion categories available".into(),
            })?;

        let api_url = format!("https://api.github.com/repos/{}/discussions", self.repo);
        let title = format!("Release {}", tag);
        let body_with_link = format!(
            "{}\n\n—\nSee release page: https://github.com/{}/releases/tag/{}",
            body, self.repo, tag
        );

        let payload = CreateDiscussionPayload {
            title,
            body: body_with_link,
            category_id,
        };

        let response = self
            .client
            .post(&api_url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&payload)
            .send()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "github-create-discussion".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        if response.status().is_success() {
            println!("Opened GitHub Discussion for {}", tag);
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().unwrap_or_default();
            let hint = if status.as_u16() == 404 {
                "Hint: Discussions may be disabled on this repository, or the token lacks discussions permissions. Enable Discussions in Settings and grant `permissions: discussions: write` in the workflow."
            } else if status.as_u16() == 410 {
                "Hint: Discussions are disabled for this repository. Enable Discussions in Settings > Features."
            } else if status.as_u16() == 403 {
                "Hint: Missing permissions. Grant `permissions: discussions: write` in the workflow."
            } else {
                ""
            };
            Err(ActionError::SampoCommandFailed {
                operation: "github-create-discussion".to_string(),
                message: format!(
                    "Failed to create discussion ({}): {}{}{}",
                    status,
                    error_text,
                    if hint.is_empty() { "" } else { " — " },
                    hint
                ),
            })
        }
    }

    /// Upload an existing file as a release asset
    pub fn upload_release_asset(
        &self,
        upload_url: &str,
        asset_path: &Path,
        asset_name: &str,
    ) -> Result<()> {
        if !asset_path.is_file() {
            return Err(ActionError::SampoCommandFailed {
                operation: "release-asset-upload".to_string(),
                message: format!(
                    "Release asset not found or not a file: {}",
                    asset_path.display()
                ),
            });
        }

        let asset_bytes = std::fs::read(asset_path).map_err(ActionError::Io)?;

        let mut url = Url::parse(upload_url).map_err(|e| ActionError::SampoCommandFailed {
            operation: "release-asset-upload".to_string(),
            message: format!("Invalid upload URL '{}': {}", upload_url, e),
        })?;
        url.query_pairs_mut().append_pair("name", asset_name);

        let response = self
            .client
            .post(url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("Content-Type", "application/octet-stream")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .body(asset_bytes)
            .send()
            .map_err(|e| ActionError::SampoCommandFailed {
                operation: "release-asset-upload".to_string(),
                message: format!("HTTP request failed: {}", e),
            })?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().unwrap_or_default();
            Err(ActionError::SampoCommandFailed {
                operation: "release-asset-upload".to_string(),
                message: format!("Failed to upload asset ({}): {}", status, error_text),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_serialization() {
        let pr_payload = CreatePullRequestPayload {
            title: "Test PR with \"quotes\" and \n newlines".to_string(),
            head: "feature-branch".to_string(),
            base: "main".to_string(),
            body: "This is a test body\nwith multiple lines\nand \"quoted\" text.".to_string(),
        };

        let json =
            serde_json::to_string(&pr_payload).expect("PR payload should serialize to valid JSON");
        // Verify that serde_json properly escapes the content
        assert!(json.contains("Test PR with \\\"quotes\\\" and \\n newlines"));
        assert!(json.contains("with multiple lines\\nand \\\"quoted\\\" text"));

        let release_payload = CreateReleasePayload {
            tag_name: "v1.0.0".to_string(),
            name: "v1.0.0".to_string(),
            body: "Release notes with\nmultiple lines".to_string(),
            draft: false,
            prerelease: true,
        };

        let json = serde_json::to_string(&release_payload)
            .expect("Release payload should serialize to valid JSON");
        assert!(json.contains("v1.0.0"));
        assert!(json.contains("Release notes with\\nmultiple lines"));
        assert!(json.contains("\"prerelease\":true"));
    }

    #[test]
    fn test_github_client_creation() {
        let result = GitHubClient::new("owner/repo".to_string(), "token".to_string());
        assert!(result.is_ok(), "GitHub client creation should succeed");

        let client = result.expect("Client should be created successfully");
        assert_eq!(client.repo, "owner/repo");
        assert_eq!(client.token, "token");
    }
}
