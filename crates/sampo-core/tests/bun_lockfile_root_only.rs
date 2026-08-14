//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const ROOT_MANIFEST: &str = r#"{"name":"pkg-root","version":"1.0.0"}"#;

fn write_root_only_repo(root: &Path, lockfile: &str) {
    fs::write(root.join("bun.lock"), lockfile).unwrap();
    fs::write(root.join("package.json"), ROOT_MANIFEST).unwrap();

    fs::create_dir_all(root.join(".sampo/changesets")).unwrap();
    fs::write(
        root.join(".sampo/config.toml"),
        "[git]\nrelease_branches = [\"main\"]\n",
    )
    .unwrap();
    fs::write(
        root.join(".sampo/changesets/c.md"),
        "---\nnpm/pkg-root: minor\n---\n\nfeat: a change\n",
    )
    .unwrap();
}

fn write_silently_successful_bun(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join("bun");
    fs::write(
        &stub,
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then echo 1.3.14; exit 0; fi\n\
         exit 0\n",
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

/// bun writes the root's `""` entry with a `name` and no `version`.
#[test]
fn a_root_only_repo_has_nothing_to_verify() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let bin = root.join("fake-bin");

    // Verbatim the shape bun emits for a repo without workspaces.
    write_root_only_repo(root, "{\"workspaces\":{\"\":{\"name\":\"pkg-root\"}}}\n");
    write_silently_successful_bun(&bin);
    let _guard = common::use_only(&bin);

    sampo_core::run_release(root, false).expect("root-only release is not blocked");

    assert!(
        fs::read_to_string(root.join("package.json"))
            .unwrap()
            .contains("1.1.0"),
        "the release must have bumped the manifest"
    );
    assert_eq!(
        fs::read_to_string(root.join("bun.lock")).unwrap(),
        "{\"workspaces\":{\"\":{\"name\":\"pkg-root\"}}}\n",
        "the fake bun refreshed nothing, and the release shipped anyway"
    );
}

/// The case above is vacuous only through bun's omission: the root key is checked.
#[test]
fn a_root_entry_carrying_a_version_is_verified() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let bin = root.join("fake-bin");

    write_root_only_repo(
        root,
        "{\"workspaces\":{\"\":{\"name\":\"pkg-root\",\"version\":\"1.0.0\"}}}\n",
    );
    write_silently_successful_bun(&bin);
    let _guard = common::use_only(&bin);

    let err = sampo_core::run_release(root, false).expect_err("a stale root entry must fail");
    let message = err.to_string();
    assert!(
        message.contains("pkg-root") && message.contains("1.0.0") && message.contains("1.1.0"),
        "must name the package and both versions: {message}"
    );
}
