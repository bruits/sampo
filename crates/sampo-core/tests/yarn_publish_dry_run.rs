//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use sampo_core::adapters::{PackageAdapter, PublishOutcome};
use sampo_core::types::{PackageInfo, PackageKind};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn write_yarn_package(dir: &Path, yarn_version: &str) -> PathBuf {
    let manifest = dir.join("package.json");
    fs::write(
        &manifest,
        format!(r#"{{"name":"pkg","version":"1.0.0","packageManager":"yarn@{yarn_version}"}}"#),
    )
    .unwrap();
    manifest
}

/// Records every argv. Anything but `--version` exits with `exit_code`: 1 keeps the
/// spawned case off the network.
fn write_fake_yarn(dir: &Path, argv_log: &Path, version: &str, exit_code: u8) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join("yarn");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$*\" >> '{}'\n\
             if [ \"$1\" = \"--version\" ]; then echo {version}; exit 0; fi\n\
             exit {exit_code}\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
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
    manifest: PathBuf,
    argv_log: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(yarn_version: &str) -> Self {
        Self::build(yarn_version, 1)
    }

    /// A yarn whose publish succeeds.
    fn permissive(yarn_version: &str) -> Self {
        Self::build(yarn_version, 0)
    }

    fn build(yarn_version: &str, exit_code: u8) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let argv_log = root.join("yarn-argv.log");
        let bin = root.join("fake-bin");

        let manifest = write_yarn_package(root, yarn_version);
        write_fake_yarn(&bin, &argv_log, yarn_version, exit_code);
        let guard = common::use_only(&bin);

        Self {
            _temp: temp,
            manifest,
            argv_log,
            _guard: guard,
        }
    }

    fn argv(&self) -> Vec<String> {
        argv_lines(&self.argv_log)
    }

    fn package(&self) -> PackageInfo {
        PackageInfo {
            name: "pkg".to_string(),
            identifier: "npm/pkg".to_string(),
            version: "1.0.0".to_string(),
            path: self.manifest.parent().unwrap().to_path_buf(),
            internal_deps: BTreeSet::new(),
            internal_dev_deps: BTreeSet::new(),
            kind: PackageKind::Npm,
        }
    }
}

#[test]
fn yarn_classic_dry_run_is_skipped_without_spawning_a_publish() {
    let fixture = Fixture::new("1.22.22");

    let outcome = PackageAdapter::Npm
        .publish(&fixture.manifest, true, &[])
        .expect("a dry run yarn cannot simulate must be skipped, not failed");

    assert_eq!(
        outcome,
        PublishOutcome::DryRunSkipped,
        "the skip must reach the caller, which reports what went unvalidated"
    );
    assert_eq!(
        fixture.argv(),
        ["--version"],
        "nothing but the version probe may be spawned"
    );
}

#[test]
fn a_skipped_dry_run_is_reported_to_the_caller() {
    let fixture = Fixture::new("1.22.22");
    let package = fixture.package();

    let skipped = PackageAdapter::Npm
        .publish_dry_run(
            fixture.manifest.parent().unwrap(),
            &[(&package, fixture.manifest.as_path())],
            &[],
        )
        .expect("a dry run yarn cannot simulate must be skipped, not failed");

    assert_eq!(
        skipped,
        [package.display_name(true)],
        "a skip must reach the caller, which would otherwise report a validation that never ran"
    );
}

#[test]
fn a_validated_dry_run_reports_nothing_skipped() {
    let fixture = Fixture::permissive("4.9.3");
    let package = fixture.package();

    let skipped = PackageAdapter::Npm
        .publish_dry_run(
            fixture.manifest.parent().unwrap(),
            &[(&package, fixture.manifest.as_path())],
            &[],
        )
        .expect("the stub yarn accepts the dry run");

    assert!(
        skipped.is_empty(),
        "a yarn that simulated the publish leaves nothing unvalidated: {skipped:?}"
    );
    assert_eq!(fixture.argv(), ["--version", "npm publish --dry-run"]);
}

#[test]
fn yarn_berry_below_the_dry_run_floor_is_skipped_without_spawning_a_publish() {
    let fixture = Fixture::new("4.9.2");

    let outcome = PackageAdapter::Npm
        .publish(&fixture.manifest, true, &[])
        .expect("`yarn npm publish` gained --dry-run in 4.9.3");

    assert_eq!(outcome, PublishOutcome::DryRunSkipped);
    assert_eq!(
        fixture.argv(),
        ["--version"],
        "nothing but the version probe may be spawned"
    );
}

#[test]
fn yarn_berry_at_the_dry_run_floor_reaches_yarn_npm_publish() {
    let fixture = Fixture::new("4.9.3");

    let err = PackageAdapter::Npm
        .publish(&fixture.manifest, true, &[])
        .expect_err("the stub yarn fails the publish");
    assert!(
        err.to_string().contains("yarn npm publish failed"),
        "the failure must name the command that actually ran: {err}"
    );

    assert_eq!(
        fixture.argv(),
        ["--version", "npm publish --dry-run"],
        "a yarn with a dry run must run it, namespaced under `npm`"
    );
}

#[test]
fn a_real_publish_is_never_skipped_on_a_yarn_without_a_dry_run() {
    let fixture = Fixture::new("1.22.22");

    let err = PackageAdapter::Npm
        .publish(&fixture.manifest, false, &[])
        .expect_err("the stub yarn fails the publish");
    assert!(
        err.to_string().contains("yarn publish failed"),
        "the failure must name the command that actually ran: {err}"
    );

    assert_eq!(
        fixture.argv(),
        ["--version", "publish"],
        "the skip covers dry runs only; a real publish must still be spawned"
    );
}

#[test]
fn a_dry_run_asked_for_through_the_passthrough_is_skipped_too() {
    let fixture = Fixture::new("1.22.22");

    // `sampo publish -- --dry-run` reaches the adapter as a plain publish carrying the flag.
    PackageAdapter::Npm
        .publish(&fixture.manifest, false, &["--dry-run".to_string()])
        .expect("a dry run yarn cannot simulate must be skipped, not failed");

    assert_eq!(
        fixture.argv(),
        ["--version"],
        "nothing but the version probe may be spawned"
    );
}

#[test]
fn the_probe_wins_over_the_manifest_field() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let argv_log = root.join("yarn-argv.log");
    let bin = root.join("fake-bin");

    let manifest = write_yarn_package(root, "1.22.22");
    write_fake_yarn(&bin, &argv_log, "4.9.3", 1);
    let _guard = common::use_only(&bin);

    PackageAdapter::Npm
        .publish(&manifest, true, &[])
        .expect_err("the stub yarn fails the publish");

    assert_eq!(
        argv_lines(&argv_log),
        ["--version", "npm publish --dry-run"],
        "the reported version must decide, not the packageManager field"
    );
}
