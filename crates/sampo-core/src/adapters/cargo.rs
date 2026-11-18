/// Cargo ecosystem adapter for all Cargo operations.
use crate::adapters::{format_command_display, has_flag};
use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{ConstraintCheckResult, PackageInfo, PackageKind, Workspace};
use cargo_metadata::{Dependency, DependencyKind, MetadataCommand};
use rustc_hash::FxHashSet;
use semver::{Version, VersionReq};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use toml_edit::{DocumentMut, Item, TableLike, Value};

const CARGO_MANIFEST: &str = "Cargo.toml";
const DEPENDENCY_SECTIONS: [(&str, DependencyKind); 3] = [
    ("dependencies", DependencyKind::Normal),
    ("dev-dependencies", DependencyKind::Development),
    ("build-dependencies", DependencyKind::Build),
];

#[cfg(test)]
thread_local! {
    static CRATES_IO_API_BASE_OVERRIDE: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct CratesIoApiBaseOverrideGuard {
    previous: Option<String>,
}

/// Point the crates.io existence check at a local stand-in: the real API rate-limits
/// busy IPs, which made network-dependent tests flaky.
#[cfg(test)]
pub(crate) fn override_crates_io_api_base_for_tests(base: &str) -> CratesIoApiBaseOverrideGuard {
    let previous =
        CRATES_IO_API_BASE_OVERRIDE.with(|cell| cell.borrow_mut().replace(base.to_string()));
    CratesIoApiBaseOverrideGuard { previous }
}

#[cfg(test)]
impl Drop for CratesIoApiBaseOverrideGuard {
    fn drop(&mut self) {
        CRATES_IO_API_BASE_OVERRIDE.with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some(prev) = self.previous.take() {
                slot.replace(prev);
            } else {
                slot.take();
            }
        });
    }
}

