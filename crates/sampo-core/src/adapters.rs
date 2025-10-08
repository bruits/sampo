/// Ecosystem-specific adapters (Cargo, npm, etc.) for all package operations.
pub mod cargo;

use crate::errors::{Result, WorkspaceError};
use crate::types::PackageInfo;
use std::path::Path;

/// Package ecosystem adapter (Cargo, npm, etc.).
#[derive(Debug, Clone, Copy)]
pub enum PackageAdapter {
    Cargo,
}

impl PackageAdapter {
    /// All registered adapters, checked in order during workspace discovery.
    /// TODO: it's fine for now, but eventually we could using strum or enum-iterators here.
    pub fn all() -> &'static [PackageAdapter] {
        &[PackageAdapter::Cargo]
    }

    /// Check if this adapter can handle the given directory.
    pub fn can_discover(&self, root: &Path) -> bool {
        match self {
            Self::Cargo => cargo::CargoAdapter.can_discover(root),
        }
    }

    /// Discover all packages in the workspace.
    pub fn discover(&self, root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        match self {
            Self::Cargo => cargo::CargoAdapter.discover(root),
        }
    }

    /// Get the path to the manifest file for a package directory.
    pub fn manifest_path(&self, package_dir: &Path) -> std::path::PathBuf {
        match self {
            Self::Cargo => cargo::CargoAdapter.manifest_path(package_dir),
        }
    }

    /// Check if a package is publishable to its primary registry.
    pub fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        match self {
            Self::Cargo => cargo::CargoAdapter.is_publishable(manifest_path),
        }
    }

    /// Check if a specific version already exists on the registry.
    pub fn version_exists(&self, package_name: &str, version: &str) -> Result<bool> {
        match self {
            Self::Cargo => cargo::CargoAdapter.version_exists(package_name, version),
        }
    }

    /// Execute the publish command for a package.
    pub fn publish(
        &self,
        manifest_path: &Path,
        dry_run: bool,
        extra_args: &[String],
    ) -> Result<()> {
        match self {
            Self::Cargo => cargo::CargoAdapter.publish(manifest_path, dry_run, extra_args),
        }
    }

    /// Regenerate the workspace lockfile after version updates.
    pub fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        match self {
            Self::Cargo => cargo::CargoAdapter.regenerate_lockfile(workspace_root),
        }
    }
}
