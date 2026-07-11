use super::*;
use crate::types::PackageKind;
use std::collections::BTreeMap;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn simple_pom(group_id: &str, artifact_id: &str, version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n\
         \x20 <modelVersion>4.0.0</modelVersion>\n\
         \x20 <groupId>{group_id}</groupId>\n\
         \x20 <artifactId>{artifact_id}</artifactId>\n\
         \x20 <version>{version}</version>\n\
         </project>\n"
    )
}

#[test]
fn can_discover_requires_root_pom() {
    let temp = tempfile::tempdir().unwrap();
    assert!(!can_discover(temp.path()));
    write_file(
        &temp.path().join("pom.xml"),
        &simple_pom("com.example", "lib", "1.0.0"),
    );
    assert!(can_discover(temp.path()));
}

#[test]
fn discover_single_root_package() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        &simple_pom("com.example", "my-lib", "1.2.3"),
    );

    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);
    let pkg = &packages[0];
    assert_eq!(pkg.name, "com.example/my-lib");
    assert_eq!(pkg.version, "1.2.3");
    assert_eq!(pkg.kind, PackageKind::Maven);
    assert_eq!(pkg.identifier, "maven/com.example/my-lib");
    assert_eq!(pkg.path, temp.path().join("pom.xml").parent().unwrap());
}

#[test]
fn discover_multi_module_reactor_with_inherited_versions() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>parent</artifactId>\n\
         \x20 <version>1.2.3</version>\n\
         \x20 <packaging>pom</packaging>\n\
         \x20 <modules>\n\
         \x20   <module>core</module>\n\
         \x20   <module>cli</module>\n\
         \x20 </modules>\n\
         </project>\n",
    );
    // `core` inherits both groupId and version from the parent block.
    write_file(
        &temp.path().join("core/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.2.3</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>core</artifactId>\n\
         </project>\n",
    );
    // `cli` has its own version and depends on its sibling via `${project.groupId}`.
    write_file(
        &temp.path().join("cli/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.2.3</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>cli</artifactId>\n\
         \x20 <version>0.9.0</version>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>${project.groupId}</groupId>\n\
         \x20     <artifactId>core</artifactId>\n\
         \x20     <version>1.2.3</version>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         </project>\n",
    );

    let mut packages = discover(temp.path()).unwrap();
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(packages.len(), 3);

    let cli = &packages[0];
    assert_eq!(cli.name, "com.example/cli");
    assert_eq!(cli.version, "0.9.0");
    assert!(cli.internal_deps.contains("maven/com.example/parent"));
    assert!(cli.internal_deps.contains("maven/com.example/core"));

    let core = &packages[1];
    assert_eq!(core.name, "com.example/core");
    assert_eq!(core.version, "1.2.3");
    assert!(core.internal_deps.contains("maven/com.example/parent"));

    let parent = &packages[2];
    assert_eq!(parent.name, "com.example/parent");
    assert!(parent.internal_deps.is_empty());
}

#[test]
fn discover_treats_test_scoped_deps_as_dev_deps() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>root</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <packaging>pom</packaging>\n\
         \x20 <modules>\n\
         \x20   <module>a</module>\n\
         \x20   <module>b</module>\n\
         \x20 </modules>\n\
         </project>\n",
    );
    write_file(
        &temp.path().join("a/pom.xml"),
        &simple_pom("com.example", "a", "1.0.0"),
    );
    write_file(
        &temp.path().join("b/pom.xml"),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>b</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>a</artifactId>\n\
         \x20     <version>1.0.0</version>\n\
         \x20     <scope>test</scope>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         </project>\n",
    );

    let packages = discover(temp.path()).unwrap();
    let b = packages.iter().find(|p| p.name == "com.example/b").unwrap();
    assert!(b.internal_dev_deps.contains("maven/com.example/a"));
    assert!(!b.internal_deps.contains("maven/com.example/a"));
}

#[test]
fn discover_skips_custom_named_module_poms() {
    // Sampo resolves every manifest as <dir>/pom.xml downstream, so a <module> entry
    // naming a custom POM file must be skipped rather than silently misresolved.
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>root</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <packaging>pom</packaging>\n\
         \x20 <modules>\n\
         \x20   <module>sub/custom-pom.xml</module>\n\
         \x20   <module>standard/pom.xml</module>\n\
         \x20 </modules>\n\
         </project>\n",
    );
    write_file(
        &temp.path().join("sub/custom-pom.xml"),
        &simple_pom("com.example", "sub", "1.0.0"),
    );
    write_file(
        &temp.path().join("standard/pom.xml"),
        &simple_pom("com.example", "standard", "1.0.0"),
    );

    let packages = discover(temp.path()).unwrap();
    // A file entry named pom.xml still works; the custom-named one is skipped.
    assert_eq!(packages.len(), 2);
    assert!(packages.iter().any(|p| p.name == "com.example/standard"));
    assert!(!packages.iter().any(|p| p.name == "com.example/sub"));
}

