use crate::error::{ActionError, Result};
use sampo_core::format_markdown_list_item;
use sampo_core::types::{
    ChangelogCategory, PackageSpecifier, SpecResolution, format_ambiguity_options,
};
use sampo_core::{
    Config, PublishExtraArgs, PublishOutput, VersionChange, detect_all_dependency_explanations,
    detect_github_repo_slug_with_config, discover_workspace, enrich_changeset_message,
    exit_prerelease as core_exit_prerelease, get_commit_hash_for_path, load_changesets,
    run_publish as core_publish, run_release as core_release,
};
use std::collections::BTreeMap;
use std::path::Path;

fn set_cargo_env_var(value: &str) {
    unsafe {
        std::env::set_var("CARGO_REGISTRY_TOKEN", value);
    }
}

#[derive(Debug)]
pub struct ReleasePlan {
    pub has_changes: bool,
    /// key: canonical identifier, value: (display name, old version, new version)
    pub releases: BTreeMap<String, (String, String, String)>,
}

/// Run sampo release and capture the plan
pub fn capture_release_plan(workspace: &Path) -> Result<ReleasePlan> {
    let release_output =
        core_release(workspace, true).map_err(|e| ActionError::SampoCommandFailed {
            operation: "release-plan".to_string(),
            message: format!("Release plan failed: {}", e),
        })?;

    let has_changes = !release_output.released_packages.is_empty();
    let mut releases: BTreeMap<String, (String, String, String)> = BTreeMap::new();
    if has_changes {
        for pkg in release_output.released_packages {
            releases.insert(pkg.identifier, (pkg.name, pkg.old_version, pkg.new_version));
        }
    }

    Ok(ReleasePlan {
        has_changes,
        releases,
    })
}

/// Execute sampo release
pub fn run_release(workspace: &Path, dry_run: bool, cargo_token: Option<&str>) -> Result<()> {
    // Set cargo token if provided
    if let Some(token) = cargo_token {
        set_cargo_env_var(token);
    }

    core_release(workspace, dry_run).map_err(|e| ActionError::SampoCommandFailed {
        operation: "release".to_string(),
        message: format!("sampo release failed: {}", e),
    })?;

    Ok(())
}

/// Execute sampo publish and return information about created/would-be-created tags
pub fn run_publish(
    workspace: &Path,
    dry_run: bool,
    extra_args: Option<&str>,
    cargo_token: Option<&str>,
) -> Result<PublishOutput> {
    // Set cargo token if provided
    if let Some(token) = cargo_token {
        set_cargo_env_var(token);
    }

    // Parse extra args into universal passthrough
    let extra_args = PublishExtraArgs {
        universal: if let Some(args) = extra_args {
            args.split_whitespace().map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        },
        ..PublishExtraArgs::default()
    };

    core_publish(workspace, dry_run, &extra_args).map_err(|e| ActionError::SampoCommandFailed {
        operation: "publish".to_string(),
        message: format!("sampo publish failed: {}", e),
    })
}

