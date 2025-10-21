use super::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn discover_single_mix_package() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_file(
        &root.join("mix.exs"),
        r#"
defmodule Example.MixProject do
  use Mix.Project

  def project do
    [
      app: :example,
      version: "0.1.0",
      deps: deps()
    ]
  end

  defp deps do
    []
  end
end
"#,
    );

    let packages = HexAdapter.discover(root).unwrap();
    assert_eq!(packages.len(), 1);
    let pkg = &packages[0];
    assert_eq!(pkg.name, "example");
    assert_eq!(pkg.version, "0.1.0");
    assert_eq!(pkg.kind, PackageKind::Hex);
    assert!(pkg.internal_deps.is_empty());
}

#[test]
fn discover_umbrella_projects_with_internal_path_deps() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    write_file(
        &root.join("mix.exs"),
        r#"
defmodule Umbrella.MixProject do
  use Mix.Project

  def project do
    [
      apps_path: "apps",
      deps: deps()
    ]
  end

  defp deps do
    []
  end
end
"#,
    );

    write_file(
        &root.join("apps/foo/mix.exs"),
        r#"
defmodule Foo.MixProject do
  use Mix.Project

  def project do
    [
      app: :foo,
      version: "0.1.0",
      deps: deps()
    ]
  end

  defp deps do
    [
      {:bar, path: "../bar"}
    ]
  end
end
"#,
    );

    write_file(
        &root.join("apps/bar/mix.exs"),
        r#"
defmodule Bar.MixProject do
  use Mix.Project

  def project do
    [
      app: :bar,
      version: "0.2.0",
      deps: deps()
    ]
  end

  defp deps do
    []
  end
end
"#,
    );

    let mut packages = HexAdapter.discover(root).unwrap();
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].name, "bar");
    assert!(packages[0].internal_deps.is_empty());

    assert_eq!(packages[1].name, "foo");
    assert!(packages[1].internal_deps.contains("hex/bar"));
}

#[test]
fn update_manifest_versions_updates_version_and_dependency() {
    let manifest = r#"
defmodule Foo.MixProject do
  use Mix.Project

  def project do
    [
      app: :foo,
      version: "0.1.0",
      deps: deps()
    ]
  end

  defp deps do
    [
      {:bar, "~> 0.2.0"}
    ]
  end
end
"#;

    let mut versions = BTreeMap::new();
    versions.insert("bar".to_string(), "0.3.0".to_string());

    let (updated, applied) =
        update_manifest_versions(Path::new("mix.exs"), manifest, Some("0.2.0"), &versions).unwrap();

    assert!(updated.contains(r#"version: "0.2.0""#));
    assert!(updated.contains(r#"{:bar, "~> 0.3.0"}"#));
    assert_eq!(applied, vec![("bar".to_string(), "0.3.0".to_string())]);
}

#[test]
fn update_manifest_versions_skips_complex_requirement() {
    let manifest = r#"
defmodule Foo.MixProject do
  use Mix.Project

  def project do
    [
      app: :foo,
      version: "0.1.0",
      deps: deps()
    ]
  end

  defp deps do
    [
      {:bar, ">= 0.2.0 and < 0.3.0"}
    ]
  end
end
"#;

    let mut versions = BTreeMap::new();
    versions.insert("bar".to_string(), "0.4.0".to_string());

    let (updated, applied) =
        update_manifest_versions(Path::new("mix.exs"), manifest, None, &versions).unwrap();

    assert_eq!(updated, manifest);
    assert!(applied.is_empty());
}

#[test]
fn is_publishable_requires_app_and_version() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("mix.exs");

    write_file(
        &manifest,
        r#"
defmodule Example.MixProject do
  use Mix.Project

  def project do
    [
      version: "0.1.0",
      deps: deps()
    ]
  end

  defp deps do
    []
  end
end
"#,
    );
    let err = HexAdapter.is_publishable(&manifest).unwrap_err();
    assert!(format!("{}", err).contains("missing an :app declaration"));

    write_file(
        &manifest,
        r#"
defmodule Example.MixProject do
  use Mix.Project

  def project do
    [
      app: :example,
      deps: deps()
    ]
  end

  defp deps do
    []
  end
end
"#,
    );
    let err = HexAdapter.is_publishable(&manifest).unwrap_err();
    assert!(format!("{}", err).contains("missing a version field"));
}

#[test]
fn version_exists_rejects_empty_name() {
    let err = HexAdapter
        .version_exists("", "1.0.0", None)
        .expect_err("expected empty package name to fail");
    assert!(format!("{}", err).contains("Package name cannot be empty"));
}

#[test]
fn regenerate_lockfile_requires_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let err = HexAdapter
        .regenerate_lockfile(temp.path())
        .expect_err("expected missing manifest to fail");
    assert!(format!("{}", err).contains("mix.exs"));
}
