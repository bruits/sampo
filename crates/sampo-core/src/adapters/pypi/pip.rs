use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use toml_edit::{DocumentMut, Item, Value};

const PYPROJECT_MANIFEST: &str = "pyproject.toml";

/// PEP 508 version comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionOperator {
    GreaterOrEqual,
    LessOrEqual,
    Equal,
    Compatible,
    NotEqual,
    Greater,
    Less,
}

impl VersionOperator {
    /// Returns the string representation of the operator.
    fn as_str(self) -> &'static str {
        match self {
            Self::GreaterOrEqual => ">=",
            Self::LessOrEqual => "<=",
            Self::Equal => "==",
            Self::Compatible => "~=",
            Self::NotEqual => "!=",
            Self::Greater => ">",
            Self::Less => "<",
        }
    }

    /// All operators in order of precedence for parsing.
    /// Two-character operators MUST come before single-character ones.
    const ALL: &'static [Self] = &[
        Self::GreaterOrEqual,
        Self::LessOrEqual,
        Self::Equal,
        Self::Compatible,
        Self::NotEqual,
        Self::Greater,
        Self::Less,
    ];
}

pub(super) fn can_discover(root: &Path) -> bool {
    root.join(PYPROJECT_MANIFEST).exists()
}

pub(super) fn discover(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    let manifest_path = root.join(PYPROJECT_MANIFEST);
    if !manifest_path.exists() {
        return Err(WorkspaceError::ManifestNotFound {
            manifest: PYPROJECT_MANIFEST,
            path: root.to_path_buf(),
        });
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
    let workspace_config = parse_uv_workspace_config(&manifest_text);
    if let Some(ref members) = workspace_config.members {
        for pattern in members {
            expand_uv_member_pattern(root, pattern, &mut package_dirs)?;
        }
    }

    // Apply exclude patterns
    if let Some(ref excludes) = workspace_config.exclude {
        let mut excluded_dirs: BTreeSet<PathBuf> = BTreeSet::new();
        for pattern in excludes {
            expand_uv_member_pattern(root, pattern, &mut excluded_dirs)?;
        }
        package_dirs.retain(|dir| !excluded_dirs.contains(dir));
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

/// Configuration parsed from [tool.uv.workspace]
#[derive(Default)]
struct UvWorkspaceConfig {
    members: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

/// Parse uv workspace configuration from [tool.uv.workspace]
fn parse_uv_workspace_config(source: &str) -> UvWorkspaceConfig {
    let Ok(doc) = source.parse::<DocumentMut>() else {
        return UvWorkspaceConfig::default();
    };

    let Some(workspace) = doc
        .get("tool")
        .and_then(|t| t.as_table())
        .and_then(|t| t.get("uv"))
        .and_then(|t| t.as_table())
        .and_then(|t| t.get("workspace"))
        .and_then(|t| t.as_table())
    else {
        return UvWorkspaceConfig::default();
    };

    let members = workspace
        .get("members")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty());

    let exclude = workspace
        .get("exclude")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty());

    UvWorkspaceConfig { members, exclude }
}

/// Expand a member pattern (plain path or glob) into concrete paths containing pyproject.toml
fn expand_uv_member_pattern(
    root: &Path,
    pattern: &str,
    paths: &mut BTreeSet<PathBuf>,
) -> std::result::Result<(), WorkspaceError> {
    if pattern.contains('*') {
        let full_pattern = root.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();
        let entries = glob::glob(&pattern_str).map_err(|e| {
            WorkspaceError::InvalidWorkspace(format!("invalid glob pattern '{}': {}", pattern, e))
        })?;
        for entry in entries {
            let path = entry
                .map_err(|e| WorkspaceError::InvalidWorkspace(format!("glob error: {}", e)))?;
            if path.is_dir() && path.join(PYPROJECT_MANIFEST).exists() {
                paths.insert(normalize_path(&path));
            }
        }
    } else {
        let member_path = normalize_path(&root.join(pattern));
        if member_path.join(PYPROJECT_MANIFEST).exists() {
            paths.insert(member_path);
        }
        // Unlike Cargo, uv silently ignores non-existent members
    }
    Ok(())
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

    if after_extras.is_empty() {
        return None;
    }

    let new_spec = VersionOperator::ALL.iter().find_map(|&op| {
        after_extras
            .strip_prefix(op.as_str())
            .and_then(|current| compute_new_spec(version_part, op, current.trim(), new_version))
    })?;

    let result = match markers {
        Some(m) => format!("{} {}", new_spec, m),
        None => new_spec,
    };

    Some((original_name.clone(), result))
}

/// Compute a new dependency spec by replacing only the version.
fn compute_new_spec(
    version_part: &str,
    operator: VersionOperator,
    current_version: &str,
    new_version: &str,
) -> Option<String> {
    if current_version == new_version {
        return None;
    }

    if !is_valid_version_token(current_version) {
        return None;
    }

    let op_str = operator.as_str();
    let op_start = version_part.find(op_str)?;
    let prefix = &version_part[..op_start];

    Some(format!("{}{}{}", prefix, op_str, new_version))
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

pub(super) fn find_dependency_constraint(source: &str, dep_name: &str) -> Result<Option<String>> {
    let doc: DocumentMut = source
        .parse()
        .map_err(|e| SampoError::Release(format!("Failed to parse pyproject.toml: {}", e)))?;
    let Some(project) = doc.get("project").and_then(Item::as_table) else {
        return Ok(None);
    };
    let target = normalize_package_name(dep_name);

    let find_in_array = |arr: &toml_edit::Array| -> Option<String> {
        for item in arr.iter() {
            let Some(spec) = item.as_str() else { continue };
            let Some(name) = extract_package_name(spec) else {
                continue;
            };
            if normalize_package_name(&name) != target {
                continue;
            }
            return Some(extract_constraint_from_spec(spec));
        }
        None
    };

    if let Some(deps) = project.get("dependencies").and_then(Item::as_array)
        && let Some(c) = find_in_array(deps)
    {
        return Ok(Some(c));
    }

    if let Some(optional) = project
        .get("optional-dependencies")
        .and_then(Item::as_table)
    {
        for (_, group) in optional.iter() {
            if let Some(arr) = group.as_array()
                && let Some(c) = find_in_array(arr)
            {
                return Ok(Some(c));
            }
        }
    }

    Ok(None)
}

fn extract_constraint_from_spec(spec: &str) -> String {
    let trimmed = spec.trim();

    let without_markers = match trimmed.find(';') {
        Some(pos) => trimmed[..pos].trim(),
        None => trimmed,
    };

    // URL dependencies have no parseable constraint
    if without_markers.contains(" @ ") {
        return String::new();
    }

    let after_extras = match (without_markers.find('['), without_markers.find(']')) {
        (Some(start), Some(end)) if start < end => &without_markers[end + 1..],
        _ => {
            let name_end = without_markers
                .find(|c: char| ['>', '<', '=', '!', '~'].contains(&c))
                .unwrap_or(without_markers.len());
            &without_markers[name_end..]
        }
    };

    after_extras.trim().to_string()
}

pub(super) fn check_pep440_constraint(
    constraint: &str,
    new_version: &str,
) -> Result<crate::types::ConstraintCheckResult> {
    use crate::types::ConstraintCheckResult;

    let trimmed = constraint.trim();

    if trimmed.is_empty() {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "empty constraint".to_string(),
        });
    }

    if trimmed == "*" {
        return Ok(ConstraintCheckResult::Satisfied);
    }

    if new_version.contains('-') {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pre-release version".to_string(),
        });
    }

    if pep440_constraint_contains_prerelease(trimmed) {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pre-release constraint".to_string(),
        });
    }

    if is_pep440_pinned_version(trimmed) {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pinned version".to_string(),
        });
    }

    let version = match parse_pep440_version(new_version) {
        Some(v) => v,
        None => {
            return Ok(ConstraintCheckResult::Skipped {
                reason: format!("unparseable version '{}'", new_version),
            });
        }
    };

    match pep440_version_satisfies(trimmed, version) {
        Some(true) => Ok(ConstraintCheckResult::Satisfied),
        Some(false) => Ok(ConstraintCheckResult::NotSatisfied {
            constraint: trimmed.to_string(),
            new_version: new_version.to_string(),
        }),
        None => Ok(ConstraintCheckResult::Skipped {
            reason: format!("unparseable constraint '{}'", trimmed),
        }),
    }
}