/// Exit pre-release mode for the provided packages.
pub fn exit_prerelease(workspace: &Path, packages: &[String]) -> Result<Vec<VersionChange>> {
    core_exit_prerelease(workspace, packages).map_err(|e| ActionError::SampoCommandFailed {
        operation: "exit-prerelease".to_string(),
        message: format!("sampo pre exit failed: {}", e),
    })
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
/// * `config` - Configuration reference to use for dependency policies and GitHub settings
///
/// # Returns
/// A formatted markdown string for the PR body, or empty string if no releases are planned
pub fn build_release_pr_body(
    workspace: &Path,
    releases: &BTreeMap<String, (String, String, String)>,
    config: &Config,
) -> Result<String> {
    if releases.is_empty() {
        return Ok(String::new());
    }

    let changesets_dir = workspace.join(".sampo").join("changesets");
    let changesets = load_changesets(&changesets_dir, &config.changesets_tags)?;

    // Load workspace for dependency explanations
    let ws = discover_workspace(workspace).map_err(|e| ActionError::SampoCommandFailed {
        operation: "workspace-discovery".into(),
        message: e.to_string(),
    })?;
    let include_kind = ws.has_multiple_package_kinds();

    // Group messages per canonical package id by category
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, ChangelogCategory)>> = BTreeMap::new();

    // Resolve GitHub slug and token for commit links and acknowledgments
    let repo_slug =
        detect_github_repo_slug_with_config(workspace, config.github_repository.as_deref());
    let github_token = std::env::var("GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GH_TOKEN").ok());

    for cs in &changesets {
        for (pkg_spec, bump, tag) in &cs.entries {
            let identifier = resolve_specifier_identifier(&ws, pkg_spec)?;
            if releases.contains_key(&identifier) {
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
                let category = if let Some(t) = tag {
                    ChangelogCategory::Tag(t.clone())
                } else {
                    ChangelogCategory::Bump(*bump)
                };
                messages_by_pkg
                    .entry(identifier)
                    .or_default()
                    .push((enriched, category));
            }
        }
    }

    // Add automatic dependency explanations using unified function
    let release_lookup: BTreeMap<String, (String, String)> = releases
        .iter()
        .map(|(identifier, (_display, old_version, new_version))| {
            (
                identifier.clone(),
                (old_version.clone(), new_version.clone()),
            )
        })
        .collect();
    let explanations =
        detect_all_dependency_explanations(&changesets, &ws, config, &release_lookup)?;

    // Merge explanations into messages_by_pkg
    for (pkg_name, explanations) in explanations {
        messages_by_pkg
            .entry(pkg_name)
            .or_default()
            .extend(explanations);
    }

    // Compose header
    let mut output = String::new();
    output.push_str("This PR was generated by ");
    output.push_str("[Sampo GitHub Action](https://github.com/bruits/sampo/blob/main/crates/sampo-github-action/README.md).");
    output.push_str(" When you're ready to do a release, you can merge this and the packages will be published automatically. ");
    output.push_str("Not ready yet? Just keep adding changesets to the default branch, and this PR will stay up to date.\n\n");

    // Deterministic crate order by name
    let mut crate_ids: Vec<_> = releases.keys().cloned().collect();
    crate_ids.sort();
    for identifier in crate_ids {
        let (fallback_name, old_version, new_version) = &releases[&identifier];
        let pretty_name = display_label_for_release(&ws, &identifier, include_kind, fallback_name);
        output.push_str(&format!(
            "## {} {} -> {}\n\n",
            pretty_name, old_version, new_version
        ));

        // Collect changes by category
        let mut changes_by_category: BTreeMap<ChangelogCategory, Vec<String>> = BTreeMap::new();

        if let Some(changeset_list) = messages_by_pkg.get(&identifier) {
            // Helper to push without duplicates (preserve append order)
            let push_unique = |list: &mut Vec<String>, msg: &str| {
                if !list.iter().any(|m| m == msg) {
                    list.push(msg.to_string());
                }
            };

            for (message, category) in changeset_list {
                push_unique(
                    changes_by_category.entry(category.clone()).or_default(),
                    message,
                );
            }
        }

        // Sort categories: tags alphabetically first, then bump types by severity
        let mut categories: Vec<_> = changes_by_category.keys().cloned().collect();
        categories.sort_by_key(|c| c.sort_key());

        for category in categories {
            if let Some(changes) = changes_by_category.get(&category) {
                append_changes_section(&mut output, &category.heading(), changes);
            }
        }
    }

    Ok(output)
}

