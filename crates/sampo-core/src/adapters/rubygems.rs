use crate::adapters::format_command_display;
use crate::adapters::scan::LazyScan;
use crate::errors::{Result, SampoError, WorkspaceError};
use crate::process::command;
use crate::types::{ConstraintCheckResult, PackageInfo};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

mod gemspec;

const RUBYGEMS_API_BASE: &str = "https://rubygems.org/api/v2/rubygems";

// rubygems.org allows 10 API requests per second per IP; stay well under it.
const RUBYGEMS_RATE_LIMIT: Duration = Duration::from_millis(200);

static RUBYGEMS_LAST_CALL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Stateless adapter for Ruby gems, published to rubygems.org with Bundler lockfiles.
pub(super) struct RubyGemsAdapter;

impl RubyGemsAdapter {
    pub(super) fn can_discover_scanned(&self, scan: &LazyScan) -> bool {
        gemspec::can_discover_scanned(scan)
    }

    pub(super) fn discover_scanned(
        &self,
        scan: &LazyScan,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        gemspec::discover_scanned(scan)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        gemspec::manifest_path(package_dir)
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        gemspec::is_publishable(manifest_path)
    }

    pub(super) fn queries_registry(&self, manifest_path: &Path) -> bool {
        gemspec::private_push_host(manifest_path).is_none()
    }

    pub(super) fn version_exists(
        &self,
        package_name: &str,
        version: &str,
        manifest_path: Option<&Path>,
    ) -> Result<bool> {
        let name = package_name.trim();
        if name.is_empty() {
            return Err(SampoError::Publish(
                "Package name cannot be empty when checking the RubyGems registry".into(),
            ));
        }

        // `allowed_push_host` sends `gem push` to a private gem server, where rubygems.org
        // knows nothing of the gem; querying there risks a false positive from a same-named
        // public gem.
        let published = match manifest_path {
            Some(path) if !self.queries_registry(path) => return Ok(false),
            Some(path) => gemspec::published_version(path, version),
            None => gemspec::semver_to_gem(version),
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(crate::USER_AGENT)
            .build()
            .map_err(|e| {
                SampoError::Publish(format!("failed to build HTTP client for RubyGems: {}", e))
            })?;

        let url = format!("{RUBYGEMS_API_BASE}/{name}/versions/{published}.json");
        enforce_rubygems_rate_limit();

        let response = client.get(&url).send().map_err(|e| {
            SampoError::Publish(format!(
                "failed to query the RubyGems registry for '{}': {}",
                name, e
            ))
        })?;

        let status_code = response.status();
        match status_code {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .map(|value| format!(" Retry-After: {}", value))
                    .unwrap_or_default();
                Err(SampoError::Publish(format!(
                    "RubyGems registry returned 429 Too Many Requests for '{}@{}'.{}",
                    name, published, retry_after
                )))
            }
            other => {
                let body = response.text().unwrap_or_default();
                let snippet: String = body.trim().chars().take(300).collect();
                let snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
                let body_part = if snippet.is_empty() {
                    String::new()
                } else {
                    format!(" body=\"{}\"", snippet)
                };
                Err(SampoError::Publish(format!(
                    "RubyGems registry returned {} for '{}@{}'{}",
                    other, name, published, body_part
                )))
            }
        }
    }

    /// A dry run stops after `gem build`. `rake release` is avoided: it tags and pushes git
    /// itself, which Sampo already does.
    pub(super) fn publish(
        &self,
        manifest_path: &Path,
        dry_run: bool,
        extra_args: &[String],
    ) -> Result<()> {
        let gem_dir = manifest_path.parent().ok_or_else(|| {
            SampoError::Publish(format!(
                "Gemspec {} does not have a parent directory",
                manifest_path.display()
            ))
        })?;
        let file_name = manifest_path.file_name().ok_or_else(|| {
            SampoError::Publish(format!(
                "Gemspec {} has no file name",
                manifest_path.display()
            ))
        })?;

        // The built file is named with RubyGems' own version spelling, so name it ourselves.
        let out_dir = tempfile::tempdir()?;
        let gem_file = out_dir.path().join("package.gem");

        let mut build = command("gem");
        build
            .current_dir(gem_dir)
            .arg("build")
            .arg(file_name)
            .arg("--output")
            .arg(&gem_file);
        run_gem(&mut build, manifest_path)?;

        if dry_run {
            return Ok(());
        }

        let mut push = command("gem");
        push.current_dir(gem_dir)
            .arg("push")
            .arg(&gem_file)
            .args(extra_args);
        run_gem(&mut push, manifest_path)
    }

