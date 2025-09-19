use crate::errors::{Result, SampoError};
use std::process::Command;

fn read_env_branch(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Detect the current git branch, preferring explicit overrides when provided.
///
/// Order of precedence:
/// 1. `SAMPO_RELEASE_BRANCH`
/// 2. `GITHUB_REF_NAME`
/// 3. `git rev-parse --abbrev-ref HEAD`
pub fn current_branch() -> Result<String> {
    if let Some(branch) = read_env_branch("SAMPO_RELEASE_BRANCH") {
        return Ok(branch);
    }

    if let Some(branch) = read_env_branch("GITHUB_REF_NAME") {
        return Ok(branch);
    }

    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(SampoError::Io)?;

    if !output.status.success() {
        return Err(SampoError::Release(
            "Unable to determine current git branch (git rev-parse failed)".into(),
        ));
    }

    let branch = String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_start_matches("refs/heads/")
        .to_string();

    if branch.is_empty() || branch == "HEAD" {
        return Err(SampoError::Release(
            "Unable to determine current git branch (detached HEAD)".into(),
        ));
    }

    Ok(branch)
}
