/// Cargo ecosystem adapter for all Cargo operations.
use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind, Workspace};
use cargo_metadata::{DependencyKind, MetadataCommand};
use rustc_hash::FxHashSet;
use semver::{Version, VersionReq};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

const CARGO_MANIFEST: &str = "Cargo.toml";

/// Stateless adapter for all Cargo operations (discovery, publish, registry, lockfile).
pub(super) struct CargoAdapter;

impl CargoAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        root.join("Cargo.toml").exists()
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        discover_cargo(root)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        package_dir.join("Cargo.toml")
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        is_publishable_to_crates_io(manifest_path)
    }

    pub(super) fn version_exists(&self, package_name: &str, version: &str) -> Result<bool> {
        version_exists_on_crates_io(package_name, version)
    }

    pub(super) fn publish(
        &self,
        manifest_path: &Path,
        dry_run: bool,
        extra_args: &[String],
    ) -> Result<()> {
        let mut cmd = Command::new("cargo");
        cmd.arg("publish").arg("--manifest-path").arg(manifest_path);

        if dry_run {
            cmd.arg("--dry-run");
        }

        if !extra_args.is_empty() {
            cmd.args(extra_args);
        }

        println!(
            "Running: {}",
            format_command_display(cmd.get_program(), cmd.get_args())
        );

        let status = cmd.status()?;
        if !status.success() {
            return Err(SampoError::Publish(format!(
                "cargo publish failed for {} with status {}",
                manifest_path.display(),
                status
            )));
        }

        Ok(())
    }

    pub(super) fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        regenerate_cargo_lockfile(workspace_root)
    }
}

/// Check if a Cargo dependency version constraint is satisfied by a new version.
pub(super) fn check_dependency_constraint(
    _dep_name: &str,
    current_constraint: &str,
    new_version: &str,
) -> Result<crate::types::ConstraintCheckResult> {
    use crate::types::ConstraintCheckResult;

    let constraint = current_constraint.trim();
    let version_str = new_version.trim();

    let req = match VersionReq::parse(constraint) {
        Ok(r) => r,
        Err(_) => {
            return Ok(ConstraintCheckResult::Skipped {
                reason: format!("unparseable constraint '{}'", constraint),
            });
        }
    };

    let version = match Version::parse(version_str) {
        Ok(v) => v,
        Err(_) => {
            return Ok(ConstraintCheckResult::Skipped {
                reason: format!("unparseable version '{}'", version_str),
            });
        }
    };

    if req.matches(&version) {
        Ok(ConstraintCheckResult::Satisfied)
    } else {
        Ok(ConstraintCheckResult::NotSatisfied {
            constraint: constraint.to_string(),
            new_version: version_str.to_string(),
        })
    }
}

/// Detect the version of the `cargo` binary available on the PATH.
pub fn detect_version() -> Result<Option<Version>> {
    let output = match Command::new("cargo").arg("--version").output() {
        Ok(value) => value,
        Err(err) => return Err(SampoError::Io(err)),
    };

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut parts = stdout.split_whitespace();
    // First token is the binary name, second token is the version string.
    let _ = parts.next();
    let version_str = match parts.next() {
        Some(value) => value,
        None => return Ok(None),
    };

    match Version::parse(version_str) {
        Ok(version) => Ok(Some(version)),
        Err(_) => Ok(None),
    }
}

/// Run `cargo publish --workspace --dry-run` for a subset of workspace members.
pub fn workspace_publish_dry_run(
    workspace_root: &Path,
    packages: &[&str],
    extra_args: &[String],
) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }

    let manifest_path = workspace_root.join("Cargo.toml");
    let mut cmd = Command::new("cargo");
    cmd.arg("publish")
        .arg("--workspace")
        .arg("--dry-run")
        .arg("--manifest-path")
        .arg(&manifest_path);

    for pkg in packages {
        cmd.arg("--package").arg(pkg);
    }

    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }

    println!(
        "Running: {}",
        format_command_display(cmd.get_program(), cmd.get_args())
    );

    let status = cmd.status()?;
    if !status.success() {
        return Err(SampoError::Publish(format!(
            "cargo publish --workspace --dry-run failed with status {}",
            status
        )));
    }

    Ok(())
}

