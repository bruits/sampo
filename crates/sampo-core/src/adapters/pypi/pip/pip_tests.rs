use super::*;
use crate::types::PackageKind;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn discover_single_pypi_package() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "example-pkg"
version = "0.1.0"
dependencies = []
"#,
    );

    let packages = discover(root).unwrap();
    assert_eq!(packages.len(), 1);
    let pkg = &packages[0];
    assert_eq!(pkg.name, "example-pkg");
    assert_eq!(pkg.version, "0.1.0");
    assert_eq!(pkg.kind, PackageKind::PyPI);
    assert!(pkg.internal_deps.is_empty());
}

#[test]
fn discover_uv_workspace_packages() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Root workspace manifest
    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "workspace-root"
version = "1.0.0"
dependencies = ["pkg-a"]

[tool.uv.workspace]
members = ["packages/*"]
"#,
    );

    // Package A
    write_file(
        &root.join("packages/pkg-a/pyproject.toml"),
        r#"
[project]
name = "pkg-a"
version = "0.1.0"
dependencies = ["pkg-b>=0.2.0"]
"#,
    );

    // Package B
    write_file(
        &root.join("packages/pkg-b/pyproject.toml"),
        r#"
[project]
name = "pkg-b"
version = "0.2.0"
dependencies = []
"#,
    );

    let packages = discover(root).unwrap();
    assert_eq!(packages.len(), 3);

    let root_pkg = packages
        .iter()
        .find(|p| p.name == "workspace-root")
        .unwrap();
    assert!(root_pkg.internal_deps.contains("pypi/pkg-a"));

    let pkg_a = packages.iter().find(|p| p.name == "pkg-a").unwrap();
    assert!(pkg_a.internal_deps.contains("pypi/pkg-b"));

    let pkg_b = packages.iter().find(|p| p.name == "pkg-b").unwrap();
    assert!(pkg_b.internal_deps.is_empty());
}

#[test]
fn discover_uv_workspace_with_explicit_members() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "root"
version = "1.0.0"

[tool.uv.workspace]
members = ["libs/core", "libs/utils"]
"#,
    );

    write_file(
        &root.join("libs/core/pyproject.toml"),
        r#"
[project]
name = "core-lib"
version = "0.1.0"
"#,
    );

    write_file(
        &root.join("libs/utils/pyproject.toml"),
        r#"
[project]
name = "utils-lib"
version = "0.2.0"
"#,
    );

    let packages = discover(root).unwrap();
    assert_eq!(packages.len(), 3);

    let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"root"));
    assert!(names.contains(&"core-lib"));
    assert!(names.contains(&"utils-lib"));
}

#[test]
fn update_manifest_versions_updates_version() {
    let manifest = r#"
[project]
name = "my-pkg"
version = "0.1.0"
dependencies = []
"#;

    let versions = BTreeMap::new();
    let (updated, _applied) = update_manifest_versions(
        Path::new("pyproject.toml"),
        manifest,
        Some("0.2.0"),
        &versions,
    )
    .unwrap();

    assert!(updated.contains(r#"version = "0.2.0""#));
}

#[test]
fn update_manifest_versions_updates_dependencies() {
    let manifest = r#"
[project]
name = "my-pkg"
version = "0.1.0"
dependencies = ["other-pkg>=1.0.0", "another>=2.0"]
"#;

    let mut versions = BTreeMap::new();
    versions.insert("other-pkg".to_string(), "1.5.0".to_string());

    let (updated, applied) =
        update_manifest_versions(Path::new("pyproject.toml"), manifest, None, &versions).unwrap();

    assert!(updated.contains("other-pkg>=1.5.0"));
    assert!(applied.contains(&("other-pkg".to_string(), "1.5.0".to_string())));
}

#[test]
fn update_manifest_versions_updates_optional_dependencies() {
    let manifest = r#"
[project]
name = "my-pkg"
version = "0.1.0"
dependencies = []

[project.optional-dependencies]
dev = ["pytest>=7.0.0", "my-dep>=1.0.0"]
"#;

    let mut versions = BTreeMap::new();
    versions.insert("my-dep".to_string(), "2.0.0".to_string());

    let (updated, applied) =
        update_manifest_versions(Path::new("pyproject.toml"), manifest, None, &versions).unwrap();

    assert!(updated.contains("my-dep>=2.0.0"));
    assert!(applied.contains(&("my-dep".to_string(), "2.0.0".to_string())));
}

#[test]
fn is_publishable_requires_name_and_version() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("pyproject.toml");

    // Missing name
    write_file(
        &manifest,
        r#"
[project]
version = "0.1.0"
"#,
    );
    let err = is_publishable(&manifest).unwrap_err();
    assert!(format!("{}", err).contains("missing a project.name"));

    // Missing version
    write_file(
        &manifest,
        r#"
[project]
name = "example"
"#,
    );
    let err = is_publishable(&manifest).unwrap_err();
    assert!(format!("{}", err).contains("missing a project.version"));
}

#[test]
fn is_publishable_accepts_valid_package() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("pyproject.toml");

    write_file(
        &manifest,
        r#"
[project]
name = "example"
version = "0.1.0"
"#,
    );
    assert!(is_publishable(&manifest).unwrap());
}

