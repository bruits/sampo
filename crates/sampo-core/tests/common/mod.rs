//! Fixtures shared by the integration tests that stub package-manager tools.
//!
//! Compiled into each of those test binaries, hence the blanket allow.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Compared verbatim, so a prerelease bump cannot pass for untouched.
pub const PKG_A_MANIFEST: &str = r#"{"name":"pkg-a","version":"1.0.0"}"#;

/// A bun workspace with one member and a committed `bun.lock`, ready to release.
pub fn write_workspace(root: &Path) {
    // Selects the bun branch in both package-manager detection and the lockfile gate.
    fs::write(root.join("bun.lock"), "{}\n").unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/pkg-a")).unwrap();
    fs::write(root.join("packages/pkg-a/package.json"), PKG_A_MANIFEST).unwrap();

    fs::create_dir_all(root.join(".sampo/changesets")).unwrap();
    fs::write(
        root.join(".sampo/config.toml"),
        "[git]\nrelease_branches = [\"main\"]\n",
    )
    .unwrap();
    fs::write(
        root.join(".sampo/changesets/c.md"),
        "---\nnpm/pkg-a: minor\n---\n\nfeat: a change\n",
    )
    .unwrap();
}

pub fn write_bun_with_version_body(dir: &Path, argv_log: &Path, version_body: &str) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join("bun");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then {version_body}; fi\n\
             printf '%s\\n' \"$*\" >> '{}'\n\
             exit 1\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn write_fake_bun(dir: &Path, argv_log: &Path, version: &str) {
    write_bun_with_version_body(dir, argv_log, &format!("echo {version}; exit 0"));
}

static ORIGINAL_PATH: std::sync::OnceLock<std::ffi::OsString> = std::sync::OnceLock::new();

/// The `PATH` this test binary started with, which [`use_only`] replaces and never restores.
pub fn original_path() -> &'static std::ffi::OsStr {
    ORIGINAL_PATH
        .get()
        .expect("use_only records the original PATH")
}

/// Point the release at `main` and make `dir` the only place tools are found.
///
/// The environment is process-wide; hold the guard for the whole test body.
#[must_use = "the returned guard keeps sibling tests out of this test's environment"]
pub fn use_only(dir: &Path) -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    ORIGINAL_PATH.get_or_init(|| std::env::var_os("PATH").unwrap_or_default());
    unsafe {
        std::env::set_var("SAMPO_RELEASE_BRANCH", "main");
        std::env::set_var("PATH", dir);
        // Set on GitHub Actions, where it sends changelog enrichment down another path.
        std::env::remove_var("GITHUB_REPOSITORY");
    }
    guard
}

pub fn assert_workspace_untouched(root: &Path) {
    assert_eq!(
        fs::read_to_string(root.join("packages/pkg-a/package.json")).unwrap(),
        PKG_A_MANIFEST,
        "manifest must be untouched"
    );
    assert!(
        root.join(".sampo/changesets/c.md").exists(),
        "changeset must not be consumed"
    );
    assert!(
        !root.join("packages/pkg-a/CHANGELOG.md").exists(),
        "no changelog must be written"
    );
}

pub fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// Commit `root` on `main` with `config` as `.sampo/config.toml`, making the real `git`
/// available in `root/fake-bin`, the tool directory given to [`use_only`].
pub fn commit_repo(root: &Path, config: &str) {
    std::os::unix::fs::symlink(which_git(), root.join("fake-bin/git")).unwrap();
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

fn which_git() -> PathBuf {
    std::env::split_paths(original_path())
        .map(|dir| dir.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("git on PATH")
}
