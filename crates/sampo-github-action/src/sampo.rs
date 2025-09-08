use crate::{ActionError, Result};
use sampo_core::{
    Bump, Config, build_dependency_updates, create_dependency_update_entry, detect_changesets_dir,
    detect_github_repo_slug_with_config, discover_workspace, enrich_changeset_message,
    get_commit_hash_for_path, load_changesets,
};
use std::collections::BTreeMap;
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

/// Detect automatic dependency updates for crates that are being released
///
/// This function identifies crates that have been auto-bumped due to internal
/// dependency updates and creates appropriate changelog entries for them.
fn detect_dependency_updates(
    workspace: &Path,
    releases: &BTreeMap<String, (String, String)>,
    messages_by_pkg: &mut BTreeMap<String, Vec<(String, Bump)>>,
) -> Result<()> {
    let ws = discover_workspace(workspace)
        .map_err(|e| ActionError::Io(std::io::Error::other(e.to_string())))?;
    let changesets_dir = detect_changesets_dir(workspace);
    let changesets = load_changesets(&changesets_dir).map_err(ActionError::Io)?;

    // Collect explicitly changed packages from changesets
    let mut explicitly_changed = std::collections::BTreeSet::new();
    for cs in &changesets {
        for pkg in &cs.packages {
            explicitly_changed.insert(pkg.clone());
        }
    }

    // Build new version lookup from releases
    let mut new_version_by_name = std::collections::BTreeMap::new();
    for (name, (_old, new_ver)) in releases {
        new_version_by_name.insert(name.clone(), new_ver.clone());
    }

    // Build map of crate name -> CrateInfo for quick lookup
    let mut by_name = std::collections::BTreeMap::new();
    for c in &ws.members {
        by_name.insert(c.name.clone(), c);
    }

    // For each released crate that wasn't explicitly changed,
    // check if it has internal dependencies that were updated
    for crate_name in releases.keys() {
        if explicitly_changed.contains(crate_name) {
            // Skip explicitly changed crates - they have their own changelog entries
            continue;
        }

        if let Some(crate_info) = by_name.get(crate_name) {
            // Find which internal dependencies were updated
            let mut updated_deps = Vec::new();
            for dep_name in &crate_info.internal_deps {
                if let Some(new_version) = new_version_by_name.get(dep_name) {
                    // This internal dependency was updated
                    updated_deps.push((dep_name.clone(), new_version.clone()));
                }
            }

            if !updated_deps.is_empty() {
                // Create dependency update entry
                let updates = build_dependency_updates(&updated_deps);
                if let Some((msg, bump)) = create_dependency_update_entry(&updates) {
                    messages_by_pkg
                        .entry(crate_name.clone())
                        .or_default()
                        .push((msg, bump));
                }
            }
        }
    }

    Ok(())
}

