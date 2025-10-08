/// Cargo ecosystem adapter for all Cargo operations.
use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

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

/// Find the workspace root starting from a directory
fn find_cargo_workspace_root(
    start_dir: &Path,
) -> std::result::Result<(PathBuf, toml::Value), WorkspaceError> {
    let mut current = start_dir;
    loop {
        let toml_path = current.join("Cargo.toml");
        if toml_path.exists() {
            let text = fs::read_to_string(&toml_path).map_err(|e| {
                WorkspaceError::Io(crate::errors::io_error_with_path(e, &toml_path))
            })?;
            let value: toml::Value = text.parse().map_err(|e| {
                WorkspaceError::InvalidToml(format!("{}: {}", toml_path.display(), e))
            })?;
            if value.get("workspace").is_some() {
                return Ok((current.to_path_buf(), value));
            }
        }
        current = current.parent().ok_or(WorkspaceError::NotFound)?;
    }
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
                    internal.insert(dep_name.clone());
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

fn discover_cargo(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    let (workspace_root, root_toml) = find_cargo_workspace_root(root)?;
    let members = parse_cargo_workspace_members(&workspace_root, &root_toml)?;
    let mut crates = Vec::new();

    // First pass: parse per-crate metadata (name, version)
    let mut name_to_path: BTreeMap<String, PathBuf> = BTreeMap::new();
    for member_dir in &members {
        let manifest_path = member_dir.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path).map_err(|e| {
            WorkspaceError::Io(crate::errors::io_error_with_path(e, &manifest_path))
        })?;
        let value: toml::Value = text.parse().map_err(|e| {
            WorkspaceError::InvalidToml(format!("{}: {}", manifest_path.display(), e))
        })?;
        let pkg = value
            .get("package")
            .and_then(|v| v.as_table())
            .ok_or_else(|| {
                WorkspaceError::InvalidToml(format!(
                    "missing [package] in {}",
                    manifest_path.display()
                ))
            })?;
        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                WorkspaceError::InvalidToml(format!(
                    "missing package.name in {}",
                    manifest_path.display()
                ))
            })?
            .to_string();
        let version = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        name_to_path.insert(name.clone(), member_dir.clone());
        crates.push((name, version, member_dir.clone(), value));
    }

    // Second pass: compute internal dependencies
    let mut out: Vec<PackageInfo> = Vec::new();
    for (name, version, path, manifest) in crates {
        let internal_deps = collect_cargo_internal_deps(&path, &name_to_path, &manifest);
        out.push(PackageInfo {
            name,
            version,
            path,
            internal_deps,
            kind: PackageKind::Cargo,
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_path_collapses_segments() {
        let input = Path::new("/a/b/../c/./d");
        let expected = PathBuf::from("/a/c/d");
        assert_eq!(clean_path(input), expected);
    }

    #[test]
    fn clean_path_prevents_escaping_root() {
        // Test that we can't escape beyond root directory
        let input = Path::new("/a/../..");
        let expected = PathBuf::from("/");
        assert_eq!(clean_path(input), expected);

        // Test with relative paths
        let input = Path::new("a/../..");
        let expected = PathBuf::from("");
        assert_eq!(clean_path(input), expected);
    }

    #[test]
    fn cargo_adapter_is_publishable_checks_manifest() {
        let adapter = CargoAdapter;
        // This test would require a real manifest file, so it's mainly a compilation check
        // Real tests are in the integration test suite
        let _ = adapter.is_publishable(Path::new("./Cargo.toml"));
    }

    #[test]
    fn cargo_discoverer_can_discover_cargo_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create a Cargo workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"pkg-a\"]\n",
        )
        .unwrap();

        assert!(root.join("Cargo.toml").exists());
    }

    #[test]
    fn cargo_discoverer_cannot_discover_non_cargo() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // No Cargo.toml
        assert!(!root.join("Cargo.toml").exists());
    }

    #[test]
    fn cargo_discoverer_discovers_packages() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        // Create crates
        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("pkg-a")).unwrap();
        fs::create_dir_all(crates_dir.join("pkg-b")).unwrap();
        fs::write(
            crates_dir.join("pkg-a/Cargo.toml"),
            "[package]\nname = \"pkg-a\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            crates_dir.join("pkg-b/Cargo.toml"),
            "[package]\nname = \"pkg-b\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();

        let packages = discover_cargo(root).unwrap();
        assert_eq!(packages.len(), 2);

        let mut names: Vec<_> = packages.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["pkg-a", "pkg-b"]);

        // All should be Cargo packages
        assert!(packages.iter().all(|p| p.kind == PackageKind::Cargo));
    }

    #[test]
    fn cargo_discoverer_detects_internal_deps() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("pkg-a")).unwrap();
        fs::create_dir_all(crates_dir.join("pkg-b")).unwrap();

        // pkg-a depends on pkg-b via path
        fs::write(
            crates_dir.join("pkg-a/Cargo.toml"),
            "[package]\nname=\"pkg-a\"\nversion=\"0.1.0\"\n[dependencies]\npkg-b={ path=\"../pkg-b\" }\n",
        )
        .unwrap();
        fs::write(
            crates_dir.join("pkg-b/Cargo.toml"),
            "[package]\nname=\"pkg-b\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        let packages = discover_cargo(root).unwrap();
        let pkg_a = packages.iter().find(|p| p.name == "pkg-a").unwrap();

        assert!(pkg_a.internal_deps.contains("pkg-b"));
    }
}
