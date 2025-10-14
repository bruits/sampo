use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(super) struct NpmAdapter;

impl NpmAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        root.join("package.json").exists() || root.join("pnpm-workspace.yaml").exists()
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        discover_npm(root)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        package_dir.join("package.json")
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        let value = load_package_json(manifest_path)?;
        if value
            .get("private")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            return Ok(false);
        }

        let has_name = value.get("name").and_then(JsonValue::as_str).is_some();
        let has_version = value
            .get("version")
            .and_then(JsonValue::as_str)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);

        Ok(has_name && has_version)
    }

    pub(super) fn version_exists(&self, _package_name: &str, _version: &str) -> Result<bool> {
        Ok(false)
    }

    pub(super) fn publish(
        &self,
        _manifest_path: &Path,
        _dry_run: bool,
        _extra_args: &[String],
    ) -> Result<()> {
        Err(SampoError::Publish(
            "npm package publishing is not implemented yet".to_string(),
        ))
    }

    pub(super) fn regenerate_lockfile(&self, _workspace_root: &Path) -> Result<()> {
        Ok(())
    }
}

/// Update an npm manifest (`package.json`) by bumping the package version (if provided) and
/// rewriting internal dependency specifiers when a new version is available.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    #[derive(Deserialize)]
    struct PackageJsonBorrowed<'a> {
        #[serde(borrow)]
        version: Option<&'a RawValue>,
        #[serde(borrow)]
        dependencies: Option<HashMap<String, &'a RawValue>>,
        #[serde(borrow, rename = "devDependencies")]
        dev_dependencies: Option<HashMap<String, &'a RawValue>>,
        #[serde(borrow, rename = "peerDependencies")]
        peer_dependencies: Option<HashMap<String, &'a RawValue>>,
        #[serde(borrow, rename = "optionalDependencies")]
        optional_dependencies: Option<HashMap<String, &'a RawValue>>,
    }

    let borrowed: PackageJsonBorrowed = serde_json::from_str(input).map_err(|err| {
        SampoError::Release(format!(
            "Failed to parse package.json {}: {err}",
            manifest_path.display()
        ))
    })?;

    struct Replacement {
        start: usize,
        end: usize,
        replacement: String,
    }

    let mut replacements: Vec<Replacement> = Vec::new();
    let mut applied: Vec<(String, String)> = Vec::new();

    if let Some(target_version) = new_pkg_version {
        let version_raw = borrowed.version.ok_or_else(|| {
            SampoError::Release(format!(
                "Manifest {} is missing a version field",
                manifest_path.display()
            ))
        })?;
        let current: String = serde_json::from_str(version_raw.get()).map_err(|err| {
            SampoError::Release(format!(
                "Version field in {} is not a string: {err}",
                manifest_path.display()
            ))
        })?;
        if current != target_version {
            let (start, end) = raw_span(version_raw, input);
            replacements.push(Replacement {
                start,
                end,
                replacement: format!("\"{target_version}\""),
            });
        }
    }

    let sections: [(&str, Option<&HashMap<String, &RawValue>>); 4] = [
        ("dependencies", borrowed.dependencies.as_ref()),
        ("devDependencies", borrowed.dev_dependencies.as_ref()),
        ("peerDependencies", borrowed.peer_dependencies.as_ref()),
        (
            "optionalDependencies",
            borrowed.optional_dependencies.as_ref(),
        ),
    ];

    for (dep_name, new_version) in new_version_by_name {
        let mut updated = false;

        for (section_name, maybe_map) in sections {
            let Some(map) = maybe_map else { continue };
            let Some(raw) = map.get(dep_name.as_str()) else {
                continue;
            };
            let current_spec: String = serde_json::from_str(raw.get()).map_err(|err| {
                SampoError::Release(format!(
                    "Dependency specifier for '{}' in {}.{} is not a string: {err}",
                    dep_name,
                    manifest_path.display(),
                    section_name
                ))
            })?;

            if let Some(new_spec) = compute_dependency_specifier(&current_spec, new_version)
                && new_spec != current_spec
            {
                let (start, end) = raw_span(raw, input);
                replacements.push(Replacement {
                    start,
                    end,
                    replacement: format!("\"{new_spec}\""),
                });
                updated = true;
            }
        }

        if updated {
            applied.push((dep_name.clone(), new_version.clone()));
        }
    }

    if replacements.is_empty() {
        return Ok((input.to_string(), applied));
    }

    replacements.sort_by(|a, b| a.start.cmp(&b.start));
    let mut output = input.to_string();
    for replacement in replacements.into_iter().rev() {
        output.replace_range(replacement.start..replacement.end, &replacement.replacement);
    }

    Ok((output, applied))
}