/// Compute a markdown PR body summarizing the pending release by crate,
/// grouping changes by semantic bump type, and showing old -> new versions.
///
/// This function builds the PR body using stdout from `sampo release --dry-run`
/// to infer planned crate version changes, and reads changesets for change messages.
///
/// # Arguments
/// * `workspace` - Path to the workspace root
/// * `plan_stdout` - Output from `sampo release --dry-run`
///
/// # Returns
/// A formatted markdown string for the PR body, or empty string if no releases are planned
pub fn build_release_pr_body_from_stdout(workspace: &Path, plan_stdout: &str) -> Result<String> {
    let releases = parse_planned_releases(plan_stdout);
    if releases.is_empty() {
        return Ok(String::new());
    }

    let changesets_dir = detect_changesets_dir(workspace);
    let changesets = load_changesets(&changesets_dir)?;

    // Group messages per crate by bump
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, Bump)>> = BTreeMap::new();

    // Load configuration to get GitHub repository setting
    let config = Config::load(workspace).unwrap_or(Config {
        version: 1,
        github_repository: None,
        changelog_show_commit_hash: true,
        changelog_show_acknowledgments: true,
        fixed_dependencies: Vec::new(),
        linked_dependencies: Vec::new(),
    });

    // Resolve GitHub slug and token for commit links and acknowledgments
    let repo_slug =
        detect_github_repo_slug_with_config(workspace, config.github_repository.as_deref());
    let github_token = std::env::var("GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GH_TOKEN").ok());

    for cs in &changesets {
        for pkg in &cs.packages {
            if releases.contains_key(pkg) {
                let commit_hash = get_commit_hash_for_path(workspace, &cs.path);
                let enriched = if let Some(hash) = commit_hash {
                    enrich_changeset_message(
                        &cs.message,
                        &hash,
                        workspace,
                        repo_slug.as_deref(),
                        github_token.as_deref(),
                        config.changelog_show_commit_hash,
                        config.changelog_show_acknowledgments,
                    )
                } else {
                    cs.message.clone()
                };
                messages_by_pkg
                    .entry(pkg.clone())
                    .or_default()
                    .push((enriched, cs.bump));
            }
        }
    }

    // Add automatic dependency update messages for auto-bumped crates
    detect_dependency_updates(workspace, &releases, &mut messages_by_pkg)?;

    // Compose header
    let mut output = String::new();
    output.push_str("This PR was generated by ");
    output.push_str("[Sampo GitHub Action](https://github.com/bruits/sampo/blob/main/crates/sampo-github-action/README.md).");
    output.push_str(" When you're ready to do a release, you can merge this and the packages will be published automatically. ");
    output.push_str("Not ready yet? Just keep adding changesets to the default branch, and this PR will stay up to date.\n\n");

    // Deterministic crate order by name
    let mut crate_names: Vec<_> = releases.keys().cloned().collect();
    crate_names.sort();
    for name in crate_names {
        let (old_version, new_version) = &releases[&name];
        output.push_str(&format!(
            "## {} {} -> {}\n\n",
            name, old_version, new_version
        ));

        let mut major_changes = Vec::new();
        let mut minor_changes = Vec::new();
        let mut patch_changes = Vec::new();

        if let Some(changeset_list) = messages_by_pkg.get(&name) {
            // Helper to push without duplicates (preserve append order)
            let push_unique = |list: &mut Vec<String>, msg: &str| {
                if !list.iter().any(|m| m == msg) {
                    list.push(msg.to_string());
                }
            };

            for (message, bump_type) in changeset_list {
                match bump_type {
                    Bump::Major => push_unique(&mut major_changes, message),
                    Bump::Minor => push_unique(&mut minor_changes, message),
                    Bump::Patch => push_unique(&mut patch_changes, message),
                }
            }
        }

        append_changes_section(&mut output, "Major changes", &major_changes);
        append_changes_section(&mut output, "Minor changes", &minor_changes);
        append_changes_section(&mut output, "Patch changes", &patch_changes);
    }

    Ok(output)
}

/// Append a changes section to the output if the changes list is not empty
fn append_changes_section(output: &mut String, section_title: &str, changes: &[String]) {
    if !changes.is_empty() {
        output.push_str(&format!("### {}\n\n", section_title));
        for change in changes {
            output.push_str("- ");
            output.push_str(change);
            output.push('\n');
        }
        output.push('\n');
    }
}

