use crate::adapters::{ManifestMetadata, PackageAdapter};
use crate::errors::{Result, SampoError, io_error_with_path};
use crate::filters::should_ignore_package;
use crate::types::{
    Bump, ChangelogCategory, DependencyUpdate, PackageInfo, PackageKind, PackageSpecifier,
    ReleaseOutput, ReleasedPackage, SpecResolution, Workspace, format_ambiguity_options,
};
use crate::{
    changeset::ChangesetInfo, config::Config, current_branch, detect_github_repo_slug_with_config,
    discover_workspace, enrich_changeset_message, get_commit_hash_for_path, load_changesets,
};
use chrono::{DateTime, FixedOffset, Local, Utc};
use chrono_tz::Tz;
use semver::{BuildMetadata, Prerelease, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Format dependency updates for changelog display
///
/// Creates a message in the style of Changesets for dependency updates,
/// e.g., "Updated dependencies [hash]: pkg1@1.2.0, pkg2@2.0.0"
pub fn format_dependency_updates_message(updates: &[DependencyUpdate]) -> Option<String> {
    if updates.is_empty() {
        return None;
    }

    let mut parsed_updates: Vec<(
        Option<PackageSpecifier>,
        Option<String>,
        String,
        &DependencyUpdate,
    )> = Vec::with_capacity(updates.len());
    let mut labels_by_name: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for dep in updates {
        if let Ok(spec) = PackageSpecifier::parse(&dep.name) {
            let base_name = spec.name.clone();
            if let Some(kind) = spec.kind {
                labels_by_name
                    .entry(base_name.clone())
                    .or_default()
                    .insert(kind.as_str().to_string());
            } else {
                labels_by_name.entry(base_name.clone()).or_default();
            }
            parsed_updates.push((Some(spec), None, base_name, dep));
        } else if let Some((prefix, name)) = dep.name.split_once('/') {
            let base_name = name.to_string();
            labels_by_name
                .entry(base_name.clone())
                .or_default()
                .insert(prefix.to_ascii_lowercase());
            parsed_updates.push((None, Some(prefix.to_string()), base_name, dep));
        } else {
            let base_name = dep.name.clone();
            labels_by_name.entry(base_name.clone()).or_default();
            parsed_updates.push((None, None, base_name, dep));
        }
    }

    let ambiguous_names: BTreeSet<String> = labels_by_name
        .iter()
        .filter_map(|(name, labels)| {
            if labels.len() > 1 {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    let dep_list = parsed_updates
        .into_iter()
        .map(|(spec_opt, raw_prefix, base_name, dep)| {
            let is_ambiguous = ambiguous_names.contains(&base_name);
            let display_label = if let Some(spec) = spec_opt.as_ref() {
                if let Some(kind) = spec.kind {
                    if is_ambiguous {
                        format!("{}/{}", kind.as_str(), spec.name)
                    } else {
                        spec.display_name(false)
                    }
                } else {
                    spec.display_name(false)
                }
            } else if let Some(prefix) = raw_prefix.as_ref() {
                if is_ambiguous {
                    format!("{}/{}", prefix.to_ascii_lowercase(), base_name)
                } else {
                    base_name.clone()
                }
            } else {
                base_name.clone()
            };
            format!("{display_label}@{}", dep.new_version)
        })
        .collect::<Vec<_>>()
        .join(", ");

    Some(format!("Updated dependencies: {}", dep_list))
}

/// Convert a list of (name, version) tuples into DependencyUpdate structs
pub fn build_dependency_updates(updates: &[(String, String)]) -> Vec<DependencyUpdate> {
    updates
        .iter()
        .map(|(name, version)| DependencyUpdate {
            name: name.clone(),
            new_version: version.clone(),
        })
        .collect()
}

fn resolve_package_spec<'a>(
    workspace: &'a Workspace,
    spec: &PackageSpecifier,
) -> Result<&'a PackageInfo> {
    match workspace.resolve_specifier(spec) {
        SpecResolution::Match(info) => Ok(info),
        SpecResolution::NotFound { query } => match query.identifier() {
            Some(identifier) => Err(SampoError::Changeset(format!(
                "Changeset references '{}', but it was not found in the workspace.",
                identifier
            ))),
            None => Err(SampoError::Changeset(format!(
                "Changeset references '{}', but no matching package exists in the workspace.",
                query.base_name()
            ))),
        },
        SpecResolution::Ambiguous { query, matches } => {
            let options = format_ambiguity_options(&matches);
            Err(SampoError::Changeset(format!(
                "Changeset references '{}', which matches multiple packages. \
                 Disambiguate using one of: {}.",
                query.base_name(),
                options
            )))
        }
    }
}

fn resolve_config_value(workspace: &Workspace, value: &str, context: &str) -> Result<String> {
    let spec = PackageSpecifier::parse(value).map_err(|reason| {
        SampoError::Config(format!(
            "{}: invalid package reference '{}': {}",
            context, value, reason
        ))
    })?;

    match workspace.resolve_specifier(&spec) {
        SpecResolution::Match(info) => Ok(info.canonical_identifier().to_string()),
        SpecResolution::NotFound { query } => Err(SampoError::Config(format!(
            "{}: package '{}' not found in the workspace.",
            context,
            query.display()
        ))),
        SpecResolution::Ambiguous { query, matches } => {
            let options = format_ambiguity_options(&matches);
            Err(SampoError::Config(format!(
                "{}: package '{}' is ambiguous. Use one of: {}.",
                context,
                query.base_name(),
                options
            )))
        }
    }
}

fn resolve_config_groups(
    workspace: &Workspace,
    groups: &[Vec<String>],
    section: &str,
) -> Result<Vec<Vec<String>>> {
    let mut resolved = Vec::with_capacity(groups.len());
    for (idx, group) in groups.iter().enumerate() {
        let mut resolved_group = Vec::with_capacity(group.len());
        let context = format!("{} group {}", section, idx + 1);
        for value in group {
            let identifier = resolve_config_value(workspace, value, &context)?;
            resolved_group.push(identifier);
        }
        resolved.push(resolved_group);
    }
    Ok(resolved)
}

/// Create a changelog entry for dependency updates
///
/// Returns a tuple of (message, bump_type) suitable for adding to changelog messages
pub fn create_dependency_update_entry(updates: &[DependencyUpdate]) -> Option<(String, Bump)> {
    format_dependency_updates_message(updates).map(|msg| (msg, Bump::Patch))
}

/// Create a changelog entry for fixed dependency group policy
///
/// Returns a tuple of (message, bump_type) suitable for adding to changelog messages
pub fn create_fixed_dependency_policy_entry(bump: Bump) -> (String, Bump) {
    (
        "Bumped due to fixed dependency group policy".to_string(),
        bump,
    )
}

/// Infer bump type from version changes
///
/// This helper function determines the semantic version bump type based on
/// the difference between old and new version strings.
pub fn infer_bump_from_versions(old_ver: &str, new_ver: &str) -> Bump {
    let old_parts: Vec<u32> = old_ver.split('.').filter_map(|s| s.parse().ok()).collect();
    let new_parts: Vec<u32> = new_ver.split('.').filter_map(|s| s.parse().ok()).collect();

    if old_parts.len() >= 3 && new_parts.len() >= 3 {
        if new_parts[0] > old_parts[0] {
            Bump::Major
        } else if new_parts[1] > old_parts[1] {
            Bump::Minor
        } else {
            Bump::Patch
        }
    } else {
        Bump::Patch
    }
}

/// Detect all dependency-related explanations for package releases
///
/// This function is the unified entry point for detecting all types of automatic
/// dependency-related changelog entries. It identifies:
/// - Packages bumped due to internal dependency updates ("Updated dependencies: ...")
/// - Packages bumped due to fixed dependency group policy ("Bumped due to fixed dependency group policy")
///
/// # Arguments
/// * `changesets` - The changesets being processed
/// * `workspace` - The workspace containing all packages
/// * `config` - The configuration with dependency policies
/// * `releases` - Map of package name to (old_version, new_version) for all planned releases
///
/// # Returns
/// A map of package name to list of (message, category) explanations to add to changelogs
pub fn detect_all_dependency_explanations(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    releases: &BTreeMap<String, (String, String)>,
) -> Result<BTreeMap<String, Vec<(String, ChangelogCategory)>>> {
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, ChangelogCategory)>> = BTreeMap::new();
    let include_kind = workspace.has_multiple_package_kinds();

    // 1. Detect packages bumped due to fixed dependency group policy
    let bumped_packages: BTreeSet<String> = releases.keys().cloned().collect();
    let policy_packages =
        detect_fixed_dependency_policy_packages(changesets, workspace, config, &bumped_packages)?;

    for (pkg_name, policy_bump) in policy_packages {
        // For accurate bump detection, infer from actual version changes
        let actual_bump = if let Some((old_ver, new_ver)) = releases.get(&pkg_name) {
            infer_bump_from_versions(old_ver, new_ver)
        } else {
            policy_bump
        };

        let (msg, bump_type) = create_fixed_dependency_policy_entry(actual_bump);
        messages_by_pkg
            .entry(pkg_name)
            .or_default()
            .push((msg, ChangelogCategory::Bump(bump_type)));
    }

    // 2. Detect packages bumped due to internal dependency updates
    // Note: Even packages with explicit changesets can have dependency updates

    // Build new version lookup from releases
    let new_version_by_name: BTreeMap<String, String> = releases
        .iter()
        .map(|(name, (_old, new_ver))| (name.clone(), new_ver.clone()))
        .collect();

    // Build map of package name -> PackageInfo for quick lookup (only non-ignored packages)
    let by_id: BTreeMap<String, &PackageInfo> = workspace
        .members
        .iter()
        .filter(|c| !should_ignore_package(config, workspace, c).unwrap_or(false))
        .map(|c| (c.canonical_identifier().to_string(), c))
        .collect();

    // For each released crate, check if it has internal dependencies that were updated
    for crate_id in releases.keys() {
        if let Some(crate_info) = by_id.get(crate_id) {
            // Find which internal dependencies were updated
            let mut updated_deps = Vec::new();
            for dep_name in &crate_info.internal_deps {
                if let Some(new_version) = new_version_by_name.get(dep_name as &str) {
                    // This internal dependency was updated
                    let display_dep = by_id
                        .get(dep_name)
                        .map(|info| info.display_name(include_kind))
                        .or_else(|| {
                            PackageSpecifier::parse(dep_name)
                                .ok()
                                .map(|spec| spec.display_name(include_kind))
                        })
                        .unwrap_or_else(|| dep_name.clone());
                    updated_deps.push((display_dep, new_version.clone()));
                }
            }

            if !updated_deps.is_empty() {
                // Create dependency update entry
                let updates = build_dependency_updates(&updated_deps);
                if let Some((msg, bump)) = create_dependency_update_entry(&updates) {
                    messages_by_pkg
                        .entry(crate_id.clone())
                        .or_default()
                        .push((msg, ChangelogCategory::Bump(bump)));
                }
            }
        }
    }

    Ok(messages_by_pkg)
}

/// Detect packages that need fixed dependency group policy messages
///
/// This function identifies packages that were bumped solely due to fixed dependency
/// group policies (not due to direct changesets or normal dependency cascades).
/// Returns a map of package name to the bump level they received.
pub fn detect_fixed_dependency_policy_packages(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    bumped_packages: &BTreeSet<String>,
) -> Result<BTreeMap<String, Bump>> {
    // Build set of packages with direct changesets
    let mut packages_with_changesets: BTreeSet<String> = BTreeSet::new();
    for cs in changesets {
        for (spec, _, _) in &cs.entries {
            let info = resolve_package_spec(workspace, spec)?;
            packages_with_changesets.insert(info.canonical_identifier().to_string());
        }
    }

    let resolved_groups =
        resolve_config_groups(workspace, &config.fixed_dependencies, "packages.fixed")?;

    // Build dependency graph (dependent -> set of dependencies) - only non-ignored packages
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for crate_info in &workspace.members {
        // Skip ignored packages when building the dependency graph
        if should_ignore_package(config, workspace, crate_info).unwrap_or(false) {
            continue;
        }

        for dep_name in &crate_info.internal_deps {
            dependents
                .entry(dep_name.clone())
                .or_default()
                .insert(crate_info.canonical_identifier().to_string());
        }
    }

    // Find packages affected by normal dependency cascade
    let mut packages_affected_by_cascade = BTreeSet::new();
    for pkg_with_changeset in &packages_with_changesets {
        let mut queue = vec![pkg_with_changeset.clone()];
        let mut visited = BTreeSet::new();

        while let Some(pkg) = queue.pop() {
            if visited.contains(&pkg) {
                continue;
            }
            visited.insert(pkg.clone());

            if let Some(deps) = dependents.get(&pkg) {
                for dep in deps {
                    packages_affected_by_cascade.insert(dep.clone());
                    queue.push(dep.clone());
                }
            }
        }
    }

    // Find packages that need fixed dependency policy messages
    let mut result = BTreeMap::new();

    for pkg_name in bumped_packages {
        // Skip if package has direct changeset
        if packages_with_changesets.contains(pkg_name) {
            continue;
        }

        // Skip if package is affected by normal dependency cascade
        if packages_affected_by_cascade.contains(pkg_name) {
            continue;
        }

        // Check if this package is in a fixed dependency group with an affected package
        for group in &resolved_groups {
            if !group.contains(pkg_name) {
                continue;
            }

            let has_affected_group_member = group.iter().any(|member_id| {
                member_id != pkg_name
                    && (packages_with_changesets.contains(member_id)
                        || packages_affected_by_cascade.contains(member_id))
            });

            if !has_affected_group_member {
                continue;
            }

            // Find the highest bump level in the group to determine the policy bump
            let group_bump = group
                .iter()
                .filter_map(|member_id| {
                    if !packages_with_changesets.contains(member_id) {
                        return None;
                    }
                    changesets
                        .iter()
                        .filter_map(|cs| {
                            cs.entries.iter().find_map(|(spec, bump, _)| {
                                let info = resolve_package_spec(workspace, spec).ok()?;
                                if info.canonical_identifier() == member_id.as_str() {
                                    Some(*bump)
                                } else {
                                    None
                                }
                            })
                        })
                        .max()
                })
                .max()
                .unwrap_or(Bump::Patch);

            result.insert(pkg_name.clone(), group_bump);
            break;
        }
    }

    Ok(result)
}

/// Type alias for initial bumps computation result
type InitialBumpsResult = (
    BTreeMap<String, Bump>,                             // bump_by_pkg
    BTreeMap<String, Vec<(String, ChangelogCategory)>>, // messages_by_pkg
    BTreeSet<std::path::PathBuf>,                       // used_paths
);

/// Type alias for release plan
type ReleasePlan = Vec<(String, String, String)>; // (name, old_version, new_version)

/// Aggregated data required to apply a planned release
struct PlanState {
    messages_by_pkg: BTreeMap<String, Vec<(String, ChangelogCategory)>>,
    used_paths: BTreeSet<PathBuf>,
    releases: ReleasePlan,
    released_packages: Vec<ReleasedPackage>,
}

/// Possible outcomes when computing a release plan from a set of changesets
enum PlanOutcome {
    NoApplicablePackages,
    NoMatchingCrates,
    Plan(PlanState),
}

/// Main release function that can be called from CLI or other interfaces
pub fn run_release(root: &std::path::Path, dry_run: bool) -> Result<ReleaseOutput> {
    let workspace = discover_workspace(root)?;
    let config = Config::load(&workspace.root)?;

    let branch = current_branch()?;
    if !config.is_release_branch(&branch) {
        return Err(SampoError::Release(format!(
            "Branch '{}' is not configured for releases (allowed: {:?})",
            branch,
            config.release_branches().into_iter().collect::<Vec<_>>()
        )));
    }

    // Validate fixed dependencies configuration
    validate_fixed_dependencies(&config, &workspace)?;

    let changesets_dir = workspace.root.join(".sampo").join("changesets");
    let prerelease_dir = workspace.root.join(".sampo").join("prerelease");

    let current_changesets = load_changesets(&changesets_dir, &config.changesets_tags)?;
    let preserved_changesets = load_changesets(&prerelease_dir, &config.changesets_tags)?;

    let mut using_preserved = false;
    let mut cached_plan_state: Option<PlanState> = None;

    if current_changesets.is_empty() {
        if preserved_changesets.is_empty() {
            println!(
                "No changesets found in {}",
                workspace.root.join(".sampo").join("changesets").display()
            );
            return Ok(ReleaseOutput {
                released_packages: vec![],
                dry_run,
            });
        }

        let workspace_in_prerelease = workspace.members.iter().any(|m| m.version.contains('-'));
        if workspace_in_prerelease {
            println!(
                "No new changesets found. Preserved changesets exist but workspace \
                 is in pre-release mode; skipping to avoid duplicate bump."
            );
            return Ok(ReleaseOutput {
                released_packages: vec![],
                dry_run,
            });
        }

        using_preserved = true;
    } else {
        match compute_plan_state(&current_changesets, &workspace, &config)? {
            PlanOutcome::Plan(plan) => {
                let is_prerelease_preview = releases_include_prerelease(&plan.releases);
                if !is_prerelease_preview && !preserved_changesets.is_empty() {
                    using_preserved = true;
                } else {
                    cached_plan_state = Some(plan);
                }
            }
            PlanOutcome::NoApplicablePackages => {
                if preserved_changesets.is_empty() {
                    println!("No applicable packages found in changesets.");
                    return Ok(ReleaseOutput {
                        released_packages: vec![],
                        dry_run,
                    });
                }
                using_preserved = true;
            }
            PlanOutcome::NoMatchingCrates => {
                if preserved_changesets.is_empty() {
                    println!("No matching workspace crates to release.");
                    return Ok(ReleaseOutput {
                        released_packages: vec![],
                        dry_run,
                    });
                }
                using_preserved = true;
            }
        }
    }

    let mut final_changesets;
    let plan_state = if using_preserved {
        if dry_run {
            final_changesets = current_changesets;
            final_changesets.extend(preserved_changesets);
        } else {
            restore_prerelease_changesets(&prerelease_dir, &changesets_dir)?;
            final_changesets = load_changesets(&changesets_dir, &config.changesets_tags)?;
        }

        match compute_plan_state(&final_changesets, &workspace, &config)? {
            PlanOutcome::Plan(plan) => plan,
            PlanOutcome::NoApplicablePackages => {
                println!("No applicable packages found in changesets.");
                return Ok(ReleaseOutput {
                    released_packages: vec![],
                    dry_run,
                });
            }
            PlanOutcome::NoMatchingCrates => {
                println!("No matching workspace crates to release.");
                return Ok(ReleaseOutput {
                    released_packages: vec![],
                    dry_run,
                });
            }
        }
    } else {
        final_changesets = current_changesets;
        match cached_plan_state {
            Some(plan) => plan,
            None => match compute_plan_state(&final_changesets, &workspace, &config)? {
                PlanOutcome::Plan(plan) => plan,
                PlanOutcome::NoApplicablePackages => {
                    println!("No applicable packages found in changesets.");
                    return Ok(ReleaseOutput {
                        released_packages: vec![],
                        dry_run,
                    });
                }
                PlanOutcome::NoMatchingCrates => {
                    println!("No matching workspace crates to release.");
                    return Ok(ReleaseOutput {
                        released_packages: vec![],
                        dry_run,
                    });
                }
            },
        }
    };

    let PlanState {
        mut messages_by_pkg,
        used_paths,
        releases,
        released_packages,
    } = plan_state;

    print_release_plan(&workspace, &releases);

    let is_prerelease_release = releases_include_prerelease(&releases);

    if dry_run {
        println!("Dry-run: no files modified, no tags created.");
        return Ok(ReleaseOutput {
            released_packages,
            dry_run: true,
        });
    }

    apply_releases(
        &releases,
        &workspace,
        &mut messages_by_pkg,
        &final_changesets,
        &config,
    )?;

    finalize_consumed_changesets(used_paths, &workspace.root, is_prerelease_release)?;

    // Regenerate lockfiles for all ecosystems present in the workspace.
    // This ensures the release branch includes consistent, up-to-date lockfiles
    // and avoids a dirty working tree later. Only runs when lockfiles already exist,
    // to keep tests (which create ephemeral workspaces without lockfiles) fast.
    // Errors are logged but do not fail the release to keep behavior resilient.
    let _ = regenerate_lockfile(&workspace);

    Ok(ReleaseOutput {
        released_packages,
        dry_run: false,
    })
}

fn compute_plan_state(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
) -> Result<PlanOutcome> {
    let (mut bump_by_pkg, messages_by_pkg, used_paths) =
        compute_initial_bumps(changesets, workspace, config)?;

    if bump_by_pkg.is_empty() {
        return Ok(PlanOutcome::NoApplicablePackages);
    }

    let dependents = build_dependency_graph(workspace, config);
    apply_dependency_cascade(&mut bump_by_pkg, &dependents, config, workspace)?;
    apply_linked_dependencies(&mut bump_by_pkg, config, workspace)?;

    let releases = prepare_release_plan(&bump_by_pkg, workspace)?;
    if releases.is_empty() {
        return Ok(PlanOutcome::NoMatchingCrates);
    }

    let released_packages: Vec<ReleasedPackage> = releases
        .iter()
        .map(|(name, old_version, new_version)| {
            let bump = bump_by_pkg.get(name).copied().unwrap_or(Bump::Patch);
            let display_name = workspace
                .find_by_identifier(name)
                .map(|info| info.name.clone())
                .unwrap_or_else(|| name.clone());
            ReleasedPackage {
                name: display_name,
                identifier: name.clone(),
                old_version: old_version.clone(),
                new_version: new_version.clone(),
                bump,
            }
        })
        .collect();

    Ok(PlanOutcome::Plan(PlanState {
        messages_by_pkg,
        used_paths,
        releases,
        released_packages,
    }))
}

fn releases_include_prerelease(releases: &ReleasePlan) -> bool {
    releases.iter().any(|(_, _, new_version)| {
        Version::parse(new_version)
            .map(|v| !v.pre.is_empty())
            .unwrap_or(false)
    })
}

pub(crate) fn restore_prerelease_changesets(
    prerelease_dir: &Path,
    changesets_dir: &Path,
) -> Result<()> {
    if !prerelease_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(prerelease_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }

        // Ignore the new location; only errors matter here
        let _ = move_changeset_file(&path, changesets_dir)?;
    }

    Ok(())
}

