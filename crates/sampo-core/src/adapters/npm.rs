use crate::errors::{Result, SampoError, WorkspaceError};
use crate::process::command;
use crate::types::{PackageInfo, PackageKind};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_NPM_REGISTRY: &str = "https://registry.npmjs.org/";
const REGISTRY_RATE_LIMIT: Duration = Duration::from_millis(300);

static REGISTRY_LAST_CALL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

#[derive(Debug, Clone, Default)]
struct NpmPublishConfig {
    registry: Option<String>,
    access: Option<String>,
    tag: Option<String>,
}

#[derive(Debug, Clone)]
struct NpmManifestInfo {
    name: String,
    #[allow(dead_code)]
    version: Option<String>,
    private: bool,
    package_manager: Option<String>,
    publish_config: NpmPublishConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

pub(super) struct NpmAdapter;

impl NpmAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        root.join("package.json").exists() || root.join("pnpm-workspace.yaml").exists()
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        discover_npm(root)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        package_dir.join("package.json")
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        let manifest = load_package_json(manifest_path)?;
        let info = parse_manifest_info(manifest_path, &manifest)?;
        if info.private { Ok(false) } else { Ok(true) }
    }

    pub(super) fn version_exists(
        &self,
        package_name: &str,
        version: &str,
        manifest_path: Option<&Path>,
    ) -> Result<bool> {
        match manifest_path {
            Some(path) => {
                let manifest = load_package_json(path)?;
                let info = parse_manifest_info(path, &manifest)?;
                version_exists_on_registry(
                    package_name,
                    version,
                    info.publish_config.registry.as_deref(),
                )
            }
            None => version_exists_on_registry(package_name, version, None),
        }
    }

    pub(super) fn publish(
        &self,
        manifest_path: &Path,
        dry_run: bool,
        extra_args: &[String],
    ) -> Result<()> {
        let manifest_dir = manifest_path.parent().ok_or_else(|| {
            SampoError::Publish(format!(
                "Manifest {} does not have a parent directory",
                manifest_path.display()
            ))
        })?;
        let manifest = load_package_json(manifest_path)?;
        let info = parse_manifest_info(manifest_path, &manifest)?;

        if info.private {
            return Err(SampoError::Publish(format!(
                "Package '{}' is marked as private and cannot be published",
                info.name
            )));
        }

        let manager = detect_package_manager(manifest_dir, &info);
        let manager_name = match manager {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun",
        };
        let mut cmd = command(manager_name);
        cmd.arg("publish");
        cmd.current_dir(manifest_dir);

        if dry_run && !has_flag(extra_args, "--dry-run") {
            cmd.arg("--dry-run");
        }

        if let Some(registry) = info.publish_config.registry.as_deref()
            && !has_flag(extra_args, "--registry")
        {
            cmd.arg("--registry").arg(registry);
        }

        if !has_flag(extra_args, "--access") {
            if let Some(access) = info.publish_config.access.as_deref() {
                cmd.arg("--access").arg(access);
            } else if info.name.starts_with('@') {
                cmd.arg("--access").arg("public");
            }
        }

        if let Some(tag) = info.publish_config.tag.as_deref()
            && !has_flag(extra_args, "--tag")
        {
            cmd.arg("--tag").arg(tag);
        }

        if !extra_args.is_empty() {
            cmd.args(extra_args);
        }

        println!("Running: {}", format_command_display(&cmd));

        let status = cmd.status().map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SampoError::Publish(format!(
                    "{} not found in PATH; ensure {} is installed to publish packages",
                    manager_name, manager_name
                ))
            } else {
                SampoError::Io(err)
            }
        })?;
        if !status.success() {
            return Err(SampoError::Publish(format!(
                "{} publish failed for {} (package '{}') with status {}",
                manager_name,
                manifest_path.display(),
                info.name,
                status
            )));
        }

        Ok(())
    }

    pub(super) fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        regenerate_npm_lockfile(workspace_root)
    }
}

/// Check if a new version satisfies the npm dependency constraint found in `package.json`.
///
/// Reads the real constraint from the manifest rather than relying on the passed-in
/// `_current_constraint` (which may be a placeholder like `"*"`).
pub(super) fn check_dependency_constraint(
    manifest_path: &Path,
    dep_name: &str,
    _current_constraint: &str,
    new_version: &str,
) -> Result<crate::types::ConstraintCheckResult> {
    use crate::types::ConstraintCheckResult;

    let manifest = load_package_json(manifest_path)?;

    let constraint = match find_dependency_constraint(&manifest, dep_name) {
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

    if trimmed == "*" {
        return Ok(ConstraintCheckResult::Satisfied);
    }

    if trimmed.starts_with("workspace:") {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "workspace protocol".to_string(),
        });
    }

    for prefix in ["file:", "link:", "npm:", "git:", "http:", "https:"] {
        if trimmed.starts_with(prefix) {
            return Ok(ConstraintCheckResult::Skipped {
                reason: format!("'{}' protocol", prefix.trim_end_matches(':')),
            });
        }
    }

    if new_version.contains('-') {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pre-release version".to_string(),
        });
    }

    if constraint_contains_prerelease(trimmed) {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pre-release constraint".to_string(),
        });
    }

    if is_pinned_version(trimmed) {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pinned version".to_string(),
        });
    }

    match npm_version_satisfies(trimmed, new_version) {
        Some(true) => Ok(ConstraintCheckResult::Satisfied),
        Some(false) => Ok(ConstraintCheckResult::NotSatisfied {
            constraint: trimmed.to_string(),
            new_version: new_version.to_string(),
        }),
        None => Ok(ConstraintCheckResult::Skipped {
            reason: format!("unparseable constraint '{}'", trimmed),
        }),
    }
}

