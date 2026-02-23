use crate::adapters::PackageAdapter;
use crate::types::{PackageInfo, PackageKind, PublishOutput};
use crate::{
    Config, current_branch, discover_workspace,
    errors::{Result, SampoError},
    filters::should_ignore_package,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::process::Command;

/// Holds universal and per-ecosystem extra arguments for publish commands.
///
/// Universal args (from `-- <args>`) are forwarded to every adapter.
/// Per-ecosystem args are forwarded only to the matching adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublishExtraArgs {
    pub universal: Vec<String>,
    pub cargo: Vec<String>,
    pub npm: Vec<String>,
    pub hex: Vec<String>,
    pub pypi: Vec<String>,
    pub packagist: Vec<String>,
}

impl PublishExtraArgs {
    /// Returns the merged universal + ecosystem-specific args for a given package kind.
    pub fn args_for_kind(&self, kind: PackageKind) -> Vec<String> {
        let ecosystem_args = match kind {
            PackageKind::Cargo => &self.cargo,
            PackageKind::Npm => &self.npm,
            PackageKind::Hex => &self.hex,
            PackageKind::PyPI => &self.pypi,
            PackageKind::Packagist => &self.packagist,
        };
        let mut merged = self.universal.clone();
        merged.extend(ecosystem_args.iter().cloned());
        merged
    }
}