pub(super) fn publish_dry_run(
    workspace_root: &Path,
    packages: &[(&PackageInfo, &Path)],
    extra_args: &[String],
) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }

    let has_dependent_packages = packages
        .iter()
        .any(|(package, _)| !package.internal_deps.is_empty());

    let mut skip_dependent_packages = false;

    match detect_version() {
        Ok(Some(version)) => {
            let min_supported = Version::new(1, 91, 0);
            if version >= min_supported {
                let package_names: Vec<&str> = packages
                    .iter()
                    .map(|(package, _)| package.name.as_str())
                    .collect();
                return workspace_publish_dry_run(workspace_root, &package_names, extra_args)
                    .map_err(|err| match err {
                        SampoError::Publish(message) => SampoError::Publish(format!(
                            "Cargo workspace dry-run failed: {}",
                            message
                        )),
                        other => other,
                    });
            }

            if has_dependent_packages {
                skip_dependent_packages = true;
                eprintln!(
                    "Warning: Cargo {version} does not support workspace dry-run publish; skipping dry-run for crates that depend on internal workspace packages."
                );
            }
        }
        Ok(None) => {
            if has_dependent_packages {
                skip_dependent_packages = true;
                eprintln!(
                    "Warning: could not determine Cargo version. Skipping dry-run for crates that depend on internal workspace packages."
                );
            }
        }
        Err(err) => {
            if has_dependent_packages {
                skip_dependent_packages = true;
                eprintln!(
                    "Warning: failed to determine Cargo version: {err}. Skipping dry-run for crates that depend on internal workspace packages."
                );
            }
        }
    }

    for (package, manifest) in packages {
        if skip_dependent_packages && !package.internal_deps.is_empty() {
            println!(
                "  - Skipping dry-run for {} (requires workspace-aware Cargo to validate dependencies)",
                package.display_name(true)
            );
            continue;
        }

        run_cargo_dry_run(package, manifest, extra_args)?;
    }

    Ok(())
}

fn run_cargo_dry_run(
    package: &PackageInfo,
    manifest_path: &Path,
    extra_args: &[String],
) -> Result<()> {
    CargoAdapter
        .publish(manifest_path, true, extra_args)
        .map_err(|err| match err {
            SampoError::Publish(message) => SampoError::Publish(format!(
                "Dry-run publish failed for {}: {}",
                package.display_name(true),
                message
            )),
            other => other,
        })
}

/// Check Cargo.toml `publish` field per Cargo rules (false, array of registries, or default true).
fn is_publishable_to_crates_io(manifest_path: &Path) -> Result<bool> {
    let text = fs::read_to_string(manifest_path)
        .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
    let value: toml::Value = text.parse().map_err(|e| {
        SampoError::InvalidData(format!("invalid TOML in {}: {e}", manifest_path.display()))
    })?;

    let pkg = match value.get("package").and_then(|v| v.as_table()) {
        Some(p) => p,
        None => return Ok(false),
    };

    // If publish = false => skip
    if let Some(val) = pkg.get("publish") {
        match val {
            toml::Value::Boolean(false) => return Ok(false),
            toml::Value::Array(arr) => {
                // Only publish if the array contains "crates-io"
                // (Cargo uses this to whitelist registries.)
                let allowed: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                return Ok(allowed.iter().any(|s| s == "crates-io"));
            }
            _ => {}
        }
    }

    // Default case: publishable
    Ok(true)
}

