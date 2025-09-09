mod git;
mod github;
mod sampo;

use sampo_core::workspace::discover_workspace;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use thiserror::Error;

#[derive(Debug, Error)]
enum ActionError {
    #[error("No working directory provided and GITHUB_WORKSPACE is not set")]
    NoWorkingDirectory,
    #[error("Failed to execute sampo {operation}: {message}")]
    SampoCommandFailed { operation: String, message: String },
    #[error("GitHub credentials not available: GITHUB_REPOSITORY and GITHUB_TOKEN must be set")]
    GitHubCredentialsNotAvailable,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, ActionError>;

#[derive(Debug, Clone)]
struct GitHubReleaseOptions {
    /// Create GitHub releases for newly created tags during publish
    create_github_release: bool,
    /// Also open a GitHub Discussion for each created release
    open_discussion: bool,
    /// Preferred Discussions category slug (e.g., "announcements")
    discussion_category: Option<String>,
    /// Upload Linux binary as release asset when creating GitHub releases
    upload_binary: bool,
    /// Binary name to upload (defaults to the main package name)
    binary_name: Option<String>,
}

impl GitHubReleaseOptions {
    fn from_config(config: &Config) -> Self {
        Self {
            create_github_release: config.create_github_release,
            open_discussion: config.open_discussion,
            discussion_category: config.discussion_category.clone(),
            upload_binary: config.upload_binary,
            binary_name: config.binary_name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Release,
    Publish,
    All,
    /// Detect changesets and open/update a Release PR (no tags)
    PreparePr,
    /// After merge to main: create tags for current versions, push, and publish
    PostMergePublish,
}

impl Mode {
    fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "release" => Mode::Release,
            "publish" => Mode::Publish,
            "release-and-publish" | "all" => Mode::All,
            "prepare-pr" | "release-pr" | "open-pr" => Mode::PreparePr,
            "post-merge-publish" | "finalize" => Mode::PostMergePublish,
            _ => Mode::All, // default fallback
        }
    }
}

/// Sampo GitHub Action configuration
///
/// This configuration reads inputs from GitHub Actions environment variables.
/// GitHub Actions inputs are exposed as INPUT_* environment variables.
#[derive(Debug)]
struct Config {
    /// Which operation to run (release, publish, or both)
    mode: Mode,

    /// Simulate actions without changing files or publishing artifacts
    dry_run: bool,

    /// Path to the repository root (defaults to GITHUB_WORKSPACE)
    working_directory: Option<PathBuf>,

    /// Optional crates.io token (exported to child processes)
    cargo_token: Option<String>,

    /// Extra args passed through to `sampo publish` (after `--`)
    /// Accepts a single string that will be split on whitespace.
    args: Option<String>,

    /// Base branch for the Release PR (default: current ref name or 'main')
    base_branch: Option<String>,

    /// Branch name to use for the Release PR (default: 'release/sampo')
    pr_branch: Option<String>,

    /// Title to use for the Release PR (default: 'Release')
    pr_title: Option<String>,

    /// Create GitHub releases for newly created tags during publish
    create_github_release: bool,

    /// Also open a GitHub Discussion for each created release
    open_discussion: bool,

    /// Preferred Discussions category slug (e.g., "announcements")
    discussion_category: Option<String>,

    /// Upload Linux binary as release asset when creating GitHub releases
    upload_binary: bool,

    /// Binary name to upload (defaults to the main package name)
    binary_name: Option<String>,
}

impl Config {
    /// Load configuration from GitHub Actions environment variables
    fn from_environment() -> Self {
        let mode = std::env::var("INPUT_COMMAND")
            .map(|v| Mode::parse(&v))
            .unwrap_or(Mode::All);

        let dry_run = std::env::var("INPUT_DRY_RUN")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let working_directory = std::env::var("INPUT_WORKING_DIRECTORY")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);

        let cargo_token = std::env::var("INPUT_CARGO_TOKEN")
            .ok()
            .filter(|v| !v.is_empty());