/// Look up the constraint string for `dep_name` across all npm dependency sections.
fn find_dependency_constraint(manifest: &JsonValue, dep_name: &str) -> Option<String> {
    for key in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(deps) = manifest.get(key).and_then(JsonValue::as_object)
            && let Some(value) = deps.get(dep_name).and_then(JsonValue::as_str)
        {
            return Some(value.to_string());
        }
    }
    None
}

/// Heuristic: a constraint contains a pre-release tag when a digit is followed by `-`
/// then an alphanumeric character (e.g. `1.0.0-beta`), excluding hyphen-range separators
/// which always have surrounding spaces.
fn constraint_contains_prerelease(constraint: &str) -> bool {
    let bytes = constraint.as_bytes();
    for i in 1..bytes.len().saturating_sub(1) {
        if bytes[i] == b'-' && bytes[i - 1].is_ascii_digit() && bytes[i + 1].is_ascii_alphanumeric()
        {
            return true;
        }
    }
    false
}

/// A pinned version is a bare `M.m.p` string with no operator or wildcard.
fn is_pinned_version(s: &str) -> bool {
    parse_version(s).is_some()
}

/// Check if an npm semver version satisfies a constraint range string.
///
/// Returns `None` when the constraint or version cannot be parsed.
fn npm_version_satisfies(constraint: &str, version_str: &str) -> Option<bool> {
    let version = parse_version(version_str)?;

    for or_part in constraint.split("||") {
        let trimmed = or_part.trim();
        if trimmed.is_empty() || trimmed == "*" {
            return Some(true);
        }
        match satisfies_comparator_set(trimmed, version) {
            Some(true) => return Some(true),
            Some(false) => continue,
            None => return None,
        }
    }

    Some(false)
}

/// Collapse whitespace between an operator (`>=`, `<=`, `>`, `<`, `^`, `~`, `=`)
/// and the version number so that `>= 1.2.3` becomes `>=1.2.3`.
fn normalize_comparator_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        result.push(ch);
        if matches!(ch, '>' | '<' | '~' | '^' | '=') {
            // Handle two-character operators (>=, <=)
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                i += 1;
                result.push('=');
            }
            // Skip whitespace between operator and version
            while i + 1 < bytes.len() && bytes[i + 1] == b' ' {
                i += 1;
            }
        }
        i += 1;
    }
    result
}

/// Check if `version` satisfies every comparator in a space-separated set (AND semantics).
fn satisfies_comparator_set(set: &str, version: (u64, u64, u64)) -> Option<bool> {
    if let Some(result) = try_hyphen_range(set, version) {
        return Some(result);
    }

    let normalized = normalize_comparator_whitespace(set);
    for comp in normalized.split_whitespace() {
        if !satisfies_comparator(comp, version)? {
            return Some(false);
        }
    }

    Some(true)
}

/// Try to parse `set` as a hyphen range (`A - B`). Returns `None` if the pattern
/// does not match so the caller can fall back to normal comparator parsing.
fn try_hyphen_range(set: &str, version: (u64, u64, u64)) -> Option<bool> {
    let tokens: Vec<&str> = set.split_whitespace().collect();
    if tokens.len() != 3 || tokens[1] != "-" {
        return None;
    }
    let lower = parse_partial_version(tokens[0])?;
    let upper = parse_partial_version(tokens[2])?;

    let lower_full = (lower.0, lower.1.unwrap_or(0), lower.2.unwrap_or(0));
    if version < lower_full {
        return Some(false);
    }

    let upper_ok = match (upper.1, upper.2) {
        (Some(m), Some(p)) => version <= (upper.0, m, p),
        (Some(m), None) => version < (upper.0, m + 1, 0),
        (None, _) => version < (upper.0 + 1, 0, 0),
    };

    Some(upper_ok)
}

