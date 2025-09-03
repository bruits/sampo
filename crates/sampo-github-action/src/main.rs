use clap::{ArgAction, Parser, ValueEnum};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use thiserror::Error;

#[derive(Debug, Error)]
enum ActionError {
    #[error("No working directory provided and GITHUB_WORKSPACE is not set")]
    NoWorkingDirectory,
    #[error("Failed to execute sampo {operation}: {message}")]
    SampoCommandFailed { operation: String, message: String },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, ActionError>;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    Release,
    Publish,
    #[value(alias = "release-and-publish")]
    All,
}

/// Sampo GitHub Action entrypoint
///
/// This wrapper executes the `sampo` CLI inside the workspace, running
/// release and/or publish depending on inputs. It is designed to be invoked
/// by a composite GitHub Action which ensures Rust is available.
#[derive(Debug, Parser)]
#[command(name = "sampo-github-action", version, about = "Run Sampo in CI")]
struct Cli {
    /// Which operation to run (release, publish, or both)
    #[arg(short, long, value_enum, default_value = "all")]
    mode: Mode,

    /// Simulate actions without changing files or publishing artifacts
    #[arg(long, action = ArgAction::SetTrue)]
    dry_run: bool,

    /// Path to the repository root (defaults to GITHUB_WORKSPACE)
    #[arg(long)]
    working_directory: Option<PathBuf>,

    /// Optional crates.io token (exported to child processes)
    #[arg(long)]
    cargo_token: Option<String>,

    /// Extra args passed through to `sampo publish` (after `--`)
    /// Accepts a single string that will be split on whitespace.
    #[arg(long)]
    args: Option<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    let mut cli = Cli::parse();

    // Apply GitHub Actions environment variable overrides
    apply_environment_overrides(&mut cli);

    let workspace = determine_workspace(&cli)?;

    // Execute the requested operations
    let (released, published) = execute_operations(&cli, &workspace)?;

    // Emit outputs for the workflow
    emit_github_output("released", released)?;
    emit_github_output("published", published)?;

    Ok(())
}

/// Apply GitHub Actions input environment variables to CLI arguments
fn apply_environment_overrides(cli: &mut Cli) {
    // Override mode if INPUT_COMMAND is provided and mode is default
    if matches!(cli.mode, Mode::All)
        && let Ok(v) = std::env::var("INPUT_COMMAND")
    {
        cli.mode = match v.to_ascii_lowercase().as_str() {
            "release" => Mode::Release,
            "publish" => Mode::Publish,
            _ => Mode::All,
        };
    }

    // Override dry_run if INPUT_DRY_RUN is provided
    if !cli.dry_run
        && let Ok(v) = std::env::var("INPUT_DRY_RUN")
    {
        let val = v == "1" || v.eq_ignore_ascii_case("true");
        if val {
            cli.dry_run = true;
        }
    }

    // Override working_directory if INPUT_WORKING_DIRECTORY is provided
    if cli.working_directory.is_none()
        && let Some(v) = std::env::var_os("INPUT_WORKING_DIRECTORY")
    {
        cli.working_directory = Some(PathBuf::from(v));
    }

    // Override cargo_token if INPUT_CARGO_TOKEN is provided
    if cli.cargo_token.is_none()
        && let Ok(v) = std::env::var("INPUT_CARGO_TOKEN")
        && !v.is_empty()
    {
        cli.cargo_token = Some(v);
    }

    // Override args if INPUT_ARGS is provided
    if cli.args.is_none()
        && let Ok(v) = std::env::var("INPUT_ARGS")
        && !v.is_empty()
    {
        cli.args = Some(v);
    }
}

/// Apply GitHub Actions input environment variables to CLI arguments (testable version)
#[cfg(test)]
fn apply_environment_overrides_with_env<F1, F2>(cli: &mut Cli, env_var: F1, env_var_os: F2) 
where
    F1: Fn(&str) -> std::result::Result<String, std::env::VarError>,
    F2: Fn(&str) -> Option<std::ffi::OsString>,
{
    // Override mode if INPUT_COMMAND is provided and mode is default
    if matches!(cli.mode, Mode::All)
        && let Ok(v) = env_var("INPUT_COMMAND")
    {
        cli.mode = match v.to_ascii_lowercase().as_str() {
            "release" => Mode::Release,
            "publish" => Mode::Publish,
            _ => Mode::All,
        };
    }

    // Override dry_run if INPUT_DRY_RUN is provided
    if !cli.dry_run
        && let Ok(v) = env_var("INPUT_DRY_RUN")
    {
        let val = v == "1" || v.eq_ignore_ascii_case("true");
        if val {
            cli.dry_run = true;
        }
    }

    // Override working_directory if INPUT_WORKING_DIRECTORY is provided
    if cli.working_directory.is_none()
        && let Some(v) = env_var_os("INPUT_WORKING_DIRECTORY")
    {
        cli.working_directory = Some(PathBuf::from(v));
    }

    // Override cargo_token if INPUT_CARGO_TOKEN is provided
    if cli.cargo_token.is_none()
        && let Ok(v) = env_var("INPUT_CARGO_TOKEN")
        && !v.is_empty()
    {
        cli.cargo_token = Some(v);
    }

    // Override args if INPUT_ARGS is provided
    if cli.args.is_none()
        && let Ok(v) = env_var("INPUT_ARGS")
        && !v.is_empty()
    {
        cli.args = Some(v);
    }
}

