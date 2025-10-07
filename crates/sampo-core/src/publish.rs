use crate::types::PackageInfo;
use crate::{
    Config, current_branch, discover_workspace,
    errors::{Result, SampoError},
    filters::should_ignore_package,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Publishes all publishable crates in a workspace to crates.io in dependency order.
///
/// This function discovers all crates in the workspace, determines which ones are
/// publishable to crates.io, validates their dependencies, and publishes them in
/// topological order (dependencies first).
///
/// # Arguments
/// * `root` - Path to the workspace root directory
/// * `dry_run` - If true, performs validation and shows what would be published without actually publishing
/// * `cargo_args` - Additional arguments to pass to `cargo publish`
///
/// # Examples
/// ```no_run
/// use std::path::Path;
/// use sampo_core::run_publish;
///
/// // Dry run to see what would be published
/// run_publish(Path::new("."), true, &[]).unwrap();
///
/// // Actual publish with custom cargo args
/// run_publish(Path::new("."), false, &["--allow-dirty".to_string()]).unwrap();
/// ```
pub fn run_publish(root: &std::path::Path, dry_run: bool, cargo_args: &[String]) -> Result<()> {
    let ws = discover_workspace(root)?;
    let config = Config::load(&ws.root)?;

    let branch = current_branch()?;
    if !config.is_release_branch(&branch) {
        return Err(SampoError::Release(format!(
            "Branch '{}' is not configured for publishing (allowed: {:?})",
            branch,
            config.release_branches().into_iter().collect::<Vec<_>>()
        )));
    }

    // Determine which packages are publishable to crates.io and not ignored
    let mut name_to_package: BTreeMap<String, &PackageInfo> = BTreeMap::new();
    let mut publishable: BTreeSet<String> = BTreeSet::new();
    for c in &ws.members {
        // Skip ignored packages
        if should_ignore_package(&config, &ws, c)? {
            continue;
        }

        let manifest = c.path.join("Cargo.toml");
        if is_publishable_to_crates_io(&manifest)? {
            publishable.insert(c.name.clone());
            name_to_package.insert(c.name.clone(), c);
        }
    }

    if publishable.is_empty() {
        println!("No publishable crates for crates.io were found in the workspace.");
        return Ok(());
    }

    // Validate internal deps do not include non-publishable crates
    let mut errors: Vec<String> = Vec::new();
    for name in &publishable {
        let c = name_to_package.get(name).ok_or_else(|| {
            SampoError::Publish(format!(
                "internal error: crate '{}' not found in workspace",
                name
            ))
        })?;
        for dep in &c.internal_deps {
            if !publishable.contains(dep) {
                errors.push(format!(
                    "crate '{}' depends on internal crate '{}' which is not publishable",
                    name, dep
                ));
            }
        }
    }
    if !errors.is_empty() {
        for e in errors {
            eprintln!("{e}");
        }
        return Err(SampoError::Publish(
            "cannot publish due to non-publishable internal dependencies".into(),
        ));
    }

    // Compute publish order (topological: deps first) for all publishable crates.
    let order = topo_order(&name_to_package, &publishable)?;

    println!("Publish plan (crates.io):");
    for name in &order {
        println!("  - {name}");
    }

    // Execute cargo publish in order
    for name in &order {
        let c = name_to_package.get(name).ok_or_else(|| {
            SampoError::Publish(format!(
                "internal error: crate '{}' not found in workspace",
                name
            ))
        })?;
        let manifest = c.path.join("Cargo.toml");
        // Skip if the exact version already exists on crates.io
        match version_exists_on_crates_io(&c.name, &c.version) {
            Ok(true) => {
                println!(
                    "Skipping {}@{} (already exists on crates.io)",
                    c.name, c.version
                );
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!(
                    "Warning: could not check crates.io for {}@{}: {}. Attempting publish…",
                    c.name, c.version, e
                );
            }
        }

        let mut cmd = Command::new("cargo");
        cmd.arg("publish").arg("--manifest-path").arg(&manifest);
        if dry_run {
            cmd.arg("--dry-run");
        }
        if !cargo_args.is_empty() {
            cmd.args(cargo_args);
        }

        println!(
            "Running: {}",
            format_command_display(cmd.get_program(), cmd.get_args())
        );

        let status = cmd.status()?;
        if !status.success() {
            return Err(SampoError::Publish(format!(
                "cargo publish failed for crate '{}' with status {}",
                name, status
            )));
        }

        // Create an annotated git tag after successful publish (not in dry-run)
        if !dry_run && let Err(e) = tag_published_crate(&ws.root, &c.name, &c.version) {
            eprintln!(
                "Warning: failed to create tag for {}@{}: {}",
                c.name, c.version, e
            );
        }
    }

    if dry_run {
        println!("Dry-run complete.");
    } else {
        println!("Publish complete.");
    }

    Ok(())
}

/// Determines if a crate is publishable to crates.io based on its Cargo.toml manifest.
///
/// Checks the `publish` field in the `[package]` section according to Cargo's rules:
/// - No `publish` field: publishable (default)
/// - `publish = false`: not publishable
/// - `publish = ["registry1", "registry2"]`: publishable only if "crates-io" is in the array
///
/// # Arguments
/// * `manifest_path` - Path to the Cargo.toml file to check
///
/// # Examples
/// ```no_run
/// use std::path::Path;
/// use sampo_core::is_publishable_to_crates_io;
///
/// // Check if a crate is publishable
/// let publishable = is_publishable_to_crates_io(Path::new("./Cargo.toml")).unwrap();
/// if publishable {
///     println!("This crate can be published to crates.io");
/// }
/// ```
///
/// # Errors
/// Returns an error if:
/// - The manifest file cannot be read
/// - The TOML is malformed
/// - The manifest has no `[package]` section (returns `Ok(false)`)
pub fn is_publishable_to_crates_io(manifest_path: &Path) -> Result<bool> {
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

/// Creates an annotated git tag for a published crate.
///
/// Creates a tag in the format `{crate_name}-v{version}` (e.g., "my-crate-v1.2.3")
/// with a descriptive message. Skips tagging if not in a git repository or if
/// the tag already exists.
///
/// # Arguments
/// * `repo_root` - Path to the git repository root
/// * `crate_name` - Name of the crate that was published
/// * `version` - Version that was published
///
/// # Examples
/// ```no_run
/// use std::path::Path;
/// use sampo_core::tag_published_crate;
///
/// // Tag a published crate
/// tag_published_crate(Path::new("."), "my-crate", "1.2.3").unwrap();
/// // Creates tag: "my-crate-v1.2.3" with message "Release my-crate 1.2.3"
/// ```
pub fn tag_published_crate(repo_root: &Path, crate_name: &str, version: &str) -> Result<()> {
    if !repo_root.join(".git").exists() {
        // Not a git repo, skip
        return Ok(());
    }
    let tag = format!("{}-v{}", crate_name, version);
    // If tag already exists, do not recreate
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("tag")
        .arg("--list")
        .arg(&tag)
        .output()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout);
        if s.lines().any(|l| l.trim() == tag) {
            return Ok(());
        }
    }

    let msg = format!("Release {} {}", crate_name, version);
    let status = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("tag")
        .arg("-a")
        .arg(&tag)
        .arg("-m")
        .arg(&msg)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(SampoError::Publish(format!(
            "git tag failed with status {}",
            status
        )))
    }
}

