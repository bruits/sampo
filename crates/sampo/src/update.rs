//! Self-update functionality for Sampo CLI.

use crate::cli::UpdateArgs;
use crate::ui::{log_info, log_success_value, log_warning, prompt_theme};
use crate::version_check::CURRENT_VERSION;
use sampo_core::errors::{Result, SampoError};
use self_update::backends::github::{ReleaseList, Update};
use self_update::update::Release;
use semver::Version;

const REPO_OWNER: &str = "bruits";
const REPO_NAME: &str = "sampo";
const BIN_NAME: &str = "sampo";
const TAG_PREFIX: &str = "sampo-v";

/// Runs the update command.
pub fn run(args: &UpdateArgs) -> Result<()> {
    log_info("Checking for updates...");

    let releases = fetch_sampo_releases()?;
    let latest = find_latest_sampo_release(&releases)?;
    let latest_version = parse_version_from_tag(&latest.name)?;
    let current_version = Version::parse(CURRENT_VERSION)
        .map_err(|e| SampoError::InvalidData(format!("Invalid current version: {e}")))?;

    if latest_version <= current_version {
        log_success_value("Already up to date", &current_version.to_string());
        return Ok(());
    }

    log_warning(&format!(
        "New version available: {} → {}",
        current_version, latest_version
    ));

    if !args.yes && !confirm_update()? {
        log_info("Update cancelled.");
        return Ok(());
    }

    log_info("Downloading and installing...");
    perform_update(&latest.name)?;

    log_success_value("Updated to version", &latest_version.to_string());
    Ok(())
}

/// Fetches all releases from the GitHub repository.
fn fetch_sampo_releases() -> Result<Vec<Release>> {
    let releases = ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .map_err(|e| SampoError::GitHub(format!("Failed to configure release list: {e}")))?
        .fetch()
        .map_err(|e| SampoError::GitHub(format!("Failed to fetch releases: {e}")))?;

    Ok(releases)
}

/// Filters releases to only those matching the `sampo-v<version>` tag pattern
/// and returns the one with the highest semver version.
fn find_latest_sampo_release(releases: &[Release]) -> Result<&Release> {
    releases
        .iter()
        .filter(|r| r.name.starts_with(TAG_PREFIX))
        .filter(|r| parse_version_from_tag(&r.name).is_ok())
        .max_by(|a, b| {
            let v_a = parse_version_from_tag(&a.name).unwrap_or_else(|_| Version::new(0, 0, 0));
            let v_b = parse_version_from_tag(&b.name).unwrap_or_else(|_| Version::new(0, 0, 0));
            v_a.cmp(&v_b)
        })
        .ok_or_else(|| SampoError::NotFound("No Sampo CLI releases found on GitHub".to_string()))
}

/// Parses a semver version from a tag like `sampo-v0.13.0`.
fn parse_version_from_tag(tag: &str) -> Result<Version> {
    let version_str = tag
        .strip_prefix(TAG_PREFIX)
        .ok_or_else(|| SampoError::InvalidData(format!("Invalid tag format: {tag}")))?;

    Version::parse(version_str)
        .map_err(|e| SampoError::InvalidData(format!("Invalid version in tag '{tag}': {e}")))
}

/// Prompts the user to confirm the update.
fn confirm_update() -> Result<bool> {
    use dialoguer::Confirm;

    Confirm::with_theme(&prompt_theme())
        .with_prompt("Do you want to update?")
        .default(true)
        .interact()
        .map_err(|e| SampoError::Io(std::io::Error::other(e)))
}

/// Performs the actual update by downloading and replacing the binary.
fn perform_update(target_tag: &str) -> Result<()> {
    Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .target_version_tag(target_tag)
        .current_version(CURRENT_VERSION)
        .show_download_progress(true)
        .show_output(false)
        .no_confirm(true)
        .build()
        .map_err(|e| SampoError::GitHub(format!("Failed to configure update: {e}")))?
        .update()
        .map_err(|e| SampoError::GitHub(format!("Failed to update: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_from_valid_tag() {
        let v = parse_version_from_tag("sampo-v0.13.0").unwrap();
        assert_eq!(v, Version::new(0, 13, 0));
    }

    #[test]
    fn parse_version_from_prerelease_tag() {
        let v = parse_version_from_tag("sampo-v1.0.0-alpha.1").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 0);
        assert!(!v.pre.is_empty());
    }

    #[test]
    fn parse_version_rejects_invalid_prefix() {
        let result = parse_version_from_tag("other-v1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn parse_version_rejects_invalid_version() {
        let result = parse_version_from_tag("sampo-vnot-a-version");
        assert!(result.is_err());
    }

    #[test]
    fn find_latest_selects_highest_version() {
        let releases = vec![
            Release {
                name: "sampo-v0.12.0".to_string(),
                version: "0.12.0".to_string(),
                ..Default::default()
            },
            Release {
                name: "sampo-v0.13.0".to_string(),
                version: "0.13.0".to_string(),
                ..Default::default()
            },
            Release {
                name: "sampo-v0.11.0".to_string(),
                version: "0.11.0".to_string(),
                ..Default::default()
            },
            Release {
                name: "other-component-v1.0.0".to_string(),
                version: "1.0.0".to_string(),
                ..Default::default()
            },
        ];

        let latest = find_latest_sampo_release(&releases).unwrap();
        assert_eq!(latest.name, "sampo-v0.13.0");
    }

    #[test]
    fn find_latest_returns_error_when_no_sampo_releases() {
        let releases = vec![Release {
            name: "other-v1.0.0".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        }];

        let result = find_latest_sampo_release(&releases);
        assert!(result.is_err());
    }

    #[test]
    fn find_latest_ignores_invalid_versions() {
        let releases = vec![
            Release {
                name: "sampo-vinvalid".to_string(),
                version: "invalid".to_string(),
                ..Default::default()
            },
            Release {
                name: "sampo-v0.10.0".to_string(),
                version: "0.10.0".to_string(),
                ..Default::default()
            },
        ];

        let latest = find_latest_sampo_release(&releases).unwrap();
        assert_eq!(latest.name, "sampo-v0.10.0");
    }
}
