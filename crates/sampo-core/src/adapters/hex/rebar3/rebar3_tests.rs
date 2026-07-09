use super::*;
use std::collections::BTreeMap;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// A minimal, well-formed `.app.src` with the given version literal.
fn app_src(name: &str, vsn: &str) -> String {
    format!(
        "{{application, {name},\n [{{description, \"An app\"}},\n  {{vsn, \"{vsn}\"}},\n  {{applications, [kernel, stdlib]}}]}}.\n"
    )
}

#[test]
fn can_discover_detects_app_src() {
    let temp = tempfile::tempdir().unwrap();
    assert!(!can_discover(temp.path()));
    write_file(
        &temp.path().join("src").join("app.app.src"),
        &app_src("app", "1.0.0"),
    );
    assert!(can_discover(temp.path()));
    assert!(has_app_src(temp.path()));
}

#[test]
fn discover_single_application() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("src").join("my_app.app.src"),
        &app_src("my_app", "0.3.1"),
    );

    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);
    let pkg = &packages[0];
    assert_eq!(pkg.name, "my_app");
    assert_eq!(pkg.version, "0.3.1");
    assert_eq!(pkg.kind, PackageKind::Hex);
    // The package root is the application directory, not its `src/`.
    assert_eq!(pkg.path, temp.path());
}

#[test]
fn discover_umbrella_applications() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp
            .path()
            .join("apps")
            .join("a")
            .join("src")
            .join("a.app.src"),
        &app_src("a", "1.0.0"),
    );
    write_file(
        &temp
            .path()
            .join("apps")
            .join("b")
            .join("src")
            .join("b.app.src"),
        &app_src("b", "2.0.0"),
    );

    let mut packages = discover(temp.path()).unwrap();
    packages.sort_by(|l, r| l.name.cmp(&r.name));
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].name, "a");
    assert_eq!(packages[0].path, temp.path().join("apps").join("a"));
    assert_eq!(packages[1].name, "b");
}

#[test]
fn discover_links_internal_applications() {
    let temp = tempfile::tempdir().unwrap();
    // `b` depends on `a` (a sibling) plus OTP apps that are not workspace members.
    write_file(
        &temp
            .path()
            .join("apps")
            .join("a")
            .join("src")
            .join("a.app.src"),
        &app_src("a", "1.0.0"),
    );
    write_file(
        &temp
            .path()
            .join("apps")
            .join("b")
            .join("src")
            .join("b.app.src"),
        "{application, b,\n [{vsn, \"1.0.0\"},\n  {applications, [kernel, stdlib, a]}]}.\n",
    );

    let packages = discover(temp.path()).unwrap();
    let b = packages.iter().find(|p| p.name == "b").unwrap();
    let a_id = PackageInfo::dependency_identifier(PackageKind::Hex, "a");
    assert!(b.internal_deps.contains(&a_id));
    // `kernel`/`stdlib` are not workspace members, so they are not internal deps.
    assert_eq!(b.internal_deps.len(), 1);
}

#[test]
fn discover_skips_git_derived_versions() {
    let temp = tempfile::tempdir().unwrap();
    // Atom form.
    write_file(
        &temp
            .path()
            .join("apps")
            .join("atom")
            .join("src")
            .join("atom.app.src"),
        "{application, atom, [{vsn, git}, {applications, [kernel]}]}.\n",
    );
    // String form.
    write_file(
        &temp
            .path()
            .join("apps")
            .join("str")
            .join("src")
            .join("str.app.src"),
        "{application, str, [{vsn, \"git\"}, {applications, [kernel]}]}.\n",
    );
    // A managed sibling remains discoverable.
    write_file(
        &temp
            .path()
            .join("apps")
            .join("ok")
            .join("src")
            .join("ok.app.src"),
        &app_src("ok", "1.0.0"),
    );

    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "ok");
}

#[test]
fn discover_skips_template_versions() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("src").join("otp.app.src"),
        "{application, otp, [{vsn, \"%VSN%\"}]}.\n",
    );
    assert!(discover(temp.path()).unwrap().is_empty());
}

