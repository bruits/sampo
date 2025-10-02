use crate::discover_workspace;
use crate::errors::{Result, SampoError};
use crate::release::{
    parse_version_string, regenerate_lockfile, restore_prerelease_changesets,
    update_manifest_versions,
};
use crate::types::{CrateInfo, Workspace};
use semver::{BuildMetadata, Prerelease};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Represents a version change applied to a package manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionChange {
    pub name: String,
    pub old_version: String,
    pub new_version: String,
}

/// Enter pre-release mode for the selected packages with the provided label.
pub fn enter_prerelease(
    root: &Path,
    packages: &[String],
    label: &str,
) -> Result<Vec<VersionChange>> {
    let workspace = discover_workspace(root)?;
    let targets = resolve_targets(&workspace, packages)?;
    let prerelease = validate_label(label)?;

    let (changes, new_versions) = plan_enter_updates(&targets, &prerelease)?;
    if new_versions.is_empty() {
        return Ok(Vec::new());
    }

    apply_version_updates(&workspace, &new_versions)?;
    Ok(changes)
}

/// Exit pre-release mode for the selected packages, restoring stable versions.
pub fn exit_prerelease(root: &Path, packages: &[String]) -> Result<Vec<VersionChange>> {
    let workspace = discover_workspace(root)?;
    let targets = resolve_targets(&workspace, packages)?;

    let (changes, new_versions) = plan_exit_updates(&targets)?;
    if new_versions.is_empty() {
        return Ok(Vec::new());
    }

    apply_version_updates(&workspace, &new_versions)?;
    Ok(changes)
}

/// Restore any preserved changesets from a prior pre-release phase back into the
/// workspace changeset directory.
///
/// Returns the number of files moved. When no preserved changesets are present,
/// the function behaves as a no-op.
pub fn restore_preserved_changesets(root: &Path) -> Result<usize> {
    let prerelease_dir = root.join(".sampo").join("prerelease");
    if !prerelease_dir.exists() {
        return Ok(0);
    }

    let changesets_dir = root.join(".sampo").join("changesets");
    let mut preserved = 0usize;

    for entry in fs::read_dir(&prerelease_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        preserved += 1;
    }

    if preserved == 0 {
        return Ok(0);
    }

    restore_prerelease_changesets(&prerelease_dir, &changesets_dir)?;
    Ok(preserved)
}

fn resolve_targets<'a>(
    workspace: &'a Workspace,
    packages: &[String],
) -> Result<Vec<&'a CrateInfo>> {
    if packages.is_empty() {
        return Err(SampoError::Prerelease(
            "At least one package must be specified.".to_string(),
        ));
    }

    let mut lookup: BTreeMap<&str, &CrateInfo> = BTreeMap::new();
    for info in &workspace.members {
        lookup.insert(info.name.as_str(), info);
    }

    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut targets = Vec::new();

    for name in packages {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(SampoError::Prerelease(
                "Package names cannot be empty.".to_string(),
            ));
        }
        if !seen.insert(trimmed) {
            continue;
        }
        let info = lookup.get(trimmed).ok_or_else(|| {
            SampoError::NotFound(format!("Package '{}' not found in workspace", trimmed))
        })?;
        targets.push(*info);
    }

    targets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(targets)
}

fn validate_label(label: &str) -> Result<Prerelease> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err(SampoError::Prerelease(
            "Pre-release label cannot be empty.".to_string(),
        ));
    }

    let has_non_numeric = trimmed
        .split('.')
        .any(|segment| segment.chars().any(|ch| !ch.is_ascii_digit()));
    if !has_non_numeric {
        return Err(SampoError::Prerelease(
            "Pre-release label must contain at least one non-numeric identifier.".to_string(),
        ));
    }

    Prerelease::new(trimmed).map_err(|err| {
        SampoError::Prerelease(format!("Invalid pre-release label '{}': {err}", trimmed))
    })
}