/// Evaluate a single comparator (`^`, `~`, `>=`, `>`, `<=`, `<`, `=`, or bare/x-range).
fn satisfies_comparator(comp: &str, version: (u64, u64, u64)) -> Option<bool> {
    let comp = comp.trim();
    if comp.is_empty() || matches!(comp, "*" | "x" | "X") {
        return Some(true);
    }

    if let Some(rest) = comp.strip_prefix('^') {
        let rest = rest.trim();
        if rest.is_empty() || matches!(rest, "*" | "x" | "X") {
            return Some(true);
        }
        let partial = parse_partial_version(rest)?;
        let (lower, upper) = expand_caret(partial.0, partial.1, partial.2);
        return Some(version >= lower && version < upper);
    }

    if let Some(rest) = comp.strip_prefix('~') {
        let rest = rest.trim();
        if rest.is_empty() || matches!(rest, "*" | "x" | "X") {
            return Some(true);
        }
        let partial = parse_partial_version(rest)?;
        let (lower, upper) = expand_tilde(partial.0, partial.1, partial.2);
        return Some(version >= lower && version < upper);
    }

    // >=  (checked before >)
    if let Some(rest) = comp.strip_prefix(">=") {
        let partial = parse_partial_version(rest.trim())?;
        let target = (partial.0, partial.1.unwrap_or(0), partial.2.unwrap_or(0));
        return Some(version >= target);
    }

    // >
    if let Some(rest) = comp.strip_prefix('>') {
        let partial = parse_partial_version(rest.trim())?;
        return Some(match (partial.1, partial.2) {
            (Some(m), Some(p)) => version > (partial.0, m, p),
            (Some(m), None) => version >= (partial.0, m + 1, 0),
            (None, _) => version >= (partial.0 + 1, 0, 0),
        });
    }

    // <=  (checked before <)
    if let Some(rest) = comp.strip_prefix("<=") {
        let partial = parse_partial_version(rest.trim())?;
        return Some(match (partial.1, partial.2) {
            (Some(m), Some(p)) => version <= (partial.0, m, p),
            (Some(m), None) => version < (partial.0, m + 1, 0),
            (None, _) => version < (partial.0 + 1, 0, 0),
        });
    }

    // <
    if let Some(rest) = comp.strip_prefix('<') {
        let partial = parse_partial_version(rest.trim())?;
        let target = (partial.0, partial.1.unwrap_or(0), partial.2.unwrap_or(0));
        return Some(version < target);
    }

    // Explicit =
    if let Some(rest) = comp.strip_prefix('=') {
        let partial = parse_partial_version(rest.trim())?;
        return Some(matches_partial(version, partial.0, partial.1, partial.2));
    }

    // Bare version or x-range
    let partial = parse_partial_version(comp)?;
    Some(matches_partial(version, partial.0, partial.1, partial.2))
}

/// Check if `version` falls within the range described by a partial version.
///
/// A complete `(M, Some(m), Some(p))` requires exact equality.
/// A partial version acts as an x-range (e.g. `1.2` → `>=1.2.0 <1.3.0`).
fn matches_partial(
    version: (u64, u64, u64),
    major: u64,
    minor: Option<u64>,
    patch: Option<u64>,
) -> bool {
    match (minor, patch) {
        (Some(m), Some(p)) => version == (major, m, p),
        (Some(m), None) => version >= (major, m, 0) && version < (major, m + 1, 0),
        (None, _) => version >= (major, 0, 0) && version < (major + 1, 0, 0),
    }
}

/// Expand a caret range (`^M.m.p`) to inclusive lower and exclusive upper bounds.
///
/// The caret allows changes that do not modify the left-most non-zero digit:
/// - `^1.2.3` → `[1.2.3, 2.0.0)`
/// - `^0.2.3` → `[0.2.3, 0.3.0)`
/// - `^0.0.3` → `[0.0.3, 0.0.4)`
fn expand_caret(
    major: u64,
    minor: Option<u64>,
    patch: Option<u64>,
) -> ((u64, u64, u64), (u64, u64, u64)) {
    let m = minor.unwrap_or(0);
    let p = patch.unwrap_or(0);
    let lower = (major, m, p);

    let upper = if major > 0 {
        (major + 1, 0, 0)
    } else if minor.is_none() {
        // ^0 → [0.0.0, 1.0.0)
        (1, 0, 0)
    } else if m > 0 {
        (0, m + 1, 0)
    } else if patch.is_none() {
        // ^0.0 or ^0.0.x → [0.0.0, 0.1.0)
        (0, 1, 0)
    } else {
        // ^0.0.3 → [0.0.3, 0.0.4)
        (0, 0, p + 1)
    };

    (lower, upper)
}

/// Expand a tilde range (`~M.m.p`) to inclusive lower and exclusive upper bounds.
///
/// The tilde pins to the minor version when one is specified, otherwise pins to major:
/// - `~1.2.3` → `[1.2.3, 1.3.0)`
/// - `~1.2`   → `[1.2.0, 1.3.0)`
/// - `~1`     → `[1.0.0, 2.0.0)`
fn expand_tilde(
    major: u64,
    minor: Option<u64>,
    patch: Option<u64>,
) -> ((u64, u64, u64), (u64, u64, u64)) {
    let m = minor.unwrap_or(0);
    let p = patch.unwrap_or(0);
    let lower = (major, m, p);

    let upper = if minor.is_some() {
        (major, m + 1, 0)
    } else {
        (major + 1, 0, 0)
    };

    (lower, upper)
}

/// Parse a complete semver version string (`M.m.p`) into its three numeric components.
///
/// Rejects pre-release tags (contains `-`) and strips build metadata (`+…`).
/// Returns `None` for anything that is not exactly three numeric dot-separated parts.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);

    if s.contains('-') {
        return None;
    }

    let s = s.split('+').next()?;
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    let patch = parts[2].parse::<u64>().ok()?;

    Some((major, minor, patch))
}