fn resolve_specifier_identifier(
    workspace: &sampo_core::Workspace,
    spec: &PackageSpecifier,
) -> Result<String> {
    match workspace.resolve_specifier(spec) {
        SpecResolution::Match(info) => Ok(info.canonical_identifier().to_string()),
        SpecResolution::NotFound { query } => {
            let error = if let Some(identifier) = query.identifier() {
                sampo_core::errors::SampoError::Changeset(format!(
                    "Changeset references '{}', but it was not found in the workspace.",
                    identifier
                ))
            } else {
                sampo_core::errors::SampoError::Changeset(format!(
                    "Changeset references '{}', but no matching package exists in the workspace.",
                    query.base_name()
                ))
            };
            Err(error.into())
        }
        SpecResolution::Ambiguous { query, matches } => {
            let options = format_ambiguity_options(&matches);
            Err(sampo_core::errors::SampoError::Changeset(format!(
                "Changeset references '{}', which matches multiple packages. Disambiguate using one of: {}.",
                query.base_name(),
                options
            ))
            .into())
        }
    }
}

fn display_label_for_release(
    workspace: &sampo_core::Workspace,
    identifier: &str,
    include_kind: bool,
    fallback: &str,
) -> String {
    if let Some(info) = workspace.find_by_identifier(identifier) {
        return info.display_name(include_kind);
    }
    if let Ok(spec) = PackageSpecifier::parse(identifier) {
        return spec.display_name(include_kind);
    }
    fallback.to_string()
}

