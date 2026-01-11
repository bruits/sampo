use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use toml_edit::{DocumentMut, Item, Value};

const PYPROJECT_MANIFEST: &str = "pyproject.toml";

pub(super) fn can_discover(root: &Path) -> bool {
    root.join(PYPROJECT_MANIFEST).exists()
}

pub(super) fn discover(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    let manifest_path = root.join(PYPROJECT_MANIFEST);
    if !manifest_path.exists() {
        return Err(WorkspaceError::InvalidWorkspace(format!(
            "Expected {} in {}",
            PYPROJECT_MANIFEST,
            root.display()
        )));
    }

    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, &manifest_path)))?;
    let project_meta = parse_project_metadata(&manifest_text);

    let mut package_dirs: BTreeSet<PathBuf> = BTreeSet::new();

    // Check if this is a valid package (has name)
    if project_meta.name.is_some() {
        package_dirs.insert(normalize_path(root));
    }

    // Check for uv workspace members in [tool.uv.workspace]
    if let Some(members) = parse_uv_workspace_members(&manifest_text) {
        for pattern in members {
            let pattern_path = root.join(&pattern);

            // Handle glob patterns like "packages/*"
            if pattern.contains('*') {
                let base = pattern.trim_end_matches("/*").trim_end_matches("/**");
                let base_dir = root.join(base);
                if base_dir.exists()
                    && let Ok(entries) = fs::read_dir(&base_dir)
                {
                    for entry in entries.flatten() {
                        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                            continue;
                        }
                        let dir = entry.path();
                        if dir.join(PYPROJECT_MANIFEST).exists() {
                            package_dirs.insert(normalize_path(&dir));
                        }
                    }
                }
            } else if pattern_path.join(PYPROJECT_MANIFEST).exists() {
                package_dirs.insert(normalize_path(&pattern_path));
            }
        }
    }

    let mut manifests = Vec::new();
    // PEP 503: package names are case-insensitive and treat `.`, `-`, `_` as equivalent
    let mut normalized_name_to_original: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();

    for dir in package_dirs {
        let manifest_path = dir.join(PYPROJECT_MANIFEST);
        let text = fs::read_to_string(&manifest_path).map_err(|e| {
            WorkspaceError::Io(crate::errors::io_error_with_path(e, &manifest_path))
        })?;
        let meta = parse_project_metadata(&text);
        let name = meta.name.ok_or_else(|| {
            WorkspaceError::InvalidManifest(format!(
                "missing project.name in {}",
                manifest_path.display()
            ))
        })?;
        let version = meta.version.unwrap_or_default();
        let deps = collect_dependencies(&text);

        let normalized = normalize_package_name(&name);

        // Detect collision: two packages normalizing to the same PEP 503 name
        if let Some((existing_name, existing_path)) = normalized_name_to_original.get(&normalized) {
            return Err(WorkspaceError::InvalidWorkspace(format!(
                "packages '{}' (at {}) and '{}' (at {}) normalize to the same PEP 503 name '{}'. \
                 Python ecosystems do not allow duplicate normalized names",
                existing_name,
                existing_path.display(),
                name,
                dir.display(),
                normalized
            )));
        }

        normalized_name_to_original.insert(normalized, (name.clone(), dir.clone()));
        manifests.push((name, version, dir, deps));
    }

    let mut packages = Vec::new();
    for (name, version, dir, deps) in manifests {
        let identifier = PackageInfo::dependency_identifier(PackageKind::PyPI, &name);
        let mut internal = BTreeSet::new();

        for dep_name in deps {
            let normalized_dep = normalize_package_name(&dep_name);
            if let Some((original_name, _)) = normalized_name_to_original.get(&normalized_dep) {
                internal.insert(PackageInfo::dependency_identifier(
                    PackageKind::PyPI,
                    original_name,
                ));
            }
        }

        packages.push(PackageInfo {
            name,
            identifier,
            version,
            path: dir,
            internal_deps: internal,
            kind: PackageKind::PyPI,
        });
    }

    Ok(packages)
}

pub(super) fn manifest_path(package_dir: &Path) -> PathBuf {
    package_dir.join(PYPROJECT_MANIFEST)
}