fn finalize_consumed_changesets(
    used_paths: BTreeSet<PathBuf>,
    workspace_root: &Path,
    preserve_for_prerelease: bool,
) -> Result<()> {
    if used_paths.is_empty() {
        return Ok(());
    }

    if preserve_for_prerelease {
        let prerelease_dir = workspace_root.join(".sampo").join("prerelease");
        for path in used_paths {
            if !path.exists() {
                continue;
            }
            let _ = move_changeset_file(&path, &prerelease_dir)?;
        }
        println!("Preserved consumed changesets for pre-release.");
    } else {
        for path in used_paths {
            if !path.exists() {
                continue;
            }
            fs::remove_file(&path).map_err(|err| SampoError::Io(io_error_with_path(err, &path)))?;
        }
        println!("Removed consumed changesets.");
    }

    Ok(())
}

pub(crate) fn move_changeset_file(source: &Path, dest_dir: &Path) -> Result<PathBuf> {
    if !source.exists() {
        return Ok(source.to_path_buf());
    }

    fs::create_dir_all(dest_dir)?;
    let file_name = source
        .file_name()
        .ok_or_else(|| SampoError::Changeset("Invalid changeset file name".to_string()))?;

    let mut destination = dest_dir.join(file_name);
    if destination == source {
        return Ok(destination);
    }

    if destination.exists() {
        destination = unique_destination_path(dest_dir, file_name);
    }

    fs::rename(source, &destination)?;
    Ok(destination)
}