/// Publishes all publishable packages in a workspace to their registries in dependency order.
///
/// This function discovers all packages in the workspace, determines which ones are
/// publishable for their respective ecosystems, validates their dependencies, and publishes
/// them in topological order (dependencies first).
///
/// After publishing, git tags are created for all packages that have been released
/// (including non-publishable packages), as long as they are not ignored by the configuration.
///
/// Returns a `PublishOutput` containing the tags that were created (non-dry-run) or would
/// be created (dry-run), allowing callers to know what happened or would happen.
///
/// # Arguments
/// * `root` - Path to the workspace root directory
/// * `dry_run` - If true, performs validation and shows what would be published without actually publishing
/// * `extra_args` - Universal and per-ecosystem extra arguments for the publish commands
///
/// # Examples
/// ```no_run
/// use std::path::Path;
/// use sampo_core::run_publish;
/// use sampo_core::publish::PublishExtraArgs;
///
/// // Dry run to see what would be published
/// let output = run_publish(Path::new("."), true, &PublishExtraArgs::default()).unwrap();
/// println!("Would create {} tags", output.tags.len());
///
/// // Actual publish with cargo-specific args
/// let args = PublishExtraArgs {
///     cargo: vec!["--allow-dirty".to_string()],
///     ..Default::default()
/// };
/// let output = run_publish(Path::new("."), false, &args).unwrap();
/// println!("Created {} tags", output.tags.len());
/// ```
pub fn run_publish(
    root: &std::path::Path,
    dry_run: bool,
    extra_args: &PublishExtraArgs,
) -> Result<PublishOutput> {
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
            crate::types::PackageKind::PyPI => PackageAdapter::PyPI,
            crate::types::PackageKind::Packagist => PackageAdapter::Packagist,
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
        return Ok(PublishOutput {
            tags: Vec::new(),
            dry_run,
        });
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

    // Build list of packages that actually need publishing (version doesn't exist on registry).
    // We check version_exists() BEFORE dry-run validation to avoid unnecessary compilation
    // and failures when all packages are already published.
    println!("Checking registry for existing versions…");
    let mut publish_targets: Vec<(&PackageInfo, PackageAdapter, std::path::PathBuf)> = Vec::new();

    for identifier in &order {
        let package = id_to_package.get(identifier).copied().ok_or_else(|| {
            SampoError::Publish(format!(
                "internal error: crate '{}' not found in workspace",
                identifier
            ))
        })?;
        let adapter = PackageAdapter::from_kind(package.kind);
        let manifest = adapter.manifest_path(&package.path);

        match adapter.version_exists(&package.name, &package.version, Some(manifest.as_path())) {
            Ok(true) => {
                println!(
                    "  - {} (already exists on {})",
                    package.display_name(true),
                    package.kind.display_name()
                );
            }
            Ok(false) => {
                publish_targets.push((package, adapter, manifest));
            }
            Err(e) => {
                // If we can't check, include in publish targets to be safe
                eprintln!(
                    "Warning: could not check {} registry for {}@{}: {}. Will attempt publish.",
                    package.kind.display_name(),
                    package.name,
                    package.version,
                    e
                );
                publish_targets.push((package, adapter, manifest));
            }
        }
    }

    if publish_targets.is_empty() {
        println!("All packages are already published. Nothing to do.");
        // Still need to handle private package tagging below
    } else {
        println!("Publish plan:");
        for (package, _, _) in &publish_targets {
            println!("  - {}", package.display_name(true));
        }
    }

    if !dry_run && !publish_targets.is_empty() {
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
            let args = extra_args.args_for_kind(*kind);
            adapter.publish_dry_run(&ws.root, packages, &args)?;
        }

        println!("Dry-run validation passed.");
    }

    let mut tags_to_create: Vec<String> = Vec::new();
    let mut any_published = false;

    for (package, adapter, manifest) in &publish_targets {
        let args = extra_args.args_for_kind(package.kind);
        adapter.publish(manifest.as_path(), dry_run, &args)?;
        any_published = true;

        let tag = config.build_tag_name(&package.name, &package.version);

        // Tag immediately after successful publish to ensure partial failures still tag what succeeded
        if !dry_run {
            if let Err(e) = tag_published_crate(&ws.root, &config, &package.name, &package.version)
            {
                eprintln!(
                    "Warning: failed to create tag for {}@{}: {}",
                    package.name, package.version, e
                );
            } else {
                tags_to_create.push(tag);
            }
        } else if !package_tag_exists(&ws.root, &config, &package.name, &package.version)? {
            tags_to_create.push(tag);
        }
    }

    // Determine which private (non-publishable) packages still need tags.
    // We only want to emit new tags for private packages when:
    //   1. At least one publishable package was actually published in this run, OR
    //   2. There exist private packages whose current version does not have a tag yet
    // This avoids creating tags during no-op invocations (e.g., auto mode with no releases)
    let mut private_packages_to_tag: Vec<&PackageInfo> = Vec::new();
    for package in &all_non_ignored {
        if publishable.contains(package.canonical_identifier()) {
            continue;
        }
        if !package_tag_exists(&ws.root, &config, &package.name, &package.version)? {
            private_packages_to_tag.push(*package);
        }
    }

    if any_published || !private_packages_to_tag.is_empty() {
        for package in &private_packages_to_tag {
            let tag = config.build_tag_name(&package.name, &package.version);
            if !dry_run {
                if let Err(e) =
                    tag_published_crate(&ws.root, &config, &package.name, &package.version)
                {
                    eprintln!(
                        "Warning: failed to create tag for {}@{}: {}",
                        package.name, package.version, e
                    );
                } else {
                    tags_to_create.push(tag);
                }
            } else {
                tags_to_create.push(tag);
            }
        }
    }

    if dry_run {
        println!("Dry-run complete.");
    } else {
        println!("Publish complete.");
    }

    Ok(PublishOutput {
        tags: tags_to_create,
        dry_run,
    })
}

fn package_tag_exists(
    repo_root: &Path,
    config: &Config,
    crate_name: &str,
    version: &str,
) -> Result<bool> {
    if !repo_root.join(".git").exists() {
        return Ok(false);
    }

    let tag = config.build_tag_name(crate_name, version);
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
            return Ok(true);
        }
    }

    Ok(false)
}