#[cfg(test)]
fn crates_io_api_base_override() -> Option<String> {
    CRATES_IO_API_BASE_OVERRIDE.with(|cell| cell.borrow().clone())
}

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
        read_manifest_publishable(manifest_path)
    }

    pub(super) fn version_exists(
        &self,
        package_name: &str,
        version: &str,
        manifest_path: Option<&Path>,
    ) -> Result<bool> {
        cargo_version_exists(package_name, version, manifest_path)
    }

    pub(super) fn publish(
        &self,
        manifest_path: &Path,
        dry_run: bool,
        extra_args: &[String],
    ) -> Result<()> {
        let mut cmd = Command::new("cargo");
        cmd.arg("publish").arg("--manifest-path").arg(manifest_path);

        // Cargo rejects a repeated `--dry-run`.
        if dry_run && !has_flag(extra_args, "--dry-run") {
            cmd.arg("--dry-run");
        }

        if !extra_args.is_empty() {
            cmd.args(extra_args);
        }

        println!("Running: {}", format_command_display(&cmd));

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
    cmd.arg("publish").arg("--workspace");
    if !has_flag(extra_args, "--dry-run") {
        cmd.arg("--dry-run");
    }
    cmd.arg("--manifest-path").arg(&manifest_path);

    for pkg in packages {
        cmd.arg("--package").arg(pkg);
    }

    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }

    println!("Running: {}", format_command_display(&cmd));

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
) -> Result<Vec<String>> {
    if packages.is_empty() {
        return Ok(Vec::new());
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
                    .map(|()| Vec::new())
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

    let mut skipped = Vec::new();

    for (package, manifest) in packages {
        if skip_dependent_packages && !package.internal_deps.is_empty() {
            println!(
                "  - Skipping dry-run for {} (requires workspace-aware Cargo to validate dependencies)",
                package.display_name(true)
            );
            skipped.push(package.display_name(true));
            continue;
        }

        run_cargo_dry_run(package, manifest, extra_args)?;
    }

    Ok(skipped)
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

fn read_manifest_publishable(manifest_path: &Path) -> Result<bool> {
    let manifest = read_manifest_toml(manifest_path)?;
    let workspace_root = inherited_publish_workspace_root(&manifest, manifest_path)?;
    Ok(manifest_allows_publish(
        &manifest,
        workspace_root.as_ref(),
        manifest_path,
    )?)
}

fn read_manifest_toml(manifest_path: &Path) -> Result<toml::Value> {
    let text = fs::read_to_string(manifest_path)
        .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
    toml::from_str(&text).map_err(|e| {
        SampoError::InvalidData(format!("invalid TOML in {}: {e}", manifest_path.display()))
    })
}

/// The workspace root manifest, read only when `[package].publish` inherits from it so
/// checks on standalone manifests stay a single file read.
///
/// Locates the root from the `package.workspace` path when set, otherwise the nearest
/// `Cargo.toml` with a `[workspace]` table, starting from the crate's own directory.
/// Unlike Cargo, a `[workspace].exclude` covering the crate does not skip that root.
fn inherited_publish_workspace_root(
    manifest: &toml::Value,
    manifest_path: &Path,
) -> Result<Option<toml::Value>> {
    let Some(pkg) = manifest.get("package") else {
        return Ok(None);
    };
    if !pkg.get("publish").is_some_and(is_workspace_inherited) {
        return Ok(None);
    }

    let manifest_path = std::path::absolute(manifest_path)
        .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
    let Some(crate_dir) = manifest_path.parent() else {
        return Ok(None);
    };
    if let Some(explicit_root) = pkg.get("workspace").and_then(toml::Value::as_str) {
        let root_manifest = crate_dir.join(explicit_root).join(CARGO_MANIFEST);
        return read_manifest_toml(&root_manifest).map(Some);
    }
    for dir in crate_dir.ancestors() {
        let candidate = dir.join(CARGO_MANIFEST);
        if !candidate.is_file() {
            continue;
        }
        let value = read_manifest_toml(&candidate)?;
        if value.get("workspace").is_some() {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

/// Whether a field uses `field.workspace = true` (or `field = { workspace = true }`).
fn is_workspace_inherited(value: &toml::Value) -> bool {
    value.get("workspace").and_then(toml::Value::as_bool) == Some(true)
}

/// The effective `[package].publish`, resolving `publish.workspace = true` to
/// `[workspace.package].publish` in `workspace_root`. `None` when the field is absent.
fn resolve_package_publish<'a>(
    pkg: &'a toml::map::Map<String, toml::Value>,
    workspace_root: Option<&'a toml::Value>,
    manifest_path: &Path,
) -> std::result::Result<Option<&'a toml::Value>, WorkspaceError> {
    let Some(publish) = pkg.get("publish") else {
        return Ok(None);
    };
    if !is_workspace_inherited(publish) {
        return Ok(Some(publish));
    }
    workspace_root
        .and_then(|root| root.get("workspace")?.get("package")?.get("publish"))
        .map(Some)
        .ok_or_else(|| {
            WorkspaceError::InvalidManifest(format!(
                "{}: publish.workspace = true requires workspace.package.publish",
                manifest_path.display()
            ))
        })
}

/// Per the effective `[package].publish`, Cargo forbids publishing only when it is `false`
/// or an empty list; a single alternative registry stays publishable.
fn manifest_allows_publish(
    manifest: &toml::Value,
    workspace_root: Option<&toml::Value>,
    manifest_path: &Path,
) -> std::result::Result<bool, WorkspaceError> {
    let Some(pkg) = manifest.get("package").and_then(|v| v.as_table()) else {
        return Ok(false);
    };
    let allowed = match resolve_package_publish(pkg, workspace_root, manifest_path)? {
        Some(toml::Value::Boolean(false)) => false,
        Some(toml::Value::Array(arr)) => !arr.is_empty(),
        _ => true,
    };
    Ok(allowed)
}

/// The single alternative registry a crate publishes to, if any.
///
/// Mirrors Cargo's auto-selection: a lone non-crates.io entry is the target; an absent
/// field, multiple registries, or an explicit `crates-io` all default to crates.io
/// (`None`).
fn manifest_publish_target(
    manifest: &toml::Value,
    workspace_root: Option<&toml::Value>,
    manifest_path: &Path,
) -> Option<String> {
    let pkg = manifest.get("package")?.as_table()?;
    let arr = resolve_package_publish(pkg, workspace_root, manifest_path)
        .ok()
        .flatten()?
        .as_array()?;
    let [only] = arr.as_slice() else {
        return None;
    };
    let name = only.as_str()?;
    (name != "crates-io").then(|| name.to_string())
}

/// Alternative registries go through `cargo info` to reuse Cargo's config resolution
/// and stored credentials; crates.io uses its HTTP API directly.
fn cargo_version_exists(
    crate_name: &str,
    version: &str,
    manifest_path: Option<&Path>,
) -> Result<bool> {
    if let Some(path) = manifest_path
        && let Some(registry) = publish_target_registry(path)
    {
        return version_exists_via_cargo_info(crate_name, version, &registry, path);
    }
    version_exists_on_crates_io(crate_name, version)
}

fn publish_target_registry(manifest_path: &Path) -> Option<String> {
    let manifest = read_manifest_toml(manifest_path).ok()?;
    let workspace_root = inherited_publish_workspace_root(&manifest, manifest_path).ok()?;
    manifest_publish_target(&manifest, workspace_root.as_ref(), manifest_path)
}

/// Runs from the manifest directory so Cargo's config discovery walks up to the
/// `.cargo/config.toml` that defines the registry.
fn version_exists_via_cargo_info(
    crate_name: &str,
    version: &str,
    registry: &str,
    manifest_path: &Path,
) -> Result<bool> {
    let spec = format!("{}@{}", crate_name, version);
    let mut cmd = Command::new("cargo");
    cmd.args(cargo_info_args(&spec, registry));
    if let Some(dir) = manifest_path.parent() {
        cmd.current_dir(dir);
    }

    let output = cmd.output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SampoError::Publish(
                "cargo not found in PATH; it is needed to check existing versions on alternative registries"
                    .to_string(),
            )
        } else {
            SampoError::Io(err)
        }
    })?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    interpret_cargo_info(output.status.success(), stderr.as_ref(), &spec, registry)
}

