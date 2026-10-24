//! Its own test binary: these releases regenerate `Cargo.lock` with the real `cargo`, which the
//! lib's publish tests shadow with a stand-in on `PATH` while they run.
#![cfg(unix)]

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

const ROOT_MANIFEST: &str = "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n";
const WORKSPACE_PIN: &str =
    "\n[workspace.dependencies]\na-lib = { path = \"crates/a-lib\", version = \"=0.1.0\" }\n";
const DIRECT_PIN: &str = r#"a-lib = { path = "../a-lib", version = "=0.1.0" }"#;
const DEFAULT_CONFIG: &str = "[git]\nrelease_branches = [\"main\"]\n";

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// A workspace where `a-lib` is released as a patch (0.1.0 -> 0.1.1) and each dependent
/// declares it with the given `[dependencies]` line, with a generated `Cargo.lock`: a release
/// regenerates the lockfile when one exists.
fn write_workspace(root: &Path, root_manifest: &str, dependents: &[(&str, &str)], config: &str) {
    write_file(&root.join("Cargo.toml"), root_manifest);
    write_file(
        &root.join("crates/a-lib/Cargo.toml"),
        "[package]\nname = \"a-lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write_file(&root.join("crates/a-lib/src/lib.rs"), "");
    for (name, dependency) in dependents {
        write_file(
            &root.join(format!("crates/{name}/Cargo.toml")),
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{dependency}\n"
            ),
        );
        write_file(&root.join(format!("crates/{name}/src/lib.rs")), "");
    }

    write_file(&root.join(".sampo/config.toml"), config);
    write_file(
        &root.join(".sampo/changesets/fix.md"),
        "---\ncargo/a-lib: patch\n---\n\nfix: a change\n",
    );

    let status = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "cargo generate-lockfile failed");
}

fn release(root: &Path) {
    sampo_core::run_release(root, false).unwrap_or_else(|err| panic!("release failed: {err}"));
}

fn read_toml(path: &Path) -> toml::Value {
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// The `a-lib` requirement in a crate's `[dependencies]`.
fn dependency_requirement(root: &Path, crate_name: &str) -> String {
    let manifest = read_toml(&root.join(format!("crates/{crate_name}/Cargo.toml")));
    manifest["dependencies"]["a-lib"]["version"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The `a-lib` requirement in the root's `[workspace.dependencies]`.
fn workspace_requirement(root: &Path) -> String {
    let manifest = read_toml(&root.join("Cargo.toml"));
    manifest["workspace"]["dependencies"]["a-lib"]["version"]
        .as_str()
        .unwrap()
        .to_string()
}

fn locked_version(root: &Path, name: &str) -> String {
    read_toml(&root.join("Cargo.lock"))["package"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"].as_str() == Some(name))
        .and_then(|package| package["version"].as_str())
        .unwrap()
        .to_string()
}

#[test]
fn exact_pin_follows_the_release_and_the_lockfile_regenerates() {
    let _guard = common::pin_release_branch();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_workspace(
        root,
        ROOT_MANIFEST,
        &[("b-app", DIRECT_PIN)],
        DEFAULT_CONFIG,
    );

    release(root);

    assert_eq!(dependency_requirement(root, "b-app"), "=0.1.1");
    assert_eq!(locked_version(root, "a-lib"), "0.1.1");
    assert_eq!(locked_version(root, "b-app"), "0.1.1");
}

#[test]
fn workspace_exact_pin_follows_the_release_and_the_lockfile_regenerates() {
    let _guard = common::pin_release_branch();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_workspace(
        root,
        &format!("{ROOT_MANIFEST}{WORKSPACE_PIN}"),
        &[("b-app", "a-lib.workspace = true")],
        DEFAULT_CONFIG,
    );

    release(root);

    assert_eq!(workspace_requirement(root), "=0.1.1");
    assert_eq!(locked_version(root, "a-lib"), "0.1.1");
    assert_eq!(locked_version(root, "b-app"), "0.1.1");
}

#[test]
fn exact_pins_do_not_violate_fixed_group_constraints() {
    // `=0.1.0` does not match 0.1.1, but Sampo rewrites the pin instead of validating it as a
    // range, whether it sits in the crate or is inherited from the workspace root.
    let _guard = common::pin_release_branch();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_workspace(
        root,
        &format!("{ROOT_MANIFEST}{WORKSPACE_PIN}"),
        &[("b-app", DIRECT_PIN), ("c-app", "a-lib.workspace = true")],
        "[packages]\nfixed = [[\"cargo/a-lib\", \"cargo/b-app\", \"cargo/c-app\"]]\n",
    );

    release(root);

    assert_eq!(dependency_requirement(root, "b-app"), "=0.1.1");
    assert_eq!(workspace_requirement(root), "=0.1.1");
}

#[test]
fn partial_exact_requirement_is_a_range_and_stays() {
    // `=0.1` means `>=0.1.0, <0.2.0`, which the patch release still satisfies.
    let _guard = common::pin_release_branch();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_workspace(
        root,
        ROOT_MANIFEST,
        &[(
            "b-app",
            r#"a-lib = { path = "../a-lib", version = "=0.1" }"#,
        )],
        DEFAULT_CONFIG,
    );

    release(root);

    assert_eq!(dependency_requirement(root, "b-app"), "=0.1");
    assert_eq!(locked_version(root, "a-lib"), "0.1.1");
}