#[test]
fn discover_resolves_parent_group_id_property() {
    // A module overriding its groupId can still reference siblings through
    // `${project.parent.groupId}`, which must resolve against the <parent> block.
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>parent</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <packaging>pom</packaging>\n\
         \x20 <modules>\n\
         \x20   <module>core</module>\n\
         \x20   <module>plugin</module>\n\
         \x20 </modules>\n\
         </project>\n",
    );
    write_file(
        &temp.path().join("core/pom.xml"),
        &simple_pom("com.example", "core", "1.0.0"),
    );
    write_file(
        &temp.path().join("plugin/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.0.0</version>\n\
         \x20 </parent>\n\
         \x20 <groupId>com.example.plugins</groupId>\n\
         \x20 <artifactId>plugin</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>${project.parent.groupId}</groupId>\n\
         \x20     <artifactId>core</artifactId>\n\
         \x20     <version>1.0.0</version>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         </project>\n",
    );

    let packages = discover(temp.path()).unwrap();
    let plugin = packages
        .iter()
        .find(|p| p.name == "com.example.plugins/plugin")
        .unwrap();
    assert!(plugin.internal_deps.contains("maven/com.example/core"));
}

#[test]
fn discover_skips_snapshot_versions() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        &simple_pom("com.example", "lib", "1.2.3-SNAPSHOT"),
    );

    let packages = discover(temp.path()).unwrap();
    assert!(packages.is_empty());
}

#[test]
fn discover_skips_property_versions() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        &simple_pom("com.example", "lib", "${revision}"),
    );

    let packages = discover(temp.path()).unwrap();
    assert!(packages.is_empty());
}

#[test]
fn discover_skips_versionless_packages() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>lib</artifactId>\n\
         </project>\n",
    );

    let packages = discover(temp.path()).unwrap();
    assert!(packages.is_empty());
}

#[test]
fn is_publishable_accepts_static_versions() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("pom.xml");
    write_file(&manifest, &simple_pom("com.example", "lib", "1.0.0"));
    assert!(is_publishable(&manifest).unwrap());
}

#[test]
fn is_publishable_errors_without_version() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("pom.xml");
    write_file(
        &manifest,
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>lib</artifactId>\n\
         </project>\n",
    );
    let err = is_publishable(&manifest).unwrap_err();
    assert!(err.to_string().contains("missing a version field"));
}

#[test]
fn is_publishable_honors_maven_deploy_skip() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("pom.xml");
    write_file(
        &manifest,
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>lib</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <properties>\n\
         \x20   <maven.deploy.skip>true</maven.deploy.skip>\n\
         \x20 </properties>\n\
         </project>\n",
    );
    assert!(!is_publishable(&manifest).unwrap());
}

#[test]
fn update_splices_own_version_and_preserves_bytes() {
    let input = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!-- release managed by sampo -->\n\
         <project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>lib</artifactId>\n\
         \x20 <version>1.2.3</version>\n\
         \x20 <name>lib</name>\n\
         </project>\n";

    let (output, applied) =
        update_manifest_versions(Path::new("pom.xml"), input, Some("1.3.0"), &BTreeMap::new())
            .unwrap();

    assert_eq!(
        output,
        input.replace("<version>1.2.3</version>", "<version>1.3.0</version>")
    );
    assert!(applied.is_empty());
}

#[test]
fn update_rewrites_parent_and_dependency_references() {
    let input = "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.2.3</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>cli</artifactId>\n\
         \x20 <version>0.9.0</version>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>${project.groupId}</groupId>\n\
         \x20     <artifactId>core</artifactId>\n\
         \x20     <version>1.2.3</version>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         </project>\n";

    let mut versions = BTreeMap::new();
    versions.insert("com.example/parent".to_string(), "2.0.0".to_string());
    versions.insert("com.example/core".to_string(), "2.0.0".to_string());

    let (output, applied) =
        update_manifest_versions(Path::new("pom.xml"), input, Some("1.0.0"), &versions).unwrap();

    // Parent ref and sibling dep both move to 2.0.0, the package itself to 1.0.0;
    // everything else is byte-identical.
    let expected = input.replace("1.2.3", "2.0.0").replace("0.9.0", "1.0.0");
    assert_eq!(output, expected);
    assert_eq!(
        applied,
        vec![
            ("com.example/core".to_string(), "2.0.0".to_string()),
            ("com.example/parent".to_string(), "2.0.0".to_string()),
        ]
    );
}

