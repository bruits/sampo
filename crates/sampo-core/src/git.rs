use crate::errors::{Result, SampoError};
#[cfg(test)]
use std::cell::RefCell;
use std::process::Command;

#[cfg(test)]
thread_local! {
    static BRANCH_OVERRIDE: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct BranchOverrideGuard {
    previous: Option<String>,
}

#[cfg(test)]
pub(crate) fn override_current_branch_for_tests(branch: &str) -> BranchOverrideGuard {
    let previous = BRANCH_OVERRIDE.with(|cell| cell.borrow_mut().replace(branch.to_string()));
    BranchOverrideGuard { previous }
}

#[cfg(test)]
impl Drop for BranchOverrideGuard {
    fn drop(&mut self) {
        BRANCH_OVERRIDE.with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some(prev) = self.previous.take() {
                slot.replace(prev);
            } else {
                slot.take();
            }
        });
    }
}

#[cfg(test)]
fn branch_override() -> Option<String> {
    BRANCH_OVERRIDE.with(|cell| cell.borrow().clone())
}

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
    #[cfg(test)]
    if let Some(branch) = branch_override() {
        return Ok(branch);
    }

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