        let args = std::env::var("INPUT_ARGS").ok().filter(|v| !v.is_empty());

        let base_branch = std::env::var("INPUT_BASE_BRANCH")
            .ok()
            .filter(|v| !v.is_empty());

        let pr_branch = std::env::var("INPUT_PR_BRANCH")
            .ok()
            .filter(|v| !v.is_empty());

        let pr_title = std::env::var("INPUT_PR_TITLE")
            .ok()
            .filter(|v| !v.is_empty());

        let create_github_release = std::env::var("INPUT_CREATE_GITHUB_RELEASE")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let open_discussion = std::env::var("INPUT_OPEN_DISCUSSION")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let discussion_category = std::env::var("INPUT_DISCUSSION_CATEGORY")
            .ok()
            .filter(|v| !v.is_empty());

        let upload_binary = std::env::var("INPUT_UPLOAD_BINARY")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let binary_name = std::env::var("INPUT_BINARY_NAME")
            .ok()
            .filter(|v| !v.is_empty());

        Self {
            mode,
            dry_run,
            working_directory,
            cargo_token,
            args,
            base_branch,
            pr_branch,
            pr_title,
            create_github_release,
            open_discussion,
            discussion_category,
            upload_binary,
            binary_name,
        }
    }
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
    let config = Config::from_environment();

    let workspace = determine_workspace(&config)?;

    // Execute the requested operations
    let (released, published) = execute_operations(&config, &workspace)?;

    // Emit outputs for the workflow
    emit_github_output("released", released)?;
    emit_github_output("published", published)?;

    Ok(())
}

/// Determine the workspace root directory
fn determine_workspace(config: &Config) -> Result<PathBuf> {
    config
        .working_directory
        .clone()
        .or_else(|| std::env::var("GITHUB_WORKSPACE").ok().map(PathBuf::from))
        .ok_or(ActionError::NoWorkingDirectory)
}