#[test]
fn discover_skips_app_src_script() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    write_file(&src.join("dyn.app.src"), &app_src("dyn", "1.0.0"));
    // A `.app.src.script` recomputes metadata at build time, overriding the static file.
    write_file(&src.join("dyn.app.src.script"), "{application, dyn, []}.\n");

    assert!(discover(temp.path()).unwrap().is_empty());
}

#[test]
fn discover_ignores_build_output_and_deps() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("src").join("app.app.src"),
        &app_src("app", "1.0.0"),
    );
    // Fetched dependencies and build output carry their own `.app.src` files.
    write_file(
        &temp
            .path()
            .join("_build")
            .join("default")
            .join("lib")
            .join("cowboy")
            .join("src")
            .join("cowboy.app.src"),
        &app_src("cowboy", "2.9.0"),
    );
    write_file(
        &temp
            .path()
            .join("deps")
            .join("jsx")
            .join("src")
            .join("jsx.app.src"),
        &app_src("jsx", "3.0.0"),
    );

    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "app");
}

#[test]
fn discover_skips_mix_owned_directories() {
    let temp = tempfile::tempdir().unwrap();
    // A Mix project can carry a generated `.app.src`; Mix owns it, so rebar3 discovery
    // must not claim it too.
    let dir = temp.path().join("lib_app");
    write_file(&dir.join("mix.exs"), "defmodule X do\nend\n");
    write_file(
        &dir.join("src").join("lib_app.app.src"),
        &app_src("lib_app", "1.0.0"),
    );

    assert!(discover(temp.path()).unwrap().is_empty());
}

#[test]
fn manifest_path_resolves_app_src() {
    let temp = tempfile::tempdir().unwrap();
    let expected = temp.path().join("src").join("thing.app.src");
    write_file(&expected, &app_src("thing", "1.0.0"));
    assert_eq!(manifest_path(temp.path()), expected);
}

#[test]
fn is_publishable_accepts_well_formed_app() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("src").join("app.app.src");
    write_file(&manifest, &app_src("app", "1.0.0"));
    assert!(is_publishable(&manifest).unwrap());
}

#[test]
fn is_publishable_rejects_missing_version() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("src").join("app.app.src");
    write_file(
        &manifest,
        "{application, app, [{applications, [kernel]}]}.\n",
    );
    assert!(is_publishable(&manifest).is_err());
}

#[test]
fn update_manifest_bumps_version_and_preserves_layout() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("src").join("app.app.src");
    let input = "%% keep this comment\n{application, app,\n [{description, \"An app\"}, %% trailing note\n  {vsn, \"1.0.0\"},\n  {applications, [kernel, stdlib]}]}.\n";
    write_file(&manifest, input);

    let (output, applied) =
        update_manifest_versions(&manifest, input, Some("2.1.0"), &BTreeMap::new()).unwrap();

    assert!(output.contains("{vsn, \"2.1.0\"}"));
    assert!(!output.contains("1.0.0"));
    // Comments and the rest of the terms survive the byte-splice untouched.
    assert!(output.contains("%% keep this comment"));
    assert!(output.contains("%% trailing note"));
    assert!(output.contains("{applications, [kernel, stdlib]}"));
    // Version bumps do not touch the (versionless) internal application deps.
    assert!(applied.is_empty());
}

#[test]
fn update_manifest_is_noop_when_version_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("src").join("app.app.src");
    let input = app_src("app", "1.0.0");
    write_file(&manifest, &input);

    let (output, applied) =
        update_manifest_versions(&manifest, &input, Some("1.0.0"), &BTreeMap::new()).unwrap();
    assert_eq!(output, input);
    assert!(applied.is_empty());
}

#[test]
fn find_dependency_constraint_reads_rebar_config() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("src").join("app.app.src");
    write_file(&manifest, &app_src("app", "1.0.0"));
    write_file(
        &temp.path().join("rebar.config"),
        "{erl_opts, [debug_info]}.\n{deps, [{cowboy, \"~> 2.9\"}, {local, {path, \"../local\"}}]}.\n",
    );

    assert_eq!(
        find_dependency_constraint_value(&manifest, "cowboy").unwrap(),
        Some("~> 2.9".to_string())
    );
    // A path dependency carries no registry requirement.
    assert_eq!(
        find_dependency_constraint_value(&manifest, "local").unwrap(),
        None
    );
    assert_eq!(
        find_dependency_constraint_value(&manifest, "absent").unwrap(),
        None
    );
}