/// Checks if a specific version of a crate already exists on crates.io.
///
/// Makes an HTTP request to the crates.io API to determine if the exact
/// version is already published. Useful for skipping redundant publishes.
///
/// # Arguments
/// * `crate_name` - Name of the crate to check
/// * `version` - Exact version string to check
///
/// # Examples
/// ```no_run
/// use sampo_core::version_exists_on_crates_io;
///
/// // Check if serde 1.0.0 exists (it does)
/// let exists = version_exists_on_crates_io("serde", "1.0.0").unwrap();
/// assert!(exists);
///
/// // Check if a fictional version exists
/// let exists = version_exists_on_crates_io("serde", "999.999.999").unwrap();
/// assert!(!exists);
/// ```
pub fn version_exists_on_crates_io(crate_name: &str, version: &str) -> Result<bool> {
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

/// Computes topological ordering for publishing crates (dependencies first).
///
/// Given a set of crates and their internal dependencies, returns the order
/// in which they should be published so that dependencies are always published
/// before the crates that depend on them.
///
/// # Arguments
/// * `name_to_package` - Map from package names to their info
/// * `include` - Set of package names to include in the ordering
///
/// # Examples
/// ```no_run
/// use std::collections::{BTreeMap, BTreeSet};
/// use sampo_core::{topo_order, types::PackageInfo};
/// use std::path::PathBuf;
///
/// let mut packages = BTreeMap::new();
/// let mut include = BTreeSet::new();
///
/// // Setup packages: foundation -> middleware -> app
/// // ... (create PackageInfo instances) ...
///
/// let order = topo_order(&packages, &include).unwrap();
/// // Returns: ["foundation", "middleware", "app"]
/// ```
pub fn topo_order(
    name_to_package: &BTreeMap<String, &PackageInfo>,
    include: &BTreeSet<String>,
) -> Result<Vec<String>> {
    // Build graph: edge dep -> crate
    let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut forward: BTreeMap<&str, Vec<&str>> = BTreeMap::new();

    for name in include {
        indegree.insert(name.as_str(), 0);
        forward.entry(name.as_str()).or_default();
    }

    for name in include {
        let c = name_to_package
            .get(name)
            .ok_or_else(|| SampoError::Publish(format!("missing package info for '{}'", name)))?;
        for dep in &c.internal_deps {
            if include.contains(dep) {
                // dep -> name
                let entry = forward.entry(dep.as_str()).or_default();
                entry.push(name.as_str());
                *indegree.get_mut(name.as_str()).unwrap() += 1;
            }
        }
    }

    let mut q: VecDeque<&str> = indegree
        .iter()
        .filter_map(|(k, &d)| if d == 0 { Some(*k) } else { None })
        .collect();
    let mut out: Vec<String> = Vec::new();

    while let Some(n) = q.pop_front() {
        out.push(n.to_string());
        if let Some(children) = forward.get(n) {
            for &m in children {
                if let Some(d) = indegree.get_mut(m) {
                    *d -= 1;
                    if *d == 0 {
                        q.push_back(m);
                    }
                }
            }
        }
    }

    if out.len() != include.len() {
        return Err(SampoError::Publish(
            "dependency cycle detected among publishable crates".into(),
        ));
    }
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PackageInfo, PackageKind, Workspace};
    use rustc_hash::FxHashMap;
    use std::{
        fs,
        path::PathBuf,
        sync::{Mutex, MutexGuard, OnceLock},
    };

    /// Test workspace builder for publish testing
    struct TestWorkspace {
        root: PathBuf,
        _temp_dir: tempfile::TempDir,
        crates: FxHashMap<String, PathBuf>,
    }

    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_MUTEX.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let lock = env_lock().lock().unwrap();
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key,
                original,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    impl TestWorkspace {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().unwrap();
            let root = temp_dir.path().to_path_buf();

            {
                let _lock = env_lock().lock().unwrap();
                unsafe {
                    std::env::set_var("SAMPO_RELEASE_BRANCH", "main");
                }
            }

            // Create basic workspace structure
            fs::write(
                root.join("Cargo.toml"),
                "[workspace]\nmembers=[\"crates/*\"]\n",
            )
            .unwrap();

            Self {
                root,
                _temp_dir: temp_dir,
                crates: FxHashMap::default(),
            }
        }

        fn add_crate(&mut self, name: &str, version: &str) -> &mut Self {
            let crate_dir = self.root.join("crates").join(name);
            fs::create_dir_all(&crate_dir).unwrap();

            fs::write(
                crate_dir.join("Cargo.toml"),
                format!("[package]\nname=\"{}\"\nversion=\"{}\"\n", name, version),
            )
            .unwrap();

            // Create minimal src/lib.rs so cargo can build the crate
            fs::create_dir_all(crate_dir.join("src")).unwrap();
            fs::write(crate_dir.join("src/lib.rs"), "// test crate").unwrap();

            self.crates.insert(name.to_string(), crate_dir);
            self
        }

        fn add_dependency(&mut self, from: &str, to: &str, version: &str) -> &mut Self {
            let from_dir = self.crates.get(from).expect("from crate must exist");
            let current_manifest = fs::read_to_string(from_dir.join("Cargo.toml")).unwrap();

            let dependency_section = format!(
                "\n[dependencies]\n{} = {{ path=\"../{}\", version=\"{}\" }}\n",
                to, to, version
            );

            fs::write(
                from_dir.join("Cargo.toml"),
                current_manifest + &dependency_section,
            )
            .unwrap();

            self
        }

        fn set_publishable(&self, crate_name: &str, publishable: bool) -> &Self {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            let manifest_path = crate_dir.join("Cargo.toml");
            let current_manifest = fs::read_to_string(&manifest_path).unwrap();

            let new_manifest = if publishable {
                current_manifest
            } else {
                current_manifest + "\npublish = false\n"
            };

            fs::write(manifest_path, new_manifest).unwrap();
            self
        }

        fn set_config(&self, content: &str) -> &Self {
            fs::create_dir_all(self.root.join(".sampo")).unwrap();
            fs::write(self.root.join(".sampo/config.toml"), content).unwrap();
            self
        }

        fn run_publish(&self, dry_run: bool) -> Result<()> {
            run_publish(&self.root, dry_run, &[])
        }

        fn assert_publishable_crates(&self, expected: &[&str]) {
            let ws = discover_workspace(&self.root).unwrap();
            let mut actual_publishable = Vec::new();

            for c in &ws.members {
                let manifest = c.path.join("Cargo.toml");
                if is_publishable_to_crates_io(&manifest).unwrap() {
                    actual_publishable.push(c.name.clone());
                }
            }

            actual_publishable.sort();
            let mut expected_sorted: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
            expected_sorted.sort();

            assert_eq!(actual_publishable, expected_sorted);
        }
    }

    #[test]
    fn run_publish_rejects_unconfigured_branch() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("foo", "0.1.0");
        workspace.set_publishable("foo", false);
        workspace.set_config("[git]\nrelease_branches = [\"main\"]\n");

        let _guard = EnvVarGuard::set("SAMPO_RELEASE_BRANCH", "feature");
        let branch = current_branch().expect("branch should be readable");
        assert_eq!(branch, "feature");
        let err = workspace.run_publish(true).unwrap_err();
        match err {
            SampoError::Release(message) => {
                assert!(
                    message.contains("not configured for publishing"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Release error, got {other:?}"),
        }
    }

    #[test]
    fn run_publish_allows_configured_branch() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("foo", "0.1.0");
        workspace.set_publishable("foo", false);
        workspace.set_config("[git]\nrelease_branches = [\"3.x\"]\n");

        let _guard = EnvVarGuard::set("SAMPO_RELEASE_BRANCH", "3.x");
        workspace
            .run_publish(true)
            .expect("publish should succeed on configured branch");
    }

    #[test]
    fn topo_orders_deps_first() {
        // Build a small fake graph using PackageInfo structures
        let a = PackageInfo {
            name: "a".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/a"),
            internal_deps: BTreeSet::new(),
            kind: PackageKind::Cargo,
        };
        let mut deps_b = BTreeSet::new();
        deps_b.insert("a".into());
        let b = PackageInfo {
            name: "b".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/b"),
            internal_deps: deps_b,
            kind: PackageKind::Cargo,
        };
        let mut deps_c = BTreeSet::new();
        deps_c.insert("b".into());
        let c = PackageInfo {
            name: "c".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/c"),
            internal_deps: deps_c,
            kind: PackageKind::Cargo,
        };

        let mut map: BTreeMap<String, &PackageInfo> = BTreeMap::new();
        map.insert("a".into(), &a);
        map.insert("b".into(), &b);
        map.insert("c".into(), &c);

        let mut include = BTreeSet::new();
        include.insert("a".into());
        include.insert("b".into());
        include.insert("c".into());

        let order = topo_order(&map, &include).unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn detects_dependency_cycle() {
        // Create a circular dependency: a -> b -> a
        let mut deps_a = BTreeSet::new();
        deps_a.insert("b".into());
        let a = PackageInfo {
            name: "a".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/a"),
            internal_deps: deps_a,
            kind: PackageKind::Cargo,
        };

        let mut deps_b = BTreeSet::new();
        deps_b.insert("a".into());
        let b = PackageInfo {
            name: "b".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/b"),
            internal_deps: deps_b,
            kind: PackageKind::Cargo,
        };

        let mut map: BTreeMap<String, &PackageInfo> = BTreeMap::new();
        map.insert("a".into(), &a);
        map.insert("b".into(), &b);

        let mut include = BTreeSet::new();
        include.insert("a".into());
        include.insert("b".into());

        let result = topo_order(&map, &include);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("dependency cycle"));
    }

    #[test]
    fn identifies_publishable_crates() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("publishable", "0.1.0")
            .add_crate("not-publishable", "0.1.0")
            .set_publishable("not-publishable", false);

        workspace.assert_publishable_crates(&["publishable"]);
    }

    #[test]
    fn handles_empty_workspace() {
        let workspace = TestWorkspace::new();

        // Should succeed with no output
        let result = workspace.run_publish(true);
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_invalid_internal_dependencies() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("publishable", "0.1.0")
            .add_crate("not-publishable", "0.1.0")
            .add_dependency("publishable", "not-publishable", "0.1.0")
            .set_publishable("not-publishable", false);

        let result = workspace.run_publish(true);
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("cannot publish due to non-publishable internal dependencies"));
    }

    #[test]
    fn dry_run_publishes_in_dependency_order() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("foundation", "0.1.0")
            .add_crate("middleware", "0.1.0")
            .add_crate("app", "0.1.0")
            .add_dependency("middleware", "foundation", "0.1.0")
            .add_dependency("app", "middleware", "0.1.0");

        // Dry run should succeed and show correct order
        let result = workspace.run_publish(true);
        assert!(result.is_ok());
    }

    #[test]
    fn parses_manifest_publish_field_correctly() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Test publish = false
        let manifest_false = temp_dir.path().join("false.toml");
        fs::write(
            &manifest_false,
            "[package]\nname=\"test\"\nversion=\"0.1.0\"\npublish = false\n",
        )
        .unwrap();
        assert!(!is_publishable_to_crates_io(&manifest_false).unwrap());

        // Test publish = ["custom-registry"] (not crates-io)
        let manifest_custom = temp_dir.path().join("custom.toml");
        fs::write(
            &manifest_custom,
            "[package]\nname=\"test\"\nversion=\"0.1.0\"\npublish = [\"custom-registry\"]\n",
        )
        .unwrap();
        assert!(!is_publishable_to_crates_io(&manifest_custom).unwrap());

        // Test publish = ["crates-io"] (explicitly allowed)
        let manifest_allowed = temp_dir.path().join("allowed.toml");
        fs::write(
            &manifest_allowed,
            "[package]\nname=\"test\"\nversion=\"0.1.0\"\npublish = [\"crates-io\"]\n",
        )
        .unwrap();
        assert!(is_publishable_to_crates_io(&manifest_allowed).unwrap());

        // Test default (no publish field)
        let manifest_default = temp_dir.path().join("default.toml");
        fs::write(
            &manifest_default,
            "[package]\nname=\"test\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        assert!(is_publishable_to_crates_io(&manifest_default).unwrap());
    }

    #[test]
    fn handles_missing_package_section() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest_path = temp_dir.path().join("no-package.toml");
        fs::write(&manifest_path, "[dependencies]\nserde = \"1.0\"\n").unwrap();

        // Should return false (not publishable) for manifests without [package]
        assert!(!is_publishable_to_crates_io(&manifest_path).unwrap());
    }

    #[test]
    fn handles_malformed_toml() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest_path = temp_dir.path().join("broken.toml");
        fs::write(&manifest_path, "[package\nname=\"test\"\n").unwrap(); // Missing closing bracket

        let result = is_publishable_to_crates_io(&manifest_path);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("Invalid data"));
    }

    #[test]
    fn skips_ignored_packages_during_publish() {
        use std::collections::BTreeSet;

        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        // Create config that ignores examples/*
        let config_dir = root.join(".sampo");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            "[packages]\nignore = [\"examples/*\"]\n",
        )
        .unwrap();

        // Create a mock workspace with packages
        let main_pkg = root.join("main-package");
        let examples_pkg = root.join("examples/demo");

        fs::create_dir_all(&main_pkg).unwrap();
        fs::create_dir_all(&examples_pkg).unwrap();

        // Create publishable Cargo.toml files
        let main_toml = r#"
