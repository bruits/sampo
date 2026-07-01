use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::PackageInfo;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

mod gleam;
mod mix;

const HEX_API_BASE: &str = "https://hex.pm/api";
const GLEAM_MANIFEST: &str = "gleam.toml";
// Hex public docs specify 100 anonymous requests per minute -> https://hexpm.docs.apiary.io/#introduction/rate-limiting
const HEX_RATE_LIMIT: Duration = Duration::from_millis(600);

static HEX_LAST_CALL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Stateless adapter for Hex/Mix workspaces.
pub(super) struct HexAdapter;

impl HexAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        mix::can_discover(root) || gleam::can_discover(root)
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        // Elixir and Gleam packages can live side by side in a BEAM monorepo; each
        // build tool discovers its own members and they merge into one Hex workspace.
        let mut packages = Vec::new();
        if mix::can_discover(root) {
            packages.extend(mix::discover(root)?);
        }
        if gleam::can_discover(root) {
            packages.extend(gleam::discover(root)?);
        }
        Ok(packages)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        // Mix wins a directory that holds both manifests (mix_gleam interop), matching
        // discovery; a Gleam-only directory resolves to gleam.toml. Default to mix.exs.
        if package_dir.join("mix.exs").exists() {
            mix::manifest_path(package_dir)
        } else if package_dir.join(GLEAM_MANIFEST).exists() {
            gleam::manifest_path(package_dir)
        } else {
            mix::manifest_path(package_dir)
        }
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        if is_gleam_manifest(manifest_path) {
            gleam::is_publishable(manifest_path)
        } else {
            mix::is_publishable(manifest_path)
        }
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
                "Package name cannot be empty when checking Hex registry".into(),
            ));
        }

        // Private packages live under an organisation and require authentication; the
        // public path needs neither. Gleam has no private-org concept, so its packages
        // always resolve through the public endpoint.
        let organization = manifest_path.and_then(|path| {
            if is_gleam_manifest(path) {
                None
            } else {
                mix::read_organization(path)
            }
        });
        let api_key = hex_api_key();

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(crate::USER_AGENT)
            .build()
            .map_err(|e| {
                SampoError::Publish(format!("failed to build HTTP client for Hex: {}", e))
            })?;

        let url = registry_url(organization.as_deref(), name, version);
        enforce_hex_rate_limit();
        let response = version_check_request(&client, &url, api_key.as_deref())
            .send()
            .map_err(|e| {
                SampoError::Publish(format!(
                    "failed to query Hex registry for '{}': {}",
                    name, e
                ))
            })?;

        let status_code = response.status();
        match status_code {
            StatusCode::OK => Ok(true),
            // Hex hides private packages behind 404, so an unauthenticated 404 for an
            // organisation is ambiguous: error out rather than assert the version is absent.
            StatusCode::NOT_FOUND => match (&organization, &api_key) {
                (Some(org), None) => Err(SampoError::Publish(format!(
                    "Hex returned 404 for private organisation '{org}'; set HEX_API_KEY to \
                     check whether '{name}@{version}' is already published"
                ))),
                // A 404 with a key set is still ambiguous (the key may lack org access),
                // but proceed as absent since publishing enforces access anyway.
                (Some(org), Some(_)) => {
                    eprintln!(
                        "Warning: Hex returned 404 for '{name}@{version}' in private \
                         organisation '{org}'; the version may be unpublished, or HEX_API_KEY \
                         may lack access to the organisation."
                    );
                    Ok(false)
                }
                _ => Ok(false),
            },
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
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                // With a key present the request was authenticated and still rejected, so the
                // key — not its absence — is the likely cause.
                let detail = if api_key.is_some() {
                    "HEX_API_KEY may be invalid or lack the required access"
                } else {
                    "authentication may be required; set HEX_API_KEY to authenticate"
                };
                Err(SampoError::Publish(format!(
                    "Hex registry returned {} for '{}@{}'; {}",
                    status_code, name, version, detail
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
        if is_gleam_manifest(manifest_path) {
            gleam::publish(manifest_path, dry_run, extra_args)
        } else {
            mix::publish(manifest_path, dry_run, extra_args)
        }
    }

    pub(super) fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        if workspace_root.join("mix.exs").exists() {
            mix::regenerate_lockfile(workspace_root)?;
        }
        if gleam::can_discover(workspace_root) {
            gleam::regenerate_lockfile(workspace_root)?;
        }
        Ok(())
    }
}

fn is_gleam_manifest(manifest_path: &Path) -> bool {
    manifest_path.file_name().and_then(|n| n.to_str()) == Some(GLEAM_MANIFEST)
}

fn registry_url(organization: Option<&str>, name: &str, version: &str) -> String {
    match organization {
        Some(org) => format!("{HEX_API_BASE}/repos/{org}/packages/{name}/releases/{version}"),
        None => format!("{HEX_API_BASE}/packages/{name}/releases/{version}"),
    }
}

fn hex_api_key() -> Option<String> {
    resolve_hex_api_key(|key| std::env::var(key).ok())
}

fn resolve_hex_api_key(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    let trimmed = lookup("HEX_API_KEY")?.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Hex expects the raw key in `Authorization`, with no `Bearer` prefix.
/// See https://github.com/hexpm/specifications.
fn version_check_request(
    client: &Client,
    url: &str,
    api_key: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    let request = client.get(url);
    match api_key {
        Some(key) => request.header(reqwest::header::AUTHORIZATION, key),
        None => request,
    }
}

pub(super) fn check_dependency_constraint(
    manifest_path: &Path,
    dep_name: &str,
    _current_constraint: &str,
    new_version: &str,
) -> Result<crate::types::ConstraintCheckResult> {
    use crate::types::ConstraintCheckResult;

    let dependency_value = if is_gleam_manifest(manifest_path) {
        gleam::find_dependency_constraint_value(manifest_path, dep_name)?
    } else {
        mix::find_dependency_constraint_value(manifest_path, dep_name)?
    };
    let constraint = match dependency_value {
        Some(c) => c,
        None => {
            return Ok(ConstraintCheckResult::Skipped {
                reason: format!("dependency '{}' not found in manifest", dep_name),
            });
        }
    };

    let trimmed = constraint.trim();
    if trimmed.is_empty() {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "empty constraint".to_string(),
        });
    }

    if new_version.trim().contains('-') {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pre-release version".to_string(),
        });
    }

    // Skip pinned (bare) versions without any operator or conjunction
    if !trimmed.starts_with("~>")
        && !trimmed.starts_with("!=")
        && !trimmed.starts_with("==")
        && !trimmed.starts_with(">=")
        && !trimmed.starts_with("<=")
        && !trimmed.starts_with('>')
        && !trimmed.starts_with('<')
        && !trimmed.starts_with('=')
        && !trimmed.contains(" and ")
        && !trimmed.contains(" or ")
        && !trimmed.contains(" AND ")
        && !trimmed.contains(" OR ")
        && parse_hex_version(trimmed).is_some()
    {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pinned version".to_string(),
        });
    }

    let version = match parse_hex_version(new_version.trim()) {
        Some(v) => v,
        None => {
            return Ok(ConstraintCheckResult::Skipped {
                reason: format!("unparseable version '{}'", new_version),
            });
        }
    };

    match hex_version_satisfies(trimmed, version) {
        Some(true) => Ok(ConstraintCheckResult::Satisfied),
        Some(false) => Ok(ConstraintCheckResult::NotSatisfied {
            constraint: trimmed.to_string(),
            new_version: new_version.trim().to_string(),
        }),
        None => Ok(ConstraintCheckResult::Skipped {
            reason: format!("unparseable constraint '{}'", trimmed),
        }),
    }
}