/// Parse a version that may have missing or wildcard parts.
///
/// Returns `(major, Option<minor>, Option<patch>)` where `None` indicates a
/// wildcard (`*`, `x`, `X`) or an absent component.
fn parse_partial_version(s: &str) -> Option<(u64, Option<u64>, Option<u64>)> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);

    if s.contains('-') {
        return None;
    }

    let s = s.split('+').next()?;
    if s.is_empty() {
        return None;
    }

    let parts: Vec<&str> = s.split('.').collect();
    match parts.len() {
        1 => {
            if is_wildcard(parts[0]) {
                return Some((0, None, None));
            }
            let major = parts[0].parse::<u64>().ok()?;
            Some((major, None, None))
        }
        2 => {
            let major = parts[0].parse::<u64>().ok()?;
            if is_wildcard(parts[1]) {
                return Some((major, None, None));
            }
            let minor = parts[1].parse::<u64>().ok()?;
            Some((major, Some(minor), None))
        }
        3 => {
            let major = parts[0].parse::<u64>().ok()?;
            if is_wildcard(parts[1]) {
                return Some((major, None, None));
            }
            let minor = parts[1].parse::<u64>().ok()?;
            if is_wildcard(parts[2]) {
                return Some((major, Some(minor), None));
            }
            let patch = parts[2].parse::<u64>().ok()?;
            Some((major, Some(minor), Some(patch)))
        }
        _ => None,
    }
}

fn is_wildcard(s: &str) -> bool {
    matches!(s, "*" | "x" | "X")
}

pub(super) fn publish_dry_run(
    packages: &[(&PackageInfo, &Path)],
    extra_args: &[String],
) -> Result<()> {
    for (package, manifest) in packages {
        NpmAdapter
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

fn parse_manifest_info(manifest_path: &Path, manifest: &JsonValue) -> Result<NpmManifestInfo> {
    let name = manifest
        .get("name")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            SampoError::Publish(format!(
                "Manifest {} is missing a non-empty 'name' field",
                manifest_path.display()
            ))
        })?;
    validate_package_name(name).map_err(|msg| {
        SampoError::Publish(format!(
            "Manifest {} has invalid package name '{}': {}",
            manifest_path.display(),
            name,
            msg
        ))
    })?;

    let version = manifest
        .get("version")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let private = manifest
        .get("private")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);

    if !private && version.is_none() {
        return Err(SampoError::Publish(format!(
            "Manifest {} is missing a non-empty 'version' field",
            manifest_path.display()
        )));
    }

    let package_manager = manifest
        .get("packageManager")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let publish_config = manifest
        .get("publishConfig")
        .and_then(JsonValue::as_object)
        .map(|map| {
            let mut cfg = NpmPublishConfig::default();
            if let Some(registry) = map.get("registry").and_then(JsonValue::as_str) {
                let trimmed = registry.trim();
                if !trimmed.is_empty() {
                    cfg.registry = Some(trimmed.to_string());
                }
            }
            if let Some(access) = map.get("access").and_then(JsonValue::as_str) {
                let trimmed = access.trim();
                if !trimmed.is_empty() {
                    cfg.access = Some(trimmed.to_string());
                }
            }
            if let Some(tag) = map.get("tag").and_then(JsonValue::as_str) {
                let trimmed = tag.trim();
                if !trimmed.is_empty() {
                    cfg.tag = Some(trimmed.to_string());
                }
            }
            cfg
        })
        .unwrap_or_default();

    Ok(NpmManifestInfo {
        name: name.to_string(),
        version,
        private,
        package_manager,
        publish_config,
    })
}

fn validate_package_name(name: &str) -> std::result::Result<(), String> {
    if name.len() > 214 {
        return Err("package name must be 214 characters or fewer".into());
    }
    if name.starts_with('.') || name.starts_with('_') {
        return Err("package name must not start with '.' or '_'".into());
    }
    if name.contains(' ') {
        return Err("package name must not contain spaces".into());
    }
    if name.chars().any(|c| c.is_ascii_uppercase()) {
        return Err("package name must be lowercase".into());
    }

    let (scope_part, pkg_part) = if name.starts_with('@') {
        let (scope, rest) = name
            .split_once('/')
            .ok_or_else(|| "scoped packages must use the form '@scope/name'".to_string())?;
        if scope.len() <= 1 {
            return Err("scope name must not be empty".into());
        }
        (&scope[1..], rest)
    } else {
        ("", name)
    };

    for (label, part) in [("scope", scope_part), ("name", pkg_part)] {
        if part.is_empty() {
            continue;
        }
        if part.starts_with('.') || part.starts_with('_') {
            return Err(format!("{label} must not start with '.' or '_'"));
        }
        if !part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
        {
            return Err(format!(
                "{label} may only contain lowercase letters, digits, '-', '_', or '.'"
            ));
        }
    }

    Ok(())
}

