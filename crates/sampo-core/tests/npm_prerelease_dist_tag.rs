//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use sampo_core::adapters::PackageAdapter;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Records every argv and succeeds, so the publish command is observed without a registry.
/// Yarn tells a Berry that has a dry run when probed.
fn write_fake_tool(dir: &Path, name: &str, argv_log: &Path) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join(name);
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then echo 4.9.3; exit 0; fi\n\
             printf '%s\\n' \"$*\" >> '{}'\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    temp: tempfile::TempDir,
    argv_log: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let argv_log = temp.path().join("argv.log");
        let bin = temp.path().join("fake-bin");
        for tool in ["npm", "pnpm", "yarn", "bun"] {
            write_fake_tool(&bin, tool, &argv_log);
        }
        let guard = common::use_only(&bin);

        Self {
            temp,
            argv_log,
            _guard: guard,
        }
    }

    /// The command line a dry-run publish of `@x/lib` at `version` spawns under `package_manager`.
    fn publish_command(
        &self,
        package_manager: &str,
        version: &str,
        extra_args: &[String],
    ) -> String {
        let dir = self.temp.path().join(package_manager.replace('@', "-"));
        fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("package.json");
        fs::write(
            &manifest,
            format!(
                r#"{{"name":"@x/lib","version":"{version}","packageManager":"{package_manager}"}}"#
            ),
        )
        .unwrap();
        let _ = fs::remove_file(&self.argv_log);

        PackageAdapter::Npm
            .publish(&manifest, true, extra_args)
            .expect("the stand-in package manager accepts the publish");

        fs::read_to_string(&self.argv_log)
            .expect("the package manager must have been spawned")
            .trim()
            .to_string()
    }
}

const PACKAGE_MANAGERS: [(&str, &str); 4] = [
    ("npm@11.12.1", "publish"),
    ("pnpm@11.1.2", "publish"),
    ("bun@1.2.0", "publish"),
    // Berry namespaces publishing under `npm`.
    ("yarn@4.9.3", "npm publish"),
];

#[test]
fn a_prerelease_is_published_under_its_first_identifier_by_every_package_manager() {
    let fixture = Fixture::new();

    for (package_manager, publish) in PACKAGE_MANAGERS {
        assert_eq!(
            fixture.publish_command(package_manager, "1.2.0-alpha.1", &[]),
            format!("{publish} --dry-run --access public --tag alpha"),
            "{package_manager}"
        );
    }
}

#[test]
fn a_stable_version_is_published_without_a_tag() {
    let fixture = Fixture::new();

    for (package_manager, publish) in PACKAGE_MANAGERS {
        assert_eq!(
            fixture.publish_command(package_manager, "1.2.0", &[]),
            format!("{publish} --dry-run --access public"),
            "{package_manager}"
        );
    }
}

#[test]
fn a_tag_passed_through_is_not_overridden_by_the_prerelease_one() {
    let fixture = Fixture::new();
    let passthrough = ["--tag".to_string(), "beta".to_string()];

    for (package_manager, publish) in PACKAGE_MANAGERS {
        assert_eq!(
            fixture.publish_command(package_manager, "1.2.0-alpha.1", &passthrough),
            format!("{publish} --dry-run --access public --tag beta"),
            "{package_manager}"
        );
    }
}