fn raw_span(raw: &RawValue, source: &str) -> (usize, usize) {
    let slice = raw.get();
    let start = unsafe { slice.as_ptr().offset_from(source.as_ptr()) };
    assert!(
        start >= 0,
        "raw JSON segment is not derived from the provided source"
    );
    let start = start as usize;
    assert!(
        start + slice.len() <= source.len(),
        "raw JSON segment exceeds source bounds"
    );
    let end = start + slice.len();
    (start, end)
}

fn compute_dependency_specifier(old_spec: &str, new_version: &str) -> Option<String> {
    let trimmed = old_spec.trim();
    if trimmed.is_empty() {
        return Some(new_version.to_string());
    }

    if let Some(suffix) = trimmed.strip_prefix("workspace:") {
        return match suffix {
            "*" => None,
            "^" => Some(format!("workspace:^{}", new_version)),
            "~" => Some(format!("workspace:~{}", new_version)),
            "" => Some(format!("workspace:{}", new_version)),
            _ if suffix.starts_with('^') => Some(format!("workspace:^{}", new_version)),
            _ if suffix.starts_with('~') => Some(format!("workspace:~{}", new_version)),
            _ => Some(format!("workspace:{}", new_version)),
        };
    }

    if trimmed == "*" {
        return None;
    }

    for prefix in ["file:", "link:", "npm:", "git:", "http:", "https:"] {
        if trimmed.starts_with(prefix) {
            return None;
        }
    }

    if let Some(rest) = trimmed.strip_prefix('^') {
        if rest == new_version {
            return None;
        }
        return Some(format!("^{}", new_version));
    }

    if let Some(rest) = trimmed.strip_prefix('~') {
        if rest == new_version {
            return None;
        }
        return Some(format!("~{}", new_version));
    }

    if trimmed == new_version {
        return None;
    }

    if trimmed.starts_with('>') || trimmed.starts_with('<') {
        return None;
    }

    Some(new_version.to_string())
}

fn discover_npm(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    let package_json_path = root.join("package.json");
    let root_manifest = if package_json_path.exists() {
        Some(load_package_json(&package_json_path)?)
    } else {
        None
    };

    let mut patterns: BTreeSet<String> = BTreeSet::new();

    if let Some(manifest) = &root_manifest {
        for pattern in extract_workspace_patterns(manifest)? {
            patterns.insert(pattern);
        }
    }

    let pnpm_patterns = load_pnpm_workspace_patterns(&root.join("pnpm-workspace.yaml"))?;
    for pattern in pnpm_patterns {
        patterns.insert(pattern);
    }

    let mut package_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    if patterns.is_empty() {
        if package_json_path.exists() {
            package_dirs.insert(root.to_path_buf());
        }
    } else {
        for pattern in patterns {
            expand_npm_member_pattern(root, &pattern, &mut package_dirs)?;
        }
    }

    if let Some(manifest) = &root_manifest
        && manifest
            .get("name")
            .and_then(JsonValue::as_str)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    {
        package_dirs.insert(root.to_path_buf());
    }

    let mut manifests: Vec<(String, String, PathBuf, JsonValue)> = Vec::new();
    let mut name_to_path: BTreeMap<String, PathBuf> = BTreeMap::new();

    for dir in &package_dirs {
        let manifest_path = dir.join("package.json");
        if !manifest_path.exists() {
            return Err(WorkspaceError::InvalidWorkspace(format!(
                "workspace member '{}' does not contain package.json",
                dir.display()
            )));
        }
        let manifest = load_package_json(&manifest_path)?;
        let name = manifest
            .get("name")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                WorkspaceError::InvalidWorkspace(format!(
                    "missing name field in {}",
                    manifest_path.display()
                ))
            })?
            .to_string();
        let version = manifest
            .get("version")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();

        manifests.push((name.clone(), version, dir.clone(), manifest));
        name_to_path.insert(name, dir.clone());
    }

    let mut packages = Vec::new();
    for (name, version, path, manifest) in manifests {
        let identifier = PackageInfo::dependency_identifier(PackageKind::Npm, &name);
        let internal_deps = collect_internal_deps(&manifest, &name_to_path);
        packages.push(PackageInfo {
            name,
            version,
            path,
            identifier,
            internal_deps,
            kind: PackageKind::Npm,
        });
    }

    Ok(packages)
}

fn load_package_json(path: &Path) -> std::result::Result<JsonValue, WorkspaceError> {
    let text = fs::read_to_string(path)
        .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, path)))?;
    serde_json::from_str(&text)
        .map_err(|e| WorkspaceError::InvalidManifest(format!("{}: {}", path.display(), e)))
}

