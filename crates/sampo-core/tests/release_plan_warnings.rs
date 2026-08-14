//! Its own test binary: it overrides `PATH` process-wide.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn write_npm_workspace(root: &Path) {
    // Gives the gate something to check; no `workspaces` key, so the post-condition passes.
    fs::write(root.join("bun.lock"), "{}\n").unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/pkg-a")).unwrap();
    fs::write(
        root.join("packages/pkg-a/package.json"),
        r#"{"name":"pkg-a","version":"1.0.0-alpha.1"}"#,
    )
    .unwrap();

    fs::create_dir_all(root.join(".sampo/changesets")).unwrap();
    fs::write(
        root.join(".sampo/config.toml"),
        "[git]\nrelease_branches = [\"main\"]\n",
    )
    .unwrap();
    fs::write(
        root.join(".sampo/changesets/c.md"),
        "---\nnpm/pkg-a: patch\n---\n\nfix: a change\n",
    )
    .unwrap();
}

/// One `git remote` call per plan computation: that is what the log counts.
fn write_stubs(dir: &Path, log: &Path) {
    fs::create_dir_all(dir).unwrap();
    let git = dir.join("git");
    fs::write(
        &git,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"remote\" ]; then printf '%s\\n' \"$*\" >> '{}'; exit 1; fi\n\
             exit 1\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();

    let bun = dir.join("bun");
    fs::write(
        &bun,
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then echo 1.3.14; fi\n\
         exit 0\n",
    )
    .unwrap();
    fs::set_permissions(&bun, fs::Permissions::from_mode(0o755)).unwrap();
}

fn plans_computed(log: &Path) -> usize {
    fs::read_to_string(log)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// Re-runs this binary to capture the process stderr the warnings are written to.
#[test]
fn constraint_warning_is_printed_once() {
    if std::env::var("DUP_PLAN_PROBE_INNER").is_ok() {
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(exe)
        .args(["--exact", "warning_scenario", "--nocapture"])
        .env("DUP_PLAN_PROBE_INNER", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warnings = stderr.matches("does not satisfy it").count();
    assert!(out.status.success(), "inner test failed: {stderr}");
    assert_eq!(
        warnings, 1,
        "the constraint warning must reach the user once, got:\n{stderr}"
    );
}

#[test]
fn warning_scenario() {
    if std::env::var("DUP_PLAN_PROBE_INNER").is_err() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_npm_workspace(root);
    fs::write(
        root.join("packages/pkg-a/package.json"),
        r#"{"name":"pkg-a","version":"2.0.0-alpha.1"}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/pkg-b")).unwrap();
    fs::write(
        root.join("packages/pkg-b/package.json"),
        r#"{"name":"pkg-b","version":"1.0.0","dependencies":{"pkg-a":"^1.0.0"}}"#,
    )
    .unwrap();
    // Preserved rather than current, so a restore is ahead and the preflight must preview
    // the plan.
    fs::create_dir_all(root.join(".sampo/prerelease")).unwrap();
    fs::rename(
        root.join(".sampo/changesets/c.md"),
        root.join(".sampo/prerelease/p.md"),
    )
    .unwrap();

    let bin = root.join("bin");
    let log = root.join("git-remote-calls.log");
    write_stubs(&bin, &log);
    let _guard = common::use_only(&bin);

    sampo_core::run_stabilize_release(root, false).expect("stabilize must succeed");
    assert_eq!(
        plans_computed(&log),
        2,
        "expected a previewed plan and a real one; if the preview no longer runs, the paired \
         warning count proves nothing and this scenario needs rebuilding"
    );
}

#[test]
fn no_lockfile_means_no_preview_even_with_a_restore_pending() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_npm_workspace(root);
    fs::remove_file(root.join("bun.lock")).unwrap();
    fs::create_dir_all(root.join(".sampo/prerelease")).unwrap();
    fs::rename(
        root.join(".sampo/changesets/c.md"),
        root.join(".sampo/prerelease/p.md"),
    )
    .unwrap();

    let bin = root.join("bin");
    let log = root.join("git-remote-calls.log");
    write_stubs(&bin, &log);
    let _guard = common::use_only(&bin);

    sampo_core::run_stabilize_release(root, false).expect("stabilize must succeed");

    assert_eq!(
        plans_computed(&log),
        1,
        "nothing to regenerate, so the pending restore must not have forced a preview"
    );
}

#[test]
fn plan_is_computed_once_when_nothing_is_restored() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_npm_workspace(root);

    let bin = root.join("bin");
    let log = root.join("git-remote-calls.log");
    write_stubs(&bin, &log);
    let _guard = common::use_only(&bin);

    sampo_core::run_stabilize_release(root, false).expect("stabilize must succeed");

    assert_eq!(
        plans_computed(&log),
        1,
        "the plan must be computed once when no changeset is restored"
    );
}

#[test]
fn stabilize_dry_run_computes_the_plan_once() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_npm_workspace(root);

    let bin = root.join("bin");
    let log = root.join("git-remote-calls.log");
    write_stubs(&bin, &log);
    let _guard = common::use_only(&bin);

    sampo_core::run_stabilize_release(root, true).expect("stabilize dry run must succeed");

    assert_eq!(
        plans_computed(&log),
        1,
        "control: the dry run has no preflight, so it computes one plan"
    );
}
