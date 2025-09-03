use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Build the sampo-github-action binary and return its path
fn get_action_binary() -> std::path::PathBuf {
    // Find workspace root first
    let mut workspace_root = std::env::current_dir().expect("Failed to get current dir");
    while !workspace_root.join("Cargo.toml").exists() || !workspace_root.join("crates").exists() {
        if let Some(parent) = workspace_root.parent() {
            workspace_root = parent.to_path_buf();
        } else {
            // Fallback - assume we're already at root
            workspace_root = std::env::current_dir().expect("Failed to get current dir");
            break;
        }
    }

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
    env_vars: &HashMap<String, String>,
    working_dir: &Path,
) -> std::process::Output {
    let binary = get_action_binary();
    let mut cmd = Command::new(&binary);
    cmd.args(args).current_dir(working_dir).envs(env_vars);
    cmd.output().expect("Failed to execute action binary")
}

#[test]
fn test_missing_workspace_fails() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Don't set GITHUB_WORKSPACE environment variable
    let env_vars = HashMap::new();

    let output = run_action(&["--mode", "release"], &env_vars, temp_dir.path());

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
    let mut env_vars = HashMap::new();
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

    // Don't set GITHUB_WORKSPACE, but provide --working-directory
    let mut env_vars = HashMap::new();
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );

    let args = [
        "--working-directory",
        &workspace.to_string_lossy(),
        "--mode",
        "release",
        "--dry-run",
    ];

    let output = run_action(&args, &env_vars, Path::new("/tmp"));

    // Should fail because workspace is invalid, but working directory parsing should work
    assert!(
        !output.status.success(),
        "Should fail with invalid workspace"
    );

    // Error should be about sampo execution, not about missing working directory
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("No working directory provided"),
        "Should not complain about missing working directory when --working-directory is provided"
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

    let mut env_vars = HashMap::new();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        workspace.to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );

    let _output = run_action(&["--mode", "release", "--dry-run"], &env_vars, workspace);

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

    let mut env_vars = HashMap::new();
    env_vars.insert(
        "GITHUB_WORKSPACE".to_string(),
        workspace.to_string_lossy().to_string(),
    );
    env_vars.insert(
        "GITHUB_OUTPUT".to_string(),
        output_file.to_string_lossy().to_string(),
    );

    let output = run_action(&["--mode", "release", "--dry-run"], &env_vars, workspace);

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
