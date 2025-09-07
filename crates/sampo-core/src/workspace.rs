use crate::types::{CrateInfo, Workspace};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Errors that can occur when working with workspaces
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("No Cargo.toml with [workspace] found")]
    NotFound,
    #[error("Invalid Cargo.toml: {0}")]
    InvalidToml(String),
    #[error("Invalid workspace: {0}")]
    InvalidWorkspace(String),
}

type Result<T> = std::result::Result<T, WorkspaceError>;

/// Discover a Cargo workspace starting from the given directory
pub fn discover_workspace(start_dir: &Path) -> Result<Workspace> {
    let (root, root_toml) = find_workspace_root(start_dir)?;
    let members = parse_workspace_members(&root, &root_toml)?;
    let mut crates = Vec::new();

    // First pass: parse per-crate metadata (name, version)
    let mut name_to_path: BTreeMap<String, PathBuf> = BTreeMap::new();
    for member_dir in &members {
        let manifest_path = member_dir.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path)?;
        let value: toml::Value = text.parse().map_err(|e| {
            WorkspaceError::InvalidToml(format!("{}: {}", manifest_path.display(), e))
        })?;
        let pkg = value
            .get("package")
            .and_then(|v| v.as_table())
            .ok_or_else(|| {
                WorkspaceError::InvalidToml(format!(
                    "missing [package] in {}",
                    manifest_path.display()
                ))
            })?;
        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                WorkspaceError::InvalidToml(format!(
                    "missing package.name in {}",
                    manifest_path.display()
                ))
            })?
            .to_string();
        let version = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        name_to_path.insert(name.clone(), member_dir.clone());
        crates.push((name, version, member_dir.clone(), value));
    }

    // Second pass: compute internal dependencies
    let mut out: Vec<CrateInfo> = Vec::new();
    for (name, version, path, manifest) in crates {
        let internal_deps = collect_internal_deps(&path, &name_to_path, &manifest);
        out.push(CrateInfo {
            name,
            version,
            path,
            internal_deps,
        });
    }

    Ok(Workspace { root, members: out })
}

/// Parse workspace members from the root Cargo.toml
pub fn parse_workspace_members(root: &Path, root_toml: &toml::Value) -> Result<Vec<PathBuf>> {
    let workspace = root_toml
        .get("workspace")
        .and_then(|v| v.as_table())
        .ok_or(WorkspaceError::NotFound)?;

    let members = workspace
        .get("members")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            WorkspaceError::InvalidWorkspace("missing 'members' in [workspace]".into())
        })?;

    let mut paths = Vec::new();
    for mem in members {
        let pattern = mem.as_str().ok_or_else(|| {
            WorkspaceError::InvalidWorkspace("non-string member in workspace.members".into())
        })?;
        expand_member_pattern(root, pattern, &mut paths)?;
    }

    Ok(paths)
}

/// Find the workspace root starting from a directory
fn find_workspace_root(start_dir: &Path) -> Result<(PathBuf, toml::Value)> {
    let mut current = start_dir;
    loop {
        let toml_path = current.join("Cargo.toml");
        if toml_path.exists() {
            let text = fs::read_to_string(&toml_path)?;
            let value: toml::Value = text.parse().map_err(|e| {
                WorkspaceError::InvalidToml(format!("{}: {}", toml_path.display(), e))
            })?;
            if value.get("workspace").is_some() {
                return Ok((current.to_path_buf(), value));
            }
        }
        current = current.parent().ok_or(WorkspaceError::NotFound)?;
    }
}

/// Expand a member pattern (plain path or glob) into concrete paths
fn expand_member_pattern(root: &Path, pattern: &str, paths: &mut Vec<PathBuf>) -> Result<()> {
    if pattern.contains('*') {
        // Glob pattern
        let full_pattern = root.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();
        let entries = glob::glob(&pattern_str).map_err(|e| {
            WorkspaceError::InvalidWorkspace(format!("invalid glob pattern '{}': {}", pattern, e))
        })?;
        for entry in entries {
            let path = entry
                .map_err(|e| WorkspaceError::InvalidWorkspace(format!("glob error: {}", e)))?;
            // Only include if it has a Cargo.toml
            if path.join("Cargo.toml").exists() {
                paths.push(path);
            }
        }
    } else {
        // Plain path
        let member_path = clean_path(&root.join(pattern));
        if member_path.join("Cargo.toml").exists() {
            paths.push(member_path);
        } else {
            return Err(WorkspaceError::InvalidWorkspace(format!(
                "member '{}' does not contain Cargo.toml",
                pattern
            )));
        }
    }
    Ok(())
}