#[test]
fn regenerate_lockfile_requires_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let err = regenerate_lockfile(temp.path()).expect_err("expected missing manifest to fail");
    assert!(format!("{}", err).contains("pyproject.toml"));
}

#[test]
fn extract_package_name_handles_simple_names() {
    assert_eq!(
        extract_package_name("requests"),
        Some("requests".to_string())
    );
    assert_eq!(
        extract_package_name("my-package"),
        Some("my-package".to_string())
    );
    assert_eq!(
        extract_package_name("some_pkg"),
        Some("some_pkg".to_string())
    );
}

#[test]
fn extract_package_name_handles_version_specifiers() {
    assert_eq!(
        extract_package_name("requests>=2.0"),
        Some("requests".to_string())
    );
    assert_eq!(
        extract_package_name("flask==2.0.0"),
        Some("flask".to_string())
    );
    assert_eq!(
        extract_package_name("django~=4.0"),
        Some("django".to_string())
    );
    assert_eq!(extract_package_name("numpy<2.0"), Some("numpy".to_string()));
    assert_eq!(
        extract_package_name("pandas>1.0,<2.0"),
        Some("pandas".to_string())
    );
}

#[test]
fn extract_package_name_handles_extras() {
    assert_eq!(
        extract_package_name("requests[security]>=2.0"),
        Some("requests".to_string())
    );
    assert_eq!(
        extract_package_name("package[extra1,extra2]"),
        Some("package".to_string())
    );
}

#[test]
fn extract_package_name_handles_environment_markers() {
    assert_eq!(
        extract_package_name("pywin32; sys_platform == 'win32'"),
        Some("pywin32".to_string())
    );
    assert_eq!(
        extract_package_name("typing-extensions>=4.0; python_version < '3.10'"),
        Some("typing-extensions".to_string())
    );
}

#[test]
fn extract_package_name_rejects_empty() {
    assert_eq!(extract_package_name(""), None);
    assert_eq!(extract_package_name("   "), None);
}

#[test]
fn try_update_dependency_spec_preserves_operator() {
    let mut versions = BTreeMap::new();
    versions.insert("requests".to_string(), "3.0.0".to_string());

    let result = try_update_dependency_spec("requests>=2.0.0", &versions);
    assert_eq!(
        result,
        Some(("requests".to_string(), "requests>=3.0.0".to_string()))
    );

    let result = try_update_dependency_spec("requests==2.0.0", &versions);
    assert_eq!(
        result,
        Some(("requests".to_string(), "requests==3.0.0".to_string()))
    );

    let result = try_update_dependency_spec("requests~=2.0.0", &versions);
    assert_eq!(
        result,
        Some(("requests".to_string(), "requests~=3.0.0".to_string()))
    );
}

