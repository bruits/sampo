//! The release path against a stand-in `bun`.
//!
//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use std::fs;

use sampo_core::run_release;

#[test]
fn failing_bun_stops_the_release_and_pins_the_argv() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let argv_log = root.join("bun-argv.log");
    let bin = root.join("fake-bin");

    common::write_workspace(root);
    common::write_fake_bun(&bin, &argv_log, "1.3.14");
    let _guard = common::use_only(&bin);

    let err = run_release(root, false).expect_err("a failing bun must fail the release");
    let message = err.to_string();
    assert!(
        message.contains("npm"),
        "must name the ecosystem: {message}"
    );
    assert!(
        message.contains("already written") && message.contains("discard"),
        "must say the release was partially applied and must be discarded: {message}"
    );
    assert_eq!(
        message.matches("Release error").count(),
        1,
        "the adapter's prefix must not be nested inside the wrapper's: {message}"
    );

    assert_eq!(
        fs::read_to_string(&argv_log).unwrap().trim(),
        "update --lockfile-only --no-save",
        "the exact argv handed to bun"
    );
}