fn strip_post_release_suffix(s: &str) -> &str {
    let lower = s.to_ascii_lowercase();
    if let Some(pos) = lower.rfind(".post") {
        let after = &s[pos + 5..];
        if after.is_empty() || after.chars().all(|c| c.is_ascii_digit()) {
            return &s[..pos];
        }
    }
    s
}

fn parse_pep440_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().strip_prefix('v').unwrap_or(s.trim());
    if s.is_empty() {
        return None;
    }

    let lower = s.to_ascii_lowercase();
    for marker in &[".dev", "a", "b", "rc"] {
        if let Some(pos) = lower.find(marker) {
            // Only treat as pre-release if preceded by a digit (avoids matching
            // inside regular version segments like "abc")
            if pos > 0 && s.as_bytes()[pos - 1].is_ascii_digit() {
                return None;
            }
        }
    }

    // .postN is stable, not pre-release
    let s = strip_post_release_suffix(s);

    let parts: Vec<&str> = s.split('.').collect();
    match parts.len() {
        2 => Some((parts[0].parse().ok()?, parts[1].parse().ok()?, 0)),
        3 => Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        )),
        _ => None,
    }
}

fn pep440_constraint_contains_prerelease(constraint: &str) -> bool {
    let lower = constraint.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for marker in &["dev", "rc"] {
        if let Some(pos) = lower.find(marker)
            && (pos > 0 && bytes[pos - 1].is_ascii_digit()
                || (pos > 0 && bytes[pos - 1] == b'.')
                    && pos > 1
                    && bytes[pos - 2].is_ascii_digit())
        {
            return true;
        }
    }
    // Single-letter 'a'/'b' pre-release tags (e.g. `1.0a1`, `2.0b3`)
    for i in 1..bytes.len() {
        if (bytes[i] == b'a' || bytes[i] == b'b')
            && bytes[i - 1].is_ascii_digit()
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
        {
            return true;
        }
    }
    false
}