#[test]
fn try_update_dependency_spec_skips_bare_dependencies() {
    let mut versions = BTreeMap::new();
    versions.insert("requests".to_string(), "3.0.0".to_string());

    // Bare dependency without version - skip (nothing to update atomically)
    let result = try_update_dependency_spec("requests", &versions);
    assert_eq!(result, None);
}

#[test]
fn try_update_dependency_spec_ignores_unknown_packages() {
    let versions = BTreeMap::new();

    let result = try_update_dependency_spec("requests>=2.0.0", &versions);
    assert_eq!(result, None);
}

#[test]
fn try_update_dependency_spec_skips_when_already_at_version() {
    let mut versions = BTreeMap::new();
    versions.insert("requests".to_string(), "2.0.0".to_string());

    // Already at target version - no change needed
    let result = try_update_dependency_spec("requests>=2.0.0", &versions);
    assert_eq!(result, None);
}

#[test]
fn try_update_dependency_spec_preserves_extras() {
    let mut versions = BTreeMap::new();
    versions.insert("requests".to_string(), "3.0.0".to_string());

    // Single extra
    let result = try_update_dependency_spec("requests[security]>=2.0.0", &versions);
    assert_eq!(
        result,
        Some((
            "requests".to_string(),
            "requests[security]>=3.0.0".to_string()
        ))
    );

    // Multiple extras
    let result = try_update_dependency_spec("requests[security,socks]>=2.0.0", &versions);
    assert_eq!(
        result,
        Some((
            "requests".to_string(),
            "requests[security,socks]>=3.0.0".to_string()
        ))
    );

    // Extra without version - skip (bare dependency)
    let result = try_update_dependency_spec("requests[security]", &versions);
    assert_eq!(result, None);
}

#[test]
fn try_update_dependency_spec_preserves_environment_markers() {
    let mut versions = BTreeMap::new();
    versions.insert("typing-extensions".to_string(), "5.0.0".to_string());
    versions.insert("pywin32".to_string(), "400".to_string());

    // Python version marker
    let result = try_update_dependency_spec(
        "typing-extensions>=4.0; python_version < \"3.10\"",
        &versions,
    );
    assert_eq!(
        result,
        Some((
            "typing-extensions".to_string(),
            "typing-extensions>=5.0.0 ; python_version < \"3.10\"".to_string()
        ))
    );

    // Platform marker
    let result = try_update_dependency_spec("pywin32>=300; sys_platform == 'win32'", &versions);
    assert_eq!(
        result,
        Some((
            "pywin32".to_string(),
            "pywin32>=400 ; sys_platform == 'win32'".to_string()
        ))
    );

    // Marker without version - skip (bare dependency)
    let result = try_update_dependency_spec("pywin32; sys_platform == 'win32'", &versions);
    assert_eq!(result, None);
}

#[test]
fn try_update_dependency_spec_skips_multiple_constraints() {
    let mut versions = BTreeMap::new();
    versions.insert("pandas".to_string(), "2.0.0".to_string());
    versions.insert("numpy".to_string(), "2.0.0".to_string());

    // Multiple constraints should be skipped - they require manual review
    // as bumping may create invalid ranges like >=2.0.0,<2.0
    let result = try_update_dependency_spec("pandas>=1.0,<2.0", &versions);
    assert_eq!(result, None);

    let result = try_update_dependency_spec("numpy>=1.20,!=1.22.0,<2.0", &versions);
    assert_eq!(result, None);
}

#[test]
fn try_update_dependency_spec_skips_url_references() {
    let mut versions = BTreeMap::new();
    versions.insert("my-package".to_string(), "2.0.0".to_string());

    // URL reference should be skipped (not modified)
    let result = try_update_dependency_spec(
        "my-package @ https://github.com/user/repo/archive/main.zip",
        &versions,
    );
    assert_eq!(result, None);

    // File URL reference
    let result = try_update_dependency_spec("my-package @ file:///local/path", &versions);
    assert_eq!(result, None);
}

