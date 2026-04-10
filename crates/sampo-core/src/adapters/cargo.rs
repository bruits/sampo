/// Cargo ecosystem adapter for all Cargo operations.
use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind};
use semver::{Version, VersionReq};
use std::collections::{BTreeMap, BTreeSet};
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
    let Some(item) = item else { return false };

    if let Some(table) = item.as_table() {
        return table
            .get("workspace")
            .and_then(Item::as_value)
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }

    if let Some(value) = item.as_value()
        && let Some(inline) = value.as_inline_table()
    {
        return inline
            .get("workspace")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }

    false
}

/// Update all dependency occurrences of `dep_name` in the manifest, including renamed deps
/// where `package = "dep_name"` is specified.
fn update_all_dependencies(doc: &mut DocumentMut, dep_name: &str, new_version: &str) -> bool {
    let mut changed = false;
    let top_level = doc.as_table_mut();

    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = top_level.get_mut(section).and_then(Item::as_table_mut) {
            changed |= update_deps_in_table(table, dep_name, new_version);
        }
    }

    if let Some(targets) = top_level.get_mut("target").and_then(Item::as_table_mut) {
        for (_, target_item) in targets.iter_mut() {
            if let Some(target_table) = target_item.as_table_mut() {
                for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(table) = target_table.get_mut(section).and_then(Item::as_table_mut)
                    {
                        changed |= update_deps_in_table(table, dep_name, new_version);
                    }
                }
            }
        }
    }

    changed
}

fn update_deps_in_table(table: &mut Table, dep_name: &str, new_version: &str) -> bool {
    let mut changed = false;

    // Direct match by key name
    if let Some(item) = table.get_mut(dep_name) {
        changed |= update_standard_dependency_item(item, new_version);
    }

    // Renamed deps: entries where key != dep_name but `package = "dep_name"`
    let renamed_keys: Vec<String> = table
        .iter()
        .filter(|(key, item)| *key != dep_name && has_package_field(item, dep_name))
        .map(|(key, _)| key.to_string())
        .collect();

    for key in renamed_keys {
        if let Some(item) = table.get_mut(&key) {
            changed |= update_standard_dependency_item(item, new_version);
        }
    }

    changed
}

fn has_package_field(item: &Item, name: &str) -> bool {
    match item {
        Item::Value(Value::InlineTable(table)) => {
            table.get("package").and_then(Value::as_str) == Some(name)
        }
        Item::Table(table) => {
            table
                .get("package")
                .and_then(Item::as_value)
                .and_then(Value::as_str)
                == Some(name)
        }
        _ => false,
    }
}

fn update_standard_dependency_item(item: &mut Item, new_version: &str) -> bool {
    match item {
        Item::Value(Value::InlineTable(table)) => update_inline_dependency(table, new_version),
        Item::Table(table) => update_table_dependency(table, new_version),
        Item::Value(value) => {
            if value.as_str() == Some(new_version) {
                false
            } else {
                *item = Item::Value(Value::from(new_version));
                true
            }
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

    let needs_update = table
        .get("version")
        .and_then(Value::as_str)
        .map(|current| current != new_version)
        .unwrap_or(true);

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

    let needs_update = table
        .get("version")
        .and_then(Item::as_value)
        .and_then(Value::as_str)
        .map(|current| current != new_version)
        .unwrap_or(true);

    if needs_update {
        table.insert("version", Item::Value(Value::from(new_version)));
    }

    needs_update
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

/// Collect internal dependencies for a crate.
/// Returns (internal_deps, internal_dev_deps). Dev-dependencies are tracked
/// separately because they don't create publish-order constraints (cargo strips
/// them during publish), but their versions still potentially need updating during releases.
fn collect_cargo_internal_deps(
    crate_dir: &Path,
    name_to_path: &BTreeMap<String, PathBuf>,
    manifest: &toml::Value,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut internal = BTreeSet::new();
    let mut internal_dev = BTreeSet::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(tbl) = manifest.get(key).and_then(|v| v.as_table()) {
            for (dep_name, dep_val) in tbl {
                if is_cargo_internal_dep(crate_dir, name_to_path, dep_name, dep_val) {
                    let id = PackageInfo::dependency_identifier(PackageKind::Cargo, dep_name);
                    if key == "dev-dependencies" {
                        // Only track dev-deps that have a version field —
                        // path-only dev-deps have no version to update.
                        let has_version = dep_val
                            .as_table()
                            .map(|t| t.contains_key("version"))
                            .unwrap_or(false);
                        if has_version {
                            internal_dev.insert(id);
                        }
                    } else {
                        internal.insert(id);
                    }
                }
            }
        }
    }
    (internal, internal_dev)
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
        let (internal_deps, internal_dev_deps) =
            collect_cargo_internal_deps(&path, &name_to_path, &manifest);
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
