use crate::cli::ReleaseArgs;
use sampo_core::{
    Bump, Config, CrateInfo, build_dependency_updates, create_dependency_update_entry,
    detect_changesets_dir, detect_github_repo_slug_with_config, discover_workspace,
    enrich_changeset_message, get_commit_hash_for_path, load_changesets,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;

/// Type alias for initial bumps computation result
type InitialBumpsResult = (
    BTreeMap<String, Bump>,                // bump_by_pkg
    BTreeMap<String, Vec<(String, Bump)>>, // messages_by_pkg
    BTreeSet<std::path::PathBuf>,          // used_paths
);

/// Type alias for release plan
type ReleasePlan = Vec<(String, String, String)>; // (name, old_version, new_version)

pub fn run(args: &ReleaseArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    run_in(&cwd, args)
}

pub fn run_in(root: &std::path::Path, args: &ReleaseArgs) -> io::Result<()> {
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

    // Add explanatory messages for packages bumped by policy
    add_fixed_dependency_policy_messages(
        &bump_by_pkg,
        &mut messages_by_pkg,
        &changesets,
        &ws,
        &cfg,
    );

    // Prepare and validate release plan
    let releases = prepare_release_plan(&bump_by_pkg, &ws)?;
    if releases.is_empty() {
        println!("No matching workspace crates to release.");
        return Ok(());
    }

    print_release_plan(&releases);

    if args.dry_run {
        println!("Dry-run: no files modified, no tags created.");
        return Ok(());
    }

    // Apply changes
    apply_releases(&releases, &ws, &mut messages_by_pkg)?;

    // Clean up
    cleanup_consumed_changesets(used_paths)?;

    Ok(())
}

/// Compute initial bumps from changesets and collect messages
fn compute_initial_bumps(
    changesets: &[sampo_core::ChangesetInfo],
    ws: &sampo_core::Workspace,
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
fn build_dependency_graph(ws: &sampo_core::Workspace) -> BTreeMap<String, BTreeSet<String>> {
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

/// Add policy messages for packages bumped due to fixed dependency groups
///
/// Identifies packages that were bumped solely due to fixed dependency group policy
/// and adds explanatory messages to their changelogs.
fn add_fixed_dependency_policy_messages(
    bump_by_pkg: &BTreeMap<String, Bump>,
    messages_by_pkg: &mut BTreeMap<String, Vec<(String, Bump)>>,
    changesets: &[sampo_core::ChangesetInfo],
    ws: &sampo_core::Workspace,
    cfg: &Config,
) {
    let bumped_packages: BTreeSet<String> = bump_by_pkg.keys().cloned().collect();
    let policy_packages =
        sampo_core::detect_fixed_dependency_policy_packages(changesets, ws, cfg, &bumped_packages);

    for (pkg_name, bump) in policy_packages {
        let (msg, bump_type) = sampo_core::create_fixed_dependency_policy_entry(bump);
        messages_by_pkg
            .entry(pkg_name)
            .or_default()
            .push((msg, bump_type));
    }
}

/// Prepare the release plan by matching bumps to workspace members
fn prepare_release_plan(
    bump_by_pkg: &BTreeMap<String, Bump>,
    ws: &sampo_core::Workspace,
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
    ws: &sampo_core::Workspace,
    messages_by_pkg: &mut BTreeMap<String, Vec<(String, Bump)>>,
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

    // Apply updates for each release
    for (name, old, newv) in releases {
        let info = by_name.get(name.as_str()).unwrap();
        let manifest_path = info.path.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path)?;

        // Update manifest and collect which internal deps were retargeted
        let (updated, dep_updates) =
            update_manifest_versions(&text, Some(newv.as_str()), ws, &new_version_by_name)?;
        fs::write(&manifest_path, updated)?;

        // Augment messages with dependency update notes
        if !dep_updates.is_empty() {
            let updates = build_dependency_updates(&dep_updates);
            if let Some((msg, bump)) = create_dependency_update_entry(&updates) {
                messages_by_pkg
                    .entry(name.clone())
                    .or_default()
                    .push((msg, bump));
            }
        }

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

fn bump_version(old: &str, bump: Bump) -> Result<String, String> {
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
fn update_manifest_versions(
    input: &str,
    new_pkg_version: Option<&str>,
    ws: &sampo_core::Workspace,
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
    use std::path::{Component, PathBuf};
    let mut out = PathBuf::new();
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
fn validate_fixed_dependencies(cfg: &Config, ws: &sampo_core::Workspace) -> Result<(), String> {
    let workspace_packages: std::collections::HashSet<String> =
        ws.members.iter().map(|c| c.name.clone()).collect();

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
    use crate::cli::ReleaseArgs;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    /// Test workspace builder for reducing test boilerplate
    struct TestWorkspace {
        root: PathBuf,
        _temp_dir: tempfile::TempDir,
        crates: HashMap<String, PathBuf>,
    }

    impl TestWorkspace {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().unwrap();
            let root = temp_dir.path().to_path_buf();

            // Create basic workspace structure
            fs::write(
                root.join("Cargo.toml"),
                "[workspace]\nmembers=[\"crates/*\"]\n",
            )
            .unwrap();

            Self {
                root,
                _temp_dir: temp_dir,
                crates: HashMap::new(),
            }
        }

        fn add_crate(&mut self, name: &str, version: &str) -> &mut Self {
            let crate_dir = self.root.join("crates").join(name);
            fs::create_dir_all(&crate_dir).unwrap();

            fs::write(
                crate_dir.join("Cargo.toml"),
                format!("[package]\nname=\"{}\"\nversion=\"{}\"\n", name, version),
            )
            .unwrap();

            self.crates.insert(name.to_string(), crate_dir);
            self
        }

        fn add_dependency(&mut self, from: &str, to: &str, version: &str) -> &mut Self {
            let from_dir = self.crates.get(from).expect("from crate must exist");
            let current_manifest = fs::read_to_string(from_dir.join("Cargo.toml")).unwrap();

            let dependency_section = format!(
                "\n[dependencies]\n{} = {{ path=\"../{}\", version=\"{}\" }}\n",
                to, to, version
            );

            fs::write(
                from_dir.join("Cargo.toml"),
                current_manifest + &dependency_section,
            )
            .unwrap();

            self
        }

        fn add_changeset(&self, packages: &[&str], release: Bump, message: &str) -> &Self {
            let changesets_dir = self.root.join(".sampo/changesets");
            fs::create_dir_all(&changesets_dir).unwrap();

            let packages_yaml = packages
                .iter()
                .map(|p| format!("  - {}", p))
                .collect::<Vec<_>>()
                .join("\n");

            let release_type = match release {
                Bump::Patch => "patch",
                Bump::Minor => "minor",
                Bump::Major => "major",
            };

            let changeset_content = format!(
                "---\npackages:\n{}\nrelease: {}\n---\n\n{}\n",
                packages_yaml, release_type, message
            );

            // Use message slug as filename to avoid conflicts
            let filename = message
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-')
                .collect::<String>()
                .to_lowercase()
                + ".md";

            fs::write(changesets_dir.join(filename), changeset_content).unwrap();
            self
        }

        fn set_config(&self, config_content: &str) -> &Self {
            fs::create_dir_all(self.root.join(".sampo")).unwrap();
            fs::write(self.root.join(".sampo/config.toml"), config_content).unwrap();
            self
        }

        fn add_existing_changelog(&self, crate_name: &str, content: &str) -> &Self {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            fs::write(crate_dir.join("CHANGELOG.md"), content).unwrap();
            self
        }

        fn run_release(&self, dry_run: bool) -> Result<(), std::io::Error> {
            super::run_in(&self.root, &ReleaseArgs { dry_run })
        }

        fn assert_crate_version(&self, crate_name: &str, expected_version: &str) {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            let manifest = fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();

            let version_check = format!("version=\"{}\"", expected_version);
            let version_check_spaces = format!("version = \"{}\"", expected_version);

            assert!(
                manifest.contains(&version_check) || manifest.contains(&version_check_spaces),
                "Expected {} to have version {}, but manifest was:\n{}",
                crate_name,
                expected_version,
                manifest
            );
        }

        fn assert_dependency_version(
            &self,
            from_crate: &str,
            to_crate: &str,
            expected_version: &str,
        ) {
            let from_dir = self.crates.get(from_crate).expect("from crate must exist");
            let manifest = fs::read_to_string(from_dir.join("Cargo.toml")).unwrap();
            let manifest_toml: toml::Value = manifest.parse().unwrap();

            let dep_entry = manifest_toml
                .get("dependencies")
                .and_then(toml::Value::as_table)
                .and_then(|t| t.get(to_crate))
                .cloned()
                .unwrap_or_else(|| {
                    panic!("dependency '{}' must exist in {}", to_crate, from_crate)
                });

            match dep_entry {
                toml::Value::String(v) => assert_eq!(v, expected_version),
                toml::Value::Table(tbl) => {
                    let v = tbl.get("version").and_then(toml::Value::as_str).unwrap();
                    assert_eq!(v, expected_version);
                }
                _ => panic!("unexpected dependency entry type"),
            }
        }

        fn assert_changelog_contains(&self, crate_name: &str, content: &str) {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            let changelog_path = crate_dir.join("CHANGELOG.md");
            assert!(
                changelog_path.exists(),
                "CHANGELOG.md should exist for {}",
                crate_name
            );

            let changelog = fs::read_to_string(changelog_path).unwrap();
            assert!(
                changelog.contains(content),
                "Expected changelog for {} to contain '{}', but was:\n{}",
                crate_name,
                content,
                changelog
            );
        }

        fn read_changelog(&self, crate_name: &str) -> String {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            let changelog_path = crate_dir.join("CHANGELOG.md");
            if changelog_path.exists() {
                fs::read_to_string(changelog_path).unwrap()
            } else {
                String::new()
            }
        }
    }

    #[test]
    fn bumps_versions() {
        assert_eq!(bump_version("0.0.0", Bump::Patch).unwrap(), "0.0.1");
        assert_eq!(bump_version("0.1.2", Bump::Minor).unwrap(), "0.2.0");
        assert_eq!(bump_version("1.2.3", Bump::Major).unwrap(), "2.0.0");
    }

    #[test]
    fn updates_version_in_toml() {
        use sampo_core::{CrateInfo, Workspace};
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        let input = "[package]\nname=\"x\"\nversion = \"0.1.0\"\n\n[dependencies]\n";
        let ws = Workspace {
            root: PathBuf::from("/test"),
            members: vec![CrateInfo {
                name: "x".to_string(),
                version: "0.1.0".to_string(),
                path: PathBuf::from("/test/crates/x"),
                internal_deps: Default::default(),
            }],
        };
        let new_versions = BTreeMap::new();
        let (out, _) = update_manifest_versions(input, Some("0.2.0"), &ws, &new_versions).unwrap();
        assert!(out.contains("version = \"0.2.0\""));
        assert!(out.contains("[dependencies]"));
    }

    #[test]
    fn no_changesets_returns_ok_and_no_changes() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("x", "0.1.0");

        // No changesets directory created -> load_all returns empty
        workspace.run_release(false).unwrap();

        // Verify no change to manifest
        workspace.assert_crate_version("x", "0.1.0");

        // No changelog created
        let crate_dir = workspace.crates.get("x").unwrap();
        assert!(!crate_dir.join("CHANGELOG.md").exists());
    }

    #[test]
    fn changelog_top_section_is_merged_and_reheaded() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("x", "0.1.0")
            .add_existing_changelog(
                "x",
                "# x\n\n## 0.1.1\n\n### Patch changes\n\n- fix: a bug\n\n",
            )
            .add_changeset(&["x"], Bump::Minor, "feat: new thing");

        workspace.run_release(false).unwrap();

        workspace.assert_crate_version("x", "0.2.0");
        workspace.assert_changelog_contains("x", "# x");
        workspace.assert_changelog_contains("x", "## 0.2.0");
        workspace.assert_changelog_contains("x", "### Minor changes");
        workspace.assert_changelog_contains("x", "feat: new thing");
        workspace.assert_changelog_contains("x", "### Patch changes");
        workspace.assert_changelog_contains("x", "fix: a bug");

        // Ensure only one top section, and previous 0.1.1 header is gone
        let crate_dir = workspace.crates.get("x").unwrap();
        let log = fs::read_to_string(crate_dir.join("CHANGELOG.md")).unwrap();
        assert!(!log.contains("## 0.1.1\n"));
    }

    #[test]
    fn published_top_section_is_preserved_and_new_section_is_added() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("x", "0.1.0")
            .add_existing_changelog(
                "x",
                "# x\n\n## 0.1.0\n\n### Patch changes\n\n- initial patch\n\n",
            )
            .add_changeset(&["x"], Bump::Minor, "feat: new minor");

        workspace.run_release(false).unwrap();

        workspace.assert_crate_version("x", "0.2.0");

        // The new section should be present and come before 0.1.0
        let crate_dir = workspace.crates.get("x").unwrap();
        let log = fs::read_to_string(crate_dir.join("CHANGELOG.md")).unwrap();
        let idx_new = log.find("## 0.2.0").unwrap();
        let idx_old = log.find("## 0.1.0").unwrap();
        assert!(idx_new < idx_old, "new section must precede published one");

        workspace.assert_changelog_contains("x", "### Minor changes");
        workspace.assert_changelog_contains("x", "feat: new minor");
        workspace.assert_changelog_contains("x", "### Patch changes");
        workspace.assert_changelog_contains("x", "initial patch");
    }

    #[test]
    fn auto_bumps_dependents_and_updates_internal_dep_versions() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "0.1.0")
            .add_crate("b", "0.1.0")
            .add_dependency("a", "b", "0.1.0")
            .add_changeset(&["b"], Bump::Minor, "feat: b adds new feature");

        workspace.run_release(false).unwrap();

        // Verify b bumped minor -> 0.2.0
        workspace.assert_crate_version("b", "0.2.0");

        // Verify a auto-bumped patch and its dependency updated to 0.2.0
        workspace.assert_crate_version("a", "0.1.1");
        workspace.assert_dependency_version("a", "b", "0.2.0");

        // Changelog for a exists with 0.1.1 section and dependency update message
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 0.1.1");
        workspace.assert_changelog_contains("a", "Updated dependencies: b@0.2.0");
    }

    #[test]
    fn fixed_dependencies_bump_with_same_level() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("a", "b", "1.0.0")
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"b\"]]\n")
            .add_changeset(&["b"], Bump::Major, "breaking: b breaking change");

        workspace.run_release(false).unwrap();

        // Both should be bumped to 2.0.0 (same level as fixed dependencies)
        workspace.assert_crate_version("a", "2.0.0");
        workspace.assert_crate_version("b", "2.0.0");
        workspace.assert_dependency_version("a", "b", "2.0.0");

        // Both should have changelogs with major bump
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 2.0.0");
        workspace.assert_changelog_contains("b", "# b");
        workspace.assert_changelog_contains("b", "## 2.0.0");
        // Check that the automatically bumped package 'a' has dependency update message
        workspace.assert_changelog_contains("a", "Updated dependencies: b@2.0.0");
    }

    #[test]
    fn fixed_dependencies_bidirectional() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("b", "a", "1.0.0") // b depends on a (reverse)
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"b\"]]\n")
            .add_changeset(&["a"], Bump::Minor, "feat: a adds new feature");

        workspace.run_release(false).unwrap();

        // Both should be bumped to 1.1.0 (bidirectional)
        workspace.assert_crate_version("a", "1.1.0");
        workspace.assert_crate_version("b", "1.1.0");
        workspace.assert_dependency_version("b", "a", "1.1.0");

        // Both should have changelogs
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 1.1.0");
        workspace.assert_changelog_contains("b", "# b");
        workspace.assert_changelog_contains("b", "## 1.1.0");
    }

    #[test]
    fn multiple_fixed_dependency_groups() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_crate("c", "1.0.0")
            .add_crate("d", "1.0.0")
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"b\"], [\"c\", \"d\"]]\n")
            .add_changeset(&["a"], Bump::Minor, "feat: a feature");

        workspace.run_release(false).unwrap();

        // Only a and b should be bumped (same group)
        workspace.assert_crate_version("a", "1.1.0");
        workspace.assert_crate_version("b", "1.1.0");

        // c and d should remain unchanged (different group)
        workspace.assert_crate_version("c", "1.0.0");
        workspace.assert_crate_version("d", "1.0.0");
    }

    #[test]
    fn rejects_nonexistent_package_in_fixed_dependencies() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"nonexistent\"]]\n");

        let result = workspace.run_release(false);
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("Package 'nonexistent' in fixed dependency group"));
        assert!(error_msg.contains("does not exist in the workspace"));
    }

    #[test]
    fn linked_dependencies_basic_scenario() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("a", "b", "1.0.0") // a depends on b
            .set_config("[packages]\nlinked_dependencies = [[\"a\", \"b\"]]\n")
            .add_changeset(&["b"], Bump::Major, "breaking: b breaking change");

        workspace.run_release(false).unwrap();

        // Both should be bumped to 2.0.0 (highest bump level)
        workspace.assert_crate_version("a", "2.0.0");
        workspace.assert_crate_version("b", "2.0.0");
        workspace.assert_dependency_version("a", "b", "2.0.0");
    }

    #[test]
    fn linked_dependencies_mixed_bump_levels() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_crate("c", "1.0.0")
            .add_dependency("a", "b", "1.0.0") // a depends on b
            .add_dependency("c", "b", "1.0.0") // c depends on b
            .set_config("[packages]\nlinked_dependencies = [[\"a\", \"b\", \"c\"]]\n")
            .add_changeset(&["b"], Bump::Minor, "feat: b new feature")
            .add_changeset(&["c"], Bump::Patch, "fix: c bug fix");

        workspace.run_release(false).unwrap();

        // All should be bumped to 1.1.0 (highest bump level is minor)
        workspace.assert_crate_version("a", "1.1.0");
        workspace.assert_crate_version("b", "1.1.0");
        workspace.assert_crate_version("c", "1.1.0");

        // Check that auto-bumped package 'a' has dependency update message
        workspace.assert_changelog_contains("a", "Updated dependencies: b@1.1.0");
    }

    #[test]
    fn linked_dependencies_only_affected_packages() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_crate("c", "1.0.0") // c is in group but has no dependencies
            .add_dependency("a", "b", "1.0.0") // a depends on b
            .set_config("[packages]\nlinked_dependencies = [[\"a\", \"b\", \"c\"]]\n")
            .add_changeset(&["b"], Bump::Minor, "feat: b new feature");

        workspace.run_release(false).unwrap();

        // Only a and b should be bumped (affected by changes)
        workspace.assert_crate_version("a", "1.1.0");
        workspace.assert_crate_version("b", "1.1.0");

        // c should remain unchanged (not affected by dependency cascade)
        workspace.assert_crate_version("c", "1.0.0");
    }

    #[test]
    fn linked_dependencies_comprehensive_behavior() {
        // Comprehensive test to document linked dependencies behavior
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("affected_directly", "1.0.0")      // Has changeset
            .add_crate("affected_by_cascade", "1.0.0")    // Depends on affected_directly  
            .add_crate("unaffected_in_group", "1.0.0")    // In group but no relation
            .add_crate("outside_group", "1.0.0")          // Not in group at all
            .add_dependency("affected_by_cascade", "affected_directly", "1.0.0")
            .set_config("[packages]\nlinked_dependencies = [[\"affected_directly\", \"affected_by_cascade\", \"unaffected_in_group\"]]\n")
            .add_changeset(&["affected_directly"], Bump::Minor, "feat: new feature");

        workspace.run_release(false).unwrap();

        // affected_directly: has changeset -> bumped to 1.1.0 (minor)
        workspace.assert_crate_version("affected_directly", "1.1.0");

        // affected_by_cascade: depends on affected_directly -> bumped by cascade,
        // then upgraded to 1.1.0 due to linked group highest bump
        workspace.assert_crate_version("affected_by_cascade", "1.1.0");

        // unaffected_in_group: in linked group but no changeset and no dependencies
        // -> should NOT be bumped (key behavior!)
        workspace.assert_crate_version("unaffected_in_group", "1.0.0");

        // outside_group: not in any group -> should NOT be bumped
        workspace.assert_crate_version("outside_group", "1.0.0");

        // Verify changelogs
        workspace.assert_changelog_contains("affected_directly", "feat: new feature");
        workspace.assert_changelog_contains(
            "affected_by_cascade",
            "Updated dependencies: affected_directly@1.1.0",
        );

        // unaffected_in_group should have no changelog (not bumped)
        let changelog = workspace.read_changelog("unaffected_in_group");
        assert!(
            changelog.is_empty(),
            "unaffected_in_group should have no changelog"
        );
    }

    #[test]
    fn linked_dependencies_multiple_direct_changes() {
        // Test case: multiple packages in linked group have their own changesets
        // The unaffected package should still not be bumped
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("pkg_a", "1.0.0")           // Has major changeset
            .add_crate("pkg_b", "1.0.0")           // Has minor changeset  
            .add_crate("pkg_c", "1.0.0")           // In group but no changeset, no deps
            .add_crate("pkg_d", "1.0.0")           // Depends on pkg_a
            .add_dependency("pkg_d", "pkg_a", "1.0.0")
            .set_config("[packages]\nlinked_dependencies = [[\"pkg_a\", \"pkg_b\", \"pkg_c\", \"pkg_d\"]]\n")
            .add_changeset(&["pkg_a"], Bump::Major, "breaking: major change in a")
            .add_changeset(&["pkg_b"], Bump::Minor, "feat: minor change in b");

        workspace.run_release(false).unwrap();

        // pkg_a: major changeset -> 2.0.0 (highest bump in group)
        workspace.assert_crate_version("pkg_a", "2.0.0");

        // pkg_b: minor changeset, but upgraded to major due to linked group -> 2.0.0
        workspace.assert_crate_version("pkg_b", "2.0.0");

        // pkg_d: depends on pkg_a, affected by cascade, upgraded to major -> 2.0.0
        workspace.assert_crate_version("pkg_d", "2.0.0");

        // pkg_c: in linked group but no changeset and no dependencies -> NOT bumped
        workspace.assert_crate_version("pkg_c", "1.0.0");

        // Verify changelog messages
        workspace.assert_changelog_contains("pkg_a", "breaking: major change in a");
        workspace.assert_changelog_contains("pkg_b", "feat: minor change in b");
        workspace.assert_changelog_contains("pkg_d", "Updated dependencies: pkg_a@2.0.0");

        // pkg_c should have no changelog
        let changelog = workspace.read_changelog("pkg_c");
        assert!(
            changelog.is_empty(),
            "pkg_c should have no changelog since it wasn't affected"
        );
    }

    #[test]
    fn fixed_dependencies_without_actual_dependency() {
        // Test case: two packages in fixed group but no actual dependency between them
        // Should the auto-bumped package still show "Updated dependencies" message?
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            // Note: no dependency between a and b
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"b\"]]\n")
            .add_changeset(&["b"], Bump::Major, "breaking: b breaking change");

        workspace.run_release(false).unwrap();

        // Both should be bumped to 2.0.0 (same level as fixed dependencies)
        workspace.assert_crate_version("a", "2.0.0");
        workspace.assert_crate_version("b", "2.0.0");

        // The question: should 'a' have "Updated dependencies" message when
        // it doesn't actually depend on 'b'? Currently it won't because
        // apply_releases only adds dependency update messages for actual dependencies.

        // Let's verify this behavior
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 2.0.0");
        // This should NOT contain "Updated dependencies" since there's no actual dependency

        // Let's check what the actual changelog content is
        let changelog_content = workspace.read_changelog("a");
        println!("Changelog content for 'a':\n{}", changelog_content);

        // Package 'a' should have a changelog but with empty sections since no explicit changes
        assert!(!changelog_content.contains("Updated dependencies"));
        assert!(!changelog_content.contains("breaking: b breaking change"));

        // FIXED: Package 'a' should now have an explanation for why it was bumped!
        workspace.assert_changelog_contains("a", "Bumped due to fixed dependency group policy");
    }

    #[test]
    fn fixed_dependencies_complex_scenario() {
        // Test case: multiple packages in fixed group, some with dependencies, some without
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("pkg_a", "1.0.0") // In group but no changes, no dependencies
            .add_crate("pkg_b", "1.0.0") // In group with changeset
            .add_crate("pkg_c", "1.0.0") // In group, depends on pkg_d (outside group)
            .add_crate("pkg_d", "1.0.0") // Not in group but has changeset
            .add_dependency("pkg_c", "pkg_d", "1.0.0")
            .set_config("[packages]\nfixed_dependencies = [[\"pkg_a\", \"pkg_b\", \"pkg_c\"]]\n")
            .add_changeset(&["pkg_b"], Bump::Minor, "feat: pkg_b new feature")
            .add_changeset(&["pkg_d"], Bump::Patch, "fix: pkg_d bug fix");

        workspace.run_release(false).unwrap();

        // All packages in fixed group should be bumped to 1.1.0 (highest bump in group)
        workspace.assert_crate_version("pkg_a", "1.1.0");
        workspace.assert_crate_version("pkg_b", "1.1.0");
        workspace.assert_crate_version("pkg_c", "1.1.0");
        // pkg_d is bumped to 1.0.1 (its own patch changeset)
        workspace.assert_crate_version("pkg_d", "1.0.1");

        // Check changelog messages
        workspace.assert_changelog_contains("pkg_a", "Bumped due to fixed dependency group policy");
        workspace.assert_changelog_contains("pkg_b", "feat: pkg_b new feature");
        workspace.assert_changelog_contains("pkg_c", "Updated dependencies: pkg_d@1.0.1");
        workspace.assert_changelog_contains("pkg_d", "fix: pkg_d bug fix");
    }

    #[test]
    fn package_with_both_changeset_and_dependency_update() {
        // Test case: package has its own changeset AND gets dependency updates
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "0.1.0")
            .add_crate("b", "0.1.0")
            .add_dependency("a", "b", "0.1.0")
            .add_changeset(&["a"], Bump::Minor, "feat: a adds new feature")
            .add_changeset(&["b"], Bump::Patch, "fix: b bug fix");

        workspace.run_release(false).unwrap();

        // a should be bumped minor (0.2.0) due to its own changeset
        workspace.assert_crate_version("a", "0.2.0");
        // b should be bumped patch (0.1.1) due to its changeset
        workspace.assert_crate_version("b", "0.1.1");

        // a should have both its own message AND dependency update message
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 0.2.0");
        workspace.assert_changelog_contains("a", "feat: a adds new feature");
        workspace.assert_changelog_contains("a", "Updated dependencies: b@0.1.1");
    }

    /// Test the complete README scenario: multiple releases in sequence
    #[test]
    fn linked_dependencies_readme_scenario_complete() {
        let mut workspace = TestWorkspace::new();

        // Step 1: Initial state a@1.0.0 depends on b@1.0.0
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("a", "b", "1.0.0")
            .set_config("[packages]\nlinked_dependencies = [[\"a\", \"b\"]]\n");

        // Step 2: b is updated to 2.0.0 (major), a should also get 2.0.0
        workspace.add_changeset(&["b"], Bump::Major, "breaking: b major update");
        workspace.run_release(false).unwrap();

        workspace.assert_crate_version("a", "2.0.0");
        workspace.assert_crate_version("b", "2.0.0");
        workspace.assert_dependency_version("a", "b", "2.0.0");

        // Step 3: Manually update manifests to simulate progression
        // In real scenario, these would be updated by previous release
        let a_dir = workspace.crates.get("a").unwrap();
        let b_dir = workspace.crates.get("b").unwrap();

        fs::write(
            a_dir.join("Cargo.toml"),
            "[package]\nname=\"a\"\nversion=\"2.0.0\"\n\n[dependencies]\nb = { path=\"../b\", version=\"2.0.0\" }\n",
        ).unwrap();
        fs::write(
            b_dir.join("Cargo.toml"),
            "[package]\nname=\"b\"\nversion=\"2.0.0\"\n",
        )
        .unwrap();

        // Step 4: a is updated to 2.1.0 (minor), b should remain at 2.0.0
        workspace.add_changeset(&["a"], Bump::Minor, "feat: a minor update");
        workspace.run_release(false).unwrap();

        workspace.assert_crate_version("a", "2.1.0");
        workspace.assert_crate_version("b", "2.0.0"); // b not affected
    }
}
