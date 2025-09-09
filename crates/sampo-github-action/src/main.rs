mod git;
mod github;
mod sampo;

use sampo_core::workspace::discover_workspace;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    #[serde(rename = "upload_url")]
    upload_url: String,
}

#[derive(Debug, Serialize)]
struct CreateReleaseRequest {
    tag_name: String,
    name: String,
    body: String,
    draft: bool,
    prerelease: bool,
}

#[derive(Debug, Deserialize)]
struct DiscussionCategory {
    id: u64,
    slug: String,
}

#[derive(Debug, Serialize)]
struct CreateDiscussionRequest {
    title: String,
    body: String,
    category_id: u64,
}

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
            prepare_release_pr(workspace, config)?;
        }
        Mode::PostMergePublish => {
            let github_options = GitHubReleaseOptions::from_config(config);
            post_merge_publish(
                workspace,
                config.dry_run,
                config.args.as_deref(),
                config.cargo_token.as_deref(),
                &github_options,
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

fn prepare_release_pr(workspace: &Path, config: &Config) -> Result<()> {
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
    github_options: &GitHubReleaseOptions,
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
    if github_options.create_github_release && !new_tags.is_empty() {
        let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
        let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();

        if !repo.is_empty() && !token.is_empty() {
            for tag in &new_tags {
                println!("Creating GitHub release for {}", tag);
                create_github_release_for_tag(&repo, &token, tag, workspace, github_options)?;
            }
        }
    }

    Ok(())
}

fn create_github_release_for_tag(
    repo: &str,
    token: &str,
    tag: &str,
    workspace: &Path,
    github_options: &GitHubReleaseOptions,
) -> Result<()> {
    let api = format!("https://api.github.com/repos/{}/releases", repo);
    let body = match build_release_body_from_changelog(workspace, tag) {
        Some(body) => body,
        None => format!("Automated release for tag {}", tag),
    };

    let request = CreateReleaseRequest {
        tag_name: tag.to_string(),
        name: tag.to_string(),
        body: body.clone(), // Clone for later use in discussion
        draft: false,
        prerelease: false,
    };

    let payload = serde_json::to_string(&request).map_err(|e| ActionError::SampoCommandFailed {
        operation: "serialize-release-request".to_string(),
        message: e.to_string(),
    })?;

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

    // Parse response to get upload URL for binary upload
    let upload_url = if output.status.success() {
        match serde_json::from_slice::<GitHubRelease>(&output.stdout) {
            Ok(release) => {
                // GitHub returns URLs with {?name,label} template - remove the template part
                release
                    .upload_url
                    .split('{')
                    .next()
                    .unwrap_or("")
                    .to_string()
            }
            Err(_) => {
                eprintln!("Warning: Could not parse GitHub release response");
                String::new()
            }
        }
    } else {
        String::new()
    };

    // If binary upload is requested, compile and upload the binary
    if github_options.upload_binary && !upload_url.is_empty() {
        if let Err(e) = build_and_upload_binary(
            workspace,
            &upload_url,
            token,
            github_options.binary_name.as_deref(),
        ) {
            eprintln!("Warning: Failed to upload binary: {}", e);
        } else {
            println!("Successfully uploaded binary for {}", tag);
        }
    }

    // Optionally open a Discussion for this release
    if github_options.open_discussion
        && let Err(e) = create_discussion_for_release(
            repo,
            token,
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

/// Create a GitHub Discussion for a release using the preferred category if available
fn create_discussion_for_release(
    repo: &str,
    token: &str,
    tag: &str,
    body: &str,
    preferred_category: Option<&str>,
) -> Result<()> {
    let categories_url = format!(
        "https://api.github.com/repos/{}/discussions/categories",
        repo
    );

    let out = Command::new("curl")
        .args([
            "-sS",
            "-H",
            &format!("Authorization: Bearer {}", token),
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            &categories_url,
        ])
        .output()
        .map_err(ActionError::Io)?;

    if !out.status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "github-list-discussion-categories".to_string(),
            message: String::from_utf8_lossy(&out.stderr).to_string(),
        });
    }

    let resp = String::from_utf8_lossy(&out.stdout);
    let categories: Vec<DiscussionCategory> =
        serde_json::from_str(&resp).map_err(|e| ActionError::SampoCommandFailed {
            operation: "github-parse-discussion-categories".to_string(),
            message: format!("Failed to parse categories JSON: {}", e),
        })?;

    let desired_slug = preferred_category
        .and_then(|s| if s.trim().is_empty() { None } else { Some(s) })
        .unwrap_or("announcements");

    // Find category by slug, with fallbacks
    let category_id = categories
        .iter()
        .find(|cat| cat.slug == desired_slug)
        .or_else(|| categories.iter().find(|cat| cat.slug == "announcements"))
        .or_else(|| categories.first())
        .map(|cat| cat.id)
        .ok_or_else(|| ActionError::SampoCommandFailed {
            operation: "github-find-discussion-category".to_string(),
            message: "No discussion categories available".into(),
        })?;

    let discussions_url = format!("https://api.github.com/repos/{}/discussions", repo);
    let title = format!("Release {}", tag);
    let body_with_link = format!(
        "{}\n\n—\nSee release page: https://github.com/{}/releases/tag/{}",
        body, repo, tag
    );

    let request = CreateDiscussionRequest {
        title,
        body: body_with_link,
        category_id,
    };

    let payload = serde_json::to_string(&request).map_err(|e| ActionError::SampoCommandFailed {
        operation: "serialize-discussion-request".to_string(),
        message: e.to_string(),
    })?;

    let out = Command::new("curl")
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
            &discussions_url,
            "-d",
            &payload,
        ])
        .output()
        .map_err(ActionError::Io)?;

    if !out.status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "github-create-discussion".to_string(),
            message: format!(
                "Failed to create discussion: stdout={} stderr={}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        });
    }

    println!("Opened GitHub Discussion for {}", tag);
    Ok(())
}

/// Build a Linux binary and upload it to GitHub release
fn build_and_upload_binary(
    workspace: &Path,
    upload_url: &str,
    token: &str,
    binary_name: Option<&str>,
) -> Result<()> {
    // Determine binary name - use provided name or workspace directory name
    let bin_name = binary_name.unwrap_or_else(|| {
        workspace
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("app")
    });

    println!("Building Linux binary: {}", bin_name);

    // Cross-compile for Linux
    let output = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--bin",
            bin_name,
        ])
        .current_dir(workspace)
        .output()?;

    if !output.status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "cross-compile".to_string(),
            message: format!(
                "Failed to build Linux binary: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }

    // Path to the compiled binary
    let binary_path = workspace
        .join("target")
        .join("x86_64-unknown-linux-gnu")
        .join("release")
        .join(bin_name);

    if !binary_path.exists() {
        return Err(ActionError::SampoCommandFailed {
            operation: "binary-locate".to_string(),
            message: format!("Binary not found at {}", binary_path.display()),
        });
    }

    // Upload the binary as a release asset
    let asset_name = format!("{}-linux-x64", bin_name);
    println!("Uploading binary as {}", asset_name);

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
            "Content-Type: application/octet-stream",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            &format!("{}?name={}", upload_url, asset_name),
            "--data-binary",
            &format!("@{}", binary_path.display()),
        ])
        .output()?;

    if !output.status.success() {
        return Err(ActionError::SampoCommandFailed {
            operation: "binary-upload".to_string(),
            message: format!(
                "Failed to upload binary: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }

    println!("Binary uploaded successfully");
    Ok(())
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
