use super::*;
use crate::types::PackageKind;
use std::collections::BTreeMap;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[test]
fn can_discover_detects_manifest() {
    let temp = tempfile::tempdir().unwrap();
    assert!(!can_discover(temp.path()));
    write_file(
        &temp.path().join("gleam.toml"),
        "name = \"app\"\nversion = \"1.0.0\"\n",
    );
    assert!(can_discover(temp.path()));
}

#[test]
fn discover_single_root_package() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("gleam.toml"),
        "name = \"my_app\"\nversion = \"1.2.3\"\n",
    );

    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);
    let pkg = &packages[0];
    assert_eq!(pkg.name, "my_app");
    assert_eq!(pkg.version, "1.2.3");
    assert_eq!(pkg.kind, PackageKind::Hex);
    assert_eq!(pkg.identifier, "hex/my_app");
}

#[test]
fn discover_finds_nested_packages() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("packages/a/gleam.toml"),
        "name = \"pkg_a\"\nversion = \"0.1.0\"\n",
    );
    write_file(
        &temp.path().join("packages/b/gleam.toml"),
        "name = \"pkg_b\"\nversion = \"0.2.0\"\n",
    );

    let mut names: Vec<String> = discover(temp.path())
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    names.sort();
    assert_eq!(names, vec!["pkg_a", "pkg_b"]);
}

#[test]
fn discover_skips_build_output() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("gleam.toml"),
        "name = \"app\"\nversion = \"1.0.0\"\n",
    );
    write_file(
        &temp.path().join("build/dev/erlang/dep/gleam.toml"),
        "name = \"vendored\"\nversion = \"9.9.9\"\n",
    );

    let names: Vec<String> = discover(temp.path())
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, vec!["app"]);
}

#[test]
fn discover_links_internal_path_dependency() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("packages/app/gleam.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib = { path = \"../lib\" }\ngleam_stdlib = \">= 0.34.0 and < 2.0.0\"\n",
    );
    write_file(
        &temp.path().join("packages/lib/gleam.toml"),
        "name = \"lib\"\nversion = \"0.5.0\"\n",
    );

    let packages = discover(temp.path()).unwrap();
    let app = packages.iter().find(|p| p.name == "app").unwrap();
    assert!(app.internal_deps.contains("hex/lib"));
    assert!(!app.internal_deps.contains("hex/gleam_stdlib"));
}

#[test]
fn discover_does_not_link_git_dependency_by_name() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("packages/app/gleam.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib = { git = \"https://example.com/lib.git\", ref = \"main\" }\n",
    );
    write_file(
        &temp.path().join("packages/lib/gleam.toml"),
        "name = \"lib\"\nversion = \"0.5.0\"\n",
    );

    let packages = discover(temp.path()).unwrap();
    let app = packages.iter().find(|p| p.name == "app").unwrap();
    assert!(app.internal_deps.is_empty());
}

#[test]
fn discover_skips_directory_owned_by_mix() {
    let temp = tempfile::tempdir().unwrap();
    // A directory carrying both manifests belongs to Mix (mix_gleam interop); the
    // Gleam scan must ignore it so the package is not discovered twice.
    write_file(
        &temp.path().join("gleam.toml"),
        "name = \"app\"\nversion = \"1.0.0\"\n",
    );
    write_file(
        &temp.path().join("mix.exs"),
        "defmodule App.MixProject do\nend\n",
    );

    assert!(!can_discover(temp.path()));
    assert!(discover(temp.path()).unwrap().is_empty());
}

#[test]
fn discover_errors_on_malformed_manifest() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("gleam.toml"),
        "name = \"app\"\nversion =\n",
    );
    assert!(discover(temp.path()).is_err());
}

