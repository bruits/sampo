use crate::errors::{Result, SampoError};
use crate::types::{PackageKind, Workspace};
use cargo_metadata::{DependencyKind, MetadataCommand};
use rustc_hash::FxHashSet;
use semver::{Version, VersionReq};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

/// Metadata extracted from `cargo_metadata` for the workspace.
#[derive(Debug, Clone)]
pub struct ManifestMetadata {
    packages: Vec<PackageInfo>,
    by_manifest: HashMap<PathBuf, usize>,
    by_name: HashMap<String, usize>,
}

#[derive(Debug, Clone)]
struct PackageInfo {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    manifest_path: PathBuf,
    dependencies: Vec<DependencyInfo>,
}

#[derive(Debug, Clone)]
struct DependencyInfo {
    manifest_key: String,
    package_name: String,
    kind: DependencyKind,
    target: Option<String>,
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
                .map(|dep| DependencyInfo {
                    manifest_key: dep.rename.clone().unwrap_or_else(|| dep.name.clone()),
                    package_name: dep.name.clone(),
                    kind: dep.kind,
                    target: dep.target.as_ref().map(|platform| platform.to_string()),
                })
                .collect();

            let idx = packages.len();
            by_manifest.insert(manifest_path.clone(), idx);
            by_name.insert(package.name.clone(), idx);
            packages.push(PackageInfo {
                name: package.name,
                manifest_path,
                dependencies,
            });
        }

        Ok(Self {
            packages,
            by_manifest,
            by_name,
        })
    }

    fn package_for_manifest(&self, manifest_path: &Path) -> Option<&PackageInfo> {
        self.by_manifest
            .get(manifest_path)
            .and_then(|idx| self.packages.get(*idx))
    }

    fn is_workspace_package(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }
}

/// Update a manifest for the given ecosystem, setting the package version (if provided) and
/// retargeting internal dependency requirements to the latest planned versions.
/// Returns the updated manifest contents along with a list of (dep_name, new_version) applied.
pub fn update_manifest_versions(
    kind: PackageKind,
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
    metadata: Option<&ManifestMetadata>,
) -> Result<(String, Vec<(String, String)>)> {
    match kind {
        PackageKind::Cargo => update_cargo_manifest_versions(
            manifest_path,
            input,
            new_pkg_version,
            new_version_by_name,
            metadata,
        ),
        PackageKind::Npm => {
            update_package_json_versions(manifest_path, input, new_pkg_version, new_version_by_name)
        }
    }
}

