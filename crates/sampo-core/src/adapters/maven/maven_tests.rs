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
fn check_dependency_constraint_always_skips() {
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
    expect_skip("com.example/tracked", "property-managed version");
    expect_skip("com.example/ranged", "version range");
    expect_skip(
        "com.example/missing",
        "dependency 'com.example/missing' not found in manifest",
    );
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