/// Query crates.io API to check if a specific version already exists.
fn version_exists_on_crates_io(crate_name: &str, version: &str) -> Result<bool> {
    // Query crates.io: https://crates.io/api/v1/crates/<name>/<version>
    let url = format!("https://crates.io/api/v1/crates/{}/{}", crate_name, version);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("sampo-core/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| SampoError::Publish(format!("failed to build HTTP client: {}", e)))?;

    let res = client
        .get(&url)
        .send()
        .map_err(|e| SampoError::Publish(format!("HTTP request failed: {}", e)))?;

    let status = res.status();
    if status == reqwest::StatusCode::OK {
        Ok(true)
    } else if status == reqwest::StatusCode::NOT_FOUND {
        Ok(false)
    } else {
        // Include a short, normalized snippet of the response body for diagnostics
        let body = res.text().unwrap_or_default();
        let snippet: String = body.trim().chars().take(500).collect();
        let snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");

        let body_part = if snippet.is_empty() {
            String::new()
        } else {
            format!(" body=\"{}\"", snippet)
        };

        Err(SampoError::Publish(format!(
            "Crates.io {} response:{}",
            status, body_part
        )))
    }
}

/// Run `cargo generate-lockfile` to rebuild the lockfile with updated versions.
fn regenerate_cargo_lockfile(root: &Path) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("generate-lockfile").current_dir(root);

    println!("Regenerating Cargo.lock…");
    let status = cmd.status().map_err(SampoError::Io)?;
    if !status.success() {
        return Err(SampoError::Release(format!(
            "cargo generate-lockfile failed with status {}",
            status
        )));
    }
    println!("Cargo.lock updated.");
    Ok(())
}

fn format_command_display(program: &std::ffi::OsStr, args: std::process::CommandArgs) -> String {
    let prog = program.to_string_lossy();
    let mut s = String::new();
    s.push_str(&prog);
    for a in args {
        s.push(' ');
        s.push_str(&a.to_string_lossy());
    }
    s
}

/// Metadata extracted from `cargo_metadata` for the workspace.
pub struct ManifestMetadata {
    packages: Vec<MetadataPackage>,
    by_manifest: HashMap<PathBuf, usize>,
    by_name: HashMap<String, usize>,
}

struct MetadataPackage {
    dependencies: Vec<MetadataDependency>,
}

struct MetadataDependency {
    manifest_key: String,
    package_name: String,
    kind: DependencyKind,
    target: Option<String>,
    version_req: String,
}

impl ManifestMetadata {
    pub fn load(workspace: &Workspace) -> Result<Self> {
        let manifest_path = workspace.root.join("Cargo.toml");
        let metadata = MetadataCommand::new()
            .manifest_path(&manifest_path)
            .no_deps()
            .exec()
            .map_err(|err| {
                SampoError::Release(format!(
                    "Failed to load cargo metadata for {}: {err}",
                    manifest_path.display()
                ))
            })?;

        let workspace_ids: FxHashSet<_> = metadata.workspace_members.iter().cloned().collect();

        let mut packages = Vec::new();
        let mut by_manifest = HashMap::new();
        let mut by_name = HashMap::new();

        for package in metadata.packages {
            if !workspace_ids.contains(&package.id) {
                continue;
            }

            let manifest_path: PathBuf = package.manifest_path.clone().into();
            let dependencies = package
                .dependencies
                .iter()
                .map(|dep| MetadataDependency {
                    manifest_key: dep.rename.clone().unwrap_or_else(|| dep.name.clone()),
                    package_name: dep.name.clone(),
                    kind: dep.kind,
                    target: dep.target.as_ref().map(|platform| platform.to_string()),
                    version_req: dep.req.to_string(),
                })
                .collect();

            let idx = packages.len();
            by_manifest.insert(manifest_path.clone(), idx);
            by_name.insert(package.name.clone(), idx);
            packages.push(MetadataPackage { dependencies });
        }

        Ok(Self {
            packages,
            by_manifest,
            by_name,
        })
    }

    fn package_for_manifest(&self, manifest_path: &Path) -> Option<&MetadataPackage> {
        self.by_manifest
            .get(manifest_path)
            .and_then(|idx| self.packages.get(*idx))
    }