/// Determine the workspace root directory
fn determine_workspace(cli: &Cli) -> Result<PathBuf> {
    cli.working_directory
        .clone()
        .or_else(|| std::env::var_os("GITHUB_WORKSPACE").map(PathBuf::from))
        .ok_or(ActionError::NoWorkingDirectory)
}

/// Determine the workspace root directory (testable version)
#[cfg(test)]
fn determine_workspace_with_env<F>(cli: &Cli, env_var: F) -> Result<PathBuf>
where
    F: Fn(&str) -> Option<std::ffi::OsString>,
{
    cli.working_directory
        .clone()
        .or_else(|| env_var("GITHUB_WORKSPACE").map(PathBuf::from))
        .ok_or(ActionError::NoWorkingDirectory)
}

/// Execute the requested operations and return (released, published) status
fn execute_operations(cli: &Cli, workspace: &Path) -> Result<(bool, bool)> {
    let mut released = false;
    let mut published = false;

    match cli.mode {
        Mode::Release => {
            run_sampo_release(workspace, cli.dry_run, cli.cargo_token.as_deref())?;
            released = true;
        }
        Mode::Publish => {
            run_sampo_publish(
                workspace,
                cli.dry_run,
                cli.args.as_deref(),
                cli.cargo_token.as_deref(),
            )?;
            published = true;
        }
        Mode::All => {
            run_sampo_release(workspace, cli.dry_run, cli.cargo_token.as_deref())?;
            released = true;

            run_sampo_publish(
                workspace,
                cli.dry_run,
                cli.args.as_deref(),
                cli.cargo_token.as_deref(),
            )?;
            published = true;
        }
    }

    Ok((released, published))
}

/// Execute sampo release command
fn run_sampo_release(workspace: &Path, dry_run: bool, cargo_token: Option<&str>) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("-p")
        .arg("sampo")
        .arg("--")
        .arg("release");

    if dry_run {
        cmd.arg("--dry-run");
    }

    if let Some(token) = cargo_token {
        cmd.env("CARGO_REGISTRY_TOKEN", token);
    }

    println!("Running: {}", display_command(&cmd));
    let status = cmd.status()?;

    if !status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "release".to_string(),
            message: format!("Command failed with status {}", status),
        });
    }

    Ok(())
}

/// Execute sampo publish command
fn run_sampo_publish(
    workspace: &Path,
    dry_run: bool,
    args: Option<&str>,
    cargo_token: Option<&str>,
) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("-p")
        .arg("sampo")
        .arg("--")
        .arg("publish");

    if dry_run {
        cmd.arg("--dry-run");
    }

    if let Some(extra_args) = args {
        let parts = split_args(extra_args);
        if !parts.is_empty() {
            cmd.arg("--");
            cmd.args(parts);
        }
    }

    if let Some(token) = cargo_token {
        cmd.env("CARGO_REGISTRY_TOKEN", token);
    }

    println!("Running: {}", display_command(&cmd));
    let status = cmd.status()?;

    if !status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "publish".to_string(),
            message: format!("Command failed with status {}", status),
        });
    }

    Ok(())
}

/// Format a command for display
fn display_command(cmd: &Command) -> String {
    let mut s = String::new();
    s.push_str(&cmd.get_program().to_string_lossy());
    for arg in cmd.get_args() {
        s.push(' ');
        s.push_str(&arg.to_string_lossy());
    }
    s
}

/// Split whitespace-separated arguments
/// Note: Does not handle quoted arguments
fn split_args(s: &str) -> Vec<String> {
    s.split_whitespace().map(|x| x.to_string()).collect()
}

/// Emit a GitHub Actions output
fn emit_github_output(key: &str, value: bool) -> Result<()> {
    let value_str = if value { "true" } else { "false" };

    if let Some(path) = std::env::var_os("GITHUB_OUTPUT") {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{}={}", key, value_str)?;
    }

    Ok(())
}