/// Clean a path by resolving .. and . components
fn clean_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // pop only normal components; keep root prefixes
                if !matches!(
                    result.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_))
                ) {
                    result.pop();
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                result.push(component);
            }
        }
    }
    result
}

/// Collect internal dependencies for a crate
fn collect_internal_deps(
    crate_dir: &Path,
    name_to_path: &BTreeMap<String, PathBuf>,
    manifest: &toml::Value,
) -> BTreeSet<String> {
    let mut internal = BTreeSet::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(tbl) = manifest.get(key).and_then(|v| v.as_table()) {
            for (dep_name, dep_val) in tbl {
                if is_internal_dep(crate_dir, name_to_path, dep_val) {
                    internal.insert(dep_name.clone());
                }
            }
        }
    }
    internal
}

/// Check if a dependency is internal to the workspace
fn is_internal_dep(
    crate_dir: &Path,
    name_to_path: &BTreeMap<String, PathBuf>,
    dep_val: &toml::Value,
) -> bool {
    if let Some(tbl) = dep_val.as_table() {
        // Check for `path = "..."` dependency
        if let Some(path_val) = tbl.get("path")
            && let Some(path_str) = path_val.as_str()
        {
            let dep_path = clean_path(&crate_dir.join(path_str));
            return name_to_path.values().any(|p| *p == dep_path);
        }
        // Check for `workspace = true` dependency
        if let Some(workspace_val) = tbl.get("workspace")
            && workspace_val.as_bool() == Some(true)
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn clean_path_collapses_segments() {
        let input = Path::new("/a/b/../c/./d");
        let expected = PathBuf::from("/a/c/d");
        assert_eq!(clean_path(input), expected);
    }

    #[test]
    fn clean_path_prevents_escaping_root() {
        // Test that we can't escape beyond root directory
        let input = Path::new("/a/../..");
        let expected = PathBuf::from("/");
        assert_eq!(clean_path(input), expected);

        // Test with relative paths
        let input = Path::new("a/../..");
        let expected = PathBuf::from("");
        assert_eq!(clean_path(input), expected);
    }

    #[test]
    fn expand_members_supports_plain_and_glob() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // Create workspace Cargo.toml
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        // Create crates/a and crates/b with manifests
        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("a")).unwrap();
        fs::create_dir_all(crates_dir.join("b")).unwrap();
        fs::write(
            crates_dir.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            crates_dir.join("b/Cargo.toml"),
            "[package]\nname = \"b\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();

        let (_root, root_toml) = find_workspace_root(root).unwrap();
        let members = parse_workspace_members(root, &root_toml).unwrap();
        let mut names: Vec<_> = members
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn glob_skips_non_crate_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("real-crate")).unwrap();
        fs::create_dir_all(crates_dir.join("not-a-crate")).unwrap();
        // Only create Cargo.toml for one
        fs::write(
            crates_dir.join("real-crate/Cargo.toml"),
            "[package]\nname=\"real-crate\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        let (_root, root_toml) = find_workspace_root(root).unwrap();
        let members = parse_workspace_members(root, &root_toml).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(
            members[0].file_name().unwrap().to_string_lossy(),
            "real-crate"
        );
    }

    #[test]
    fn internal_deps_detect_path_and_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        // crates: x depends on y via path, and on z via workspace
        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("x")).unwrap();
        fs::create_dir_all(crates_dir.join("y")).unwrap();
        fs::create_dir_all(crates_dir.join("z")).unwrap();
        fs::write(
            crates_dir.join("x/Cargo.toml"),
            format!(
                "{}{}{}",
                "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
                "[dependencies]\n",
                "y={ path=\"../y\" }\n z={ workspace=true }\n"
            ),
        )
        .unwrap();
        fs::write(
            crates_dir.join("y/Cargo.toml"),
            "[package]\nname=\"y\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            crates_dir.join("z/Cargo.toml"),
            "[package]\nname=\"z\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        let ws = discover_workspace(root).unwrap();
        let x = ws.members.iter().find(|c| c.name == "x").unwrap();
        assert!(x.internal_deps.contains("y"));
        assert!(x.internal_deps.contains("z"));
    }
}
