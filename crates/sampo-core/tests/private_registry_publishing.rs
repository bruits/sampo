//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use sampo_core::publish::PublishExtraArgs;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const PRIVATE_GEMSPEC: &str = "Gem::Specification.new do |spec|\n  spec.name = \"private\"\n  \
     spec.version = \"1.0.0\"\n  \
     spec.metadata[\"allowed_push_host\"] = \"https://gems.example.com\"\nend\n";

/// A gem for a private gem server, and a `gem` stub that logs its arguments.
fn private_gem(root: &Path) -> std::sync::MutexGuard<'static, ()> {
    let bin = root.join("fake-bin");
    fs::create_dir_all(&bin).unwrap();
    let gem = bin.join("gem");
    fs::write(
        &gem,
        format!(
            "#!/bin/sh\nprintf 'gem %s\\n' \"$*\" >> '{}'\nexit 0\n",
            root.join("argv.log").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&gem, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.join("private.gemspec"), PRIVATE_GEMSPEC).unwrap();

    let guard = common::use_only(&bin);
    common::commit_repo(root, "");
    guard
}

fn pushes(root: &Path) -> usize {
    fs::read_to_string(root.join("argv.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("gem push"))
        .count()
}

#[test]
fn a_tagged_version_on_a_private_registry_is_not_pushed_again() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let _guard = private_gem(root);

    let first = sampo_core::run_publish(root, false, &PublishExtraArgs::default()).unwrap();
    assert_eq!(first.tags, ["private-v1.0.0"]);
    assert_eq!(pushes(root), 1);

    let second = sampo_core::run_publish(root, false, &PublishExtraArgs::default()).unwrap();
    assert!(second.tags.is_empty(), "{:?}", second.tags);
    assert_eq!(pushes(root), 1);
}

#[test]
fn an_untagged_version_on_a_private_registry_is_pushed_again() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let _guard = private_gem(root);

    sampo_core::run_publish(root, false, &PublishExtraArgs::default()).unwrap();
    common::git(root, &["tag", "-d", "private-v1.0.0"]);

    let second = sampo_core::run_publish(root, false, &PublishExtraArgs::default()).unwrap();
    assert_eq!(second.tags, ["private-v1.0.0"]);
    assert_eq!(pushes(root), 2);
}