    fn is_workspace_package(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Returns the version constraint for a dependency of a package, if found.
    /// The `dep_name` is the package name (not the alias/manifest key).
    pub fn get_dependency_constraint(
        &self,
        manifest_path: &Path,
        dep_name: &str,
    ) -> Option<String> {
        let package = self.package_for_manifest(manifest_path)?;
        package
            .dependencies
            .iter()
            .find(|dep| dep.package_name == dep_name)
            .map(|dep| dep.version_req.clone())
    }

    /// Check if a dependency's version in the raw TOML is a pinned exact version.
    /// Pinned versions (bare semver like `"1.0.0"`) will be updated during manifest updates,
    /// while range constraints (e.g., `"^1.0"`) are preserved and need constraint validation.
    pub fn is_dependency_pinned(&self, manifest_path: &Path, dep_name: &str) -> bool {
        let manifest_key = self
            .package_for_manifest(manifest_path)
            .and_then(|pkg| {
                pkg.dependencies
                    .iter()
                    .find(|d| d.package_name == dep_name)
                    .map(|d| d.manifest_key.as_str())
            })
            .unwrap_or(dep_name);

        let content = match fs::read_to_string(manifest_path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let doc: DocumentMut = match content.parse() {
            Ok(d) => d,
            Err(_) => return false,
        };

        let top = doc.as_table();
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(raw) = raw_dep_version(top, section, manifest_key) {
                return Version::parse(raw.trim()).is_ok();
            }
        }

        if let Some(targets) = top.get("target").and_then(Item::as_table) {
            for (_, target_item) in targets.iter() {
                if let Some(target_table) = target_item.as_table() {
                    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                        if let Some(raw) = raw_dep_version(target_table, section, manifest_key) {
                            return Version::parse(raw.trim()).is_ok();
                        }
                    }
                }
            }
        }

        false
    }
}

/// Update a Cargo manifest by setting the package version (if provided) and retargeting internal
/// dependency requirements to the latest planned versions.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
    metadata: Option<&ManifestMetadata>,
) -> Result<(String, Vec<(String, String)>)> {
    let mut doc: DocumentMut = input.parse().map_err(|err| {
        SampoError::Release(format!(
            "Failed to parse manifest {}: {err}",
            manifest_path.display()
        ))
    })?;

    if let Some(version) = new_pkg_version {
        update_package_version(&mut doc, manifest_path, version)?;
    }

    let mut applied = Vec::new();
    let package_info = metadata.and_then(|data| data.package_for_manifest(manifest_path));

    for (dep_name, new_version) in new_version_by_name {
        if let Some(meta) = metadata
            && !meta.is_workspace_package(dep_name)
        {
            continue;
        }

        let mut changed = false;

        if let Some(package) = package_info {
            changed |= update_dependencies_from_metadata(&mut doc, package, dep_name, new_version);
        }

        let workspace_changed = update_workspace_dependency(&mut doc, dep_name, new_version);
        changed |= workspace_changed;

        if !changed {
            changed |= update_dependencies_fallback(&mut doc, dep_name, new_version);
        }

        if changed {
            applied.push((dep_name.clone(), new_version.clone()));
        }
    }

    Ok((doc.to_string(), applied))
}

fn update_package_version(
    doc: &mut DocumentMut,
    manifest_path: &Path,
    new_version: &str,
) -> Result<()> {
    let package_table = doc
        .as_table_mut()
        .get_mut("package")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            SampoError::Release(format!(
                "Manifest {} is missing a [package] section",
                manifest_path.display()
            ))
        })?;

    let current = package_table
        .get("version")
        .and_then(Item::as_value)
        .and_then(Value::as_str);

    if current == Some(new_version) {
        return Ok(());
    }

    package_table.insert("version", Item::Value(Value::from(new_version)));
    Ok(())
}

fn update_dependencies_from_metadata(
    doc: &mut DocumentMut,
    package: &MetadataPackage,
    dep_name: &str,
    new_version: &str,
) -> bool {
    let mut changed = false;

    for dependency in &package.dependencies {
        if dependency.package_name != dep_name {
            continue;
        }

        if let Some(table) =
            dependency_table_mut(doc, dependency.target.as_deref(), dependency.kind)
            && let Some(item) = table.get_mut(&dependency.manifest_key)
        {
            changed |= update_standard_dependency_item(item, new_version);
        }
    }

    changed
}