/// Append a changes section to the output if the changes list is not empty
fn append_changes_section(output: &mut String, section_title: &str, changes: &[String]) {
    if !changes.is_empty() {
        output.push_str(&format!("### {}\n\n", section_title));
        for change in changes {
            output.push_str(&format_markdown_list_item(change));
        }
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_MUTEX.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvVarGuard {
        fn set_branch(value: &str) -> Self {
            let key = "SAMPO_RELEASE_BRANCH";
            let lock = env_lock().lock().unwrap();
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key,
                original,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
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
    fn test_append_changes_section_multiline_with_nested_list() {
        let mut output = String::new();
        let changes = vec!["feat: big change with details\n- add A\n- add B".to_string()];

        append_changes_section(&mut output, "Minor changes", &changes);

        let expected =
            "### Minor changes\n\n- feat: big change with details\n  - add A\n  - add B\n\n";
        assert_eq!(output, expected);
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
        let _branch = EnvVarGuard::set_branch("main");
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
            "---\nb: minor\n---\n\nfeat: b adds new feature\n",
        )
        .unwrap();

        // Compute the release plan using core logic (structured)
        let plan = capture_release_plan(root).unwrap();
        assert!(plan.has_changes);
        let config = Config::default();
        let pr_body = build_release_pr_body(root, &plan.releases, &config).unwrap();

        // Should contain dependency update information for package a
        assert!(pr_body.contains("## a 0.1.0 -> 0.1.1"));
        assert!(pr_body.contains("## b 0.1.0 -> 0.2.0"));
        assert!(pr_body.contains("feat: b adds new feature"));
        assert!(pr_body.contains("Updated dependencies: b@0.2.0"));
    }

    #[test]
    fn test_fixed_dependencies_in_pr_body() {
        let _branch = EnvVarGuard::set_branch("main");
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
            "[package]\nname=\"b\"\nversion=\"1.0.0\"\n",
        )
        .unwrap();

        // a depends on b
        fs::write(
            a_dir.join("Cargo.toml"),
            "[package]\nname=\"a\"\nversion=\"1.0.0\"\n\n[dependencies]\nb = { path=\"../b\", version=\"1.0.0\" }\n",
        )
        .unwrap();

        // Create sampo config with fixed dependencies
        let sampo_dir = root.join(".sampo");
        fs::create_dir_all(&sampo_dir).unwrap();
        fs::write(
            sampo_dir.join("config.toml"),
            "[packages]\nfixed = [[\"a\", \"b\"]]\n",
        )
        .unwrap();

        // Create a changeset that only affects b
        let csdir = root.join(".sampo/changesets");
        fs::create_dir_all(&csdir).unwrap();
        fs::write(
            csdir.join("b-major.md"),
            "---\nb: major\n---\n\nbreaking: b breaking change\n",
        )
        .unwrap();

        // Compute the plan using core logic and the fixed dependency config
        let plan = capture_release_plan(root).unwrap();
        assert!(plan.has_changes);
        let config = Config::load(root).unwrap();
        let pr_body = build_release_pr_body(root, &plan.releases, &config).unwrap();

        // Should contain information for both packages with same version
        assert!(pr_body.contains("## a 1.0.0 -> 2.0.0"));
        assert!(pr_body.contains("## b 1.0.0 -> 2.0.0"));
        assert!(pr_body.contains("breaking: b breaking change"));
        // Fixed dependency should show dependency update too
        assert!(pr_body.contains("Updated dependencies: b@2.0.0"));
    }

    #[test]
    fn test_fixed_dependencies_without_actual_dependency() {
        let _branch = EnvVarGuard::set_branch("main");
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
            "[package]\nname=\"b\"\nversion=\"1.0.0\"\n",
        )
        .unwrap();

        // a does NOT depend on b (important difference)
        fs::write(
            a_dir.join("Cargo.toml"),
            "[package]\nname=\"a\"\nversion=\"1.0.0\"\n",
        )
        .unwrap();

        // Create sampo config with fixed dependencies
        let sampo_dir = root.join(".sampo");
        fs::create_dir_all(&sampo_dir).unwrap();
        fs::write(
            sampo_dir.join("config.toml"),
            "[packages]\nfixed = [[\"a\", \"b\"]]\n",
        )
        .unwrap();

        // Create a changeset that only affects b
        let csdir = root.join(".sampo/changesets");
        fs::create_dir_all(&csdir).unwrap();
        fs::write(
            csdir.join("b-major.md"),
            "---\nb: major\n---\n\nbreaking: b breaking change\n",
        )
        .unwrap();

        // Compute the plan using core logic and the fixed dependency config
        let plan = capture_release_plan(root).unwrap();
        assert!(plan.has_changes);
        let config = Config::load(root).unwrap();
        let pr_body = build_release_pr_body(root, &plan.releases, &config).unwrap();

        println!("PR Body:\n{}", pr_body);

        // Should contain information for both packages with same version
        assert!(pr_body.contains("## a 1.0.0 -> 2.0.0"));
        assert!(pr_body.contains("## b 1.0.0 -> 2.0.0"));
        assert!(pr_body.contains("breaking: b breaking change"));

        // FIXED: Package 'a' should now have an explanation for the bump!
        assert!(pr_body.contains("Bumped due to fixed dependency group policy"));
    }

    #[test]
    fn test_capture_plan_and_pr_body_end_to_end() {
        let _branch = EnvVarGuard::set_branch("main");
        use std::fs;
        // Setup a minimal workspace with one crate and a minor changeset
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Workspace root
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        // Single crate 'example' v0.1.0
        let ex_dir = root.join("crates/example");
        fs::create_dir_all(&ex_dir).unwrap();
        fs::write(
            ex_dir.join("Cargo.toml"),
            "[package]\nname=\"example\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        // Changeset: example minor change
        let cs_dir = root.join(".sampo/changesets");
        fs::create_dir_all(&cs_dir).unwrap();
        fs::write(
            cs_dir.join("example-minor.md"),
            "---\nexample: minor\n---\n\nfeat: add new thing\n",
        )
        .unwrap();

        // Capture release plan (dry-run) using core logic
        let plan = capture_release_plan(root).expect("plan should succeed");
        assert!(plan.has_changes);
        assert_eq!(
            plan.releases.get("cargo/example"),
            Some(&(
                "example".to_string(),
                "0.1.0".to_string(),
                "0.2.0".to_string()
            ))
        );

        // Build PR body from the structured plan
        let config = Config::load(root).unwrap_or_default();
        let pr_body = build_release_pr_body(root, &plan.releases, &config).unwrap();

        // Ensure PR body uses the changelog layout
        assert!(pr_body.contains("## example 0.1.0 -> 0.2.0"));
        assert!(pr_body.contains("### Minor changes"));
        assert!(pr_body.contains("- feat: add new thing"));
    }
}