/// Creates an annotated git tag for a published crate.
///
/// Skips tagging if not in a git repository or if the tag already exists.
pub fn tag_published_crate(
    repo_root: &Path,
    config: &Config,
    crate_name: &str,
    version: &str,
) -> Result<bool> {
    if !repo_root.join(".git").exists() {
        // Not a git repo, skip
        return Ok(false);
    }
    if package_tag_exists(repo_root, config, crate_name, version)? {
        return Ok(false);
    }
    let tag = config.build_tag_name(crate_name, version);

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
        Ok(true)
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

    #[test]
    fn args_for_kind_returns_universal_only_when_no_ecosystem_args() {
        let extra = PublishExtraArgs {
            universal: vec!["--tag".into(), "beta".into()],
            ..Default::default()
        };
        assert_eq!(
            extra.args_for_kind(PackageKind::Cargo),
            vec!["--tag", "beta"]
        );
        assert_eq!(extra.args_for_kind(PackageKind::Npm), vec!["--tag", "beta"]);
        assert_eq!(extra.args_for_kind(PackageKind::Hex), vec!["--tag", "beta"]);
        assert_eq!(
            extra.args_for_kind(PackageKind::PyPI),
            vec!["--tag", "beta"]
        );
        assert_eq!(
            extra.args_for_kind(PackageKind::Packagist),
            vec!["--tag", "beta"]
        );
    }

    #[test]
    fn args_for_kind_merges_universal_and_ecosystem_args() {
        let extra = PublishExtraArgs {
            universal: vec!["--tag".into(), "beta".into()],
            cargo: vec!["--allow-dirty".into()],
            npm: vec!["--access".into(), "restricted".into()],
            ..Default::default()
        };

        assert_eq!(
            extra.args_for_kind(PackageKind::Cargo),
            vec!["--tag", "beta", "--allow-dirty"]
        );
        assert_eq!(
            extra.args_for_kind(PackageKind::Npm),
            vec!["--tag", "beta", "--access", "restricted"]
        );
    }

    #[test]
    fn args_for_kind_returns_only_ecosystem_args_when_no_universal() {
        let extra = PublishExtraArgs {
            cargo: vec!["--allow-dirty".into()],
            hex: vec!["--replace".into()],
            ..Default::default()
        };

        assert_eq!(
            extra.args_for_kind(PackageKind::Cargo),
            vec!["--allow-dirty"]
        );
        assert_eq!(extra.args_for_kind(PackageKind::Npm), Vec::<String>::new());
        assert_eq!(extra.args_for_kind(PackageKind::Hex), vec!["--replace"]);
    }

    #[test]
    fn args_for_kind_returns_empty_when_no_args() {
        let extra = PublishExtraArgs::default();
        assert!(extra.args_for_kind(PackageKind::Cargo).is_empty());
        assert!(extra.args_for_kind(PackageKind::Npm).is_empty());
    }

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

            // Create .sampo/ directory (required for discover_workspace)
            fs::create_dir_all(root.join(".sampo")).unwrap();

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

        fn run_publish(&self, dry_run: bool) -> Result<PublishOutput> {
            let _branch_guard = override_current_branch_for_tests(&self.branch);
            super::run_publish(&self.root, dry_run, &super::PublishExtraArgs::default())
        }

        fn run_publish_with_args(
            &self,
            dry_run: bool,
            extra_args: &super::PublishExtraArgs,
        ) -> Result<PublishOutput> {
            let _branch_guard = override_current_branch_for_tests(&self.branch);
            super::run_publish(&self.root, dry_run, extra_args)
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

        // Empty workspace should error early (no packages found)
        let result = workspace.run_publish(true);
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("No packages found"));
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
    fn per_ecosystem_args_reach_cargo_publish_command() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("sampo-extra-args", "0.0.1");

        let fake_cargo = FakeCargo::install(false, false, "1.91.0");

        let extra = super::PublishExtraArgs {
            cargo: vec!["--allow-dirty".into(), "--no-verify".into()],
            ..Default::default()
        };

        workspace
            .run_publish_with_args(false, &extra)
            .expect("publish with per-ecosystem args should succeed");

        let log = fs::read_to_string(fake_cargo.log_path()).expect("fake cargo log should exist");
        let publish_lines: Vec<&str> = log
            .lines()
            .filter(|line| line.starts_with("publish "))
            .collect();

        assert!(
            publish_lines.len() >= 2,
            "expected at least dry-run + real publish, got: {:?}",
            publish_lines
        );
        for line in &publish_lines {
            assert!(
                line.contains("--allow-dirty"),
                "publish invocation should include --allow-dirty: {:?}",
                line
            );
            assert!(
                line.contains("--no-verify"),
                "publish invocation should include --no-verify: {:?}",
                line
            );
        }
    }

    #[test]
    fn universal_and_ecosystem_args_both_forwarded() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("sampo-merged-args", "0.0.1");

        let fake_cargo = FakeCargo::install(false, false, "1.91.0");

        let extra = super::PublishExtraArgs {
            universal: vec!["--tag".into(), "beta".into()],
            cargo: vec!["--allow-dirty".into()],
            ..Default::default()
        };

        workspace
            .run_publish_with_args(false, &extra)
            .expect("publish with merged args should succeed");

        let log = fs::read_to_string(fake_cargo.log_path()).expect("fake cargo log should exist");
        let actual_publish: Vec<&str> = log
            .lines()
            .filter(|line| line.starts_with("publish ") && !line.contains("--dry-run"))
            .collect();
        assert_eq!(actual_publish.len(), 1, "expected one real publish");
        assert!(
            actual_publish[0].contains("--tag"),
            "should forward universal --tag arg: {:?}",
            actual_publish[0]
        );
        assert!(
            actual_publish[0].contains("--allow-dirty"),
            "should forward cargo-specific --allow-dirty arg: {:?}",
            actual_publish[0]
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

    #[test]
    fn private_packages_not_tagged_without_publish() {
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

        // First publish creates tags
        workspace
            .run_publish(false)
            .expect("first publish should succeed");

        // List tags after first publish
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .arg("tag")
            .arg("--list")
            .output()
            .expect("git tag list should succeed");

        let tags = String::from_utf8_lossy(&output.stdout);
        let tag_lines: Vec<&str> = tags.lines().collect();
        assert_eq!(tag_lines.len(), 2, "Expected 2 tags after first publish");

        // Run publish again without any version changes (simulates fresh auto mode call)
        // This should NOT create new tags
        workspace
            .run_publish(false)
            .expect("second publish should succeed");

        // List tags after second publish
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .arg("tag")
            .arg("--list")
            .output()
            .expect("git tag list should succeed");

        let tags = String::from_utf8_lossy(&output.stdout);
        let tag_lines: Vec<&str> = tags.lines().collect();

        // Should still have exactly 2 tags (no duplicates created)
        assert_eq!(
            tag_lines.len(),
            2,
            "Expected still 2 tags after second publish without version changes"
        );
    }

    #[test]
    fn private_only_workspace_creates_tags() {
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
        // Create workspace with ONLY private packages
        workspace
            .add_crate("private-one", "1.0.0")
            .add_crate("private-two", "1.0.0")
            .set_publishable("private-one", false)
            .set_publishable("private-two", false);

        // Initialize git repository
        init_git_repo_for_test(&workspace.root);

        let _fake_cargo = FakeCargo::install(false, false, "1.91.0");

        // Run publish
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

        // Should have 2 tags (one per private package)
        // This is the edge case: workspace with ONLY private packages should still create tags
        assert_eq!(
            tag_lines.len(),
            2,
            "Expected 2 tags for private-only workspace, got: {:?}",
            tag_lines
        );

        assert!(
            tag_lines.contains(&"private-one-v1.0.0"),
            "Missing tag for private-one"
        );
        assert!(
            tag_lines.contains(&"private-two-v1.0.0"),
            "Missing tag for private-two"
        );
    }

    #[test]
    fn mixed_workspace_after_publish_creates_all_tags() {
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

        // Run publish - should create tags for both publishable AND private packages
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

        // Should have 2 tags: one for publishable-crate, one for private-crate
        // This verifies that when a publishable package is published, private packages also get tagged
        assert_eq!(
            tag_lines.len(),
            2,
            "Expected 2 tags (publishable + private), got: {:?}",
            tag_lines
        );

        assert!(
            tag_lines.contains(&"publishable-crate-v1.0.0"),
            "Missing tag for publishable crate"
        );
        assert!(
            tag_lines.contains(&"private-crate-v1.0.0"),
            "Private tag should have been created because publishable package was published"
        );
    }

    #[test]
    fn private_package_tagged_in_mixed_workspace() {
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

        // Regression test for the bug identified in review:
        // "In a mixed workspace where a release affects only private crates (e.g., publish = false
        // internal services) while other publishable crates exist but had no version bump,
        // private packages should still receive tags."
        //
        // This test verifies that in a workspace with both publishable and private packages,
        // ALL packages (including private ones) receive tags after a publish operation,
        // regardless of whether the publishable packages were actually published to registries.
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("lib-core", "1.0.0")
            .add_crate("internal-service", "0.5.0")
            .set_publishable("internal-service", false);

        // Initialize git repository
        init_git_repo_for_test(&workspace.root);

        let _fake_cargo = FakeCargo::install(false, false, "1.91.0");

        // Run publish - both packages should be tagged
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

        // Both packages should receive tags
        assert!(
            tag_lines.contains(&"lib-core-v1.0.0"),
            "Publishable package should receive a tag. Got tags: {:?}",
            tag_lines
        );
        assert!(
            tag_lines.contains(&"internal-service-v0.5.0"),
            "Private package should receive a tag in mixed workspace. Got tags: {:?}",
            tag_lines
        );
    }

    #[test]
    fn no_tags_created_when_run_publish_without_new_versions() {
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

        // CRITICAL REGRESSION TEST: Verify that run_publish called without any new releases
        // does NOT create tags. This simulates the "auto" mode workflow where run_publish
        // is called on every push to main, even when there are no changesets.
        //
        // Without this safeguard, every commit would create tags for all packages and
        // trigger production deploys, breaking the contract that "published = true only
        // when new versions were released".
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("lib-core", "1.0.0")
            .add_crate("internal-service", "0.5.0")
            .set_publishable("internal-service", false);

        // Initialize git repository
        init_git_repo_for_test(&workspace.root);

        let _fake_cargo = FakeCargo::install(false, false, "1.91.0");

        // First publish creates the initial tags
        workspace
            .run_publish(false)
            .expect("first publish should succeed");

        // Count tags after first publish
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .arg("tag")
            .arg("--list")
            .output()
            .expect("git tag list should succeed");
        let tags_after_first = String::from_utf8_lossy(&output.stdout);
        let initial_tag_count = tags_after_first.lines().count();
        assert_eq!(
            initial_tag_count, 2,
            "Should have 2 tags after first publish"
        );

        // Make a commit that doesn't change any versions (simulates a commit with no changesets)
        let readme = workspace.root.join("README.md");
        fs::write(&readme, "# Updated README\n").expect("failed to write README");
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&workspace.root)
            .status()
            .expect("failed to git add");
        Command::new("git")
            .args(["commit", "-m", "docs: update README"])
            .current_dir(&workspace.root)
            .status()
            .expect("failed to git commit");

        // Run publish again WITHOUT bumping any versions
        // This simulates what happens in "auto" mode on a commit with no changesets
        workspace
            .run_publish(false)
            .expect("second publish should succeed");

        // Verify NO new tags were created
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .arg("tag")
            .arg("--list")
            .output()
            .expect("git tag list should succeed");
        let tags_after_second = String::from_utf8_lossy(&output.stdout);
        let final_tag_count = tags_after_second.lines().count();

        assert_eq!(
            final_tag_count,
            initial_tag_count,
            "No new tags should be created when run_publish is called without version bumps. \
             This is critical to prevent spurious tags on every commit in auto mode. \
             Initial tags: {:?}, Final tags: {:?}",
            tags_after_first.lines().collect::<Vec<_>>(),
            tags_after_second.lines().collect::<Vec<_>>()
        );
    }

    #[test]
    fn private_package_tagged_when_bumped_in_mixed_workspace() {
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

        // Regression test for the exact scenario from the review:
        // "In a mixed workspace where a release affects only private crates (e.g., publish = false
        // internal services) while other publishable crates exist but had no version bump,
        // any_published stays false and publishable.is_empty() is false, so the new tagging loop
        // is skipped entirely. Those private crates never get tags, post_merge_publish sees no
        // 'new tags', and the GitHub Action still reports published = false."
        //
        // Setup: Mixed workspace with publishable crate (already published) + private crate (newly bumped)
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("lib-core", "1.0.0")
            .add_crate("internal-service", "0.5.0")
            .set_publishable("internal-service", false);

        init_git_repo_for_test(&workspace.root);
        let _fake_cargo = FakeCargo::install(false, false, "1.91.0");

        // First: publish lib-core@1.0.0 and tag both packages
        workspace
            .run_publish(false)
            .expect("first publish should succeed");

        // Verify both were tagged
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .arg("tag")
            .arg("--list")
            .output()
            .expect("git tag list should succeed");
        let tags = String::from_utf8_lossy(&output.stdout);
        assert!(tags.contains("lib-core-v1.0.0"));
        assert!(tags.contains("internal-service-v0.5.0"));

        // Now: bump ONLY the private package (simulates a release PR affecting only private crates)
        let service_manifest = workspace.root.join("crates/internal-service/Cargo.toml");
        let manifest_content = fs::read_to_string(&service_manifest).unwrap();
        let updated_manifest = manifest_content.replace("0.5.0", "0.6.0");
        fs::write(&service_manifest, updated_manifest).unwrap();

        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&workspace.root)
            .status()
            .expect("failed to git add");
        Command::new("git")
            .args(["commit", "-m", "Bump internal-service to 0.6.0"])
            .current_dir(&workspace.root)
            .status()
            .expect("failed to git commit");

        // Second publish: lib-core will be skipped (already exists), but internal-service should get tagged
        workspace
            .run_publish(false)
            .expect("second publish should succeed");

        // CRITICAL ASSERTION: The private package MUST have received a tag even though
        // no publishable package was actually published in this run
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace.root)
            .arg("tag")
            .arg("--list")
            .output()
            .expect("git tag list should succeed");
        let tags = String::from_utf8_lossy(&output.stdout);

        assert!(
            tags.contains("internal-service-v0.6.0"),
            "Private package should receive a tag even when the publishable package was skipped. \
             This is the exact bug from the review. Got tags: {}",
            tags
        );
    }

    /// Regression test: when all packages already exist on the registry,
    /// preflight dry-run validation should be skipped entirely.
    ///
    /// This test uses `serde` version `1.0.0` which is known to exist on crates.io.
    /// The test verifies that:
    /// 1. `version_exists()` returns true for the package
    /// 2. No cargo publish commands are executed (FakeCargo log is empty)
    /// 3. The publish function completes successfully
    ///
    /// NOTE: This test requires network access to query crates.io.
    #[test]
    fn skips_preflight_when_all_packages_already_published() {
        // Use a well-known crate that definitely exists on crates.io
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().to_path_buf();

        // Create .sampo/ directory
        fs::create_dir_all(root.join(".sampo")).unwrap();

        // Create workspace structure with a crate name matching an existing crates.io package
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        let crate_dir = root.join("crates").join("serde");
        fs::create_dir_all(crate_dir.join("src")).unwrap();
        fs::write(
            crate_dir.join("Cargo.toml"),
            // Use a version that definitely exists on crates.io
            "[package]\nname=\"serde\"\nversion=\"1.0.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        fs::write(crate_dir.join("src/lib.rs"), "// test").unwrap();

        // Install FakeCargo to intercept publish commands
        let fake_cargo = FakeCargo::install(false, false, "1.91.0");

        // Run publish (not dry-run)
        let _branch_guard = override_current_branch_for_tests("main");
        let result = super::run_publish(&root, false, &super::PublishExtraArgs::default());

        // The publish should succeed (no error)
        assert!(
            result.is_ok(),
            "Publish should succeed when all packages already exist. Error: {:?}",
            result.err()
        );

        // CRITICAL ASSERTION: The FakeCargo log should contain NO publish commands
        // because version_exists() should return true and skip the entire publish phase.
        // The only allowed command is --version check.
        let log_content = fs::read_to_string(fake_cargo.log_path()).unwrap_or_default();
        let publish_commands: Vec<&str> = log_content
            .lines()
            .filter(|line| line.contains("publish"))
            .collect();

        assert!(
            publish_commands.is_empty(),
            "No publish commands should be executed when all packages already exist on registry. \
             Found commands: {:?}. Full log: {}",
            publish_commands,
            log_content
        );
    }
}
