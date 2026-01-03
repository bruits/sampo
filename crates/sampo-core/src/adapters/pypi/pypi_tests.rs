use super::*;

#[test]
fn version_exists_rejects_empty_name() {
    let err = PyPIAdapter
        .version_exists("", "1.0.0", None)
        .expect_err("expected empty package name to fail");
    assert!(format!("{}", err).contains("Package name cannot be empty"));
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
