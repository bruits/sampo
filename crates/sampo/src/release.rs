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

pub fn run(args: &ReleaseArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    run_in(&cwd, args)
}

pub fn run_in(root: &std::path::Path, args: &ReleaseArgs) -> io::Result<()> {
    let ws = discover_workspace(root).map_err(io::Error::other)?;
    let cfg = Config::load(&ws.root).map_err(io::Error::other)?;

    let changesets_dir = detect_changesets_dir(&ws.root);
    let changesets = load_changesets(&changesets_dir)?;
    if changesets.is_empty() {
        println!(
            "No changesets found in {}",
            ws.root.join(".sampo").join("changesets").display()
        );
        return Ok(());
    }

    // Compute highest bump per package and collect messages per package
    let mut bump_by_pkg: BTreeMap<String, Bump> = BTreeMap::new();
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, Bump)>> = BTreeMap::new();
    let mut used_paths: BTreeSet<std::path::PathBuf> = BTreeSet::new();

    // Resolve GitHub repo slug once if available (config, env or origin remote)
    let repo_slug = detect_github_repo_slug_with_config(&ws.root, cfg.github_repository.as_deref());
    let github_token = std::env::var("GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GH_TOKEN").ok());

    for cs in &changesets {
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

    if bump_by_pkg.is_empty() {
        println!("No applicable packages found in changesets.");
        return Ok(());
    }

    // Map crate name -> CrateInfo for quick lookup
    let mut by_name: BTreeMap<String, &CrateInfo> = BTreeMap::new();
    for c in &ws.members {
        by_name.insert(c.name.clone(), c);
    }

    // Build reverse dependency graph: dep -> set of dependents
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in &ws.members {
        for dep in &c.internal_deps {
            dependents
                .entry(dep.clone())
                .or_default()
                .insert(c.name.clone());
        }
    }

    // Cascade: auto-bump dependents (at least patch) when an internal dep is bumped
    {
        let mut queue: Vec<String> = bump_by_pkg.keys().cloned().collect();
        let mut seen: BTreeSet<String> = queue.iter().cloned().collect();
        while let Some(changed) = queue.pop() {
            if let Some(deps) = dependents.get(&changed) {
                for dep_name in deps {
                    let entry = bump_by_pkg.entry(dep_name.clone()).or_insert(Bump::Patch);
                    // If already present, keep the higher bump
                    if *entry < Bump::Patch {
                        *entry = Bump::Patch;
                    }
                    if !seen.contains(dep_name) {
                        queue.push(dep_name.clone());
                        seen.insert(dep_name.clone());
                    }
                }
            }
        }
    }

    // Prepare plan
    let mut releases: Vec<(String, String, String)> = Vec::new(); // (name, old_version, new_version)
    for (name, bump) in &bump_by_pkg {
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

    if releases.is_empty() {
        println!("No matching workspace crates to release.");
        return Ok(());
    }

    // Print plan
    println!("Planned releases:");
    for (name, old, newv) in &releases {
        println!("  {name}: {old} -> {newv}");
    }

    if args.dry_run {
        println!("Dry-run: no files modified, no tags created.");
        return Ok(());
    }

    // Build a quick lookup for new versions
    let mut new_version_by_name: BTreeMap<String, String> = BTreeMap::new();
    for (name, _old, newv) in &releases {
        new_version_by_name.insert(name.clone(), newv.clone());
    }

    // Apply: update Cargo.toml (package version + internal dependency versions) and CHANGELOG
    for (name, old, newv) in &releases {
        let info = by_name.get(name.as_str()).unwrap();
        let manifest_path = info.path.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path)?;

        // Update manifest and collect which internal deps were retargeted
        let (updated, dep_updates) =
            update_manifest_versions(&text, Some(newv.as_str()), &ws, &new_version_by_name)?;
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

    // Remove consumed changesets
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
                    let dep_path = base_dir.join(path_str);
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

#[cfg(test)]
mod tests {
    use super::*;

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
        use crate::cli::ReleaseArgs;
        use std::fs;
        use std::path::Path;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        // one crate
        let cdir = root.join("crates/x");
        fs::create_dir_all(&cdir).unwrap();
        let manifest_path = cdir.join("Cargo.toml");
        fs::write(&manifest_path, "[package]\nname=\"x\"\nversion=\"0.1.0\"\n").unwrap();

        // No .sampo/changesets directory created -> load_all returns empty
        super::run_in(root, &ReleaseArgs { dry_run: false }).unwrap();

        // Verify no change to manifest
        let after = fs::read_to_string(&manifest_path).unwrap();
        assert!(after.contains("version=\"0.1.0\"") || after.contains("version = \"0.1.0\""));

        // No changelog created
        assert!(!Path::new(&cdir.join("CHANGELOG.md")).exists());
    }

    #[test]
    fn changelog_top_section_is_merged_and_reheaded() {
        use crate::cli::ReleaseArgs;
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        // crate x @ 0.1.0
        let cdir = root.join("crates/x");
        fs::create_dir_all(&cdir).unwrap();
        fs::write(
            cdir.join("Cargo.toml"),
            "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        // initial changelog with an unpublished section 0.1.1
        fs::write(
            cdir.join("CHANGELOG.md"),
            "# x\n\n## 0.1.1\n\n### Patch changes\n\n- fix: a bug\n\n",
        )
        .unwrap();

        // Configure .sampo changesets
        let csdir = root.join(".sampo/changesets");
        fs::create_dir_all(&csdir).unwrap();
        // add a minor changeset -> should rehead to 0.2.0 and merge entries
        fs::write(
            csdir.join("one.md"),
            "---\npackages:\n  - x\nrelease: minor\n---\n\nfeat: new thing\n",
        )
        .unwrap();

        // run release (not dry-run)
        super::run_in(root, &ReleaseArgs { dry_run: false }).unwrap();

        let log = fs::read_to_string(cdir.join("CHANGELOG.md")).unwrap();
        assert!(log.contains("# x"));
        assert!(log.contains("## 0.2.0"), "should rehead to next version");
        assert!(log.contains("### Minor changes"));
        assert!(log.contains("feat: new thing"));
        assert!(log.contains("### Patch changes"));
        assert!(log.contains("fix: a bug"));

        // ensure only one top section, and previous 0.1.1 header is gone
        assert!(!log.contains("## 0.1.1\n"));
    }

    #[test]
    fn published_top_section_is_preserved_and_new_section_is_added() {
        use crate::cli::ReleaseArgs;
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        // crate x @ 0.1.0
        let cdir = root.join("crates/x");
        fs::create_dir_all(&cdir).unwrap();
        fs::write(
            cdir.join("Cargo.toml"),
            "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        // existing changelog with published 0.1.0 at the top
        fs::write(
            cdir.join("CHANGELOG.md"),
            "# x\n\n## 0.1.0\n\n### Patch changes\n\n- initial patch\n\n",
        )
        .unwrap();

        // Configure .sampo changesets
        let csdir = root.join(".sampo/changesets");
        fs::create_dir_all(&csdir).unwrap();
        // add a minor changeset -> should add a new 0.2.0 section above 0.1.0
        fs::write(
            csdir.join("one.md"),
            "---\npackages:\n  - x\nrelease: minor\n---\n\nfeat: new minor\n",
        )
        .unwrap();

        // run release (not dry-run)
        super::run_in(root, &ReleaseArgs { dry_run: false }).unwrap();

        let log = fs::read_to_string(cdir.join("CHANGELOG.md")).unwrap();
        // The new section should be present and come before 0.1.0
        let idx_new = log.find("## 0.2.0").unwrap();
        let idx_old = log.find("## 0.1.0").unwrap();
        assert!(idx_new < idx_old, "new section must precede published one");
        assert!(log.contains("### Minor changes"));
        assert!(log.contains("feat: new minor"));
        // old section remains intact
        assert!(log.contains("### Patch changes"));
        assert!(log.contains("initial patch"));
    }

    #[test]
    fn auto_bumps_dependents_and_updates_internal_dep_versions() {
        use crate::cli::ReleaseArgs;
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // workspace with two crates: a depends on b
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

        // a depends on b via path + version
        fs::write(
            a_dir.join("Cargo.toml"),
            "[package]\nname=\"a\"\nversion=\"0.1.0\"\n\n[dependencies]\nb = { path=\"../b\", version=\"0.1.0\" }\n",
        )
        .unwrap();

        // Changeset: bump b minor
        let csdir = root.join(".sampo/changesets");
        fs::create_dir_all(&csdir).unwrap();
        fs::write(
            csdir.join("b-minor.md"),
            "---\npackages:\n  - b\nrelease: minor\n---\n\nfeat: b adds new feature\n",
        )
        .unwrap();

        // run release
        super::run_in(root, &ReleaseArgs { dry_run: false }).unwrap();

        // verify b -> 0.2.0
        let b_manifest = fs::read_to_string(b_dir.join("Cargo.toml")).unwrap();
        assert!(
            b_manifest.contains("version=\"0.2.0\"") || b_manifest.contains("version = \"0.2.0\"")
        );

        // verify a bumped patch and its dependency updated to 0.2.0
        let a_manifest = fs::read_to_string(a_dir.join("Cargo.toml")).unwrap();
        assert!(
            a_manifest.contains("version=\"0.1.1\"") || a_manifest.contains("version = \"0.1.1\"")
        );
        // Parse to verify dependency version updated
        let a_toml: toml::Value = a_manifest.parse().unwrap();
        let dep_entry = a_toml
            .get("dependencies")
            .and_then(toml::Value::as_table)
            .and_then(|t| t.get("b"))
            .cloned()
            .expect("dependency 'b' must exist");
        match dep_entry {
            toml::Value::String(v) => assert_eq!(v, "0.2.0"),
            toml::Value::Table(tbl) => {
                let v = tbl.get("version").and_then(toml::Value::as_str).unwrap();
                assert_eq!(v, "0.2.0");
                assert_eq!(
                    tbl.get("path").and_then(toml::Value::as_str).unwrap(),
                    "../b"
                );
            }
            _ => panic!("unexpected dependency entry type"),
        }

        // changelog for a exists with 0.1.1 section
        let a_log = fs::read_to_string(a_dir.join("CHANGELOG.md")).unwrap();
        assert!(a_log.contains("# a"));
        assert!(a_log.contains("## 0.1.1"));
    }
}
