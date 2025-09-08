mod git;
mod github;
mod sampo;

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
    /// Detect changesets and open/update a Release PR (no tags)
    #[value(alias = "prepare-pr", alias = "release-pr", alias = "open-pr")]
    PreparePr,
    /// After merge to main: create tags for current versions, push, and publish
    #[value(alias = "post-merge-publish", alias = "finalize")]
    PostMergePublish,
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

    /// Base branch for the Release PR (default: current ref name or 'main')
    #[arg(long)]
    base_branch: Option<String>,

    /// Branch name to use for the Release PR (default: 'release/sampo')
    #[arg(long)]
    pr_branch: Option<String>,

    /// Title to use for the Release PR (default: 'Release')
    #[arg(long)]
    pr_title: Option<String>,

    /// Create GitHub releases for newly created tags during publish
    #[arg(long, action = ArgAction::SetTrue)]
    create_github_release: bool,

    /// Optional GitHub token to create/update PRs (defaults to GITHUB_TOKEN)
    #[arg(long)]
    github_token: Option<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
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
            "release-and-publish" | "all" => Mode::All,
            "prepare-pr" | "release-pr" | "open-pr" => Mode::PreparePr,
            "post-merge-publish" | "finalize" => Mode::PostMergePublish,
            _ => cli.mode,
        };
    }

    // Override dry_run if INPUT_DRY_RUN is provided
    if !cli.dry_run
        && let Ok(v) = std::env::var("INPUT_DRY_RUN")
    {
        cli.dry_run = v.eq_ignore_ascii_case("true");
    }

    // Override working_directory if INPUT_WORKING_DIRECTORY is provided
    if cli.working_directory.is_none()
        && let Ok(v) = std::env::var("INPUT_WORKING_DIRECTORY")
        && !v.is_empty()
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

    // Override base_branch/pr_branch/pr_title if provided
    if cli.base_branch.is_none()
        && let Ok(v) = std::env::var("INPUT_BASE_BRANCH")
        && !v.is_empty()
    {
        cli.base_branch = Some(v);
    }
    if cli.pr_branch.is_none()
        && let Ok(v) = std::env::var("INPUT_PR_BRANCH")
        && !v.is_empty()
    {
        cli.pr_branch = Some(v);
    }
    if cli.pr_title.is_none()
        && let Ok(v) = std::env::var("INPUT_PR_TITLE")
        && !v.is_empty()
    {
        cli.pr_title = Some(v);
    }

    if !cli.create_github_release
        && let Ok(v) = std::env::var("INPUT_CREATE_GITHUB_RELEASE")
    {
        cli.create_github_release = v.eq_ignore_ascii_case("true");
    }

    // Optional GitHub token override
    if cli.github_token.is_none()
        && let Ok(v) = std::env::var("INPUT_GITHUB_TOKEN")
        && !v.is_empty()
    {
        cli.github_token = Some(v);
    }
}

/// Determine the workspace root directory
fn determine_workspace(cli: &Cli) -> Result<PathBuf> {
    cli.working_directory
        .clone()
        .or_else(|| std::env::var("GITHUB_WORKSPACE").ok().map(PathBuf::from))
        .ok_or(ActionError::NoWorkingDirectory)
}