fn unique_destination_path(dir: &Path, file_name: &OsStr) -> PathBuf {
    let file_path = Path::new(file_name);
    let stem = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.to_string_lossy().into_owned());
    let ext = file_path
        .extension()
        .map(|s| s.to_string_lossy().into_owned());

    let mut counter = 1;
    loop {
        let candidate_name = if let Some(ref ext) = ext {
            format!("{}-{}.{}", stem, counter, ext)
        } else {
            format!("{}-{}", stem, counter)
        };
        let candidate = dir.join(&candidate_name);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

/// Regenerate the Cargo.lock at the workspace root using Cargo.
///
/// Uses `cargo generate-lockfile`, which will rebuild the lockfile with the latest
/// compatible versions, ensuring the lockfile reflects the new workspace versions.
/// Regenerate lockfiles for all ecosystems present in a workspace.
pub(crate) fn regenerate_lockfile(workspace: &Workspace) -> Result<()> {
    use crate::types::PackageKind;
    use rustc_hash::FxHashSet;

    // Determine which ecosystems are present in the workspace
    let mut ecosystems: FxHashSet<PackageKind> = FxHashSet::default();
    for pkg in &workspace.members {
        ecosystems.insert(pkg.kind);
    }

    // Regenerate lockfiles for each ecosystem present
    let mut errors: Vec<(PackageKind, String)> = Vec::new();

    for kind in ecosystems {
        let adapter = match kind {
            PackageKind::Cargo => PackageAdapter::Cargo,
            PackageKind::Npm => PackageAdapter::Npm,
            PackageKind::Hex => PackageAdapter::Hex,
            PackageKind::PyPI => PackageAdapter::PyPI,
            PackageKind::Packagist => PackageAdapter::Packagist,
        };

        let lockfile_exists = match kind {
            PackageKind::Cargo => workspace.root.join("Cargo.lock").exists(),
            PackageKind::Npm => {
                workspace.root.join("package-lock.json").exists()
                    || workspace.root.join("pnpm-lock.yaml").exists()
                    || workspace.root.join("yarn.lock").exists()
                    || workspace.root.join("bun.lockb").exists()
                    || workspace.root.join("npm-shrinkwrap.json").exists()
            }
            PackageKind::Hex => workspace.root.join("mix.lock").exists(),
            PackageKind::PyPI => workspace.root.join("uv.lock").exists(),
            PackageKind::Packagist => workspace.root.join("composer.lock").exists(),
        };

        if lockfile_exists && let Err(e) = adapter.regenerate_lockfile(&workspace.root) {
            errors.push((kind, e.to_string()));
        }
    }

    // If there were errors, report them but don't fail the release
    if !errors.is_empty() {
        for (kind, err) in errors {
            eprintln!(
                "Warning: failed to regenerate {} lockfile: {}",
                kind.display_name(),
                err
            );
        }
    }

    Ok(())
}

/// Compute initial bumps from changesets and collect messages
fn compute_initial_bumps(
    changesets: &[ChangesetInfo],
    ws: &Workspace,
    cfg: &Config,
) -> Result<InitialBumpsResult> {
    let mut bump_by_pkg: BTreeMap<String, Bump> = BTreeMap::new();
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, ChangelogCategory)>> = BTreeMap::new();
    let mut used_paths: BTreeSet<std::path::PathBuf> = BTreeSet::new();

    // Resolve GitHub repo slug once if available (config, env or origin remote)
    let repo_slug = detect_github_repo_slug_with_config(&ws.root, cfg.github_repository.as_deref());
    let github_token = std::env::var("GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GH_TOKEN").ok());

    for cs in changesets {
        let mut consumed_changeset = false;
        for (spec, bump, tag) in &cs.entries {
            let info = resolve_package_spec(ws, spec)?;
            if should_ignore_package(cfg, ws, info)? {
                continue;
            }

            // Mark this changeset as consumed since at least one package is applicable
            consumed_changeset = true;

            let identifier = info.canonical_identifier().to_string();

            bump_by_pkg
                .entry(identifier.clone())
                .and_modify(|b| {
                    if *bump > *b {
                        *b = *bump;
                    }
                })
                .or_insert(*bump);

            // Enrich message with commit info and acknowledgments
            let commit_hash = get_commit_hash_for_path(&ws.root, &cs.path);
            let enriched = if let Some(hash) = commit_hash {
                enrich_changeset_message(
                    &cs.message,
                    &hash,
                    &ws.root,
                    repo_slug.as_deref(),
                    github_token.as_deref(),
                    cfg.changelog_show_commit_hash,
                    cfg.changelog_show_acknowledgments,
                )
            } else {
                cs.message.clone()
            };

            // Determine changelog category based on tag presence
            let category = match tag {
                Some(t) => ChangelogCategory::Tag(t.clone()),
                None => ChangelogCategory::Bump(*bump),
            };

            messages_by_pkg
                .entry(identifier)
                .or_default()
                .push((enriched, category));
        }
        if consumed_changeset {
            used_paths.insert(cs.path.clone());
        }
    }

    Ok((bump_by_pkg, messages_by_pkg, used_paths))
}

