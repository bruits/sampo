use crate::adapters::PackageAdapter;
use crate::types::PackageInfo;
use crate::{
    Config, current_branch, discover_workspace,
    errors::{Result, SampoError},
    filters::should_ignore_package,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::process::Command;

/// Publishes all publishable packages in a workspace to their registries in dependency order.
///
/// This function discovers all packages in the workspace, determines which ones are
/// publishable for their respective ecosystems, validates their dependencies, and publishes
/// them in topological order (dependencies first).
///
/// After publishing, git tags are created for all packages that have been released
/// (including non-publishable packages), as long as they are not ignored by the configuration.
///
/// # Arguments
/// * `root` - Path to the workspace root directory
/// * `dry_run` - If true, performs validation and shows what would be published without actually publishing
/// * `publish_args` - Additional arguments forwarded to the underlying publish command
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
pub fn run_publish(root: &std::path::Path, dry_run: bool, publish_args: &[String]) -> Result<()> {
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

    // Determine which packages are publishable and not ignored
    let mut id_to_package: BTreeMap<String, &PackageInfo> = BTreeMap::new();
    let mut publishable: BTreeSet<String> = BTreeSet::new();
    let mut all_non_ignored: Vec<&PackageInfo> = Vec::new();

    for c in &ws.members {
        // Skip ignored packages
        if should_ignore_package(&config, &ws, c)? {
            continue;
        }

        all_non_ignored.push(c);

        let adapter = match c.kind {
            crate::types::PackageKind::Cargo => PackageAdapter::Cargo,
            crate::types::PackageKind::Npm => PackageAdapter::Npm,
            crate::types::PackageKind::Hex => PackageAdapter::Hex,
        };

        let manifest = adapter.manifest_path(&c.path);
        if !adapter.is_publishable(&manifest)? {
            continue;
        }

        let identifier = c.canonical_identifier().to_string();
        publishable.insert(identifier.clone());
        id_to_package.insert(identifier, c);
    }

    if publishable.is_empty() && all_non_ignored.is_empty() {
        println!("No publishable packages were found in the workspace.");
        return Ok(());
    }

    // Validate internal deps do not include non-publishable packages
    let mut errors: Vec<String> = Vec::new();
    for identifier in &publishable {
        let c = id_to_package.get(identifier).ok_or_else(|| {
            SampoError::Publish(format!(
                "internal error: package '{}' not found in workspace",
                identifier
            ))
        })?;
        for dep in &c.internal_deps {
            if !publishable.contains(dep) {
                errors.push(format!(
                    "package '{}' depends on internal package '{}' which is not publishable",
                    c.name, dep
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
    let order = topo_order(&id_to_package, &publishable)?;

    println!("Publish plan:");
    let mut publish_targets = Vec::new();
    for identifier in &order {
        let package = id_to_package.get(identifier).copied().ok_or_else(|| {
            SampoError::Publish(format!(
                "internal error: crate '{}' not found in workspace",
                identifier
            ))
        })?;
        println!("  - {}", package.display_name(true));
        let adapter = PackageAdapter::from_kind(package.kind);
        let manifest = adapter.manifest_path(&package.path);
        publish_targets.push((package, adapter, manifest));
    }

    if !dry_run {
        println!("Validating publish commands (dry-run)…");

        let mut packages_by_kind: BTreeMap<
            crate::types::PackageKind,
            Vec<(&PackageInfo, &std::path::Path)>,
        > = BTreeMap::new();
        for (package, _, manifest) in &publish_targets {
            packages_by_kind
                .entry(package.kind)
                .or_default()
                .push((*package, manifest.as_path()));
        }

        for (kind, packages) in &packages_by_kind {
            let adapter = PackageAdapter::from_kind(*kind);
            adapter.publish_dry_run(&ws.root, packages, publish_args)?;
        }

        println!("Dry-run validation passed.");
    }

    // Execute publish in order using the appropriate adapter for each package
    for (package, adapter, manifest) in &publish_targets {
        // Skip if the exact version already exists on the registry
        match adapter.version_exists(&package.name, &package.version, Some(manifest.as_path())) {
            Ok(true) => {
                println!(
                    "Skipping {}@{} (already exists on {})",
                    package.display_name(true),
                    package.version,
                    package.kind.display_name()
                );
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!(
                    "Warning: could not check {} registry for {}@{}: {}. Attempting publish…",
                    package.kind.display_name(),
                    package.name,
                    package.version,
                    e
                );
            }
        }

        // Publish using the adapter
        adapter.publish(manifest.as_path(), dry_run, publish_args)?;

        // Tag immediately after successful publish to ensure partial failures still tag what succeeded
        if !dry_run && let Err(e) = tag_published_crate(&ws.root, &package.name, &package.version) {
            eprintln!(
                "Warning: failed to create tag for {}@{}: {}",
                package.name, package.version, e
            );
        }
    }

    // Create tags for all non-publishable but non-ignored packages
    // This ensures that private packages (publish = false) also get version tags for Git tracking
    if !dry_run {
        for package in all_non_ignored {
            // Skip packages that were already tagged in the publish loop
            if publishable.contains(package.canonical_identifier()) {
                continue;
            }
            if let Err(e) = tag_published_crate(&ws.root, &package.name, &package.version) {
                eprintln!(
                    "Warning: failed to create tag for {}@{}: {}",
                    package.name, package.version, e
                );
            }
        }
    }

    if dry_run {
        println!("Dry-run complete.");
    } else {
        println!("Publish complete.");
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::override_current_branch_for_tests;
    use crate::types::{PackageInfo, PackageKind, Workspace};
    use rustc_hash::FxHashMap;
    use std::{
        ffi::OsString,
        fs,
        path::PathBuf,
        process::Command,
        sync::{Mutex, MutexGuard, OnceLock},
    };

    /// Test workspace builder for publish testing
    struct TestWorkspace {
        root: PathBuf,
        _temp_dir: tempfile::TempDir,
        crates: FxHashMap<String, PathBuf>,
        branch: String,
    }

    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_MUTEX.get_or_init(|| Mutex::new(()))
    }

    struct ScopedEnv {
        original: Vec<(&'static str, Option<OsString>)>,
        _lock: MutexGuard<'static, ()>,
    }

    impl ScopedEnv {
        fn set(overrides: &[(&'static str, OsString)]) -> Self {
            let lock = env_lock().lock().unwrap();
            let mut original = Vec::with_capacity(overrides.len());
            for (key, _) in overrides {
                original.push((*key, std::env::var_os(key)));
            }

            for (key, value) in overrides {
                unsafe {
                    std::env::set_var(key, value);
                }
            }

            Self {
                original,
                _lock: lock,
            }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (key, value) in &self.original {
                unsafe {
                    if let Some(v) = value {
                        std::env::set_var(key, v);
                    } else {
                        std::env::remove_var(key);
                    }
                }
            }
        }
    }

    const FAKE_CARGO_SRC: &str = r#"
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::process;

fn main() {
    let log_path = env::var("SAMPO_FAKE_CARGO_LOG").expect("SAMPO_FAKE_CARGO_LOG not set");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("failed to open SAMPO_FAKE_CARGO_LOG");

    let args: Vec<String> = env::args().skip(1).collect();
    writeln!(file, "{}", args.join(" ")).expect("failed to write fake cargo log");

    if args.len() == 1 && args[0] == "--version" {
        let version = env::var("SAMPO_FAKE_CARGO_VERSION").unwrap_or_else(|_| "1.91.0".to_string());
        println!("cargo {} (fake)", version);
        return;
    }

    let is_dry_run = args.iter().any(|arg| arg == "--dry-run");
    let should_fail = if is_dry_run {
        matches!(env::var("SAMPO_FAKE_CARGO_FAIL_DRY_RUN"), Ok(val) if val == "1")
    } else {
        matches!(env::var("SAMPO_FAKE_CARGO_FAIL_ACTUAL"), Ok(val) if val == "1")
    };

    if should_fail {
        process::exit(1);
    }
}
"#;

    struct FakeCargo {
        log_path: PathBuf,
        _env: ScopedEnv,
        _temp: tempfile::TempDir,
    }

    impl FakeCargo {
        fn install(fail_dry_run: bool, fail_actual: bool, version: &str) -> Self {
            let temp_dir = tempfile::tempdir().unwrap();
            let bin_dir = temp_dir.path().join("bin");
            fs::create_dir_all(&bin_dir).unwrap();

            let src_path = bin_dir.join("cargo_stub.rs");
            fs::write(&src_path, FAKE_CARGO_SRC).unwrap();

            let cargo_bin = if cfg!(windows) {
                bin_dir.join("cargo.exe")
            } else {
                bin_dir.join("cargo")
            };

            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
            let status = Command::new(&rustc)
                .arg(&src_path)
                .arg("-Cdebuginfo=0")
                .arg("-Copt-level=0")
                .arg("-o")
                .arg(&cargo_bin)
                .status()
                .expect("failed to compile fake cargo binary");
            assert!(
                status.success(),
                "rustc failed to compile fake cargo binary: {:?}",
                status
            );

            let log_path = temp_dir.path().join("fake_cargo.log");

            let mut path_override = OsString::from(bin_dir.as_os_str());
            if let Some(existing) = std::env::var_os("PATH") {
                let separator_value = if cfg!(windows) { ";" } else { ":" };
                let separator = OsString::from(separator_value);
                path_override.push(&separator);
                path_override.push(&existing);
            }

            let overrides = vec![
                ("PATH", path_override),
                ("SAMPO_FAKE_CARGO_LOG", log_path.clone().into_os_string()),
                (
                    "SAMPO_FAKE_CARGO_FAIL_DRY_RUN",
                    OsString::from(if fail_dry_run { "1" } else { "0" }),
                ),
                (
                    "SAMPO_FAKE_CARGO_FAIL_ACTUAL",
                    OsString::from(if fail_actual { "1" } else { "0" }),
                ),
                ("SAMPO_FAKE_CARGO_VERSION", OsString::from(version)),
            ];

            let env_guard = ScopedEnv::set(&overrides);

            Self {
                log_path,
                _env: env_guard,
                _temp: temp_dir,
            }
        }

        fn log_path(&self) -> &std::path::Path {
            &self.log_path
        }
    }

    impl TestWorkspace {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().unwrap();
            let root = temp_dir.path().to_path_buf();

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
                branch: "main".to_string(),
            }
        }

        fn set_branch(&mut self, branch: &str) -> &mut Self {
            self.branch = branch.to_string();
            self
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
            let _branch_guard = override_current_branch_for_tests(&self.branch);
            super::run_publish(&self.root, dry_run, &[])
        }

        fn assert_publishable_crates(&self, expected: &[&str]) {
            let ws = discover_workspace(&self.root).unwrap();
            let mut actual_publishable = Vec::new();
            let adapter = PackageAdapter::Cargo;

            for c in &ws.members {
                let manifest = adapter.manifest_path(&c.path);
                if adapter.is_publishable(&manifest).unwrap() {
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

        workspace.set_branch("feature");
        let err = workspace.run_publish(true).unwrap_err();
        match err {
            SampoError::Release(message) => {
                assert!(
                    message.contains("not configured for publishing"),
                    "unexpected message: {message}"
                );
                assert!(
                    message.contains("feature"),
                    "branch name should be mentioned in error: {message}"
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

        workspace.set_branch("3.x");
        workspace
            .run_publish(true)
            .expect("publish should succeed on configured branch");
    }

    #[test]
    fn topo_orders_deps_first() {
        // Build a small fake graph using PackageInfo structures
        let a = PackageInfo {
            name: "a".into(),
            identifier: "cargo/a".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/a"),
            internal_deps: BTreeSet::new(),
            kind: PackageKind::Cargo,
        };
        let mut deps_b = BTreeSet::new();
        deps_b.insert("cargo/a".into());
        let b = PackageInfo {
            name: "b".into(),
            identifier: "cargo/b".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/b"),
            internal_deps: deps_b,
            kind: PackageKind::Cargo,
        };
        let mut deps_c = BTreeSet::new();
        deps_c.insert("cargo/b".into());
        let c = PackageInfo {
            name: "c".into(),
            identifier: "cargo/c".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/c"),
            internal_deps: deps_c,
            kind: PackageKind::Cargo,
        };

        let mut map: BTreeMap<String, &PackageInfo> = BTreeMap::new();
        map.insert("cargo/a".into(), &a);
        map.insert("cargo/b".into(), &b);
        map.insert("cargo/c".into(), &c);

        let mut include = BTreeSet::new();
        include.insert("cargo/a".into());
        include.insert("cargo/b".into());
        include.insert("cargo/c".into());

        let order = topo_order(&map, &include).unwrap();
        assert_eq!(order, vec!["cargo/a", "cargo/b", "cargo/c"]);
    }

    #[test]
    fn detects_dependency_cycle() {
        // Create a circular dependency: a -> b -> a
        let mut deps_a = BTreeSet::new();
        deps_a.insert("cargo/b".into());
        let a = PackageInfo {
            name: "a".into(),
            identifier: "cargo/a".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/a"),
            internal_deps: deps_a,
            kind: PackageKind::Cargo,
        };

        let mut deps_b = BTreeSet::new();
        deps_b.insert("cargo/a".into());
        let b = PackageInfo {
            name: "b".into(),
            identifier: "cargo/b".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/b"),
            internal_deps: deps_b,
            kind: PackageKind::Cargo,
        };

        let mut map: BTreeMap<String, &PackageInfo> = BTreeMap::new();
        map.insert("cargo/a".into(), &a);
        map.insert("cargo/b".into(), &b);

        let mut include = BTreeSet::new();
        include.insert("cargo/a".into());
        include.insert("cargo/b".into());

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
            .add_crate("sampo-test-foundation", "0.1.0")
            .add_crate("sampo-test-middleware", "0.1.0")
            .add_crate("sampo-test-app", "0.1.0")
            .add_dependency("sampo-test-middleware", "sampo-test-foundation", "0.1.0")
            .add_dependency("sampo-test-app", "sampo-test-middleware", "0.1.0");

        let _fake_cargo = FakeCargo::install(false, false, "1.91.0");

        // Dry run should succeed and show correct order
        let result = workspace.run_publish(true);
        assert!(result.is_ok());
    }

    #[test]
    fn run_publish_performs_preflight_dry_runs() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("sampo-preflight", "0.0.1");

        let fake_cargo = FakeCargo::install(false, false, "1.91.0");

        workspace
            .run_publish(false)
            .expect("publish should succeed with fake cargo");

        let log = fs::read_to_string(fake_cargo.log_path()).expect("fake cargo log should exist");
        let publish_lines: Vec<&str> = log
            .lines()
            .filter(|line| line.starts_with("publish "))
            .collect();

        assert_eq!(
            publish_lines.len(),
            2,
            "expected dry-run validation followed by real publish, got: {:?}",
            publish_lines
        );
        assert!(
            publish_lines[0].contains("--dry-run"),
            "first invocation should include --dry-run: {:?}",
            publish_lines[0]
        );
        assert!(
            publish_lines[0].contains("--workspace"),
            "workspace dry-run should leverage --workspace flag: {:?}",
            publish_lines[0]
        );
        assert!(
            !publish_lines[1].contains("--dry-run"),
            "second invocation should omit --dry-run: {:?}",
            publish_lines[1]
        );
    }

    #[test]
    fn dry_run_validation_failure_blocks_publish() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("sampo-preflight-failure", "0.0.1");

        let fake_cargo = FakeCargo::install(true, false, "1.91.0");

        let err = workspace
            .run_publish(false)
            .expect_err("dry-run failure should stop publish");
        let message = format!("{err}");
        assert!(
            message.contains("Cargo workspace dry-run failed"),
            "expected dry-run failure context, got {message}"
        );

        let log = fs::read_to_string(fake_cargo.log_path()).expect("fake cargo log should exist");
        let publish_lines: Vec<&str> = log
            .lines()
            .filter(|line| line.starts_with("publish "))
            .collect();
        assert_eq!(publish_lines.len(), 1, "expected only dry-run invocation");
        assert!(
            publish_lines[0].contains("--dry-run"),
            "dry-run invocation should include --dry-run: {:?}",
            publish_lines[0]
        );
    }

    #[test]
    fn skips_dependent_dry_runs_on_old_cargo_versions() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("sampo-base", "0.1.0")
            .add_crate("sampo-app", "0.1.0")
            .add_dependency("sampo-app", "sampo-base", "0.1.0");

        let fake_cargo = FakeCargo::install(false, false, "1.80.0");

        workspace
            .run_publish(false)
            .expect("publish should succeed when dependent dry-runs are skipped");

        let log = fs::read_to_string(fake_cargo.log_path()).expect("fake cargo log should exist");
        let publish_lines: Vec<&str> = log
            .lines()
            .filter(|line| line.starts_with("publish "))
            .collect();

        assert_eq!(
            publish_lines.len(),
            3,
            "expected dry-run + two actual publishes: {:?}",
            publish_lines
        );
        assert!(
            publish_lines[0].contains("--dry-run"),
            "first invocation should dry-run the dependency crate: {:?}",
            publish_lines[0]
        );
        assert!(
            !publish_lines[1].contains("--dry-run"),
            "second invocation should be the first real publish: {:?}",
            publish_lines[1]
        );
        assert!(
            !publish_lines[2].contains("--dry-run"),
            "dependent crate should skip dry-run when workspace publish is unavailable: {:?}",
            publish_lines[2]
        );
        assert!(
            !publish_lines
                .iter()
                .any(|line| line.contains("--workspace")),
            "legacy fallback should not invoke --workspace: {:?}",
            publish_lines
        );
    }

    #[test]
    fn parses_manifest_publish_field_correctly() {
        let temp_dir = tempfile::tempdir().unwrap();
        let adapter = PackageAdapter::Cargo;

        // Test publish = false
        let manifest_false = temp_dir.path().join("false.toml");
        fs::write(
            &manifest_false,
            "[package]\nname=\"test\"\nversion=\"0.1.0\"\npublish = false\n",
        )
        .unwrap();
        assert!(!adapter.is_publishable(&manifest_false).unwrap());

        // Test publish = ["custom-registry"] (not crates-io)
        let manifest_custom = temp_dir.path().join("custom.toml");
        fs::write(
            &manifest_custom,
            "[package]\nname=\"test\"\nversion=\"0.1.0\"\npublish = [\"custom-registry\"]\n",
        )
        .unwrap();
        assert!(!adapter.is_publishable(&manifest_custom).unwrap());

        // Test publish = ["crates-io"] (explicitly allowed)
        let manifest_allowed = temp_dir.path().join("allowed.toml");
        fs::write(
            &manifest_allowed,
            "[package]\nname=\"test\"\nversion=\"0.1.0\"\npublish = [\"crates-io\"]\n",
        )
        .unwrap();
        assert!(adapter.is_publishable(&manifest_allowed).unwrap());

        // Test default (no publish field)
        let manifest_default = temp_dir.path().join("default.toml");
        fs::write(
            &manifest_default,
            "[package]\nname=\"test\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        assert!(adapter.is_publishable(&manifest_default).unwrap());
    }

    #[test]
    fn handles_missing_package_section() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest_path = temp_dir.path().join("no-package.toml");
        fs::write(&manifest_path, "[dependencies]\nserde = \"1.0\"\n").unwrap();

        let adapter = PackageAdapter::Cargo;
        // Should return false (not publishable) for manifests without [package]
        assert!(!adapter.is_publishable(&manifest_path).unwrap());
    }

    #[test]
    fn handles_malformed_toml() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest_path = temp_dir.path().join("broken.toml");
        fs::write(&manifest_path, "[package\nname=\"test\"\n").unwrap(); // Missing closing bracket

        let adapter = PackageAdapter::Cargo;
        let result = adapter.is_publishable(&manifest_path);
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
                    identifier: PackageInfo::dependency_identifier(
                        PackageKind::Cargo,
                        "main-package",
                    ),
                    version: "1.0.0".to_string(),
                    path: main_pkg,
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "examples-demo".to_string(),
                    identifier: PackageInfo::dependency_identifier(
                        PackageKind::Cargo,
                        "examples-demo",
                    ),
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
        let adapter = PackageAdapter::Cargo;
        for c in &workspace.members {
            // Skip ignored packages
            if should_ignore_package(&config, &workspace, c).unwrap() {
                continue;
            }

            let manifest = adapter.manifest_path(&c.path);
            if adapter.is_publishable(&manifest).unwrap() {
                publishable.insert(c.name.clone());
            }
        }

        // Only main-package should be publishable, examples-demo should be ignored
        assert_eq!(publishable.len(), 1);
        assert!(publishable.contains("main-package"));
        assert!(!publishable.contains("examples-demo"));
    }

    #[test]
    fn tags_each_package_only_once() {
        fn init_git_repo_for_test(path: &Path) {
            let status = Command::new("git")
                .arg("init")
                .current_dir(path)
                .status()
                .expect("failed to run git init");
            assert!(status.success(), "git init failed");

            let email_status = Command::new("git")
                .args(["config", "user.email", "test@example.com"])
                .current_dir(path)
                .status()
                .expect("failed to configure git user email");
            assert!(email_status.success(), "git config user.email failed");

            let name_status = Command::new("git")
                .args(["config", "user.name", "Test User"])
                .current_dir(path)
                .status()
                .expect("failed to configure git user name");
            assert!(name_status.success(), "git config user.name failed");

            // Create initial commit so HEAD exists
            let add_status = Command::new("git")
                .args(["add", "-A"])
                .current_dir(path)
                .status()
                .expect("failed to run git add");
            assert!(add_status.success(), "git add failed");

            let commit_status = Command::new("git")
                .args(["commit", "-m", "Initial commit"])
                .current_dir(path)
                .status()
                .expect("failed to run git commit");
            assert!(commit_status.success(), "git commit failed");
        }

        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("publishable-crate", "1.0.0")
            .add_crate("private-crate", "1.0.0")
            .set_publishable("private-crate", false);

        // Initialize git repository
        init_git_repo_for_test(&workspace.root);

        let _fake_cargo = FakeCargo::install(false, false, "1.91.0");

        // Run publish (not dry-run to actually create tags)
        workspace
            .run_publish(false)
            .expect("publish should succeed");

        // List all tags
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .arg("tag")
            .arg("--list")
            .output()
            .expect("git tag list should succeed");

        let tags = String::from_utf8_lossy(&output.stdout);
        let tag_lines: Vec<&str> = tags.lines().collect();

        // Should have exactly 2 tags (one per package)
        assert_eq!(
            tag_lines.len(),
            2,
            "Expected exactly 2 tags, got: {:?}",
            tag_lines
        );

        // Verify each tag exists once
        assert!(
            tag_lines.contains(&"publishable-crate-v1.0.0"),
            "Missing tag for publishable crate"
        );
        assert!(
            tag_lines.contains(&"private-crate-v1.0.0"),
            "Missing tag for private crate"
        );

        // Verify no duplicate tags (already checked by length, but be explicit)
        let unique_tags: BTreeSet<&str> = tag_lines.iter().copied().collect();
        assert_eq!(
            unique_tags.len(),
            tag_lines.len(),
            "Duplicate tags detected: {:?}",
            tag_lines
        );
    }
}
