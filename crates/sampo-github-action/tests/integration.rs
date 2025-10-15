use rustc_hash::FxHashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use tempfile::TempDir;

/// Build the sampo-github-action binary and return its path
fn get_action_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();

    BINARY
        .get_or_init(|| {
            let crate_dir = std::env::var("CARGO_MANIFEST_DIR")
                .expect("CARGO_MANIFEST_DIR should be set during tests");
            let workspace_root = std::path::Path::new(&crate_dir)
                .parent()
                .and_then(|p| p.parent())
                .expect("Expected to find workspace root")
                .to_path_buf();

            let output = Command::new("cargo")
                .args(["build", "--bin", "sampo-github-action"])
                .current_dir(&workspace_root)
                .output()
                .expect("Failed to build sampo-github-action");

            if !output.status.success() {
                panic!(
                    "Failed to build binary: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            let full_path = workspace_root.join("target/debug/sampo-github-action");
            if !full_path.exists() {
                panic!("Binary not found at expected path: {}", full_path.display());
            }

            full_path
        })
        .as_path()
}

/// Run the action binary with environment variables
fn run_action(args: &[&str], env_vars: &FxHashMap<String, String>, working_dir: &Path) -> Output {
    let binary = get_action_binary();
    let mut cmd = Command::new(binary);
    cmd.args(args).current_dir(working_dir);

    // Clear the environment and only set our specified variables
    cmd.env_clear();

    // Set minimal required environment variables for Rust/cargo to work
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        cmd.env("CARGO_HOME", cargo_home);
    }
    if let Ok(rustup_home) = std::env::var("RUSTUP_HOME") {
        cmd.env("RUSTUP_HOME", rustup_home);
    }

    // Add our test-specific environment variables
    cmd.envs(env_vars);

    cmd.output().expect("Failed to execute action binary")
}

struct TestWorkspace {
    temp: TempDir,
}

impl TestWorkspace {
    fn new() -> Self {
        Self {
            temp: TempDir::new().expect("Failed to create temp dir"),
        }
    }

    fn path(&self) -> &Path {
        self.temp.path()
    }

    fn file_path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.path().join(relative)
    }

    fn write_file(&self, relative: impl AsRef<Path>, contents: &str) {
        let path = self.file_path(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create parent directories");
        }
        fs::write(path, contents).expect("failed to write file");
    }

    fn read_file(&self, relative: impl AsRef<Path>) -> String {
        fs::read_to_string(self.file_path(relative)).expect("failed to read file")
    }

    fn exists(&self, relative: impl AsRef<Path>) -> bool {
        self.file_path(relative).exists()
    }
}

/// Builder for creating test workspaces with various configurations
struct WorkspaceBuilder {
    crate_name: String,
    crate_version: String,
    with_changesets: bool,
    publish_enabled: bool,
    with_git: bool,
}

impl WorkspaceBuilder {
    fn new() -> Self {
        Self {
            crate_name: "foo".to_string(),
            crate_version: "0.1.0".to_string(),
            with_changesets: false,
            publish_enabled: true,
            with_git: false,
        }
    }

    #[allow(dead_code)]
    fn crate_name(mut self, name: &str) -> Self {
        self.crate_name = name.to_string();
        self
    }

    #[allow(dead_code)]
    fn crate_version(mut self, version: &str) -> Self {
        self.crate_version = version.to_string();
        self
    }

    fn with_changesets(mut self) -> Self {
        self.with_changesets = true;
        self
    }

    fn publish_disabled(mut self) -> Self {
        self.publish_enabled = false;
        self
    }

    fn with_git(mut self) -> Self {
        self.with_git = true;
        self
    }

    fn build(self, ws: &TestWorkspace) {
        // Create workspace Cargo.toml
        ws.write_file(
            "Cargo.toml",
            &format!(
                r#"[workspace]
resolver = "2"
members = ["crates/{}"]
"#,
                self.crate_name
            ),
        );

        // Create crate Cargo.toml
        let publish_line = if self.publish_enabled {
            ""
        } else {
            "publish = false\n"
        };

        ws.write_file(
            format!("crates/{}/Cargo.toml", self.crate_name),
            &format!(
                r#"[package]
name = "{}"
version = "{}"
edition = "2021"
{}
[lib]
path = "src/lib.rs"
"#,
                self.crate_name, self.crate_version, publish_line
            ),
        );

        // Create source files
        ws.write_file(
            format!("crates/{}/src/lib.rs", self.crate_name),
            &format!("pub fn {}() {{}}\n", self.crate_name.replace('-', "_")),
        );

        ws.write_file(
            format!("crates/{}/CHANGELOG.md", self.crate_name),
            "# Changelog\n\n## Unreleased\n\n",
        );

        // Add changesets if requested
        if self.with_changesets {
            ws.write_file(
                ".sampo/changesets/add-feature.md",
                &format!("---\n{}: minor\n---\n\n- add feature\n", self.crate_name),
            );
        }

        // Initialize git if requested
        if self.with_git {
            init_git_repo(ws.path());

            let add_status = Command::new("git")
                .args(["add", "."])
                .current_dir(ws.path())
                .status()
                .expect("failed to run git add");
            assert!(add_status.success(), "git add failed: {:?}", add_status);

            let commit_status = Command::new("git")
                .args(["commit", "-m", "Initial commit"])
                .current_dir(ws.path())
                .status()
                .expect("failed to run git commit");
            assert!(
                commit_status.success(),
                "git commit failed: {:?}",
                commit_status
            );
        }
    }
}