/// A pinned version is a bare version number with no operator, wildcard, or comma.
fn is_pep440_pinned_version(s: &str) -> bool {
    let s = s.trim();
    for op in VersionOperator::ALL {
        if s.starts_with(op.as_str()) {
            return false;
        }
    }
    !s.contains(',') && !s.contains('*') && parse_pep440_version(s).is_some()
}

/// Returns `None` if the constraint is unparseable.
fn pep440_version_satisfies(constraint: &str, version: (u64, u64, u64)) -> Option<bool> {
    for part in constraint.split(',') {
        match satisfies_single_pep440_specifier(part.trim(), version) {
            Some(true) => continue,
            Some(false) => return Some(false),
            None => return None,
        }
    }
    Some(true)
}

fn satisfies_single_pep440_specifier(spec: &str, version: (u64, u64, u64)) -> Option<bool> {
    let spec = spec.trim();
    if spec.is_empty() || spec == "*" {
        return Some(true);
    }

    // ~=
    if let Some(rest) = spec.strip_prefix("~=") {
        let rest = rest.trim();
        let parts_count = rest.split('.').count();
        let parsed = parse_pep440_version(rest)?;
        let (lower, upper) = expand_compatible_release(parsed, parts_count);
        return Some(version >= lower && version < upper);
    }

    // ==
    if let Some(rest) = spec.strip_prefix("==") {
        let rest = rest.trim();
        if rest.ends_with(".*") {
            return Some(matches_pep440_wildcard(rest, version));
        }
        let parsed = parse_pep440_version(rest)?;
        return Some(version == parsed);
    }

    // !=
    if let Some(rest) = spec.strip_prefix("!=") {
        let rest = rest.trim();
        if rest.ends_with(".*") {
            return Some(!matches_pep440_wildcard(rest, version));
        }
        let parsed = parse_pep440_version(rest)?;
        return Some(version != parsed);
    }

    // >=
    if let Some(rest) = spec.strip_prefix(">=") {
        let parsed = parse_pep440_version(rest.trim())?;
        return Some(version >= parsed);
    }

    // <=
    if let Some(rest) = spec.strip_prefix("<=") {
        let parsed = parse_pep440_version(rest.trim())?;
        return Some(version <= parsed);
    }

    // >
    if let Some(rest) = spec.strip_prefix('>') {
        let parsed = parse_pep440_version(rest.trim())?;
        return Some(version > parsed);
    }

    // <
    if let Some(rest) = spec.strip_prefix('<') {
        let parsed = parse_pep440_version(rest.trim())?;
        return Some(version < parsed);
    }

    // Bare version — exact match
    let parsed = parse_pep440_version(spec)?;
    Some(version == parsed)
}

/// Expand a PEP 440 compatible release (`~=`) to inclusive lower and exclusive upper bounds.
///
/// - `~=1.4.2` (3 parts) → `[1.4.2, 1.5.0)`
/// - `~=1.4`   (2 parts) → `[1.4.0, 2.0.0)`
fn expand_compatible_release(
    v: (u64, u64, u64),
    parts_count: usize,
) -> ((u64, u64, u64), (u64, u64, u64)) {
    let lower = v;
    let upper = if parts_count >= 3 {
        (v.0, v.1 + 1, 0)
    } else {
        (v.0 + 1, 0, 0)
    };
    (lower, upper)
}

fn matches_pep440_wildcard(pattern: &str, version: (u64, u64, u64)) -> bool {
    let stripped = pattern.trim_end_matches(".*");
    let parts: Vec<&str> = stripped.split('.').collect();
    match parts.len() {
        1 => parts[0].parse::<u64>().is_ok_and(|maj| version.0 == maj),
        2 => {
            parts[0].parse::<u64>().is_ok_and(|maj| version.0 == maj)
                && parts[1].parse::<u64>().is_ok_and(|min| version.1 == min)
        }
        _ => false,
    }
}

#[cfg(test)]
mod pip_tests;
