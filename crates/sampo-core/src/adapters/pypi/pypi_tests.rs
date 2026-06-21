use super::*;

#[test]
fn version_exists_rejects_empty_name() {
    let err = PyPIAdapter
        .version_exists("", "1.0.0", None)
        .expect_err("expected empty package name to fail");
    assert!(format!("{}", err).contains("Package name cannot be empty"));
}

#[test]
fn version_exists_defers_to_uv_for_private_index() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("pyproject.toml");
    std::fs::write(
        &manifest,
        r#"
[project]
name = "private-pkg"
version = "1.0.0"

[[tool.uv.index]]
name = "private"
publish-url = "https://private.example.com/upload/"
"#,
    )
    .unwrap();

    let exists = PyPIAdapter
        .version_exists("private-pkg", "1.0.0", Some(manifest.as_path()))
        .expect("private index should defer without erroring");
    assert!(!exists);
}

#[test]
fn normalize_package_name_converts_underscores_and_dots() {
    assert_eq!(normalize_package_name("My_Package"), "my-package");
    assert_eq!(normalize_package_name("some.package"), "some-package");
    assert_eq!(normalize_package_name("Normal-Name"), "normal-name");
    assert_eq!(
        normalize_package_name("Package_With.Mixed_CASE"),
        "package-with-mixed-case"
    );
}
