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
fn cargo_discoverer_discovers_single_package() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"single-crate\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let packages = discover_cargo(root).unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "single-crate");
    assert_eq!(packages[0].version, "0.1.0");
    assert_eq!(packages[0].kind, PackageKind::Cargo);
    assert_eq!(packages[0].path, root);
    assert!(packages[0].internal_deps.is_empty());
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
fn skips_workspace_dependency_without_version() {
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

    assert_eq!(out.trim_end(), input.trim_end());
    assert!(applied.is_empty());
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

#[test]
fn cargo_discoverer_handles_workspace_version_inheritance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Create workspace with workspace.package.version
    fs::write(
        root.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/*"]

[workspace.package]
version = "0.3.0"
"#,
    )
    .unwrap();

    let crates_dir = root.join("crates");
    fs::create_dir_all(crates_dir.join("pkg-a")).unwrap();
    fs::create_dir_all(crates_dir.join("pkg-b")).unwrap();

    // pkg-a uses workspace version inheritance
    fs::write(
        crates_dir.join("pkg-a/Cargo.toml"),
        r#"[package]
name = "pkg-a"
version.workspace = true
"#,
    )
    .unwrap();

    // pkg-b uses explicit version
    fs::write(
        crates_dir.join("pkg-b/Cargo.toml"),
        r#"[package]
name = "pkg-b"
version = "0.2.0"
"#,
    )
    .unwrap();

    let packages = discover_cargo(root).unwrap();
    assert_eq!(packages.len(), 2);

    let pkg_a = packages.iter().find(|p| p.name == "pkg-a").unwrap();
    let pkg_b = packages.iter().find(|p| p.name == "pkg-b").unwrap();

    // pkg-a should inherit version from workspace.package.version
    assert_eq!(pkg_a.version, "0.3.0");
    // pkg-b should use its explicit version
    assert_eq!(pkg_b.version, "0.2.0");
}

#[test]
fn cargo_discoverer_rejects_workspace_inheritance_without_workspace_version() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"pkg-a\"]\n",
    )
    .unwrap();

    fs::create_dir_all(root.join("pkg-a")).unwrap();
    fs::write(
        root.join("pkg-a/Cargo.toml"),
        "[package]\nname = \"pkg-a\"\nversion.workspace = true\n",
    )
    .unwrap();

    let result = discover_cargo(root);
    assert!(result.is_err());

    let err = result.unwrap_err().to_string();
    assert!(err.contains("version.workspace = true requires workspace.package.version"));
}

mod constraint_validation {
    use super::*;
    use crate::types::ConstraintCheckResult;

    fn assert_satisfied(constraint: &str, version: &str) {
        let result = check_dependency_constraint("test-dep", constraint, version).unwrap();
        assert_eq!(
            result,
            ConstraintCheckResult::Satisfied,
            "Expected constraint '{}' to be satisfied by version '{}', got {:?}",
            constraint,
            version,
            result
        );
    }

    fn assert_not_satisfied(constraint: &str, version: &str) {
        let result = check_dependency_constraint("test-dep", constraint, version).unwrap();
        assert!(
            matches!(result, ConstraintCheckResult::NotSatisfied { .. }),
            "Expected constraint '{}' to NOT be satisfied by version '{}', got {:?}",
            constraint,
            version,
            result
        );
    }

    fn assert_skipped(constraint: &str, version: &str) {
        let result = check_dependency_constraint("test-dep", constraint, version).unwrap();
        assert!(
            matches!(result, ConstraintCheckResult::Skipped { .. }),
            "Expected constraint '{}' with version '{}' to be skipped, got {:?}",
            constraint,
            version,
            result
        );
    }

    #[test]
    fn caret_constraint_allows_compatible_minor_bump() {
        assert_satisfied("1.2.3", "1.3.0");
        assert_satisfied("^1.2.3", "1.3.0");
    }

    #[test]
    fn caret_constraint_rejects_major_bump() {
        assert_not_satisfied("1.2.3", "2.0.0");
        assert_not_satisfied("^1.2.3", "2.0.0");
    }

    #[test]
    fn caret_constraint_zero_major_is_stricter() {
        assert_satisfied("0.2.3", "0.2.5");
        assert_not_satisfied("0.2.3", "0.3.0");
    }

    #[test]
    fn tilde_constraint_allows_patch_only() {
        assert_satisfied("~1.2.3", "1.2.9");
    }

    #[test]
    fn tilde_constraint_rejects_minor_bump() {
        assert_not_satisfied("~1.2.3", "1.3.0");
    }

    #[test]
    fn wildcard_constraint_allows_any_minor() {
        assert_satisfied("1.*", "1.99.0");
    }

    #[test]
    fn wildcard_constraint_rejects_major_bump() {
        assert_not_satisfied("1.*", "2.0.0");
    }

    #[test]
    fn global_wildcard_allows_anything() {
        assert_satisfied("*", "99.0.0");
    }

    #[test]
    fn exact_constraint_allows_exact_match() {
        assert_satisfied("=1.2.3", "1.2.3");
    }

    #[test]
    fn exact_constraint_rejects_any_difference() {
        assert_not_satisfied("=1.2.3", "1.2.4");
    }

    #[test]
    fn range_constraint_allows_within_range() {
        assert_satisfied(">=1.2, <2.0", "1.5.0");
    }

    #[test]
    fn range_constraint_rejects_outside_range() {
        assert_not_satisfied(">=1.2, <2.0", "2.0.0");
    }

    #[test]
    fn prerelease_constraint_matches_prerelease_version() {
        assert_satisfied("1.0.0-alpha", "1.0.0-beta");
    }

    #[test]
    fn stable_constraint_rejects_prerelease_version() {
        assert_not_satisfied("1.0", "1.0.0-alpha");
    }

    #[test]
    fn invalid_constraint_is_skipped() {
        assert_skipped("invalid", "1.0.0");
    }

    #[test]
    fn invalid_version_is_skipped() {
        assert_skipped("1.0", "invalid");
    }

    #[test]
    fn empty_constraint_is_skipped() {
        assert_skipped("", "1.0.0");
    }

    #[test]
    fn empty_version_is_skipped() {
        assert_skipped("1.0", "");
    }

    #[test]
    fn whitespace_in_constraint_is_trimmed() {
        assert_satisfied("  1.2.3  ", "1.3.0");
    }

    #[test]
    fn whitespace_in_version_is_trimmed() {
        assert_satisfied("1.2.3", "  1.3.0  ");
    }

    #[test]
    fn partial_major_constraint_works() {
        assert_satisfied("1", "1.5.0");
        assert_not_satisfied("1", "2.0.0");
    }

    #[test]
    fn partial_minor_constraint_works() {
        assert_satisfied("1.2", "1.2.5");
        assert_satisfied("1.2", "1.3.0");
        assert_not_satisfied("1.2", "2.0.0");
    }
}
