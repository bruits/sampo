/// Ecosystem-specific adapters (Cargo, npm, etc.) for all package operations.
pub mod cargo;
pub mod npm;

pub use cargo::ManifestMetadata;

use crate::errors::{Result, WorkspaceError};
use crate::types::PackageInfo;
use std::collections::BTreeMap;
use std::path::Path;

/// Package ecosystem adapter (Cargo, npm, etc.).
#[derive(Debug, Clone, Copy)]
pub enum PackageAdapter {
    Cargo,
    Npm,
}

impl PackageAdapter {
    /// All registered adapters, checked in order during workspace discovery.
    /// TODO: it's fine for now, but eventually we could using strum or enum-iterators here.
    pub fn all() -> &'static [PackageAdapter] {
        &[PackageAdapter::Cargo, PackageAdapter::Npm]
    }

    /// Check if this adapter can handle the given directory.
    pub fn can_discover(&self, root: &Path) -> bool {
        match self {
            Self::Cargo => cargo::CargoAdapter.can_discover(root),
            Self::Npm => npm::NpmAdapter.can_discover(root),
        }
    }

    /// Discover all packages in the workspace.
    pub fn discover(&self, root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        match self {
            Self::Cargo => cargo::CargoAdapter.discover(root),
            Self::Npm => npm::NpmAdapter.discover(root),
        }
    }

    /// Get the path to the manifest file for a package directory.
    pub fn manifest_path(&self, package_dir: &Path) -> std::path::PathBuf {
        match self {
            Self::Cargo => cargo::CargoAdapter.manifest_path(package_dir),
            Self::Npm => npm::NpmAdapter.manifest_path(package_dir),
        }
    }

    /// Check if a package is publishable to its primary registry.
    pub fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        match self {
            Self::Cargo => cargo::CargoAdapter.is_publishable(manifest_path),
            Self::Npm => npm::NpmAdapter.is_publishable(manifest_path),
        }
    }

    /// Check if a specific version already exists on the registry.
    pub fn version_exists(
        &self,
        package_name: &str,
        version: &str,
        manifest_path: Option<&Path>,
    ) -> Result<bool> {
        match self {
            Self::Cargo => cargo::CargoAdapter.version_exists(package_name, version),
            Self::Npm => npm::NpmAdapter.version_exists(package_name, version, manifest_path),
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
            Self::Npm => npm::NpmAdapter.publish(manifest_path, dry_run, extra_args),
        }
    }

    /// Regenerate the workspace lockfile after version updates.
    pub fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        match self {
            Self::Cargo => cargo::CargoAdapter.regenerate_lockfile(workspace_root),
            Self::Npm => npm::NpmAdapter.regenerate_lockfile(workspace_root),
        }
    }

    /// Update the manifest and dependency versions for a package.
    pub fn update_manifest_versions(
        &self,
        manifest_path: &Path,
        input: &str,
        new_pkg_version: Option<&str>,
        new_version_by_name: &BTreeMap<String, String>,
        metadata: Option<&ManifestMetadata>,
    ) -> Result<(String, Vec<(String, String)>)> {
        match self {
            Self::Cargo => cargo::update_manifest_versions(
                manifest_path,
                input,
                new_pkg_version,
                new_version_by_name,
                metadata,
            ),
            Self::Npm => {
                debug_assert!(
                    metadata.is_none(),
                    "npm adapter does not use Cargo metadata"
                );
                npm::update_manifest_versions(
                    manifest_path,
                    input,
                    new_pkg_version,
                    new_version_by_name,
                )
            }
        }
    }
}