/// Execute the requested operations and return (released, published) status
fn execute_operations(config: &Config, workspace: &Path) -> Result<(bool, bool)> {
    let mut released = false;
    let mut published = false;

    match config.mode {
        Mode::Release => {
            sampo::run_release(workspace, config.dry_run, config.cargo_token.as_deref())?;
            released = true;
        }
        Mode::Publish => {
            sampo::run_publish(
                workspace,
                config.dry_run,
                config.args.as_deref(),
                config.cargo_token.as_deref(),
            )?;
            published = true;
        }
        Mode::All => {
            sampo::run_release(workspace, config.dry_run, config.cargo_token.as_deref())?;
            released = true;

            sampo::run_publish(
                workspace,
                config.dry_run,
                config.args.as_deref(),
                config.cargo_token.as_deref(),
            )?;
            published = true;
        }
        Mode::PreparePr => {
            let github_client = create_github_client()?;
            prepare_release_pr(workspace, config, &github_client)?;
        }
        Mode::PostMergePublish => {
            let github_options = GitHubReleaseOptions::from_config(config);
            if github_options.create_github_release {
                let github_client = create_github_client()?;
                post_merge_publish(
                    workspace,
                    config.dry_run,
                    config.args.as_deref(),
                    config.cargo_token.as_deref(),
                    &github_options,
                    Some(&github_client),
                )?;
            } else {
                post_merge_publish(
                    workspace,
                    config.dry_run,
                    config.args.as_deref(),
                    config.cargo_token.as_deref(),
                    &github_options,
                    None,
                )?;
            }
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

/// Create a GitHub client if credentials are available
fn create_github_client() -> Result<github::GitHubClient> {
    let repo = std::env::var("GITHUB_REPOSITORY")
        .map_err(|_| ActionError::GitHubCredentialsNotAvailable)?;
    let token =
        std::env::var("GITHUB_TOKEN").map_err(|_| ActionError::GitHubCredentialsNotAvailable)?;

    if repo.is_empty() || token.is_empty() {
        return Err(ActionError::GitHubCredentialsNotAvailable);
    }

    github::GitHubClient::new(repo, token)
}

fn prepare_release_pr(
    workspace: &Path,
    config: &Config,
    github_client: &github::GitHubClient,
) -> Result<()> {
    // Check if there are changes to release
    let plan = sampo::capture_release_plan(workspace)?;
    if !plan.has_changes {
        println!("No changesets detected. Skipping PR preparation.");
        return Ok(());
    }

    // Configuration
    let base_branch = config
        .base_branch
        .clone()
        .or_else(|| std::env::var("GITHUB_REF_NAME").ok())
        .unwrap_or_else(|| "main".into());
    let pr_branch = config
        .pr_branch
        .clone()
        .unwrap_or_else(|| "release/sampo".into());
    let pr_title = config.pr_title.clone().unwrap_or_else(|| "Release".into());

    // Build PR body BEFORE running release (release will consume changesets)
    let pr_body = {
        // Load configuration for dependency explanations
        let sampo_config = sampo_core::Config::load(workspace).unwrap_or_default();
        let body = sampo::build_release_pr_body(workspace, &plan.releases, &sampo_config)?;
        if body.trim().is_empty() {
            println!("No applicable package changes for PR body. Skipping PR creation.");
            return Ok(());
        }
        body
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
    sampo::run_release(workspace, false, config.cargo_token.as_deref())?;

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
    github_client.ensure_pull_request(&pr_branch, &base_branch, &pr_title, &pr_body)?;

    Ok(())
}

fn post_merge_publish(
    workspace: &Path,
    dry_run: bool,
    args: Option<&str>,
    cargo_token: Option<&str>,
    github_options: &GitHubReleaseOptions,
    github_client: Option<&github::GitHubClient>,
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
    if github_options.create_github_release
        && !new_tags.is_empty()
        && let Some(client) = github_client
    {
        for tag in &new_tags {
            println!("Creating GitHub release for {}", tag);
            create_github_release_for_tag(client, tag, workspace, github_options)?;
        }
    }
    Ok(())
}

fn create_github_release_for_tag(
    github_client: &github::GitHubClient,
    tag: &str,
    workspace: &Path,
    github_options: &GitHubReleaseOptions,
) -> Result<()> {
    let body = match build_release_body_from_changelog(workspace, tag) {
        Some(body) => body,
        None => format!("Automated release for tag {}", tag),
    };

    // Create the release and get upload URL
    let upload_url = match github_client.create_release(tag, &body) {
        Ok(url) => url,
        Err(e) => {
            eprintln!(
                "Warning: Failed to create GitHub release for {}: {}",
                tag, e
            );
            return Ok(());
        }
    };

    // If binary upload is requested, compile and upload the binary
    if github_options.upload_binary && !upload_url.is_empty() {
        if let Err(e) = github_client.upload_binary_asset(
            &upload_url,
            workspace,
            github_options.binary_name.as_deref(),
        ) {
            eprintln!("Warning: Failed to upload binary: {}", e);
        } else {
            println!("Successfully uploaded binary for {}", tag);
        }
    }

    // Optionally open a Discussion for this release
    if github_options.open_discussion
        && let Err(e) = github_client.create_discussion(
            tag,
            &body,
            github_options.discussion_category.as_deref(),
        )
    {
        eprintln!("Warning: Failed to create discussion for {}: {}", tag, e);
    }

    Ok(())
}

/// Build a release body by extracting the matching section from the crate's CHANGELOG.md
fn build_release_body_from_changelog(workspace: &Path, tag: &str) -> Option<String> {
    let (crate_name, version) = parse_tag(tag)?;

    // Find crate directory by name using the workspace API
    let ws = discover_workspace(workspace).ok()?;
    let crate_dir = ws
        .members
        .iter()
        .find(|c| c.name == crate_name)
        .map(|c| c.path.clone())
        // Fallback to a conventional path if discovery failed to find it
        .unwrap_or_else(|| workspace.join("crates").join(&crate_name));

    let changelog = crate_dir.join("CHANGELOG.md");
    extract_changelog_section(&changelog, &version)
}

/// Parse tags in the format "<crate>-v<version>"
fn parse_tag(tag: &str) -> Option<(String, String)> {
    let idx = tag.rfind("-v")?;
    let (name, ver) = tag.split_at(idx);
    let version = ver.trim_start_matches("-v").to_string();
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version))
}

/// Extract the section that starts at "## <version>" until the next "## " or EOF
fn extract_changelog_section(path: &Path, version: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let needle = format!("## {}", version);
    let start = text.find(&needle)?;

    // Find the next header starting at the line after our needle
    let next = text[start + needle.len()..]
        .find("\n## ")
        .map(|ofs| start + needle.len() + ofs)
        .unwrap_or_else(|| text.len());

    // Include from the beginning of the line with our needle
    let head = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let slice = &text[head..next];
    let body = slice.trim().to_string();
    if body.is_empty() { None } else { Some(body) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_parsing() {
        assert!(matches!(Mode::parse("release"), Mode::Release));
        assert!(matches!(Mode::parse("publish"), Mode::Publish));
        assert!(matches!(Mode::parse("all"), Mode::All));
        assert!(matches!(Mode::parse("release-and-publish"), Mode::All));
        assert!(matches!(Mode::parse("prepare-pr"), Mode::PreparePr));
        assert!(matches!(
            Mode::parse("post-merge-publish"),
            Mode::PostMergePublish
        ));
        assert!(matches!(Mode::parse("unknown"), Mode::All)); // default fallback
    }

    #[test]
    fn test_determine_workspace_with_config_override() {
        let config = Config {
            working_directory: Some(PathBuf::from("/test/path")),
            mode: Mode::All,
            dry_run: false,
            cargo_token: None,
            args: None,
            base_branch: None,
            pr_branch: None,
            pr_title: None,
            create_github_release: false,
            open_discussion: false,
            discussion_category: None,
            upload_binary: false,
            binary_name: None,
        };
        let result = determine_workspace(&config).unwrap();
        assert_eq!(result, PathBuf::from("/test/path"));
    }

    #[test]
    fn test_create_github_client() {
        // Test without environment variables
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("GITHUB_TOKEN");
        }
        assert!(create_github_client().is_err());

        // Test with empty values
        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "");
            std::env::set_var("GITHUB_TOKEN", "token");
        }
        assert!(create_github_client().is_err());

        // Test with valid values
        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "owner/repo");
            std::env::set_var("GITHUB_TOKEN", "valid_token");
        }
        assert!(create_github_client().is_ok());

        // Clean up
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("GITHUB_TOKEN");
        }
    }

    #[test]
    fn test_emit_github_output() {
        // This test would need mocking for real testing
        assert!(emit_github_output("test", true).is_ok());
    }

    #[test]
    fn test_parse_tag() {
        assert_eq!(
            parse_tag("my-crate-v1.2.3"),
            Some(("my-crate".into(), "1.2.3".into()))
        );
        assert_eq!(parse_tag("nope"), None);
        assert_eq!(parse_tag("-v1.0.0"), None);
    }

    #[test]
    fn test_extract_changelog_section() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("CHANGELOG.md");
        let content =
            "# my-crate\n\n## 1.2.3\n\n### Patch changes\n\n- Fix: foo\n\n## 1.2.2\n\n- Older";
        fs::write(&file, content).unwrap();

        let got = extract_changelog_section(&file, "1.2.3").unwrap();
        assert!(got.starts_with("## 1.2.3"));
        assert!(got.contains("Fix: foo"));

        let older = extract_changelog_section(&file, "1.2.2").unwrap();
        assert!(older.starts_with("## 1.2.2"));
        assert!(!older.contains("1.2.3"));
    }
}