/// Ignores pre-release tags.
fn parse_hex_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let base = s.split('-').next()?;
    let parts: Vec<&str> = base.split('.').collect();
    match parts.len() {
        3 => {
            let major = parts[0].parse().ok()?;
            let minor = parts[1].parse().ok()?;
            let patch = parts[2].parse().ok()?;
            Some((major, minor, patch))
        }
        2 => {
            let major = parts[0].parse().ok()?;
            let minor = parts[1].parse().ok()?;
            Some((major, minor, 0))
        }
        _ => None,
    }
}

/// Returns `None` if the constraint is unparseable.
fn hex_version_satisfies(constraint: &str, version: (u64, u64, u64)) -> Option<bool> {
    let lowered = constraint.to_ascii_lowercase();

    if lowered.contains(" or ") {
        let parts = split_on_keyword(constraint, " or ");
        for part in &parts {
            match hex_version_satisfies(part.trim(), version) {
                Some(true) => return Some(true),
                Some(false) => continue,
                None => return None,
            }
        }
        return Some(false);
    }

    if lowered.contains(" and ") {
        let parts = split_on_keyword(constraint, " and ");
        for part in &parts {
            match hex_version_satisfies(part.trim(), version) {
                Some(true) => continue,
                Some(false) => return Some(false),
                None => return None,
            }
        }
        return Some(true);
    }

    satisfies_hex_comparator(constraint.trim(), version)
}

/// Split a string on a keyword, case-insensitively.
fn split_on_keyword(s: &str, keyword: &str) -> Vec<String> {
    let lowered = s.to_ascii_lowercase();
    let keyword_lower = keyword.to_ascii_lowercase();
    let mut result = Vec::new();
    let mut last = 0;
    for (idx, _) in lowered.match_indices(&keyword_lower) {
        result.push(s[last..idx].to_string());
        last = idx + keyword.len();
    }
    result.push(s[last..].to_string());
    result
}

