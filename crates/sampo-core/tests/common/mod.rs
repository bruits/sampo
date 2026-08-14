//! Fixtures shared by the bun lockfile integration tests.
//!
//! Compiled into each of those test binaries, hence the blanket allow.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

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

/// A `bun` stand-in reporting `version`, logging every other invocation to `argv_log`
/// and failing it.
pub fn write_fake_bun(dir: &Path, argv_log: &Path, version: &str) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join("bun");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then echo {version}; exit 0; fi\n\
             printf '%s\\n' \"$*\" >> '{}'\n\
             exit 1\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Point the release at `main` and make `dir` the only place tools are found.
///
/// The environment is process-wide; hold the guard for the whole test body.
#[must_use = "the returned guard keeps sibling tests out of this test's environment"]
pub fn use_only(dir: &Path) -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    unsafe {
        std::env::set_var("SAMPO_RELEASE_BRANCH", "main");
        std::env::set_var("PATH", dir);
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
