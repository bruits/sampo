use crate::error::{ActionError, Result};
use std::path::Path;
use std::process::Command;

/// Execute git commands with error handling
pub fn git(args: &[&str], cwd: Option<&Path>) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let status = cmd.status().map_err(ActionError::Io)?;

    if !status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "git".to_string(),
            message: format!("git {} failed with status {}", args.join(" "), status),
        });
    }

    Ok(())
}

/// Check if the git repository has uncommitted changes
pub fn has_changes(cwd: &Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output()
        .map_err(ActionError::Io)?;

    if !output.status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "git-status".to_string(),
            message: format!("git status failed: {}", output.status),
        });
    }

    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

/// Setup git user for automated commits
pub fn setup_bot_user(workspace: &Path) -> Result<()> {
    git(
        &[
            "config",
            "user.email",
            "github-actions[bot]@users.noreply.github.com",
        ],
        Some(workspace),
    )?;
    git(
        &["config", "user.name", "github-actions[bot]"],
        Some(workspace),
    )?;
    Ok(())
}
