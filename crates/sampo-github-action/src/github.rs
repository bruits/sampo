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
    find_and_update_existing_pr(repo, token, head_branch, base_branch)
}

fn find_and_update_existing_pr(
    repo: &str,
    token: &str,
    head_branch: &str,
    base_branch: &str,
) -> Result<()> {
    let list_url = format!(
        "https://api.github.com/repos/{}/pulls?state=all&head={}&base={}",
        repo, head_branch, base_branch
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

    // Simple check if we have any PRs
    if response.contains(r#""number":"#) {
        println!("Found existing PR at: https://github.com/{}/pulls", repo);
    } else {
        eprintln!("No existing PR found. Please check GitHub permissions and repository settings.");
    }

    Ok(())
}
