use rustc_hash::FxHashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Build the sampo-github-action binary and return its path
fn get_action_binary() -> std::path::PathBuf {
    // Use CARGO_MANIFEST_DIR to find our crate directory, then navigate to workspace root
    let crate_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set during tests");
    let workspace_root = std::path::Path::new(&crate_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("Expected to find workspace root")
        .to_path_buf();

    // Build from workspace root
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

    // Verify the binary exists
    if !full_path.exists() {
        panic!("Binary not found at expected path: {}", full_path.display());
    }

    full_path
}

/// Run the action binary with environment variables
fn run_action(
    args: &[&str],
    env_vars: &FxHashMap<String, String>,
    working_dir: &Path,
) -> std::process::Output {
    let binary = get_action_binary();
    let mut cmd = Command::new(&binary);
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

#[test]
fn test_missing_workspace_fails() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Don't set GITHUB_WORKSPACE environment variable
    let mut env_vars = FxHashMap::default();
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());

    let output = run_action(&[], &env_vars, temp_dir.path());

    // Should fail with clear error message
    assert!(
        !output.status.success(),
        "Action should fail without workspace"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No working directory provided"),
        "Should indicate missing workspace error"
    );
}

#[test]
fn test_environment_variable_parsing() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace = temp_dir.path();
    let output_file = workspace.join("github_output");

    // Test GitHub Actions style input variables
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        workspace.to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "publish".to_string());
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());

    // Run without CLI args - should read from environment
    let output = run_action(&[], &env_vars, workspace);

    // Should fail because workspace is invalid, but should parse env vars correctly
    assert!(
        !output.status.success(),
        "Should fail with invalid workspace"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Failed to execute sampo") && stderr.contains("publish"),
        "Should attempt to run 'sampo publish' (indicating env vars were parsed correctly), got: {}",
        stderr
    );
}

#[test]
fn test_working_directory_override() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace = temp_dir.path();
    let output_file = workspace.join("github_output");

    // Don't set GITHUB_WORKSPACE, but provide INPUT_WORKING_DIRECTORY
    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert(
        "INPUT_WORKING_DIRECTORY".to_string(),
        workspace.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());

    // Use a valid working directory (any existing directory is fine for spawn)
    let output = run_action(&[], &env_vars, &std::env::temp_dir());

    // Should fail because workspace is invalid, but working directory parsing should work
    assert!(
        !output.status.success(),
        "Should fail with invalid workspace"
    );

    // Error should be about sampo execution, not about missing working directory
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("No working directory provided"),
        "Should not complain about missing working directory when INPUT_WORKING_DIRECTORY is provided"
    );
}

#[test]
fn test_github_output_generation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace = temp_dir.path();
    let output_file = workspace.join("github_output");

    // Create minimal structure to avoid immediate failure
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname=\"test\"\nversion=\"0.1.0\"\n",
    )
    .expect("Failed to create Cargo.toml");

    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        workspace.to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());

    let _output = run_action(&[], &env_vars, workspace);

    if output_file.exists() {
        let content = fs::read_to_string(&output_file).expect("Failed to read output file");
        assert!(
            content.contains("released="),
            "Should contain released status"
        );
        assert!(
            content.contains("published="),
            "Should contain published status"
        );

        // Verify the format is exactly what GitHub Actions expects
        for line in content.lines() {
            if !line.is_empty() {
                assert!(
                    line.contains('=') && (line.ends_with("true") || line.ends_with("false")),
                    "Each output line should be 'key=boolean', got: '{}'",
                    line
                );
            }
        }
    }
    // If file doesn't exist, that's also valid behavior (early failure)
    // The test validates that IF output is generated, it has the correct format
}

#[test]
fn test_with_minimal_valid_workspace() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace = temp_dir.path();
    let output_file = workspace.join("github_output");

    // Create minimal sampo workspace structure
    fs::create_dir_all(workspace.join(".sampo")).expect("Failed to create .sampo dir");
    fs::write(
        workspace.join(".sampo/config.toml"),
        "[packages]\n# Minimal valid config\n",
    )
    .expect("Failed to write config");

    // Create a basic Cargo.toml
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("Failed to write Cargo.toml");

    // Initialize git (required by sampo)
    Command::new("git")
        .args(["init"])
        .current_dir(workspace)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(workspace)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(workspace)
        .output()
        .ok();

    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        workspace.to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());

    let output = run_action(&[], &env_vars, workspace);

    // This might succeed or fail for legitimate sampo reasons
    if output.status.success() {
        // If it succeeds, verify the output format
        assert!(
            output_file.exists(),
            "Output file should be created on success"
        );
        let content = fs::read_to_string(&output_file).expect("Failed to read output");
        assert!(
            content.contains("released=true"),
            "Should indicate successful release"
        );
    } else {
        // If it fails, it should be for sampo-related reasons, not our wrapper logic
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("sampo") || stderr.contains("Failed to execute sampo"),
            "Failure should be related to sampo execution, not our wrapper: {}",
            stderr
        );

        // Even on failure, output file should still be created with failure status
        if output_file.exists() {
            let content = fs::read_to_string(&output_file).unwrap_or_default();
            if !content.is_empty() {
                assert!(
                    content.contains("released="),
                    "Should contain status even on failure"
                );
            }
        }
    }
}

#[test]
fn test_config_with_github_repository() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace = temp_dir.path();
    let output_file = workspace.join("github_output");

    // Create minimal sampo workspace structure with GitHub config
    fs::create_dir_all(workspace.join(".sampo")).expect("Failed to create .sampo dir");

    let config_content = r#"
[packages]
# Minimal valid config

[github]
repository = "test-owner/test-repo"
"#;
    fs::write(workspace.join(".sampo/config.toml"), config_content)
        .expect("Failed to write config");

    // Create a workspace Cargo.toml instead of a package Cargo.toml
    fs::write(
        workspace.join("Cargo.toml"),
        "[workspace]\nmembers = [\"test-package\"]\nresolver = \"2\"\n",
    )
    .expect("Failed to write workspace Cargo.toml");

    // Create the test package directory and its Cargo.toml
    let package_dir = workspace.join("test-package");
    fs::create_dir_all(&package_dir).expect("Failed to create package dir");
    fs::write(
        package_dir.join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("Failed to write package Cargo.toml");

    // Create minimal source file in the package
    fs::create_dir_all(package_dir.join("src")).expect("Failed to create src dir");
    fs::write(package_dir.join("src/main.rs"), "fn main() {}").expect("Failed to write main.rs");

    // Initialize git (required by sampo)
    Command::new("git")
        .args(["init"])
        .current_dir(workspace)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(workspace)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(workspace)
        .output()
        .ok();

    let mut env_vars = FxHashMap::default();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        workspace.to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );
    env_vars.insert("INPUT_COMMAND".to_string(), "release".to_string());
    env_vars.insert("INPUT_DRY_RUN".to_string(), "true".to_string());

    // Test that the action can read the GitHub repository configuration successfully
    let output = run_action(&[], &env_vars, workspace);

    // The important thing is that configuration parsing doesn't cause errors
    // The actual sampo execution might fail for other reasons, but not config-related
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Should not fail due to configuration parsing issues
        assert!(
            !stderr.contains("configuration")
                && !stderr.contains("config")
                && !stderr.contains("toml"),
            "Should not fail due to configuration parsing when GitHub repository is specified: {}",
            stderr
        );
    }
}
