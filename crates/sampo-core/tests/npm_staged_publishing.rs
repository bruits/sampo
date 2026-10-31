//! Its own test binary: it overrides `PATH` and the publish tokens process-wide.
#![cfg(unix)]

mod common;

use sampo_core::adapters::{PackageAdapter, PublishOutcome};
use sampo_core::errors::SampoError;
use sampo_core::publish::PublishExtraArgs;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write_stub(dir: &Path, program: &str, version: &str, argv_log: &Path) {
    let stub = dir.join(program);
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '{program} %s\\n' \"$*\" >> '{}'\n\
             if [ \"$1\" = \"--version\" ]; then echo {version}; fi\n\
             if [ \"$1 $2\" = \"pm pack\" ]; then : > \"$4/pkg.tgz\"; fi\nexit 0\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    temp: tempfile::TempDir,
    bin: PathBuf,
    argv_log: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(package_manager: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("fake-bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(
            temp.path().join("package.json"),
            format!(
                r#"{{"name":"@s/pkg","version":"1.0.0-beta.1","packageManager":"{package_manager}"}}"#
            ),
        )
        .unwrap();

        let guard = common::use_only(&bin);
        unsafe {
            for var in [
                "NPM_TOKEN",
                "NODE_AUTH_TOKEN",
                "NPM_CONFIG_TOKEN",
                "npm_config_token",
                "BUN_CONFIG_TOKEN",
                "NPM_ID_TOKEN",
                "ACTIONS_ID_TOKEN_REQUEST_URL",
                "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
            ] {
                std::env::remove_var(var);
            }
        }

        let argv_log = temp.path().join("argv.log");
        Self {
            temp,
            bin,
            argv_log,
            _guard: guard,
        }
    }

    fn stub(self, program: &str, version: &str) -> Self {
        write_stub(&self.bin, program, version, &self.argv_log);
        self
    }

    fn stage(&self, dry_run: bool) -> Result<PublishOutcome, SampoError> {
        PackageAdapter::Npm.stage(&self.temp.path().join("package.json"), dry_run, &[])
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
fn npm_stages_with_the_usual_flags() {
    let fixture = Fixture::new("npm@11.15.0").stub("npm", "11.15.0");

    assert_eq!(fixture.stage(false).unwrap(), PublishOutcome::Staged);
    assert_eq!(
        fixture.argv(),
        [
            "npm --version",
            "npm stage publish --access public --tag beta"
        ]
    );
}

#[test]
fn the_dry_run_validates_the_stage_command() {
    let fixture = Fixture::new("npm@11.15.0").stub("npm", "11.19.0");

    assert_eq!(fixture.stage(true).unwrap(), PublishOutcome::Ran);
    assert_eq!(
        fixture.argv().last().unwrap(),
        "npm stage publish --dry-run --access public --tag beta"
    );
}

#[test]
fn an_npm_without_stage_stops_before_publishing() {
    let fixture = Fixture::new("npm@11.14.0").stub("npm", "11.14.0");

    let err = fixture.stage(false).unwrap_err().to_string();

    assert!(err.contains("11.15.0") && err.contains("11.14.0"), "{err}");
    assert_eq!(fixture.argv(), ["npm --version"]);
}

#[test]
fn pnpm_stages_with_its_own_command() {
    let fixture = Fixture::new("pnpm@11.3.0").stub("pnpm", "11.3.0");

    assert_eq!(fixture.stage(false).unwrap(), PublishOutcome::Staged);
    assert_eq!(
        fixture.argv(),
        [
            "pnpm --version",
            "pnpm stage publish --access public --tag beta --no-git-checks"
        ]
    );
}

#[test]
fn a_pnpm_without_stage_stops_before_it_runs_a_stage_script() {
    let fixture = Fixture::new("pnpm@11.1.2").stub("pnpm", "11.1.2");

    let err = fixture.stage(false).unwrap_err().to_string();

    assert!(err.contains("11.3.0") && err.contains("11.1.2"), "{err}");
    assert_eq!(fixture.argv(), ["pnpm --version"]);
}

#[test]
fn yarn_berry_stages_through_its_flag() {
    let fixture = Fixture::new("yarn@4.16.0").stub("yarn", "4.16.0");

    assert_eq!(fixture.stage(false).unwrap(), PublishOutcome::Staged);
    assert_eq!(
        fixture.argv(),
        [
            "yarn --version",
            "yarn npm publish --staged --access public --tag beta"
        ]
    );
}

#[test]
fn yarn_classic_cannot_stage() {
    let fixture = Fixture::new("yarn@1.22.22").stub("yarn", "1.22.22");

    let err = fixture.stage(true).unwrap_err().to_string();

    assert!(err.contains("Yarn Classic"), "{err}");
    assert_eq!(fixture.argv(), ["yarn --version"]);
}

#[test]
fn bun_stages_through_npm_even_with_a_token() {
    let fixture = Fixture::new("bun@1.4.4")
        .stub("bun", "1.4.4")
        .stub("npm", "11.15.0");
    unsafe { std::env::set_var("NPM_CONFIG_TOKEN", "token") };

    assert_eq!(fixture.stage(false).unwrap(), PublishOutcome::Staged);
    let argv = fixture.argv();
    let pack_dir = argv[1]
        .strip_prefix("bun pm pack --destination ")
        .expect("bun packed the package");
    assert_eq!(
        argv,
        [
            "npm --version".to_string(),
            format!("bun pm pack --destination {pack_dir}"),
            format!("npm stage publish {pack_dir}/pkg.tgz --access public --tag beta"),
        ]
    );
}

/// Answers 404 to every request, as npm does for a version it holds staged.
fn serve_missing_versions() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.read(&mut [0; 4096]);
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    format!("http://{address}/")
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn commit_repo(fixture: &Fixture, config: &str) {
    let root = fixture.temp.path();
    std::os::unix::fs::symlink(which_git(), fixture.bin.join("git")).unwrap();
    unsafe {
        for (key, value) in [
            ("GIT_AUTHOR_NAME", "t"),
            ("GIT_AUTHOR_EMAIL", "t@t"),
            ("GIT_COMMITTER_NAME", "t"),
            ("GIT_COMMITTER_EMAIL", "t@t"),
            // Keep the developer's git config (e.g. signing) out of the commit and the tag.
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ] {
            std::env::set_var(key, value);
        }
    }

    fs::create_dir_all(root.join(".sampo")).unwrap();
    fs::write(root.join(".sampo/config.toml"), config).unwrap();
    fs::write(root.join(".gitignore"), "fake-bin/\nargv.log\n").unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "init"]);
}