fn version_exists_on_registry(
    package_name: &str,
    version: &str,
    registry_override: Option<&str>,
) -> Result<bool> {
    enforce_registry_rate_limit();

    let base_url = registry_override
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_NPM_REGISTRY);

    let url = build_registry_url(base_url, package_name)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(crate::USER_AGENT)
        .build()
        .map_err(|err| SampoError::Publish(format!("failed to build HTTP client: {}", err)))?;

    let response = client
        .get(url.clone())
        .send()
        .map_err(|err| SampoError::Publish(format!("HTTP request to {} failed: {}", url, err)))?;

    let status = response.status();

    if status == StatusCode::OK {
        let body = response.text().map_err(|err| {
            SampoError::Publish(format!("failed to read registry response: {}", err))
        })?;
        let value: JsonValue = serde_json::from_str(&body)
            .map_err(|err| SampoError::Publish(format!("invalid JSON from {}: {}", url, err)))?;
        let versions = value
            .get("versions")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| {
                SampoError::Publish(format!(
                    "registry response for {} is missing a 'versions' object",
                    package_name
                ))
            })?;
        Ok(versions.contains_key(version))
    } else if status == StatusCode::NOT_FOUND {
        Ok(false)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(|s| format!(" Retry-After: {s}"));
        let msg = format!(
            "Registry {} returned 429 Too Many Requests{}",
            url,
            retry_after.unwrap_or_default()
        );
        Err(SampoError::Publish(msg))
    } else {
        let body = response.text().unwrap_or_default();
        let snippet: String = body.trim().chars().take(400).collect();
        Err(SampoError::Publish(format!(
            "Registry {} returned {}: {}",
            url, status, snippet
        )))
    }
}

fn enforce_registry_rate_limit() {
    let lock = REGISTRY_LAST_CALL.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().unwrap();
    let now = Instant::now();
    if let Some(last) = *guard {
        let elapsed = now.saturating_duration_since(last);
        if elapsed < REGISTRY_RATE_LIMIT {
            thread::sleep(REGISTRY_RATE_LIMIT - elapsed);
        }
    }
    *guard = Some(Instant::now());
}

fn build_registry_url(base: &str, package_name: &str) -> Result<reqwest::Url> {
    let trimmed = if base.trim().is_empty() {
        DEFAULT_NPM_REGISTRY
    } else {
        base.trim()
    };
    let normalized = if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    };
    let base_url = reqwest::Url::parse(&normalized)
        .map_err(|err| SampoError::Publish(format!("invalid registry URL '{}': {}", base, err)))?;
    let encoded = encode_package_name(package_name);
    base_url.join(&encoded).map_err(|err| {
        SampoError::Publish(format!(
            "failed to construct registry URL for '{}': {}",
            package_name, err
        ))
    })
}

fn encode_package_name(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for b in name.bytes() {
        match b {
            b'0'..=b'9' | b'a'..=b'z' | b'-' | b'_' | b'.' | b'~' => encoded.push(b as char),
            b'@' => encoded.push_str("%40"),
            b'/' => encoded.push_str("%2F"),
            other => encoded.push_str(&format!("%{:02X}", other)),
        }
    }
    encoded
}

fn detect_package_manager(dir: &Path, info: &NpmManifestInfo) -> PackageManager {
    if let Some(field) = info.package_manager.as_deref()
        && let Some(manager) = parse_package_manager_field(field)
    {
        return manager;
    }

    for ancestor in dir.ancestors() {
        if ancestor.join("pnpm-lock.yaml").exists() {
            return PackageManager::Pnpm;
        }
        if ancestor.join("bun.lockb").exists() {
            return PackageManager::Bun;
        }
        if ancestor.join("yarn.lock").exists() {
            return PackageManager::Yarn;
        }
        if ancestor.join("package-lock.json").exists()
            || ancestor.join("npm-shrinkwrap.json").exists()
        {
            return PackageManager::Npm;
        }
    }

    PackageManager::Npm
}

fn parse_package_manager_field(field: &str) -> Option<PackageManager> {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (tool, _) = trimmed.split_once('@').unwrap_or((trimmed, ""));
    match tool {
        "pnpm" => Some(PackageManager::Pnpm),
        "npm" => Some(PackageManager::Npm),
        "yarn" => Some(PackageManager::Yarn),
        "bun" => Some(PackageManager::Bun),
        _ => None,
    }
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

/// Regenerate the lockfile for npm-ecosystem packages.
///
/// Detects which package manager is in use (npm, pnpm, yarn, or bun) by examining
/// lockfiles and package.json packageManager field, then runs the appropriate install
/// command to regenerate the lockfile after version updates.
fn regenerate_npm_lockfile(workspace_root: &Path) -> Result<()> {
    let package_manager = detect_workspace_package_manager(workspace_root)?;

    let (program, args, lockfile_name) = match package_manager {
        PackageManager::Npm => (
            "npm",
            vec!["install", "--package-lock-only"],
            "package-lock.json",
        ),
        PackageManager::Pnpm => ("pnpm", vec!["install", "--lockfile-only"], "pnpm-lock.yaml"),
        PackageManager::Yarn => (
            "yarn",
            vec!["install", "--mode", "update-lockfile"],
            "yarn.lock",
        ),
        PackageManager::Bun => (
            "bun",
            vec!["install", "--frozen-lockfile=false"],
            "bun.lockb",
        ),
    };

    println!("Regenerating {} using {}…", lockfile_name, program);

    let mut cmd = command(program);
    cmd.args(&args).current_dir(workspace_root);

    let status = cmd.status().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SampoError::Release(format!(
                "{} not found in PATH; ensure {} is installed to regenerate {}",
                program, program, lockfile_name
            ))
        } else {
            SampoError::Io(err)
        }
    })?;

    if !status.success() {
        return Err(SampoError::Release(format!(
            "{} failed with status {}",
            program, status
        )));
    }

    println!("{} updated.", lockfile_name);
    Ok(())
}