fn cargo_info_args(spec: &str, registry: &str) -> Vec<String> {
    vec![
        "info".to_string(),
        spec.to_string(),
        "--registry".to_string(),
        registry.to_string(),
    ]
}

/// Only a "could not find ... in registry" failure means absent; any other failure
/// (auth, network) is surfaced as an error rather than mistaken for a missing version.
fn interpret_cargo_info(success: bool, stderr: &str, spec: &str, registry: &str) -> Result<bool> {
    if success {
        return Ok(true);
    }
    if stderr.contains("could not find") && stderr.contains("in registry") {
        return Ok(false);
    }
    let snippet: String = stderr.trim().chars().take(400).collect();
    Err(SampoError::Publish(format!(
        "cargo info failed for '{}' on registry '{}': {}",
        spec, registry, snippet
    )))
}

/// Query crates.io API to check if a specific version already exists.
fn version_exists_on_crates_io(crate_name: &str, version: &str) -> Result<bool> {
    // Query crates.io: https://crates.io/api/v1/crates/<name>/<version>
    #[cfg(test)]
    let base =
        crates_io_api_base_override().unwrap_or_else(|| "https://crates.io/api/v1".to_string());
    #[cfg(not(test))]
    let base = "https://crates.io/api/v1".to_string();
    let url = format!("{}/crates/{}/{}", base, crate_name, version);

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

fn regenerate_cargo_lockfile(root: &Path) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args(["update", "--workspace"]).current_dir(root);

    println!("Regenerating Cargo.lock…");
    let status = cmd.status().map_err(SampoError::Io)?;
    if !status.success() {
        return Err(SampoError::Release(format!(
            "cargo update --workspace failed with status {}",
            status
        )));
    }
    println!("Cargo.lock updated.");
    Ok(())
}

/// Metadata extracted from `cargo_metadata` for the workspace.
pub struct ManifestMetadata {
    by_manifest: HashMap<PathBuf, Vec<MetadataDependency>>,
}

struct MetadataDependency {
    package_name: String,
    path: Option<PathBuf>,
    version_req: Option<String>,
    inherited: bool,
}

