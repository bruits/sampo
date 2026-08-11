use super::*;
use crate::types::ConstraintCheckResult;
use std::fs;

fn write_manifest(dir: &Path, contents: &str) -> std::path::PathBuf {
    let manifest = dir.join("pom.xml");
    fs::write(&manifest, contents).unwrap();
    manifest
}

#[test]
fn registry_url_maps_group_dots_to_path_segments() {
    assert_eq!(
        registry_url("com.example", "my-lib", "1.0.0"),
        "https://repo1.maven.org/maven2/com/example/my-lib/1.0.0/my-lib-1.0.0.pom"
    );
}

#[test]
fn split_coordinates_requires_group_and_artifact() {
    assert_eq!(
        split_coordinates("com.example/my-lib").unwrap(),
        ("com.example", "my-lib")
    );
    assert!(split_coordinates("my-lib").is_err());
    assert!(split_coordinates("/my-lib").is_err());
    assert!(split_coordinates("com.example/").is_err());
}

#[test]
fn check_dependency_constraint_skips_or_warns() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = write_manifest(
        temp.path(),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>cli</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>pinned</artifactId>\n\
         \x20     <version>1.2.3</version>\n\
         \x20   </dependency>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>tracked</artifactId>\n\
         \x20     <version>${project.version}</version>\n\
         \x20   </dependency>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>held</artifactId>\n\
         \x20     <version>${held.version}</version>\n\
         \x20   </dependency>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>ranged</artifactId>\n\
         \x20     <version>[1.0,2.0)</version>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         </project>\n",
    );

    let expect_skip = |dep: &str, expected_reason: &str| match check_dependency_constraint(
        &manifest, dep, "", "2.0.0",
    )
    .unwrap()
    {
        ConstraintCheckResult::Skipped { reason } => assert_eq!(reason, expected_reason),
        other => panic!("expected Skipped for {dep}, got {other:?}"),
    };

    expect_skip("com.example/pinned", "pinned version");
    expect_skip("com.example/tracked", "tracks the project version");
    expect_skip("com.example/ranged", "version range");
    expect_skip(
        "com.example/missing",
        "dependency 'com.example/missing' not found in manifest",
    );

    match check_dependency_constraint(&manifest, "com.example/held", "", "2.0.0").unwrap() {
        ConstraintCheckResult::Unverifiable { constraint } => {
            assert_eq!(constraint, "${held.version}");
        }
        other => panic!("expected Unverifiable for a custom property, got {other:?}"),
    }
}

#[test]
fn legacy_project_version_aliases_track_like_the_canonical_form() {
    // Maven still resolves `${pom.version}` and `${version}` to the project version;
    // warning that such a pin "will not follow" would be factually wrong.
    let temp = tempfile::tempdir().unwrap();
    let manifest = write_manifest(
        temp.path(),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>cli</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>legacy</artifactId>\n\
         \x20     <version>${pom.version}</version>\n\
         \x20   </dependency>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>bare</artifactId>\n\
         \x20     <version>${version}</version>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         </project>\n",
    );

    for dep in ["com.example/legacy", "com.example/bare"] {
        match check_dependency_constraint(&manifest, dep, "", "2.0.0").unwrap() {
            ConstraintCheckResult::Skipped { reason } => {
                assert_eq!(reason, "tracks the project version");
            }
            other => panic!("expected Skipped for {dep}, got {other:?}"),
        }
    }
}

#[test]
fn dependency_management_property_pin_is_unverifiable() {
    // The canonical layout keeps `${dep.version}` pins in <dependencyManagement>; a
    // versionless usage declared earlier must not shadow the entry carrying the pin.
    let temp = tempfile::tempdir().unwrap();
    let manifest = write_manifest(
        temp.path(),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>parent</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <properties>\n\
         \x20   <core.version>1.0.0</core.version>\n\
         \x20 </properties>\n\
         \x20 <dependencies>\n\
         \x20   <dependency>\n\
         \x20     <groupId>com.example</groupId>\n\
         \x20     <artifactId>core</artifactId>\n\
         \x20   </dependency>\n\
         \x20 </dependencies>\n\
         \x20 <dependencyManagement>\n\
         \x20   <dependencies>\n\
         \x20     <dependency>\n\
         \x20       <groupId>com.example</groupId>\n\
         \x20       <artifactId>core</artifactId>\n\
         \x20       <version>${core.version}</version>\n\
         \x20     </dependency>\n\
         \x20   </dependencies>\n\
         \x20 </dependencyManagement>\n\
         </project>\n",
    );

    match check_dependency_constraint(&manifest, "com.example/core", "", "1.1.0").unwrap() {
        ConstraintCheckResult::Unverifiable { constraint } => {
            assert_eq!(constraint, "${core.version}");
        }
        other => panic!("expected Unverifiable for a dependencyManagement pin, got {other:?}"),
    }
}

#[test]
fn version_exists_defers_to_private_deploy_repositories() {
    // A <distributionManagement> release repository pointing away from Central means
    // the public existence check would be meaningless (or a false positive); the
    // adapter must answer "not published" without touching the network.
    let temp = tempfile::tempdir().unwrap();
    let manifest = write_manifest(
        temp.path(),
        "<project>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>internal-lib</artifactId>\n\
         \x20 <version>1.0.0</version>\n\
         \x20 <distributionManagement>\n\
         \x20   <repository>\n\
         \x20     <id>corp</id>\n\
         \x20     <url>https://artifactory.example.com/releases</url>\n\
         \x20   </repository>\n\
         \x20 </distributionManagement>\n\
         </project>\n",
    );

    let exists = MavenAdapter
        .version_exists("com.example/internal-lib", "1.0.0", Some(&manifest))
        .unwrap();
    assert!(!exists);
}

#[test]
fn validate_release_plan_tells_a_stale_parent_from_diverging_bumps() {
    // The planner keeps a coupled pair on one bump level, so divergence at equal
    // versions only reaches here from a caller that built the map some other way.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(
        root.join("pom.xml"),
        "<project>\n  <groupId>com.example</groupId>\n  <artifactId>parent</artifactId>\n  <version>1.0.0</version>\n  <packaging>pom</packaging>\n  <modules>\n    <module>core</module>\n  </modules>\n</project>\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("core")).unwrap();
    fs::write(
        root.join("core/pom.xml"),
        "<project>\n  <parent>\n    <groupId>com.example</groupId>\n    <artifactId>parent</artifactId>\n    <version>1.0.0</version>\n  </parent>\n  <artifactId>core</artifactId>\n</project>\n",
    )
    .unwrap();

    let members = pom::discover(root).unwrap();
    let plan = |child: &str, parent: &str| {
        BTreeMap::from([
            ("maven/com.example/core".to_string(), child.to_string()),
            ("maven/com.example/parent".to_string(), parent.to_string()),
        ])
    };

    // Both POMs read 1.0.0, so nothing is stale — only the planned versions differ.
    let err = validate_release_plan(&members, &plan("1.0.1", "2.0.0")).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("planned bumps diverged") && !message.contains("out of date"),
        "expected the divergence diagnosis, got: {message}"
    );
}
