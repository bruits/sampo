//! Its own test binary: it overrides `PATH` and the CI identity process-wide.
#![cfg(unix)]

mod common;

use sampo_core::adapters::{PackageAdapter, PublishOutcome};
use sampo_core::errors::SampoError;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const TARBALL: &str = "s-pkg-1.0.0-beta.1.tgz";

fn write_stub(dir: &Path, program: &str, body: &str, argv_log: &Path) {
    let stub = dir.join(program);
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '{program} %s\\n' \"$*\" >> '{}'\n{body}\nexit 0\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    _temp: tempfile::TempDir,
    bin: PathBuf,
    manifest: PathBuf,
    argv_log: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(scripts: &str, npm_version: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let argv_log = root.join("argv.log");
        let bin = root.join("fake-bin");
        fs::create_dir_all(&bin).unwrap();

        let manifest = root.join("package.json");
        fs::write(
            &manifest,
            format!(
                r#"{{"name":"@s/pkg","version":"1.0.0-beta.1","packageManager":"bun@1.4.4","scripts":{{{scripts}}}}}"#
            ),
        )
        .unwrap();
        write_stub(
            &bin,
            "bun",
            &format!(r#"if [ "$1 $2" = "pm pack" ]; then : > "$4/{TARBALL}"; fi"#),
            &argv_log,
        );
        write_stub(
            &bin,
            "npm",
            &format!(r#"if [ "$1" = "--version" ]; then echo {npm_version}; fi"#),
            &argv_log,
        );

        let guard = common::use_only(&bin);
        unsafe {
            std::env::set_var(
                "ACTIONS_ID_TOKEN_REQUEST_URL",
                "https://token.actions.example/",
            );
            std::env::set_var("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "request-token");
            for token_var in [
                "NPM_TOKEN",
                "NODE_AUTH_TOKEN",
                "NPM_CONFIG_TOKEN",
                "npm_config_token",
                "BUN_CONFIG_TOKEN",
                "NPM_ID_TOKEN",
            ] {
                std::env::remove_var(token_var);
            }
        }

        Self {
            _temp: temp,
            bin,
            manifest,
            argv_log,
            _guard: guard,
        }
    }

    fn publish(&self, dry_run: bool, extra_args: &[String]) -> Result<PublishOutcome, SampoError> {
        PackageAdapter::Npm.publish(&self.manifest, dry_run, extra_args)
    }

    fn argv(&self) -> Vec<String> {
        fs::read_to_string(&self.argv_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn pack_dir(&self) -> PathBuf {
        let argv = self.argv();
        let pack = argv
            .iter()
            .find_map(|line| line.strip_prefix("bun pm pack --destination "))
            .expect("bun packed the package");
        PathBuf::from(pack)
    }
}

#[test]
fn bun_packs_and_npm_publishes_the_tarball_with_the_usual_flags() {
    let fixture = Fixture::new("", "11.5.1");

    let outcome = fixture
        .publish(false, &["--loglevel=verbose".to_string()])
        .unwrap();

    assert_eq!(outcome, PublishOutcome::Ran);
    let pack_dir = fixture.pack_dir();
    assert_eq!(
        fixture.argv(),
        [
            "npm --version".to_string(),
            format!("bun pm pack --destination {}", pack_dir.display()),
            format!(
                "npm publish {}/{TARBALL} --access public --tag beta --loglevel=verbose",
                pack_dir.display()
            ),
        ]
    );
    assert!(!pack_dir.exists(), "the tarball must be cleaned up");
}

#[test]
fn the_dry_run_packs_and_validates_through_npm_too() {
    let fixture = Fixture::new("", "11.19.0");

    fixture.publish(true, &[]).unwrap();

    let last = fixture.argv().pop().unwrap();
    assert!(
        last.starts_with("npm publish ") && last.ends_with(" --dry-run --access public --tag beta"),
        "{last}"
    );
}

#[test]
fn the_scripts_bun_publish_runs_still_run_around_the_upload() {
    let fixture = Fixture::new(
        r#""prepublishOnly":"bun run build","publish":"echo","postpublish":"echo""#,
        "11.5.1",
    );

    fixture.publish(false, &[]).unwrap();

    let argv = fixture.argv();
    assert_eq!(argv[1], "bun run prepublishOnly");
    assert!(argv[2].starts_with("bun pm pack "));
    assert!(argv[3].starts_with("npm publish "));
    assert_eq!(argv[4..], ["bun run publish", "bun run postpublish"]);
}

#[test]
fn a_token_keeps_bun_publish() {
    let fixture = Fixture::new("", "11.5.1");
    unsafe { std::env::set_var("NPM_CONFIG_TOKEN", "token") };

    fixture.publish(false, &[]).unwrap();

    assert_eq!(fixture.argv(), ["bun publish --access public --tag beta"]);
}

#[test]
fn an_npm_too_old_for_trusted_publishing_stops_before_packing() {
    let fixture = Fixture::new("", "10.9.9");

    let err = fixture.publish(false, &[]).unwrap_err().to_string();

    assert!(err.contains("11.5.1") && err.contains("10.9.9"), "{err}");
    assert_eq!(fixture.argv(), ["npm --version"]);
}

#[test]
fn a_pack_without_a_tarball_stops_before_npm_publish() {
    let fixture = Fixture::new("", "11.5.1");
    write_stub(&fixture.bin, "bun", "", &fixture.argv_log);

    let err = fixture.publish(false, &[]).unwrap_err().to_string();

    assert!(err.contains("wrote no tarball"), "{err}");
    assert!(fixture.argv().last().unwrap().starts_with("bun pm pack "));
}

#[test]
fn a_failed_pack_stops_before_npm_publish() {
    let fixture = Fixture::new("", "11.5.1");
    write_stub(&fixture.bin, "bun", "exit 1", &fixture.argv_log);

    let err = fixture.publish(false, &[]).unwrap_err().to_string();

    assert!(err.contains("bun pm pack failed"), "{err}");
    assert!(fixture.argv().last().unwrap().starts_with("bun pm pack "));
}
