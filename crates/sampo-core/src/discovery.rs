use crate::errors::WorkspaceError;
use crate::types::{PackageInfo, PackageKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

type Result<T> = std::result::Result<T, WorkspaceError>;

/// Pluggable interface for ecosystem-specific package discovery
pub trait PackageDiscovery {
    fn discover(&self, root: &Path) -> Result<Vec<PackageInfo>>;
    fn can_discover(&self, root: &Path) -> bool;
    fn package_kind(&self) -> PackageKind;
}

pub struct CargoDiscovery;

impl CargoDiscovery {
    /// Find the workspace root starting from a directory
    pub fn find_workspace_root(&self, start_dir: &Path) -> Result<(PathBuf, toml::Value)> {
        let mut current = start_dir;
        loop {
            let toml_path = current.join("Cargo.toml");
            if toml_path.exists() {
                let text = fs::read_to_string(&toml_path).map_err(|e| {
                    WorkspaceError::Io(crate::errors::io_error_with_path(e, &toml_path))
                })?;
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

    /// Parse workspace members from the root Cargo.toml
    pub fn parse_workspace_members(
        &self,
        root: &Path,
        root_toml: &toml::Value,
    ) -> Result<Vec<PathBuf>> {
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
            self.expand_member_pattern(root, pattern, &mut paths)?;
        }

        Ok(paths)
    }

    /// Expand a member pattern (plain path or glob) into concrete paths
    fn expand_member_pattern(
        &self,
        root: &Path,
        pattern: &str,
        paths: &mut Vec<PathBuf>,
    ) -> Result<()> {
        if pattern.contains('*') {
            // Glob pattern
            let full_pattern = root.join(pattern);
            let pattern_str = full_pattern.to_string_lossy();
            let entries = glob::glob(&pattern_str).map_err(|e| {
                WorkspaceError::InvalidWorkspace(format!(
                    "invalid glob pattern '{}': {}",
                    pattern, e
                ))
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

    /// Collect internal dependencies for a crate
    fn collect_internal_deps(
        &self,
        crate_dir: &Path,
        name_to_path: &BTreeMap<String, PathBuf>,
        manifest: &toml::Value,
    ) -> BTreeSet<String> {
        let mut internal = BTreeSet::new();
        for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(tbl) = manifest.get(key).and_then(|v| v.as_table()) {
                for (dep_name, dep_val) in tbl {
                    if self.is_internal_dep(crate_dir, name_to_path, dep_name, dep_val) {
                        internal.insert(dep_name.clone());
                    }
                }
            }
        }
        internal
    }

    /// Check if a dependency is internal to the workspace
    fn is_internal_dep(
        &self,
        crate_dir: &Path,
        name_to_path: &BTreeMap<String, PathBuf>,
        dep_name: &str,
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
                // Only internal if dependency name is another workspace member
                return name_to_path.contains_key(dep_name);
            }
        }
        false
    }
}

impl PackageDiscovery for CargoDiscovery {
    fn discover(&self, root: &Path) -> Result<Vec<PackageInfo>> {
        let (workspace_root, root_toml) = self.find_workspace_root(root)?;
        let members = self.parse_workspace_members(&workspace_root, &root_toml)?;
        let mut crates = Vec::new();

        // First pass: parse per-crate metadata (name, version)
        let mut name_to_path: BTreeMap<String, PathBuf> = BTreeMap::new();
        for member_dir in &members {
            let manifest_path = member_dir.join("Cargo.toml");
            let text = fs::read_to_string(&manifest_path).map_err(|e| {
                WorkspaceError::Io(crate::errors::io_error_with_path(e, &manifest_path))
            })?;
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
        let mut out: Vec<PackageInfo> = Vec::new();
        for (name, version, path, manifest) in crates {
            let internal_deps = self.collect_internal_deps(&path, &name_to_path, &manifest);
            out.push(PackageInfo {
                name,
                version,
                path,
                internal_deps,
                kind: PackageKind::Cargo,
            });
        }

        Ok(out)
    }

    fn can_discover(&self, root: &Path) -> bool {
        root.join("Cargo.toml").exists()
    }

    fn package_kind(&self) -> PackageKind {
        PackageKind::Cargo
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn cargo_discovery_can_discover_cargo_workspace() {
        let discovery = CargoDiscovery;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create a Cargo workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"pkg-a\"]\n",
        )
        .unwrap();

        assert!(discovery.can_discover(root));
    }

    #[test]
    fn cargo_discovery_cannot_discover_non_cargo() {
        let discovery = CargoDiscovery;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // No Cargo.toml
        assert!(!discovery.can_discover(root));
    }

    #[test]
    fn cargo_discovery_discovers_packages() {
        let discovery = CargoDiscovery;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        // Create crates
        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("pkg-a")).unwrap();
        fs::create_dir_all(crates_dir.join("pkg-b")).unwrap();
        fs::write(
            crates_dir.join("pkg-a/Cargo.toml"),
            "[package]\nname = \"pkg-a\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            crates_dir.join("pkg-b/Cargo.toml"),
            "[package]\nname = \"pkg-b\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();

        let packages = discovery.discover(root).unwrap();
        assert_eq!(packages.len(), 2);

        let mut names: Vec<_> = packages.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);

        // All should be Cargo packages
        assert!(packages.iter().all(|p| p.kind == PackageKind::Cargo));
    }

    #[test]
    fn cargo_discovery_detects_internal_deps() {
        let discovery = CargoDiscovery;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("pkg-a")).unwrap();
        fs::create_dir_all(crates_dir.join("pkg-b")).unwrap();

        // pkg-a depends on pkg-b via path
        fs::write(
            crates_dir.join("pkg-a/Cargo.toml"),
            "[package]\nname=\"pkg-a\"\nversion=\"0.1.0\"\n[dependencies]\npkg-b={ path=\"../pkg-b\" }\n",
        )
        .unwrap();
        fs::write(
            crates_dir.join("pkg-b/Cargo.toml"),
            "[package]\nname=\"pkg-b\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        let packages = discovery.discover(root).unwrap();
        let pkg_a = packages.iter().find(|p| p.name == "pkg-a").unwrap();

        assert!(pkg_a.internal_deps.contains("pkg-b"));
    }
}