/// Detect which package manager is in use for the workspace.
///
/// Checks for lockfiles and the packageManager field in the root package.json.
/// Returns an error if no package manager can be detected (no lockfile or package.json).
fn detect_workspace_package_manager(workspace_root: &Path) -> Result<PackageManager> {
    // First, check for lockfiles (most reliable indicator)
    if workspace_root.join("pnpm-lock.yaml").exists() {
        return Ok(PackageManager::Pnpm);
    }
    if workspace_root.join("bun.lockb").exists() {
        return Ok(PackageManager::Bun);
    }
    if workspace_root.join("yarn.lock").exists() {
        return Ok(PackageManager::Yarn);
    }
    if workspace_root.join("package-lock.json").exists()
        || workspace_root.join("npm-shrinkwrap.json").exists()
    {
        return Ok(PackageManager::Npm);
    }

    // No lockfile found, try reading packageManager field from root package.json
    let package_json_path = workspace_root.join("package.json");
    if package_json_path.exists() {
        let manifest = load_package_json(&package_json_path)?;
        if let Some(package_manager_field) = manifest
            .get("packageManager")
            .and_then(|v| v.as_str())
            .and_then(parse_package_manager_field)
        {
            return Ok(package_manager_field);
        }
    }

    // If we can't detect a package manager, it's an error since we're in an npm workspace
    Err(SampoError::Release(
        "cannot detect package manager for npm workspace; no lockfile found and no packageManager field in package.json".to_string()
    ))
}

/// Update an npm manifest (`package.json`) by bumping the package version (if provided) and
/// rewriting internal dependency specifiers when a new version is available.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    #[derive(Deserialize)]
    struct PackageJsonBorrowed<'a> {
        #[serde(borrow)]
        version: Option<&'a RawValue>,
        #[serde(borrow)]
        dependencies: Option<HashMap<String, &'a RawValue>>,
        #[serde(borrow, rename = "devDependencies")]
        dev_dependencies: Option<HashMap<String, &'a RawValue>>,
        #[serde(borrow, rename = "peerDependencies")]
        peer_dependencies: Option<HashMap<String, &'a RawValue>>,
        #[serde(borrow, rename = "optionalDependencies")]
        optional_dependencies: Option<HashMap<String, &'a RawValue>>,
    }

    let borrowed: PackageJsonBorrowed = serde_json::from_str(input).map_err(|err| {
        SampoError::Release(format!(
            "Failed to parse package.json {}: {err}",
            manifest_path.display()
        ))
    })?;

    struct Replacement {
        start: usize,
        end: usize,
        replacement: String,
    }

    let mut replacements: Vec<Replacement> = Vec::new();
    let mut applied: Vec<(String, String)> = Vec::new();

    if let Some(target_version) = new_pkg_version {
        let version_raw = borrowed.version.ok_or_else(|| {
            SampoError::Release(format!(
                "Manifest {} is missing a version field",
                manifest_path.display()
            ))
        })?;
        let current: String = serde_json::from_str(version_raw.get()).map_err(|err| {
            SampoError::Release(format!(
                "Version field in {} is not a string: {err}",
                manifest_path.display()
            ))
        })?;
        if current != target_version {
            let (start, end) = raw_span(version_raw, input)?;
            replacements.push(Replacement {
                start,
                end,
                replacement: format!("\"{target_version}\""),
            });
        }
    }

    let sections: [(&str, Option<&HashMap<String, &RawValue>>); 4] = [
        ("dependencies", borrowed.dependencies.as_ref()),
        ("devDependencies", borrowed.dev_dependencies.as_ref()),
        ("peerDependencies", borrowed.peer_dependencies.as_ref()),
        (
            "optionalDependencies",
            borrowed.optional_dependencies.as_ref(),
        ),
    ];

    for (dep_name, new_version) in new_version_by_name {
        let mut updated = false;

        for (section_name, maybe_map) in sections {
            let Some(map) = maybe_map else { continue };
            let Some(raw) = map.get(dep_name.as_str()) else {
                continue;
            };
            let current_spec: String = serde_json::from_str(raw.get()).map_err(|err| {
                SampoError::Release(format!(
                    "Dependency specifier for '{}' in {}.{} is not a string: {err}",
                    dep_name,
                    manifest_path.display(),
                    section_name
                ))
            })?;

            if let Some(new_spec) = compute_dependency_specifier(&current_spec, new_version)
                && new_spec != current_spec
            {
                let (start, end) = raw_span(raw, input)?;
                replacements.push(Replacement {
                    start,
                    end,
                    replacement: format!("\"{new_spec}\""),
                });
                updated = true;
            }
        }

        if updated {
            applied.push((dep_name.clone(), new_version.clone()));
        }
    }

    if replacements.is_empty() {
        return Ok((input.to_string(), applied));
    }

    replacements.sort_by(|a, b| a.start.cmp(&b.start));
    let mut output = input.to_string();
    for replacement in replacements.into_iter().rev() {
        output.replace_range(replacement.start..replacement.end, &replacement.replacement);
    }

    Ok((output, applied))
}

