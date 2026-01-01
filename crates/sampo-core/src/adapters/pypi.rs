use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::PackageInfo;
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

mod pip;

const PYPI_API_BASE: &str = "https://pypi.org/pypi";

// PyPI doesn't have strict rate limits for JSON API, but we add a small delay for courtesy
const PYPI_RATE_LIMIT: Duration = Duration::from_millis(200);

static PYPI_LAST_CALL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Stateless adapter for PyPI/pip workspaces.
pub(super) struct PyPIAdapter;

impl PyPIAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        pip::can_discover(root)
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        pip::discover(root)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        pip::manifest_path(package_dir)
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        pip::is_publishable(manifest_path)
    }

    pub(super) fn version_exists(
        &self,
        package_name: &str,
        version: &str,
        _manifest_path: Option<&Path>,
    ) -> Result<bool> {
        let name = package_name.trim();
        if name.is_empty() {
            return Err(SampoError::Publish(
                "Package name cannot be empty when checking PyPI registry".into(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(crate::USER_AGENT)
            .build()
            .map_err(|e| {
                SampoError::Publish(format!("failed to build HTTP client for PyPI: {}", e))
            })?;

        // PyPI uses normalized package names (lowercase, hyphens instead of underscores)
        let normalized_name = normalize_package_name(name);
        let url = format!("{}/{}/json", PYPI_API_BASE, normalized_name);
        enforce_pypi_rate_limit();

        let response = client.get(&url).send().map_err(|e| {
            SampoError::Publish(format!(
                "failed to query PyPI registry for '{}': {}",
                name, e
            ))
        })?;

        let status_code = response.status();
        match status_code {
            StatusCode::OK => {
                let body = response.text().map_err(|e| {
                    SampoError::Publish(format!("failed to read PyPI response: {}", e))
                })?;
                let json: JsonValue = serde_json::from_str(&body)
                    .map_err(|e| SampoError::Publish(format!("invalid JSON from PyPI: {}", e)))?;

                // Check if the specific version exists in the releases
                let releases = json
                    .get("releases")
                    .and_then(JsonValue::as_object)
                    .ok_or_else(|| {
                        SampoError::Publish(format!(
                            "PyPI response for '{}' is missing a 'releases' object",
                            name
                        ))
                    })?;

                Ok(releases.contains_key(version))
            }
            StatusCode::NOT_FOUND => Ok(false),
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .map(|value| format!(" Retry-After: {}", value))
                    .unwrap_or_default();
                Err(SampoError::Publish(format!(
                    "PyPI registry returned 429 Too Many Requests for '{}@{}'.{}",
                    name, version, retry_after
                )))
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(SampoError::Publish(format!(
                "PyPI registry returned {} for '{}@{}'; authentication may be required",
                status_code, name, version
            ))),
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
                    "PyPI registry returned {} for '{}@{}'{}",
                    other, name, version, body_part
                )))
            }
        }
    }

    pub(super) fn publish(
        &self,
        manifest_path: &Path,
        dry_run: bool,
        extra_args: &[String],
    ) -> Result<()> {
        pip::publish(manifest_path, dry_run, extra_args)
    }

    pub(super) fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        pip::regenerate_lockfile(workspace_root)
    }
}

pub(super) fn publish_dry_run(
    packages: &[(&PackageInfo, &Path)],
    extra_args: &[String],
) -> Result<()> {
    for (package, manifest) in packages {
        PyPIAdapter
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

    Ok(())
}

fn enforce_pypi_rate_limit() {
    let lock = PYPI_LAST_CALL.get_or_init(|| Mutex::new(None));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    if let Some(last_call) = *guard {
        let elapsed = now.saturating_duration_since(last_call);
        if elapsed < PYPI_RATE_LIMIT {
            thread::sleep(PYPI_RATE_LIMIT - elapsed);
        }
    }
    *guard = Some(now);
}

/// Normalize a Python package name according to PEP 503.
/// Converts to lowercase and replaces underscores/dots with hyphens.
fn normalize_package_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c == '_' || c == '.' { '-' } else { c })
        .collect()
}

/// Update a pyproject.toml manifest with a new package version and refreshed dependency requirements.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    pip::update_manifest_versions(manifest_path, input, new_pkg_version, new_version_by_name)
}

#[cfg(test)]
mod pypi_tests;
