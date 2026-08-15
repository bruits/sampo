//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use sampo_core::adapters::PackageAdapter;
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

/// Records every argv. Anything but `--version` fails, which keeps the spawned case off
/// the network.
fn write_fake_yarn(dir: &Path, argv_log: &Path, version: &str) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join("yarn");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$*\" >> '{}'\n\
             if [ \"$1\" = \"--version\" ]; then echo {version}; exit 0; fi\n\
             exit 1\n",
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
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let argv_log = root.join("yarn-argv.log");
        let bin = root.join("fake-bin");

        let manifest = write_yarn_package(root, yarn_version);
        write_fake_yarn(&bin, &argv_log, yarn_version);
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
}

#[test]
fn yarn_classic_dry_run_is_skipped_without_spawning_a_publish() {
    let fixture = Fixture::new("1.22.22");

    PackageAdapter::Npm
        .publish(&fixture.manifest, true, &[])
        .expect("a dry run yarn cannot simulate must be skipped, not failed");

    assert_eq!(
        fixture.argv(),
        ["--version"],
        "nothing but the version probe may be spawned"
    );
}

#[test]
fn yarn_berry_below_the_dry_run_floor_is_skipped_without_spawning_a_publish() {
    let fixture = Fixture::new("4.9.2");

    PackageAdapter::Npm
        .publish(&fixture.manifest, true, &[])
        .expect("`yarn npm publish` gained --dry-run in 4.9.3");

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
        err.to_string().contains("pkg"),
        "the failure must be the spawned publish: {err}"
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
        err.to_string().contains("pkg"),
        "the failure must be the spawned publish: {err}"
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
    write_fake_yarn(&bin, &argv_log, "4.9.3");
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