fn dependency_table_mut<'a>(
    doc: &'a mut DocumentMut,
    target: Option<&str>,
    kind: DependencyKind,
) -> Option<&'a mut Table> {
    let section = dependency_section_name(kind);

    match target {
        None => doc.get_mut(section).and_then(Item::as_table_mut),
        Some(target_spec) => doc
            .get_mut("target")
            .and_then(Item::as_table_mut)?
            .get_mut(target_spec)
            .and_then(Item::as_table_mut)?
            .get_mut(section)
            .and_then(Item::as_table_mut),
    }
}

/// Extract the raw version string for a dependency from a TOML table section.
fn raw_dep_version<'a>(parent: &'a Table, section: &str, dep_name: &str) -> Option<&'a str> {
    let dep_table = parent.get(section)?.as_table()?;
    let item = dep_table.get(dep_name)?;
    match item {
        Item::Value(Value::String(s)) => Some(s.value()),
        Item::Value(Value::InlineTable(t)) => t.get("version").and_then(Value::as_str),
        Item::Table(t) => t
            .get("version")
            .and_then(Item::as_value)
            .and_then(Value::as_str),
        _ => None,
    }
}

fn dependency_section_name(kind: DependencyKind) -> &'static str {
    match kind {
        DependencyKind::Normal | DependencyKind::Unknown => "dependencies",
        DependencyKind::Development => "dev-dependencies",
        DependencyKind::Build => "build-dependencies",
    }
}

/// Returns true if the existing dependency version should be replaced with the exact new version.
/// Pinned versions (bare semver) are always replaced; range constraints are preserved.
fn should_update_dependency_version(existing: &str, new_version: &str) -> bool {
    let trimmed = existing.trim();

    if Version::parse(trimmed).is_ok() {
        return trimmed != new_version;
    }

    // Deferred to constraint validation in release.rs
    false
}

fn update_standard_dependency_item(item: &mut Item, new_version: &str) -> bool {
    match item {
        Item::Value(Value::InlineTable(table)) => update_inline_dependency(table, new_version),
        Item::Table(table) => update_table_dependency(table, new_version),
        Item::Value(value) => {
            let current = value.as_str().unwrap_or_default();
            if !should_update_dependency_version(current, new_version) {
                return false;
            }
            *item = Item::Value(Value::from(new_version));
            true
        }
        _ => false,
    }
}

fn update_inline_dependency(table: &mut InlineTable, new_version: &str) -> bool {
    if table
        .get("workspace")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }

    let current = table.get("version").and_then(Value::as_str);
    let needs_update = match current {
        Some(existing) => should_update_dependency_version(existing, new_version),
        // No version field present — don't insert one
        None => false,
    };

    if needs_update {
        table.insert("version", Value::from(new_version));
    }

    needs_update
}

fn update_table_dependency(table: &mut Table, new_version: &str) -> bool {
    if table
        .get("workspace")
        .and_then(Item::as_value)
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }

    let current = table
        .get("version")
        .and_then(Item::as_value)
        .and_then(Value::as_str);
    let needs_update = match current {
        Some(existing) => should_update_dependency_version(existing, new_version),
        // No version field present — don't insert one
        None => false,
    };

    if needs_update {
        table.insert("version", Item::Value(Value::from(new_version)));
    }

    needs_update
}

