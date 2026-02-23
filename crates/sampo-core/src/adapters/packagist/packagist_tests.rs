use super::*;
use std::collections::BTreeMap;
use std::path::Path;

#[test]
fn compute_dependency_constraint_caret() {
    assert_eq!(
        compute_dependency_constraint("^1.0.0", "2.0.0"),
        Some("^2.0.0".to_string())
    );
    assert_eq!(compute_dependency_constraint("^2.0.0", "2.0.0"), None);
}

#[test]
fn compute_dependency_constraint_tilde() {
    assert_eq!(
        compute_dependency_constraint("~1.0.0", "2.0.0"),
        Some("~2.0.0".to_string())
    );
    assert_eq!(compute_dependency_constraint("~2.0.0", "2.0.0"), None);
}

#[test]
fn compute_dependency_constraint_exact() {
    assert_eq!(
        compute_dependency_constraint("1.0.0", "2.0.0"),
        Some("^2.0.0".to_string())
    );
    assert_eq!(compute_dependency_constraint("2.0.0", "2.0.0"), None);
}

#[test]
fn compute_dependency_constraint_skips_complex() {
    // Complex constraints with logical operators should not be modified
    assert_eq!(compute_dependency_constraint(">=1.0 <2.0", "2.0.0"), None);
    assert_eq!(compute_dependency_constraint("^1.0 || ^2.0", "3.0.0"), None);
}

#[test]
fn compute_dependency_constraint_skips_comparison_operators() {
    assert_eq!(compute_dependency_constraint(">=1.0.0", "2.0.0"), None);
    assert_eq!(compute_dependency_constraint("<=2.0.0", "1.0.0"), None);
    assert_eq!(compute_dependency_constraint(">1.0.0", "2.0.0"), None);
    assert_eq!(compute_dependency_constraint("<2.0.0", "1.0.0"), None);
}

#[test]
fn compute_dependency_constraint_skips_wildcard() {
    assert_eq!(compute_dependency_constraint("1.0.*", "1.1.0"), None);
    assert_eq!(compute_dependency_constraint("2.*", "2.1.0"), None);
}

#[test]
fn compute_dependency_constraint_empty_uses_caret() {
    assert_eq!(
        compute_dependency_constraint("", "1.0.0"),
        Some("^1.0.0".to_string())
    );
}

#[test]
fn update_manifest_versions_updates_version() {
    let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require": {}
}"#;

    let new_version_by_name = BTreeMap::new();
    let (output, applied) = update_manifest_versions(
        Path::new("composer.json"),
        input,
        Some("2.0.0"),
        &new_version_by_name,
    )
    .unwrap();

    assert!(output.contains(r#""version": "2.0.0""#));
    assert!(applied.is_empty());
}

#[test]
fn update_manifest_versions_updates_dependencies() {
    let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require": {
        "other/dep": "^1.0.0"
    }
}"#;

    let mut new_version_by_name = BTreeMap::new();
    new_version_by_name.insert("other/dep".to_string(), "2.0.0".to_string());

    let (output, applied) = update_manifest_versions(
        Path::new("composer.json"),
        input,
        None,
        &new_version_by_name,
    )
    .unwrap();

    assert!(output.contains(r#""other/dep": "^2.0.0""#));
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].0, "other/dep");
    assert_eq!(applied[0].1, "2.0.0");
}

#[test]
fn update_manifest_versions_updates_dev_dependencies() {
    let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require-dev": {
        "dev/package": "^1.0.0"
    }
}"#;

    let mut new_version_by_name = BTreeMap::new();
    new_version_by_name.insert("dev/package".to_string(), "3.0.0".to_string());

    let (output, applied) = update_manifest_versions(
        Path::new("composer.json"),
        input,
        None,
        &new_version_by_name,
    )
    .unwrap();

    assert!(output.contains(r#""dev/package": "^3.0.0""#));
    assert_eq!(applied.len(), 1);
}

#[test]
fn update_manifest_versions_preserves_tilde_constraint() {
    let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require": {
        "other/dep": "~1.0.0"
    }
}"#;

    let mut new_version_by_name = BTreeMap::new();
    new_version_by_name.insert("other/dep".to_string(), "2.0.0".to_string());

    let (output, _) = update_manifest_versions(
        Path::new("composer.json"),
        input,
        None,
        &new_version_by_name,
    )
    .unwrap();

    assert!(output.contains(r#""other/dep": "~2.0.0""#));
}

#[test]
fn update_manifest_versions_no_changes_when_same_version() {
    let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require": {
        "other/dep": "^1.0.0"
    }
}"#;

    let mut new_version_by_name = BTreeMap::new();
    new_version_by_name.insert("other/dep".to_string(), "1.0.0".to_string());

    let (output, applied) = update_manifest_versions(
        Path::new("composer.json"),
        input,
        Some("1.0.0"),
        &new_version_by_name,
    )
    .unwrap();

    // No changes when versions are the same
    assert_eq!(output, input);
    assert!(applied.is_empty());
}

