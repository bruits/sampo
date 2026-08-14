//! The preflight that runs before a release mutates anything.
//!
//! Its own test binary: it empties `PATH` process-wide, which no other test may observe.
#![cfg(unix)]

mod common;

use sampo_core::{enter_prerelease, run_release};

#[test]
fn missing_bun_fails_before_the_release_is_applied() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    common::write_workspace(root);
    let _guard = common::use_only(&root.join("empty-bin"));

    let err = run_release(root, false).expect_err("a missing bun must fail the release");
    let message = err.to_string();
    assert!(
        message.contains("bun") && message.contains("PATH"),
        "must name the missing tool: {message}"
    );

    common::assert_workspace_untouched(root);
}

#[test]
fn missing_bun_fails_before_prerelease_entry_is_applied() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    common::write_workspace(root);
    let _guard = common::use_only(&root.join("empty-bin"));

    let err = enter_prerelease(root, &["pkg-a".to_string()], "alpha")
        .expect_err("a missing bun must fail prerelease entry");
    assert!(
        err.to_string().contains("bun"),
        "must name the missing tool: {err}"
    );

    common::assert_workspace_untouched(root);
}