#[test]
fn try_update_dependency_spec_preserves_extras_with_markers() {
    let mut versions = BTreeMap::new();
    versions.insert("requests".to_string(), "3.0.0".to_string());

    // Extras + version + markers
    let result = try_update_dependency_spec(
        "requests[security]>=2.0; python_version >= \"3.8\"",
        &versions,
    );
    assert_eq!(
        result,
        Some((
            "requests".to_string(),
            "requests[security]>=3.0.0 ; python_version >= \"3.8\"".to_string()
        ))
    );
}

#[test]
fn try_update_dependency_spec_skips_multiple_constraints_with_markers() {
    let mut versions = BTreeMap::new();
    versions.insert("pandas".to_string(), "2.5.0".to_string());

    // Multiple constraints + markers should be skipped
    let result =
        try_update_dependency_spec("pandas>=1.0,<2.0; python_version >= \"3.9\"", &versions);
    assert_eq!(result, None);
}

#[test]
fn try_update_dependency_spec_handles_all_operators() {
    let mut versions = BTreeMap::new();
    versions.insert("pkg".to_string(), "5.0.0".to_string());

    // >=
    assert_eq!(
        try_update_dependency_spec("pkg>=1.0", &versions),
        Some(("pkg".to_string(), "pkg>=5.0.0".to_string()))
    );

    // <=
    assert_eq!(
        try_update_dependency_spec("pkg<=1.0", &versions),
        Some(("pkg".to_string(), "pkg<=5.0.0".to_string()))
    );

    // ==
    assert_eq!(
        try_update_dependency_spec("pkg==1.0", &versions),
        Some(("pkg".to_string(), "pkg==5.0.0".to_string()))
    );

    // ~=
    assert_eq!(
        try_update_dependency_spec("pkg~=1.0", &versions),
        Some(("pkg".to_string(), "pkg~=5.0.0".to_string()))
    );

    // !=
    assert_eq!(
        try_update_dependency_spec("pkg!=1.0", &versions),
        Some(("pkg".to_string(), "pkg!=5.0.0".to_string()))
    );

    // >
    assert_eq!(
        try_update_dependency_spec("pkg>1.0", &versions),
        Some(("pkg".to_string(), "pkg>5.0.0".to_string()))
    );

    // <
    assert_eq!(
        try_update_dependency_spec("pkg<1.0", &versions),
        Some(("pkg".to_string(), "pkg<5.0.0".to_string()))
    );
}

#[test]
fn normalize_path_collapses_segments() {
    let input = Path::new("/a/b/../c/./d");
    let expected = PathBuf::from("/a/c/d");
    assert_eq!(normalize_path(input), expected);
}

#[test]
fn normalize_path_prevents_escaping_root() {
    let input = Path::new("/a/../..");
    let expected = PathBuf::from("/");
    assert_eq!(normalize_path(input), expected);

    let input = Path::new("a/../..");
    let expected = PathBuf::from("");
    assert_eq!(normalize_path(input), expected);
}

#[test]
fn can_discover_checks_for_pyproject_toml() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // No pyproject.toml
    assert!(!can_discover(root));

    // With pyproject.toml
    write_file(&root.join("pyproject.toml"), "[project]\nname = \"test\"\n");
    assert!(can_discover(root));
}

#[test]
fn manifest_path_returns_pyproject_toml() {
    let path = manifest_path(Path::new("/some/package"));
    assert_eq!(path, PathBuf::from("/some/package/pyproject.toml"));
}

#[test]
fn parse_project_metadata_extracts_pep621_fields() {
    let source = r#"
[project]
name = "my-package"
version = "1.2.3"
"#;
    let meta = parse_project_metadata(source);
    assert_eq!(meta.name, Some("my-package".to_string()));
    assert_eq!(meta.version, Some("1.2.3".to_string()));
}

#[test]
fn parse_project_metadata_returns_none_for_missing_fields() {
    let source = r#"
[tool.other]
key = "value"
"#;
    let meta = parse_project_metadata(source);
    assert_eq!(meta.name, None);
    assert_eq!(meta.version, None);
}

