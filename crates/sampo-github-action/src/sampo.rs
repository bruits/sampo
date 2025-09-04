use crate::{ActionError, Result};
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct ReleasePlan {
    pub has_changes: bool,
    pub description: String,
}

/// Run sampo release and capture the plan
pub fn capture_release_plan(workspace: &Path) -> Result<ReleasePlan> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("-p")
        .arg("sampo")
        .arg("--")
        .arg("release")
        .arg("--dry-run");

    let output = cmd.output().map_err(ActionError::Io)?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "release-plan".to_string(),
            message: format!("stdout: {}\nstderr: {}", stdout, stderr),
        });
    }

    let has_changes = !stdout.contains("No changesets found");

    Ok(ReleasePlan {
        has_changes,
        description: stdout,
    })
}

/// Execute sampo release
pub fn run_release(workspace: &Path, dry_run: bool, cargo_token: Option<&str>) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("-p")
        .arg("sampo")
        .arg("--")
        .arg("release");

    if dry_run {
        cmd.arg("--dry-run");
    }

    if let Some(token) = cargo_token {
        cmd.env("CARGO_REGISTRY_TOKEN", token);
    }

    let status = cmd.status().map_err(ActionError::Io)?;

    if !status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "release".to_string(),
            message: format!("sampo release failed with status {}", status),
        });
    }

    Ok(())
}

/// Execute sampo publish
pub fn run_publish(
    workspace: &Path,
    dry_run: bool,
    extra_args: Option<&str>,
    cargo_token: Option<&str>,
) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("-p")
        .arg("sampo")
        .arg("--")
        .arg("publish");

    if dry_run {
        cmd.arg("--dry-run");
    }

    // Add extra args if provided
    if let Some(args) = extra_args {
        cmd.arg("--");
        for arg in args.split_whitespace() {
            cmd.arg(arg);
        }
    }

    if let Some(token) = cargo_token {
        cmd.env("CARGO_REGISTRY_TOKEN", token);
    }

    let status = cmd.status().map_err(ActionError::Io)?;

    if !status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "publish".to_string(),
            message: format!("sampo publish failed with status {}", status),
        });
    }

    Ok(())
}