/// Build reverse dependency graph: dep -> set of dependents
/// Only includes non-ignored packages in the graph
fn build_dependency_graph(ws: &Workspace, cfg: &Config) -> BTreeMap<String, BTreeSet<String>> {
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    // Build a set of ignored package names for quick lookup
    let ignored_packages: BTreeSet<String> = ws
        .members
        .iter()
        .filter(|c| should_ignore_package(cfg, ws, c).unwrap_or(false))
        .map(|c| c.canonical_identifier().to_string())
        .collect();

    for c in &ws.members {
        // Skip ignored packages when building the dependency graph
        let identifier = c.canonical_identifier();
        if ignored_packages.contains(identifier) {
            continue;
        }

        for dep in &c.internal_deps {
            // Also skip dependencies that point to ignored packages
            if ignored_packages.contains(dep) {
                continue;
            }

            dependents
                .entry(dep.clone())
                .or_default()
                .insert(identifier.to_string());
        }
    }
    dependents
}

/// Apply dependency cascade logic and fixed dependency groups
fn apply_dependency_cascade(
    bump_by_pkg: &mut BTreeMap<String, Bump>,
    dependents: &BTreeMap<String, BTreeSet<String>>,
    cfg: &Config,
    ws: &Workspace,
) -> Result<()> {
    let resolved_fixed_groups =
        resolve_config_groups(ws, &cfg.fixed_dependencies, "packages.fixed")?;

    // Helper function to find which fixed group a package belongs to, if any
    let find_fixed_group = |pkg_id: &str| -> Option<usize> {
        resolved_fixed_groups
            .iter()
            .position(|group| group.contains(&pkg_id.to_string()))
    };

    // Build a quick lookup map for package info by canonical identifier
    let mut by_id: BTreeMap<String, &PackageInfo> = BTreeMap::new();
    for c in &ws.members {
        by_id.insert(c.canonical_identifier().to_string(), c);
    }

    let mut queue: Vec<String> = bump_by_pkg.keys().cloned().collect();
    let mut seen: BTreeSet<String> = queue.iter().cloned().collect();

    while let Some(changed) = queue.pop() {
        let changed_bump = bump_by_pkg.get(&changed).copied().unwrap_or(Bump::Patch);

        // 1. Handle normal dependency relationships (unchanged → dependent)
        if let Some(deps) = dependents.get(&changed) {
            for dep_name in deps {
                // Check if this dependent package should be ignored
                if let Some(info) = by_id.get(dep_name) {
                    match should_ignore_package(cfg, ws, info) {
                        Ok(true) => continue,
                        Ok(false) => {} // Continue processing
                        Err(_) => {
                            // On I/O error reading manifest, err on the side of not ignoring
                            // This maintains backwards compatibility and avoids silent failures
                        }
                    }
                }

                // Determine bump level for this dependent
                let dependent_bump = if find_fixed_group(dep_name).is_some() {
                    // Fixed dependencies: same bump level as the dependency
                    changed_bump
                } else {
                    // Normal dependencies: at least patch
                    Bump::Patch
                };

                let entry = bump_by_pkg
                    .entry(dep_name.clone())
                    .or_insert(dependent_bump);
                // If already present, keep the higher bump
                if *entry < dependent_bump {
                    *entry = dependent_bump;
                }
                if !seen.contains(dep_name) {
                    queue.push(dep_name.clone());
                    seen.insert(dep_name.clone());
                }
            }
        }

        // 2. Handle fixed dependency groups (bidirectional)
        if let Some(group_idx) = find_fixed_group(&changed) {
            // All packages in the same fixed group should bump together
            for group_member in &resolved_fixed_groups[group_idx] {
                if group_member == &changed {
                    continue;
                }

                // Check if this group member should be ignored
                if let Some(info) = by_id.get(group_member) {
                    match should_ignore_package(cfg, ws, info) {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(_) => {
                            // On I/O error reading manifest, err on the side of not ignoring
                            // This maintains backwards compatibility and avoids silent failures
                        }
                    }
                }

                let entry = bump_by_pkg
                    .entry(group_member.clone())
                    .or_insert(changed_bump);
                // If already present, keep the higher bump
                if *entry < changed_bump {
                    *entry = changed_bump;
                }
                if !seen.contains(group_member) {
                    queue.push(group_member.clone());
                    seen.insert(group_member.clone());
                }
            }
        }
    }

    Ok(())
}

