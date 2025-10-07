use crate::discovery::{CargoDiscovery, PackageDiscovery};
use crate::errors::WorkspaceError;
use crate::types::Workspace;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, WorkspaceError>;

/// Discover workspace packages using registered ecosystem discoverers
pub fn discover_workspace(start_dir: &Path) -> Result<Workspace> {
    // Registry of available discovery implementations
    // TODO: When adding new ecosystems, register them here
    let discoverers: Vec<Box<dyn PackageDiscovery>> = vec![Box::new(CargoDiscovery)];

    // Try each discoverer until one succeeds
    let mut root = None;
    let mut all_members = Vec::new();

    for discoverer in &discoverers {
        if discoverer.can_discover(start_dir) {
            // Find the workspace root by walking up from start_dir
            let discovered_root =
                find_workspace_root_for_discoverer(start_dir, discoverer.as_ref())?;

            // Discover packages in this ecosystem
            let packages = discoverer.discover(&discovered_root)?;

            // Use the first discovered root as the workspace root
            if root.is_none() {
                root = Some(discovered_root);
            }
            all_members.extend(packages);
        }
    }

    let workspace_root = root.ok_or(WorkspaceError::NotFound)?;

    Ok(Workspace {
        root: workspace_root,
        members: all_members,
    })
}

/// Find the workspace root for a given discoverer by walking up the directory tree
fn find_workspace_root_for_discoverer(
    start_dir: &Path,
    discoverer: &dyn PackageDiscovery,
) -> Result<PathBuf> {
    let mut current = start_dir;
    loop {
        if discoverer.can_discover(current) {
            return Ok(current.to_path_buf());
        }
        current = current.parent().ok_or(WorkspaceError::NotFound)?;
    }
}

/// Parse Cargo workspace members (delegates to CargoDiscovery)
pub fn parse_workspace_members(root: &Path, root_toml: &toml::Value) -> Result<Vec<PathBuf>> {
    let discovery = CargoDiscovery;
    discovery.parse_workspace_members(root, root_toml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PackageKind;
    use std::fs;

    #[test]
    fn discover_workspace_finds_cargo_packages() {
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

    #[test]
    fn parse_workspace_members_delegates_to_cargo_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\n",
        )
        .unwrap();

        // Create crates
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

        let root_toml: toml::Value = std::fs::read_to_string(root.join("Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();

        let members = parse_workspace_members(root, &root_toml).unwrap();
        let mut names: Vec<_> = members
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn discover_workspace_returns_only_cargo_packages() {
        // This test verifies that when a workspace is discovered, only Cargo packages
        // are returned (since that's currently the only supported ecosystem).
        // In the future, when more ecosystems are added, this test demonstrates that
        // the abstraction correctly aggregates packages from multiple discoverers.

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create a Cargo workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("cargo-pkg")).unwrap();
        fs::write(
            crates_dir.join("cargo-pkg/Cargo.toml"),
            "[package]\nname = \"cargo-pkg\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let ws = discover_workspace(root).unwrap();

        // Should discover the Cargo package
        assert_eq!(ws.members.len(), 1);
        assert_eq!(ws.members[0].name, "cargo-pkg");
        assert_eq!(ws.members[0].kind, PackageKind::Cargo);
    }

    #[test]
    fn discover_workspace_handles_empty_workspace() {
        // Test that an empty workspace (workspace defined but no packages) is valid
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();

        let ws = discover_workspace(root).unwrap();
        assert_eq!(ws.members.len(), 0);
        assert_eq!(ws.root, root);
    }

    #[test]
    fn discover_workspace_fails_when_no_workspace_found() {
        // Test that we get an error when there's no workspace at all
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // No Cargo.toml, no workspace
        let result = discover_workspace(root);
        assert!(result.is_err());

        // Verify it's the right error
        match result {
            Err(WorkspaceError::NotFound) => {}
            _ => panic!("Expected WorkspaceError::NotFound"),
        }
    }
}
