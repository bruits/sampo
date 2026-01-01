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

    let root_pkg = packages.iter().find(|p| p.name == "workspace-root").unwrap();
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
fn try_update_dependency_spec_adds_version_when_missing() {
    let mut versions = BTreeMap::new();
    versions.insert("requests".to_string(), "3.0.0".to_string());

    let result = try_update_dependency_spec("requests", &versions);
    assert_eq!(
        result,
        Some(("requests".to_string(), "requests==3.0.0".to_string()))
    );
}

#[test]
fn try_update_dependency_spec_ignores_unknown_packages() {
    let versions = BTreeMap::new();

    let result = try_update_dependency_spec("requests>=2.0.0", &versions);
    assert_eq!(result, None);
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
