use crate::types::{Bump, CrateInfo, DependencyUpdate, Workspace};
use crate::{
    changeset::ChangesetInfo, config::Config, detect_changesets_dir,
    detect_github_repo_slug_with_config, discover_workspace, enrich_changeset_message,
    get_commit_hash_for_path, load_changesets,
};
use rustc_hash::FxHashSet;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path};

/// Format dependency updates for changelog display
///
/// Creates a message in the style of Changesets for dependency updates,
/// e.g., "Updated dependencies [hash]: pkg1@1.2.0, pkg2@2.0.0"
pub fn format_dependency_updates_message(updates: &[DependencyUpdate]) -> Option<String> {
    if updates.is_empty() {
        return None;
    }

    let dep_list = updates
        .iter()
        .map(|dep| format!("{}@{}", dep.name, dep.new_version))
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
/// A map of package name to list of (message, bump_type) explanations to add to changelogs
pub fn detect_all_dependency_explanations(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    releases: &BTreeMap<String, (String, String)>,
) -> BTreeMap<String, Vec<(String, Bump)>> {
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, Bump)>> = BTreeMap::new();

    // 1. Detect packages bumped due to fixed dependency group policy
    let bumped_packages: BTreeSet<String> = releases.keys().cloned().collect();
    let policy_packages =
        detect_fixed_dependency_policy_packages(changesets, workspace, config, &bumped_packages);

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
            .push((msg, bump_type));
    }

    // 2. Detect packages bumped due to internal dependency updates
    // Note: Even packages with explicit changesets can have dependency updates

    // Build new version lookup from releases
    let new_version_by_name: BTreeMap<String, String> = releases
        .iter()
        .map(|(name, (_old, new_ver))| (name.clone(), new_ver.clone()))
        .collect();

    // Build map of crate name -> CrateInfo for quick lookup
    let by_name: BTreeMap<String, &CrateInfo> = workspace
        .members
        .iter()
        .map(|c| (c.name.clone(), c))
        .collect();

    // For each released crate, check if it has internal dependencies that were updated
    for crate_name in releases.keys() {
        if let Some(crate_info) = by_name.get(crate_name) {
            // Find which internal dependencies were updated
            let mut updated_deps = Vec::new();
            for dep_name in &crate_info.internal_deps {
                if let Some(new_version) = new_version_by_name.get(dep_name as &str) {
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

    messages_by_pkg
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
) -> BTreeMap<String, Bump> {
    // Build set of packages with direct changesets
    let packages_with_changesets: BTreeSet<String> = changesets
        .iter()
        .flat_map(|cs| cs.packages.iter().cloned())
        .collect();

    // Build dependency graph (dependent -> set of dependencies)
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for crate_info in &workspace.members {
        for dep_name in &crate_info.internal_deps {
            dependents
                .entry(dep_name.clone())
                .or_default()
                .insert(crate_info.name.clone());
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
        for group in &config.fixed_dependencies {
            if group.contains(&pkg_name.to_string()) {
                // Check if any other package in this group has changes
                let has_affected_group_member = group.iter().any(|group_member| {
                    group_member != pkg_name
                        && (packages_with_changesets.contains(group_member)
                            || packages_affected_by_cascade.contains(group_member))
                });

                if has_affected_group_member {
                    // Find the highest bump level in the group to determine the policy bump
                    let group_bump = group
                        .iter()
                        .filter_map(|member| {
                            if packages_with_changesets.contains(member) {
                                // Find the highest bump from changesets affecting this member
                                changesets
                                    .iter()
                                    .filter(|cs| cs.packages.contains(member))
                                    .map(|cs| cs.bump)
                                    .max()
                            } else {
                                None
                            }
                        })
                        .max()
                        .unwrap_or(Bump::Patch);

                    result.insert(pkg_name.clone(), group_bump);
                    break;
                }
            }
        }
    }

    result
}

/// Type alias for initial bumps computation result
type InitialBumpsResult = (
    BTreeMap<String, Bump>,                // bump_by_pkg
    BTreeMap<String, Vec<(String, Bump)>>, // messages_by_pkg
    BTreeSet<std::path::PathBuf>,          // used_paths
);

/// Type alias for release plan
type ReleasePlan = Vec<(String, String, String)>; // (name, old_version, new_version)

/// Main release function that can be called from CLI or other interfaces
pub fn run_release(root: &std::path::Path, dry_run: bool) -> io::Result<()> {
    let ws = discover_workspace(root).map_err(io::Error::other)?;
    let cfg = Config::load(&ws.root).map_err(io::Error::other)?;

    // Validate fixed dependencies configuration
    validate_fixed_dependencies(&cfg, &ws).map_err(io::Error::other)?;

    let changesets_dir = detect_changesets_dir(&ws.root);
    let changesets = load_changesets(&changesets_dir)?;
    if changesets.is_empty() {
        println!(
            "No changesets found in {}",
            ws.root.join(".sampo").join("changesets").display()
        );
        return Ok(());
    }

    // Compute initial bumps from changesets
    let (mut bump_by_pkg, mut messages_by_pkg, used_paths) =
        compute_initial_bumps(&changesets, &ws, &cfg)?;

    if bump_by_pkg.is_empty() {
        println!("No applicable packages found in changesets.");
        return Ok(());
    }

    // Build dependency graph and apply cascading logic
    let dependents = build_dependency_graph(&ws);
    apply_dependency_cascade(&mut bump_by_pkg, &dependents, &cfg);
    apply_linked_dependencies(&mut bump_by_pkg, &cfg);

    // Prepare and validate release plan
    let releases = prepare_release_plan(&bump_by_pkg, &ws)?;
    if releases.is_empty() {
        println!("No matching workspace crates to release.");
        return Ok(());
    }

    print_release_plan(&releases);

    if dry_run {
        println!("Dry-run: no files modified, no tags created.");
        return Ok(());
    }

    // Apply changes
    apply_releases(&releases, &ws, &mut messages_by_pkg, &changesets, &cfg)?;

    // Clean up
    cleanup_consumed_changesets(used_paths)?;

    Ok(())
}

/// Compute initial bumps from changesets and collect messages
fn compute_initial_bumps(
    changesets: &[ChangesetInfo],
    ws: &Workspace,
    cfg: &Config,
) -> io::Result<InitialBumpsResult> {
    let mut bump_by_pkg: BTreeMap<String, Bump> = BTreeMap::new();
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, Bump)>> = BTreeMap::new();
    let mut used_paths: BTreeSet<std::path::PathBuf> = BTreeSet::new();

    // Resolve GitHub repo slug once if available (config, env or origin remote)
    let repo_slug = detect_github_repo_slug_with_config(&ws.root, cfg.github_repository.as_deref());
    let github_token = std::env::var("GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GH_TOKEN").ok());

    for cs in changesets {
        for pkg in &cs.packages {
            used_paths.insert(cs.path.clone());
            bump_by_pkg
                .entry(pkg.clone())
                .and_modify(|b| {
                    if cs.bump > *b {
                        *b = cs.bump;
                    }
                })
                .or_insert(cs.bump);

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

            messages_by_pkg
                .entry(pkg.clone())
                .or_default()
                .push((enriched, cs.bump));
        }
    }

    Ok((bump_by_pkg, messages_by_pkg, used_paths))
}

/// Build reverse dependency graph: dep -> set of dependents
fn build_dependency_graph(ws: &Workspace) -> BTreeMap<String, BTreeSet<String>> {
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in &ws.members {
        for dep in &c.internal_deps {
            dependents
                .entry(dep.clone())
                .or_default()
                .insert(c.name.clone());
        }
    }
    dependents
}

/// Apply dependency cascade logic and fixed dependency groups
fn apply_dependency_cascade(
    bump_by_pkg: &mut BTreeMap<String, Bump>,
    dependents: &BTreeMap<String, BTreeSet<String>>,
    cfg: &Config,
) {
    // Helper function to find which fixed group a package belongs to, if any
    let find_fixed_group = |pkg_name: &str| -> Option<usize> {
        cfg.fixed_dependencies
            .iter()
            .position(|group| group.contains(&pkg_name.to_string()))
    };

    let mut queue: Vec<String> = bump_by_pkg.keys().cloned().collect();
    let mut seen: BTreeSet<String> = queue.iter().cloned().collect();

    while let Some(changed) = queue.pop() {
        let changed_bump = bump_by_pkg.get(&changed).copied().unwrap_or(Bump::Patch);

        // 1. Handle normal dependency relationships (unchanged → dependent)
        if let Some(deps) = dependents.get(&changed) {
            for dep_name in deps {
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
            for group_member in &cfg.fixed_dependencies[group_idx] {
                if group_member != &changed {
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
    }
}

/// Apply linked dependencies logic: highest bump level to affected packages only
fn apply_linked_dependencies(bump_by_pkg: &mut BTreeMap<String, Bump>, cfg: &Config) {
    for group in &cfg.linked_dependencies {
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
}

/// Prepare the release plan by matching bumps to workspace members
fn prepare_release_plan(
    bump_by_pkg: &BTreeMap<String, Bump>,
    ws: &Workspace,
) -> io::Result<ReleasePlan> {
    // Map crate name -> CrateInfo for quick lookup
    let mut by_name: BTreeMap<String, &CrateInfo> = BTreeMap::new();
    for c in &ws.members {
        by_name.insert(c.name.clone(), c);
    }

    let mut releases: Vec<(String, String, String)> = Vec::new(); // (name, old_version, new_version)
    for (name, bump) in bump_by_pkg {
        if let Some(info) = by_name.get(name) {
            let old = if info.version.is_empty() {
                "0.0.0".to_string()
            } else {
                info.version.clone()
            };

            let newv = bump_version(&old, *bump).unwrap_or_else(|_| old.clone());

            releases.push((name.clone(), old, newv));
        }
    }

    Ok(releases)
}

/// Print the planned releases
fn print_release_plan(releases: &ReleasePlan) {
    println!("Planned releases:");
    for (name, old, newv) in releases {
        println!("  {name}: {old} -> {newv}");
    }
}

/// Apply all releases: update manifests and changelogs
fn apply_releases(
    releases: &ReleasePlan,
    ws: &Workspace,
    messages_by_pkg: &mut BTreeMap<String, Vec<(String, Bump)>>,
    changesets: &[ChangesetInfo],
    cfg: &Config,
) -> io::Result<()> {
    // Build lookup maps
    let mut by_name: BTreeMap<String, &CrateInfo> = BTreeMap::new();
    for c in &ws.members {
        by_name.insert(c.name.clone(), c);
    }

    let mut new_version_by_name: BTreeMap<String, String> = BTreeMap::new();
    for (name, _old, newv) in releases {
        new_version_by_name.insert(name.clone(), newv.clone());
    }

    // Build releases map for dependency explanations
    let releases_map: BTreeMap<String, (String, String)> = releases
        .iter()
        .map(|(name, old, new)| (name.clone(), (old.clone(), new.clone())))
        .collect();

    // Use unified function to detect all dependency explanations
    let dependency_explanations =
        detect_all_dependency_explanations(changesets, ws, cfg, &releases_map);

    // Merge dependency explanations into existing messages
    for (pkg_name, explanations) in dependency_explanations {
        messages_by_pkg
            .entry(pkg_name)
            .or_default()
            .extend(explanations);
    }

    // Apply updates for each release
    for (name, old, newv) in releases {
        let info = by_name.get(name.as_str()).unwrap();
        let manifest_path = info.path.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path)?;

        // Update manifest versions
        let (updated, _dep_updates) =
            update_manifest_versions(&text, Some(newv.as_str()), ws, &new_version_by_name)?;
        fs::write(&manifest_path, updated)?;

        let messages = messages_by_pkg.get(name).cloned().unwrap_or_default();
        update_changelog(&info.path, name, old, newv, &messages)?;
    }

    Ok(())
}

/// Clean up consumed changeset files
fn cleanup_consumed_changesets(used_paths: BTreeSet<std::path::PathBuf>) -> io::Result<()> {
    for p in used_paths {
        let _ = fs::remove_file(p);
    }
    println!("Removed consumed changesets.");
    Ok(())
}

/// Bump a semver version string
pub fn bump_version(old: &str, bump: Bump) -> Result<String, String> {
    let mut parts = old
        .split('.')
        .map(|s| s.parse::<u64>().unwrap_or(0))
        .collect::<Vec<_>>();
    while parts.len() < 3 {
        parts.push(0);
    }
    let (maj, min, pat) = (parts[0], parts[1], parts[2]);
    let (maj, min, pat) = match bump {
        Bump::Patch => (maj, min, pat + 1),
        Bump::Minor => (maj, min + 1, 0),
        Bump::Major => (maj + 1, 0, 0),
    };
    Ok(format!("{maj}.{min}.{pat}"))
}

/// Update a crate manifest, setting the crate version (if provided) and retargeting
/// internal dependency version requirements to the latest planned versions.
/// Returns the updated TOML string along with a list of (dep_name, new_version) applied.
pub fn update_manifest_versions(
    input: &str,
    new_pkg_version: Option<&str>,
    ws: &Workspace,
    new_version_by_name: &BTreeMap<String, String>,
) -> io::Result<(String, Vec<(String, String)>)> {
    let mut value: toml::Value = input
        .parse()
        .map_err(|e| io::Error::other(format!("invalid Cargo.toml: {e}")))?;

    if let Some(v) = new_pkg_version
        && let Some(pkg) = value.get_mut("package").and_then(toml::Value::as_table_mut)
    {
        pkg.insert("version".into(), toml::Value::String(v.to_string()));
    }

    let workspace_crates: BTreeSet<String> = ws.members.iter().map(|c| c.name.clone()).collect();
    let mut applied: Vec<(String, String)> = Vec::new();

    // helper to try update one dependency entry
    fn update_dep_entry(
        key: &str,
        entry: &mut toml::Value,
        workspace_crates: &BTreeSet<String>,
        new_version_by_name: &BTreeMap<String, String>,
        crate_dirs: &BTreeMap<String, std::path::PathBuf>,
        base_dir: &std::path::Path,
    ) -> Option<(String, String)> {
        match entry {
            toml::Value::String(ver) => {
                // If the key itself matches a workspace crate with a new version, update string
                if let Some(newv) = new_version_by_name.get(key)
                    && workspace_crates.contains(key)
                {
                    *ver = newv.clone();
                    return Some((key.to_string(), newv.clone()));
                }
            }
            toml::Value::Table(tbl) => {
                // Determine the real crate name: key or overridden via 'package'
                let mut real_name = key.to_string();
                if let Some(toml::Value::String(pkg_name)) = tbl.get("package") {
                    real_name = pkg_name.clone();
                }

                // If path points to a workspace crate, prefer that crate's name
                if let Some(toml::Value::String(path_str)) = tbl.get("path") {
                    let dep_path = clean_path_like(&base_dir.join(path_str));
                    if let Some(name) = crate_name_by_path(crate_dirs, &dep_path) {
                        real_name = name;
                    }
                }

                // Skip pure workspace deps (managed at workspace level)
                if matches!(tbl.get("workspace"), Some(toml::Value::Boolean(true))) {
                    return None;
                }

                if let Some(newv) = new_version_by_name.get(&real_name)
                    && workspace_crates.contains(&real_name)
                {
                    tbl.insert("version".into(), toml::Value::String(newv.clone()));
                    return Some((real_name, newv.clone()));
                }
            }
            _ => {}
        }
        None
    }

    // Build helper maps for path resolution
    let mut crate_dirs: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    for c in &ws.members {
        crate_dirs.insert(c.name.clone(), c.path.clone());
    }

    // Resolve manifest base_dir from package.name
    let current_crate_name = value
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|t| t.get("name"))
        .and_then(toml::Value::as_str)
        .unwrap_or("")
        .to_string();
    let base_dir = ws
        .members
        .iter()
        .find(|c| c.name == current_crate_name)
        .map(|c| c.path.as_path().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // Update dependencies across dependency sections
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(t) = value.get_mut(section).and_then(toml::Value::as_table_mut) {
            // Clone keys to avoid borrow issues while mutating
            let keys: Vec<String> = t.keys().cloned().collect();
            for dep_key in keys {
                if let Some(entry) = t.get_mut(&dep_key)
                    && let Some((dep_name, ver)) = update_dep_entry(
                        &dep_key,
                        entry,
                        &workspace_crates,
                        new_version_by_name,
                        &crate_dirs,
                        &base_dir,
                    )
                {
                    applied.push((dep_name, ver));
                }
            }
        }
    }

    // Also handle table-style per-dependency sections like [dependencies.foo]
    // toml::Value already represents those as entries in the tables above, so no extra work.

    let out = toml::to_string(&value)
        .map_err(|e| io::Error::other(format!("failed to serialize Cargo.toml: {e}")))?;
    Ok((out, applied))
}

fn crate_name_by_path(
    crate_dirs: &BTreeMap<String, std::path::PathBuf>,
    dep_path: &Path,
) -> Option<String> {
    let cleaned = clean_path_like(dep_path);
    for (name, p) in crate_dirs {
        if clean_path_like(p) == cleaned {
            return Some(name.clone());
        }
    }
    None
}

fn clean_path_like(p: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn update_changelog(
    crate_dir: &Path,
    package: &str,
    old_version: &str,
    new_version: &str,
    entries: &[(String, Bump)],
) -> io::Result<()> {
    let path = crate_dir.join("CHANGELOG.md");
    let existing = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let mut body = existing.trim_start_matches('\u{feff}').to_string();
    // Remove existing top package header if present
    let package_header = format!("# {}", package);
    if body.starts_with(&package_header) {
        if let Some(idx) = body.find('\n') {
            body = body[idx + 1..].to_string();
        } else {
            body.clear();
        }
    }

    // Parse and merge the current top section only if it's an unpublished section.
    // Heuristic: if the top section header equals the current (old) version, it is published
    // and must be preserved. Otherwise, treat it as in-progress and merge its bullets.
    let mut merged_major: Vec<String> = Vec::new();
    let mut merged_minor: Vec<String> = Vec::new();
    let mut merged_patch: Vec<String> = Vec::new();

    // helper to push without duplicates (preserve append order)
    let push_unique = |list: &mut Vec<String>, msg: &str| {
        if !list.iter().any(|m| m == msg) {
            list.push(msg.to_string());
        }
    };

    // Collect new entries
    for (msg, bump) in entries {
        match bump {
            Bump::Major => push_unique(&mut merged_major, msg),
            Bump::Minor => push_unique(&mut merged_minor, msg),
            Bump::Patch => push_unique(&mut merged_patch, msg),
        }
    }

    // If body starts with a previous top section (## ...), inspect its header.
    // If header == old_version => preserve it (do not merge or strip).
    // Else => parse and merge its bullets, then strip that section.
    let trimmed = body.trim_start();
    if trimmed.starts_with("## ") {
        // Extract first header line text
        let mut lines_iter = trimmed.lines();
        let header_line = lines_iter.next().unwrap_or("").trim();
        let header_text = header_line.trim_start_matches("## ").trim();

        let is_published_top = header_text == old_version;

        if !is_published_top {
            // Determine the extent of the first section in 'trimmed'
            let after_header_offset = header_line.len();
            let rest_after_header = &trimmed[after_header_offset..];
            // Find next section marker starting at a new line
            let next_rel = rest_after_header.find("\n## ");
            let (section_text, remaining) = match next_rel {
                Some(pos) => {
                    let end = after_header_offset + pos + 1; // include leading newline
                    (&trimmed[..end], &trimmed[end..])
                }
                None => (trimmed, ""),
            };

            let mut current = None::<&str>;
            for line in section_text.lines() {
                let t = line.trim();
                if t.eq_ignore_ascii_case("### Major changes") {
                    current = Some("major");
                    continue;
                } else if t.eq_ignore_ascii_case("### Minor changes") {
                    current = Some("minor");
                    continue;
                } else if t.eq_ignore_ascii_case("### Patch changes") {
                    current = Some("patch");
                    continue;
                }
                if t.starts_with("- ") {
                    let msg = t.trim_start_matches("- ").trim();
                    match current {
                        Some("major") => push_unique(&mut merged_major, msg),
                        Some("minor") => push_unique(&mut merged_minor, msg),
                        Some("patch") => push_unique(&mut merged_patch, msg),
                        _ => {}
                    }
                }
            }

            body = remaining.to_string();
        }
    }

    // Build new aggregated top section
    let mut section = String::new();
    section.push_str(&format!("# {}\n\n", package));
    section.push_str(&format!("## {}\n\n", new_version));

    if !merged_major.is_empty() {
        section.push_str("### Major changes\n\n");
        for msg in &merged_major {
            section.push_str("- ");
            section.push_str(msg);
            section.push('\n');
        }
        section.push('\n');
    }
    if !merged_minor.is_empty() {
        section.push_str("### Minor changes\n\n");
        for msg in &merged_minor {
            section.push_str("- ");
            section.push_str(msg);
            section.push('\n');
        }
        section.push('\n');
    }
    if !merged_patch.is_empty() {
        section.push_str("### Patch changes\n\n");
        for msg in &merged_patch {
            section.push_str("- ");
            section.push_str(msg);
            section.push('\n');
        }
        section.push('\n');
    }

    let combined = if body.trim().is_empty() {
        section
    } else {
        format!("{}{}", section, body)
    };
    fs::write(&path, combined)
}

/// Validate fixed dependencies configuration against the workspace
fn validate_fixed_dependencies(cfg: &Config, ws: &Workspace) -> Result<(), String> {
    let workspace_packages: FxHashSet<String> = ws.members.iter().map(|c| c.name.clone()).collect();

    for (group_idx, group) in cfg.fixed_dependencies.iter().enumerate() {
        for package in group {
            if !workspace_packages.contains(package) {
                let available_packages: Vec<String> = workspace_packages.iter().cloned().collect();
                return Err(format!(
                    "Package '{}' in fixed dependency group {} does not exist in the workspace. Available packages: [{}]",
                    package,
                    group_idx + 1,
                    available_packages.join(", ")
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_single_dependency_update() {
        let updates = vec![DependencyUpdate {
            name: "pkg1".to_string(),
            new_version: "1.2.0".to_string(),
        }];
        let msg = format_dependency_updates_message(&updates).unwrap();
        assert_eq!(msg, "Updated dependencies: pkg1@1.2.0");
    }

    #[test]
    fn formats_multiple_dependency_updates() {
        let updates = vec![
            DependencyUpdate {
                name: "pkg1".to_string(),
                new_version: "1.2.0".to_string(),
            },
            DependencyUpdate {
                name: "pkg2".to_string(),
                new_version: "2.0.0".to_string(),
            },
        ];
        let msg = format_dependency_updates_message(&updates).unwrap();
        assert_eq!(msg, "Updated dependencies: pkg1@1.2.0, pkg2@2.0.0");
    }

    #[test]
    fn returns_none_for_empty_updates() {
        let updates = vec![];
        let msg = format_dependency_updates_message(&updates);
        assert_eq!(msg, None);
    }

    #[test]
    fn builds_dependency_updates_from_tuples() {
        let tuples = vec![
            ("pkg1".to_string(), "1.2.0".to_string()),
            ("pkg2".to_string(), "2.0.0".to_string()),
        ];
        let updates = build_dependency_updates(&tuples);
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].name, "pkg1");
        assert_eq!(updates[0].new_version, "1.2.0");
        assert_eq!(updates[1].name, "pkg2");
        assert_eq!(updates[1].new_version, "2.0.0");
    }

    #[test]
    fn creates_dependency_update_entry() {
        let updates = vec![DependencyUpdate {
            name: "pkg1".to_string(),
            new_version: "1.2.0".to_string(),
        }];
        let (msg, bump) = create_dependency_update_entry(&updates).unwrap();
        assert_eq!(msg, "Updated dependencies: pkg1@1.2.0");
        assert_eq!(bump, Bump::Patch);
    }

    #[test]
    fn creates_fixed_dependency_policy_entry() {
        let (msg, bump) = create_fixed_dependency_policy_entry(Bump::Major);
        assert_eq!(msg, "Bumped due to fixed dependency group policy");
        assert_eq!(bump, Bump::Major);

        let (msg, bump) = create_fixed_dependency_policy_entry(Bump::Minor);
        assert_eq!(msg, "Bumped due to fixed dependency group policy");
        assert_eq!(bump, Bump::Minor);
    }

    #[test]
    fn infers_bump_from_version_changes() {
        assert_eq!(infer_bump_from_versions("1.0.0", "2.0.0"), Bump::Major);
        assert_eq!(infer_bump_from_versions("1.0.0", "1.1.0"), Bump::Minor);
        assert_eq!(infer_bump_from_versions("1.0.0", "1.0.1"), Bump::Patch);

        // Edge cases
        assert_eq!(infer_bump_from_versions("0.1", "0.2"), Bump::Patch);
        assert_eq!(infer_bump_from_versions("invalid", "1.0.0"), Bump::Patch);
    }

    #[test]
    fn detect_all_dependency_explanations_comprehensive() {
        use crate::types::{CrateInfo, Workspace};
        use std::collections::BTreeSet;
        use std::path::PathBuf;

        // Create test workspace with dependencies
        let ws = Workspace {
            root: PathBuf::from("/test"),
            members: vec![
                CrateInfo {
                    name: "pkg-a".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-a"),
                    internal_deps: BTreeSet::from(["pkg-b".to_string()]),
                },
                CrateInfo {
                    name: "pkg-b".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-b"),
                    internal_deps: BTreeSet::new(),
                },
                CrateInfo {
                    name: "pkg-c".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-c"),
                    internal_deps: BTreeSet::new(),
                },
            ],
        };

        // Create config with fixed dependencies
        let config = Config {
            version: 1,
            github_repository: None,
            changelog_show_commit_hash: true,
            changelog_show_acknowledgments: true,
            fixed_dependencies: vec![vec!["pkg-a".to_string(), "pkg-c".to_string()]],
            linked_dependencies: vec![],
        };

        // Create changeset that affects pkg-b only
        let changesets = vec![ChangesetInfo {
            packages: vec!["pkg-b".to_string()],
            bump: Bump::Minor,
            message: "feat: new feature".to_string(),
            path: PathBuf::from("/test/.sampo/changesets/test.md"),
        }];

        // Simulate releases: pkg-a and pkg-c get fixed bump, pkg-b gets direct bump
        let mut releases = BTreeMap::new();
        releases.insert(
            "pkg-a".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );
        releases.insert(
            "pkg-b".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );
        releases.insert(
            "pkg-c".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );

        let explanations = detect_all_dependency_explanations(&changesets, &ws, &config, &releases);

        // pkg-a should have dependency update message (depends on pkg-b)
        let pkg_a_messages = explanations.get("pkg-a").unwrap();
        assert_eq!(pkg_a_messages.len(), 1);
        assert!(pkg_a_messages[0]
            .0
            .contains("Updated dependencies: pkg-b@1.1.0"));
        assert_eq!(pkg_a_messages[0].1, Bump::Patch);

        // pkg-c should have fixed dependency policy message (no deps but in fixed group)
        let pkg_c_messages = explanations.get("pkg-c").unwrap();
        assert_eq!(pkg_c_messages.len(), 1);
        assert_eq!(
            pkg_c_messages[0].0,
            "Bumped due to fixed dependency group policy"
        );
        assert_eq!(pkg_c_messages[0].1, Bump::Minor); // Inferred from version change

        // pkg-b should have no messages (explicit changeset)
        assert!(!explanations.contains_key("pkg-b"));
    }

    #[test]
    fn detect_all_dependency_explanations_empty_cases() {
        use crate::types::{CrateInfo, Workspace};
        use std::collections::BTreeSet;
        use std::path::PathBuf;

        let ws = Workspace {
            root: PathBuf::from("/test"),
            members: vec![CrateInfo {
                name: "pkg-a".to_string(),
                version: "1.0.0".to_string(),
                path: PathBuf::from("/test/pkg-a"),
                internal_deps: BTreeSet::new(),
            }],
        };

        let config = Config::default();
        let changesets = vec![];
        let releases = BTreeMap::new();

        let explanations = detect_all_dependency_explanations(&changesets, &ws, &config, &releases);
        assert!(explanations.is_empty());
    }
}