#[test]
fn parse_uv_workspace_members_extracts_members() {
    let source = r#"
[project]
name = "root"
version = "1.0.0"

[tool.uv.workspace]
members = ["packages/*", "libs/core"]
"#;
    let members = parse_uv_workspace_members(source).unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.contains(&"packages/*".to_string()));
    assert!(members.contains(&"libs/core".to_string()));
}

#[test]
fn parse_uv_workspace_members_returns_none_without_workspace() {
    let source = r#"
[project]
name = "single-pkg"
version = "1.0.0"
"#;
    assert!(parse_uv_workspace_members(source).is_none());
}

#[test]
fn collect_dependencies_finds_pep621_deps() {
    let source = r#"
[project]
name = "my-pkg"
version = "1.0.0"
dependencies = ["requests>=2.0", "flask", "django~=4.0"]

[project.optional-dependencies]
dev = ["pytest", "black"]
"#;
    let deps = collect_dependencies(source);
    assert!(deps.contains(&"requests".to_string()));
    assert!(deps.contains(&"flask".to_string()));
    assert!(deps.contains(&"django".to_string()));
    assert!(deps.contains(&"pytest".to_string()));
    assert!(deps.contains(&"black".to_string()));
}

#[test]
fn collect_dependencies_returns_empty_without_project() {
    let source = r#"
[tool.other]
key = "value"
"#;
    let deps = collect_dependencies(source);
    assert!(deps.is_empty());
}

#[test]
fn normalize_package_name_lowercases() {
    assert_eq!(normalize_package_name("Requests"), "requests");
    assert_eq!(normalize_package_name("DJANGO"), "django");
    assert_eq!(normalize_package_name("Flask"), "flask");
    assert_eq!(normalize_package_name("MyPackage"), "mypackage");
}

#[test]
fn normalize_package_name_replaces_separators_with_dash() {
    // Underscores become dashes
    assert_eq!(normalize_package_name("my_package"), "my-package");
    // Dots become dashes
    assert_eq!(normalize_package_name("my.package"), "my-package");
    // Dashes stay as dashes
    assert_eq!(normalize_package_name("my-package"), "my-package");
}

#[test]
fn normalize_package_name_collapses_separator_runs() {
    // Multiple underscores
    assert_eq!(normalize_package_name("my__package"), "my-package");
    // Multiple dashes
    assert_eq!(normalize_package_name("my--package"), "my-package");
    // Multiple dots
    assert_eq!(normalize_package_name("my..package"), "my-package");
    // Mixed separators
    assert_eq!(normalize_package_name("my_-._package"), "my-package");
    assert_eq!(normalize_package_name("my.-_package"), "my-package");
}

#[test]
fn normalize_package_name_handles_leading_trailing_separators() {
    // Leading separators are dropped
    assert_eq!(normalize_package_name("_package"), "package");
    assert_eq!(normalize_package_name("-package"), "package");
    assert_eq!(normalize_package_name(".package"), "package");
    assert_eq!(normalize_package_name("__package"), "package");

    // Trailing separators are dropped
    assert_eq!(normalize_package_name("package_"), "package");
    assert_eq!(normalize_package_name("package-"), "package");
    assert_eq!(normalize_package_name("package."), "package");
    assert_eq!(normalize_package_name("package__"), "package");
}

#[test]
fn normalize_package_name_combined_cases() {
    // PEP 503 example: all these should normalize to the same value
    assert_eq!(normalize_package_name("My_Package"), "my-package");
    assert_eq!(normalize_package_name("my-package"), "my-package");
    assert_eq!(normalize_package_name("my.package"), "my-package");
    assert_eq!(normalize_package_name("MY--PACKAGE"), "my-package");
    assert_eq!(normalize_package_name("My..Package"), "my-package");
    assert_eq!(normalize_package_name("my___package"), "my-package");

    // More complex cases
    assert_eq!(
        normalize_package_name("Typing_Extensions"),
        "typing-extensions"
    );
    assert_eq!(
        normalize_package_name("typing-extensions"),
        "typing-extensions"
    );
    assert_eq!(
        normalize_package_name("typing.extensions"),
        "typing-extensions"
    );
}

