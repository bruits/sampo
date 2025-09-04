use crate::{ActionError, Result};
use std::process::Command;

/// Create or update a GitHub Pull Request
pub fn ensure_pull_request(
    repo: &str,
    token: &str,
    head_branch: &str,
    base_branch: &str,
    title: &str,
    body: &str,
) -> Result<()> {
    // For same-repo PRs, GitHub API expects head to be just the branch name
    let api_url = format!("https://api.github.com/repos/{}/pulls", repo);

    println!("Creating/updating PR: {} <- {}", base_branch, head_branch);

    // Create the payload with proper JSON escaping
    let escaped_title = title.replace('"', "\\\"");
    let escaped_body = body.replace('"', "\\\"").replace('\n', "\\n");

    let payload = format!(
        r#"{{"title":"{}","head":"{}","base":"{}","body":"{}"}}"#,
        escaped_title, head_branch, base_branch, escaped_body
    );

    // Try to create the PR
    let output = Command::new("curl")
        .args([
            "-sS",
            "-X",
            "POST",
            "-H",
            &format!("Authorization: Bearer {}", token),
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "Content-Type: application/json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            &api_url,
            "-d",
            &payload,
        ])
        .output()
        .map_err(ActionError::Io)?;

    if output.status.success() {
        let response = String::from_utf8_lossy(&output.stdout);

        // Check if the response actually contains a PR URL (successful creation)
        if response.contains("\"html_url\"") && response.contains("/pull/") {
            if let Some(start) = response.find("\"html_url\":\"") {
                let start = start + 12; // length of "html_url":"
                if let Some(end) = response[start..].find('"') {
                    let url = &response[start..start + end];
                    println!("PR created successfully: {}", url);
                    return Ok(());
                }
            }
            println!("PR created successfully");
            return Ok(());
        } else if response.contains("A pull request already exists") {
            // Handle existing PR case - find and update it
            println!("PR already exists, attempting to update...");
            return find_and_update_existing_pr(repo, token, head_branch, base_branch, title, body);
        } else if response.contains("\"errors\"") || response.contains("\"message\"") {
            // HTTP 200 but API error in response
            return Err(ActionError::SampoCommandFailed {
                operation: "github-create-pr".to_string(),
                message: format!("GitHub API error: {}", response),
            });
        } else {
            // Unclear response - fail with details
            return Err(ActionError::SampoCommandFailed {
                operation: "github-create-pr".to_string(),
                message: format!("Unexpected GitHub API response: {}", response),
            });
        }
    }

    // If creation failed, try to find and update existing PR
    let response = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    eprintln!("PR creation failed (HTTP {})", output.status);
    if !response.is_empty() {
        eprintln!("Response: {}", response);
    }
    if !stderr.is_empty() {
        eprintln!("Error: {}", stderr);
    }

    // Try to find existing PR and update it
    find_and_update_existing_pr(repo, token, head_branch, base_branch, title, body)
}

fn find_and_update_existing_pr(
    repo: &str,
    token: &str,
    head_branch: &str,
    _base_branch: &str,
    title: &str,
    body: &str,
) -> Result<()> {
    let list_url = format!(
        "https://api.github.com/repos/{}/pulls?state=open&head={}:{}",
        repo,
        repo.split('/').next().unwrap_or(""),
        head_branch
    );

    let output = Command::new("curl")
        .args([
            "-sS",
            "-H",
            &format!("Authorization: Bearer {}", token),
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            &list_url,
        ])
        .output()
        .map_err(ActionError::Io)?;

    if !output.status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "github-list-prs".to_string(),
            message: format!(
                "Failed to list PRs: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }

    let response = String::from_utf8_lossy(&output.stdout);

    // Parse PR number from the response
    if let Some(pr_number) = extract_pr_number(&response) {
        println!("Found existing PR #{}, updating...", pr_number);
        update_pull_request(repo, token, pr_number, title, body)
    } else {
        println!("No open PR found for {}:{}", repo, head_branch);
        Ok(())
    }
}

/// Extract PR number from GitHub API response
fn extract_pr_number(response: &str) -> Option<u64> {
    // Look for "number": followed by a number
    if let Some(start) = response.find("\"number\":") {
        let start = start + 9; // length of "number":
        let number_part = &response[start..];

        // Skip whitespace and find the actual number
        let trimmed = number_part.trim_start();
        let end = trimmed.find(',').or_else(|| trimmed.find('}'))?;
        let number_str = trimmed[..end].trim();

        number_str.parse().ok()
    } else {
        None
    }
}

/// Update an existing Pull Request
fn update_pull_request(
    repo: &str,
    token: &str,
    pr_number: u64,
    title: &str,
    body: &str,
) -> Result<()> {
    let api_url = format!("https://api.github.com/repos/{}/pulls/{}", repo, pr_number);

    // Create the payload with proper JSON escaping
    let escaped_title = title.replace('"', "\\\"");
    let escaped_body = body.replace('"', "\\\"").replace('\n', "\\n");

    let payload = format!(
        r#"{{"title":"{}","body":"{}"}}"#,
        escaped_title, escaped_body
    );

    let output = Command::new("curl")
        .args([
            "-sS",
            "-X",
            "PATCH",
            "-H",
            &format!("Authorization: Bearer {}", token),
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "Content-Type: application/json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            &api_url,
            "-d",
            &payload,
        ])
        .output()
        .map_err(ActionError::Io)?;

    if output.status.success() {
        let response = String::from_utf8_lossy(&output.stdout);
        if response.contains("\"html_url\"")
            && let Some(start) = response.find("\"html_url\":\"")
        {
            let start = start + 12; // length of "html_url":"
            if let Some(end) = response[start..].find('"') {
                let url = &response[start..start + end];
                println!("PR updated successfully: {}", url);
                return Ok(());
            }
        }
        println!("PR #{} updated successfully", pr_number);
        Ok(())
    } else {
        let response = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ActionError::SampoCommandFailed {
            operation: "github-update-pr".to_string(),
            message: format!(
                "Failed to update PR #{}: HTTP {} - stdout: {} stderr: {}",
                pr_number, output.status, response, stderr
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pr_number() {
        // Test with typical GitHub API response
        let response = r#"[{"number": 42, "title": "Test PR"}]"#;
        assert_eq!(extract_pr_number(response), Some(42));

        // Test with no PR
        let empty_response = "[]";
        assert_eq!(extract_pr_number(empty_response), None);

        // Test with different formatting
        let response2 = r#"[{"id": 123, "number":  17 , "state": "open"}]"#;
        assert_eq!(extract_pr_number(response2), Some(17));
    }
}