impl MetadataDependency {
    fn planned_requirement(&self, new_version: &str) -> Option<String> {
        let requirement = self.version_req.as_ref()?;
        let updated = if self.inherited {
            compute_workspace_dependency_version(requirement, new_version)
        } else {
            updated_dependency_requirement(requirement, new_version)
        };
        Some(updated.unwrap_or_else(|| requirement.clone()))
    }
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

        let root = read_dependency_manifest(&manifest_path)?;
        let workspace_deps = root.get("workspace").and_then(|ws| ws.get("dependencies"));
        let mut by_manifest = HashMap::new();

        for package in metadata.packages {
            if !workspace_ids.contains(&package.id) {
                continue;
            }

            let manifest_path = package.manifest_path.into_std_path_buf();
            let manifest = read_dependency_manifest(&manifest_path)?;
            let dependencies = package
                .dependencies
                .into_iter()
                .map(|dep| {
                    let (version_req, inherited) =
                        dependency_requirement(&manifest, workspace_deps, &dep)
                            .map(|(version, inherited)| (version.map(str::to_owned), inherited))
                            .unwrap_or_else(|| (Some(dep.req.to_string()), false));
                    MetadataDependency {
                        package_name: dep.name,
                        path: dep.path.map(|path| path.into_std_path_buf()),
                        version_req,
                        inherited,
                    }
                })
                .collect();
            by_manifest.insert(manifest_path, dependencies);
        }

        Ok(Self { by_manifest })
    }

    fn dependencies<'a>(
        &'a self,
        manifest_path: &Path,
        dep_name: &'a str,
    ) -> impl Iterator<Item = &'a MetadataDependency> {
        self.by_manifest
            .get(manifest_path)
            .into_iter()
            .flatten()
            .filter(move |dep| dep.package_name == dep_name)
    }

    /// Returns a Cargo-normalized constraint for the named dependency, preferring a range over a pin.
    pub fn get_dependency_constraint(
        &self,
        manifest_path: &Path,
        dep_name: &str,
    ) -> Option<String> {
        let requirement = self
            .dependencies(manifest_path, dep_name)
            .map(|dep| dep.version_req.as_deref().unwrap_or("*"))
            .min_by_key(|requirement| is_pin(requirement))?;
        VersionReq::parse(requirement)
            .ok()
            .map(|req| req.to_string())
    }

    /// Whether every declaration of the named dependency specifies a bare or exact pin.
    pub fn is_dependency_pinned(&self, manifest_path: &Path, dep_name: &str) -> bool {
        let mut dependencies = self.dependencies(manifest_path, dep_name).peekable();
        dependencies.peek().is_some()
            && dependencies.all(|dep| dep.version_req.as_deref().is_some_and(is_pin))
    }

    pub(crate) fn check_internal_dependency(
        &self,
        manifest_path: &Path,
        dependency: &PackageInfo,
        new_version: &str,
    ) -> Result<Vec<ConstraintCheckResult>> {
        let requirements: BTreeSet<_> = self
            .dependencies(manifest_path, &dependency.name)
            .filter(|dep| dep.path.as_deref() == Some(&dependency.path))
            .map(|dep| dep.planned_requirement(new_version))
            .collect();
        if requirements.is_empty() {
            return Ok(vec![ConstraintCheckResult::Skipped {
                reason: "no matching path dependency".into(),
            }]);
        }
        requirements
            .into_iter()
            .map(|requirement| match requirement {
                Some(req) => check_dependency_constraint(&dependency.name, &req, new_version),
                None => Ok(ConstraintCheckResult::Satisfied),
            })
            .collect()
    }
}

fn read_dependency_manifest(path: &Path) -> Result<DocumentMut> {
    let content =
        fs::read_to_string(path).map_err(|err| crate::errors::io_error_with_path(err, path))?;
    content.parse().map_err(|err| {
        SampoError::Release(format!(
            "Failed to parse manifest {}: {err}",
            path.display()
        ))
    })
}

