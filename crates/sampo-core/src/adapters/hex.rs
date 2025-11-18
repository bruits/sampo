use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::PackageInfo;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

mod mix;

const HEX_API_BASE: &str = "https://hex.pm/api";
const HEX_USER_AGENT: &str = concat!("sampo-core/", env!("CARGO_PKG_VERSION"));
// Hex public docs specify 100 anonymous requests per minute -> https://hexpm.docs.apiary.io/#introduction/rate-limiting
const HEX_RATE_LIMIT: Duration = Duration::from_millis(600);

static HEX_LAST_CALL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Stateless adapter for Hex/Mix workspaces.
pub(super) struct HexAdapter;

impl HexAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        mix::can_discover(root)
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        mix::discover(root)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        mix::manifest_path(package_dir)
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        mix::is_publishable(manifest_path)
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
                "Package name cannot be empty when checking Hex registry".into(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(HEX_USER_AGENT)
            .build()
            .map_err(|e| {
                SampoError::Publish(format!("failed to build HTTP client for Hex: {}", e))
            })?;

        let url = format!("{HEX_API_BASE}/packages/{}/releases/{}", name, version);
        enforce_hex_rate_limit();
        let response = client.get(&url).send().map_err(|e| {
            SampoError::Publish(format!(
                "failed to query Hex registry for '{}': {}",
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
                    "Hex registry returned 429 Too Many Requests for '{}@{}'.{}",
                    name, version, retry_after
                )))
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(SampoError::Publish(format!(
                "Hex registry returned {} for '{}@{}'; authentication may be required",
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
                    "Hex registry returned {} for '{}@{}'{}",
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
        mix::publish(manifest_path, dry_run, extra_args)
    }

    pub(super) fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        mix::regenerate_lockfile(workspace_root)
    }
}

pub(super) fn publish_dry_run(
    packages: &[(&PackageInfo, &Path)],
    extra_args: &[String],
) -> Result<()> {
    for (package, manifest) in packages {
        HexAdapter
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

fn enforce_hex_rate_limit() {
    let lock = HEX_LAST_CALL.get_or_init(|| Mutex::new(None));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    if let Some(last_call) = *guard {
        let elapsed = now.saturating_duration_since(last_call);
        if elapsed < HEX_RATE_LIMIT {
            thread::sleep(HEX_RATE_LIMIT - elapsed);
        }
    }
    *guard = Some(now);
}

/// Update a Mix manifest with a new package version and refreshed dependency requirements.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    mix::update_manifest_versions(manifest_path, input, new_pkg_version, new_version_by_name)
}

#[cfg(test)]
mod hex_tests;