/// Extract planned release information from sampo dry-run output.
///
/// Looks for lines like "package-name: 0.1.0 -> 0.2.0" and parses them
/// into a map of package name to (old_version, new_version) pairs.
fn parse_planned_releases(stdout: &str) -> BTreeMap<String, (String, String)> {
    // Extract lines like: "  name: 0.1.0 -> 0.2.0"
    let mut map = BTreeMap::new();
    for line in stdout.lines() {
        let l = line.trim();
        if l.is_empty() || !l.contains("->") || !l.contains(':') {
            continue;
        }
        // Split on ':' first
        let mut parts = l.splitn(2, ':');
        let name = parts.next().unwrap().trim().to_string();
        let rest = parts.next().unwrap().trim();
        let mut arrow = rest.splitn(2, "->");
        let old = arrow.next().unwrap().trim().to_string();
        let new_version = arrow.next().unwrap_or("").trim().to_string();
        if !name.is_empty() && !old.is_empty() && !new_version.is_empty() {
            map.insert(name, (old, new_version));
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_planned_releases() {
        let stdout = "Planning release for packages:
  sampo: 0.1.0 -> 0.2.0
  sampo-github-action: 0.0.1 -> 0.1.0
No changesets found for other-pkg";

        let releases = parse_planned_releases(stdout);
        assert_eq!(releases.len(), 2);
        assert_eq!(
            releases.get("sampo"),
            Some(&("0.1.0".to_string(), "0.2.0".to_string()))
        );
        assert_eq!(
            releases.get("sampo-github-action"),
            Some(&("0.0.1".to_string(), "0.1.0".to_string()))
        );
    }

    #[test]
    fn test_parse_planned_releases_empty() {
        let stdout = "No changesets found";
        let releases = parse_planned_releases(stdout);
        assert!(releases.is_empty());
    }

    #[test]
    fn test_append_changes_section() {
        let mut output = String::new();
        let changes = vec!["Fix bug A".to_string(), "Add feature B".to_string()];

        append_changes_section(&mut output, "Major changes", &changes);

        let expected = "### Major changes\n\n- Fix bug A\n- Add feature B\n\n";
        assert_eq!(output, expected);
    }

    #[test]
    fn test_append_changes_section_empty() {
        let mut output = String::new();
        let changes: Vec<String> = vec![];

        append_changes_section(&mut output, "Major changes", &changes);

        assert_eq!(output, "");
    }

    #[test]
    fn test_no_duplicate_messages_in_changelog() {
        // Test that duplicate messages are filtered out properly
        let mut major_changes: Vec<String> = Vec::new();

        // Helper function that mimics the one used in build_release_pr_body_from_stdout
        let push_unique = |list: &mut Vec<String>, msg: &str| {
            if !list.iter().any(|m| m == msg) {
                list.push(msg.to_string());
            }
        };

        // Simulate adding the same message multiple times
        push_unique(&mut major_changes, "Fix critical bug");
        push_unique(&mut major_changes, "Fix critical bug"); // duplicate
        push_unique(&mut major_changes, "Add new feature");
        push_unique(&mut major_changes, "Fix critical bug"); // another duplicate

        // Should only have 2 unique messages
        assert_eq!(major_changes.len(), 2);
        assert_eq!(major_changes, vec!["Fix critical bug", "Add new feature"]);
    }

    #[test]
    fn test_dependency_updates_in_pr_body() {
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create workspace structure
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        let a_dir = root.join("crates/a");
        let b_dir = root.join("crates/b");
        fs::create_dir_all(&a_dir).unwrap();
        fs::create_dir_all(&b_dir).unwrap();

        fs::write(
            b_dir.join("Cargo.toml"),
            "[package]\nname=\"b\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        // a depends on b
        fs::write(
            a_dir.join("Cargo.toml"),
            "[package]\nname=\"a\"\nversion=\"0.1.0\"\n\n[dependencies]\nb = { path=\"../b\", version=\"0.1.0\" }\n",
        )
        .unwrap();

        // Create a changeset that only affects b
        let csdir = root.join(".sampo/changesets");
        fs::create_dir_all(&csdir).unwrap();
        fs::write(
            csdir.join("b-minor.md"),
            "---\npackages:\n  - b\nrelease: minor\n---\n\nfeat: b adds new feature\n",
        )
        .unwrap();

        // Simulate the output from `sampo release --dry-run`
        let plan_stdout = "Planned releases:\n  a: 0.1.0 -> 0.1.1\n  b: 0.1.0 -> 0.2.0\n";

        let pr_body = build_release_pr_body_from_stdout(root, plan_stdout).unwrap();

        // Should contain dependency update information for package a
        assert!(pr_body.contains("## a 0.1.0 -> 0.1.1"));
        assert!(pr_body.contains("## b 0.1.0 -> 0.2.0"));
        assert!(pr_body.contains("feat: b adds new feature"));
        assert!(pr_body.contains("Updated dependencies: b@0.2.0"));
    }
}
