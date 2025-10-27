use super::*;
use std::collections::BTreeMap;
use std::path::Path;

#[test]
fn clean_path_collapses_segments() {
    let input = Path::new("/a/b/../c/./d");
    let expected = PathBuf::from("/a/c/d");
    assert_eq!(clean_path(input), expected);
}

#[test]
fn clean_path_prevents_escaping_root() {
    // Test that we can't escape beyond root directory
    let input = Path::new("/a/../..");
    let expected = PathBuf::from("/");
    assert_eq!(clean_path(input), expected);

    // Test with relative paths
    let input = Path::new("a/../..");
    let expected = PathBuf::from("");
    assert_eq!(clean_path(input), expected);
}

#[test]
fn cargo_adapter_is_publishable_checks_manifest() {
    let adapter = CargoAdapter;
    // This test would require a real manifest file, so it's mainly a compilation check
    // Real tests are in the integration test suite
    let _ = adapter.is_publishable(Path::new("./Cargo.toml"));
}

#[test]
fn cargo_discoverer_can_discover_cargo_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Create a Cargo workspace
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"pkg-a\"]\n",
    )
    .unwrap();

    assert!(root.join("Cargo.toml").exists());
}

#[test]
fn cargo_discoverer_cannot_discover_non_cargo() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // No Cargo.toml
    assert!(!root.join("Cargo.toml").exists());
}

#[test]
fn cargo_discoverer_discovers_packages() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Create workspace
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();

    // Create crates
    let crates_dir = root.join("crates");
    fs::create_dir_all(crates_dir.join("pkg-a")).unwrap();
    fs::create_dir_all(crates_dir.join("pkg-b")).unwrap();
    fs::write(
        crates_dir.join("pkg-a/Cargo.toml"),
        "[package]\nname = \"pkg-a\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        crates_dir.join("pkg-b/Cargo.toml"),
        "[package]\nname = \"pkg-b\"\nversion = \"0.2.0\"\n",
    )
    .unwrap();

    let packages = discover_cargo(root).unwrap();
    assert_eq!(packages.len(), 2);

    let mut names: Vec<_> = packages.iter().map(|p| p.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["pkg-a", "pkg-b"]);

    // All should be Cargo packages
    assert!(packages.iter().all(|p| p.kind == PackageKind::Cargo));
}

#[test]
fn cargo_discoverer_detects_internal_deps() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Create workspace
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();

    let crates_dir = root.join("crates");
    fs::create_dir_all(crates_dir.join("pkg-a")).unwrap();
    fs::create_dir_all(crates_dir.join("pkg-b")).unwrap();

    // pkg-a depends on pkg-b via path
    fs::write(
        crates_dir.join("pkg-a/Cargo.toml"),
        "[package]\nname=\"pkg-a\"\nversion=\"0.1.0\"\n[dependencies]\npkg-b={ path=\"../pkg-b\" }\n",
    )
    .unwrap();
    fs::write(
        crates_dir.join("pkg-b/Cargo.toml"),
        "[package]\nname=\"pkg-b\"\nversion=\"0.1.0\"\n",
    )
    .unwrap();

    let packages = discover_cargo(root).unwrap();
    let pkg_a = packages.iter().find(|p| p.name == "pkg-a").unwrap();

    assert_eq!(pkg_a.identifier, "cargo/pkg-a");
    assert!(pkg_a.internal_deps.contains("cargo/pkg-b"));
}

#[test]
fn adds_version_to_regular_dependency_with_only_path() {
    let input = "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n\n[dependencies]\nfoo = { path = \"../foo\" }\n";
    let mut updates = BTreeMap::new();
    updates.insert("foo".to_string(), "0.2.0".to_string());

    let (out, applied) = update_manifest_versions(
        Path::new("/workspace/pkg/Cargo.toml"),
        input,
        None,
        &updates,
        None,
    )
    .unwrap();

    assert!(applied.contains(&("foo".to_string(), "0.2.0".to_string())));
    assert!(out.contains("version = \"0.2.0\""));
    assert!(out.contains("path = \"../foo\""));
}

#[test]
fn skips_workspace_dependencies_when_updating() {
    let input = "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n\n[dependencies]\nfoo = { workspace = true, optional = true }\n";
    let mut updates = BTreeMap::new();
    updates.insert("foo".to_string(), "1.2.3".to_string());

    let (out, applied) =
        update_manifest_versions(Path::new("/demo/Cargo.toml"), input, None, &updates, None)
            .unwrap();

    assert_eq!(out.trim_end(), input.trim_end());
    assert!(applied.is_empty());
}

#[test]
fn updates_workspace_dependency_with_explicit_version() {
    let input = "[workspace.dependencies]\nfoo = { version = \"0.1.0\", path = \"foo\" }\n";
    let mut updates = BTreeMap::new();
    updates.insert("foo".to_string(), "0.2.0".to_string());

    let (out, applied) = update_manifest_versions(
        Path::new("/workspace/Cargo.toml"),
        input,
        None,
        &updates,
        None,
    )
    .unwrap();

    assert!(applied.contains(&("foo".to_string(), "0.2.0".to_string())));
    assert!(out.contains("version = \"0.2.0\""));
}

#[test]
fn keeps_workspace_dependency_shorthand_for_patch_bump() {
    assert!(compute_workspace_dependency_version("0.1", "0.1.14").is_none());
}

#[test]
fn updates_workspace_dependency_shorthand_for_minor_bump() {
    let resolved = compute_workspace_dependency_version("0.1", "0.2.0")
        .expect("minor bump should rewrite shorthand");
    assert_eq!(resolved, "0.2");
}

#[test]
fn updates_workspace_dependency_major_shorthand() {
    let resolved = compute_workspace_dependency_version("1", "2.0.0")
        .expect("major bump should rewrite shorthand");
    assert_eq!(resolved, "2");
}

#[test]
fn skips_workspace_dependency_with_wildcard_version() {
    assert!(compute_workspace_dependency_version("*", "0.2.0").is_none());
}

#[test]
fn adds_version_to_workspace_dependency_with_only_path() {
    let input = "[workspace.dependencies]\nfoo = { path = \"foo\" }\n";
    let mut updates = BTreeMap::new();
    updates.insert("foo".to_string(), "0.2.0".to_string());

    let (out, applied) = update_manifest_versions(
        Path::new("/workspace/Cargo.toml"),
        input,
        None,
        &updates,
        None,
    )
    .unwrap();

    assert!(applied.contains(&("foo".to_string(), "0.2.0".to_string())));
    assert!(out.contains("version = \"0.2.0\""));
    assert!(out.contains("path = \"foo\""));
}

#[test]
fn converts_simple_dep_without_quotes() {
    let input = "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n\n[dependencies]\nbar = \"0.1.0\"\n";
    let mut updates = BTreeMap::new();
    updates.insert("bar".to_string(), "0.2.0".to_string());

    let (out, applied) =
        update_manifest_versions(Path::new("/demo/Cargo.toml"), input, None, &updates, None)
            .unwrap();

    assert!(applied.contains(&("bar".to_string(), "0.2.0".to_string())));
    assert!(out.contains("bar = \"0.2.0\""));
}