pub(super) fn is_publishable(manifest_path: &Path) -> Result<bool> {
    let text = fs::read_to_string(manifest_path)
        .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
    let ProjectMetadata { name, version, .. } = parse_project_metadata(&text);

    let Some(name) = name else {
        return Err(SampoError::Publish(format!(
            "Manifest {} is missing a project.name field",
            manifest_path.display()
        )));
    };
    if name.trim().is_empty() {
        return Err(SampoError::Publish(format!(
            "Manifest {} declares an empty project name",
            manifest_path.display()
        )));
    }

    let Some(version) = version else {
        return Err(SampoError::Publish(format!(
            "Manifest {} is missing a project.version field",
            manifest_path.display()
        )));
    };
    if version.trim().is_empty() {
        return Err(SampoError::Publish(format!(
            "Manifest {} declares an empty version",
            manifest_path.display()
        )));
    }

    Ok(true)
}

pub(super) fn publish(manifest_path: &Path, dry_run: bool, extra_args: &[String]) -> Result<()> {
    let manifest_dir = manifest_path.parent().ok_or_else(|| {
        SampoError::Publish(format!(
            "Manifest {} does not have a parent directory",
            manifest_path.display()
        ))
    })?;

    let text = fs::read_to_string(manifest_path)
        .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
    let ProjectMetadata { name, version } = parse_project_metadata(&text);
    let package = name.ok_or_else(|| {
        SampoError::Publish(format!(
            "Manifest {} is missing a project.name field",
            manifest_path.display()
        ))
    })?;

    let version = version.ok_or_else(|| {
        SampoError::Publish(format!(
            "Manifest {} is missing a project.version field",
            manifest_path.display()
        ))
    })?;
    if version.trim().is_empty() {
        return Err(SampoError::Publish(format!(
            "Manifest {} declares an empty version",
            manifest_path.display()
        )));
    }

    // Clean previous build artifacts
    let dist_dir = manifest_dir.join("dist");
    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir)
            .map_err(|e| SampoError::Publish(format!("failed to clean dist directory: {}", e)))?;
    }

    // Build the package using uv build
    let mut build_cmd = Command::new("uv");
    build_cmd.arg("build").current_dir(manifest_dir);

    println!("Running: {}", format_command_display(&build_cmd));

    let build_status = build_cmd.status().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SampoError::Publish("uv not found in PATH; install uv to build packages".to_string())
        } else {
            SampoError::Io(err)
        }
    })?;

    if !build_status.success() {
        return Err(SampoError::Publish(format!(
            "uv build failed for {} (package '{}') with status {}",
            manifest_path.display(),
            package,
            build_status
        )));
    }

    // For dry-run, stop here, simply verify the build succeeded (uv doesn't have a check command)
    if dry_run {
        println!("Dry-run: skipping publish for {} v{}", package, version);
        return Ok(());
    }

    // Actual upload using uv publish
    let mut publish_cmd = Command::new("uv");
    publish_cmd.arg("publish").current_dir(manifest_dir);

    if !extra_args.is_empty() {
        publish_cmd.args(extra_args);
    }

    println!("Running: {}", format_command_display(&publish_cmd));

    let publish_status = publish_cmd.status().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SampoError::Publish("uv not found in PATH; install uv to publish packages".to_string())
        } else {
            SampoError::Io(err)
        }
    })?;

    if !publish_status.success() {
        return Err(SampoError::Publish(format!(
            "uv publish failed for {} (package '{}') with status {}",
            manifest_path.display(),
            package,
            publish_status
        )));
    }

    Ok(())
}

pub(super) fn regenerate_lockfile(workspace_root: &Path) -> Result<()> {
    let manifest_path = workspace_root.join(PYPROJECT_MANIFEST);
    if !manifest_path.exists() {
        return Err(SampoError::Release(format!(
            "cannot regenerate lockfile; {} not found in {}",
            PYPROJECT_MANIFEST,
            workspace_root.display()
        )));
    }

    println!("Regenerating uv.lock…");
    let mut cmd = Command::new("uv");
    cmd.arg("lock").current_dir(workspace_root);

    println!("Running: {}", format_command_display(&cmd));

    let status = cmd.status().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SampoError::Release(
                "uv not found in PATH; install uv to regenerate uv.lock".to_string(),
            )
        } else {
            SampoError::Io(err)
        }
    })?;

    if !status.success() {
        return Err(SampoError::Release(format!(
            "uv lock failed with status {}",
            status
        )));
    }

    println!("uv.lock updated.");
    Ok(())
}