fn plan_enter_updates(
    targets: &[&CrateInfo],
    prerelease: &Prerelease,
) -> Result<(Vec<VersionChange>, BTreeMap<String, String>)> {
    let mut changes = Vec::new();
    let mut new_versions: BTreeMap<String, String> = BTreeMap::new();

    for info in targets {
        let version = parse_version_string(&info.version).map_err(|err| {
            SampoError::Prerelease(format!(
                "Invalid semantic version for package '{}': {}",
                info.name, err
            ))
        })?;

        let mut base = version.clone();
        base.pre = Prerelease::EMPTY;
        base.build = BuildMetadata::EMPTY;

        let mut updated = base.clone();
        updated.pre = prerelease.clone();
        let new_version = updated.to_string();

        if new_version == info.version {
            continue;
        }

        new_versions.insert(info.name.clone(), new_version.clone());
        changes.push(VersionChange {
            name: info.name.clone(),
            old_version: info.version.clone(),
            new_version,
        });
    }

    Ok((changes, new_versions))
}

fn plan_exit_updates(
    targets: &[&CrateInfo],
) -> Result<(Vec<VersionChange>, BTreeMap<String, String>)> {
    let mut changes = Vec::new();
    let mut new_versions: BTreeMap<String, String> = BTreeMap::new();

    for info in targets {
        let version = parse_version_string(&info.version).map_err(|err| {
            SampoError::Prerelease(format!(
                "Invalid semantic version for package '{}': {}",
                info.name, err
            ))
        })?;

        if version.pre.is_empty() {
            continue;
        }

        let mut stable = version.clone();
        stable.pre = Prerelease::EMPTY;
        stable.build = BuildMetadata::EMPTY;
        let new_version = stable.to_string();

        if new_version == info.version {
            continue;
        }

        new_versions.insert(info.name.clone(), new_version.clone());
        changes.push(VersionChange {
            name: info.name.clone(),
            old_version: info.version.clone(),
            new_version,
        });
    }

    Ok((changes, new_versions))
}