#[test]
fn discover_respects_scan_depth_limit() {
    let temp = tempfile::tempdir().unwrap();
    // MAX_SCAN_DEPTH is 4: a manifest at depth 4 is found, one deeper is not.
    write_file(
        &temp.path().join("a/b/c/d/gleam.toml"),
        "name = \"at_limit\"\nversion = \"1.0.0\"\n",
    );
    write_file(
        &temp.path().join("a/b/c/d/e/gleam.toml"),
        "name = \"too_deep\"\nversion = \"1.0.0\"\n",
    );

    let names: Vec<String> = discover(temp.path())
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, vec!["at_limit"]);
}

#[test]
fn is_publishable_accepts_valid_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("gleam.toml");
    write_file(&manifest, "name = \"app\"\nversion = \"1.0.0\"\n");
    assert!(is_publishable(&manifest).unwrap());
}

#[test]
fn is_publishable_rejects_missing_version() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("gleam.toml");
    write_file(&manifest, "name = \"app\"\n");
    assert!(is_publishable(&manifest).is_err());
}

#[test]
fn update_manifest_bumps_own_version() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("gleam.toml");
    let input = "name = \"app\"\nversion = \"1.0.0\"\n";
    write_file(&manifest, input);

    let (output, applied) =
        update_manifest_versions(&manifest, input, Some("2.0.0"), &BTreeMap::new()).unwrap();
    assert!(output.contains("version = \"2.0.0\""));
    assert!(applied.is_empty());
}

#[test]
fn update_manifest_bumps_dependency_and_preserves_path_deps() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("gleam.toml");
    let input = "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\ngleam_stdlib = \"~> 0.34\"\nlocal = { path = \"../local\" }\n";
    write_file(&manifest, input);

    let mut targets = BTreeMap::new();
    targets.insert("gleam_stdlib".to_string(), "0.40.0".to_string());

    let (output, applied) = update_manifest_versions(&manifest, input, None, &targets).unwrap();
    assert!(output.contains("gleam_stdlib = \"~> 0.40.0\""));
    assert!(output.contains("local = { path = \"../local\" }"));
    assert_eq!(
        applied,
        vec![("gleam_stdlib".to_string(), "0.40.0".to_string())]
    );
}

#[test]
fn update_manifest_is_noop_without_changes() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("gleam.toml");
    let input = "name = \"app\"\nversion = \"1.0.0\"\n";
    write_file(&manifest, input);

    let (output, applied) =
        update_manifest_versions(&manifest, input, None, &BTreeMap::new()).unwrap();
    assert_eq!(output, input);
    assert!(applied.is_empty());
}

#[test]
fn find_dependency_constraint_reads_string_and_skips_path() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("gleam.toml");
    write_file(
        &manifest,
        "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\ngleam_stdlib = \"~> 0.34\"\nlocal = { path = \"../local\" }\n\n[dev_dependencies]\ngleeunit = \">= 1.0.0 and < 2.0.0\"\n",
    );

    assert_eq!(
        find_dependency_constraint_value(&manifest, "gleam_stdlib").unwrap(),
        Some("~> 0.34".to_string())
    );
    assert_eq!(
        find_dependency_constraint_value(&manifest, "gleeunit").unwrap(),
        Some(">= 1.0.0 and < 2.0.0".to_string())
    );
    assert_eq!(
        find_dependency_constraint_value(&manifest, "local").unwrap(),
        None
    );
    assert_eq!(
        find_dependency_constraint_value(&manifest, "absent").unwrap(),
        None
    );
}

#[test]
fn regenerate_lockfile_is_noop_without_existing_lockfile() {
    let temp = tempfile::tempdir().unwrap();
    // A Gleam package that has never been resolved has no manifest.toml. Regeneration
    // must skip it — not invoke `gleam` (which would fail when absent) and not create a
    // lockfile the user never generated.
    write_file(
        &temp.path().join("gleam.toml"),
        "name = \"app\"\nversion = \"1.0.0\"\n",
    );

    assert!(regenerate_lockfile(temp.path()).is_ok());
    assert!(!temp.path().join("manifest.toml").exists());
}