#[test]
fn parses_quoted_atom_application_name() {
    // Erlang allows quoted atoms for names needing them; the quotes are not part of the
    // name Hex publishes.
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("src").join("weird.app.src"),
        "{application, 'weird-name', [{vsn, \"1.0.0\"}]}.\n",
    );
    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "weird-name");
}

#[test]
fn discover_skips_adjacent_string_vsn() {
    // Erlang concatenates adjacent string literals (`"1.2" ".3"`). That is not a single
    // `string` node, so we must skip it rather than silently mis-read half the version.
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("src").join("adj.app.src"),
        "{application, adj, [{vsn, \"1.2\" \".3\"}]}.\n",
    );
    assert!(discover(temp.path()).unwrap().is_empty());
}

#[test]
fn discover_skips_empty_vsn() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("src").join("blank.app.src"),
        "{application, blank, [{vsn, \"\"}]}.\n",
    );
    assert!(discover(temp.path()).unwrap().is_empty());
}

#[test]
fn publish_errors_when_manifest_not_under_src() {
    // The application root is the `.app.src`'s grandparent; a manifest without that
    // ancestry cannot be published (and never reaches the rebar3 command).
    let err = publish(Path::new("app.app.src"), false, &[]).unwrap_err();
    assert!(format!("{err}").contains("src"));
}

#[test]
fn publish_args_selects_public_repo_and_confirms_by_default() {
    assert_eq!(
        publish_args(false, &[]),
        ["hex", "publish", "--yes", "--repo", "hexpm"]
    );
}

#[test]
fn publish_args_dry_run_still_confirms() {
    assert_eq!(
        publish_args(true, &[]),
        ["hex", "publish", "--dry-run", "--yes", "--repo", "hexpm"]
    );
}

#[test]
fn publish_args_yields_to_a_user_supplied_repo() {
    let long = vec!["--repo".to_string(), "hexpm:acme".to_string()];
    assert_eq!(
        publish_args(false, &long),
        ["hex", "publish", "--yes", "--repo", "hexpm:acme"]
    );
    let short = vec!["-r".to_string(), "hexpm:acme".to_string()];
    assert_eq!(
        publish_args(false, &short),
        ["hex", "publish", "--yes", "-r", "hexpm:acme"]
    );
}

#[test]
fn publish_args_does_not_duplicate_user_confirmation_or_dry_run() {
    assert_eq!(
        publish_args(false, &["--yes".to_string()]),
        ["hex", "publish", "--repo", "hexpm", "--yes"]
    );
    assert_eq!(
        publish_args(false, &["-y".to_string()]),
        ["hex", "publish", "--repo", "hexpm", "-y"]
    );
    assert_eq!(
        publish_args(true, &["--dry-run".to_string()]),
        ["hex", "publish", "--yes", "--repo", "hexpm", "--dry-run"]
    );
    assert_eq!(
        publish_args(true, &["--yes".to_string()]),
        ["hex", "publish", "--dry-run", "--repo", "hexpm", "--yes"]
    );
}

#[test]
fn discover_errors_on_malformed_app_src() {
    // A malformed manifest hard-errors rather than being silently skipped, so a healthy
    // sibling in the same workspace is not returned either.
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp
            .path()
            .join("apps")
            .join("ok")
            .join("src")
            .join("ok.app.src"),
        &app_src("ok", "1.0.0"),
    );
    write_file(
        &temp
            .path()
            .join("apps")
            .join("bad")
            .join("src")
            .join("bad.app.src"),
        "{not_application, whatever, []}.\n",
    );

    match discover(temp.path()) {
        Err(WorkspaceError::InvalidManifest(msg)) => {
            assert!(msg.contains("bad.app.src"), "unexpected message: {msg}");
        }
        other => panic!("expected InvalidManifest, got {other:?}"),
    }
}