/// Cargo normalizes bare pins to caret ranges and normalizes whitespace in target expressions.
fn dependency_requirement<'a>(
    doc: &'a DocumentMut,
    workspace_deps: Option<&'a Item>,
    dep: &Dependency,
) -> Option<(Option<&'a str>, bool)> {
    let key = dep.rename.as_deref().unwrap_or(&dep.name);
    let (section, _) = DEPENDENCY_SECTIONS
        .iter()
        .find(|(_, kind)| *kind == dep.kind)?;
    let requirement = |parent: &'a dyn TableLike| {
        let item = parent.get(section)?.as_table_like()?.get(key)?;
        let inherited = has_workspace_flag(Some(item));
        let declaration = if inherited {
            workspace_deps?.get(key)?
        } else {
            item
        };
        let version = declaration
            .as_str()
            .or_else(|| declaration.get("version")?.as_str());
        Some((version, inherited))
    };
    let Some(platform) = &dep.target else {
        return requirement(doc.as_table());
    };
    doc.get("target")?
        .as_table_like()?
        .iter()
        .filter(|(target, _)| target.parse().ok().as_ref() == Some(platform))
        .filter_map(|(_, target)| requirement(target.as_table_like()?))
        .filter(|(version, _)| {
            VersionReq::parse(version.unwrap_or("*")).ok().as_ref() == Some(&dep.req)
        })
        .min_by_key(|(version, inherited)| {
            version.is_none_or(|req| {
                is_pin(req) || *inherited && parse_numeric_shorthand(req).is_some()
            })
        })
}

