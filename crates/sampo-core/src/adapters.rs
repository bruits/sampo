/// Ecosystem-specific adapters (Cargo, npm, etc.) for all package operations.
pub mod cargo;
pub mod hex;
pub mod npm;
pub mod packagist;
pub mod pypi;

use crate::errors::{Result, WorkspaceError};
use crate::types::{PackageInfo, PackageKind, Workspace};
use std::collections::BTreeMap;
use std::path::Path;

/// Package ecosystem adapter (Cargo, npm, etc.).
#[derive(Debug, Clone, Copy)]
pub enum PackageAdapter {
    Cargo,
    Npm,
    Hex,
    PyPI,
    Packagist,
}

impl PackageAdapter {
    /// All registered adapters, checked in order during workspace discovery.
    /// TODO: it's fine for now, but eventually we could using strum or enum-iterators here.
    pub fn all() -> &'static [PackageAdapter] {
        &[
            PackageAdapter::Cargo,
            PackageAdapter::Npm,
            PackageAdapter::Hex,
            PackageAdapter::PyPI,
            PackageAdapter::Packagist,
        ]
    }

    /// Check if this adapter can handle the given directory.
    pub fn can_discover(&self, root: &Path) -> bool {
        match self {
            Self::Cargo => cargo::CargoAdapter.can_discover(root),
            Self::Npm => npm::NpmAdapter.can_discover(root),
            Self::Hex => hex::HexAdapter.can_discover(root),
            Self::PyPI => pypi::PyPIAdapter.can_discover(root),
            Self::Packagist => packagist::PackagistAdapter.can_discover(root),
        }
    }

    /// Discover all packages in the workspace.
    pub fn discover(&self, root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        match self {
            Self::Cargo => cargo::CargoAdapter.discover(root),
            Self::Npm => npm::NpmAdapter.discover(root),
            Self::Hex => hex::HexAdapter.discover(root),
            Self::PyPI => pypi::PyPIAdapter.discover(root),
            Self::Packagist => packagist::PackagistAdapter.discover(root),
        }
    }

    /// Get the path to the manifest file for a package directory.
    pub fn manifest_path(&self, package_dir: &Path) -> std::path::PathBuf {
        match self {
            Self::Cargo => cargo::CargoAdapter.manifest_path(package_dir),
            Self::Npm => npm::NpmAdapter.manifest_path(package_dir),
            Self::Hex => hex::HexAdapter.manifest_path(package_dir),
            Self::PyPI => pypi::PyPIAdapter.manifest_path(package_dir),
            Self::Packagist => packagist::PackagistAdapter.manifest_path(package_dir),
        }
    }

    /// Check if a package is publishable to its primary registry.
    pub fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        match self {
            Self::Cargo => cargo::CargoAdapter.is_publishable(manifest_path),
            Self::Npm => npm::NpmAdapter.is_publishable(manifest_path),
            Self::Hex => hex::HexAdapter.is_publishable(manifest_path),
            Self::PyPI => pypi::PyPIAdapter.is_publishable(manifest_path),
            Self::Packagist => packagist::PackagistAdapter.is_publishable(manifest_path),
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
            Self::Hex => hex::HexAdapter.version_exists(package_name, version, manifest_path),
            Self::PyPI => pypi::PyPIAdapter.version_exists(package_name, version, manifest_path),
            Self::Packagist => {
                packagist::PackagistAdapter.version_exists(package_name, version, manifest_path)
            }
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
            Self::Hex => hex::HexAdapter.publish(manifest_path, dry_run, extra_args),
            Self::PyPI => pypi::PyPIAdapter.publish(manifest_path, dry_run, extra_args),
            Self::Packagist => {
                packagist::PackagistAdapter.publish(manifest_path, dry_run, extra_args)
            }
        }
    }

    /// Execute dry-run publish validation for the provided packages.
    /// Adapters can choose the most appropriate strategy (workspace-level or per-package).
    pub fn publish_dry_run(
        &self,
        workspace_root: &Path,
        packages: &[(&PackageInfo, &Path)],
        extra_args: &[String],
    ) -> Result<()> {
        match self {
            Self::Cargo => cargo::publish_dry_run(workspace_root, packages, extra_args),
            Self::Npm => npm::publish_dry_run(packages, extra_args),
            Self::Hex => hex::publish_dry_run(packages, extra_args),
            Self::PyPI => pypi::publish_dry_run(packages, extra_args),
            Self::Packagist => packagist::publish_dry_run(packages, extra_args),
        }
    }

    /// Regenerate the workspace lockfile after version updates.
    pub fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        match self {
            Self::Cargo => cargo::CargoAdapter.regenerate_lockfile(workspace_root),
            Self::Npm => npm::NpmAdapter.regenerate_lockfile(workspace_root),
            Self::Hex => hex::HexAdapter.regenerate_lockfile(workspace_root),
            Self::PyPI => pypi::PyPIAdapter.regenerate_lockfile(workspace_root),
            Self::Packagist => packagist::PackagistAdapter.regenerate_lockfile(workspace_root),
        }
    }

    /// Update the manifest and dependency versions for a package.
    pub fn update_manifest_versions(
        &self,
        manifest_path: &Path,
        input: &str,
        new_pkg_version: Option<&str>,
        new_version_by_name: &BTreeMap<String, String>,
    ) -> Result<(String, Vec<(String, String)>)> {
        match self {
            Self::Cargo => cargo::update_manifest_versions(
                manifest_path,
                input,
                new_pkg_version,
                new_version_by_name,
            ),
            Self::Npm => npm::update_manifest_versions(
                manifest_path,
                input,
                new_pkg_version,
                new_version_by_name,
            ),
            Self::Hex => hex::update_manifest_versions(
                manifest_path,
                input,
                new_pkg_version,
                new_version_by_name,
            ),
            Self::PyPI => pypi::update_manifest_versions(
                manifest_path,
                input,
                new_pkg_version,
                new_version_by_name,
            ),
            Self::Packagist => packagist::update_manifest_versions(
                manifest_path,
                input,
                new_pkg_version,
                new_version_by_name,
            ),
        }
    }

    /// Finalize workspace root manifests after all member manifests have been updated.
    ///
    /// For each ecosystem that uses workspace version inheritance (currently Cargo), this
    /// detects which members inherit their version, validates they resolve to the same version,
    /// and updates the workspace root manifest accordingly.
    pub fn finalize_workspace_roots(
        workspace: &Workspace,
        new_version_by_name: &BTreeMap<String, String>,
    ) -> Result<()> {
        let has_cargo = workspace
            .members
            .iter()
            .any(|pkg| pkg.kind == PackageKind::Cargo);

        if has_cargo {
            cargo::finalize_workspace_root(
                &workspace.root,
                &workspace.members,
                new_version_by_name,
            )?;
        }

        Ok(())
    }

    /// Adapter helper for matching from a PackageKind.
    pub fn from_kind(kind: PackageKind) -> Self {
        match kind {
            PackageKind::Cargo => Self::Cargo,
            PackageKind::Npm => Self::Npm,
            PackageKind::Hex => Self::Hex,
            PackageKind::PyPI => Self::PyPI,
            PackageKind::Packagist => Self::Packagist,
        }
    }
}
