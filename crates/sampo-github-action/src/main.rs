mod error;
mod git;
mod github;
mod sampo;

use crate::error::{ActionError, Result};
use crate::sampo::ReleasePlan;
use sampo_core::errors::SampoError;
use sampo_core::workspace::discover_workspace;
use sampo_core::{Config as SampoConfig, current_branch};
use semver::Version;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// Error type and Result are provided by sampo-core::errors

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
    /// Optional list of target triples to build and upload assets for
    targets: Vec<String>,
}

impl GitHubReleaseOptions {
    fn from_config(config: &Config) -> Self {
        Self {
            create_github_release: config.create_github_release,
            open_discussion: config.open_discussion,
            discussion_category: config.discussion_category.clone(),
            upload_binary: config.upload_binary,
            binary_name: config.binary_name.clone(),
            targets: parse_targets(config.targets.as_deref()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Auto,
    Release,
    Publish,
}

impl Mode {
    fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "auto" | "automatic" => Mode::Auto,
            "release" => Mode::Release,
            "publish" => Mode::Publish,
            _ => Mode::Auto,
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

    /// Optional list of target triples to build and upload assets for (space or comma-separated)
    targets: Option<String>,
}

impl Config {
    /// Load configuration from GitHub Actions environment variables
    fn from_environment() -> Self {
        let mode = std::env::var("INPUT_COMMAND")
            .ok()
            .filter(|v| !v.is_empty())
            .map(|v| Mode::parse(&v))
            .unwrap_or(Mode::Auto);

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

        let targets = std::env::var("INPUT_TARGETS")
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
            targets,
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

    let repo_config = SampoConfig::load(&workspace).unwrap_or_default();

    let branch = current_branch()?;
    if !repo_config.is_release_branch(&branch) {
        return Err(SampoError::Release(format!(
            "Branch '{}' is not listed in git.release_branches (allowed: {:?})",
            branch,
            repo_config
                .release_branches()
                .into_iter()
                .collect::<Vec<_>>()
        ))
        .into());
    }

    unsafe {
        std::env::set_var("SAMPO_RELEASE_BRANCH", &branch);
    }

    // Execute the requested operations
    let (released, published) = execute_operations(&config, &workspace, &repo_config, &branch)?;

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
///
/// In `auto` mode we always run a dry `sampo release` first (`capture_release_plan`).
/// If there are pending changesets we prepare/update the release PR (which runs a
/// real `sampo release`). Otherwise we fall back to `post_merge_publish`, which
/// reuses the same publish pipeline as the explicit `publish` mode. That pipeline
/// only publishes crates that still need it (via `sampo_core::run_publish`) and
/// reports `published = true` exclusively when fresh git tags were produced, so a
/// plain commit without changesets will exit cleanly without pushing anything.
fn execute_operations(
    config: &Config,
    workspace: &Path,
    repo_config: &SampoConfig,
    branch: &str,
) -> Result<(bool, bool)> {
    let mut released = false;
    let mut published = false;

    match config.mode {
        Mode::Auto => {
            let plan = sampo::capture_release_plan(workspace)?;
            if plan.has_changes {
                println!(
                    "Detected {} pending release package(s); preparing release PR.",
                    plan.releases.len()
                );
                let github_client = create_github_client()?;
                released = prepare_release_pr(
                    workspace,
                    config,
                    repo_config,
                    branch,
                    &github_client,
                    Some(plan),
                )?;
            } else {
                println!(
                    "No pending changesets found on branch '{}'. Checking for merged releases to publish.",
                    branch
                );
                let github_options = GitHubReleaseOptions::from_config(config);
                let github_client = if github_options.create_github_release {
                    Some(create_github_client()?)
                } else {
                    None
                };
                published = post_merge_publish(
                    workspace,
                    config.dry_run,
                    config.args.as_deref(),
                    config.cargo_token.as_deref(),
                    &github_options,
                    github_client.as_ref(),
                )?;
            }
        }
        Mode::Release => {
            sampo::run_release(workspace, config.dry_run, config.cargo_token.as_deref())?;
            released = true;
        }
        Mode::Publish => {
            let github_options = GitHubReleaseOptions::from_config(config);
            let github_client = if github_options.create_github_release {
                Some(create_github_client()?)
            } else {
                None
            };
            published = post_merge_publish(
                workspace,
                config.dry_run,
                config.args.as_deref(),
                config.cargo_token.as_deref(),
                &github_options,
                github_client.as_ref(),
            )?;
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
    repo_config: &SampoConfig,
    branch: &str,
    github_client: &github::GitHubClient,
    provided_plan: Option<ReleasePlan>,
) -> Result<bool> {
    let plan = match provided_plan {
        Some(plan) => plan,
        None => sampo::capture_release_plan(workspace)?,
    };

    if !plan.has_changes {
        println!("No changesets detected. Skipping PR preparation.");
        return Ok(false);
    }

    let releases = &plan.releases;

    // Configuration
    let base_branch = config
        .base_branch
        .clone()
        .unwrap_or_else(|| branch.to_string());
    let is_prerelease = releases
        .values()
        .any(|(_, new_version)| Version::parse(new_version).map(|v| !v.pre.is_empty()).unwrap_or(false));
    let pr_branch = config
        .pr_branch
        .clone()
        .unwrap_or_else(|| default_pr_branch(branch, is_prerelease));
    let pr_title = config
        .pr_title
        .clone()
        .unwrap_or_else(|| default_pr_title(branch, is_prerelease));

    // Build PR body BEFORE running release (release will consume changesets)
    let pr_body = {
        // Load configuration for dependency explanations
        let body = sampo::build_release_pr_body(workspace, releases, repo_config)?;
        if body.trim().is_empty() {
            println!("No applicable package changes for PR body. Skipping PR creation.");
            return Ok(false);
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
        return Ok(false);
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

    println!(
        "Prepared release PR with {} pending package(s).",
        releases.len()
    );

    Ok(true)
}

fn sanitized_branch_name(branch: &str) -> String {
    branch.replace('/', "-")
}

fn default_pr_branch(branch: &str, is_prerelease: bool) -> String {
    let suffix = sanitized_branch_name(branch);
    if is_prerelease {
        format!("pre-release/{}", suffix)
    } else {
        format!("release/{}", suffix)
    }
}

fn default_pr_title(branch: &str, is_prerelease: bool) -> String {
    if is_prerelease {
        format!("Pre-release ({})", branch)
    } else {
        format!("Release ({})", branch)
    }
}

/// Run `sampo publish` and handle the post-merge duties (tag push, GitHub releases).
/// Returns true only when new tags were created/pushed, so the workflow can tell if a
/// real publish happened. Combined with `sampo_core::run_publish` (which skips crates
/// already published or marked `publish = false`), this prevents accidental publishes
/// on commits sans changesets: the action simply logs "No new tags" and exits.
fn post_merge_publish(
    workspace: &Path,
    dry_run: bool,
    args: Option<&str>,
    cargo_token: Option<&str>,
    github_options: &GitHubReleaseOptions,
    github_client: Option<&github::GitHubClient>,
) -> Result<bool> {
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
    let published = !new_tags.is_empty();
    if !published {
        println!("No new tags were created during publish.");
    }

    Ok(published)
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

    // Create the release and get upload URL (or find the existing release)
    let upload_url = match github_client.create_release(tag, &body, tag_is_prerelease(tag)) {
        Ok(url) => url,
        Err(e) => {
            eprintln!(
                "Warning: Failed to create GitHub release for {}: {}",
                tag, e
            );
            return Ok(());
        }
    };

    // If binary upload is requested, compile and upload binaries for requested targets (or host)
    if github_options.upload_binary
        && !upload_url.is_empty()
        && let Some((crate_name, _version)) = parse_tag(tag)
    {
        // Locate crate dir
        let ws = discover_workspace(workspace).ok();
        let crate_dir = ws
            .as_ref()
            .and_then(|w| {
                w.members
                    .iter()
                    .find(|c| c.name == crate_name)
                    .map(|c| c.path.clone())
            })
            .unwrap_or_else(|| workspace.join("crates").join(&crate_name));

        if is_binary_crate(&crate_dir) {
            let bin_name = github_options
                .binary_name
                .as_deref()
                .map(|s| s.to_string())
                .or_else(|| resolve_primary_bin_name(&crate_dir, &crate_name));

            // Determine target list: use configured list if provided; otherwise use host only
            let targets = if !github_options.targets.is_empty() {
                github_options.targets.clone()
            } else {
                crate::github::detect_host_triple()
                    .map(|t| vec![t])
                    .unwrap_or_else(|| vec!["unknown-target".to_string()])
            };

            // Strict verification for requested targets: all must be installed
            if !github_options.targets.is_empty() {
                let installed = rustup_installed_targets();
                let missing: Vec<String> = github_options
                    .targets
                    .iter()
                    .filter(|t| !installed.iter().any(|it| it == *t))
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    return Err(ActionError::SampoCommandFailed {
                        operation: "binary-build".to_string(),
                        message: format!(
                            "Missing Rust targets: {}. Install with: rustup target add {}",
                            missing.join(", "),
                            missing.join(" ")
                        ),
                    });
                }
            }

            for triple in targets {
                if let Err(e) = github_client.upload_binary_asset(
                    &upload_url,
                    workspace,
                    bin_name.as_deref(),
                    Some(&crate_name),
                    Some(&triple),
                ) {
                    eprintln!(
                        "Warning: Failed to upload binary for target {}: {}",
                        triple, e
                    );
                } else {
                    println!("Successfully uploaded binary for {} ({})", tag, triple);
                }
            }
        } else {
            println!(
                "Skipping binary upload for {} (crate '{}' is a library)",
                tag, crate_name
            );
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

fn tag_is_prerelease(tag: &str) -> bool {
    parse_tag(tag)
        .and_then(|(_name, version)| Version::parse(&version).ok())
        .map(|parsed| !parsed.pre.is_empty())
        .unwrap_or(false)
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

fn parse_targets(input: Option<&str>) -> Vec<String> {
    input
        .map(|s| {
            s.split(|c: char| c.is_whitespace() || c == ',')
                .filter(|t| !t.is_empty())
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn rustup_installed_targets() -> Vec<String> {
    let out = std::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Determine if a crate directory contains a binary target
fn is_binary_crate(crate_dir: &Path) -> bool {
    // 1) src/main.rs exists
    if crate_dir.join("src").join("main.rs").exists() {
        return true;
    }
    // 2) src/bin contains at least one .rs file
    let bin_dir = crate_dir.join("src").join("bin");
    if bin_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&bin_dir)
    {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                return true;
            }
        }
    }
    // 3) [[bin]] entries in Cargo.toml
    let manifest = crate_dir.join("Cargo.toml");
    if let Ok(text) = std::fs::read_to_string(&manifest)
        && let Ok(val) = text.parse::<toml::Value>()
        && val.get("bin").is_some()
    {
        return true;
    }
    false
}

/// Try to resolve the primary binary name for a crate
/// - If exactly one [[bin]] with a name is defined, return it
/// - Else if src/main.rs exists, default to crate name
/// - Else if exactly one file in src/bin exists, use its stem
fn resolve_primary_bin_name(crate_dir: &Path, crate_name: &str) -> Option<String> {
    let manifest = crate_dir.join("Cargo.toml");
    if let Ok(text) = std::fs::read_to_string(&manifest)
        && let Ok(val) = text.parse::<toml::Value>()
        && let Some(arr) = val.get("bin").and_then(|v| v.as_array())
    {
        // Collect names
        let mut names: Vec<String> = arr
            .iter()
            .filter_map(|item| item.as_table())
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .map(|s| s.to_string())
            .collect();
        names.sort();
        names.dedup();
        if names.len() == 1 {
            return names.into_iter().next();
        }
    }

    // src/main.rs => default to crate name
    if crate_dir.join("src").join("main.rs").exists() {
        return Some(crate_name.to_string());
    }

    // Single file under src/bin => use stem
    let bin_dir = crate_dir.join("src").join("bin");
    if bin_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&bin_dir)
    {
        let files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rs"))
            .collect();
        if files.len() == 1
            && let Some(stem) = files[0].file_stem().and_then(|s| s.to_str())
        {
            return Some(stem.to_string());
        }
    }

    Some(crate_name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_branch_slugifies_path_segments() {
        assert_eq!(default_pr_branch("main", false), "release/main");
        assert_eq!(default_pr_branch("3.x", false), "release/3.x");
        assert_eq!(default_pr_branch("feature/foo", false), "release/feature-foo");
    }

    #[test]
    fn default_title_includes_branch_name() {
        assert_eq!(default_pr_title("main", false), "Release (main)");
        assert_eq!(default_pr_title("3.x", false), "Release (3.x)");
    }

    #[test]
    fn pre_release_branch_has_dedicated_prefix_and_title() {
        assert_eq!(default_pr_branch("next", true), "pre-release/next");
        assert_eq!(default_pr_title("next", true), "Pre-release (next)");
    }

    #[test]
    fn tag_pre_release_detection() {
        assert!(tag_is_prerelease("sampo-v1.2.0-alpha.1"));
        assert!(!tag_is_prerelease("sampo-v1.2.0"));
        assert!(!tag_is_prerelease("invalid"));
    }

    #[test]
    fn test_mode_parsing() {
        assert!(matches!(Mode::parse("auto"), Mode::Auto));
        assert!(matches!(Mode::parse("release"), Mode::Release));
        assert!(matches!(Mode::parse("publish"), Mode::Publish));
        assert!(matches!(Mode::parse("unknown"), Mode::Auto));
    }

    #[test]
    fn test_determine_workspace_with_config_override() {
        let config = Config {
            working_directory: Some(PathBuf::from("/test/path")),
            mode: Mode::Auto,
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
            targets: None,
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
