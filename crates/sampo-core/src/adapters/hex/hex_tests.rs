use super::*;
#[test]
fn version_exists_rejects_empty_name() {
    let err = HexAdapter
        .version_exists("", "1.0.0", None)
        .expect_err("expected empty package name to fail");
    assert!(format!("{}", err).contains("Package name cannot be empty"));
}

#[test]
fn registry_url_uses_repos_path_for_organization() {
    let url = registry_url(Some("acme"), "example", "1.0.0");
    assert_eq!(
        url,
        "https://hex.pm/api/repos/acme/packages/example/releases/1.0.0"
    );
}

#[test]
fn registry_url_uses_public_path_without_organization() {
    let url = registry_url(None, "example", "1.0.0");
    assert_eq!(url, "https://hex.pm/api/packages/example/releases/1.0.0");
}

#[test]
fn resolve_hex_api_key_returns_trimmed_value() {
    let key =
        resolve_hex_api_key(|name| (name == "HEX_API_KEY").then(|| "  secret-key  ".to_string()));
    assert_eq!(key.as_deref(), Some("secret-key"));
}

#[test]
fn resolve_hex_api_key_none_when_blank() {
    let key = resolve_hex_api_key(|_| Some("   ".to_string()));
    assert_eq!(key, None);
}

#[test]
fn resolve_hex_api_key_none_when_unset() {
    let key = resolve_hex_api_key(|_| None);
    assert_eq!(key, None);
}

#[test]
fn version_check_request_attaches_raw_api_key() {
    let client = Client::new();
    let request = version_check_request(
        &client,
        "https://hex.pm/api/repos/acme/packages/example/releases/1.0.0",
        Some("secret-key"),
    )
    .build()
    .expect("request should build");
    let auth = request
        .headers()
        .get(reqwest::header::AUTHORIZATION)
        .expect("authorization header present");
    assert_eq!(auth.to_str().unwrap(), "secret-key");
}

#[test]
fn version_check_request_omits_auth_without_key() {
    let client = Client::new();
    let request = version_check_request(
        &client,
        "https://hex.pm/api/packages/example/releases/1.0.0",
        None,
    )
    .build()
    .expect("request should build");
    assert!(
        request
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .is_none(),
        "expected no authorization header"
    );
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
    fn satisfies_hex_comparator_not_equal_satisfied() {
        assert_satisfied("!= 1.2.3", "2.0.0");
    }

    #[test]
    fn satisfies_hex_comparator_not_equal_not_satisfied() {
        assert_not_satisfied("!= 1.2.3", "1.2.3");
    }

    #[test]
    fn hex_version_satisfies_mixed_and_or() {
        assert_satisfied(">= 1.0.0 and < 2.0.0 or >= 3.0.0", "1.5.0");
        assert_satisfied(">= 1.0.0 and < 2.0.0 or >= 3.0.0", "3.0.0");
        assert_not_satisfied(">= 1.0.0 and < 2.0.0 or >= 3.0.0", "2.5.0");
    }

    #[test]
    fn satisfies_hex_comparator_not_equal_in_conjunction() {
        assert_satisfied("!= 1.0.0 and >= 0.5.0", "0.8.0");
        assert_not_satisfied("!= 1.0.0 and >= 0.5.0", "1.0.0");
    }

    // Skip cases

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

    #[test]
    fn compute_requirement_not_equal() {
        let temp = tempfile::tempdir().unwrap();
        write_mix_with_dep(temp.path(), r#""!= 1.0.0""#);
        let manifest = temp.path().join("mix.exs");
        let input = fs::read_to_string(&manifest).unwrap();
        let mut new_versions = std::collections::BTreeMap::new();
        new_versions.insert("test_dep".to_string(), "2.0.0".to_string());
        let (output, updated) =
            update_manifest_versions(&manifest, &input, None, &new_versions).unwrap();
        assert!(
            output.contains("!= 2.0.0"),
            "expected constraint updated to '!= 2.0.0', got:\n{}",
            output
        );
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0], ("test_dep".to_string(), "2.0.0".to_string()));
    }
}

/// A Gleam manifest routes through the shared Hex constraint/version machinery.
mod gleam_dispatch {
    use super::*;
    use crate::types::ConstraintCheckResult;
    use std::fs;

    fn write_gleam_with_dep(dir: &Path, dep_constraint: &str) {
        let content = format!(
            "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\ntest_dep = {}\n",
            dep_constraint
        );
        fs::write(dir.join("gleam.toml"), content).unwrap();
    }

