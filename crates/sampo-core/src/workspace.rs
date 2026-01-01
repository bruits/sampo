use crate::adapters::PackageAdapter;
use crate::errors::WorkspaceError;
use crate::types::Workspace;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, WorkspaceError>;

const SAMPO_DIR: &str = ".sampo";

/// Find the Sampo root by walking up from `start_dir` looking for `.sampo/`.
///
/// This is the primary way to locate the workspace root for all commands
/// except `sampo init`. Returns `NotInitialized` if `.sampo/` is not found.
pub fn find_sampo_root(start_dir: &Path) -> Result<PathBuf> {
    let mut current = start_dir;
    loop {
        if current.join(SAMPO_DIR).is_dir() {
            return Ok(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }
    Err(WorkspaceError::NotInitialized)
}

/// Discover workspace by first finding the `.sampo/` directory, then discovering packages.
///
/// This is the main entry point for all commands except `sampo init`.
/// It ensures Sampo has been initialized before proceeding.
///
/// Returns `NoPackagesFound` if `.sampo/` exists but no packages are detected,
/// which likely indicates `.sampo/` was created in the wrong directory.
pub fn discover_workspace(start_dir: &Path) -> Result<Workspace> {
    // First, find the Sampo root by looking for .sampo/
    let workspace_root = find_sampo_root(start_dir)?;

    // Then discover packages at that root
    let members = discover_packages_at(&workspace_root)?;

    // No packages found likely means .sampo/ is in the wrong location
    if members.is_empty() {
        return Err(WorkspaceError::NoPackagesFound);
    }

    Ok(Workspace {
        root: workspace_root,
        members,
    })
}

/// Discover packages in a directory using registered ecosystem adapters.
///
/// This is used by `sampo init` to detect packages in the current directory
/// before `.sampo/` exists. It only looks at the given directory, not parents.
pub fn discover_packages_at(root: &Path) -> Result<Vec<crate::types::PackageInfo>> {
    let mut all_members = Vec::new();

    for adapter in PackageAdapter::all() {
        if adapter.can_discover(root) {
            let packages = adapter.discover(root)?;
            all_members.extend(packages);
        }
    }

    Ok(all_members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PackageKind;
    use std::fs;

    fn init_sampo(root: &Path) {
        fs::create_dir_all(root.join(".sampo")).unwrap();
    }

    #[test]
    fn find_sampo_root_finds_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init_sampo(root);

        let result = find_sampo_root(root).unwrap();
        assert_eq!(result, root);
    }

    #[test]
    fn find_sampo_root_walks_up_from_subdirectory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init_sampo(root);

        let deep_dir = root.join("a/b/c/d");
        fs::create_dir_all(&deep_dir).unwrap();

        let result = find_sampo_root(&deep_dir).unwrap();
        assert_eq!(result, root);
    }

    #[test]
    fn find_sampo_root_returns_not_initialized_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        let result = find_sampo_root(root);
        assert!(matches!(result, Err(WorkspaceError::NotInitialized)));
    }

    #[test]
    fn discover_workspace_requires_sampo_init() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let result = discover_workspace(root);
        assert!(matches!(result, Err(WorkspaceError::NotInitialized)));
    }

    #[test]
    fn discover_workspace_finds_cargo_packages() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init_sampo(root);

        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

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

        let ws = discover_workspace(root).unwrap();
        assert_eq!(ws.members.len(), 2);

        let mut names: Vec<_> = ws.members.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);
    }

    #[test]
    fn discover_workspace_detects_internal_deps() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init_sampo(root);

        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

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
        assert!(x.internal_deps.contains("cargo/y"));
        assert!(x.internal_deps.contains("cargo/z"));
    }

    #[test]
    fn discover_workspace_returns_error_for_empty_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init_sampo(root);

        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();

        let result = discover_workspace(root);
        assert!(matches!(result, Err(WorkspaceError::NoPackagesFound)));
    }

    #[test]
    fn discover_workspace_returns_error_when_no_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init_sampo(root);

        // .sampo/ exists but no manifest files at all
        let result = discover_workspace(root);
        assert!(matches!(result, Err(WorkspaceError::NoPackagesFound)));
    }

    #[test]
    fn discover_workspace_from_package_subdirectory_finds_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init_sampo(root);

        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

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

        let pkg_a_dir = crates_dir.join("pkg-a");
        let ws = discover_workspace(&pkg_a_dir).unwrap();

        assert_eq!(ws.root, root);
        assert_eq!(ws.members.len(), 2);
        let mut names: Vec<_> = ws.members.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);
    }

    #[test]
    fn discover_workspace_from_intermediate_directory_finds_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init_sampo(root);

        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("pkg-a")).unwrap();
        fs::write(
            crates_dir.join("pkg-a/Cargo.toml"),
            "[package]\nname = \"pkg-a\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let ws = discover_workspace(&crates_dir).unwrap();

        assert_eq!(ws.root, root);
        assert_eq!(ws.members.len(), 1);
        assert_eq!(ws.members[0].name, "pkg-a");
    }

    #[test]
    fn discover_packages_at_finds_cargo_packages() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"standalone\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let packages = discover_packages_at(root).unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "standalone");
        assert_eq!(packages[0].kind, PackageKind::Cargo);
    }

    #[test]
    fn discover_packages_at_finds_npm_packages() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("package.json"),
            r#"{"name": "my-app", "version": "1.0.0", "workspaces": ["packages/*"]}"#,
        )
        .unwrap();

        let packages_dir = root.join("packages");
        fs::create_dir_all(packages_dir.join("pkg-a")).unwrap();
        fs::write(
            packages_dir.join("pkg-a/package.json"),
            r#"{"name": "pkg-a", "version": "0.1.0"}"#,
        )
        .unwrap();

        let packages = discover_packages_at(root).unwrap();
        assert_eq!(packages.len(), 2);
        assert!(packages.iter().any(|p| p.name == "my-app"));
        assert!(packages.iter().any(|p| p.name == "pkg-a"));
    }

    #[test]
    fn discover_packages_at_finds_hex_packages() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("mix.exs"),
            r#"
defmodule Example.MixProject do
  use Mix.Project
  def project do
    [app: :example, version: "0.1.0", deps: deps()]
  end
  defp deps, do: []
end
"#,
        )
        .unwrap();

        let packages = discover_packages_at(root).unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "example");
        assert_eq!(packages[0].kind, PackageKind::Hex);
    }

    #[test]
    fn discover_packages_at_finds_pypi_packages() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("pyproject.toml"),
            r#"
[project]
name = "my-python-pkg"
version = "1.0.0"
dependencies = []
"#,
        )
        .unwrap();

        let packages = discover_packages_at(root).unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "my-python-pkg");
        assert_eq!(packages[0].kind, PackageKind::PyPI);
    }

    #[test]
    fn discover_packages_at_returns_empty_when_no_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        let packages = discover_packages_at(root).unwrap();
        assert!(packages.is_empty());
    }
}