fn update_dependencies_fallback(doc: &mut DocumentMut, dep_name: &str, new_version: &str) -> bool {
    let mut changed = false;
    let top_level = doc.as_table_mut();

    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = top_level.get_mut(section).and_then(Item::as_table_mut)
            && let Some(item) = table.get_mut(dep_name)
        {
            changed |= update_standard_dependency_item(item, new_version);
        }
    }

    if let Some(targets) = top_level.get_mut("target").and_then(Item::as_table_mut) {
        for (_, target_item) in targets.iter_mut() {
            if let Some(target_table) = target_item.as_table_mut() {
                for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(table) = target_table.get_mut(section).and_then(Item::as_table_mut)
                        && let Some(item) = table.get_mut(dep_name)
                    {
                        changed |= update_standard_dependency_item(item, new_version);
                    }
                }
            }
        }
    }

    changed
}

fn update_workspace_dependency(doc: &mut DocumentMut, dep_name: &str, new_version: &str) -> bool {
    let workspace_table = match doc
        .as_table_mut()
        .get_mut("workspace")
        .and_then(Item::as_table_mut)
    {
        Some(table) => table,
        None => return false,
    };

    let deps_item = match workspace_table.get_mut("dependencies") {
        Some(item) => item,
        None => return false,
    };

    match deps_item {
        Item::Table(table) => {
            if let Some(item) = table.get_mut(dep_name) {
                update_workspace_dependency_item(item, new_version)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn update_workspace_dependency_item(item: &mut Item, new_version: &str) -> bool {
    match item {
        Item::Value(Value::InlineTable(table)) => {
            let current = table.get("version").and_then(Value::as_str);
            let Some(existing) = current else {
                return false;
            };

            match compute_workspace_dependency_version(existing, new_version) {
                Some(resolved) if resolved != existing => {
                    table.insert("version", Value::from(resolved));
                    true
                }
                _ => false,
            }
        }
        Item::Table(table) => {
            let current = table
                .get("version")
                .and_then(Item::as_value)
                .and_then(Value::as_str);
            let Some(existing) = current else {
                return false;
            };

            match compute_workspace_dependency_version(existing, new_version) {
                Some(resolved) if resolved != existing => {
                    table.insert("version", Item::Value(Value::from(resolved)));
                    true
                }
                _ => false,
            }
        }
        Item::Value(value) => {
            let Some(existing) = value.as_str() else {
                return false;
            };

            match compute_workspace_dependency_version(existing, new_version) {
                Some(resolved) if resolved != existing => {
                    *item = Item::Value(Value::from(resolved));
                    true
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn compute_workspace_dependency_version(existing: &str, new_version: &str) -> Option<String> {
    let trimmed_existing = existing.trim();
    if trimmed_existing == "*" {
        return None;
    }

    if Version::parse(trimmed_existing).is_ok() {
        if trimmed_existing == new_version {
            return None;
        }
        return Some(new_version.to_string());
    }

    let shorthand = parse_numeric_shorthand(trimmed_existing)?;
    VersionReq::parse(trimmed_existing).ok()?;
    let parsed_new = Version::parse(new_version).ok()?;

    let resolved = match shorthand.len() {
        1 => parsed_new.major.to_string(),
        2 => format!("{}.{}", parsed_new.major, parsed_new.minor),
        _ => return None,
    };

    if resolved == trimmed_existing {
        None
    } else {
        Some(resolved)
    }
}

fn parse_numeric_shorthand(value: &str) -> Option<Vec<u64>> {
    let segments: Vec<&str> = value.split('.').collect();
    if segments.is_empty() || segments.len() > 2 {
        return None;
    }

    let mut numeric_segments = Vec::with_capacity(segments.len());
    for segment in segments {
        if segment.is_empty() || !segment.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        let parsed = segment.parse::<u64>().ok()?;
        numeric_segments.push(parsed);
    }

    Some(numeric_segments)
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

/// Parse workspace members from the root Cargo.toml
fn parse_cargo_workspace_members(
    root: &Path,
    root_toml: &toml::Value,
) -> std::result::Result<Vec<PathBuf>, WorkspaceError> {
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
        expand_cargo_member_pattern(root, pattern, &mut paths)?;
    }

    Ok(paths)
}

/// Expand a member pattern (plain path or glob) into concrete paths
fn expand_cargo_member_pattern(
    root: &Path,
    pattern: &str,
    paths: &mut Vec<PathBuf>,
) -> std::result::Result<(), WorkspaceError> {
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

/// Collect internal dependencies for a crate
fn collect_cargo_internal_deps(
    crate_dir: &Path,
    name_to_path: &BTreeMap<String, PathBuf>,
    manifest: &toml::Value,
) -> BTreeSet<String> {
    let mut internal = BTreeSet::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(tbl) = manifest.get(key).and_then(|v| v.as_table()) {
            for (dep_name, dep_val) in tbl {
                if is_cargo_internal_dep(crate_dir, name_to_path, dep_name, dep_val) {
                    internal.insert(PackageInfo::dependency_identifier(
                        PackageKind::Cargo,
                        dep_name,
                    ));
                }
            }
        }
    }
    internal
}

/// Check if a dependency is internal to the workspace
fn is_cargo_internal_dep(
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

fn resolve_package_version(
    pkg: &toml::map::Map<String, toml::Value>,
    workspace_version: &Option<String>,
    manifest_path: &Path,
) -> std::result::Result<String, WorkspaceError> {
    match pkg.get("version") {
        Some(toml::Value::String(v)) => Ok(v.clone()),
        Some(toml::Value::Table(t)) => {
            if t.get("workspace")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                workspace_version.clone().ok_or_else(|| {
                    WorkspaceError::InvalidManifest(format!(
                        "{}: version.workspace = true requires workspace.package.version",
                        manifest_path.display()
                    ))
                })
            } else {
                Ok(String::new())
            }
        }
        _ => Ok(String::new()),
    }
}

fn discover_cargo(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    let cargo_toml_path = root.join(CARGO_MANIFEST);
    if !cargo_toml_path.exists() {
        return Err(WorkspaceError::ManifestNotFound {
            manifest: CARGO_MANIFEST,
            path: root.to_path_buf(),
        });
    }

    let text = fs::read_to_string(&cargo_toml_path)
        .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, &cargo_toml_path)))?;
    let root_toml: toml::Value = text.parse().map_err(|e| {
        WorkspaceError::InvalidManifest(format!("{}: {}", cargo_toml_path.display(), e))
    })?;

    let workspace_version = root_toml
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let member_dirs = if root_toml.get("workspace").is_some() {
        parse_cargo_workspace_members(root, &root_toml)?
    } else {
        vec![root.to_path_buf()]
    };

    let mut crates = Vec::new();
    let mut name_to_path: BTreeMap<String, PathBuf> = BTreeMap::new();

    for member_dir in &member_dirs {
        let manifest_path = member_dir.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path).map_err(|e| {
            WorkspaceError::Io(crate::errors::io_error_with_path(e, &manifest_path))
        })?;
        let value: toml::Value = text.parse().map_err(|e| {
            WorkspaceError::InvalidManifest(format!("{}: {}", manifest_path.display(), e))
        })?;
        let pkg = value
            .get("package")
            .and_then(|v| v.as_table())
            .ok_or_else(|| {
                WorkspaceError::InvalidManifest(format!(
                    "missing [package] in {}",
                    manifest_path.display()
                ))
            })?;
        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                WorkspaceError::InvalidManifest(format!(
                    "missing package.name in {}",
                    manifest_path.display()
                ))
            })?
            .to_string();
        let version = resolve_package_version(pkg, &workspace_version, &manifest_path)?;
        name_to_path.insert(name.clone(), member_dir.clone());
        crates.push((name, version, member_dir.clone(), value));
    }

    let mut out: Vec<PackageInfo> = Vec::new();
    for (name, version, path, manifest) in crates {
        let identifier = PackageInfo::dependency_identifier(PackageKind::Cargo, &name);
        let internal_deps = collect_cargo_internal_deps(&path, &name_to_path, &manifest);
        out.push(PackageInfo {
            name,
            identifier,
            version,
            path,
            internal_deps,
            kind: PackageKind::Cargo,
        });
    }

    Ok(out)
}

#[cfg(test)]
mod cargo_tests;