fn satisfies_hex_comparator(comp: &str, version: (u64, u64, u64)) -> Option<bool> {
    let comp = comp.trim();
    if comp.is_empty() {
        return None;
    }

    if let Some(rest) = comp.strip_prefix("~>") {
        return satisfies_pessimistic(rest.trim(), version);
    }

    if let Some(rest) = comp.strip_prefix("!=") {
        let target = parse_hex_version(rest.trim())?;
        return Some(version != target);
    }

    if let Some(rest) = comp.strip_prefix("==") {
        let target = parse_hex_version(rest.trim())?;
        return Some(version == target);
    }

    if let Some(rest) = comp.strip_prefix(">=") {
        let target = parse_hex_version(rest.trim())?;
        return Some(version >= target);
    }

    if let Some(rest) = comp.strip_prefix("<=") {
        let target = parse_hex_version(rest.trim())?;
        return Some(version <= target);
    }

    if let Some(rest) = comp.strip_prefix('>') {
        let target = parse_hex_version(rest.trim())?;
        return Some(version > target);
    }

    if let Some(rest) = comp.strip_prefix('<') {
        let target = parse_hex_version(rest.trim())?;
        return Some(version < target);
    }

    if let Some(rest) = comp.strip_prefix('=') {
        let target = parse_hex_version(rest.trim())?;
        return Some(version == target);
    }

    // Bare version (exact match)
    let target = parse_hex_version(comp)?;
    Some(version == target)
}

/// Evaluate the `~>` (pessimistic/compatibility) operator.
///
/// - `~> X.Y` (2 parts): `>= X.Y.0 and < (X+1).0.0`
/// - `~> X.Y.Z` (3 parts): `>= X.Y.Z and < X.(Y+1).0`
fn satisfies_pessimistic(version_str: &str, version: (u64, u64, u64)) -> Option<bool> {
    let s = version_str.trim();
    let parts: Vec<&str> = s.split('.').collect();
    match parts.len() {
        2 => {
            let major: u64 = parts[0].parse().ok()?;
            let minor: u64 = parts[1].parse().ok()?;
            let lower = (major, minor, 0);
            let upper = (major + 1, 0, 0);
            Some(version >= lower && version < upper)
        }
        3 => {
            let major: u64 = parts[0].parse().ok()?;
            let minor: u64 = parts[1].parse().ok()?;
            let patch: u64 = parts[2].parse().ok()?;
            let lower = (major, minor, patch);
            let upper = (major, minor + 1, 0);
            Some(version >= lower && version < upper)
        }
        _ => None,
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
    if is_gleam_manifest(manifest_path) {
        gleam::update_manifest_versions(manifest_path, input, new_pkg_version, new_version_by_name)
    } else {
        mix::update_manifest_versions(manifest_path, input, new_pkg_version, new_version_by_name)
    }
}

/// Recompute a Hex version requirement (`~> 1.2`, `>= 1.0`, or a bare pin) against a
/// new dependency version. Returns `None` when the requirement should be left as-is
/// (conjunctions, unparseable, or already current). The syntax is registry-level and
/// identical for Elixir and Gleam.
fn compute_requirement(old: &str, new_version: &str) -> Option<String> {
    let trimmed = old.trim();
    if trimmed == new_version {
        return None;
    }

    if contains_requirement_conjunction(trimmed) {
        return None;
    }

    const OPERATORS: [&str; 8] = ["~>", "==", "!=", ">=", "<=", ">", "<", "="];
    for op in OPERATORS {
        if let Some(rest) = trimmed.strip_prefix(op) {
            let current = rest.trim_start();
            if current.is_empty() {
                return None;
            }
            if !is_single_version_token(current) {
                return None;
            }
            if current == new_version {
                return None;
            }
            return Some(format!("{} {}", op, new_version).trim().to_string());
        }
    }

    if is_single_version_token(trimmed) {
        return Some(new_version.to_string());
    }

    None
}

fn contains_requirement_conjunction(input: &str) -> bool {
    let lowered = input.to_ascii_lowercase();
    lowered.contains(" and ") || lowered.contains(" or ")
}

fn is_single_version_token(candidate: &str) -> bool {
    !candidate.is_empty()
        && !candidate.contains(char::is_whitespace)
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Resolve `.` and `..` components without touching the filesystem, so package
/// directories compare equal regardless of how their path was spelled.
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => out.push(component),
        }
    }
    out
}

fn has_flag(args: &[String], flag: &str) -> bool {
    let prefix = format!("{flag}=");
    for arg in args {
        if arg == flag || arg.starts_with(&prefix) {
            return true;
        }
    }
    false
}

fn format_command_display(cmd: &Command) -> String {
    let mut text = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        text.push(' ');
        text.push_str(&arg.to_string_lossy());
    }
    text
}

#[cfg(test)]
mod hex_tests;