/// Emit a GitHub Actions output (testable version)
#[cfg(test)]
fn emit_github_output_with_env<F>(key: &str, value: bool, env_var_os: F) -> Result<()>
where
    F: Fn(&str) -> Option<std::ffi::OsString>,
{
    let value_str = if value { "true" } else { "false" };

    if let Some(path) = env_var_os("GITHUB_OUTPUT") {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{}={}", key, value_str)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_split_args() {
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args("--allow-dirty"), vec!["--allow-dirty"]);
        assert_eq!(
            split_args("--allow-dirty --no-verify"),
            vec!["--allow-dirty", "--no-verify"]
        );
        assert_eq!(split_args("  --flag   value  "), vec!["--flag", "value"]);
    }

    #[test]
    fn test_mode_parsing() {
        assert!(matches!(Mode::Release, Mode::Release));
        assert!(matches!(Mode::Publish, Mode::Publish));
        assert!(matches!(Mode::All, Mode::All));
    }

    #[test]
    fn test_determine_workspace_with_cli_override() {
        let temp_dir = TempDir::new().unwrap();
        let cli = Cli {
            mode: Mode::All,
            dry_run: false,
            working_directory: Some(temp_dir.path().to_path_buf()),
            cargo_token: None,
            args: None,
        };

        let workspace = determine_workspace(&cli).unwrap();
        assert_eq!(workspace, temp_dir.path().to_path_buf());
    }

    #[test]
    fn test_determine_workspace_with_github_workspace() {
        let temp_dir = TempDir::new().unwrap();
        
        // Test fallback behavior when CLI arg is None but GITHUB_WORKSPACE is available
        let mock_env = |key: &str| -> Option<std::ffi::OsString> {
            if key == "GITHUB_WORKSPACE" {
                Some(temp_dir.path().as_os_str().to_os_string())
            } else {
                None
            }
        };

        let cli = Cli {
            mode: Mode::All,
            dry_run: false,
            working_directory: None,
            cargo_token: None,
            args: None,
        };

        let workspace = determine_workspace_with_env(&cli, mock_env).unwrap();
        assert_eq!(workspace, temp_dir.path().to_path_buf());
    }

    #[test]
    fn test_determine_workspace_error() {
        // Test error path when neither CLI arg nor env var provide workspace
        let mock_env = |_key: &str| -> Option<std::ffi::OsString> { None };

        let cli = Cli {
            mode: Mode::All,
            dry_run: false,
            working_directory: None,
            cargo_token: None,
            args: None,
        };

        let result = determine_workspace_with_env(&cli, mock_env);
        assert!(matches!(result, Err(ActionError::NoWorkingDirectory)));
    }

    #[test]
    fn test_apply_environment_overrides() {
        use std::collections::HashMap;
        
        // Test that GitHub Actions INPUT_* variables override CLI defaults correctly
        let mut env_vars = HashMap::new();
        env_vars.insert("INPUT_COMMAND", "release");
        env_vars.insert("INPUT_DRY_RUN", "true");
        env_vars.insert("INPUT_CARGO_TOKEN", "test-token");
        env_vars.insert("INPUT_ARGS", "--allow-dirty");
        
        let mock_env_var = |key: &str| -> std::result::Result<String, std::env::VarError> {
            env_vars.get(key)
                .map(|v| v.to_string())
                .ok_or(std::env::VarError::NotPresent)
        };
        
        let mock_env_var_os = |key: &str| -> Option<std::ffi::OsString> {
            env_vars.get(key).map(std::ffi::OsString::from)
        };

        let mut cli = Cli {
            mode: Mode::All,
            dry_run: false,
            working_directory: None,
            cargo_token: None,
            args: None,
        };

        apply_environment_overrides_with_env(&mut cli, mock_env_var, mock_env_var_os);
        
        assert!(matches!(cli.mode, Mode::Release));
        assert!(cli.dry_run);
        assert_eq!(cli.cargo_token, Some("test-token".to_string()));
        assert_eq!(cli.args, Some("--allow-dirty".to_string()));
    }

    #[test]
    fn test_display_command() {
        let cmd = Command::new("cargo");
        assert_eq!(display_command(&cmd), "cargo");

        let mut cmd = Command::new("cargo");
        cmd.args(["run", "--package", "sampo"]);
        assert_eq!(display_command(&cmd), "cargo run --package sampo");
    }

    #[test]
    fn test_emit_github_output() {
        let temp_dir = TempDir::new().unwrap();
        let output_file = temp_dir.path().join("github_output");
        
        // Mock env function that points to our test file
        let mock_env = |key: &str| -> Option<std::ffi::OsString> {
            if key == "GITHUB_OUTPUT" {
                Some(output_file.as_os_str().to_os_string())
            } else {
                None
            }
        };
        
        // Test that output format matches GitHub Actions expectations
        emit_github_output_with_env("test_key", true, mock_env).unwrap();
        emit_github_output_with_env("another_key", false, mock_env).unwrap();

        let content = std::fs::read_to_string(&output_file).unwrap();
        assert!(content.contains("test_key=true"));
        assert!(content.contains("another_key=false"));
    }
}
