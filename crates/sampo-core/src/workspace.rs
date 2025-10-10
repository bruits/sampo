use crate::adapters::PackageAdapter;
use crate::errors::WorkspaceError;
use crate::types::Workspace;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, WorkspaceError>;

/// Discover workspace packages using registered ecosystem adapters.
pub fn discover_workspace(start_dir: &Path) -> Result<Workspace> {
    let mut root = None;
    let mut all_members = Vec::new();

    // Try each registered adapter (static dispatch, zero-cost)
    for adapter in PackageAdapter::all() {
        if adapter.can_discover(start_dir) {
            // Find the workspace root by walking up from start_dir
            let discovered_root = find_workspace_root_for_adapter(start_dir, *adapter)?;

            // Discover packages in this ecosystem
            let packages = adapter.discover(&discovered_root)?;

            // Use the first discovered root as the workspace root
            root.get_or_insert(discovered_root);
            all_members.extend(packages);
        }
    }

    let workspace_root = root.ok_or(WorkspaceError::NotFound)?;

    Ok(Workspace {
        root: workspace_root,
        members: all_members,
    })
}

/// Find the workspace root for a given adapter by walking up the directory tree.
fn find_workspace_root_for_adapter(start_dir: &Path, adapter: PackageAdapter) -> Result<PathBuf> {
    let mut current = start_dir;
    loop {
        if adapter.can_discover(current) {
            return Ok(current.to_path_buf());
        }
        current = current.parent().ok_or(WorkspaceError::NotFound)?;
    }
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
        assert!(x.internal_deps.contains("cargo:y"));
        assert!(x.internal_deps.contains("cargo:z"));
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