#[test]
fn update_leaves_project_version_references_alone() {
    let input = "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>cli</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>core</artifactId>\n\
         \x20     <version>${project.version}</version>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         </project>\n";

    let mut versions = BTreeMap::new();
    versions.insert("com.example/core".to_string(), "2.0.0".to_string());

    let (output, applied) =
        update_manifest_versions(Path::new("pom.xml"), input, None, &versions).unwrap();

    assert_eq!(output, input);
    assert!(applied.is_empty());
}

#[test]
fn update_accepts_inherited_version_when_parent_matches() {
    let input = "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.2.3</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>core</artifactId>\n\
         </project>\n";

    let mut versions = BTreeMap::new();
    versions.insert("com.example/parent".to_string(), "2.0.0".to_string());

    let (output, applied) =
        update_manifest_versions(Path::new("pom.xml"), input, Some("2.0.0"), &versions).unwrap();

    assert!(output.contains("<version>2.0.0</version>"));
    assert_eq!(
        applied,
        vec![("com.example/parent".to_string(), "2.0.0".to_string())]
    );
}

#[test]
fn update_rejects_inherited_version_without_parent_release() {
    let input = "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.2.3</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>core</artifactId>\n\
         </project>\n";

    let err =
        update_manifest_versions(Path::new("pom.xml"), input, Some("2.0.0"), &BTreeMap::new())
            .unwrap_err();
    assert!(err.to_string().contains("inherits its version"));
}

