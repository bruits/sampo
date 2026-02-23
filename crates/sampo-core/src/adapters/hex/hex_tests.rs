use super::*;
#[test]
fn version_exists_rejects_empty_name() {
    let err = HexAdapter
        .version_exists("", "1.0.0", None)
        .expect_err("expected empty package name to fail");
    assert!(format!("{}", err).contains("Package name cannot be empty"));
}

mod constraint_validation {
    use super::*;
    use crate::types::ConstraintCheckResult;
    use std::fs;

    fn write_mix_with_dep(dir: &Path, dep_constraint: &str) {
        let content = format!(
            r#"
defmodule Test.MixProject do
  use Mix.Project

  def project do
    [
      app: :test_app,
      version: "0.1.0",
      deps: deps()
    ]
  end

  defp deps do
    [
      {{:test_dep, {dep_constraint}}}
    ]
  end
end
"#
        );
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("mix.exs"), content).unwrap();
    }

    fn assert_constraint(constraint: &str, new_version: &str) -> ConstraintCheckResult {
        let temp = tempfile::tempdir().unwrap();
        write_mix_with_dep(temp.path(), &format!(r#""{}""#, constraint));
        let manifest = temp.path().join("mix.exs");
        check_dependency_constraint(&manifest, "test_dep", "*", new_version).unwrap()
    }

    fn assert_satisfied(constraint: &str, version: &str) {
        let result = assert_constraint(constraint, version);
        assert!(
            matches!(result, ConstraintCheckResult::Satisfied),
            "expected '{}' to satisfy '{}', got {:?}",
            version,
            constraint,
            result
        );
    }

    fn assert_not_satisfied(constraint: &str, version: &str) {
        let result = assert_constraint(constraint, version);
        assert!(
            matches!(result, ConstraintCheckResult::NotSatisfied { .. }),
            "expected '{}' to NOT satisfy '{}', got {:?}",
            version,
            constraint,
            result
        );
    }

    fn assert_skipped(constraint: &str, version: &str) {
        let result = assert_constraint(constraint, version);
        assert!(
            matches!(result, ConstraintCheckResult::Skipped { .. }),
            "expected '{}' to be skipped for '{}', got {:?}",
            version,
            constraint,
            result
        );
    }

    // ~> operator (pessimistic)

    #[test]
    fn pessimistic_two_part_satisfied() {
        assert_satisfied("~> 2.0", "2.5.0");
    }

    #[test]
    fn pessimistic_two_part_exact_lower_bound() {
        assert_satisfied("~> 2.0", "2.0.0");
    }

    #[test]
    fn pessimistic_two_part_not_satisfied_major_bump() {
        assert_not_satisfied("~> 2.0", "3.0.0");
    }

    #[test]
    fn pessimistic_two_part_not_satisfied_below() {
        assert_not_satisfied("~> 2.1", "2.0.0");
    }

    #[test]
    fn pessimistic_three_part_satisfied() {
        assert_satisfied("~> 2.1.0", "2.1.5");
    }

    #[test]
    fn pessimistic_three_part_exact_lower_bound() {
        assert_satisfied("~> 2.1.0", "2.1.0");
    }

    #[test]
    fn pessimistic_three_part_not_satisfied_minor_bump() {
        assert_not_satisfied("~> 2.1.0", "2.2.0");
    }

    #[test]
    fn pessimistic_three_part_not_satisfied_below() {
        assert_not_satisfied("~> 2.1.3", "2.1.2");
    }

    #[test]
    fn pessimistic_zero_minor_satisfied() {
        assert_satisfied("~> 0.1.0", "0.1.5");
    }

    #[test]
    fn pessimistic_zero_minor_not_satisfied() {
        assert_not_satisfied("~> 0.1.0", "0.2.0");
    }

    // Comparison operators

    #[test]
    fn eq_eq_satisfied() {
        assert_satisfied("== 1.2.3", "1.2.3");
    }

    #[test]
    fn eq_eq_not_satisfied() {
        assert_not_satisfied("== 1.2.3", "1.2.4");
    }

    #[test]
    fn gte_satisfied() {
        assert_satisfied(">= 1.0.0", "2.0.0");
    }

    #[test]
    fn gte_exact_satisfied() {
        assert_satisfied(">= 1.0.0", "1.0.0");
    }

    #[test]
    fn gte_not_satisfied() {
        assert_not_satisfied(">= 2.0.0", "1.9.9");
    }

    #[test]
    fn gt_satisfied() {
        assert_satisfied("> 1.0.0", "1.0.1");
    }

    #[test]
    fn gt_not_satisfied_equal() {
        assert_not_satisfied("> 1.0.0", "1.0.0");
    }

    #[test]
    fn lte_satisfied() {
        assert_satisfied("<= 2.0.0", "2.0.0");
    }

    #[test]
    fn lte_not_satisfied() {
        assert_not_satisfied("<= 2.0.0", "2.0.1");
    }

    #[test]
    fn lt_satisfied() {
        assert_satisfied("< 2.0.0", "1.9.9");
    }

    #[test]
    fn lt_not_satisfied() {
        assert_not_satisfied("< 2.0.0", "2.0.0");
    }

    // and/or conjunctions

    #[test]
    fn and_conjunction_satisfied() {
        assert_satisfied(">= 1.0.0 and < 2.0.0", "1.5.0");
    }

    #[test]
    fn and_conjunction_not_satisfied() {
        assert_not_satisfied(">= 1.0.0 and < 2.0.0", "2.0.0");
    }

    #[test]
    fn or_conjunction_satisfied_first() {
        assert_satisfied("== 1.0.0 or == 2.0.0", "1.0.0");
    }

    #[test]
    fn or_conjunction_satisfied_second() {
        assert_satisfied("== 1.0.0 or == 2.0.0", "2.0.0");
    }

    #[test]
    fn or_conjunction_not_satisfied() {
        assert_not_satisfied("== 1.0.0 or == 2.0.0", "3.0.0");
    }

    // Skip cases

    #[test]
    fn dep_not_found_skipped() {
        let temp = tempfile::tempdir().unwrap();
        write_mix_with_dep(temp.path(), r#""~> 1.0""#);
        let manifest = temp.path().join("mix.exs");
        let result = check_dependency_constraint(&manifest, "nonexistent", "*", "1.0.0").unwrap();
        assert!(matches!(result, ConstraintCheckResult::Skipped { .. }));
    }

    #[test]
    fn invalid_version_skipped() {
        assert_skipped("~> 1.0", "invalid");
    }

    #[test]
    fn empty_version_skipped() {
        assert_skipped("~> 1.0", "");
    }

    // Bare pinned version → skipped

    #[test]
    fn bare_pinned_version_skipped() {
        assert_skipped("1.2.3", "1.2.3");
    }

    #[test]
    fn bare_pinned_different_version_skipped() {
        assert_skipped("1.2.3", "1.2.4");
    }

    // Pre-release version → skipped

    #[test]
    fn prerelease_version_skipped() {
        assert_skipped("~> 1.0", "1.0.0-rc.1");
    }

    // Path dependency (no requirement string) → skipped

    #[test]
    fn path_dep_without_requirement_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let content = r#"
defmodule Test.MixProject do
  use Mix.Project

  def project do
    [
      app: :test_app,
      version: "0.1.0",
      deps: deps()
    ]
  end

  defp deps do
    [
      {:test_dep, path: "../test_dep"}
    ]
  end
end
"#;
        fs::write(temp.path().join("mix.exs"), content).unwrap();
        let manifest = temp.path().join("mix.exs");
        let result = check_dependency_constraint(&manifest, "test_dep", "*", "1.0.0").unwrap();
        assert!(
            matches!(result, ConstraintCheckResult::Skipped { .. }),
            "path dep without version requirement should be skipped, got {:?}",
            result
        );
    }
}