#[test]
fn a_staged_and_tagged_version_is_not_staged_again() {
    let fixture = Fixture::new("npm@11.15.0").stub("npm", "11.15.0");
    let root = fixture.temp.path();
    fs::write(
        root.join("package.json"),
        format!(
            r#"{{"name":"pkg","version":"1.0.0","packageManager":"npm@11.15.0","publishConfig":{{"registry":"{}"}}}}"#,
            serve_missing_versions()
        ),
    )
    .unwrap();
    commit_repo(&fixture, "[packages]\nnpm_stage = true\n");

    let first = sampo_core::run_publish(root, false, &PublishExtraArgs::default()).unwrap();
    assert_eq!(first.tags, ["pkg-v1.0.0"]);
    let stages = || {
        fixture
            .argv()
            .iter()
            .filter(|line| line.starts_with("npm stage publish"))
            .count()
    };
    assert_eq!(stages(), 2, "a dry run, then the stage");

    let second = sampo_core::run_publish(root, false, &PublishExtraArgs::default()).unwrap();
    assert!(second.tags.is_empty(), "{:?}", second.tags);
    assert_eq!(stages(), 2, "nothing staged again");
}

#[test]
fn a_list_stages_only_the_packages_it_names() {
    let fixture = Fixture::new("npm@11.15.0").stub("npm", "11.15.0");
    let root = fixture.temp.path();
    fs::write(
        root.join("package.json"),
        r#"{"name":"root","private":true,"workspaces":["packages/*"],"packageManager":"npm@11.15.0"}"#,
    )
    .unwrap();
    let registry = serve_missing_versions();
    for name in ["a", "b"] {
        fs::create_dir_all(root.join("packages").join(name)).unwrap();
        fs::write(
            root.join("packages").join(name).join("package.json"),
            // A registry path per package tells their publish commands apart.
            format!(
                r#"{{"name":"{name}","version":"1.0.0","publishConfig":{{"registry":"{registry}{name}/"}}}}"#
            ),
        )
        .unwrap();
    }
    commit_repo(&fixture, "[packages]\nnpm_stage = [\"npm/a\"]\n");

    let output = sampo_core::run_publish(root, false, &PublishExtraArgs::default()).unwrap();

    assert_eq!(output.tags, ["a-v1.0.0", "b-v1.0.0"]);
    let argv = fixture.argv();
    let runs = |prefix: &str, name: &str| {
        argv.iter()
            .filter(|line| line.starts_with(prefix) && line.contains(&format!("/{name}/")))
            .count()
    };
    assert_eq!(runs("npm stage publish", "a"), 2, "{argv:?}");
    assert_eq!(runs("npm publish", "b"), 2, "{argv:?}");
    assert_eq!(
        runs("npm stage publish", "b") + runs("npm publish", "a"),
        0,
        "{argv:?}"
    );
}

fn which_git() -> PathBuf {
    std::env::split_paths(common::original_path())
        .map(|dir| dir.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("git on PATH")
}
