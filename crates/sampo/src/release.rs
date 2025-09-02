use crate::changeset::{Bump, load_all};
use crate::cli::ReleaseArgs;
use crate::config::Config;
use crate::workspace::{CrateInfo, Workspace};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

pub fn run(args: &ReleaseArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    run_in(&cwd, args)
}

pub fn run_in(root: &std::path::Path, args: &ReleaseArgs) -> io::Result<()> {
    let ws = Workspace::discover_from(root).map_err(io::Error::other)?;
    let cfg = Config::load(&ws.root)?;

    let changesets = load_all(&cfg.changesets_dir)?;
    if changesets.is_empty() {
        println!("No changesets found in {}", cfg.changesets_dir.display());
        return Ok(());
    }

    // Compute highest bump per package and collect messages per package
    let mut bump_by_pkg: BTreeMap<String, Bump> = BTreeMap::new();
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, Bump)>> = BTreeMap::new();
    let mut used_paths: BTreeSet<std::path::PathBuf> = BTreeSet::new();
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
            messages_by_pkg
                .entry(pkg.clone())
                .or_default()
                .push((cs.message.clone(), cs.bump));
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

    // Apply: update Cargo.toml, update CHANGELOG, create tag
    for (name, _old, newv) in &releases {
        let info = by_name.get(name.as_str()).unwrap();
        let manifest_path = info.path.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path)?;
        let updated = update_package_version_in_toml(&text, newv)?;
        fs::write(&manifest_path, updated)?;

        // Update changelog
        let messages = messages_by_pkg.get(name).cloned().unwrap_or_default();
        update_changelog(&info.path, name, newv, &messages)?;

        // Git tag
        maybe_tag(&ws.root, name, newv)?;
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

fn update_package_version_in_toml(input: &str, new_version: &str) -> io::Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut in_package = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_package = trimmed == "[package]";
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_package {
            // Replace version assignment in package section
            if let Some(_idx) = trimmed.find("version") {
                // naive key detection, ensure not commented out
                if !trimmed.starts_with('#') && trimmed.starts_with("version") {
                    // preserve leading whitespace up to key
                    let leading = &line[..line.find('v').unwrap_or(0)];
                    out.push_str(leading);
                    out.push_str(&format!("version = \"{}\"\n", new_version));
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

fn update_changelog(
    crate_dir: &Path,
    package: &str,
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
    // Remove existing header if present
    let package_header = format!("# {}", package);
    if body.starts_with(&package_header) {
        // keep content after the first header line
        if let Some(idx) = body.find('\n') {
            body = body[idx + 1..].to_string();
        } else {
            body.clear();
        }
    }

    let mut section = String::new();
    section.push_str(&format!("# {}\n\n", package));
    section.push_str(&format!("## {}\n\n", new_version));

    // Group entries by bump type
    let mut major_entries = Vec::new();
    let mut minor_entries = Vec::new();
    let mut patch_entries = Vec::new();

    for (msg, bump) in entries {
        match bump {
            Bump::Major => major_entries.push(msg),
            Bump::Minor => minor_entries.push(msg),
            Bump::Patch => patch_entries.push(msg),
        }
    }

    // Add sections in order: Major, Minor, Patch
    if !major_entries.is_empty() {
        section.push_str("### Major changes\n\n");
        for msg in major_entries {
            section.push_str("- ");
            section.push_str(msg);
            section.push('\n');
        }
        section.push('\n');
    }

    if !minor_entries.is_empty() {
        section.push_str("### Minor changes\n\n");
        for msg in minor_entries {
            section.push_str("- ");
            section.push_str(msg);
            section.push('\n');
        }
        section.push('\n');
    }

    if !patch_entries.is_empty() {
        section.push_str("### Patch changes\n\n");
        for msg in patch_entries {
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

fn maybe_tag(repo_root: &Path, package: &str, version: &str) -> io::Result<()> {
    if !repo_root.join(".git").exists() {
        // Not a git repo, skip
        return Ok(());
    }
    let tag = format!("{}-v{}", package, version);
    let msg = format!("Release {} {}", package, version);
    let status = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("tag")
        .arg("-a")
        .arg(&tag)
        .arg("-m")
        .arg(&msg)
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("Created tag {}", tag);
            Ok(())
        }
        Ok(s) => Err(io::Error::other(format!(
            "git tag failed with status {}",
            s
        ))),
        Err(e) => Err(io::Error::other(format!("failed to invoke git: {}", e))),
    }
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
        let input = "[package]\nname=\"x\"\nversion = \"0.1.0\"\n\n[dependencies]\n";
        let out = update_package_version_in_toml(input, "0.2.0").unwrap();
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
}