    pub(super) fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        // Plain `bundle lock` keeps every third-party pin it can and, since Bundler
        // 2.4.21, ignores frozen and deployment mode; `--local` would fail on a lock that
        // lacks a pin for a registry gem that is not installed.
        for dir in gemspec::gemfile_lock_dirs(workspace_root) {
            println!("Regenerating Gemfile.lock in {}…", dir.display());
            let mut cmd = command("bundle");
            cmd.arg("lock").current_dir(&dir);

            println!("Running: {}", format_command_display(&cmd));

            let status = cmd.status().map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    SampoError::Release(
                        "bundle not found in PATH; install Bundler to regenerate Gemfile.lock"
                            .to_string(),
                    )
                } else {
                    SampoError::Io(err)
                }
            })?;

            if !status.success() {
                return Err(SampoError::Release(format!(
                    "bundle lock failed in {} with status {}",
                    dir.display(),
                    status
                )));
            }

            println!("Gemfile.lock updated.");
        }

        Ok(())
    }
}

fn run_gem(cmd: &mut std::process::Command, manifest_path: &Path) -> Result<()> {
    println!("Running: {}", format_command_display(cmd));

    let status = cmd.status().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SampoError::Publish(
                "gem not found in PATH; ensure Ruby and RubyGems are installed to publish gems"
                    .to_string(),
            )
        } else {
            SampoError::Io(err)
        }
    })?;

    if !status.success() {
        let subcommand = cmd
            .get_args()
            .next()
            .map(|arg| arg.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Err(SampoError::Publish(format!(
            "gem {} failed for {} with status {}",
            subcommand,
            manifest_path.display(),
            status
        )));
    }

    Ok(())
}

/// Write version constants once every gemspec is rewritten. See
/// [`crate::adapters::PackageAdapter::finalize_workspace_roots`].
pub(super) fn finalize_version_files(
    members: &[PackageInfo],
    new_version_by_id: &BTreeMap<String, String>,
) -> Result<()> {
    gemspec::finalize_version_files(members, new_version_by_id)
}

pub(super) fn check_dependency_constraint(
    manifest_path: &Path,
    dep_name: &str,
    _current_constraint: &str,
    new_version: &str,
) -> Result<ConstraintCheckResult> {
    gemspec::check_dependency_constraint(manifest_path, dep_name, new_version)
}

pub(super) fn publish_dry_run(
    packages: &[(&PackageInfo, &Path)],
    extra_args: &[String],
) -> Result<Vec<String>> {
    for (package, manifest) in packages {
        RubyGemsAdapter
            .publish(manifest, true, extra_args)
            .map_err(|err| match err {
                SampoError::Publish(message) => SampoError::Publish(format!(
                    "Dry-run publish failed for {}: {}",
                    package.display_name(true),
                    message
                )),
                other => other,
            })?;
    }

    Ok(Vec::new())
}

fn enforce_rubygems_rate_limit() {
    let lock = RUBYGEMS_LAST_CALL.get_or_init(|| Mutex::new(None));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    if let Some(last_call) = *guard {
        let elapsed = now.saturating_duration_since(last_call);
        if elapsed < RUBYGEMS_RATE_LIMIT {
            thread::sleep(RUBYGEMS_RATE_LIMIT - elapsed);
        }
    }
    *guard = Some(now);
}

/// Update a gemspec's literal version and its requirements on sibling gems.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    gemspec::update_manifest_versions(manifest_path, input, new_pkg_version, new_version_by_name)
}

#[cfg(test)]
mod rubygems_tests;