    #[test]
    fn constraint_satisfied_from_gleam_manifest() {
        let temp = tempfile::tempdir().unwrap();
        write_gleam_with_dep(temp.path(), r#"">= 1.0.0 and < 2.0.0""#);
        let manifest = temp.path().join("gleam.toml");
        let result = check_dependency_constraint(&manifest, "test_dep", "*", "1.5.0").unwrap();
        assert!(matches!(result, ConstraintCheckResult::Satisfied));
    }

    #[test]
    fn constraint_not_satisfied_from_gleam_manifest() {
        let temp = tempfile::tempdir().unwrap();
        write_gleam_with_dep(temp.path(), r#"">= 1.0.0 and < 2.0.0""#);
        let manifest = temp.path().join("gleam.toml");
        let result = check_dependency_constraint(&manifest, "test_dep", "*", "2.5.0").unwrap();
        assert!(matches!(result, ConstraintCheckResult::NotSatisfied { .. }));
    }

    #[test]
    fn update_bumps_gleam_dependency() {
        let temp = tempfile::tempdir().unwrap();
        write_gleam_with_dep(temp.path(), r#""~> 1.0""#);
        let manifest = temp.path().join("gleam.toml");
        let input = fs::read_to_string(&manifest).unwrap();
        let mut new_versions = std::collections::BTreeMap::new();
        new_versions.insert("test_dep".to_string(), "1.5.0".to_string());
        let (output, updated) =
            update_manifest_versions(&manifest, &input, None, &new_versions).unwrap();
        assert!(output.contains("~> 1.5.0"), "got:\n{}", output);
        assert_eq!(updated, vec![("test_dep".to_string(), "1.5.0".to_string())]);
    }
}

/// An Erlang `.app.src` manifest routes through the shared Hex dispatch, and Erlang
/// packages coexist with Mix ones in a mixed BEAM workspace.
mod rebar3_dispatch {
    use super::*;
    use crate::types::{ConstraintCheckResult, PackageKind};
    use std::collections::BTreeMap;
    use std::fs;

    fn write_erlang_app(root: &Path, name: &str, vsn: &str) -> std::path::PathBuf {
        let manifest = root.join("src").join(format!("{name}.app.src"));
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        fs::write(
            &manifest,
            format!("{{application, {name}, [{{vsn, \"{vsn}\"}}]}}.\n"),
        )
        .unwrap();
        manifest
    }

    #[test]
    fn is_rebar_manifest_only_matches_app_src() {
        assert!(is_rebar_manifest(Path::new("src/app.app.src")));
        assert!(!is_rebar_manifest(Path::new("gleam.toml")));
        assert!(!is_rebar_manifest(Path::new("mix.exs")));
        // The dynamic script variant is not the manifest we manage.
        assert!(!is_rebar_manifest(Path::new("src/app.app.src.script")));
    }

    #[test]
    fn constraint_checked_from_rebar_config() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_erlang_app(temp.path(), "app", "1.0.0");
        fs::write(
            temp.path().join("rebar.config"),
            "{deps, [{test_dep, \"~> 1.0\"}]}.\n",
        )
        .unwrap();

        let result = check_dependency_constraint(&manifest, "test_dep", "*", "1.5.0").unwrap();
        assert!(matches!(result, ConstraintCheckResult::Satisfied));
    }

    #[test]
    fn update_bumps_app_src_version_via_dispatch() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_erlang_app(temp.path(), "app", "1.0.0");
        let input = fs::read_to_string(&manifest).unwrap();
        let (output, applied) =
            update_manifest_versions(&manifest, &input, Some("2.0.0"), &BTreeMap::new()).unwrap();
        assert!(output.contains("{vsn, \"2.0.0\"}"), "got:\n{output}");
        assert!(applied.is_empty());
    }

    #[test]
    fn discovers_mix_and_erlang_packages_together() {
        let temp = tempfile::tempdir().unwrap();
        // A Mix package at the root...
        fs::write(
            temp.path().join("mix.exs"),
            "defmodule App.MixProject do\n  use Mix.Project\n  def project do\n    [app: :elixir_app, version: \"0.1.0\"]\n  end\nend\n",
        )
        .unwrap();
        // ...and an Erlang application beside it.
        write_erlang_app(&temp.path().join("apps").join("erl"), "erl", "3.0.0");

        let mut packages = HexAdapter.discover(temp.path()).unwrap();
        packages.sort_by(|l, r| l.name.cmp(&r.name));
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"elixir_app"), "got {names:?}");
        assert!(names.contains(&"erl"), "got {names:?}");
        assert!(packages.iter().all(|p| p.kind == PackageKind::Hex));
    }
}
