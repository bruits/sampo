//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use std::fs;
use std::path::Path;

const RUST_MANIFEST: &str = "[package]\nname = \"rust-pkg\"\nversion = \"1.0.0\"\n";

/// No `Cargo.lock`, so only the npm gate can fire.
fn write_mixed_workspace(root: &Path, changeset: &str) {
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/rust-pkg/src")).unwrap();
    fs::write(root.join("crates/rust-pkg/Cargo.toml"), RUST_MANIFEST).unwrap();
    fs::write(root.join("crates/rust-pkg/src/lib.rs"), "").unwrap();

    fs::write(root.join("bun.lock"), "{}\n").unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/js-pkg")).unwrap();
    fs::write(
        root.join("packages/js-pkg/package.json"),
        r#"{"name":"js-pkg","version":"1.0.0"}"#,
    )
    .unwrap();

    fs::create_dir_all(root.join(".sampo/changesets")).unwrap();
    fs::write(
        root.join(".sampo/config.toml"),
        "[git]\nrelease_branches = [\"main\"]\n",
    )
    .unwrap();
    fs::write(root.join(".sampo/changesets/c.md"), changeset).unwrap();
}

#[test]
fn an_untouched_ecosystem_does_not_gate_the_release() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_mixed_workspace(
        root,
        "---\ncargo/rust-pkg: minor\n---\n\nfeat: a rust-only change\n",
    );
    let _guard = common::use_only(&root.join("empty-bin"));

    sampo_core::run_release(root, false).expect("no npm package is released, so bun is not needed");
    assert!(
        fs::read_to_string(root.join("crates/rust-pkg/Cargo.toml"))
            .unwrap()
            .contains("1.1.0"),
        "the rust crate was released"
    );
}

#[test]
fn a_released_ecosystem_still_gates_the_release() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_mixed_workspace(root, "---\nnpm/js-pkg: minor\n---\n\nfeat: a js change\n");
    let _guard = common::use_only(&root.join("empty-bin"));

    let err =
        sampo_core::run_release(root, false).expect_err("an npm release needs the npm toolchain");
    let message = err.to_string();
    assert!(
        message.contains("bun") && message.contains("PATH"),
        "blocked on the npm toolchain: {message}"
    );
    assert_eq!(
        fs::read_to_string(root.join("crates/rust-pkg/Cargo.toml")).unwrap(),
        RUST_MANIFEST,
        "nothing was written before the refusal"
    );
}