/// Update a Cargo manifest by setting the package version (if provided) and retargeting internal
/// dependency requirements to the latest planned versions.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
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

    for (dep_name, new_version) in new_version_by_name {
        let mut changed = false;

        changed |= update_all_dependencies(&mut doc, dep_name, new_version);
        changed |= update_workspace_dependency(&mut doc, dep_name, new_version);

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

    // Workspace-inherited version is updated at the root manifest instead.
    if has_workspace_flag(package_table.get("version")) {
        return Ok(());
    }

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

/// Check whether a Cargo manifest inherits its version from the workspace root.
pub fn has_workspace_version_inheritance(manifest_content: &str) -> Result<bool> {
    let doc: DocumentMut = manifest_content
        .parse()
        .map_err(|err| SampoError::Release(format!("Failed to parse manifest: {err}")))?;

    let version_item = doc
        .as_table()
        .get("package")
        .and_then(Item::as_table)
        .and_then(|pkg| pkg.get("version"));

    Ok(has_workspace_flag(version_item))
}

/// Finalize the workspace root manifest after member manifests have been updated.
///
/// Reads each Cargo member manifest to detect `version.workspace = true`, cross-references with
/// `new_version_by_name` to determine the workspace version, validates that all inheriting members
/// agree on the same version, then updates `[workspace.package].version` and
/// `[workspace.dependencies]` in the root `Cargo.toml`.
pub fn finalize_workspace_root(
    workspace_root: &Path,
    members: &[PackageInfo],
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<()> {
    let mut workspace_version: Option<String> = None;

    for member in members {
        if member.kind != PackageKind::Cargo {
            continue;
        }

        let member_manifest = member.path.join(CARGO_MANIFEST);
        let content = fs::read_to_string(&member_manifest)?;

        if !has_workspace_version_inheritance(&content)? {
            continue;
        }

        if let Some(new_version) = new_version_by_name.get(&member.name) {
            match &workspace_version {
                None => {
                    workspace_version = Some(new_version.clone());
                }
                Some(existing) if existing != new_version => {
                    return Err(SampoError::Release(format!(
                        "workspace-inherited packages resolved to conflicting versions: '{}' and '{}'",
                        existing, new_version
                    )));
                }
                Some(_) => {}
            }
        }
    }

    let manifest_path = workspace_root.join(CARGO_MANIFEST);
    let input = fs::read_to_string(&manifest_path)?;

    let cargo_member_names: BTreeSet<&str> = members
        .iter()
        .filter(|m| m.kind == PackageKind::Cargo)
        .map(|m| m.name.as_str())
        .collect();
    let cargo_versions: BTreeMap<String, String> = new_version_by_name
        .iter()
        .filter(|(name, _)| cargo_member_names.contains(name.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let (updated, changed) = update_workspace_root_manifest(
        &manifest_path,
        &input,
        workspace_version.as_deref(),
        &cargo_versions,
    )?;

    if changed {
        fs::write(&manifest_path, updated)?;
    }

    Ok(())
}

/// Update version and internal dependency specs in the workspace root manifest.
pub fn update_workspace_root_manifest(
    manifest_path: &Path,
    input: &str,
    new_workspace_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, bool)> {
    let mut doc: DocumentMut = input.parse().map_err(|err| {
        SampoError::Release(format!(
            "Failed to parse root manifest {}: {err}",
            manifest_path.display()
        ))
    })?;

    let mut changed = false;

    if let Some(new_version) = new_workspace_version
        && let Some(workspace_pkg) = doc
            .as_table_mut()
            .get_mut("workspace")
            .and_then(Item::as_table_mut)
            .and_then(|ws| ws.get_mut("package"))
            .and_then(Item::as_table_mut)
    {
        let current = workspace_pkg
            .get("version")
            .and_then(Item::as_value)
            .and_then(Value::as_str);

        if current.is_some() && current != Some(new_version) {
            workspace_pkg.insert("version", Item::Value(Value::from(new_version)));
            changed = true;
        }
    }

    for (dep_name, new_version) in new_version_by_name {
        changed |= update_workspace_dependency(&mut doc, dep_name, new_version);
    }

    Ok((doc.to_string(), changed))
}

fn has_workspace_flag(item: Option<&Item>) -> bool {
    item.and_then(|item| item.get("workspace"))
        .and_then(Item::as_bool)
        == Some(true)
}

/// Update all dependency occurrences of `dep_name` in the manifest, including renamed deps
/// where `package = "dep_name"` is specified.
fn update_all_dependencies(doc: &mut DocumentMut, dep_name: &str, new_version: &str) -> bool {
    let top = doc.as_table_mut();
    let mut changed = update_dependency_sections(top, dep_name, new_version);
    if let Some(targets) = top.get_mut("target").and_then(Item::as_table_like_mut) {
        for (_, target) in targets.iter_mut() {
            if let Some(table) = target.as_table_like_mut() {
                changed |= update_dependency_sections(table, dep_name, new_version);
            }
        }
    }
    changed
}

fn update_dependency_sections(
    parent: &mut dyn TableLike,
    dep_name: &str,
    new_version: &str,
) -> bool {
    let mut changed = false;
    for (section, _) in DEPENDENCY_SECTIONS {
        if let Some(table) = parent.get_mut(section).and_then(Item::as_table_like_mut) {
            changed |=
                update_deps_in_table(table, dep_name, new_version, updated_dependency_requirement);
        }
    }
    changed
}

fn update_deps_in_table(
    table: &mut dyn TableLike,
    dep_name: &str,
    new_version: &str,
    update: fn(&str, &str) -> Option<String>,
) -> bool {
    let mut changed = false;
    for (key, item) in table.iter_mut() {
        let package_name = item.get("package").and_then(Item::as_str).unwrap_or(&key);
        if package_name != dep_name || has_workspace_flag(Some(item)) {
            continue;
        }
        let requirement = item.as_str().or_else(|| item.get("version")?.as_str());
        let Some(updated) = requirement.and_then(|req| update(req, new_version)) else {
            continue;
        };
        if let Some(table) = item.as_table_like_mut() {
            table.insert("version", toml_edit::value(updated));
        } else {
            *item = toml_edit::value(updated);
        }
        changed = true;
    }
    changed
}

/// The operator of a pin: empty for a bare `1.0.0`, `=` for an exact `=1.0.0`. `None` for a
/// range, including a partial `=1.0`, which Cargo reads as `>=1.0.0, <1.1.0`.
fn pin_operator(requirement: &str) -> Option<&'static str> {
    let trimmed = requirement.trim();

    if Version::parse(trimmed).is_ok() {
        Some("")
    } else if trimmed
        .strip_prefix('=')
        .is_some_and(|version| Version::parse(version.trim()).is_ok())
    {
        Some("=")
    } else {
        None
    }
}

fn is_pin(requirement: &str) -> bool {
    pin_operator(requirement).is_some()
}

/// A pin moved to `new_version` with its operator, or `None` when there is nothing to write:
/// ranges are deferred to constraint validation in release.rs.
fn updated_dependency_requirement(existing: &str, new_version: &str) -> Option<String> {
    pin_operator(existing)
        .map(|operator| format!("{operator}{new_version}"))
        .filter(|updated| updated != existing.trim())
}

fn update_workspace_dependency(doc: &mut DocumentMut, dep_name: &str, new_version: &str) -> bool {
    let Some(deps) = doc
        .get_mut("workspace")
        .and_then(Item::as_table_like_mut)
        .and_then(|ws| ws.get_mut("dependencies"))
        .and_then(Item::as_table_like_mut)
    else {
        return false;
    };
    update_deps_in_table(
        deps,
        dep_name,
        new_version,
        compute_workspace_dependency_version,
    )
}

fn compute_workspace_dependency_version(existing: &str, new_version: &str) -> Option<String> {
    let trimmed_existing = existing.trim();
    if trimmed_existing == "*" {
        return None;
    }

    // A pin already on `new_version` falls through, and has too many parts to be a shorthand.
    if let Some(updated) = updated_dependency_requirement(trimmed_existing, new_version) {
        return Some(updated);
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

/// Versioned dev dependencies affect release planning without adding a publish-order constraint.
fn collect_cargo_internal_deps(
    crate_dir: &Path,
    root: &Path,
    name_to_path: &BTreeMap<String, PathBuf>,
    workspace_deps: Option<&toml::map::Map<String, toml::Value>>,
    manifest: &toml::Value,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut internal = BTreeSet::new();
    let mut internal_dev = BTreeSet::new();
    let mut tables = vec![manifest];
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        tables.extend(targets.values());
    }

    for table in tables {
        for (section, kind) in DEPENDENCY_SECTIONS {
            let Some(deps) = table.get(section).and_then(toml::Value::as_table) else {
                continue;
            };
            for (dep_name, dep_val) in deps {
                let (base, declaration) = if is_workspace_inherited(dep_val) {
                    let Some(declaration) = workspace_deps.and_then(|deps| deps.get(dep_name))
                    else {
                        continue;
                    };
                    (root, declaration)
                } else {
                    (crate_dir, dep_val)
                };
                let Some(path) = declaration.get("path").and_then(toml::Value::as_str) else {
                    continue;
                };
                let package_name = declaration
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(dep_name);
                if name_to_path.get(package_name) != Some(&clean_path(&base.join(path))) {
                    continue;
                }
                let id = PackageInfo::dependency_identifier(PackageKind::Cargo, package_name);
                if kind != DependencyKind::Development {
                    internal.insert(id);
                } else if dep_val.get("version").is_some() {
                    internal_dev.insert(id);
                }
            }
        }
    }
    (internal, internal_dev)
}

fn resolve_package_version(
    pkg: &toml::map::Map<String, toml::Value>,
    workspace_version: &Option<String>,
    manifest_path: &Path,
) -> std::result::Result<String, WorkspaceError> {
    match pkg.get("version") {
        Some(toml::Value::String(v)) => Ok(v.clone()),
        Some(v @ toml::Value::Table(_)) => {
            if is_workspace_inherited(v) {
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
    let root_toml: toml::Value = toml::from_str(&text).map_err(|e| {
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
        let value: toml::Value = toml::from_str(&text).map_err(|e| {
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
        // A non-publishable, versionless crate is not a release target.
        if version.is_empty() && !manifest_allows_publish(&value, Some(&root_toml), &manifest_path)?
        {
            continue;
        }
        name_to_path.insert(name.clone(), member_dir.clone());
        crates.push((name, version, member_dir.clone(), value));
    }

    let workspace_deps = root_toml
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(toml::Value::as_table);

    let mut out: Vec<PackageInfo> = Vec::new();
    for (name, version, path, manifest) in crates {
        let identifier = PackageInfo::dependency_identifier(PackageKind::Cargo, &name);
        let (internal_deps, internal_dev_deps) =
            collect_cargo_internal_deps(&path, root, &name_to_path, workspace_deps, &manifest);
        out.push(PackageInfo {
            name,
            identifier,
            version,
            path,
            internal_deps,
            internal_dev_deps,
            kind: PackageKind::Cargo,
        });
    }

    Ok(out)
}

#[cfg(test)]
mod cargo_tests;
