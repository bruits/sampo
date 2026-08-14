//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use std::fs;
use std::path::Path;

const PKG_A_MANIFEST: &str = r#"{"name":"pkg-a","version":"1.0.0"}"#;
const PKG_A_PRERELEASE_MANIFEST: &str = r#"{"name":"pkg-a","version":"1.0.0-alpha.0"}"#;
const RUST_MANIFEST: &str = "[package]\nname = \"rust-pkg\"\nversion = \"1.0.0\"\n";
const JS_MANIFEST: &str = r#"{"name":"js-pkg","version":"1.0.0"}"#;

fn write_sampo(root: &Path, config: &str, preserved: &str) {
    fs::create_dir_all(root.join(".sampo/changesets")).unwrap();
    fs::create_dir_all(root.join(".sampo/prerelease")).unwrap();
    fs::write(root.join(".sampo/config.toml"), config).unwrap();
    // The run's only changeset, and preserved: the plan is unknown before the restore.
    fs::write(root.join(".sampo/prerelease/p.md"), preserved).unwrap();
}

fn write_bun_workspace(root: &Path, manifest: &str) {
    fs::write(root.join("bun.lock"), "{}\n").unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/pkg-a")).unwrap();
    fs::write(root.join("packages/pkg-a/package.json"), manifest).unwrap();
}

fn assert_refused_before_any_write(root: &Path, manifest: &str) {
    assert_eq!(
        fs::read_to_string(root.join("packages/pkg-a/package.json")).unwrap(),
        manifest,
        "manifest must be untouched"
    );
    assert!(
        root.join(".sampo/prerelease/p.md").exists(),
        "the preserved changeset must not be restored: the refusal comes first"
    );
    assert!(
        !root.join(".sampo/changesets/p.md").exists(),
        "the restore is a write, so it must not have happened"
    );
    assert!(
        !root.join("packages/pkg-a/CHANGELOG.md").exists(),
        "no changelog must be written"
    );
}

fn assert_names_the_missing_tool(err: &sampo_core::SampoError) {
    let message = err.to_string();
    assert!(
        message.contains("bun") && message.contains("PATH"),
        "must be the toolchain gate, naming the missing tool: {message}"
    );
}

/// A stable target on the preserved changeset is what puts `run_release` on its preview branch.
#[test]
fn preserved_stable_changeset_gates_before_the_restore() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_bun_workspace(root, PKG_A_MANIFEST);
    write_sampo(
        root,
        "[git]\nrelease_branches = [\"main\"]\n",
        "---\nnpm/pkg-a: minor\n---\n\nfeat: a change from the prerelease era\n",
    );
    let _guard = common::use_only(&root.join("empty-bin"));

    let err = sampo_core::run_release(root, false)
        .expect_err("a missing bun must fail the release, preserved changeset or not");
    assert_names_the_missing_tool(&err);
    assert_refused_before_any_write(root, PKG_A_MANIFEST);
}

#[test]
fn stabilize_gates_before_the_restore() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_bun_workspace(root, PKG_A_PRERELEASE_MANIFEST);
    write_sampo(
        root,
        "[git]\nrelease_branches = [\"main\"]\n",
        "---\nnpm/pkg-a: minor\n---\n\nfeat: a change to stabilize\n",
    );
    let _guard = common::use_only(&root.join("empty-bin"));

    let err = sampo_core::run_stabilize_release(root, false)
        .expect_err("a missing bun must fail stabilization too");
    assert_names_the_missing_tool(&err);
    assert_refused_before_any_write(root, PKG_A_PRERELEASE_MANIFEST);
}

/// No `Cargo.lock`, so only the npm gate can fire.
#[test]
fn a_fixed_group_widens_the_previewed_ecosystems() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

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
    fs::write(root.join("packages/js-pkg/package.json"), JS_MANIFEST).unwrap();

    fs::create_dir_all(root.join(".sampo/changesets")).unwrap();
    fs::create_dir_all(root.join(".sampo/prerelease")).unwrap();
    fs::write(
        root.join(".sampo/config.toml"),
        "[git]\nrelease_branches = [\"main\"]\n\
         [packages]\nfixed = [[\"cargo/rust-pkg\", \"npm/js-pkg\"]]\n",
    )
    .unwrap();
    fs::write(
        root.join(".sampo/prerelease/p.md"),
        "---\ncargo/rust-pkg: minor\n---\n\nfeat: a rust-only change\n",
    )
    .unwrap();

    let _guard = common::use_only(&root.join("empty-bin"));

    let err = sampo_core::run_release(root, false)
        .expect_err("the fixed group releases an npm package, so the npm toolchain is needed");
    assert_names_the_missing_tool(&err);

    assert_eq!(
        fs::read_to_string(root.join("crates/rust-pkg/Cargo.toml")).unwrap(),
        RUST_MANIFEST,
        "manifest must be untouched"
    );
    assert_eq!(
        fs::read_to_string(root.join("packages/js-pkg/package.json")).unwrap(),
        JS_MANIFEST,
        "manifest must be untouched"
    );
    assert!(
        root.join(".sampo/prerelease/p.md").exists(),
        "the preserved changeset must not be restored: the refusal comes first"
    );
    assert!(
        !root.join(".sampo/changesets/p.md").exists(),
        "the restore is a write, so it must not have happened"
    );
}
