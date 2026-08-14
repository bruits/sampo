//! The bun version floor enforced by the preflight.
//!
//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use sampo_core::run_release;

#[test]
fn bun_older_than_1_2_fails_before_the_release_is_applied() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let bin = root.join("fake-bin");

    common::write_workspace(root);
    common::write_fake_bun(&bin, &root.join("bun-argv.log"), "1.1.30");
    let _guard = common::use_only(&bin);

    let err = run_release(root, false).expect_err("bun 1.1.30 must fail the release");
    let message = err.to_string();
    assert!(
        message.contains("1.1.30") && message.contains("1.2"),
        "must name the version found and the one required: {message}"
    );

    common::assert_workspace_untouched(root);
    assert!(
        !root.join("bun-argv.log").exists(),
        "regeneration must never run on an unsupported bun"
    );
}

#[test]
fn bun_1_2_canary_is_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let argv_log = root.join("bun-argv.log");
    let bin = root.join("fake-bin");

    common::write_workspace(root);
    common::write_fake_bun(&bin, &argv_log, "1.2.0-canary.20260101");
    let _guard = common::use_only(&bin);

    // The stub fails regeneration; passing the version floor is what this asserts.
    run_release(root, false).expect_err("the stub bun fails regeneration");
    assert!(
        argv_log.exists(),
        "a canary 1.2 must pass the version floor and reach regeneration"
    );
}