/// Apply linked dependencies logic: highest bump level to affected packages only
fn apply_linked_dependencies(
    bump_by_pkg: &mut BTreeMap<String, Bump>,
    cfg: &Config,
    ws: &Workspace,
) -> Result<()> {
    let resolved_groups = resolve_config_groups(ws, &cfg.linked_dependencies, "packages.linked")?;

    for group in &resolved_groups {
        // Check if any package in this group has been bumped
        let mut group_has_bumps = false;
        let mut highest_bump = Bump::Patch;

        // First pass: find the highest bump level in the group among affected packages
        for group_member in group {
            if let Some(&member_bump) = bump_by_pkg.get(group_member) {
                group_has_bumps = true;
                if member_bump > highest_bump {
                    highest_bump = member_bump;
                }
            }
        }

        // If any package in the group is being bumped, apply highest bump to affected packages only
        if group_has_bumps {
            // Apply the highest bump level to packages that are already being bumped
            // (either directly affected or through dependency cascade)
            for group_member in group {
                if bump_by_pkg.contains_key(group_member) {
                    // Only update if the current bump is lower than the group's highest bump
                    let current_bump = bump_by_pkg
                        .get(group_member)
                        .copied()
                        .unwrap_or(Bump::Patch);
                    if highest_bump > current_bump {
                        bump_by_pkg.insert(group_member.clone(), highest_bump);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Prepare the release plan by matching bumps to workspace members
fn prepare_release_plan(
    bump_by_pkg: &BTreeMap<String, Bump>,
    ws: &Workspace,
) -> Result<ReleasePlan> {
    // Map package identifier -> PackageInfo for quick lookup
    let mut by_id: BTreeMap<String, &PackageInfo> = BTreeMap::new();
    for c in &ws.members {
        by_id.insert(c.canonical_identifier().to_string(), c);
    }

    let mut releases: Vec<(String, String, String)> = Vec::new(); // (name, old_version, new_version)
    for (identifier, bump) in bump_by_pkg {
        if let Some(info) = by_id.get(identifier) {
            let old = if info.version.is_empty() {
                "0.0.0".to_string()
            } else {
                info.version.clone()
            };

            let newv = bump_version(&old, *bump).unwrap_or_else(|_| old.clone());

            releases.push((identifier.clone(), old, newv));
        }
    }

    Ok(releases)
}

/// Print the planned releases
fn print_release_plan(workspace: &Workspace, releases: &ReleasePlan) {
    let include_kind = workspace.has_multiple_package_kinds();
    println!("Planned releases:");
    for (identifier, old, newv) in releases {
        let display = workspace
            .find_by_identifier(identifier)
            .map(|info| info.display_name(include_kind))
            .or_else(|| {
                PackageSpecifier::parse(identifier)
                    .ok()
                    .map(|spec| spec.display_name(include_kind))
            })
            .unwrap_or_else(|| identifier.clone());
        println!("  {display}: {old} -> {newv}");
    }
}

#[derive(Debug, Clone, Copy)]
enum ReleaseDateTimezone {
    Local,
    Utc,
    Offset(FixedOffset),
    Named(Tz),
}

fn parse_release_date_timezone(spec: &str) -> Result<ReleaseDateTimezone> {
    let trimmed = spec.trim();
    let invalid_value = || {
        SampoError::Config(format!(
            "Unsupported changelog.release_date_timezone value '{trimmed}'. Use 'UTC', 'local', a fixed offset like '+02:00', or an IANA timezone name such as 'Europe/Paris'."
        ))
    };
    if trimmed.is_empty() {
        return Ok(ReleaseDateTimezone::Local);
    }

    if trimmed.eq_ignore_ascii_case("local") {
        return Ok(ReleaseDateTimezone::Local);
    }

    if trimmed.eq_ignore_ascii_case("utc") || trimmed.eq_ignore_ascii_case("z") {
        return Ok(ReleaseDateTimezone::Utc);
    }

    if let Ok(zone) = trimmed.parse::<Tz>() {
        return Ok(ReleaseDateTimezone::Named(zone));
    }

    let bytes = trimmed.as_bytes();
    if bytes.len() < 2 {
        return Err(invalid_value());
    }

    let sign = match bytes[0] as char {
        '+' => 1,
        '-' => -1,
        _ => return Err(invalid_value()),
    };

    let remainder = &trimmed[1..];
    if remainder.is_empty() {
        return Err(invalid_value());
    }

    let (hour_part, minute_part) = if let Some(idx) = remainder.find(':') {
        let (h, m) = remainder.split_at(idx);
        if m.len() < 2 {
            return Err(invalid_value());
        }
        (h, &m[1..])
    } else if remainder.len() == 4 {
        (&remainder[..2], &remainder[2..])
    } else if remainder.len() == 2 {
        (remainder, "00")
    } else {
        return Err(invalid_value());
    };

    let hours: u32 = hour_part.parse().map_err(|_| invalid_value())?;
    let minutes: u32 = minute_part.parse().map_err(|_| invalid_value())?;

    if hours > 23 || minutes > 59 {
        return Err(SampoError::Config(format!(
            "Unsupported changelog.release_date_timezone value '{trimmed}'. Hours must be <= 23 and minutes <= 59."
        )));
    }

    let total_seconds = (hours * 3600 + minutes * 60) as i32;
    let offset = if sign >= 0 {
        FixedOffset::east_opt(total_seconds)
    } else {
        FixedOffset::west_opt(total_seconds)
    };

    match offset {
        Some(value) => Ok(ReleaseDateTimezone::Offset(value)),
        None => Err(SampoError::Config(format!(
            "Unsupported changelog.release_date_timezone value '{trimmed}'. Offset is out of range."
        ))),
    }
}

fn compute_release_date_display(cfg: &Config) -> Result<Option<String>> {
    compute_release_date_display_with_now(cfg, Utc::now())
}

fn compute_release_date_display_with_now(
    cfg: &Config,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    if !cfg.changelog_show_release_date {
        return Ok(None);
    }

    let format_str = cfg.changelog_release_date_format.trim();
    if format_str.is_empty() {
        return Ok(None);
    }

    let timezone_pref = cfg
        .changelog_release_date_timezone
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_release_date_timezone)
        .transpose()?;

    let tz = timezone_pref.unwrap_or(ReleaseDateTimezone::Local);

    let formatted = match tz {
        ReleaseDateTimezone::Local => now.with_timezone(&Local).format(format_str).to_string(),
        ReleaseDateTimezone::Utc => now.format(format_str).to_string(),
        ReleaseDateTimezone::Offset(offset) => {
            now.with_timezone(&offset).format(format_str).to_string()
        }
        ReleaseDateTimezone::Named(zone) => now.with_timezone(&zone).format(format_str).to_string(),
    };

    Ok(Some(formatted))
}

/// Apply all releases: update manifests and changelogs
fn apply_releases(
    releases: &ReleasePlan,
    ws: &Workspace,
    messages_by_pkg: &mut BTreeMap<String, Vec<(String, ChangelogCategory)>>,
    changesets: &[ChangesetInfo],
    cfg: &Config,
) -> Result<()> {
    // Build lookup map for all packages
    let mut by_id: BTreeMap<String, &PackageInfo> = BTreeMap::new();
    for c in &ws.members {
        by_id.insert(c.canonical_identifier().to_string(), c);
    }

    let has_cargo = ws.members.iter().any(|pkg| pkg.kind == PackageKind::Cargo);
    let manifest_metadata = if has_cargo {
        Some(ManifestMetadata::load(ws)?)
    } else {
        None
    };

    // Build releases map for dependency explanations
    let releases_map: BTreeMap<String, (String, String)> = releases
        .iter()
        .map(|(name, old, new)| (name.clone(), (old.clone(), new.clone())))
        .collect();

    let mut new_version_by_name: BTreeMap<String, String> = BTreeMap::new();
    for (identifier, _old, newv) in releases {
        if let Some(info) = by_id.get(identifier) {
            new_version_by_name.insert(info.name.clone(), newv.clone());
        }
    }

    // Use unified function to detect all dependency explanations
    let dependency_explanations =
        detect_all_dependency_explanations(changesets, ws, cfg, &releases_map)?;

    // Merge dependency explanations into existing messages
    for (pkg_name, explanations) in dependency_explanations {
        messages_by_pkg
            .entry(pkg_name)
            .or_default()
            .extend(explanations);
    }

    let release_date_display = compute_release_date_display(cfg)?;

    // Apply updates for each release
    for (name, old, newv) in releases {
        let info = by_id
            .get(name.as_str())
            .ok_or_else(|| SampoError::Release(format!("package '{}' not found", name)))?;
        let adapter = match info.kind {
            PackageKind::Cargo => crate::adapters::PackageAdapter::Cargo,
            PackageKind::Npm => crate::adapters::PackageAdapter::Npm,
            PackageKind::Hex => crate::adapters::PackageAdapter::Hex,
            PackageKind::PyPI => crate::adapters::PackageAdapter::PyPI,
            PackageKind::Packagist => crate::adapters::PackageAdapter::Packagist,
        };
        let manifest_path = adapter.manifest_path(&info.path);
        let text = fs::read_to_string(&manifest_path)?;

        // Update manifest versions
        let cargo_metadata = match adapter {
            PackageAdapter::Cargo => manifest_metadata.as_ref(),
            PackageAdapter::Npm
            | PackageAdapter::Hex
            | PackageAdapter::PyPI
            | PackageAdapter::Packagist => None,
        };
        let (updated, _dep_updates) = adapter.update_manifest_versions(
            &manifest_path,
            &text,
            Some(newv.as_str()),
            &new_version_by_name,
            cargo_metadata,
        )?;
        fs::write(&manifest_path, updated)?;

        let messages = messages_by_pkg.get(name).cloned().unwrap_or_default();
        update_changelog(
            &info.path,
            &info.name,
            old,
            newv,
            &messages,
            release_date_display.as_deref(),
        )?;
    }

    Ok(())
}

fn normalize_version_input(input: &str) -> std::result::Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Version string cannot be empty".to_string());
    }

    let boundary = trimmed
        .find(|ch: char| ['-', '+'].contains(&ch))
        .unwrap_or(trimmed.len());
    let (core, rest) = trimmed.split_at(boundary);

    let parts: Vec<&str> = if core.is_empty() {
        Vec::new()
    } else {
        core.split('.').collect()
    };

    if parts.is_empty() || parts.len() > 3 {
        return Err(format!(
            "Invalid semantic version '{input}': expected one to three numeric components"
        ));
    }

    let mut normalized_parts = Vec::with_capacity(3);
    for part in &parts {
        if part.is_empty() {
            return Err(format!(
                "Invalid semantic version '{input}': found empty numeric component"
            ));
        }
        normalized_parts.push(*part);
    }
    while normalized_parts.len() < 3 {
        normalized_parts.push("0");
    }

    let normalized_core = normalized_parts.join(".");
    Ok(format!("{normalized_core}{rest}"))
}

pub(crate) fn parse_version_string(input: &str) -> std::result::Result<Version, String> {
    let normalized = normalize_version_input(input)?;
    Version::parse(&normalized).map_err(|err| format!("Invalid semantic version '{input}': {err}"))
}

fn implied_prerelease_bump(version: &Version) -> std::result::Result<Bump, String> {
    if version.pre.is_empty() {
        return Err("Version does not contain a pre-release identifier".to_string());
    }

    if version.minor == 0 && version.patch == 0 {
        Ok(Bump::Major)
    } else if version.patch == 0 {
        Ok(Bump::Minor)
    } else {
        Ok(Bump::Patch)
    }
}

fn increment_prerelease(pre: &Prerelease) -> std::result::Result<Prerelease, String> {
    if pre.is_empty() {
        return Err("Pre-release identifier missing".to_string());
    }

    let mut parts: Vec<String> = pre.as_str().split('.').map(|s| s.to_string()).collect();
    if parts.is_empty() {
        return Err("Pre-release identifier missing".to_string());
    }

    let last_is_numeric = parts
        .last()
        .map(|part| part.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false);

    if last_is_numeric {
        let value = parts
            .last()
            .unwrap()
            .parse::<u64>()
            .map_err(|_| "Pre-release component is not a valid number".to_string())?;
        let incremented = value
            .checked_add(1)
            .ok_or_else(|| "Pre-release counter overflow".to_string())?;
        *parts.last_mut().unwrap() = incremented.to_string();
    } else {
        parts.push("1".to_string());
    }

    let candidate = parts.join(".");
    Prerelease::new(&candidate).map_err(|err| format!("Invalid pre-release '{candidate}': {err}"))
}

fn strip_trailing_numeric_identifiers(pre: &Prerelease) -> Option<Prerelease> {
    if pre.is_empty() {
        return None;
    }

    let mut parts: Vec<&str> = pre.as_str().split('.').collect();
    while let Some(last) = parts.last() {
        if last.chars().all(|ch| ch.is_ascii_digit()) {
            parts.pop();
        } else {
            break;
        }
    }

    if parts.is_empty() {
        None
    } else {
        let candidate = parts.join(".");
        Prerelease::new(&candidate).ok()
    }
}

fn apply_base_bump(version: &mut Version, bump: Bump) -> std::result::Result<(), String> {
    match bump {
        Bump::Patch => {
            version.patch = version
                .patch
                .checked_add(1)
                .ok_or_else(|| "Patch component overflow".to_string())?;
        }
        Bump::Minor => {
            version.minor = version
                .minor
                .checked_add(1)
                .ok_or_else(|| "Minor component overflow".to_string())?;
            version.patch = 0;
        }
        Bump::Major => {
            version.major = version
                .major
                .checked_add(1)
                .ok_or_else(|| "Major component overflow".to_string())?;
            version.minor = 0;
            version.patch = 0;
        }
    }
    version.pre = Prerelease::EMPTY;
    version.build = BuildMetadata::EMPTY;
    Ok(())
}

/// Bump a semver version string, including pre-release handling
pub fn bump_version(old: &str, bump: Bump) -> std::result::Result<String, String> {
    let mut version = parse_version_string(old)?;
    let original_pre = version.pre.clone();

    if original_pre.is_empty() {
        apply_base_bump(&mut version, bump)?;
        return Ok(version.to_string());
    }

    let implied = implied_prerelease_bump(&version)?;

    if bump <= implied {
        version.pre = increment_prerelease(&original_pre)?;
        version.build = BuildMetadata::EMPTY;
        Ok(version.to_string())
    } else {
        let base_pre = strip_trailing_numeric_identifiers(&original_pre).ok_or_else(|| {
            format!(
                "Pre-release version '{old}' must include a non-numeric identifier before the counter"
            )
        })?;

        apply_base_bump(&mut version, bump)?;
        version.pre = base_pre;
        Ok(version.to_string())
    }
}

fn split_intro_and_versions(body: &str) -> (&str, &str) {
    let mut offset = 0;
    let len = body.len();
    while offset < len {
        if body[offset..].starts_with("## ") {
            return body.split_at(offset);
        }

        match body[offset..].find('\n') {
            Some(newline_offset) => {
                offset += newline_offset + 1;
            }
            None => break,
        }
    }

    (body, "")
}

fn header_matches_release_version(header_text: &str, version: &str) -> bool {
    if header_text == version {
        return true;
    }

    header_text
        .strip_prefix(version)
        .map(|rest| {
            let trimmed = rest.trim_start();
            trimmed.is_empty() || trimmed.starts_with('—') || trimmed.starts_with('-')
        })
        .unwrap_or(false)
}

fn update_changelog(
    crate_dir: &Path,
    package: &str,
    old_version: &str,
    new_version: &str,
    entries: &[(String, ChangelogCategory)],
    release_date_display: Option<&str>,
) -> Result<()> {
    let path = crate_dir.join("CHANGELOG.md");
    let existing = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let cleaned = existing.trim_start_matches('\u{feff}');
    let (intro_part, versions_part) = split_intro_and_versions(cleaned);
    let mut intro = intro_part.to_string();
    let mut versions_body = versions_part.to_string();

    if intro.trim().is_empty() {
        intro = format!("# {}\n\n", package);
    }

    // Determine if we're using custom tags (any entry has a Tag category)
    let uses_custom_tags = entries
        .iter()
        .any(|(_, cat)| matches!(cat, ChangelogCategory::Tag(_)));

    // Group entries by heading. Use IndexMap-like behavior to preserve insertion order.
    // We use a Vec of (heading, Vec<messages>) to maintain order.
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();

    // helper to push without duplicates within a section
    let push_unique_to_section =
        |sections: &mut Vec<(String, Vec<String>)>, heading: &str, msg: &str| {
            if let Some((_h, messages)) = sections.iter_mut().find(|(h, _)| h == heading) {
                if !messages.iter().any(|m| m == msg) {
                    messages.push(msg.to_string());
                }
            } else {
                sections.push((heading.to_string(), vec![msg.to_string()]));
            }
        };

    // Collect new entries
    for (msg, category) in entries {
        let heading = category.heading();
        push_unique_to_section(&mut sections, &heading, msg);
    }

    // Parse and merge the current top section only if it's an unpublished section.
    // If header == old_version => preserve it (do not merge or strip).
    let trimmed = versions_body.trim_start();
    if trimmed.starts_with("## ") {
        let mut lines_iter = trimmed.lines();
        let header_line = lines_iter.next().unwrap_or("").trim();
        let header_text = header_line.trim_start_matches("## ").trim();

        let is_published_top = header_matches_release_version(header_text, old_version);

        if !is_published_top {
            let after_header_offset = header_line.len();
            let rest_after_header = &trimmed[after_header_offset..];
            let next_rel = rest_after_header.find("\n## ");
            let (section_text, remaining) = match next_rel {
                Some(pos) => {
                    let end = after_header_offset + pos + 1;
                    (&trimmed[..end], &trimmed[end..])
                }
                None => (trimmed, ""),
            };

            // Parse existing section headings - support both bump-based and custom tags
            let mut current_heading: Option<String> = None;
            for line in section_text.lines() {
                let t = line.trim();
                if t.starts_with("### ") {
                    let heading_text = t.trim_start_matches("### ").trim();
                    current_heading = Some(heading_text.to_string());
                    continue;
                }
                if t.starts_with("- ")
                    && let Some(ref heading) = current_heading
                {
                    let msg = t.trim_start_matches("- ").trim();
                    push_unique_to_section(&mut sections, heading, msg);
                }
            }

            versions_body = remaining.to_string();
        }
    }

    // Build new aggregated top section
    let mut section = String::new();
    match release_date_display.and_then(|d| (!d.trim().is_empty()).then_some(d)) {
        Some(date) => section.push_str(&format!("## {new_version} — {date}\n\n")),
        None => section.push_str(&format!("## {new_version}\n\n")),
    }

    // When using bump-based sections (default), render in Major > Minor > Patch order.
    // When using custom tags, render in order of first appearance.
    if !uses_custom_tags {
        // Sort sections by bump order
        let bump_order = |heading: &str| -> u8 {
            match heading {
                "Major changes" => 0,
                "Minor changes" => 1,
                "Patch changes" => 2,
                _ => 3, // Unknown headings go last
            }
        };
        sections.sort_by(|(a, _), (b, _)| bump_order(a).cmp(&bump_order(b)));
    }

    // Render sections
    for (heading, messages) in &sections {
        if !messages.is_empty() {
            section.push_str(&format!("### {}\n\n", heading));
            for msg in messages {
                section.push_str(&crate::markdown::format_markdown_list_item(msg));
            }
            section.push('\n');
        }
    }

    let mut combined = String::new();
    combined.push_str(&intro);

    if !combined.is_empty() && !combined.ends_with("\n\n") {
        if combined.ends_with('\n') {
            combined.push('\n');
        } else {
            combined.push_str("\n\n");
        }
    }

    combined.push_str(&section);

    if !versions_body.trim().is_empty() {
        if !combined.ends_with("\n\n") {
            if combined.ends_with('\n') {
                combined.push('\n');
            } else {
                combined.push_str("\n\n");
            }
        }
        combined.push_str(&versions_body);
    }

    fs::write(&path, combined)?;
    Ok(())
}

/// Validate fixed dependencies configuration against the workspace
fn validate_fixed_dependencies(config: &Config, workspace: &Workspace) -> Result<()> {
    resolve_config_groups(workspace, &config.fixed_dependencies, "packages.fixed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::collections::BTreeMap;

    #[test]
    fn preserves_changelog_intro_when_updating() {
        use std::fs;
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let crate_dir = temp.path();
        let intro = "# Custom Changelog Header\n\nIntro text before versions.\n\n";
        let existing = format!(
            "{}## 1.0.0 — 2024-06-19\n\n### Patch changes\n\n- Existing entry\n",
            intro
        );
        fs::write(crate_dir.join("CHANGELOG.md"), existing).unwrap();

        let entries = vec![(
            "Add new feature".to_string(),
            ChangelogCategory::Bump(Bump::Minor),
        )];
        update_changelog(
            crate_dir,
            "my-package",
            "1.0.0",
            "1.0.1",
            &entries,
            Some("2024-06-20"),
        )
        .unwrap();

        let updated = fs::read_to_string(crate_dir.join("CHANGELOG.md")).unwrap();
        assert!(updated.starts_with(intro));

        let new_idx = updated.find("## 1.0.1").unwrap();
        let old_idx = updated.find("## 1.0.0").unwrap();
        assert!(new_idx >= intro.len());
        assert!(new_idx < old_idx);
        assert!(updated.contains("## 1.0.1 — 2024-06-20"));
        assert!(updated.contains("- Add new feature"));
        assert!(updated.contains("- Existing entry"));
    }

    #[test]
    fn creates_default_header_when_missing_intro() {
        use std::fs;
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let crate_dir = temp.path();

        let entries = vec![(
            "Initial release".to_string(),
            ChangelogCategory::Bump(Bump::Major),
        )];
        update_changelog(crate_dir, "new-package", "0.1.0", "1.0.0", &entries, None).unwrap();

        let updated = fs::read_to_string(crate_dir.join("CHANGELOG.md")).unwrap();
        assert!(updated.starts_with("# new-package\n\n## 1.0.0"));
    }

    #[test]
    fn header_matches_release_version_handles_suffixes() {
        assert!(header_matches_release_version("1.0.0", "1.0.0"));
        assert!(header_matches_release_version(
            "1.0.0 — 2024-06-20",
            "1.0.0"
        ));
        assert!(header_matches_release_version("1.0.0-2024-06-20", "1.0.0"));
        assert!(!header_matches_release_version(
            "1.0.1 — 2024-06-20",
            "1.0.0"
        ));
    }

    #[test]
    fn update_changelog_skips_blank_release_date() {
        use std::fs;
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let crate_dir = temp.path();
        let entries = vec![("Bug fix".to_string(), ChangelogCategory::Bump(Bump::Patch))];

        update_changelog(
            crate_dir,
            "blank-date",
            "0.1.0",
            "0.1.1",
            &entries,
            Some("   "),
        )
        .unwrap();

        let updated = fs::read_to_string(crate_dir.join("CHANGELOG.md")).unwrap();
        assert!(updated.contains("## 0.1.1\n"));
        assert!(!updated.contains("—"));
    }

    #[test]
    fn parse_release_date_timezone_accepts_utc() {
        match parse_release_date_timezone("UTC").unwrap() {
            ReleaseDateTimezone::Utc => {}
            _ => panic!("Expected UTC timezone"),
        }
    }

    #[test]
    fn parse_release_date_timezone_accepts_offset() {
        match parse_release_date_timezone("+05:45").unwrap() {
            ReleaseDateTimezone::Offset(offset) => {
                assert_eq!(offset.local_minus_utc(), 5 * 3600 + 45 * 60);
            }
            _ => panic!("Expected fixed offset"),
        }
    }

    #[test]
    fn parse_release_date_timezone_rejects_invalid() {
        let err = parse_release_date_timezone("Not/AZone").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("release_date_timezone"));
    }

    #[test]
    fn compute_release_date_display_uses_utc() {
        let cfg = Config {
            changelog_release_date_format: "%Z".to_string(),
            changelog_release_date_timezone: Some("UTC".to_string()),
            ..Default::default()
        };

        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let display = compute_release_date_display_with_now(&cfg, now)
            .unwrap()
            .unwrap();
        assert_eq!(display, "UTC");
    }

    #[test]
    fn parse_release_date_timezone_accepts_named_zone() {
        match parse_release_date_timezone("Europe/Paris").unwrap() {
            ReleaseDateTimezone::Named(zone) => {
                assert_eq!(zone, chrono_tz::Europe::Paris);
            }
            _ => panic!("Expected named timezone"),
        }
    }

    #[test]
    fn compute_release_date_display_uses_offset() {
        let cfg = Config {
            changelog_release_date_format: "%z".to_string(),
            changelog_release_date_timezone: Some("-03:30".to_string()),
            ..Default::default()
        };

        let now = Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let display = compute_release_date_display_with_now(&cfg, now)
            .unwrap()
            .unwrap();
        assert_eq!(display, "-0330");
    }

    #[test]
    fn compute_release_date_display_uses_named_zone() {
        let cfg = Config {
            changelog_release_date_format: "%Z".to_string(),
            changelog_release_date_timezone: Some("America/New_York".to_string()),
            ..Default::default()
        };

        let now = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let display = compute_release_date_display_with_now(&cfg, now)
            .unwrap()
            .unwrap();
        assert_eq!(display, "EST");
    }

    #[test]
    fn test_ignore_packages_in_dependency_cascade() {
        use crate::types::{PackageInfo, PackageKind, Workspace};
        use std::path::PathBuf;

        // Create a mock workspace with packages
        let root = PathBuf::from("/tmp/test");
        let workspace = Workspace {
            root: root.clone(),
            members: vec![
                PackageInfo {
                    name: "main-package".to_string(),
                    identifier: "cargo/main-package".to_string(),
                    version: "1.0.0".to_string(),
                    path: root.join("main-package"),
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "examples-package".to_string(),
                    identifier: "cargo/examples-package".to_string(),
                    version: "1.0.0".to_string(),
                    path: root.join("examples/package"),
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "benchmarks-utils".to_string(),
                    identifier: "cargo/benchmarks-utils".to_string(),
                    version: "1.0.0".to_string(),
                    path: root.join("benchmarks/utils"),
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
            ],
        };

        // Create a config that ignores examples/* and benchmarks/*
        let config = Config {
            ignore: vec!["examples/*".to_string(), "benchmarks/*".to_string()],
            ..Default::default()
        };

        // Create a dependency graph where main-package depends on the ignored packages
        let mut dependents = BTreeMap::new();
        dependents.insert(
            "cargo/main-package".to_string(),
            ["cargo/examples-package", "cargo/benchmarks-utils"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );

        // Start with main-package being bumped
        let mut bump_by_pkg = BTreeMap::new();
        bump_by_pkg.insert("cargo/main-package".to_string(), Bump::Minor);

        // Apply dependency cascade
        apply_dependency_cascade(&mut bump_by_pkg, &dependents, &config, &workspace).unwrap();

        // The ignored packages should NOT be added to bump_by_pkg
        assert_eq!(bump_by_pkg.len(), 1);
        assert!(bump_by_pkg.contains_key("cargo/main-package"));
        assert!(!bump_by_pkg.contains_key("cargo/examples-package"));
        assert!(!bump_by_pkg.contains_key("cargo/benchmarks-utils"));
    }

    #[test]
    fn test_ignored_packages_excluded_from_dependency_graph() {
        use crate::types::{PackageInfo, PackageKind, Workspace};
        use std::collections::BTreeSet;
        use std::path::PathBuf;

        let root = PathBuf::from("/tmp/test");
        let workspace = Workspace {
            root: root.clone(),
            members: vec![
                PackageInfo {
                    name: "main-package".to_string(),
                    identifier: "cargo/main-package".to_string(),
                    version: "1.0.0".to_string(),
                    path: root.join("main-package"),
                    internal_deps: ["cargo/examples-package".to_string()].into_iter().collect(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "examples-package".to_string(),
                    identifier: "cargo/examples-package".to_string(),
                    version: "1.0.0".to_string(),
                    path: root.join("examples/package"),
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
            ],
        };

        // Config that ignores examples/*
        let config = Config {
            ignore: vec!["examples/*".to_string()],
            ..Default::default()
        };

        // Build dependency graph
        let dependents = build_dependency_graph(&workspace, &config);

        // examples-package should not appear in the dependency graph because it's ignored
        // So main-package should not appear as a dependent of examples-package
        assert!(!dependents.contains_key("cargo/examples-package"));

        // The dependency graph should be empty since examples-package is ignored
        // and main-package depends on it
        assert!(dependents.is_empty());
    }
}