/// Execute the requested operations and return (released, published) status
fn execute_operations(cli: &Cli, workspace: &Path) -> Result<(bool, bool)> {
    let mut released = false;
    let mut published = false;

    match cli.mode {
        Mode::Release => {
            sampo::run_release(workspace, cli.dry_run, cli.cargo_token.as_deref())?;
            released = true;
        }
        Mode::Publish => {
            sampo::run_publish(
                workspace,
                cli.dry_run,
                cli.args.as_deref(),
                cli.cargo_token.as_deref(),
            )?;
            published = true;
        }
        Mode::All => {
            sampo::run_release(workspace, cli.dry_run, cli.cargo_token.as_deref())?;
            released = true;

            sampo::run_publish(
                workspace,
                cli.dry_run,
                cli.args.as_deref(),
                cli.cargo_token.as_deref(),
            )?;
            published = true;
        }
        Mode::PreparePr => {
            prepare_release_pr(workspace, cli)?;
        }
        Mode::PostMergePublish => {
            post_merge_publish(
                workspace,
                cli.dry_run,
                cli.args.as_deref(),
                cli.cargo_token.as_deref(),
                cli.create_github_release,
            )?;
            published = true;
        }
    }

    Ok((released, published))
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

fn prepare_release_pr(workspace: &Path, cli: &Cli) -> Result<()> {
    // Check if there are changes to release
    let plan = sampo::capture_release_plan(workspace)?;
    if !plan.has_changes {
        println!("No changesets detected. Skipping PR preparation.");
        return Ok(());
    }

    // Configuration
    let base_branch = cli
        .base_branch
        .clone()
        .or_else(|| std::env::var("GITHUB_REF_NAME").ok())
        .unwrap_or_else(|| "main".into());
    let pr_branch = cli
        .pr_branch
        .clone()
        .unwrap_or_else(|| "release/sampo".into());
    let pr_title = cli.pr_title.clone().unwrap_or_else(|| "Release".into());

    // Build PR body BEFORE running release (release will consume changesets)
    let pr_body = {
        // Load configuration for dependency explanations
        let config = sampo_core::Config::load(workspace).unwrap_or_default();
        let body = sampo::build_release_pr_body_from_stdout(workspace, &plan.description, &config)?;
        if body.trim().is_empty() {
            // Fallback to the raw plan text when no applicable packages
            format!(
                "This PR was generated by [Sampo GitHub Action](https://github.com/bruits/sampo/blob/main/crates/sampo-github-action/README.md). When you're ready to do a release, you can merge this and the packages will be published automatically. Merging this PR will automatically publish the packages. Not ready yet? Just keep adding changesets to the default branch, and this PR will stay up to date.\n\n----\n\n{}",
                plan.description
            )
        } else {
            body
        }
    };

    // Setup git
    git::setup_bot_user(workspace)?;
    git::git(&["fetch", "origin", "--prune"], Some(workspace))?;

    // Create release branch
    git::git(
        &[
            "checkout",
            "-B",
            &pr_branch,
            &format!("origin/{}", base_branch),
        ],
        Some(workspace),
    )?;

    // Apply release (no tags)
    sampo::run_release(workspace, false, cli.cargo_token.as_deref())?;

    // Check for changes and commit
    if !git::has_changes(workspace)? {
        println!("No file changes after release. Skipping commit/PR.");
        return Ok(());
    }

    git::git(&["add", "-A"], Some(workspace))?;
    git::git(
        &[
            "commit",
            "-m",
            "chore(release): bump versions and changelogs",
        ],
        Some(workspace),
    )?;

    // Force push to release branch (overwrites any existing branch)
    git::git(
        &["push", "origin", &format!("HEAD:{}", pr_branch), "--force"],
        Some(workspace),
    )?;

    // Create PR
    let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    if repo.is_empty() || token.is_empty() {
        eprintln!("Warning: GITHUB_REPOSITORY or GITHUB_TOKEN not set. Cannot create PR.");
        return Ok(());
    }

    github::ensure_pull_request(&repo, &token, &pr_branch, &base_branch, &pr_title, &pr_body)?;

    Ok(())
}

fn post_merge_publish(
    workspace: &Path,
    dry_run: bool,
    args: Option<&str>,
    cargo_token: Option<&str>,
    create_github_release: bool,
) -> Result<()> {
    // Setup git identity for tag creation
    git::setup_bot_user(workspace)?;

    // Capture tags before publishing
    let before_tags = git::list_tags(workspace)?;

    // Publish
    sampo::run_publish(workspace, dry_run, args, cargo_token)?;

    // Compute new tags created by publish
    let after_tags = git::list_tags(workspace)?;
    let new_tags: Vec<String> = after_tags
        .into_iter()
        .filter(|tag| !before_tags.contains(tag))
        .collect();

    // Push tags
    if !new_tags.is_empty() {
        println!("Pushing {} new tags", new_tags.len());
        for tag in &new_tags {
            git::git(&["push", "origin", tag], Some(workspace))?;
        }
    }

    // Optionally create GitHub releases for new tags
    if create_github_release && !new_tags.is_empty() {
        let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
        let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();

        if !repo.is_empty() && !token.is_empty() {
            for tag in &new_tags {
                println!("Creating GitHub release for {}", tag);
                // Simplified release creation
                create_github_release_for_tag(&repo, &token, tag)?;
            }
        }
    }

    Ok(())
}

fn create_github_release_for_tag(repo: &str, token: &str, tag: &str) -> Result<()> {
    let api = format!("https://api.github.com/repos/{}/releases", repo);
    let name = tag.to_string();
    let body = format!("Automated release for tag {}", tag);
    let payload = format!(
        r#"{{"tag_name":"{}","name":"{}","body":"{}","draft":false,"prerelease":false}}"#,
        tag, name, body
    );

    let output = Command::new("curl")
        .args([
            "-sS",
            "-X",
            "POST",
            "-H",
            &format!("Authorization: Bearer {}", token),
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "Content-Type: application/json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            &api,
            "-d",
            &payload,
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("Warning: Failed to create GitHub release for {}", tag);
    } else {
        println!("Created GitHub release for {}", tag);
    }

    Ok(())
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            mode: Mode::All,
            dry_run: false,
            working_directory: None,
            cargo_token: None,
            args: None,
            base_branch: None,
            pr_branch: None,
            pr_title: None,
            create_github_release: false,
            github_token: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_parsing() {
        assert!(matches!(Mode::Release, Mode::Release));
        assert!(matches!(Mode::Publish, Mode::Publish));
        assert!(matches!(Mode::All, Mode::All));
    }

    #[test]
    fn test_determine_workspace_with_cli_override() {
        let cli = Cli {
            working_directory: Some(PathBuf::from("/test/path")),
            ..Default::default()
        };
        let result = determine_workspace(&cli).unwrap();
        assert_eq!(result, PathBuf::from("/test/path"));
    }

    #[test]
    fn test_emit_github_output() {
        // This test would need mocking for real testing
        assert!(emit_github_output("test", true).is_ok());
    }
}
