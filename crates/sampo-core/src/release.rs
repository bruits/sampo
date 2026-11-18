use crate::adapters::PackageAdapter;
use crate::adapters::cargo::{self, ManifestMetadata, update_workspace_root_manifest};
use crate::errors::{Result, SampoError, io_error_with_path};
use crate::filters::should_ignore_package;
use crate::types::{
    Bump, ChangelogCategory, ConstraintCheckResult, ConstraintViolation, DependencyUpdate,
    PackageInfo, PackageKind, PackageSpecifier, ReleaseOutput, ReleasedPackage, SpecResolution,
    Workspace, format_ambiguity_options,
};
use crate::{
    AcknowledgmentStyle, Acknowledgments,
    changeset::{ChangesetInfo, parse_changeset, render_changeset_markdown_with_tags},
    config::{AutomaticEntries, BumpDependents, Config, Versioning},
    current_branch, detect_github_repo_slug_with_config, discover_workspace,
    enrich_changeset_message, get_commit_hash_for_path, load_changesets,
    publish::package_tag_exists,
};
use chrono::{DateTime, Datelike, FixedOffset, Local, Utc};
use chrono_tz::Tz;
use semver::{BuildMetadata, Prerelease, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Sole content of the changelog section for releases with only automatic entries, when
/// `changelog.automatic_entries` is `"collapse"`.
const VERSION_BUMP_ONLY_ENTRY: &str = "Version bump only, to follow other packages.";

/// Joins the first and last version in the heading of a collapsed section.
const VERSION_RANGE_SEPARATOR: &str = "...";

/// Format dependency updates for changelog display
///
/// Creates a message in the style of Changesets for dependency updates,
/// e.g., "Updated dependencies [hash]: pkg1@1.2.0, pkg2@2.0.0"
pub fn format_dependency_updates_message(updates: &[DependencyUpdate]) -> Option<String> {
    if updates.is_empty() {
        return None;
    }

    let mut parsed_updates: Vec<(Option<PackageSpecifier>, String, &DependencyUpdate)> =
        Vec::with_capacity(updates.len());
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
            parsed_updates.push((Some(spec), base_name, dep));
        } else {
            let base_name = dep.name.clone();
            labels_by_name.entry(base_name.clone()).or_default();
            parsed_updates.push((None, base_name, dep));
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
        .map(|(spec_opt, base_name, dep)| {
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

/// The fixed groups Sampo enforces: the user's `packages.fixed` config plus any implicit
/// groups an ecosystem derives from its structure (Maven parent-inherited versions).
/// Overlapping groups are merged so no package lands in two.
fn enforced_fixed_groups(workspace: &Workspace, config: &Config) -> Result<Vec<Vec<String>>> {
    let mut groups =
        resolve_config_groups(workspace, &config.fixed_dependencies, "packages.fixed")?;
    groups.extend(PackageAdapter::implicit_fixed_groups(workspace));
    Ok(merge_overlapping_groups(groups))
}

/// Merge any groups that share a member into one, so membership is unambiguous.
pub(crate) fn merge_overlapping_groups(groups: Vec<Vec<String>>) -> Vec<Vec<String>> {
    let mut sets: Vec<BTreeSet<String>> = groups
        .into_iter()
        .map(|group| group.into_iter().collect::<BTreeSet<String>>())
        .filter(|set| !set.is_empty())
        .collect();

    let mut merged = true;
    while merged {
        merged = false;
        'scan: for i in 0..sets.len() {
            for j in (i + 1)..sets.len() {
                if sets[i].intersection(&sets[j]).next().is_some() {
                    let other = sets.remove(j);
                    sets[i].extend(other);
                    merged = true;
                    break 'scan;
                }
            }
        }
    }

    sets.into_iter()
        .map(|set| set.into_iter().collect())
        .collect()
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

/// Create a changelog entry for a package an ecosystem couples structurally
///
/// Distinct from the fixed-group entry: this coupling is derived from the manifests, so
/// citing a `packages.fixed` policy would name config the user never wrote.
///
/// Returns a tuple of (message, bump_type) suitable for adding to changelog messages
pub fn create_structural_version_coupling_entry(bump: Bump) -> (String, Bump) {
    (
        "Bumped to stay aligned with the packages it shares a version with".to_string(),
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

/// Build automatic changelog entries for a completed release plan.
/// The release index and the result use canonical package identifiers.
/// CalVer uses each release's planned changeset level for its automatic entries.
pub fn detect_all_dependency_explanations(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    releases: &BTreeMap<&str, &ReleasedPackage>,
) -> Result<BTreeMap<String, Vec<(String, ChangelogCategory)>>> {
    let mut explanations = BTreeMap::new();
    if config.changelog_automatic_entries == AutomaticEntries::Collapse {
        return Ok(explanations);
    }

    let mut direct_targets = BTreeSet::new();
    for changeset in changesets {
        for (spec, _, _) in &changeset.entries {
            direct_targets.insert(resolve_package_spec(workspace, spec)?.canonical_identifier());
        }
    }
    let declared_groups =
        resolve_config_groups(workspace, &config.fixed_dependencies, "packages.fixed")?;
    let mut groups = PackageAdapter::implicit_fixed_groups(workspace);
    groups.extend(declared_groups.iter().cloned());
    let declared: BTreeSet<_> = declared_groups.iter().flatten().collect();
    let policy_members = fixed_policy_members(
        workspace,
        config,
        &merge_overlapping_groups(groups),
        &direct_targets,
        |id| releases.contains_key(id),
    );
    let include_kind = workspace.has_multiple_package_kinds();

    for info in &workspace.members {
        let identifier = info.canonical_identifier();
        let Some(release) = releases.get(identifier) else {
            continue;
        };
        let updates: Vec<_> = info
            .internal_deps
            .union(&info.internal_dev_deps)
            .filter_map(|id| releases.get(id.as_str()))
            .map(|dep| DependencyUpdate {
                name: if include_kind {
                    PackageSpecifier::parse(&dep.identifier)
                        .map(|spec| spec.display_name(true))
                        .unwrap_or_else(|_| dep.identifier.clone())
                } else {
                    dep.name.clone()
                },
                new_version: dep.new_version.clone(),
            })
            .collect();

        let mut entries = Vec::new();
        if policy_members.contains(identifier) {
            let bump = match config.versioning {
                Versioning::CalVer => release.bump,
                Versioning::SemVer => {
                    infer_bump_from_versions(&release.old_version, &release.new_version)
                }
            };
            let (message, bump) = if declared.contains(&release.identifier) {
                create_fixed_dependency_policy_entry(bump)
            } else {
                create_structural_version_coupling_entry(bump)
            };
            entries.push((message, ChangelogCategory::Bump(bump)));
        }
        if let Some((message, bump)) = create_dependency_update_entry(&updates) {
            let level = match config.versioning {
                Versioning::CalVer => release.bump,
                Versioning::SemVer => bump,
            };
            entries.push((message, ChangelogCategory::Bump(level)));
        }
        if !entries.is_empty() {
            explanations.insert(identifier.to_string(), entries);
        }
    }
    Ok(explanations)
}

fn fixed_policy_members(
    workspace: &Workspace,
    config: &Config,
    groups: &[Vec<String>],
    direct_targets: &BTreeSet<&str>,
    is_released: impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    let dependents = build_dependency_graph(workspace, &ignored_packages(workspace, config));
    let mut queue: Vec<_> = direct_targets.iter().copied().collect();
    let mut cascade = BTreeSet::new();
    while let Some(id) = queue.pop() {
        for dependent in dependents.get(id).into_iter().flatten() {
            if config.bump_dependents == BumpDependents::OutOfRange && !is_released(dependent) {
                continue;
            }
            if cascade.insert(dependent.as_str()) {
                queue.push(dependent);
            }
        }
    }

    let mut members = BTreeSet::new();
    for group in groups {
        let follows_changeset = group
            .iter()
            .any(|id| direct_targets.contains(id.as_str()) || cascade.contains(id.as_str()));
        for id in group {
            if !is_released(id)
                || direct_targets.contains(id.as_str())
                || cascade.contains(id.as_str())
            {
                continue;
            }
            // A previous fixed group can start a new dependency cascade beyond the direct targets.
            if follows_changeset
                || workspace.find_by_identifier(id).is_some_and(|pkg| {
                    !pkg.internal_deps
                        .union(&pkg.internal_dev_deps)
                        .any(|dep| is_released(dep))
                })
            {
                members.insert(id.clone());
            }
        }
    }
    members
}

/// Return releases explained by a fixed group, keyed by canonical identifier.
/// The value is the highest direct changeset level in the group, or patch if it has none.
/// The completed plan can assign a higher level through another dependency.
pub fn detect_fixed_dependency_policy_packages(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    bumped_packages: &BTreeSet<String>,
) -> Result<BTreeMap<String, Bump>> {
    let mut direct = BTreeMap::new();
    for changeset in changesets {
        for (spec, bump, _) in &changeset.entries {
            let id = resolve_package_spec(workspace, spec)?.canonical_identifier();
            let level = direct.entry(id).or_insert(*bump);
            *level = (*level).max(*bump);
        }
    }
    let groups = enforced_fixed_groups(workspace, config)?;
    let members = fixed_policy_members(
        workspace,
        config,
        &groups,
        &direct.keys().copied().collect(),
        |id| bumped_packages.contains(id),
    );
    let mut result = BTreeMap::new();
    for group in groups {
        let level = group
            .iter()
            .filter_map(|id| direct.get(id.as_str()))
            .max()
            .copied()
            .unwrap_or(Bump::Patch);
        result.extend(
            group
                .into_iter()
                .filter(|id| members.contains(id))
                .map(|id| (id, level)),
        );
    }
    Ok(result)
}

/// Type alias for initial bumps computation result
type InitialBumpsResult = (
    BTreeMap<String, Bump>,                             // bump_by_pkg
    BTreeMap<String, Vec<(String, ChangelogCategory)>>, // messages_by_pkg
    BTreeSet<std::path::PathBuf>,                       // used_paths
);

/// Aggregated data required to apply a planned release
struct PlanState {
    messages_by_pkg: BTreeMap<String, Vec<(String, ChangelogCategory)>>,
    used_paths: BTreeSet<PathBuf>,
    released_packages: Vec<ReleasedPackage>,
}

/// Possible outcomes when computing a release plan from a set of changesets
enum PlanOutcome {
    NoApplicablePackages,
    NoMatchingCrates,
    Plan(PlanState),
}

pub fn run_release(root: &Path, dry_run: bool) -> Result<ReleaseOutput> {
    run_release_with_now(root, dry_run, Utc::now())
}

pub(crate) fn run_release_with_now(
    root: &Path,
    dry_run: bool,
    release_started_at: DateTime<Utc>,
) -> Result<ReleaseOutput> {
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
    let preserved_targets =
        collect_preserved_targets(&preserved_changesets, &workspace, config.versioning)?;

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

        if all_preserved_targets_in_prerelease(&preserved_changesets, &workspace)? {
            println!(
                "No new changesets found. Preserved changesets exist but all referenced \
                 packages are in pre-release mode; skipping to avoid duplicate bump."
            );
            return Ok(ReleaseOutput {
                released_packages: vec![],
                dry_run,
            });
        }

        using_preserved = true;
    } else {
        match compute_plan_state(
            &current_changesets,
            &workspace,
            &config,
            &preserved_targets,
            false,
            true,
            release_started_at,
        )? {
            PlanOutcome::Plan(plan) => {
                let is_prerelease_preview = plan
                    .released_packages
                    .iter()
                    .any(ReleasedPackage::is_prerelease);
                // Exited packages lose their pre-release marker in this release, so their
                // preserved entries must ship with it.
                if !preserved_changesets.is_empty()
                    && (!is_prerelease_preview
                        || !all_preserved_targets_in_prerelease(&preserved_changesets, &workspace)?)
                {
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

    // Ahead of the preserved-changeset restore below, which is already a write.
    if !dry_run && needs_release_preflight(&workspace, using_preserved)? {
        let planned = match &cached_plan_state {
            Some(plan) => released_ecosystems(&workspace, &plan.released_packages),
            // No plan yet, and the real one needs the restore first.
            None => {
                let mut preview = current_changesets.clone();
                preview.extend(filter_prerelease_entries(
                    preserved_changesets.clone(),
                    &workspace,
                )?);
                preview_released_ecosystems(
                    &preview,
                    &workspace,
                    &config,
                    &preserved_targets,
                    false,
                    release_started_at,
                )?
            }
        };
        preflight_lockfile_tools(&workspace, &planned)?;
    }

    let mut final_changesets;
    let plan_state = if using_preserved {
        if dry_run {
            let filtered_preserved = filter_prerelease_entries(preserved_changesets, &workspace)?;
            final_changesets = current_changesets;
            final_changesets.extend(filtered_preserved);
        } else {
            restore_stable_preserved_changesets(
                &prerelease_dir,
                &changesets_dir,
                &workspace,
                &config.changesets_tags,
            )?;
            final_changesets = load_changesets(&changesets_dir, &config.changesets_tags)?;
        }

        match compute_plan_state(
            &final_changesets,
            &workspace,
            &config,
            &preserved_targets,
            false,
            true,
            release_started_at,
        )? {
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
            None => match compute_plan_state(
                &final_changesets,
                &workspace,
                &config,
                &preserved_targets,
                false,
                true,
                release_started_at,
            )? {
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
        released_packages,
    } = plan_state;

    validate_cargo_workspace_version(&workspace, &config, &released_packages)?;
    print_release_plan(&workspace, &released_packages);

    let is_prerelease_release = released_packages.iter().any(ReleasedPackage::is_prerelease);

    if dry_run {
        println!("Dry-run: no files modified, no tags created.");
        return Ok(ReleaseOutput {
            released_packages,
            dry_run: true,
        });
    }

    apply_releases(
        &released_packages,
        &workspace,
        &mut messages_by_pkg,
        &final_changesets,
        &config,
        release_started_at,
    )?;

    let prerelease_targets = collect_prerelease_targets(&released_packages);
    finalize_consumed_changesets(
        used_paths,
        &workspace.root,
        is_prerelease_release,
        &final_changesets,
        &workspace,
        &prerelease_targets,
    )?;

    regenerate_lockfile(
        &workspace,
        &released_ecosystems(&workspace, &released_packages),
    )?;

    Ok(ReleaseOutput {
        released_packages,
        dry_run: false,
    })
}

/// Stabilize prerelease packages in the workspace, producing stable version numbers
pub fn run_stabilize_release(root: &Path, dry_run: bool) -> Result<ReleaseOutput> {
    let release_started_at = Utc::now();
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

    validate_fixed_dependencies(&config, &workspace)?;

    let changesets_dir = workspace.root.join(".sampo").join("changesets");
    let prerelease_dir = workspace.root.join(".sampo").join("prerelease");

    let current_changesets = load_changesets(&changesets_dir, &config.changesets_tags)?;
    let preserved_changesets = load_changesets(&prerelease_dir, &config.changesets_tags)?;
    let preserved_targets =
        collect_preserved_targets(&preserved_changesets, &workspace, config.versioning)?;

    if current_changesets.is_empty() && preserved_changesets.is_empty() {
        println!(
            "No changesets found in {}",
            workspace.root.join(".sampo").join("changesets").display()
        );
        return Ok(ReleaseOutput {
            released_packages: vec![],
            dry_run,
        });
    }

    // The restore is a write, so the gate must precede it, planning from the union it restores.
    let restores_changesets = !preserved_changesets.is_empty();
    if !dry_run && restores_changesets && needs_release_preflight(&workspace, true)? {
        let mut preview = current_changesets.clone();
        preview.extend(preserved_changesets.clone());
        let planned = preview_released_ecosystems(
            &preview,
            &workspace,
            &config,
            &preserved_targets,
            true,
            release_started_at,
        )?;
        preflight_lockfile_tools(&workspace, &planned)?;
    }

    // Always restore all preserved changesets — unlike a normal release, which skips preserved
    // changesets when every target is still in prerelease, stabilization always needs them.
    let final_changesets = if current_changesets.is_empty() {
        if dry_run {
            preserved_changesets
        } else {
            restore_prerelease_changesets(&prerelease_dir, &changesets_dir)?;
            load_changesets(&changesets_dir, &config.changesets_tags)?
        }
    } else if dry_run {
        let mut all = current_changesets;
        all.extend(preserved_changesets);
        all
    } else {
        if !preserved_changesets.is_empty() {
            restore_prerelease_changesets(&prerelease_dir, &changesets_dir)?;
        }
        load_changesets(&changesets_dir, &config.changesets_tags)?
    };

    let plan_state = match compute_plan_state(
        &final_changesets,
        &workspace,
        &config,
        &preserved_targets,
        true,
        true,
        release_started_at,
    )? {
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
    };

    let PlanState {
        mut messages_by_pkg,
        used_paths,
        released_packages,
    } = plan_state;

    validate_cargo_workspace_version(&workspace, &config, &released_packages)?;
    print_release_plan(&workspace, &released_packages);

    if dry_run {
        println!("Dry-run: no files modified, no tags created.");
        return Ok(ReleaseOutput {
            released_packages,
            dry_run: true,
        });
    }

    let released_kinds = released_ecosystems(&workspace, &released_packages);
    if !restores_changesets {
        preflight_lockfile_tools(&workspace, &released_kinds)?;
    }

    apply_releases(
        &released_packages,
        &workspace,
        &mut messages_by_pkg,
        &final_changesets,
        &config,
        release_started_at,
    )?;

    let prerelease_targets = BTreeSet::new();
    finalize_consumed_changesets(
        used_paths,
        &workspace.root,
        false,
        &final_changesets,
        &workspace,
        &prerelease_targets,
    )?;

    regenerate_lockfile(&workspace, &released_kinds)?;

    Ok(ReleaseOutput {
        released_packages,
        dry_run: false,
    })
}

/// `report_warnings` is off for the preflight preview, whose plan the user never sees: its
/// constraint warnings would reach them twice.
fn compute_plan_state(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    preserved_targets: &BTreeSet<String>,
    stabilize: bool,
    report_warnings: bool,
    release_started_at: DateTime<Utc>,
) -> Result<PlanOutcome> {
    let (mut bump_by_pkg, messages_by_pkg, used_paths) =
        compute_initial_bumps(changesets, workspace, config)?;

    if bump_by_pkg.is_empty() {
        return Ok(PlanOutcome::NoApplicablePackages);
    }

    // Snapshot the changeset targets before the cascade widens the map, so a failure can
    // tell the user whether they named the package or the release pulled it in.
    let direct_targets: BTreeSet<String> = bump_by_pkg.keys().cloned().collect();

    let ignored = ignored_packages(workspace, config);
    let dependents = build_dependency_graph(workspace, &ignored);
    let cargo_metadata = if config.bump_dependents == BumpDependents::OutOfRange
        && workspace
            .members
            .iter()
            .any(|p| p.kind == PackageKind::Cargo)
    {
        Some(ManifestMetadata::load(workspace)?)
    } else {
        None
    };
    let fixed_groups = enforced_fixed_groups(workspace, config)?;
    let packages_by_id = workspace
        .members
        .iter()
        .map(|info| (info.canonical_identifier(), info))
        .collect();
    let mut planner = VersionPlanner {
        workspace,
        config,
        preserved_targets,
        stabilize,
        direct_targets,
        release_started_at,
        packages_by_id: &packages_by_id,
        ignored: &ignored,
        fixed_groups: &fixed_groups,
        fixed_versions: BTreeMap::new(),
    };
    // A linked raise can outrun the coupling the cascade just settled, so re-settle until
    // linked has nothing left to raise. Bumps only ever rise through three levels over a
    // finite set of packages, so this converges; with no `packages.linked` it runs once.
    loop {
        apply_dependency_cascade(
            &mut bump_by_pkg,
            &dependents,
            &mut planner,
            cargo_metadata.as_ref(),
        )?;
        if !apply_linked_dependencies(&mut bump_by_pkg, config, workspace)? {
            break;
        }
    }

    let releases = prepare_release_plan(&bump_by_pkg, &mut planner)?;
    if releases.is_empty() {
        return Ok(PlanOutcome::NoMatchingCrates);
    }

    let new_version_by_id: BTreeMap<String, String> = releases
        .iter()
        .map(|pkg| (pkg.identifier.clone(), pkg.new_version.clone()))
        .collect();
    let constraint_warnings = validate_dependency_constraints(
        &new_version_by_id,
        workspace,
        config,
        &packages_by_id,
        &fixed_groups,
    )?;
    if report_warnings {
        for warning in &constraint_warnings {
            eprintln!("{}", warning);
        }
    }

    // Fail before any manifest is written when structural version coupling cannot hold,
    // keeping releases atomic.
    PackageAdapter::validate_release_plan(workspace, &new_version_by_id)?;

    Ok(PlanOutcome::Plan(PlanState {
        messages_by_pkg,
        used_paths,
        released_packages: releases,
    }))
}

/// Validates dependency constraints before applying releases.
/// Returns error for fixed/linked packages with violations, warnings otherwise.
pub(crate) fn validate_dependency_constraints(
    new_version_by_id: &BTreeMap<String, String>,
    workspace: &Workspace,
    config: &Config,
    packages_by_id: &BTreeMap<&str, &PackageInfo>,
    fixed_groups: &[Vec<String>],
) -> Result<Vec<String>> {
    let linked_groups =
        resolve_config_groups(workspace, &config.linked_dependencies, "packages.linked")?;

    // Load cargo metadata for Cargo constraint extraction. Propagate failures:
    // swallowing them defaults every constraint to `"*"` and silently bypasses
    // fixed/linked range checks.
    let has_cargo = new_version_by_id.keys().any(|id| {
        packages_by_id
            .get(id.as_str())
            .is_some_and(|p| p.kind == PackageKind::Cargo)
    });
    let cargo_metadata = if has_cargo {
        Some(ManifestMetadata::load(workspace)?)
    } else {
        None
    };

    let mut violations: Vec<ConstraintViolation> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for pkg_id in new_version_by_id.keys() {
        let Some(pkg_info) = packages_by_id.get(pkg_id.as_str()) else {
            continue;
        };

        let adapter = PackageAdapter::from_kind(pkg_info.kind);
        let manifest_path = adapter.manifest_path(&pkg_info.path);

        for dep_id in &pkg_info.internal_deps {
            let Some(new_dep_version) = new_version_by_id.get(dep_id.as_str()) else {
                continue;
            };

            let Some(dep_info) = packages_by_id.get(dep_id.as_str()) else {
                continue;
            };

            let check_result = check_internal_dependency(
                pkg_info,
                dep_info,
                new_dep_version,
                cargo_metadata.as_ref(),
            )?;

            match check_result {
                ConstraintCheckResult::Satisfied => {}
                ConstraintCheckResult::Skipped { .. } => {}
                ConstraintCheckResult::Unverifiable { constraint } => {
                    warnings.push(unverifiable_pin_warning(
                        pkg_id,
                        &dep_info.name,
                        &constraint,
                        new_dep_version,
                    ));
                }
                ConstraintCheckResult::NotSatisfied {
                    constraint,
                    new_version,
                } => {
                    let violation = ConstraintViolation {
                        dependent_identifier: pkg_id.clone(),
                        dependency_identifier: dep_id.clone(),
                        dependency_name: dep_info.name.clone(),
                        constraint: constraint.clone(),
                        new_version: new_version.clone(),
                    };

                    let is_fixed_or_linked = group_index(pkg_id, fixed_groups).is_some()
                        || group_index(pkg_id, &linked_groups).is_some();

                    if is_fixed_or_linked {
                        violations.push(violation);
                    } else {
                        warnings.push(format!("Warning: {}", violation));
                    }
                }
            }
        }

        // Declarative edges (`<dependencyManagement>` pins, dev-only deps) never block
        // a release, but a property pin there goes stale all the same. Surface those,
        // ignore every other outcome.
        for dep_id in &pkg_info.internal_dev_deps {
            if pkg_info.internal_deps.contains(dep_id) {
                continue;
            }
            let Some(new_dep_version) = new_version_by_id.get(dep_id.as_str()) else {
                continue;
            };
            let Some(dep_info) = packages_by_id.get(dep_id.as_str()) else {
                continue;
            };
            let check_result = adapter.check_dependency_constraint(
                &manifest_path,
                &dep_info.name,
                "*",
                new_dep_version,
            )?;
            if let ConstraintCheckResult::Unverifiable { constraint } = check_result {
                warnings.push(unverifiable_pin_warning(
                    pkg_id,
                    &dep_info.name,
                    &constraint,
                    new_dep_version,
                ));
            }
        }
    }

    if !violations.is_empty() {
        let messages: Vec<String> = violations.iter().map(|v| v.to_string()).collect();
        return Err(SampoError::ConstraintViolation(format!(
            "Version constraint violations in fixed/linked packages:\n  - {}",
            messages.join("\n  - ")
        )));
    }

    Ok(warnings)
}

/// A Cargo pin (bare `1.2.3` or exact `=1.2.3`) is skipped, as the release rewrites it to
/// the new version. So is a requirement `cargo metadata` does not list.
fn check_internal_dependency(
    pkg_info: &PackageInfo,
    dep_info: &PackageInfo,
    new_dep_version: &str,
    cargo_metadata: Option<&ManifestMetadata>,
) -> Result<ConstraintCheckResult> {
    let adapter = PackageAdapter::from_kind(pkg_info.kind);
    let manifest_path = adapter.manifest_path(&pkg_info.path);

    let constraint = match (pkg_info.kind, cargo_metadata) {
        (PackageKind::Cargo, Some(meta)) => {
            if meta.is_dependency_pinned(&manifest_path, &dep_info.name) {
                return Ok(ConstraintCheckResult::Skipped {
                    reason: "pinned version".to_string(),
                });
            }
            let Some(constraint) = meta.get_dependency_constraint(&manifest_path, &dep_info.name)
            else {
                return Ok(ConstraintCheckResult::Skipped {
                    reason: "requirement not found".to_string(),
                });
            };
            constraint
        }
        _ => "*".to_string(),
    };

    adapter.check_dependency_constraint(
        &manifest_path,
        &dep_info.name,
        &constraint,
        new_dep_version,
    )
}

/// Whether `dependent` can stay out of the release when `dependency` moves to `new_version`:
/// its requirement must still cover it and the release must leave its manifest as written.
/// A pin in any section is rewritten, so a range declared beside it is not enough.
fn dependent_in_range(
    dependent: &PackageInfo,
    dependency: &PackageInfo,
    new_version: &str,
    ws: &Workspace,
    cfg: &Config,
    cargo_metadata: Option<&ManifestMetadata>,
) -> Result<bool> {
    // `pre exit` left it on a version no release has written yet.
    if exited_prerelease(ws, cfg, dependent, &dependent.version)?
        || !matches!(
            check_internal_dependency(dependent, dependency, new_version, cargo_metadata)?,
            ConstraintCheckResult::Satisfied
        )
    {
        return Ok(false);
    }
    // Hex rewrites every requirement it can parse, covered or not, so its rewrite says
    // nothing about a pin.
    if dependency.kind != dependent.kind || dependent.kind == PackageKind::Hex {
        return Ok(true);
    }

    let new_versions = BTreeMap::from([(dependency.name.clone(), new_version.to_string())]);
    let adapter = PackageAdapter::from_kind(dependent.kind);
    let manifest_path = adapter.manifest_path(&dependent.path);
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|err| SampoError::Io(io_error_with_path(err, &manifest_path)))?;
    let (updated, _) =
        adapter.update_manifest_versions(&manifest_path, &manifest, None, &new_versions)?;
    if updated != manifest {
        return Ok(false);
    }

    // A requirement inherited from `[workspace.dependencies]` is rewritten in the root.
    if dependent.kind == PackageKind::Cargo {
        let root_path = ws.root.join("Cargo.toml");
        let root = fs::read_to_string(&root_path)
            .map_err(|err| SampoError::Io(io_error_with_path(err, &root_path)))?;
        let (_, changed) = update_workspace_root_manifest(&root_path, &root, None, &new_versions)?;
        return Ok(!changed);
    }
    Ok(true)
}

/// A pin the release rewrite will not touch, pointing at a package that is moving.
fn unverifiable_pin_warning(
    pkg_id: &str,
    dep_name: &str,
    constraint: &str,
    new_version: &str,
) -> String {
    format!(
        "Warning: {pkg_id} pins {dep_name} through '{constraint}', which Sampo leaves \
         as written; {dep_name} releases {new_version} — update the pin yourself if it \
         should follow"
    )
}

fn group_index(pkg_id: &str, groups: &[Vec<String>]) -> Option<usize> {
    groups
        .iter()
        .position(|group| group.iter().any(|member| member == pkg_id))
}

fn collect_prerelease_targets(releases: &[ReleasedPackage]) -> BTreeSet<String> {
    releases
        .iter()
        .filter(|pkg| pkg.is_prerelease())
        .map(|pkg| pkg.identifier.clone())
        .collect()
}

fn is_spec_in_prerelease(workspace: &Workspace, spec: &PackageSpecifier) -> Result<bool> {
    let info = resolve_package_spec(workspace, spec)?;

    let version = Version::parse(&info.version).map_err(|e| {
        SampoError::Release(format!(
            "failed to parse version '{}' for package spec {}: {}",
            info.version, spec, e
        ))
    })?;

    Ok(!version.pre.is_empty())
}

fn all_preserved_targets_in_prerelease(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
) -> Result<bool> {
    let specs: Vec<&PackageSpecifier> = changesets
        .iter()
        .flat_map(|cs| cs.entries.iter().map(|(spec, _, _)| spec))
        .collect();

    if specs.is_empty() {
        return Ok(false);
    }

    for spec in &specs {
        if !is_spec_in_prerelease(workspace, spec)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The packages preserved changesets target, widened across structural coupling groups:
/// members that cannot hold different versions must advance their pre-release the same
/// way, or the plan is rejected for versions never out of step.
///
/// Members of declared fixed groups enter pre-release separately. Sharing counters could
/// put a member below the version it last published.
fn collect_preserved_targets(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    versioning: Versioning,
) -> Result<BTreeSet<String>> {
    if versioning == Versioning::CalVer && !changesets.is_empty() {
        return Err(SampoError::Release(
            "CalVer cannot release changesets in .sampo/prerelease. Remove packages.versioning and, if set, packages.calver_year_month. Finish the pre-release before enabling CalVer.".into(),
        ));
    }
    let mut targets = BTreeSet::new();
    for changeset in changesets {
        for (spec, _, _) in &changeset.entries {
            let info = resolve_package_spec(workspace, spec)?;
            targets.insert(info.canonical_identifier().to_string());
        }
    }

    if targets.is_empty() {
        return Ok(targets);
    }

    for group in merge_overlapping_groups(PackageAdapter::implicit_fixed_groups(workspace)) {
        if group.iter().any(|id| targets.contains(id)) {
            targets.extend(group);
        }
    }

    Ok(targets)
}

/// Move all preserved changeset files from the prerelease directory to the
/// changesets directory without filtering.
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

/// Restore preserved changesets to the changesets directory, filtering out
/// entries that target packages currently in prerelease.
///
/// For each preserved changeset file:
/// - If all entries target stable packages: move the entire file to changesets dir
/// - If all entries target prerelease packages: leave untouched in prerelease dir
/// - If mixed: write stable entries to a new file in changesets dir, rewrite
///   the prerelease dir file with only the prerelease entries
///
/// The mixed case writes the stable split first, then rewrites the prerelease
/// file in place to remove the stable entries.
fn restore_stable_preserved_changesets(
    prerelease_dir: &Path,
    changesets_dir: &Path,
    workspace: &Workspace,
    allowed_tags: &[String],
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

        let text =
            fs::read_to_string(&path).map_err(|e| SampoError::Io(io_error_with_path(e, &path)))?;
        let parsed = match parse_changeset(&text, &path, allowed_tags)? {
            Some(cs) => cs,
            None => continue,
        };

        let mut stable_entries = Vec::new();
        let mut prerelease_entries = Vec::new();
        for entry in parsed.entries.iter().cloned() {
            if is_spec_in_prerelease(workspace, &entry.0)? {
                prerelease_entries.push(entry);
            } else {
                stable_entries.push(entry);
            }
        }

        if prerelease_entries.is_empty() {
            // All entries target stable packages — move entire file
            let _ = move_changeset_file(&path, changesets_dir)?;
        } else if stable_entries.is_empty() {
            // All entries target prerelease packages — leave untouched
        } else {
            // Mixed: write stable entries to changesets dir, rewrite prerelease file
            fs::create_dir_all(changesets_dir)?;
            let file_name = path
                .file_name()
                .ok_or_else(|| SampoError::Changeset("Invalid changeset file name".to_string()))?;
            let mut stable_path = changesets_dir.join(file_name);
            if stable_path.exists() {
                stable_path = unique_destination_path(changesets_dir, file_name);
            }

            let stable_content =
                render_changeset_markdown_with_tags(&stable_entries, &parsed.message);
            fs::write(&stable_path, &stable_content)
                .map_err(|e| SampoError::Io(io_error_with_path(e, &stable_path)))?;

            let prerelease_content =
                render_changeset_markdown_with_tags(&prerelease_entries, &parsed.message);
            fs::write(&path, prerelease_content)
                .map_err(|e| SampoError::Io(io_error_with_path(e, &path)))?;
        }
    }

    Ok(())
}

/// Keep, in memory, the preserved changeset entries whose package has left pre-release.
/// Drops changesets that become empty.
pub fn filter_prerelease_entries(
    changesets: Vec<ChangesetInfo>,
    workspace: &Workspace,
) -> Result<Vec<ChangesetInfo>> {
    let mut result = Vec::new();
    for mut cs in changesets {
        let mut kept = Vec::new();
        for entry in cs.entries {
            if !is_spec_in_prerelease(workspace, &entry.0)? {
                kept.push(entry);
            }
        }
        cs.entries = kept;
        if !cs.entries.is_empty() {
            result.push(cs);
        }
    }
    Ok(result)
}

fn finalize_consumed_changesets(
    used_paths: BTreeSet<PathBuf>,
    workspace_root: &Path,
    preserve_for_prerelease: bool,
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    prerelease_targets: &BTreeSet<String>,
) -> Result<()> {
    if used_paths.is_empty() {
        return Ok(());
    }

    if preserve_for_prerelease {
        let prerelease_dir = workspace_root.join(".sampo").join("prerelease");
        let mut by_path: BTreeMap<&Path, &ChangesetInfo> = BTreeMap::new();
        for changeset in changesets {
            by_path.insert(changeset.path.as_path(), changeset);
        }
        for path in used_paths {
            if !path.exists() {
                continue;
            }
            if let Some(changeset) = by_path.get(path.as_path()) {
                let mut prerelease_entries = Vec::new();
                for entry in &changeset.entries {
                    let info = resolve_package_spec(workspace, &entry.0)?;
                    if prerelease_targets.contains(info.canonical_identifier()) {
                        prerelease_entries.push(entry.clone());
                    }
                }

                if prerelease_entries.is_empty() {
                    fs::remove_file(&path)
                        .map_err(|err| SampoError::Io(io_error_with_path(err, &path)))?;
                    continue;
                }

                if prerelease_entries.len() == changeset.entries.len() {
                    let _ = move_changeset_file(&path, &prerelease_dir)?;
                    continue;
                }

                fs::create_dir_all(&prerelease_dir)?;
                let file_name = path.file_name().ok_or_else(|| {
                    SampoError::Changeset("Invalid changeset file name".to_string())
                })?;
                let mut destination = prerelease_dir.join(file_name);
                if destination.exists() {
                    destination = unique_destination_path(&prerelease_dir, file_name);
                }
                let preserved_content =
                    render_changeset_markdown_with_tags(&prerelease_entries, &changeset.message);
                fs::write(&destination, preserved_content)
                    .map_err(|err| SampoError::Io(io_error_with_path(err, &destination)))?;
                fs::remove_file(&path)
                    .map_err(|err| SampoError::Io(io_error_with_path(err, &path)))?;
            } else {
                let _ = move_changeset_file(&path, &prerelease_dir)?;
            }
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

pub(crate) fn regenerate_lockfile(
    workspace: &Workspace,
    released_kinds: &BTreeSet<PackageKind>,
) -> Result<()> {
    use crate::types::PackageKind;

    let mut errors: Vec<(PackageKind, String)> = Vec::new();

    for kind in ecosystems_with_lockfiles(workspace, released_kinds) {
        if let Err(e) = PackageAdapter::from_kind(kind).regenerate_lockfile(&workspace.root) {
            errors.push((kind, adapter_failure_detail(e)));
        }
    }

    if !errors.is_empty() {
        let details = errors
            .iter()
            .map(|(kind, err)| format!("{} ({})", kind.display_name(), err))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(SampoError::Release(format!(
            "Cannot regenerate lockfiles for {details}; make the reported command succeed, \
             or delete the lockfile to opt out of regeneration. Version updates were already \
             written to the working tree: discard all of them before retrying, including the \
             files the run created — a partial restore can release the same changeset twice"
        )));
    }

    Ok(())
}

/// Ordered, so failure messages list ecosystems deterministically.
fn ecosystems_with_lockfiles(
    workspace: &Workspace,
    released_kinds: &BTreeSet<PackageKind>,
) -> BTreeSet<PackageKind> {
    let mut present: BTreeSet<PackageKind> = BTreeSet::new();
    for pkg in &workspace.members {
        if released_kinds.contains(&pkg.kind) {
            present.insert(pkg.kind);
        }
    }

    present
        .into_iter()
        .filter(|kind| match kind {
            PackageKind::Cargo => workspace.root.join("Cargo.lock").exists(),
            PackageKind::Npm => {
                workspace.root.join("package-lock.json").exists()
                    || workspace.root.join("pnpm-lock.yaml").exists()
                    || workspace.root.join("yarn.lock").exists()
                    || workspace.root.join("bun.lockb").exists()
                    || workspace.root.join("bun.lock").exists()
                    || workspace.root.join("npm-shrinkwrap.json").exists()
            }
            PackageKind::Hex => {
                // Elixir uses a root mix.lock; Gleam keeps a per-package manifest.toml,
                // so also look for one next to any discovered Hex member (a Gleam
                // monorepo has no root manifest).
                workspace.root.join("mix.lock").exists()
                    || workspace.root.join("manifest.toml").exists()
                    || workspace.members.iter().any(|member| {
                        member.kind == PackageKind::Hex
                            && member.path.join("manifest.toml").exists()
                    })
            }
            PackageKind::PyPI => {
                // A declared uv workspace shares a single root uv.lock; scanned
                // packages are independent uv projects, each owning one.
                workspace.root.join("uv.lock").exists()
                    || workspace.members.iter().any(|member| {
                        member.kind == PackageKind::PyPI && member.path.join("uv.lock").exists()
                    })
            }
            PackageKind::Packagist => workspace.root.join("composer.lock").exists(),
            // Maven has no lockfile; dependency versions live in the POMs themselves.
            PackageKind::Maven => false,
            // Like PyPI: a monorepo may share a root Gemfile.lock, or each gem keep its own.
            PackageKind::RubyGems => {
                workspace.root.join("Gemfile.lock").exists()
                    || workspace.members.iter().any(|member| {
                        member.kind == PackageKind::RubyGems
                            && member.path.join("Gemfile.lock").exists()
                    })
            }
        })
        .collect()
}

/// Check the tooling each lockfile regeneration will need, before the release writes
/// anything. See [`PackageAdapter::preflight_lockfile_regen`].
///
/// `released_kinds` must come from a resolved plan: `fixed`/`linked` groups can pull another
/// ecosystem in, which a scan of the raw changesets would miss.
pub(crate) fn preflight_lockfile_tools(
    workspace: &Workspace,
    released_kinds: &BTreeSet<PackageKind>,
) -> Result<()> {
    for kind in ecosystems_with_lockfiles(workspace, released_kinds) {
        PackageAdapter::from_kind(kind).preflight_lockfile_regen(&workspace.root)?;
    }
    Ok(())
}

fn validate_cargo_workspace_version(
    ws: &Workspace,
    cfg: &Config,
    releases: &[ReleasedPackage],
) -> Result<()> {
    let versions = releases
        .iter()
        .filter(|pkg| pkg.identifier.starts_with("cargo/"))
        .map(|pkg| (pkg.name.clone(), pkg.new_version.clone()))
        .collect();
    cargo::inherited_workspace_version(&ws.members, &versions, |pkg| {
        should_ignore_package(cfg, ws, pkg)
    })?;
    Ok(())
}

fn released_ecosystems(
    workspace: &Workspace,
    releases: &[ReleasedPackage],
) -> BTreeSet<PackageKind> {
    releases
        .iter()
        .filter_map(|pkg| workspace.find_by_identifier(&pkg.identifier))
        .map(|info| info.kind)
        .collect()
}

/// A restore must wait for checks on lockfiles or a shared Cargo version.
fn needs_release_preflight(workspace: &Workspace, restores_changesets: bool) -> Result<bool> {
    let every_kind: BTreeSet<PackageKind> = workspace.members.iter().map(|pkg| pkg.kind).collect();
    Ok(
        !ecosystems_with_lockfiles(workspace, &every_kind).is_empty()
            || (restores_changesets
                && cargo::workspace_version_inheritors(&workspace.members)?.len() > 1),
    )
}

/// `compute_plan_state` only reads, so this preview stays write-free — the preflight runs
/// before the writes that produce the real plan.
fn preview_released_ecosystems(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    preserved_targets: &BTreeSet<String>,
    stabilize: bool,
    release_started_at: DateTime<Utc>,
) -> Result<BTreeSet<PackageKind>> {
    match compute_plan_state(
        changesets,
        workspace,
        config,
        preserved_targets,
        stabilize,
        false,
        release_started_at,
    )? {
        PlanOutcome::Plan(plan) => {
            validate_cargo_workspace_version(workspace, config, &plan.released_packages)?;
            Ok(released_ecosystems(workspace, &plan.released_packages))
        }
        PlanOutcome::NoApplicablePackages | PlanOutcome::NoMatchingCrates => Ok(BTreeSet::new()),
    }
}

/// Unwrap `Release` instead of stringifying: re-wrapping would print the prefix twice.
fn adapter_failure_detail(err: SampoError) -> String {
    match err {
        SampoError::Release(msg) => msg,
        other => other.to_string(),
    }
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
                    cfg.changelog_show_acknowledgments
                        .then_some(Acknowledgments {
                            style: AcknowledgmentStyle::Mention,
                            excluded: &cfg.changelog_exclude_acknowledgments,
                        }),
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
fn ignored_packages<'a>(ws: &'a Workspace, cfg: &Config) -> BTreeSet<&'a str> {
    ws.members
        .iter()
        .filter(|info| should_ignore_package(cfg, ws, info).unwrap_or(false))
        .map(PackageInfo::canonical_identifier)
        .collect()
}

fn build_dependency_graph(
    ws: &Workspace,
    ignored_packages: &BTreeSet<&str>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for c in &ws.members {
        // Skip ignored packages when building the dependency graph
        let identifier = c.canonical_identifier();
        if ignored_packages.contains(identifier) {
            continue;
        }

        for dep in c.internal_deps.iter().chain(&c.internal_dev_deps) {
            // Also skip dependencies that point to ignored packages
            if ignored_packages.contains(dep.as_str()) {
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
    planner: &mut VersionPlanner<'_>,
    cargo_metadata: Option<&ManifestMetadata>,
) -> Result<()> {
    let ws = planner.workspace;
    let cfg = planner.config;
    let packages_by_id = planner.packages_by_id;
    let resolved_fixed_groups = planner.fixed_groups;
    let skip_dependents_in_range = cfg.bump_dependents == BumpDependents::OutOfRange;
    let linked_groups = if skip_dependents_in_range {
        resolve_config_groups(ws, &cfg.linked_dependencies, "packages.linked")?
    } else {
        Vec::new()
    };

    let mut queue: Vec<String> = bump_by_pkg.keys().cloned().collect();
    let mut seen: BTreeSet<String> = queue.iter().cloned().collect();

    while let Some(changed) = queue.pop() {
        let changed_bump = bump_by_pkg.get(&changed).copied().unwrap_or(Bump::Patch);
        let changed_release = match packages_by_id.get(changed.as_str()) {
            Some(info) if skip_dependents_in_range => {
                Some((*info, planner.planned_version(info, changed_bump)?))
            }
            _ => None,
        };

        // 1. Handle normal dependency relationships (unchanged → dependent)
        if let Some(deps) = dependents.get(&changed) {
            for dep_name in deps {
                // Members of a `linked` group follow each other whatever their ranges say.
                if let Some((changed_info, new_version)) = &changed_release
                    && let Some(dependent_info) = packages_by_id.get(dep_name.as_str())
                    && !linked_groups
                        .iter()
                        .any(|group| group.contains(&changed) && group.contains(dep_name))
                    && dependent_in_range(
                        dependent_info,
                        changed_info,
                        new_version,
                        ws,
                        cfg,
                        cargo_metadata,
                    )?
                {
                    continue;
                }

                let dependent_bump = if group_index(dep_name, resolved_fixed_groups).is_some() {
                    changed_bump
                } else {
                    Bump::Patch
                };

                let entry = bump_by_pkg
                    .entry(dep_name.clone())
                    .or_insert(dependent_bump);
                // If already present, keep the higher bump
                let raised = *entry < dependent_bump;
                if raised {
                    *entry = dependent_bump;
                }
                // Re-queue on a raise, not only on first sight: a package popped before
                // its bump grew would otherwise never carry the higher level onward.
                let first_sight = seen.insert(dep_name.clone());
                if raised || first_sight {
                    queue.push(dep_name.clone());
                }
            }
        }

        if let Some(group_idx) = group_index(&changed, resolved_fixed_groups) {
            for group_member in &resolved_fixed_groups[group_idx] {
                if group_member == &changed || planner.ignored.contains(group_member.as_str()) {
                    continue;
                }
                let entry = bump_by_pkg
                    .entry(group_member.clone())
                    .or_insert(changed_bump);
                // If already present, keep the higher bump
                let raised = *entry < changed_bump;
                if raised {
                    *entry = changed_bump;
                }
                let first_sight = seen.insert(group_member.clone());
                if raised || first_sight {
                    queue.push(group_member.clone());
                }
            }
        }
    }

    Ok(())
}

/// Apply linked dependencies logic: highest bump level to affected packages only.
///
/// Returns whether any bump was raised, so the caller can re-settle the dependency
/// cascade and `packages.fixed` groups a raise may have outrun.
fn apply_linked_dependencies(
    bump_by_pkg: &mut BTreeMap<String, Bump>,
    cfg: &Config,
    ws: &Workspace,
) -> Result<bool> {
    let resolved_groups = resolve_config_groups(ws, &cfg.linked_dependencies, "packages.linked")?;
    let mut raised = false;

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
                        raised = true;
                    }
                }
            }
        }
    }

    Ok(raised)
}

fn prepare_release_plan(
    bump_by_pkg: &BTreeMap<String, Bump>,
    planner: &mut VersionPlanner<'_>,
) -> Result<Vec<ReleasedPackage>> {
    let mut releases = Vec::new();
    for (identifier, bump) in bump_by_pkg {
        if let Some(&info) = planner.packages_by_id.get(identifier.as_str()) {
            releases.push(ReleasedPackage {
                identifier: identifier.clone(),
                name: info.name.clone(),
                old_version: current_version(info).to_owned(),
                new_version: planner.planned_version(info, *bump)?,
                bump: *bump,
            });
        }
    }
    Ok(releases)
}

struct VersionPlanner<'a> {
    workspace: &'a Workspace,
    config: &'a Config,
    preserved_targets: &'a BTreeSet<String>,
    stabilize: bool,
    direct_targets: BTreeSet<String>,
    release_started_at: DateTime<Utc>,
    packages_by_id: &'a BTreeMap<&'a str, &'a PackageInfo>,
    ignored: &'a BTreeSet<&'a str>,
    fixed_groups: &'a [Vec<String>],
    fixed_versions: BTreeMap<usize, String>,
}

fn current_version(info: &PackageInfo) -> &str {
    if info.version.is_empty() {
        "0.0.0"
    } else {
        &info.version
    }
}

impl VersionPlanner<'_> {
    // Skipping an invalid version would consume its changeset without releasing it.
    fn version_plan_error(&self, info: &PackageInfo, reason: &str) -> SampoError {
        let identifier = info.canonical_identifier();
        let old_version = current_version(info);
        let mut message = format!("Cannot release '{identifier}' ({old_version}): ");
        if !self.direct_targets.contains(identifier) {
            message.push_str(
                "It joined this release because it depends on a package being released or \
                 shares a version group with it. ",
            );
        }
        message.push_str(reason);
        if self.config.versioning == Versioning::SemVer {
            message.push_str("; fix its version, or exclude it with packages.ignore");
        }
        SampoError::Release(message)
    }

    fn planned_version(&mut self, info: &PackageInfo, bump: Bump) -> Result<String> {
        let old_version = current_version(info);
        let version =
            parse_version_string(old_version).map_err(|err| self.version_plan_error(info, &err))?;
        if self.config.versioning == Versioning::CalVer {
            return self.planned_calver(info, version);
        }
        if self.stabilize && !version.pre.is_empty() {
            stabilize_version_from_parsed(&version, bump)
        } else if version.pre.is_empty()
            && exited_prerelease(self.workspace, self.config, info, old_version)?
        {
            stabilize_base(&version, bump)
        } else if should_bump_prerelease_base(
            &version,
            info.canonical_identifier(),
            self.preserved_targets,
        ) {
            bump_prerelease_entry(&version, bump)
        } else {
            bump_version_from_parsed(&version, bump)
        }
        .map_err(|err| self.version_plan_error(info, &err))
    }

    fn planned_calver(&mut self, info: &PackageInfo, version: Version) -> Result<String> {
        let Some(index) = group_index(info.canonical_identifier(), self.fixed_groups) else {
            return calver_version(&version, self.config, self.release_started_at)
                .map_err(|err| self.version_plan_error(info, &err));
        };
        if let Some(version) = self.fixed_versions.get(&index) {
            return Ok(version.clone());
        }

        // The cascade releases every non-ignored member of this group.
        let mut baseline_member = info;
        let mut baseline = version;
        require_stable_calver(&baseline).map_err(|err| self.version_plan_error(info, &err))?;
        for member in self.fixed_groups[index]
            .iter()
            .filter(|id| *id != info.canonical_identifier())
            .filter(|id| !self.ignored.contains(id.as_str()))
            .filter_map(|id| self.packages_by_id.get(id.as_str()).copied())
        {
            let version = parse_version_string(current_version(member))
                .map_err(|err| self.version_plan_error(member, &err))?;
            require_stable_calver(&version).map_err(|err| self.version_plan_error(member, &err))?;
            if version.cmp_precedence(&baseline).is_gt() {
                baseline_member = member;
                baseline = version;
            }
        }
        let new_version = calver_version(&baseline, self.config, self.release_started_at)
            .map_err(|err| self.version_plan_error(baseline_member, &err))?;
        self.fixed_versions.insert(index, new_version.clone());
        Ok(new_version)
    }
}

/// Print the planned releases
fn print_release_plan(workspace: &Workspace, releases: &[ReleasedPackage]) {
    let include_kind = workspace.has_multiple_package_kinds();
    println!("Planned releases:");
    for release in releases {
        let identifier = &release.identifier;
        let display = workspace
            .find_by_identifier(identifier)
            .map(|info| info.display_name(include_kind))
            .or_else(|| {
                PackageSpecifier::parse(identifier)
                    .ok()
                    .map(|spec| spec.display_name(include_kind))
            })
            .unwrap_or_else(|| identifier.clone());
        println!(
            "  {display}: {} -> {}",
            release.old_version, release.new_version
        );
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

fn compute_release_date_display(cfg: &Config, now: DateTime<Utc>) -> Result<Option<String>> {
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
    releases: &[ReleasedPackage],
    ws: &Workspace,
    messages_by_pkg: &mut BTreeMap<String, Vec<(String, ChangelogCategory)>>,
    changesets: &[ChangesetInfo],
    cfg: &Config,
    release_started_at: DateTime<Utc>,
) -> Result<()> {
    let packages_by_id: BTreeMap<_, _> = ws
        .members
        .iter()
        .map(|pkg| (pkg.canonical_identifier(), pkg))
        .collect();
    let releases_by_id = releases
        .iter()
        .map(|pkg| (pkg.identifier.as_str(), pkg))
        .collect();
    let dependency_explanations =
        detect_all_dependency_explanations(changesets, ws, cfg, &releases_by_id)?;
    let versions_by_id = releases
        .iter()
        .map(|pkg| (pkg.identifier.clone(), pkg.new_version.clone()))
        .collect();
    let versions_by_kind = PackageAdapter::versions_by_kind(ws, &versions_by_id);

    for (pkg_name, explanations) in dependency_explanations {
        messages_by_pkg
            .entry(pkg_name)
            .or_default()
            .extend(explanations);
    }

    let release_date_display = compute_release_date_display(
        cfg,
        match cfg.versioning {
            Versioning::CalVer => release_started_at,
            Versioning::SemVer => Utc::now(),
        },
    )?;

    for pkg in releases {
        let identifier = &pkg.identifier;
        let info = packages_by_id
            .get(identifier.as_str())
            .ok_or_else(|| SampoError::Release(format!("package '{identifier}' not found")))?;
        let adapter = PackageAdapter::from_kind(info.kind);
        let manifest_path = adapter.manifest_path(&info.path);
        let text = fs::read_to_string(&manifest_path)?;

        let (updated, _dep_updates) = adapter.update_manifest_versions(
            &manifest_path,
            &text,
            Some(&pkg.new_version),
            &versions_by_kind[&info.kind],
        )?;
        fs::write(&manifest_path, updated)?;

        let messages = messages_by_pkg.get(identifier).cloned().unwrap_or_default();
        update_changelog(
            &info.path,
            &info.name,
            &pkg.old_version,
            &pkg.new_version,
            &messages,
            release_date_display.as_deref(),
            cfg.changelog_automatic_entries == AutomaticEntries::Collapse,
        )?;
    }

    PackageAdapter::finalize_workspace_roots(ws, &versions_by_id)?;

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
    Ok(implied_base_bump(version))
}

fn implied_base_bump(version: &Version) -> Bump {
    if version.minor == 0 && version.patch == 0 {
        Bump::Major
    } else if version.patch == 0 {
        Bump::Minor
    } else {
        Bump::Patch
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

fn prerelease_has_trailing_numeric(pre: &Prerelease) -> bool {
    pre.as_str()
        .split('.')
        .next_back()
        .map(|part| part.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false)
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

fn should_bump_prerelease_base(
    version: &Version,
    identifier: &str,
    preserved_targets: &BTreeSet<String>,
) -> bool {
    if version.pre.is_empty() {
        return false;
    }
    if preserved_targets.contains(identifier) {
        return false;
    }
    !prerelease_has_trailing_numeric(&version.pre)
}

fn bump_prerelease_entry(version: &Version, bump: Bump) -> std::result::Result<String, String> {
    if version.pre.is_empty() {
        return Err("Version does not contain a pre-release identifier".to_string());
    }

    let base_pre = strip_trailing_numeric_identifiers(&version.pre).ok_or_else(|| {
        "Pre-release version must include a non-numeric identifier before the counter".to_string()
    })?;

    let mut updated = version.clone();
    apply_base_bump(&mut updated, bump)?;
    updated.pre = base_pre;
    Ok(updated.to_string())
}

fn calver_year_month(cfg: &Config, now: DateTime<Utc>) -> (u64, u64) {
    cfg.calver_year_month
        .unwrap_or_else(|| (now.year() as u64, u64::from(now.month())))
}

fn require_stable_calver(version: &Version) -> std::result::Result<(), String> {
    if !version.pre.is_empty() {
        return Err(format!(
            "CalVer cannot release pre-release version {version}. Remove packages.versioning and, if set, packages.calver_year_month. Finish the pre-release before enabling CalVer."
        ));
    }
    Ok(())
}

fn calver_version(
    baseline: &Version,
    config: &Config,
    release_started_at: DateTime<Utc>,
) -> std::result::Result<String, String> {
    require_stable_calver(baseline)?;
    let (year, month) = calver_year_month(config, release_started_at);
    let micro = if (baseline.major, baseline.minor) == (year, month) {
        baseline.patch.checked_add(1).ok_or_else(|| {
            format!("CalVer micro component overflow for {baseline}. Select a later release month.")
        })?
    } else {
        0
    };
    let next_version = Version::new(year, month, micro);
    if next_version.cmp_precedence(baseline).is_le() {
        let advice = if config.calver_year_month.is_some() {
            "Check packages.calver_year_month."
        } else {
            "Check the UTC date."
        };
        return Err(format!(
            "CalVer version {next_version} must be higher than {baseline}. {advice}"
        ));
    }
    Ok(next_version.to_string())
}

fn bump_version_from_parsed(parsed: &Version, bump: Bump) -> std::result::Result<String, String> {
    let mut version = parsed.clone();
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
            "Pre-release version must include a non-numeric identifier before the counter"
                .to_string()
        })?;

        apply_base_bump(&mut version, bump)?;
        version.pre = base_pre;
        Ok(version.to_string())
    }
}

fn stabilize_version_from_parsed(
    parsed: &Version,
    bump: Bump,
) -> std::result::Result<String, String> {
    let mut version = parsed.clone();
    if version.pre.is_empty() {
        apply_base_bump(&mut version, bump)?;
        return Ok(version.to_string());
    }
    stabilize_base(&version, bump)
}

/// The base already encodes the cycle's bump; `bump` applies only beyond it.
fn stabilize_base(parsed: &Version, bump: Bump) -> std::result::Result<String, String> {
    let mut version = parsed.clone();
    if bump > implied_base_bump(&version) {
        apply_base_bump(&mut version, bump)?;
    }
    version.pre = Prerelease::EMPTY;
    version.build = BuildMetadata::EMPTY;
    Ok(version.to_string())
}

/// `sampo pre exit` leaves the changelog on the pre-release cycle: a stable version whose top
/// section is a pre-release of it is unreleased, unless its tag exists. A package that
/// `git.tag_unpublished` leaves untagged never has one, so it stays unreleased until
/// `sampo release` writes its section, as nothing else marks it as shipped.
fn exited_prerelease(
    ws: &Workspace,
    config: &Config,
    info: &PackageInfo,
    version: &str,
) -> Result<bool> {
    let path = info.path.join("CHANGELOG.md");
    let changelog = match fs::read_to_string(&path) {
        Ok(changelog) => changelog,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(SampoError::Io(io_error_with_path(err, &path))),
    };
    let (_, versions) = split_intro_and_versions(changelog.trim_start_matches('\u{feff}'));
    if !split_top_section(versions)
        .is_some_and(|(header_text, _, _)| header_is_prerelease_of(header_text, version))
    {
        return Ok(false);
    }
    // Shipped without `sampo release` writing its section.
    Ok(!package_tag_exists(
        &ws.root, config, info.kind, &info.name, version,
    )?)
}

/// Bump a semver version string, including pre-release handling
pub fn bump_version(old: &str, bump: Bump) -> std::result::Result<String, String> {
    let version = parse_version_string(old)?;
    bump_version_from_parsed(&version, bump).map_err(|err| {
        if err.contains("Pre-release version must include a non-numeric identifier") {
            format!(
                "Pre-release version '{old}' must include a non-numeric identifier before the counter"
            )
        } else {
            err
        }
    })
}

/// A digit marks a release heading in any format, so `## About` or `## [Unreleased]` stay on top.
fn split_intro_and_versions(body: &str) -> (&str, &str) {
    let mut offset = 0;
    let len = body.len();
    while offset < len {
        if body[offset..]
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("## "))
            .is_some_and(|heading| heading.contains(|c: char| c.is_ascii_digit()))
        {
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

/// Splits a changelog heading text (without the `## ` prefix) into the first and last
/// version it covers: `1.2.0`, `1.2.0 — date`, or a collapsed `1.0.0 ... 1.2.0 — date`.
fn heading_versions(header_text: &str) -> Option<(&str, &str)> {
    let mut tokens = header_text.split_whitespace();
    let first = tokens.next()?;
    let mut next = tokens.next();
    let mut last = first;
    if next == Some(VERSION_RANGE_SEPARATOR) {
        last = tokens.next()?;
        next = tokens.next();
    }
    // Whatever follows the versions must be the release date.
    match next {
        Some(token) if !token.starts_with(['—', '-']) => None,
        _ => Some((first, last)),
    }
}

/// Whether a heading token names `version`, allowing a date glued to it. A glued `-` only
/// counts as a date when a digit follows, so `1.0.0-rc.1` is not taken for `1.0.0`.
fn heading_token_matches(token: &str, version: &str) -> bool {
    match token.strip_prefix(version) {
        Some("") => true,
        Some(rest) => {
            rest.starts_with('—')
                || rest
                    .strip_prefix('-')
                    .is_some_and(|date| date.starts_with(|c: char| c.is_ascii_digit()))
        }
        None => false,
    }
}

/// Whether a changelog `## ` heading text (without the `## ` prefix) is the section for
/// `version`, with or without a release date suffix. A collapsed range heading
/// (`1.0.0 ... 1.2.0`) is the section for every version it spans.
pub fn header_matches_release_version(header_text: &str, version: &str) -> bool {
    let Some((first, last)) = heading_versions(header_text) else {
        return false;
    };
    if heading_token_matches(first, version) || heading_token_matches(last, version) {
        return true;
    }
    first != last
        && matches!(
            (Version::parse(first), Version::parse(last), Version::parse(version)),
            (Ok(first), Ok(last), Ok(version)) if first < version && version < last
        )
}

fn header_is_prerelease_of(header_text: &str, version: &str) -> bool {
    let (Some((_, last)), Ok(version)) = (heading_versions(header_text), Version::parse(version))
    else {
        return false;
    };
    let token = last.split('—').next().unwrap_or(last);
    version.pre.is_empty()
        && Version::parse(token).is_ok_and(|heading| {
            !heading.pre.is_empty()
                && (heading.major, heading.minor, heading.patch)
                    == (version.major, version.minor, version.patch)
        })
}

/// Only a strict `x.y.z` heading, as Sampo writes it, can be unreleased and merged.
fn header_is_ahead_of(header_text: &str, version: &str) -> bool {
    let (Some((first, _)), Ok(version)) =
        (heading_versions(header_text), parse_version_string(version))
    else {
        return false;
    };
    let token = first.split('—').next().unwrap_or(first);
    Version::parse(token).is_ok_and(|heading| {
        heading.cmp_precedence(&version).is_gt() && !is_before_prerelease_cycle(&heading, &version)
    })
}

/// The cycle's stable base and other labels shipped before it, though SemVer ranks them higher.
fn is_before_prerelease_cycle(heading: &Version, version: &Version) -> bool {
    if version.pre.is_empty() {
        return false;
    }
    if heading.pre.is_empty() {
        return (heading.major, heading.minor, heading.patch)
            == (version.major, version.minor, version.patch);
    }
    strip_trailing_numeric_identifiers(&heading.pre)
        != strip_trailing_numeric_identifiers(&version.pre)
}

/// Splits the first `## ` section off a changelog's versions part, returning its heading
/// text, its body, and everything after it.
fn split_top_section(versions: &str) -> Option<(&str, &str, &str)> {
    let after_marker = versions.trim_start().strip_prefix("## ")?;
    let header_end = after_marker.find('\n').unwrap_or(after_marker.len());
    let after_header = &after_marker[header_end..];
    let (body, remaining) = match after_header.find("\n## ") {
        Some(pos) => after_header.split_at(pos + 1),
        None => (after_header, ""),
    };
    Some((after_marker[..header_end].trim(), body, remaining))
}

/// With `collapse_version_bump_only`, a release without entries gets the version-bump-only
/// line instead of an empty section, extending the latest section if it is one already.
fn update_changelog(
    crate_dir: &Path,
    package: &str,
    old_version: &str,
    new_version: &str,
    entries: &[(String, ChangelogCategory)],
    release_date_display: Option<&str>,
    collapse_version_bump_only: bool,
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
    let mut versions_body = versions_part;

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

    if let Some((header_text, section_body, remaining)) = split_top_section(versions_body)
        && header_is_ahead_of(header_text, old_version)
    {
        // Parse existing section headings - support both bump-based and custom tags
        let mut current_heading: Option<String> = None;
        for line in section_body.lines() {
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

        versions_body = remaining;
    }

    let collapse = collapse_version_bump_only && sections.is_empty();
    let mut heading_versions_label = new_version.to_string();
    // Consecutive version-bump-only releases share one section, but only when it is the
    // latest one, so a range never spans a release with real entries.
    if collapse
        && let Some((header_text, section_body, remaining)) = split_top_section(versions_body)
        && section_body.trim() == VERSION_BUMP_ONLY_ENTRY
        && let Some((first, last)) = heading_versions(header_text)
        && last == old_version
    {
        heading_versions_label = format!("{first} {VERSION_RANGE_SEPARATOR} {new_version}");
        versions_body = remaining;
    }

    // Build new aggregated top section
    let mut section = String::new();
    match release_date_display.and_then(|d| (!d.trim().is_empty()).then_some(d)) {
        Some(date) => section.push_str(&format!("## {heading_versions_label} — {date}\n\n")),
        None => section.push_str(&format!("## {heading_versions_label}\n\n")),
    }

    if collapse {
        section.push_str(VERSION_BUMP_ONLY_ENTRY);
        section.push_str("\n\n");
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
        sections.sort_by_key(|(a, _)| bump_order(a));
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
        combined.push_str(versions_body);
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
    fn calver_versions_from_utc_or_a_fixed_month() {
        for (old_version, instant, year_month, expected_version) in [
            ("0.0.0", "2026-03-03T00:00:00Z", None, "2026.3.0"),
            ("1.4.2", "2026-03-03T00:00:00Z", None, "2026.3.0"),
            ("2026.3.7", "2026-03-31T23:59:59Z", None, "2026.3.8"),
            ("2026.3.8", "2026-04-01T00:00:00Z", None, "2026.4.0"),
            ("2026.12.3", "2027-01-01T00:30:00Z", None, "2027.1.0"),
            ("2026.1.7", "2027-01-01T00:30:00Z", None, "2027.1.0"),
            (
                "0.0.0",
                "2026-11-18T00:00:00Z",
                Some((2026, 12)),
                "2026.12.0",
            ),
            (
                "2026.12.0",
                "2027-01-05T00:00:00Z",
                Some((2026, 12)),
                "2026.12.1",
            ),
        ] {
            let cfg = Config {
                calver_year_month: year_month,
                changelog_release_date_timezone: Some("Pacific/Honolulu".into()),
                ..Config::default()
            };
            let next_version = calver_version(
                &Version::parse(old_version).unwrap(),
                &cfg,
                instant.parse().unwrap(),
            )
            .unwrap();
            assert_eq!(
                next_version, expected_version,
                "{old_version} at {instant} with {year_month:?}"
            );
        }
    }

    #[test]
    fn calver_refuses_future_and_prerelease_versions() {
        for (old_version, instant, year_month, reason) in [
            ("2026.4.0", "2026-03-31T00:00:00Z", None, "2026.3.0"),
            ("2027.1.0", "2026-12-31T00:00:00Z", None, "2026.12.0"),
            (
                "2027.1.0",
                "2027-01-05T00:00:00Z",
                Some((2026, 12)),
                "2026.12.0",
            ),
            (
                "2026.3.0-alpha.1",
                "2026-03-31T00:00:00Z",
                None,
                "pre-release",
            ),
        ] {
            let cfg = Config {
                calver_year_month: year_month,
                ..Config::default()
            };
            let err = calver_version(
                &Version::parse(old_version).unwrap(),
                &cfg,
                instant.parse().unwrap(),
            )
            .unwrap_err();
            assert!(err.contains(old_version) && err.contains(reason), "{err}");
        }
    }

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
            false,
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
        update_changelog(
            crate_dir,
            "new-package",
            "0.1.0",
            "1.0.0",
            &entries,
            None,
            false,
        )
        .unwrap();

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
        assert!(!header_matches_release_version("1.0.0-rc.1", "1.0.0"));
        assert!(!header_matches_release_version(
            "1.0.0-rc.1 — 2024-06-20",
            "1.0.0"
        ));
    }

    #[test]
    fn header_matches_release_version_covers_collapsed_ranges() {
        let heading = "0.2.0 ... 0.4.0 — 2024-06-20";
        for version in ["0.2.0", "0.3.1", "0.4.0-rc.1", "0.4.0"] {
            assert!(
                header_matches_release_version(heading, version),
                "{version} should be in {heading}"
            );
        }
        for version in ["0.1.9", "0.4.1", "0.2.0-rc.1"] {
            assert!(
                !header_matches_release_version(heading, version),
                "{version} should not be in {heading}"
            );
        }
        assert!(header_matches_release_version("0.2.0 ... 0.4.0", "0.3.0"));
        assert!(!header_matches_release_version("0.2.0 ...", "0.2.0"));
        assert!(!header_matches_release_version("0.2.0 to 0.4.0", "0.2.0"));
    }

    #[test]
    fn header_is_prerelease_of_needs_the_same_stable_base() {
        for heading in [
            "1.1.0-alpha.1",
            "1.1.0-alpha ... 1.1.0-alpha.2 — 2024-06-20",
        ] {
            assert!(header_is_prerelease_of(heading, "1.1.0"), "{heading}");
        }
        assert!(!header_is_prerelease_of("1.1.0 — 2024-06-20", "1.1.0"));
        assert!(!header_is_prerelease_of("1.1.1-rc.1", "1.1.0"));
        assert!(!header_is_prerelease_of("1.1.0-alpha.1", "1.1.0-alpha.2"));
        assert!(!header_is_prerelease_of("Unreleased", "1.1.0"));
    }

    #[test]
    fn header_is_ahead_of_needs_a_newer_version_in_sampo_format() {
        for (heading, version) in [
            ("0.1.1", "0.1.0"),
            ("0.2.0 — 2024-01-01", "0.1.0"),
            ("0.2.0 ... 0.3.0 — 2024-01-01", "0.1.0"),
            ("1.1.0-alpha", "1.0.0"),
            ("1.2.4-alpha", "1.2.3-alpha"),
            ("1.1.0-alpha.2", "1.1.0-alpha.1"),
            ("1.3.0", "1.2.3-alpha"),
            ("0.1.1", "0.1"),
            ("2.2.0", "2.1"),
        ] {
            assert!(header_is_ahead_of(heading, version), "{heading}");
        }
        for (heading, version) in [
            ("1.0.1", "1.0.1"),
            ("1.0.1 — 2024-01-01", "1.0.1"),
            ("1.0.1+20240101", "1.0.1"),
            ("2.1.0", "2.1"),
            ("2.1", "2.1.1"),
            ("1.0.0", "1.0.1"),
            ("0.2.0 ... 0.4.0", "0.3.0"),
            ("1.1.0-alpha.1", "1.1.0"),
            ("1.1.0-alpha", "1.1.0-beta"),
            ("[1.1.0] - 2024-01-01", "1.0.1"),
            ("v1.1.0", "1.0.1"),
            ("2.0 migration guide", "1.0.1"),
            ("2.2", "2.1"),
            ("2024-01-01", "1.0.1"),
            ("Unreleased", "1.0.0"),
            ("1.2.3", "1.2.3-alpha"),
            ("1.2.3 — 2024-01-01", "1.2.3-alpha.2"),
            ("1.0.1-beta", "1.0.1-alpha"),
            ("1.0.1-rc.1", "1.0.1-beta.2"),
        ] {
            assert!(!header_is_ahead_of(heading, version), "{heading}");
        }
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
            false,
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
        let display = compute_release_date_display(&cfg, now).unwrap().unwrap();
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
        let display = compute_release_date_display(&cfg, now).unwrap().unwrap();
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
        let display = compute_release_date_display(&cfg, now).unwrap().unwrap();
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
                    internal_dev_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "examples-package".to_string(),
                    identifier: "cargo/examples-package".to_string(),
                    version: "1.0.0".to_string(),
                    path: root.join("examples/package"),
                    internal_deps: BTreeSet::new(),
                    internal_dev_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "benchmarks-utils".to_string(),
                    identifier: "cargo/benchmarks-utils".to_string(),
                    version: "1.0.0".to_string(),
                    path: root.join("benchmarks/utils"),
                    internal_deps: BTreeSet::new(),
                    internal_dev_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
            ],
        };

        // Create a config that ignores examples/* and benchmarks/*
        let config = Config {
            ignore: vec!["examples/*".to_string(), "benchmarks/*".to_string()],
            ..Default::default()
        };

        let ignored = ignored_packages(&workspace, &config);
        let dependents = build_dependency_graph(&workspace, &ignored);
        let packages_by_id = workspace
            .members
            .iter()
            .map(|pkg| (pkg.canonical_identifier(), pkg))
            .collect();
        let fixed_groups = vec![
            workspace
                .members
                .iter()
                .map(|pkg| pkg.identifier.clone())
                .collect(),
        ];
        let mut planner = VersionPlanner {
            workspace: &workspace,
            config: &config,
            preserved_targets: &BTreeSet::new(),
            stabilize: false,
            direct_targets: BTreeSet::new(),
            release_started_at: Utc::now(),
            packages_by_id: &packages_by_id,
            ignored: &ignored,
            fixed_groups: &fixed_groups,
            fixed_versions: BTreeMap::new(),
        };
        let mut bump_by_pkg = BTreeMap::from([("cargo/main-package".to_string(), Bump::Minor)]);
        apply_dependency_cascade(&mut bump_by_pkg, &dependents, &mut planner, None).unwrap();

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
                    internal_dev_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "examples-package".to_string(),
                    identifier: "cargo/examples-package".to_string(),
                    version: "1.0.0".to_string(),
                    path: root.join("examples/package"),
                    internal_deps: BTreeSet::new(),
                    internal_dev_deps: BTreeSet::new(),
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
        let dependents = build_dependency_graph(&workspace, &ignored_packages(&workspace, &config));

        // examples-package should not appear in the dependency graph because it's ignored
        // So main-package should not appear as a dependent of examples-package
        assert!(!dependents.contains_key("cargo/examples-package"));

        // The dependency graph should be empty since examples-package is ignored
        // and main-package depends on it
        assert!(dependents.is_empty());
    }

    #[test]
    fn test_stabilize_version_from_parsed() {
        fn stabilize(v: &str, bump: Bump) -> String {
            let parsed = parse_version_string(v).unwrap();
            stabilize_version_from_parsed(&parsed, bump).unwrap()
        }

        // Patch-implied prerelease: bump ≤ implied strips suffix, bump > implied applies bump first
        assert_eq!(stabilize("0.2.7-alpha.6", Bump::Patch), "0.2.7");
        assert_eq!(stabilize("0.2.7-alpha.6", Bump::Minor), "0.3.0");
        assert_eq!(stabilize("0.2.7-alpha.6", Bump::Major), "1.0.0");

        // Minor-implied prerelease
        assert_eq!(stabilize("0.3.0-alpha", Bump::Patch), "0.3.0");
        assert_eq!(stabilize("0.3.0-alpha", Bump::Minor), "0.3.0");
        assert_eq!(stabilize("0.3.0-alpha", Bump::Major), "1.0.0");
        assert_eq!(stabilize("0.3.0-alpha.1", Bump::Patch), "0.3.0");
        assert_eq!(stabilize("0.3.0-alpha.1", Bump::Minor), "0.3.0");

        // Major-implied prerelease: any bump strips the suffix
        assert_eq!(stabilize("1.0.0-alpha", Bump::Patch), "1.0.0");
        assert_eq!(stabilize("1.0.0-alpha", Bump::Minor), "1.0.0");
        assert_eq!(stabilize("1.0.0-alpha", Bump::Major), "1.0.0");

        // Stable input: behaves like a normal bump
        assert_eq!(stabilize("0.2.7", Bump::Patch), "0.2.8");
        assert_eq!(stabilize("0.2.7", Bump::Minor), "0.3.0");
        assert_eq!(stabilize("0.2.7", Bump::Major), "1.0.0");
    }
}