[package]
name = "main-package"
version = "1.0.0"
edition = "2021"
"#;
        let examples_toml = r#"
[package]
name = "examples-demo"
version = "1.0.0"
edition = "2021"
"#;

        fs::write(main_pkg.join("Cargo.toml"), main_toml).unwrap();
        fs::write(examples_pkg.join("Cargo.toml"), examples_toml).unwrap();

        // Create a workspace with both packages
        let workspace = Workspace {
            root: root.to_path_buf(),
            members: vec![
                PackageInfo {
                    name: "main-package".to_string(),
                    version: "1.0.0".to_string(),
                    path: main_pkg,
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "examples-demo".to_string(),
                    version: "1.0.0".to_string(),
                    path: examples_pkg,
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
            ],
        };

        let config = crate::Config::load(&workspace.root).unwrap();

        // Simulate what run_publish does for determining publishable packages
        let mut publishable: BTreeSet<String> = BTreeSet::new();
        for c in &workspace.members {
            // Skip ignored packages
            if should_ignore_package(&config, &workspace, c).unwrap() {
                continue;
            }

            let manifest = c.path.join("Cargo.toml");
            if is_publishable_to_crates_io(&manifest).unwrap() {
                publishable.insert(c.name.clone());
            }
        }

        // Only main-package should be publishable, examples-demo should be ignored
        assert_eq!(publishable.len(), 1);
        assert!(publishable.contains("main-package"));
        assert!(!publishable.contains("examples-demo"));
    }
}