pub(super) fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    let mut doc: DocumentMut = input.parse().map_err(|err| {
        SampoError::Release(format!(
            "Failed to parse pyproject.toml {}: {err}",
            manifest_path.display()
        ))
    })?;

    let mut applied: Vec<(String, String)> = Vec::new();

    // Update the package version in [project] table
    if let Some(target_version) = new_pkg_version
        && let Some(project) = doc.get_mut("project").and_then(Item::as_table_mut)
        && let Some(version_item) = project.get_mut("version")
    {
        let current = version_item
            .as_value()
            .and_then(Value::as_str)
            .unwrap_or("");
        if current != target_version {
            *version_item = Item::Value(Value::from(target_version));
        }
    }

    // Update dependencies in [project.dependencies]
    if let Some(project) = doc.get_mut("project").and_then(Item::as_table_mut) {
        if let Some(deps) = project.get_mut("dependencies").and_then(Item::as_array_mut) {
            for item in deps.iter_mut() {
                if let Some(dep_str) = item.as_str()
                    && let Some((name, new_spec)) =
                        try_update_dependency_spec(dep_str, new_version_by_name)
                {
                    *item = Value::from(new_spec.clone());
                    let version = new_version_by_name.get(&name).cloned().unwrap_or_default();
                    applied.push((name, version));
                }
            }
        }

        // Update optional dependencies
        if let Some(optional) = project
            .get_mut("optional-dependencies")
            .and_then(Item::as_table_mut)
        {
            for (_, deps_item) in optional.iter_mut() {
                if let Some(deps) = deps_item.as_array_mut() {
                    for item in deps.iter_mut() {
                        if let Some(dep_str) = item.as_str()
                            && let Some((name, new_spec)) =
                                try_update_dependency_spec(dep_str, new_version_by_name)
                        {
                            *item = Value::from(new_spec.clone());
                            if !applied.iter().any(|(n, _)| n == &name) {
                                let version =
                                    new_version_by_name.get(&name).cloned().unwrap_or_default();
                                applied.push((name, version));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok((doc.to_string(), applied))
}

#[derive(Debug, Default)]
struct ProjectMetadata {
    name: Option<String>,
    version: Option<String>,
}

/// Parse PEP 621 [project] table metadata from pyproject.toml
fn parse_project_metadata(source: &str) -> ProjectMetadata {
    let doc: DocumentMut = match source.parse() {
        Ok(d) => d,
        Err(_) => return ProjectMetadata::default(),
    };

    let mut metadata = ProjectMetadata::default();

    if let Some(project) = doc.get("project").and_then(Item::as_table) {
        if let Some(name) = project.get("name").and_then(Item::as_str) {
            metadata.name = Some(name.to_string());
        }
        if let Some(version) = project.get("version").and_then(Item::as_str) {
            metadata.version = Some(version.to_string());
        }
    }

    metadata
}

/// Parse uv workspace members from [tool.uv.workspace]
fn parse_uv_workspace_members(source: &str) -> Option<Vec<String>> {
    let doc: DocumentMut = source.parse().ok()?;

    let tool = doc.get("tool")?.as_table()?;
    let uv = tool.get("uv")?.as_table()?;
    let workspace = uv.get("workspace")?.as_table()?;
    let members = workspace.get("members")?.as_array()?;

    let result: Vec<String> = members
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Collect dependency names from PEP 621 [project.dependencies] and [project.optional-dependencies]
fn collect_dependencies(source: &str) -> Vec<String> {
    let doc: DocumentMut = match source.parse() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut deps = Vec::new();

    let Some(project) = doc.get("project").and_then(Item::as_table) else {
        return deps;
    };

    // Parse [project.dependencies]
    if let Some(dependencies) = project.get("dependencies").and_then(Item::as_array) {
        for dep in dependencies.iter() {
            if let Some(dep_str) = dep.as_str()
                && let Some(name) = extract_package_name(dep_str)
            {
                deps.push(name);
            }
        }
    }

    // Also check optional dependencies
    if let Some(optional) = project
        .get("optional-dependencies")
        .and_then(Item::as_table)
    {
        for (_, deps_item) in optional.iter() {
            if let Some(deps_array) = deps_item.as_array() {
                for dep in deps_array.iter() {
                    if let Some(dep_str) = dep.as_str()
                        && let Some(name) = extract_package_name(dep_str)
                    {
                        deps.push(name);
                    }
                }
            }
        }
    }

    deps
}

/// Extract the package name from a PEP 508 dependency specifier.
/// e.g., "requests>=2.0" -> "requests", "my-package[extra]>=1.0" -> "my-package"
fn extract_package_name(spec: &str) -> Option<String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Find the end of the package name (before any version specifier, extras, or markers)
    // Include '~' for ~= operator
    let end_chars = ['>', '<', '=', '!', '~', '[', ';', ' ', '@'];
    let end_pos = trimmed
        .find(|c: char| end_chars.contains(&c))
        .unwrap_or(trimmed.len());

    let name = trimmed[..end_pos].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Try to update a dependency specifier if the package name matches one in new_version_by_name.
/// Returns Some((package_name, new_spec)) if updated, None otherwise.
///
/// Handles simple PEP 508 specifiers with a single version constraint.
/// Skips complex cases that require manual review:
/// - URL references: `package @ https://...`
/// - Multiple constraints: `pandas>=1.0,<2.0`
/// - No version specified: `requests`
fn try_update_dependency_spec(
    spec: &str,
    new_version_by_name: &BTreeMap<String, String>,
) -> Option<(String, String)> {
    let name = extract_package_name(spec)?;
    let normalized_name = normalize_package_name(&name);

    let (original_name, new_version) = new_version_by_name
        .iter()
        .find(|(k, _)| normalize_package_name(k) == normalized_name)?;

    let trimmed = spec.trim();

    if trimmed.contains(" @ ") {
        return None;
    }

    let (version_part, markers) = match trimmed.find(';') {
        Some(pos) => (&trimmed[..pos], Some(trimmed[pos..].trim())),
        None => (trimmed, None),
    };
    let version_part = version_part.trim();

    let after_extras = match (version_part.find('['), version_part.find(']')) {
        (Some(start), Some(end)) if start < end => &version_part[end + 1..],
        _ => {
            let name_end = version_part
                .find(|c: char| ['>', '<', '=', '!', '~'].contains(&c))
                .unwrap_or(version_part.len());
            &version_part[name_end..]
        }
    };
    let after_extras = after_extras.trim();

    // Multiple constraints require manual review as bumping may create invalid ranges
    if after_extras.contains(',') {
        return None;
    }

    let new_spec = if after_extras.is_empty() {
        return None;
    } else if let Some(current) = after_extras.strip_prefix(">=") {
        compute_new_spec(version_part, ">=", current.trim(), new_version)
    } else if let Some(current) = after_extras.strip_prefix("<=") {
        compute_new_spec(version_part, "<=", current.trim(), new_version)
    } else if let Some(current) = after_extras.strip_prefix("==") {
        compute_new_spec(version_part, "==", current.trim(), new_version)
    } else if let Some(current) = after_extras.strip_prefix("~=") {
        compute_new_spec(version_part, "~=", current.trim(), new_version)
    } else if let Some(current) = after_extras.strip_prefix("!=") {
        compute_new_spec(version_part, "!=", current.trim(), new_version)
    } else if let Some(current) = after_extras.strip_prefix('>') {
        compute_new_spec(version_part, ">", current.trim(), new_version)
    } else if let Some(current) = after_extras.strip_prefix('<') {
        compute_new_spec(version_part, "<", current.trim(), new_version)
    } else {
        return None;
    }?;

    let result = match markers {
        Some(m) => format!("{} {}", new_spec, m),
        None => new_spec,
    };

    Some((original_name.clone(), result))
}

/// Compute a new dependency spec by replacing only the version.
fn compute_new_spec(
    version_part: &str,
    operator: &str,
    current_version: &str,
    new_version: &str,
) -> Option<String> {
    if current_version == new_version {
        return None;
    }

    if !is_valid_version_token(current_version) {
        return None;
    }

    let op_start = version_part.find(operator)?;
    let prefix = &version_part[..op_start];

    Some(format!("{}{}{}", prefix, operator, new_version))
}

/// Check if a string looks like a valid simple version token.
fn is_valid_version_token(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(char::is_whitespace)
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+' | '*'))
}

/// Normalize a Python package name for PEP 503-compatible comparison.
///
/// Based on PEP 503, this lowercases the name and collapses runs of `.`, `-`,
/// or `_` into a single `-`. Additionally, leading and trailing separators are
/// stripped (such names are invalid on PyPI anyway).
///
/// Reference: https://peps.python.org/pep-0503/#normalized-names
fn normalize_package_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut prev_was_separator = false;

    for c in name.chars() {
        if c == '-' || c == '_' || c == '.' {
            if !prev_was_separator && !result.is_empty() {
                result.push('-');
            }
            prev_was_separator = true;
        } else {
            result.push(c.to_ascii_lowercase());
            prev_was_separator = false;
        }
    }

    if result.ends_with('-') {
        result.pop();
    }

    result
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => out.push(component),
        }
    }
    out
}

fn format_command_display(cmd: &Command) -> String {
    let mut text = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        text.push(' ');
        text.push_str(&arg.to_string_lossy());
    }
    text
}

#[cfg(test)]
mod pip_tests;