/// Compute the byte span of a `RawValue` within the original JSON source.
fn raw_span(raw: &RawValue, source: &str) -> Result<(usize, usize)> {
    let slice = raw.get();
    let start = unsafe { slice.as_ptr().offset_from(source.as_ptr()) };
    if start < 0 {
        return Err(SampoError::Release(
            "internal error: RawValue is not derived from the provided JSON source".into(),
        ));
    }
    let start = start as usize;
    if start + slice.len() > source.len() {
        return Err(SampoError::Release(
            "internal error: RawValue span exceeds JSON source bounds".into(),
        ));
    }
    let end = start + slice.len();
    Ok((start, end))
}

fn compute_dependency_specifier(old_spec: &str, new_version: &str) -> Option<String> {
    let trimmed = old_spec.trim();
    if trimmed.is_empty() {
        return Some(new_version.to_string());
    }

    if let Some(suffix) = trimmed.strip_prefix("workspace:") {
        return match suffix {
            "*" => None,
            "^" => Some(format!("workspace:^{}", new_version)),
            "~" => Some(format!("workspace:~{}", new_version)),
            "" => Some(format!("workspace:{}", new_version)),
            _ if suffix.starts_with('^') => Some(format!("workspace:^{}", new_version)),
            _ if suffix.starts_with('~') => Some(format!("workspace:~{}", new_version)),
            _ => Some(format!("workspace:{}", new_version)),
        };
    }

    if trimmed == "*" {
        return None;
    }

    for prefix in ["file:", "link:", "npm:", "git:", "http:", "https:"] {
        if trimmed.starts_with(prefix) {
            return None;
        }
    }

    if let Some(rest) = trimmed.strip_prefix('^') {
        if rest == new_version {
            return None;
        }
        return Some(format!("^{}", new_version));
    }

    if let Some(rest) = trimmed.strip_prefix('~') {
        if rest == new_version {
            return None;
        }
        return Some(format!("~{}", new_version));
    }

    if trimmed == new_version {
        return None;
    }

    if trimmed.starts_with('>') || trimmed.starts_with('<') {
        return None;
    }

    Some(new_version.to_string())
}

fn discover_npm(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    let package_json_path = root.join("package.json");
    let root_manifest = if package_json_path.exists() {
        Some(load_package_json(&package_json_path)?)
    } else {
        None
    };

    let mut patterns: BTreeSet<String> = BTreeSet::new();

    if let Some(manifest) = &root_manifest {
        for pattern in extract_workspace_patterns(manifest)? {
            patterns.insert(pattern);
        }
    }

    let pnpm_patterns = load_pnpm_workspace_patterns(&root.join("pnpm-workspace.yaml"))?;
    for pattern in pnpm_patterns {
        patterns.insert(pattern);
    }

    let mut package_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    if patterns.is_empty() {
        if package_json_path.exists() {
            package_dirs.insert(root.to_path_buf());
        }
    } else {
        for pattern in patterns {
            expand_npm_member_pattern(root, &pattern, &mut package_dirs)?;
        }
    }

    if let Some(manifest) = &root_manifest
        && manifest
            .get("name")
            .and_then(JsonValue::as_str)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    {
        package_dirs.insert(root.to_path_buf());
    }

    let mut manifests: Vec<(String, String, PathBuf, JsonValue)> = Vec::new();
    let mut name_to_path: BTreeMap<String, PathBuf> = BTreeMap::new();

    for dir in &package_dirs {
        let manifest_path = dir.join("package.json");
        if !manifest_path.exists() {
            return Err(WorkspaceError::InvalidWorkspace(format!(
                "workspace member '{}' does not contain package.json",
                dir.display()
            )));
        }
        let manifest = load_package_json(&manifest_path)?;
        let name = manifest
            .get("name")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                WorkspaceError::InvalidWorkspace(format!(
                    "missing name field in {}",
                    manifest_path.display()
                ))
            })?
            .to_string();
        let version = manifest
            .get("version")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();

        manifests.push((name.clone(), version, dir.clone(), manifest));
        name_to_path.insert(name, dir.clone());
    }

    let mut packages = Vec::new();
    for (name, version, path, manifest) in manifests {
        let identifier = PackageInfo::dependency_identifier(PackageKind::Npm, &name);
        let internal_deps = collect_internal_deps(&manifest, &name_to_path);
        packages.push(PackageInfo {
            name,
            version,
            path,
            identifier,
            internal_deps,
            kind: PackageKind::Npm,
        });
    }

    Ok(packages)
}