fn apply_version_updates(
    workspace: &Workspace,
    new_versions: &BTreeMap<String, String>,
) -> Result<()> {
    for info in &workspace.members {
        let manifest_path = info.path.join("Cargo.toml");
        let original = fs::read_to_string(&manifest_path)?;
        let new_pkg_version = new_versions.get(&info.name).map(|s| s.as_str());
        let (updated, _deps) =
            update_manifest_versions(&original, new_pkg_version, workspace, new_versions)?;

        if updated != original {
            fs::write(&manifest_path, updated)?;
        }
    }

    if workspace.root.join("Cargo.lock").exists() {
        regenerate_lockfile(&workspace.root).map_err(|err| match err {
            SampoError::Release(msg) => SampoError::Prerelease(msg),
            other => other,
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_workspace() -> tempfile::TempDir {
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::create_dir_all(root.join("crates/foo")).unwrap();
        fs::create_dir_all(root.join("crates/bar")).unwrap();

        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        temp
    }

    fn write_manifest(path: &Path, name: &str, version: &str) {
        fs::write(
            path.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
    }

    fn append_dependency(path: &Path, dep: &str, dep_version: &str) {
        let manifest_path = path.join("Cargo.toml");
        let current = fs::read_to_string(&manifest_path).unwrap();
        fs::write(
            &manifest_path,
            format!(
                "{}\n[dependencies]\n{dep} = {{ path = \"../{dep}\", version = \"{dep_version}\" }}\n",
                current.trim_end()
            ),
        )
        .unwrap();
    }

    #[test]
    fn enter_sets_prerelease_label_and_updates_dependents() {
        let temp = init_workspace();
        let root = temp.path();

        write_manifest(&root.join("crates/foo"), "foo", "1.2.3");
        write_manifest(&root.join("crates/bar"), "bar", "0.1.0");
        append_dependency(&root.join("crates/bar"), "foo", "1.2.3");

        let updates = enter_prerelease(root, &[String::from("foo")], "alpha").unwrap();
        assert_eq!(
            updates,
            vec![VersionChange {
                name: "foo".to_string(),
                old_version: "1.2.3".to_string(),
                new_version: "1.2.3-alpha".to_string(),
            }]
        );

        let foo_manifest = fs::read_to_string(root.join("crates/foo/Cargo.toml")).unwrap();
        assert!(
            foo_manifest.contains("version = \"1.2.3-alpha\"")
                || foo_manifest.contains("version=\"1.2.3-alpha\"")
        );

        let bar_manifest = fs::read_to_string(root.join("crates/bar/Cargo.toml")).unwrap();
        assert!(
            bar_manifest.contains("version = \"1.2.3-alpha\"")
                || bar_manifest.contains("version=\"1.2.3-alpha\"")
        );
    }

    #[test]
    fn enter_switches_between_labels() {
        let temp = init_workspace();
        let root = temp.path();

        write_manifest(&root.join("crates/foo"), "foo", "1.0.0-beta.3");

        let updates = enter_prerelease(root, &[String::from("foo")], "alpha").unwrap();
        assert_eq!(
            updates,
            vec![VersionChange {
                name: "foo".to_string(),
                old_version: "1.0.0-beta.3".to_string(),
                new_version: "1.0.0-alpha".to_string(),
            }]
        );

        let foo_manifest = fs::read_to_string(root.join("crates/foo/Cargo.toml")).unwrap();
        assert!(foo_manifest.contains("1.0.0-alpha"));
    }

    #[test]
    fn enter_rejects_numeric_only_label() {
        let temp = init_workspace();
        let root = temp.path();

        write_manifest(&root.join("crates/foo"), "foo", "0.1.0");

        let err = enter_prerelease(root, &[String::from("foo")], "123").unwrap_err();
        match err {
            SampoError::Prerelease(msg) => {
                assert!(msg.contains("non-numeric"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn exit_clears_prerelease_and_updates_dependents() {
        let temp = init_workspace();
        let root = temp.path();

        write_manifest(&root.join("crates/foo"), "foo", "2.3.4-alpha.5");
        write_manifest(&root.join("crates/bar"), "bar", "0.2.0");
        append_dependency(&root.join("crates/bar"), "foo", "2.3.4-alpha.5");

        let updates = exit_prerelease(root, &[String::from("foo")]).unwrap();
        assert_eq!(
            updates,
            vec![VersionChange {
                name: "foo".to_string(),
                old_version: "2.3.4-alpha.5".to_string(),
                new_version: "2.3.4".to_string(),
            }]
        );

        let foo_manifest = fs::read_to_string(root.join("crates/foo/Cargo.toml")).unwrap();
        assert!(
            foo_manifest.contains("version = \"2.3.4\"")
                || foo_manifest.contains("version=\"2.3.4\"")
        );

        let bar_manifest = fs::read_to_string(root.join("crates/bar/Cargo.toml")).unwrap();
        assert!(
            bar_manifest.contains("version = \"2.3.4\"")
                || bar_manifest.contains("version=\"2.3.4\"")
        );
    }

    #[test]
    fn restore_preserved_changesets_moves_files() {
        let temp = init_workspace();
        let root = temp.path();

        let prerelease_dir = root.join(".sampo/prerelease");
        fs::create_dir_all(&prerelease_dir).unwrap();
        fs::write(prerelease_dir.join("change.md"), "---\nfoo: minor\n---\n").unwrap();

        let restored = restore_preserved_changesets(root).unwrap();
        assert_eq!(restored, 1);

        let changesets_dir = root.join(".sampo/changesets");
        let restored_entries = fs::read_dir(&changesets_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(restored_entries.len(), 1);
        assert!(
            restored_entries[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("change")
        );

        let remaining = fs::read_dir(&prerelease_dir).unwrap().collect::<Vec<_>>();
        assert!(remaining.is_empty());
    }
}