#[test]
fn normalize_package_name_handles_empty_and_simple() {
    assert_eq!(normalize_package_name(""), "");
    assert_eq!(normalize_package_name("a"), "a");
    assert_eq!(normalize_package_name("pkg"), "pkg");
}

#[test]
fn discover_matches_internal_deps_with_different_casing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Root workspace manifest
    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "workspace-root"
version = "1.0.0"
dependencies = []

[tool.uv.workspace]
members = ["packages/*"]
"#,
    );

    // Package A with uppercase name
    write_file(
        &root.join("packages/pkg-a/pyproject.toml"),
        r#"
[project]
name = "My_Core_Package"
version = "0.1.0"
dependencies = []
"#,
    );

    // Package B depends on A with lowercase/different separators
    write_file(
        &root.join("packages/pkg-b/pyproject.toml"),
        r#"
[project]
name = "my-app"
version = "0.2.0"
dependencies = ["my-core-package>=0.1.0"]
"#,
    );

    let packages = discover(root).unwrap();
    assert_eq!(packages.len(), 3);

    let pkg_b = packages.iter().find(|p| p.name == "my-app").unwrap();
    // Should detect internal dependency despite different naming conventions
    assert!(
        pkg_b.internal_deps.contains("pypi/My_Core_Package"),
        "Expected my-app to have internal dep on My_Core_Package, got: {:?}",
        pkg_b.internal_deps
    );
}

#[test]
fn discover_matches_internal_deps_with_different_separators() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "workspace-root"
version = "1.0.0"
dependencies = []

[tool.uv.workspace]
members = ["packages/*"]
"#,
    );

    // Package with underscores
    write_file(
        &root.join("packages/core/pyproject.toml"),
        r#"
[project]
name = "my_core"
version = "1.0.0"
dependencies = []
"#,
    );

    // Package with dots in dependency
    write_file(
        &root.join("packages/app/pyproject.toml"),
        r#"
[project]
name = "my-app"
version = "1.0.0"
dependencies = ["my.core>=1.0.0"]
"#,
    );

    let packages = discover(root).unwrap();

    let app = packages.iter().find(|p| p.name == "my-app").unwrap();
    assert!(
        app.internal_deps.contains("pypi/my_core"),
        "Expected my-app to have internal dep on my_core (matching my.core), got: {:?}",
        app.internal_deps
    );
}

#[test]
fn try_update_dependency_spec_matches_with_normalized_names() {
    let mut versions = BTreeMap::new();
    // Map has uppercase/underscore name
    versions.insert("My_Package".to_string(), "3.0.0".to_string());

    // Dependency uses lowercase/dash - should still match
    let result = try_update_dependency_spec("my-package>=2.0.0", &versions);
    assert_eq!(
        result,
        Some(("My_Package".to_string(), "my-package>=3.0.0".to_string()))
    );
}

#[test]
fn try_update_dependency_spec_matches_underscore_to_dash() {
    let mut versions = BTreeMap::new();
    versions.insert("typing-extensions".to_string(), "5.0.0".to_string());

    // Dependency uses underscore
    let result = try_update_dependency_spec("typing_extensions>=4.0.0", &versions);
    assert_eq!(
        result,
        Some((
            "typing-extensions".to_string(),
            "typing_extensions>=5.0.0".to_string()
        ))
    );
}

#[test]
fn try_update_dependency_spec_matches_dot_to_dash() {
    let mut versions = BTreeMap::new();
    versions.insert("zope-interface".to_string(), "6.0.0".to_string());

    // Dependency uses dots
    let result = try_update_dependency_spec("zope.interface>=5.0.0", &versions);
    assert_eq!(
        result,
        Some((
            "zope-interface".to_string(),
            "zope.interface>=6.0.0".to_string()
        ))
    );
}