fn load_package_json(path: &Path) -> std::result::Result<JsonValue, WorkspaceError> {
    let text = fs::read_to_string(path)
        .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, path)))?;
    serde_json::from_str(&text)
        .map_err(|e| WorkspaceError::InvalidManifest(format!("{}: {}", path.display(), e)))
}

fn extract_workspace_patterns(
    manifest: &JsonValue,
) -> std::result::Result<Vec<String>, WorkspaceError> {
    let mut patterns = Vec::new();
    if let Some(workspaces) = manifest.get("workspaces") {
        match workspaces {
            JsonValue::Array(items) => {
                for item in items {
                    let pattern = item.as_str().ok_or_else(|| {
                        WorkspaceError::InvalidWorkspace(
                            "workspaces entries must be strings".into(),
                        )
                    })?;
                    patterns.push(pattern.to_string());
                }
            }
            JsonValue::Object(map) => {
                if let Some(packages) = map.get("packages") {
                    if let JsonValue::Array(items) = packages {
                        for item in items {
                            let pattern = item.as_str().ok_or_else(|| {
                                WorkspaceError::InvalidWorkspace(
                                    "workspaces.packages entries must be strings".into(),
                                )
                            })?;
                            patterns.push(pattern.to_string());
                        }
                    } else if !packages.is_null() {
                        return Err(WorkspaceError::InvalidWorkspace(
                            "workspaces.packages must be an array of strings".into(),
                        ));
                    }
                }
            }
            _ => {
                return Err(WorkspaceError::InvalidWorkspace(
                    "workspaces field must be an array or object".into(),
                ));
            }
        }
    }
    Ok(patterns)
}

fn load_pnpm_workspace_patterns(path: &Path) -> std::result::Result<Vec<String>, WorkspaceError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = fs::read_to_string(path)
        .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, path)))?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text)
        .map_err(|e| WorkspaceError::InvalidManifest(format!("{}: {}", path.display(), e)))?;

    let mut patterns = Vec::new();
    if let Some(packages) = value.get("packages") {
        if let Some(seq) = packages.as_sequence() {
            for item in seq {
                let pattern = item.as_str().ok_or_else(|| {
                    WorkspaceError::InvalidWorkspace(
                        "pnpm-workspace.yaml packages entries must be strings".into(),
                    )
                })?;
                patterns.push(pattern.to_string());
            }
        } else if !packages.is_null() {
            return Err(WorkspaceError::InvalidWorkspace(
                "pnpm-workspace.yaml packages field must be a sequence of strings".into(),
            ));
        }
    }

    Ok(patterns)
}

fn expand_npm_member_pattern(
    root: &Path,
    pattern: &str,
    paths: &mut BTreeSet<PathBuf>,
) -> std::result::Result<(), WorkspaceError> {
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        let full_pattern = root.join(pattern);
        let pattern_str = full_pattern.to_string_lossy().to_string();
        let matches = glob::glob(&pattern_str).map_err(|e| {
            WorkspaceError::InvalidWorkspace(format!(
                "invalid workspace pattern '{}': {}",
                pattern, e
            ))
        })?;
        for entry in matches {
            let path = entry
                .map_err(|e| WorkspaceError::InvalidWorkspace(format!("glob error: {}", e)))?;
            if path.is_dir() {
                if path.join("package.json").exists() {
                    paths.insert(clean_path(&path));
                }
            } else if path
                .file_name()
                .map(|name| name == "package.json")
                .unwrap_or(false)
                && let Some(parent) = path.parent()
            {
                paths.insert(clean_path(parent));
            }
        }
    } else {
        let candidate = clean_path(&root.join(pattern));
        let manifest_path = candidate.join("package.json");
        if manifest_path.exists() {
            paths.insert(candidate);
        } else {
            return Err(WorkspaceError::InvalidWorkspace(format!(
                "workspace member '{}' does not contain package.json",
                pattern
            )));
        }
    }
    Ok(())
}

fn collect_internal_deps(
    manifest: &JsonValue,
    name_to_path: &BTreeMap<String, PathBuf>,
) -> BTreeSet<String> {
    let mut internal = BTreeSet::new();

    for key in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(deps) = manifest.get(key).and_then(JsonValue::as_object) {
            for dep_name in deps.keys() {
                if name_to_path.contains_key(dep_name.as_str()) {
                    internal.insert(PackageInfo::dependency_identifier(
                        PackageKind::Npm,
                        dep_name,
                    ));
                }
            }
        }
    }

    if let Some(array) = manifest
        .get("bundledDependencies")
        .or_else(|| manifest.get("bundleDependencies"))
        .and_then(JsonValue::as_array)
    {
        for dep in array {
            if let Some(dep_name) = dep.as_str()
                && name_to_path.contains_key(dep_name)
            {
                internal.insert(PackageInfo::dependency_identifier(
                    PackageKind::Npm,
                    dep_name,
                ));
            }
        }
    }

    internal
}

fn clean_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    result.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_))
                ) {
                    result.pop();
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                result.push(component);
            }
        }
    }
    result
}

#[cfg(test)]
mod npm_tests;