#[test]
fn discover_packagist_valid_package() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "name": "vendor/my-package",
    "version": "1.2.3",
    "require": {}
}"#;
    std::fs::write(temp.path().join("composer.json"), manifest).unwrap();

    let packages = discover_packagist(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);

    let pkg = &packages[0];
    assert_eq!(pkg.name, "vendor/my-package");
    assert_eq!(pkg.version, "1.2.3");
    assert_eq!(pkg.kind, PackageKind::Packagist);
    assert_eq!(pkg.identifier, "packagist/vendor/my-package");
}

#[test]
fn discover_packagist_requires_vendor_format() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "name": "my-package",
    "version": "1.0.0"
}"#;
    std::fs::write(temp.path().join("composer.json"), manifest).unwrap();

    let result = discover_packagist(temp.path());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("vendor/package"));
}

#[test]
fn discover_packagist_missing_name() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "version": "1.0.0"
}"#;
    std::fs::write(temp.path().join("composer.json"), manifest).unwrap();

    let result = discover_packagist(temp.path());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("missing name"));
}

#[test]
fn is_publishable_valid_package() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "name": "vendor/package",
    "version": "1.0.0"
}"#;
    let path = temp.path().join("composer.json");
    std::fs::write(&path, manifest).unwrap();

    let result = PackagistAdapter.is_publishable(&path).unwrap();
    assert!(result);
}

#[test]
fn is_publishable_abandoned_package() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "abandoned": true
}"#;
    let path = temp.path().join("composer.json");
    std::fs::write(&path, manifest).unwrap();

    let result = PackagistAdapter.is_publishable(&path).unwrap();
    assert!(!result);
}

#[test]
fn is_publishable_abandoned_with_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "abandoned": "vendor/new-package"
}"#;
    let path = temp.path().join("composer.json");
    std::fs::write(&path, manifest).unwrap();

    let result = PackagistAdapter.is_publishable(&path).unwrap();
    assert!(!result);
}

#[test]
fn is_publishable_missing_vendor_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "name": "package-without-vendor",
    "version": "1.0.0"
}"#;
    let path = temp.path().join("composer.json");
    std::fs::write(&path, manifest).unwrap();

    let result = PackagistAdapter.is_publishable(&path);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("vendor/package"));
}

#[test]
fn is_publishable_missing_version() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "name": "vendor/package"
}"#;
    let path = temp.path().join("composer.json");
    std::fs::write(&path, manifest).unwrap();

    let result = PackagistAdapter.is_publishable(&path).unwrap();
    assert!(!result);
}

#[test]
fn is_publishable_empty_version() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = r#"{
    "name": "vendor/package",
    "version": ""
}"#;
    let path = temp.path().join("composer.json");
    std::fs::write(&path, manifest).unwrap();

    let result = PackagistAdapter.is_publishable(&path).unwrap();
    assert!(!result);
}

mod constraint_validation {
    use super::*;
    use crate::types::ConstraintCheckResult;

    fn assert_constraint(constraint: &str, new_version: &str) -> ConstraintCheckResult {
        let temp = tempfile::tempdir().unwrap();
        let manifest_path = temp.path().join("composer.json");
        let content = format!(
            r#"{{"name":"vendor/test","version":"1.0.0","require":{{"test/dep":"{}"}}}}"#,
            constraint
        );
        fs::write(&manifest_path, &content).unwrap();
        check_dependency_constraint(&manifest_path, "test/dep", "*", new_version).unwrap()
    }

    fn assert_satisfied(constraint: &str, new_version: &str) {
        assert_eq!(
            assert_constraint(constraint, new_version),
            ConstraintCheckResult::Satisfied,
            "expected '{}' to be satisfied by '{}'",
            constraint,
            new_version
        );
    }

    fn assert_not_satisfied(constraint: &str, new_version: &str) {
        let result = assert_constraint(constraint, new_version);
        assert!(
            matches!(result, ConstraintCheckResult::NotSatisfied { .. }),
            "expected '{}' to be not satisfied by '{}', got {:?}",
            constraint,
            new_version,
            result
        );
    }

    fn assert_skipped(constraint: &str, new_version: &str) {
        let result = assert_constraint(constraint, new_version);
        assert!(
            matches!(result, ConstraintCheckResult::Skipped { .. }),
            "expected '{}' to be skipped for '{}', got {:?}",
            constraint,
            new_version,
            result
        );
    }

    #[test]
    fn caret_satisfied() {
        assert_satisfied("^1.2.3", "1.5.0");
    }

    #[test]
    fn caret_exact_match() {
        assert_satisfied("^1.2.3", "1.2.3");
    }

    #[test]
    fn caret_zero_minor_satisfied() {
        assert_satisfied("^0.2.3", "0.2.5");
    }

    #[test]
    fn caret_not_satisfied_major_bump() {
        assert_not_satisfied("^1.2.3", "2.0.0");
    }

    #[test]
    fn caret_zero_minor_not_satisfied() {
        assert_not_satisfied("^0.2.3", "0.3.0");
    }

    #[test]
    fn caret_zero_zero_patch_not_satisfied() {
        assert_not_satisfied("^0.0.3", "0.0.4");
    }

    #[test]
    fn tilde_two_parts_satisfied() {
        assert_satisfied("~1.2", "1.5.0");
    }

