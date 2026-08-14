//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use sampo_core::run_release;

fn write_silently_successful_bun(dir: &Path, argv_log: &Path) {
    fs::create_dir_all(dir).unwrap();
    let stub = dir.join("bun");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then echo 1.3.14; exit 0; fi\n\
             printf '%s\\n' \"$*\" >> '{}'\n\
             exit 0\n",
            argv_log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_lockfile_left_stale_fails_the_release() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let argv_log = root.join("bun-argv.log");
    let bin = root.join("fake-bin");

    common::write_workspace(root);
    let stale_lock = "{\"workspaces\":{\"packages/pkg-a\":{\"version\":\"1.0.0\"}}}\n";
    fs::write(root.join("bun.lock"), stale_lock).unwrap();

    write_silently_successful_bun(&bin, &argv_log);
    let _guard = common::use_only(&bin);

    let err = run_release(root, false).expect_err("a stale lockfile must fail the release");
    let message = err.to_string();
    assert!(
        message.contains("pkg-a") && message.contains("1.0.0") && message.contains("1.1.0"),
        "must name the package and both versions: {message}"
    );

    // Proves the post-condition fired, rather than a path skipping regeneration.
    assert_eq!(
        fs::read_to_string(&argv_log).unwrap().trim(),
        "update --lockfile-only --no-save"
    );
}

#[test]
fn a_refreshed_lockfile_passes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let bin = root.join("fake-bin");

    common::write_workspace(root);
    fs::write(root.join("bun.lock"), "{\"workspaces\":{}}\n").unwrap();

    // The trailing commas below are what bun emits.
    fs::create_dir_all(&bin).unwrap();
    let stub = bin.join("bun");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then echo 1.3.14; exit 0; fi\n\
             printf '%s' '{{\"workspaces\":{{\"packages/pkg-a\":{{\"version\":\"1.1.0\"}},}},}}' > '{}'\n\
             exit 0\n",
            root.join("bun.lock").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    let _guard = common::use_only(&bin);

    run_release(root, false).expect("a refreshed lockfile releases cleanly");
}