fn extract_workspace_patterns(
    manifest: &JsonValue,
) -> std::result::Result<Vec<String>, WorkspaceError> {
    let mut patterns = Vec::new();
    if let Some(workspaces) = manifest.get("workspaces") {
        match workspaces {
            JsonValue::Array(items) => {
                for item in items {
                    let pattern = item.as_str().ok_or_else(|| {
                        WorkspaceError::InvalidWorkspace(
                            "workspaces entries must be strings".into(),
                        )
                    })?;
                    patterns.push(pattern.to_string());
                }
            }
            JsonValue::Object(map) => {
                if let Some(packages) = map.get("packages") {
                    if let JsonValue::Array(items) = packages {
                        for item in items {
                            let pattern = item.as_str().ok_or_else(|| {
                                WorkspaceError::InvalidWorkspace(
                                    "workspaces.packages entries must be strings".into(),
                                )
                            })?;
                            patterns.push(pattern.to_string());
                        }
                    } else if !packages.is_null() {
                        return Err(WorkspaceError::InvalidWorkspace(
                            "workspaces.packages must be an array of strings".into(),
                        ));
                    }
                }
            }
            _ => {
                return Err(WorkspaceError::InvalidWorkspace(
                    "workspaces field must be an array or object".into(),
                ));
            }
        }
    }
    Ok(patterns)
}

fn load_pnpm_workspace_patterns(path: &Path) -> std::result::Result<Vec<String>, WorkspaceError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = fs::read_to_string(path)
        .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, path)))?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text)
        .map_err(|e| WorkspaceError::InvalidManifest(format!("{}: {}", path.display(), e)))?;

    let mut patterns = Vec::new();
    if let Some(packages) = value.get("packages") {
        if let Some(seq) = packages.as_sequence() {
            for item in seq {
                let pattern = item.as_str().ok_or_else(|| {
                    WorkspaceError::InvalidWorkspace(
                        "pnpm-workspace.yaml packages entries must be strings".into(),
                    )
                })?;
                patterns.push(pattern.to_string());
            }
        } else if !packages.is_null() {
            return Err(WorkspaceError::InvalidWorkspace(
                "pnpm-workspace.yaml packages field must be a sequence of strings".into(),
            ));
        }
    }

    Ok(patterns)
}

fn expand_npm_member_pattern(
    root: &Path,
    pattern: &str,
    paths: &mut BTreeSet<PathBuf>,
) -> std::result::Result<(), WorkspaceError> {
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        let full_pattern = root.join(pattern);
        let pattern_str = full_pattern.to_string_lossy().to_string();
        let matches = glob::glob(&pattern_str).map_err(|e| {
            WorkspaceError::InvalidWorkspace(format!(
                "invalid workspace pattern '{}': {}",
                pattern, e
            ))
        })?;
        for entry in matches {
            let path = entry
                .map_err(|e| WorkspaceError::InvalidWorkspace(format!("glob error: {}", e)))?;
            if path.is_dir() {
                if path.join("package.json").exists() {
                    paths.insert(clean_path(&path));
                }
            } else if path
                .file_name()
                .map(|name| name == "package.json")
                .unwrap_or(false)
                && let Some(parent) = path.parent()
            {
                paths.insert(clean_path(parent));
            }
        }
    } else {
        let candidate = clean_path(&root.join(pattern));
        let manifest_path = candidate.join("package.json");
        if manifest_path.exists() {
            paths.insert(candidate);
        } else {
            return Err(WorkspaceError::InvalidWorkspace(format!(
                "workspace member '{}' does not contain package.json",
                pattern
            )));
        }
    }
    Ok(())
}

fn collect_internal_deps(
    manifest: &JsonValue,
    name_to_path: &BTreeMap<String, PathBuf>,
) -> BTreeSet<String> {
    let mut internal = BTreeSet::new();

    for key in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(deps) = manifest.get(key).and_then(JsonValue::as_object) {
            for dep_name in deps.keys() {
                if name_to_path.contains_key(dep_name.as_str()) {
                    internal.insert(PackageInfo::dependency_identifier(
                        PackageKind::Npm,
                        dep_name,
                    ));
                }
            }
        }
    }

    if let Some(array) = manifest
        .get("bundledDependencies")
        .or_else(|| manifest.get("bundleDependencies"))
        .and_then(JsonValue::as_array)
    {
        for dep in array {
            if let Some(dep_name) = dep.as_str()
                && name_to_path.contains_key(dep_name)
            {
                internal.insert(PackageInfo::dependency_identifier(
                    PackageKind::Npm,
                    dep_name,
                ));
            }
        }
    }

    internal
}

fn clean_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
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

#[cfg(test)]
mod npm_tests;
