//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use sampo_core::adapters::PackageAdapter;
use sampo_core::types::{PackageInfo, PackageKind};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Records every argv. Anything but `--version` succeeds, so a spawned dry run cannot
/// fail for unrelated reasons.
fn write_fake_cargo(dir: &Path, argv_log: &Path, version: &str) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join("cargo");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$*\" >> '{}'\n\
             if [ \"$1\" = \"--version\" ]; then echo 'cargo {version} (stub)'; fi\n\
             exit 0\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

fn write_crate(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    let manifest = dir.join("Cargo.toml");
    fs::write(
        &manifest,
        format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n"),
    )
    .unwrap();
    manifest
}

fn package(name: &str, manifest: &Path, internal_deps: &[&str]) -> PackageInfo {
    PackageInfo {
        name: name.to_string(),
        identifier: format!("cargo/{name}"),
        version: "1.0.0".to_string(),
        path: manifest.parent().unwrap().to_path_buf(),
        internal_deps: internal_deps.iter().map(|d| d.to_string()).collect(),
        internal_dev_deps: BTreeSet::new(),
        kind: PackageKind::Cargo,
    }
}

fn argv_lines(argv_log: &Path) -> Vec<String> {
    fs::read_to_string(argv_log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    leaf_manifest: PathBuf,
    dependent_manifest: PathBuf,
    argv_log: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(cargo_version: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let argv_log = root.join("cargo-argv.log");
        let bin = root.join("fake-bin");

        let leaf_manifest = write_crate(&root, "leaf");
        let dependent_manifest = write_crate(&root, "dependent");
        write_fake_cargo(&bin, &argv_log, cargo_version);
        let guard = common::use_only(&bin);

        Self {
            _temp: temp,
            root,
            leaf_manifest,
            dependent_manifest,
            argv_log,
            _guard: guard,
        }
    }

    fn dry_run(&self) -> Vec<String> {
        let leaf = package("leaf", &self.leaf_manifest, &[]);
        let dependent = package("dependent", &self.dependent_manifest, &["cargo/leaf"]);
        PackageAdapter::Cargo
            .publish_dry_run(
                &self.root,
                &[
                    (&leaf, self.leaf_manifest.as_path()),
                    (&dependent, self.dependent_manifest.as_path()),
                ],
                &[],
            )
            .expect("the stub cargo accepts every invocation")
    }

    fn argv(&self) -> Vec<String> {
        argv_lines(&self.argv_log)
    }
}

#[test]
fn a_cargo_without_workspace_dry_run_reports_the_crates_it_skipped() {
    let fixture = Fixture::new("1.90.0");

    let skipped = fixture.dry_run();

    assert_eq!(
        skipped,
        ["dependent (Cargo)"],
        "a skip must reach the caller, which would otherwise report a validation that never ran"
    );

    let argv = fixture.argv();
    assert_eq!(
        argv.len(),
        2,
        "only the version probe and the leaf's dry run may be spawned: {argv:?}"
    );
    assert_eq!(argv[0], "--version");
    assert!(
        argv[1].contains("publish --manifest-path")
            && argv[1].contains(fixture.leaf_manifest.to_str().unwrap())
            && argv[1].ends_with("--dry-run"),
        "the leaf must still be validated: {argv:?}"
    );
    assert!(
        !argv[1].contains(fixture.dependent_manifest.to_str().unwrap()),
        "the dependent crate must not be spawned: {argv:?}"
    );
}

#[test]
fn a_workspace_aware_cargo_skips_nothing() {
    let fixture = Fixture::new("1.91.0");

    let skipped = fixture.dry_run();

    assert!(
        skipped.is_empty(),
        "a workspace dry run validates every crate, dependent ones included: {skipped:?}"
    );
    assert_eq!(
        fixture.argv().len(),
        2,
        "the version probe plus a single workspace dry run: {:?}",
        fixture.argv()
    );
    assert!(
        fixture.argv()[1].contains("publish --workspace --dry-run"),
        "{:?}",
        fixture.argv()
    );
}