#[test]
fn try_update_dependency_spec_case_insensitive_match() {
    let mut versions = BTreeMap::new();
    versions.insert("Flask".to_string(), "3.0.0".to_string());

    // Dependency uses lowercase
    let result = try_update_dependency_spec("flask>=2.0.0", &versions);
    assert_eq!(
        result,
        Some(("Flask".to_string(), "flask>=3.0.0".to_string()))
    );

    // Dependency uses uppercase
    let result = try_update_dependency_spec("FLASK>=2.0.0", &versions);
    assert_eq!(
        result,
        Some(("Flask".to_string(), "FLASK>=3.0.0".to_string()))
    );
}

#[test]
fn try_update_dependency_spec_returns_original_name_from_map() {
    let mut versions = BTreeMap::new();
    // The map has the "canonical" name as stored in package info
    versions.insert("My_Special_Package".to_string(), "2.0.0".to_string());

    let result = try_update_dependency_spec("my-special-package>=1.0.0", &versions);

    // Should return the original name from the map, not the dependency spec name
    assert!(result.is_some());
    let (returned_name, _) = result.unwrap();
    assert_eq!(returned_name, "My_Special_Package");
}

#[test]
fn discover_rejects_collision_dash_underscore() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "workspace-root"
version = "1.0.0"
dependencies = []

[tool.uv.workspace]
members = ["packages/*"]
"#,
    );

    // Two packages that normalize to the same name
    write_file(
        &root.join("packages/my-package/pyproject.toml"),
        r#"
[project]
name = "my-package"
version = "1.0.0"
"#,
    );

    write_file(
        &root.join("packages/my_package/pyproject.toml"),
        r#"
[project]
name = "my_package"
version = "1.0.0"
"#,
    );

    let err = discover(root).expect_err("expected collision to fail");
    let msg = format!("{}", err);
    assert!(
        msg.contains("normalize to the same PEP 503 name"),
        "error should mention PEP 503 collision: {}",
        msg
    );
    assert!(
        msg.contains("my-package"),
        "error should mention first package name"
    );
}

#[test]
fn discover_rejects_collision_case_insensitive() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "workspace-root"
version = "1.0.0"
dependencies = []

[tool.uv.workspace]
members = ["packages/*"]
"#,
    );

    write_file(
        &root.join("packages/mypackage/pyproject.toml"),
        r#"
[project]
name = "MyPackage"
version = "1.0.0"
"#,
    );

    write_file(
        &root.join("packages/mypackage2/pyproject.toml"),
        r#"
[project]
name = "mypackage"
version = "1.0.0"
"#,
    );

    let err = discover(root).expect_err("expected collision to fail");
    let msg = format!("{}", err);
    assert!(
        msg.contains("normalize to the same PEP 503 name"),
        "error should mention PEP 503 collision: {}",
        msg
    );
}

#[test]
fn discover_rejects_collision_dot_separator() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "workspace-root"
version = "1.0.0"
dependencies = []

[tool.uv.workspace]
members = ["packages/*"]
"#,
    );

    write_file(
        &root.join("packages/pkg1/pyproject.toml"),
        r#"
[project]
name = "my.package"
version = "1.0.0"
"#,
    );

    write_file(
        &root.join("packages/pkg2/pyproject.toml"),
        r#"
[project]
name = "my-package"
version = "1.0.0"
"#,
    );

    let err = discover(root).expect_err("expected collision to fail");
    let msg = format!("{}", err);
    assert!(
        msg.contains("normalize to the same PEP 503 name"),
        "error should mention PEP 503 collision: {}",
        msg
    );
}

#[test]
fn discover_allows_distinct_normalized_names() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    write_file(
        &root.join("pyproject.toml"),
        r#"
[project]
name = "workspace-root"
version = "1.0.0"
dependencies = []

[tool.uv.workspace]
members = ["packages/*"]
"#,
    );

    // These normalize to different names: "my-package" vs "mypackage"
    write_file(
        &root.join("packages/pkg1/pyproject.toml"),
        r#"
[project]
name = "my-package"
version = "1.0.0"
"#,
    );

    write_file(
        &root.join("packages/pkg2/pyproject.toml"),
        r#"
[project]
name = "mypackage"
version = "1.0.0"
"#,
    );

    let packages = discover(root).expect("distinct names should succeed");
    assert_eq!(packages.len(), 3);
}
