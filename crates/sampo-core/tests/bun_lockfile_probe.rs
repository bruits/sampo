//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use sampo_core::run_release;
use std::path::Path;

fn assert_refused_before_writing(root: &Path, version_body: &str) -> String {
    let argv_log = root.join("bun-argv.log");
    let bin = root.join("fake-bin");

    common::write_workspace(root);
    common::write_bun_with_version_body(&bin, &argv_log, version_body);
    let _guard = common::use_only(&bin);

    let err = run_release(root, false).expect_err("an unusable bun must fail the release");

    common::assert_workspace_untouched(root);
    assert!(
        !argv_log.exists(),
        "regeneration must never run on an unusable bun"
    );
    err.to_string()
}

#[test]
fn a_failing_version_probe_stops_the_release() {
    let temp = tempfile::tempdir().unwrap();
    let message =
        assert_refused_before_writing(temp.path(), "echo 'dyld: symbol not found' >&2; exit 1");
    assert!(
        message.contains("symbol not found"),
        "must quote what bun printed: {message}"
    );
}

#[test]
fn a_version_the_probe_cannot_read_stops_the_release() {
    let temp = tempfile::tempdir().unwrap();
    // On stderr, so stdout carries no version.
    let message = assert_refused_before_writing(temp.path(), "echo 1.1.30 >&2; exit 0");
    assert!(
        message.contains("1.2"),
        "must name the version required: {message}"
    );
}
