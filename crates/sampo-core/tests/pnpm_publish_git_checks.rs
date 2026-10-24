//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use sampo_core::adapters::{PackageAdapter, PublishOutcome};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Records every argv. pnpm 11 and earlier exit 0 even when the branch prompt stops them, so
/// only the argv shows whether the publish could go through.
fn write_fake_pnpm(dir: &Path, argv_log: &Path) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join("pnpm");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    _temp: tempfile::TempDir,
    manifest: PathBuf,
    argv_log: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let argv_log = root.join("pnpm-argv.log");
        let bin = root.join("fake-bin");

        let manifest = root.join("package.json");
        fs::write(
            &manifest,
            r#"{"name":"pkg","version":"1.0.0","packageManager":"pnpm@11.1.2"}"#,
        )
        .unwrap();
        write_fake_pnpm(&bin, &argv_log);
        let guard = common::use_only(&bin);

        Self {
            _temp: temp,
            manifest,
            argv_log,
            _guard: guard,
        }
    }

    fn publish(&self, dry_run: bool) {
        let outcome = PackageAdapter::Npm
            .publish(&self.manifest, dry_run, &[])
            .expect("the stand-in pnpm succeeds");
        assert_eq!(outcome, PublishOutcome::Ran);
    }

    fn argv(&self) -> Vec<String> {
        fs::read_to_string(&self.argv_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

#[test]
fn a_real_publish_turns_pnpm_git_checks_off() {
    let fixture = Fixture::new();

    fixture.publish(false);

    assert_eq!(fixture.argv(), ["publish --no-git-checks"]);
}

#[test]
fn the_dry_run_turns_pnpm_git_checks_off_too() {
    let fixture = Fixture::new();

    fixture.publish(true);

    assert_eq!(
        fixture.argv(),
        ["publish --dry-run --no-git-checks"],
        "a dry run that stops at the prompt validates nothing yet reports success"
    );
}