#[test]
fn update_without_changes_returns_input() {
    let input = simple_pom("com.example", "lib", "1.0.0");
    let (output, applied) = update_manifest_versions(
        Path::new("pom.xml"),
        &input,
        Some("1.0.0"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(output, input);
    assert!(applied.is_empty());
}

#[test]
fn find_dependency_constraint_reads_literals_and_properties() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("pom.xml");
    write_file(
        &manifest,
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.2.3</version>\n\
         \x20 </parent>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>cli</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>core</artifactId>\n\
         \x20     <version>1.2.3</version>\n\
         \x20   </dependency>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>util</artifactId>\n\
         \x20     <version>${project.version}</version>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         </project>\n",
    );

    assert_eq!(
        find_dependency_constraint_value(&manifest, "com.example/core").unwrap(),
        Some("1.2.3".to_string())
    );
    assert_eq!(
        find_dependency_constraint_value(&manifest, "com.example/util").unwrap(),
        Some("${project.version}".to_string())
    );
    assert_eq!(
        find_dependency_constraint_value(&manifest, "com.example/parent").unwrap(),
        Some("1.2.3".to_string())
    );
    assert_eq!(
        find_dependency_constraint_value(&manifest, "com.example/missing").unwrap(),
        None
    );
}

#[test]
fn publish_args_injects_guarded_flags() {
    assert_eq!(
        publish_args(false, &[]),
        vec!["--batch-mode", "--non-recursive", "deploy"]
    );
    assert_eq!(
        publish_args(true, &[]),
        vec!["--batch-mode", "--non-recursive", "verify"]
    );

    // User-supplied flags win over the injected ones.
    let args = publish_args(false, &["-B".to_string(), "-N".to_string()]);
    assert_eq!(args, vec!["deploy", "-B", "-N"]);

    let args = publish_args(false, &["-DskipTests".to_string()]);
    assert_eq!(
        args,
        vec!["--batch-mode", "--non-recursive", "deploy", "-DskipTests"]
    );
}

#[test]
fn dependency_management_does_not_create_ordering_edges() {
    // A parent aggregator pinning its own modules in <dependencyManagement> must not
    // gain publish-order edges: combined with each child's <parent> edge back to the
    // aggregator, they would form a cycle and topo_order would reject the publish run.
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n\
         \x20 <modelVersion>4.0.0</modelVersion>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>parent</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <packaging>pom</packaging>\n\
         \x20 <modules>\n\
         \x20   <module>a</module>\n\
         \x20   <module>b</module>\n\
         \x20 </modules>\n\
         \x20 <dependencyManagement>\n\
         \x20   <dependencies>\n\
         \x20     <dependency>\n\
         \x20       <groupId>com.example</groupId>\n\
         \x20       <artifactId>a</artifactId>\n\
         \x20       <version>1.0.0</version>\n\
         \x20     </dependency>\n\
         \x20     <dependency>\n\
         \x20       <groupId>com.example</groupId>\n\
         \x20       <artifactId>b</artifactId>\n\
         \x20       <version>1.0.0</version>\n\
         \x20     </dependency>\n\
         \x20   </dependencies>\n\
         \x20 </dependencyManagement>\n\
         </project>\n",
    );
    write_file(
        &temp.path().join("a/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.0.0</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>a</artifactId>\n\
         </project>\n",
    );
    write_file(
        &temp.path().join("b/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.0.0</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>b</artifactId>\n\
         </project>\n",
    );

    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 3);

    let parent = packages
        .iter()
        .find(|p| p.name == "com.example/parent")
        .unwrap();
    // <dependencyManagement> pins must not constrain publish order.
    assert!(
        !parent.internal_deps.contains("maven/com.example/a"),
        "parent internal_deps must not contain child a (got {:?})",
        parent.internal_deps
    );
    assert!(
        !parent.internal_deps.contains("maven/com.example/b"),
        "parent internal_deps must not contain child b (got {:?})",
        parent.internal_deps
    );
    // They should still be version-rewritten on release, like test-scoped deps.
    assert!(parent.internal_dev_deps.contains("maven/com.example/a"));
    assert!(parent.internal_dev_deps.contains("maven/com.example/b"));

    // The children's <parent> edges remain real publish-order dependencies.
    for child in ["com.example/a", "com.example/b"] {
        let pkg = packages.iter().find(|p| p.name == child).unwrap();
        assert!(pkg.internal_deps.contains("maven/com.example/parent"));
    }
}

#[test]
fn update_rewrites_dependency_management_pins() {
    let input = "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>parent</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <dependencyManagement>\n\
         \x20   <dependencies>\n\
         \x20     <dependency>\n\
         \x20       <groupId>com.example</groupId>\n\
         \x20       <artifactId>core</artifactId>\n\
         \x20       <version>1.0.0</version>\n\
         \x20     </dependency>\n\
         \x20   </dependencies>\n\
         \x20 </dependencyManagement>\n\
         </project>\n";

    let mut versions = BTreeMap::new();
    versions.insert("com.example/core".to_string(), "1.1.0".to_string());

    let (output, applied) =
        update_manifest_versions(Path::new("pom.xml"), input, None, &versions).unwrap();

    assert!(output.contains("<version>1.1.0</version>"));
    assert_eq!(
        applied,
        vec![("com.example/core".to_string(), "1.1.0".to_string())]
    );
}

#[test]
fn is_publishable_inherits_maven_deploy_skip_from_parent() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>parent</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <packaging>pom</packaging>\n\
         \x20 <properties>\n\
         \x20   <maven.deploy.skip>true</maven.deploy.skip>\n\
         \x20 </properties>\n\
         </project>\n",
    );
    // `child` inherits the property through the default ../pom.xml parent resolution.
    write_file(
        &temp.path().join("child/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.0.0</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>child</artifactId>\n\
         </project>\n",
    );
    // `override` re-enables deployment locally: the nearest definition wins.
    write_file(
        &temp.path().join("override/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.0.0</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>override</artifactId>\n\
         \x20 <properties>\n\
         \x20   <maven.deploy.skip>false</maven.deploy.skip>\n\
         \x20 </properties>\n\
         </project>\n",
    );

    assert!(!is_publishable(&temp.path().join("child/pom.xml")).unwrap());
    assert!(is_publishable(&temp.path().join("override/pom.xml")).unwrap());
}

#[test]
fn parent_walk_stops_at_external_parents() {
    // A POM whose parent is repository-resolved (spring-boot-starter-parent style)
    // must not walk into whatever file happens to sit at ../pom.xml — here an
    // unrelated POM outside the workspace that would wrongly mark it unpublishable.
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<project>\n\
         \x20 <groupId>com.unrelated</groupId>\n\
         \x20 <artifactId>other-checkout</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <properties>\n\
         \x20   <maven.deploy.skip>true</maven.deploy.skip>\n\
         \x20 </properties>\n\
         </project>\n",
    );
    write_file(
        &temp.path().join("repo/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>org.springframework.boot</groupId>\n\
         \x20   <artifactId>spring-boot-starter-parent</artifactId>\n\
         \x20   <version>3.2.0</version>\n\
         \x20 </parent>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>app</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         </project>\n",
    );

    assert!(is_publishable(&temp.path().join("repo/pom.xml")).unwrap());
}

#[test]
fn private_deploy_repository_inherits_from_parent() {
    // <distributionManagement> is declared once at the root in the canonical private
    // setup; children inherit it through the validated parent chain.
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("pom.xml"),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>parent</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <packaging>pom</packaging>\n\
         \x20 <distributionManagement>\n\
         \x20   <repository>\n\
         \x20     <id>corp</id>\n\
         \x20     <url>https://artifactory.example.com/releases</url>\n\
         \x20   </repository>\n\
         \x20 </distributionManagement>\n\
         </project>\n",
    );
    write_file(
        &temp.path().join("child/pom.xml"),
        "<project>\n\
         \x20 <parent>\n\
         \x20   <groupId>com.example</groupId>\n\
         \x20   <artifactId>parent</artifactId>\n\
         \x20   <version>1.0.0</version>\n\
         \x20 </parent>\n\
         \x20 <artifactId>child</artifactId>\n\
         </project>\n",
    );

    assert!(has_private_deploy_repository(&temp.path().join("pom.xml")));
    assert!(has_private_deploy_repository(
        &temp.path().join("child/pom.xml")
    ));
}
