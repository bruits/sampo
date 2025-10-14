use super::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn npm_adapter_discovers_single_package() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    fs::write(
        root.join("package.json"),
        r#"{
  "name": "root-pkg",
  "version": "0.1.0"
}
"#,
    )
    .unwrap();

    let packages = NpmAdapter.discover(root).unwrap();
    assert_eq!(packages.len(), 1);
    let pkg = &packages[0];
    assert_eq!(pkg.name, "root-pkg");
    assert_eq!(pkg.version, "0.1.0");
    assert_eq!(pkg.kind, PackageKind::Npm);
    assert!(pkg.internal_deps.is_empty());
}

#[test]
fn npm_adapter_discovers_workspace_members_and_internal_deps() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    fs::write(
        root.join("package.json"),
        r#"{
  "name": "root-workspace",
  "version": "1.0.0",
  "workspaces": ["packages/*"]
}
"#,
    )
    .unwrap();

    fs::write(
        root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'extras/*'\n",
    )
    .unwrap();

    let packages_dir = root.join("packages");
    fs::create_dir_all(packages_dir.join("pkg-a")).unwrap();
    fs::create_dir_all(packages_dir.join("pkg-b")).unwrap();

    fs::write(
        packages_dir.join("pkg-a/package.json"),
        r#"{
  "name": "pkg-a",
  "version": "0.1.0",
  "dependencies": {
    "pkg-b": "^0.2.0"
  }
}
"#,
    )
    .unwrap();

    fs::write(
        packages_dir.join("pkg-b/package.json"),
        r#"{
  "name": "pkg-b",
  "version": "0.2.0"
}
"#,
    )
    .unwrap();

    let extras_dir = root.join("extras");
    fs::create_dir_all(extras_dir.join("pkg-c")).unwrap();
    fs::write(
        extras_dir.join("pkg-c/package.json"),
        r#"{
  "name": "pkg-c",
  "version": "0.3.0"
}
"#,
    )
    .unwrap();

    let packages = NpmAdapter.discover(root).unwrap();
    assert_eq!(packages.len(), 4);

    let root_pkg = packages
        .iter()
        .find(|p| p.name == "root-workspace")
        .unwrap();
    assert_eq!(root_pkg.kind, PackageKind::Npm);

    let pkg_a = packages.iter().find(|p| p.name == "pkg-a").unwrap();
    assert!(
        pkg_a
            .internal_deps
            .contains(&PackageInfo::dependency_identifier(
                PackageKind::Npm,
                "pkg-b"
            ))
    );

    assert!(packages.iter().any(|p| p.name == "pkg-c"));
}

#[test]
fn updates_package_json_versions_preserving_formatting() {
    let input = r#"{
  "name": "app",
  "version": "1.0.0",
  "dependencies": {
    "pkg-a": "^1.0.0",
    "pkg-b": "workspace:*",
    "pkg-c": "file:../pkg-c",
    "pkg-d": "workspace:^1.0.0"
  },
  "devDependencies": {
    "pkg-a": "~1.0.0"
  }
}
"#;
    let mut updates = BTreeMap::new();
    updates.insert("pkg-a".to_string(), "2.0.0".to_string());
    updates.insert("pkg-b".to_string(), "3.0.0".to_string());
    updates.insert("pkg-c".to_string(), "4.0.0".to_string());
    updates.insert("pkg-d".to_string(), "1.5.0".to_string());

    let (out, applied) = update_manifest_versions(
        Path::new("/repo/package.json"),
        input,
        Some("1.1.0"),
        &updates,
    )
    .unwrap();

    assert!(out.contains("\"version\": \"1.1.0\""));
    assert!(out.contains("\"pkg-a\": \"^2.0.0\""));
    assert!(out.contains("\"pkg-b\": \"workspace:*\""));
    assert!(out.contains("\"pkg-c\": \"file:../pkg-c\""));
    assert!(out.contains("\"pkg-d\": \"workspace:^1.5.0\""));
    assert!(out.contains("\"pkg-a\": \"~2.0.0\""));
    assert!(applied.contains(&("pkg-a".to_string(), "2.0.0".to_string())));
    assert!(applied.contains(&("pkg-d".to_string(), "1.5.0".to_string())));
    assert!(!applied.iter().any(|(name, _)| name == "pkg-b"));
    assert!(!applied.iter().any(|(name, _)| name == "pkg-c"));
}