fn init_git_repo(path: &Path) {
    let status = Command::new("git")
        .arg("init")
        .current_dir(path)
        .status()
        .expect("failed to run git init");
    assert!(status.success(), "git init failed: {:?}", status);

    let email_status = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .status()
        .expect("failed to configure git user email");
    assert!(
        email_status.success(),
        "git config user.email failed: {:?}",
        email_status
    );

    let name_status = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .status()
        .expect("failed to configure git user name");
    assert!(
        name_status.success(),
        "git config user.name failed: {:?}",
        name_status
    );
}

fn parse_outputs(path: &Path) -> FxHashMap<String, String> {
    let mut outputs = FxHashMap::default();
    if !path.exists() {
        return outputs;
    }
    let content = fs::read_to_string(path).expect("failed to read outputs file");
    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            outputs.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    outputs
}

fn setup_release_workspace(ws: &TestWorkspace) {
    WorkspaceBuilder::new().with_changesets().build(ws);
}

fn setup_publish_workspace(ws: &TestWorkspace) {
    WorkspaceBuilder::new()
        .publish_disabled()
        .with_git()
        .build(ws);
}

fn write_git_config(ws: &TestWorkspace, contents: &str) {
    let path = ws.file_path(".sampo/config.toml");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create .sampo directory");
    }
    fs::write(path, contents).expect("failed to write config");
}

#[test]
fn test_missing_workspace_fails() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Don't set GITHUB_WORKSPACE environment variable
    let mut env_vars = FxHashMap::default();
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());

    let output = run_action(&[], &env_vars, temp_dir.path());

    // Should fail with specific exit code
    assert_eq!(
        output.status.code(),
        Some(1),
        "Action should fail with exit code 1"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error: NoWorkingDirectory")
            || stderr.contains("No working directory provided"),
        "Should indicate missing workspace error, got: {}",
        stderr
    );
}

#[test]
fn test_default_command_is_auto() {
    let ws = TestWorkspace::new();
    WorkspaceBuilder::new().with_git().build(&ws);

    let output_file = ws.file_path("github_output");
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        ws.path().to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    // Don't set INPUT_COMMAND - should default to "auto"
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());
    env_vars.insert("SAMPO_RELEASE_BRANCH".to_string(), "main".to_string());

    let output = run_action(&[], &env_vars, ws.path());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "Default mode should succeed in dry-run"
    );

    let outputs = parse_outputs(&output_file);
    assert_eq!(outputs.get("released").map(String::as_str), Some("false"));
    assert_eq!(outputs.get("published").map(String::as_str), Some("false"));

    assert!(
        stdout.contains("Publish plan:"),
        "Expected auto mode to trigger publish path, got stdout: {}",
        stdout
    );
}

#[test]
fn test_release_updates_versions_and_outputs() {
    let ws = TestWorkspace::new();
    setup_release_workspace(&ws);

    let output_file = ws.file_path("github_output");
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        ws.path().to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());
    env_vars.insert("SAMPO_RELEASE_BRANCH".to_string(), "main".to_string());

    let output = run_action(&[], &env_vars, ws.path());
    assert!(output.status.success(), "release command should succeed");

    let outputs = parse_outputs(&output_file);
    assert!(outputs.contains_key("released"));
    assert!(outputs.contains_key("published"));

    let manifest = ws.read_file("crates/foo/Cargo.toml");
    assert!(manifest.contains("version = \"0.2.0\""));
    assert!(manifest.contains("name = \"foo\""));

    let changelog = ws.read_file("crates/foo/CHANGELOG.md");
    assert!(changelog.contains("## 0.2.0"));
    assert!(!ws.exists(".sampo/changesets/add-feature.md"));
}

#[test]
fn test_publish_dry_run_reports_no_publishable_crates() {
    let ws = TestWorkspace::new();
    setup_publish_workspace(&ws);

    let output_file = ws.file_path("github_output");
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        ws.path().to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "publish".to_string());
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());
    env_vars.insert("SAMPO_RELEASE_BRANCH".to_string(), "main".to_string());

    let output = run_action(&[], &env_vars, ws.path());
    assert!(output.status.success(), "publish command should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No publishable packages were found in the workspace."),
        "Expected dry-run publish to report missing crates"
    );

    let outputs = parse_outputs(&output_file);
    assert_eq!(outputs.get("released").map(String::as_str), Some("false"));
    assert_eq!(outputs.get("published").map(String::as_str), Some("false"));
}