fn update_cargo_manifest_versions(
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

fn update_package_json_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    #[derive(Deserialize)]
    struct PackageJsonBorrowed<'a> {
        #[serde(borrow)]
        version: Option<&'a RawValue>,
        #[serde(borrow, rename = "dependencies")]
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
    package: &PackageInfo,
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

fn dependency_section_name(kind: DependencyKind) -> &'static str {
    match kind {
        DependencyKind::Normal | DependencyKind::Unknown => "dependencies",
        DependencyKind::Development => "dev-dependencies",
        DependencyKind::Build => "build-dependencies",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    #[test]
    fn skips_workspace_dependencies_when_updating() {
        let input = "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n\n[dependencies]\nfoo = { workspace = true, optional = true }\n";
        let mut updates = BTreeMap::new();
        updates.insert("foo".to_string(), "1.2.3".to_string());

        let (out, applied) = update_manifest_versions(
            PackageKind::Cargo,
            Path::new("/demo/Cargo.toml"),
            input,
            None,
            &updates,
            None,
        )
        .unwrap();

        assert_eq!(out.trim_end(), input.trim_end());
        assert!(applied.is_empty());
    }

    #[test]
    fn updates_workspace_dependency_with_explicit_version() {
        let input = "[workspace.dependencies]\nfoo = { version = \"0.1.0\", path = \"foo\" }\n";
        let mut updates = BTreeMap::new();
        updates.insert("foo".to_string(), "0.2.0".to_string());

        let (out, applied) = update_manifest_versions(
            PackageKind::Cargo,
            Path::new("/workspace/Cargo.toml"),
            input,
            None,
            &updates,
            None,
        )
        .unwrap();

        assert!(applied.contains(&("foo".to_string(), "0.2.0".to_string())));
        assert!(out.contains("version = \"0.2.0\""));
    }

    #[test]
    fn keeps_workspace_dependency_shorthand_for_patch_bump() {
        assert!(compute_workspace_dependency_version("0.1", "0.1.14").is_none());
    }

    #[test]
    fn updates_workspace_dependency_shorthand_for_minor_bump() {
        let resolved = compute_workspace_dependency_version("0.1", "0.2.0")
            .expect("minor bump should rewrite shorthand");
        assert_eq!(resolved, "0.2");
    }

    #[test]
    fn updates_workspace_dependency_major_shorthand() {
        let resolved = compute_workspace_dependency_version("1", "2.0.0")
            .expect("major bump should rewrite shorthand");
        assert_eq!(resolved, "2");
    }

    #[test]
    fn skips_workspace_dependency_with_wildcard_version() {
        assert!(compute_workspace_dependency_version("*", "0.2.0").is_none());
    }

    #[test]
    fn skips_workspace_dependency_without_version() {
        let input = "[workspace.dependencies]\nfoo = { path = \"foo\" }\n";
        let mut updates = BTreeMap::new();
        updates.insert("foo".to_string(), "0.2.0".to_string());

        let (out, applied) = update_manifest_versions(
            PackageKind::Cargo,
            Path::new("/workspace/Cargo.toml"),
            input,
            None,
            &updates,
            None,
        )
        .unwrap();

        assert_eq!(out.trim_end(), input.trim_end());
        assert!(applied.is_empty());
    }

    #[test]
    fn converts_simple_dep_without_quotes() {
        let input =
            "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n\n[dependencies]\nbar = \"0.1.0\"\n";
        let mut updates = BTreeMap::new();
        updates.insert("bar".to_string(), "0.2.0".to_string());

        let (out, applied) = update_manifest_versions(
            PackageKind::Cargo,
            Path::new("/demo/Cargo.toml"),
            input,
            None,
            &updates,
            None,
        )
        .unwrap();

        assert!(applied.contains(&("bar".to_string(), "0.2.0".to_string())));
        assert!(out.contains("bar = \"0.2.0\""));
    }

    #[test]
    fn updates_package_json_versions_preserving_formatting() {
        let input = r#"{
  "name": "app",
  "version": "1.0.0",
  "dependencies": {
    "pkg-a": "^1.0.0",
    "pkg-b": "workspace:*",
    "pkg-c": "file:../pkg-c",
    "pkg-d": "workspace:^1.0.0"
  },
  "devDependencies": {
    "pkg-a": "~1.0.0"
  }
}
"#;
        let mut updates = BTreeMap::new();
        updates.insert("pkg-a".to_string(), "2.0.0".to_string());
        updates.insert("pkg-b".to_string(), "3.0.0".to_string());
        updates.insert("pkg-c".to_string(), "4.0.0".to_string());
        updates.insert("pkg-d".to_string(), "1.5.0".to_string());

        let (out, applied) = update_manifest_versions(
            PackageKind::Npm,
            Path::new("/repo/package.json"),
            input,
            Some("1.1.0"),
            &updates,
            None,
        )
        .unwrap();

        assert!(out.contains("\"version\": \"1.1.0\""));
        assert!(out.contains("\"pkg-a\": \"^2.0.0\""));
        assert!(out.contains("\"pkg-b\": \"workspace:*\""));
        assert!(out.contains("\"pkg-c\": \"file:../pkg-c\""));
        assert!(out.contains("\"pkg-d\": \"workspace:^1.5.0\""));
        assert!(out.contains("\"pkg-a\": \"~2.0.0\""));
        assert!(applied.contains(&("pkg-a".to_string(), "2.0.0".to_string())));
        assert!(applied.contains(&("pkg-d".to_string(), "1.5.0".to_string())));
        assert!(!applied.iter().any(|(name, _)| name == "pkg-b"));
        assert!(!applied.iter().any(|(name, _)| name == "pkg-c"));
    }
}