    #[test]
    fn tilde_two_parts_not_satisfied() {
        assert_not_satisfied("~1.2", "2.0.0");
    }

    #[test]
    fn tilde_three_parts_satisfied() {
        assert_satisfied("~1.2.3", "1.2.9");
    }

    #[test]
    fn tilde_three_parts_not_satisfied() {
        assert_not_satisfied("~1.2.3", "1.3.0");
    }

    #[test]
    fn gte_satisfied() {
        assert_satisfied(">=1.0.0", "2.0.0");
    }

    #[test]
    fn gte_not_satisfied() {
        assert_not_satisfied(">=2.0.0", "1.9.9");
    }

    #[test]
    fn gt_satisfied() {
        assert_satisfied(">1.0.0", "1.0.1");
    }

    #[test]
    fn gt_not_satisfied_equal() {
        assert_not_satisfied(">1.0.0", "1.0.0");
    }

    #[test]
    fn lte_satisfied() {
        assert_satisfied("<=2.0.0", "2.0.0");
    }

    #[test]
    fn lte_not_satisfied() {
        assert_not_satisfied("<=2.0.0", "2.0.1");
    }

    #[test]
    fn lt_satisfied() {
        assert_satisfied("<2.0.0", "1.9.9");
    }

    #[test]
    fn lt_not_satisfied() {
        assert_not_satisfied("<2.0.0", "2.0.0");
    }

    #[test]
    fn ne_satisfied() {
        assert_satisfied("!=1.0.0", "2.0.0");
    }

    #[test]
    fn ne_not_satisfied() {
        assert_not_satisfied("!=1.0.0", "1.0.0");
    }

    #[test]
    fn and_comma_satisfied() {
        assert_satisfied(">=1.0.0,<2.0.0", "1.5.0");
    }

    #[test]
    fn and_comma_not_satisfied() {
        assert_not_satisfied(">=1.0.0,<2.0.0", "2.0.0");
    }

    #[test]
    fn and_space_satisfied() {
        assert_satisfied(">=1.0.0 <2.0.0", "1.5.0");
    }

    #[test]
    fn and_space_not_satisfied() {
        assert_not_satisfied(">=1.0.0 <2.0.0", "2.0.0");
    }

    #[test]
    fn or_satisfied() {
        assert_satisfied("^1.0.0 || ^2.0.0", "2.1.0");
    }

    #[test]
    fn or_not_satisfied() {
        assert_not_satisfied("^1.0.0 || ^2.0.0", "3.0.0");
    }

    #[test]
    fn wildcard_star_satisfied() {
        assert_satisfied("*", "5.0.0");
    }

    #[test]
    fn wildcard_patch_satisfied() {
        assert_satisfied("1.0.*", "1.0.5");
    }

    #[test]
    fn wildcard_patch_not_satisfied() {
        assert_not_satisfied("1.0.*", "1.1.0");
    }

    #[test]
    fn wildcard_minor_satisfied() {
        assert_satisfied("1.*", "1.5.0");
    }

    #[test]
    fn wildcard_minor_not_satisfied() {
        assert_not_satisfied("1.*", "2.0.0");
    }

    #[test]
    fn whitespace_gte() {
        assert_satisfied(">= 1.0.0", "1.5.0");
    }

    #[test]
    fn whitespace_caret() {
        assert_satisfied("^ 1.2.3", "1.5.0");
    }

    #[test]
    fn whitespace_tilde() {
        assert_satisfied("~ 1.2.3", "1.2.9");
    }

    #[test]
    fn skip_pinned_version() {
        assert_skipped("1.2.3", "2.0.0");
    }

    #[test]
    fn skip_prerelease_version() {
        assert_skipped("^1.0.0", "2.0.0-beta.1");
    }

    #[test]
    fn skip_prerelease_constraint() {
        assert_skipped("^1.0.0-beta", "2.0.0");
    }

    #[test]
    fn skip_stability_flag() {
        assert_skipped("^1.0@dev", "2.0.0");
    }

    #[test]
    fn skip_dep_not_found() {
        let temp = tempfile::tempdir().unwrap();
        let manifest_path = temp.path().join("composer.json");
        let content = r#"{"name":"vendor/test","version":"1.0.0","require":{}}"#;
        fs::write(&manifest_path, content).unwrap();
        let result =
            check_dependency_constraint(&manifest_path, "missing/dep", "*", "1.0.0").unwrap();
        assert!(matches!(result, ConstraintCheckResult::Skipped { .. }));
    }

    #[test]
    fn dev_deps_found() {
        let temp = tempfile::tempdir().unwrap();
        let manifest_path = temp.path().join("composer.json");
        let content =
            r#"{"name":"vendor/test","version":"1.0.0","require-dev":{"test/dep":"^1.0.0"}}"#;
        fs::write(&manifest_path, content).unwrap();
        let result = check_dependency_constraint(&manifest_path, "test/dep", "*", "1.5.0").unwrap();
        assert_eq!(result, ConstraintCheckResult::Satisfied);
    }

    #[test]
    fn skip_empty_constraint() {
        assert_skipped("", "1.0.0");
    }
}