#[test]
fn test_auto_mode_detects_changesets() {
    let ws = TestWorkspace::new();
    WorkspaceBuilder::new().with_changesets().build(&ws);

    let output_file = ws.file_path("github_output");
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        ws.path().to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "auto".to_string());
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());
    env_vars.insert("SAMPO_RELEASE_BRANCH".to_string(), "main".to_string());

    let output = run_action(&[], &env_vars, ws.path());

    let stdout = String::from_utf8_lossy(&output.stdout);

    // In dry-run mode, auto should detect changesets and show planned releases
    assert!(
        stdout.contains("Detected") && stdout.contains("pending release package"),
        "Expected auto mode to announce pending releases, got stdout: {}",
        stdout
    );

    assert!(
        !output.status.success(),
        "Auto mode with changesets should fail without GitHub setup to avoid false positives"
    );
}

#[test]
fn test_action_rejects_non_release_branch() {
    let ws = TestWorkspace::new();
    WorkspaceBuilder::new().with_git().build(&ws);
    write_git_config(&ws, "[git]\nrelease_branches = [\"main\"]\n");

    let output_file = ws.file_path("github_output");
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        ws.path().to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());
    env_vars.insert("SAMPO_RELEASE_BRANCH".to_string(), "feature".to_string());

    let output = run_action(&[], &env_vars, ws.path());
    assert!(
        !output.status.success(),
        "action should fail on disallowed branch"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not listed in git.release_branches")
            || stderr.contains("not configured for releases"),
        "expected branch guard error, got stderr: {}",
        stderr
    );
}

#[test]
fn test_action_accepts_configured_release_branch() {
    let ws = TestWorkspace::new();
    setup_release_workspace(&ws);
    write_git_config(&ws, "[git]\nrelease_branches = [\"main\", \"3.x\"]\n");

    let output_file = ws.file_path("github_output");
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        ws.path().to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());
    env_vars.insert("SAMPO_RELEASE_BRANCH".to_string(), "3.x".to_string());

    let output = run_action(&[], &env_vars, ws.path());
    assert!(
        output.status.success(),
        "action should allow configured branch"
    );

    let outputs = parse_outputs(&output_file);
    assert_eq!(outputs.get("released").map(String::as_str), Some("true"));
    assert_eq!(outputs.get("published").map(String::as_str), Some("false"));

    let manifest = ws.read_file("crates/foo/Cargo.toml");
    assert!(
        manifest.contains("version = \"0.2.0\"") || manifest.contains("version=\"0.2.0\""),
        "release should bump version, manifest was:\n{}",
        manifest
    );
    assert!(
        !ws.exists(".sampo/changesets/add-feature.md"),
        "release should consume the pending changeset"
    );
}

#[test]
fn test_action_accepts_configured_pre_release_branch() {
    let ws = TestWorkspace::new();
    setup_release_workspace(&ws);
    write_git_config(&ws, "[git]\nrelease_branches = [\"main\", \"next\"]\n");

    let output_file = ws.file_path("github_output");
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        ws.path().to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());
    env_vars.insert("SAMPO_RELEASE_BRANCH".to_string(), "next".to_string());

    let output = run_action(&[], &env_vars, ws.path());
    assert!(
        output.status.success(),
        "action should allow pre-release branch"
    );

    let outputs = parse_outputs(&output_file);
    assert_eq!(outputs.get("released").map(String::as_str), Some("true"));
    assert_eq!(outputs.get("published").map(String::as_str), Some("false"));
}

#[test]
fn test_auto_mode_without_changesets_attempts_publish() {
    let ws = TestWorkspace::new();
    WorkspaceBuilder::new().with_git().build(&ws);

    let output_file = ws.file_path("github_output");
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        ws.path().to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "auto".to_string());
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());
    env_vars.insert("SAMPO_RELEASE_BRANCH".to_string(), "main".to_string());

    let output = run_action(&[], &env_vars, ws.path());

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should succeed in dry-run mode and show expected behavior
    assert!(
        output.status.success(),
        "Auto mode should succeed in dry-run without changesets"
    );

    /*
    Should follow the "no changesets -> try publish" logic path
    */
    assert!(
        stdout.contains("Publish plan:"),
        "Expected auto mode to attempt publish path, got stdout: {}",
        stdout
    );

    let outputs = parse_outputs(&output_file);
    assert_eq!(outputs.get("released").map(String::as_str), Some("false"));
    assert_eq!(outputs.get("published").map(String::as_str), Some("false"));
}
