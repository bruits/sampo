use crate::errors::{Result, SampoError, WorkspaceError};
use crate::process::command;
use crate::types::{PackageInfo, PackageKind};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::Value as JsonValue;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const COMPOSER_MANIFEST: &str = "composer.json";
const PACKAGIST_API_BASE: &str = "https://packagist.org/packages";

// Packagist doesn't have strict rate limits for public API, but we add a small delay for courtesy
const PACKAGIST_RATE_LIMIT: Duration = Duration::from_millis(200);

static PACKAGIST_LAST_CALL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Stateless adapter for Packagist/Composer packages.
///
/// Packagist auto-updates from VCS tags, but Composer only recognizes `vX.Y.Z` format.
/// Use `git.short_tags` config for compatibility (see README). Monorepos with multiple
/// Packagist packages are not supported due to this tag format constraint.
pub(super) struct PackagistAdapter;

impl PackagistAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        root.join(COMPOSER_MANIFEST).exists()
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        discover_packagist(root)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        package_dir.join(COMPOSER_MANIFEST)
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        let text = fs::read_to_string(manifest_path)
            .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
        let manifest: JsonValue = serde_json::from_str(&text).map_err(|e| {
            SampoError::Publish(format!(
                "Invalid JSON in {}: {}",
                manifest_path.display(),
                e
            ))
        })?;

        // Check for required fields
        let name = manifest
            .get("name")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let Some(name) = name else {
            return Err(SampoError::Publish(format!(
                "Manifest {} is missing a 'name' field",
                manifest_path.display()
            )));
        };

        // Validate vendor/package format
        if !name.contains('/') {
            return Err(SampoError::Publish(format!(
                "Manifest {} has invalid package name '{}': must be in 'vendor/package' format",
                manifest_path.display(),
                name
            )));
        }

        // Require version field for publishing (Composer allows omitting it, but Sampo needs
        // a version to create tags and track releases)
        let version = manifest
            .get("version")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if version.is_none() {
            return Ok(false);
        }

        // Check if package is abandoned
        if let Some(abandoned) = manifest.get("abandoned")
            && (abandoned.as_bool() == Some(true) || abandoned.is_string())
        {
            return Ok(false);
        }

        Ok(true)
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
                "Package name cannot be empty when checking Packagist registry".into(),
            ));
        }

        enforce_packagist_rate_limit();

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(crate::USER_AGENT)
            .build()
            .map_err(|e| {
                SampoError::Publish(format!("failed to build HTTP client for Packagist: {}", e))
            })?;

        let url = format!("{}/{}.json", PACKAGIST_API_BASE, name);

        let response = client.get(&url).send().map_err(|e| {
            SampoError::Publish(format!(
                "failed to query Packagist registry for '{}': {}",
                name, e
            ))
        })?;

        let status_code = response.status();
        match status_code {
            StatusCode::OK => {
                let body = response.text().map_err(|e| {
                    SampoError::Publish(format!("failed to read Packagist response: {}", e))
                })?;
                let json: JsonValue = serde_json::from_str(&body).map_err(|e| {
                    SampoError::Publish(format!("invalid JSON from Packagist: {}", e))
                })?;

                // Check if the specific version exists in package.versions
                let versions = json
                    .get("package")
                    .and_then(|p| p.get("versions"))
                    .and_then(JsonValue::as_object)
                    .ok_or_else(|| {
                        SampoError::Publish(format!(
                            "Packagist response for '{}' is missing package.versions object",
                            name
                        ))
                    })?;

                // Packagist versions may be prefixed with 'v' (e.g., "v1.0.0" or "1.0.0")
                let version_key = format!("v{}", version);
                Ok(versions.contains_key(version) || versions.contains_key(&version_key))
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
                    "Packagist registry returned 429 Too Many Requests for '{}@{}'.{}",
                    name, version, retry_after
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
                    "Packagist registry returned {} for '{}@{}'{}",
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
        let manifest_dir = manifest_path.parent().ok_or_else(|| {
            SampoError::Publish(format!(
                "Manifest {} does not have a parent directory",
                manifest_path.display()
            ))
        })?;

        let text = fs::read_to_string(manifest_path)
            .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
        let manifest: JsonValue = serde_json::from_str(&text).map_err(|e| {
            SampoError::Publish(format!(
                "Invalid JSON in {}: {}",
                manifest_path.display(),
                e
            ))
        })?;

        let package = manifest
            .get("name")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                SampoError::Publish(format!(
                    "Manifest {} is missing a 'name' field",
                    manifest_path.display()
                ))
            })?;

        // Packagist is VCS-based: it auto-updates when you push git tags.
        // The "publish" step validates the package structure.
        let mut cmd = command("composer");
        cmd.current_dir(manifest_dir);
        cmd.arg("validate");

        if !extra_args.is_empty() {
            cmd.args(extra_args);
        }

        println!("Running: {}", format_command_display(&cmd));

        let status = cmd.status().map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SampoError::Publish(
                    "composer not found in PATH; install Composer to validate packages".to_string(),
                )
            } else {
                SampoError::Io(err)
            }
        })?;

        if !status.success() {
            return Err(SampoError::Publish(format!(
                "composer validate failed for {} (package '{}') with status {}",
                manifest_path.display(),
                package,
                status
            )));
        }

        if dry_run {
            println!(
                "Dry-run: package '{}' validated. Packagist will update from VCS when you push a git tag.",
                package
            );
        } else {
            println!(
                "Package '{}' validated. Packagist will update from VCS when you push a git tag.",
                package
            );
        }

        Ok(())
    }

    pub(super) fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        let manifest_path = workspace_root.join(COMPOSER_MANIFEST);
        if !manifest_path.exists() {
            return Err(SampoError::Release(format!(
                "cannot regenerate lockfile; {} not found in {}",
                COMPOSER_MANIFEST,
                workspace_root.display()
            )));
        }

        println!("Regenerating composer.lock…");

        let mut cmd = command("composer");
        cmd.arg("update").arg("--lock").current_dir(workspace_root);

        println!("Running: {}", format_command_display(&cmd));

        let status = cmd.status().map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SampoError::Release(
                    "composer not found in PATH; install Composer to regenerate composer.lock"
                        .to_string(),
                )
            } else {
                SampoError::Io(err)
            }
        })?;

        if !status.success() {
            return Err(SampoError::Release(format!(
                "composer update --lock failed with status {}",
                status
            )));
        }

        println!("composer.lock updated.");
        Ok(())
    }
}

pub(super) fn publish_dry_run(
    packages: &[(&PackageInfo, &Path)],
    extra_args: &[String],
) -> Result<()> {
    for (package, manifest) in packages {
        PackagistAdapter
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

pub(super) fn check_dependency_constraint(
    _manifest_path: &Path,
    _dep_name: &str,
    _current_constraint: &str,
    _new_version: &str,
) -> Result<crate::types::ConstraintCheckResult> {
    Ok(crate::types::ConstraintCheckResult::Skipped {
        reason: "packagist constraint validation not yet implemented".to_string(),
    })
}

fn enforce_packagist_rate_limit() {
    let lock = PACKAGIST_LAST_CALL.get_or_init(|| Mutex::new(None));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    if let Some(last_call) = *guard {
        let elapsed = now.saturating_duration_since(last_call);
        if elapsed < PACKAGIST_RATE_LIMIT {
            thread::sleep(PACKAGIST_RATE_LIMIT - elapsed);
        }
    }
    *guard = Some(now);
}

/// Update a composer.json manifest with a new package version and refreshed dependency requirements.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    #[derive(serde::Deserialize)]
    struct ComposerJsonBorrowed<'a> {
        #[serde(borrow)]
        version: Option<&'a RawValue>,
        #[serde(borrow)]
        require: Option<std::collections::HashMap<String, &'a RawValue>>,
        #[serde(borrow, rename = "require-dev")]
        require_dev: Option<std::collections::HashMap<String, &'a RawValue>>,
    }

    let borrowed: ComposerJsonBorrowed = serde_json::from_str(input).map_err(|err| {
        SampoError::Release(format!(
            "Failed to parse composer.json {}: {err}",
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

    // Update package version
    if let Some(target_version) = new_pkg_version
        && let Some(version_raw) = borrowed.version
    {
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

    // Update dependencies in require and require-dev sections
    let sections: [(&str, Option<&std::collections::HashMap<String, &RawValue>>); 2] = [
        ("require", borrowed.require.as_ref()),
        ("require-dev", borrowed.require_dev.as_ref()),
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

            if let Some(new_spec) = compute_dependency_constraint(&current_spec, new_version)
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

/// Compute a new Composer version constraint based on the old constraint and new version.
fn compute_dependency_constraint(old_spec: &str, new_version: &str) -> Option<String> {
    let trimmed = old_spec.trim();
    if trimmed.is_empty() {
        return Some(format!("^{}", new_version));
    }

    // Skip complex constraints with logical operators
    if trimmed.contains("||") || trimmed.contains(" ") && !trimmed.starts_with('^') {
        return None;
    }

    // Handle caret (^) constraints - most common in Composer
    if let Some(rest) = trimmed.strip_prefix('^') {
        if rest == new_version {
            return None;
        }
        return Some(format!("^{}", new_version));
    }

    // Handle tilde (~) constraints
    if let Some(rest) = trimmed.strip_prefix('~') {
        if rest == new_version {
            return None;
        }
        return Some(format!("~{}", new_version));
    }

    // Handle exact version constraints
    if trimmed == new_version {
        return None;
    }

    // Handle comparison operators
    if trimmed.starts_with(">=")
        || trimmed.starts_with("<=")
        || trimmed.starts_with('>')
        || trimmed.starts_with('<')
    {
        // Don't modify comparison constraints
        return None;
    }

    // Wildcard constraints (e.g., "1.0.*")
    if trimmed.contains('*') {
        return None;
    }

    // Default: use caret constraint for new version
    Some(format!("^{}", new_version))
}

fn discover_packagist(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    let manifest_path = root.join(COMPOSER_MANIFEST);
    if !manifest_path.exists() {
        return Err(WorkspaceError::ManifestNotFound {
            manifest: COMPOSER_MANIFEST,
            path: root.to_path_buf(),
        });
    }

    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, &manifest_path)))?;
    let manifest: JsonValue = serde_json::from_str(&text).map_err(|e| {
        WorkspaceError::InvalidManifest(format!("{}: {}", manifest_path.display(), e))
    })?;

    let name = manifest
        .get("name")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            WorkspaceError::InvalidManifest(format!(
                "missing name field in {}",
                manifest_path.display()
            ))
        })?
        .to_string();

    // Validate vendor/package format
    if !name.contains('/') {
        return Err(WorkspaceError::InvalidManifest(format!(
            "package name '{}' in {} must be in 'vendor/package' format",
            name,
            manifest_path.display()
        )));
    }

    let version = manifest
        .get("version")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();

    let identifier = PackageInfo::dependency_identifier(PackageKind::Packagist, &name);

    // Composer doesn't have native workspace support, so we only return the root package
    let packages = vec![PackageInfo {
        name,
        identifier,
        version,
        path: root.to_path_buf(),
        internal_deps: std::collections::BTreeSet::new(),
        kind: PackageKind::Packagist,
    }];

    Ok(packages)
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
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    #[test]
    fn compute_dependency_constraint_caret() {
        assert_eq!(
            compute_dependency_constraint("^1.0.0", "2.0.0"),
            Some("^2.0.0".to_string())
        );
        assert_eq!(compute_dependency_constraint("^2.0.0", "2.0.0"), None);
    }

    #[test]
    fn compute_dependency_constraint_tilde() {
        assert_eq!(
            compute_dependency_constraint("~1.0.0", "2.0.0"),
            Some("~2.0.0".to_string())
        );
        assert_eq!(compute_dependency_constraint("~2.0.0", "2.0.0"), None);
    }

    #[test]
    fn compute_dependency_constraint_exact() {
        assert_eq!(
            compute_dependency_constraint("1.0.0", "2.0.0"),
            Some("^2.0.0".to_string())
        );
        assert_eq!(compute_dependency_constraint("2.0.0", "2.0.0"), None);
    }

    #[test]
    fn compute_dependency_constraint_skips_complex() {
        // Complex constraints with logical operators should not be modified
        assert_eq!(compute_dependency_constraint(">=1.0 <2.0", "2.0.0"), None);
        assert_eq!(compute_dependency_constraint("^1.0 || ^2.0", "3.0.0"), None);
    }

    #[test]
    fn compute_dependency_constraint_skips_comparison_operators() {
        assert_eq!(compute_dependency_constraint(">=1.0.0", "2.0.0"), None);
        assert_eq!(compute_dependency_constraint("<=2.0.0", "1.0.0"), None);
        assert_eq!(compute_dependency_constraint(">1.0.0", "2.0.0"), None);
        assert_eq!(compute_dependency_constraint("<2.0.0", "1.0.0"), None);
    }

    #[test]
    fn compute_dependency_constraint_skips_wildcard() {
        assert_eq!(compute_dependency_constraint("1.0.*", "1.1.0"), None);
        assert_eq!(compute_dependency_constraint("2.*", "2.1.0"), None);
    }

    #[test]
    fn compute_dependency_constraint_empty_uses_caret() {
        assert_eq!(
            compute_dependency_constraint("", "1.0.0"),
            Some("^1.0.0".to_string())
        );
    }

    #[test]
    fn update_manifest_versions_updates_version() {
        let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require": {}
}"#;

        let new_version_by_name = BTreeMap::new();
        let (output, applied) = update_manifest_versions(
            Path::new("composer.json"),
            input,
            Some("2.0.0"),
            &new_version_by_name,
        )
        .unwrap();

        assert!(output.contains(r#""version": "2.0.0""#));
        assert!(applied.is_empty());
    }

    #[test]
    fn update_manifest_versions_updates_dependencies() {
        let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require": {
        "other/dep": "^1.0.0"
    }
}"#;

        let mut new_version_by_name = BTreeMap::new();
        new_version_by_name.insert("other/dep".to_string(), "2.0.0".to_string());

        let (output, applied) = update_manifest_versions(
            Path::new("composer.json"),
            input,
            None,
            &new_version_by_name,
        )
        .unwrap();

        assert!(output.contains(r#""other/dep": "^2.0.0""#));
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].0, "other/dep");
        assert_eq!(applied[0].1, "2.0.0");
    }

    #[test]
    fn update_manifest_versions_updates_dev_dependencies() {
        let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require-dev": {
        "dev/package": "^1.0.0"
    }
}"#;

        let mut new_version_by_name = BTreeMap::new();
        new_version_by_name.insert("dev/package".to_string(), "3.0.0".to_string());

        let (output, applied) = update_manifest_versions(
            Path::new("composer.json"),
            input,
            None,
            &new_version_by_name,
        )
        .unwrap();

        assert!(output.contains(r#""dev/package": "^3.0.0""#));
        assert_eq!(applied.len(), 1);
    }

    #[test]
    fn update_manifest_versions_preserves_tilde_constraint() {
        let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require": {
        "other/dep": "~1.0.0"
    }
}"#;

        let mut new_version_by_name = BTreeMap::new();
        new_version_by_name.insert("other/dep".to_string(), "2.0.0".to_string());

        let (output, _) = update_manifest_versions(
            Path::new("composer.json"),
            input,
            None,
            &new_version_by_name,
        )
        .unwrap();

        assert!(output.contains(r#""other/dep": "~2.0.0""#));
    }

    #[test]
    fn update_manifest_versions_no_changes_when_same_version() {
        let input = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "require": {
        "other/dep": "^1.0.0"
    }
}"#;

        let mut new_version_by_name = BTreeMap::new();
        new_version_by_name.insert("other/dep".to_string(), "1.0.0".to_string());

        let (output, applied) = update_manifest_versions(
            Path::new("composer.json"),
            input,
            Some("1.0.0"),
            &new_version_by_name,
        )
        .unwrap();

        // No changes when versions are the same
        assert_eq!(output, input);
        assert!(applied.is_empty());
    }

    #[test]
    fn discover_packagist_valid_package() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "name": "vendor/my-package",
    "version": "1.2.3",
    "require": {}
}"#;
        std::fs::write(temp.path().join("composer.json"), manifest).unwrap();

        let packages = discover_packagist(temp.path()).unwrap();
        assert_eq!(packages.len(), 1);

        let pkg = &packages[0];
        assert_eq!(pkg.name, "vendor/my-package");
        assert_eq!(pkg.version, "1.2.3");
        assert_eq!(pkg.kind, PackageKind::Packagist);
        assert_eq!(pkg.identifier, "packagist/vendor/my-package");
    }

    #[test]
    fn discover_packagist_requires_vendor_format() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "name": "my-package",
    "version": "1.0.0"
}"#;
        std::fs::write(temp.path().join("composer.json"), manifest).unwrap();

        let result = discover_packagist(temp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("vendor/package"));
    }

    #[test]
    fn discover_packagist_missing_name() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "version": "1.0.0"
}"#;
        std::fs::write(temp.path().join("composer.json"), manifest).unwrap();

        let result = discover_packagist(temp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("missing name"));
    }

    #[test]
    fn is_publishable_valid_package() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "name": "vendor/package",
    "version": "1.0.0"
}"#;
        let path = temp.path().join("composer.json");
        std::fs::write(&path, manifest).unwrap();

        let result = PackagistAdapter.is_publishable(&path).unwrap();
        assert!(result);
    }

    #[test]
    fn is_publishable_abandoned_package() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "abandoned": true
}"#;
        let path = temp.path().join("composer.json");
        std::fs::write(&path, manifest).unwrap();

        let result = PackagistAdapter.is_publishable(&path).unwrap();
        assert!(!result);
    }

    #[test]
    fn is_publishable_abandoned_with_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "name": "vendor/package",
    "version": "1.0.0",
    "abandoned": "vendor/new-package"
}"#;
        let path = temp.path().join("composer.json");
        std::fs::write(&path, manifest).unwrap();

        let result = PackagistAdapter.is_publishable(&path).unwrap();
        assert!(!result);
    }

    #[test]
    fn is_publishable_missing_vendor_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "name": "package-without-vendor",
    "version": "1.0.0"
}"#;
        let path = temp.path().join("composer.json");
        std::fs::write(&path, manifest).unwrap();

        let result = PackagistAdapter.is_publishable(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("vendor/package"));
    }

    #[test]
    fn is_publishable_missing_version() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "name": "vendor/package"
}"#;
        let path = temp.path().join("composer.json");
        std::fs::write(&path, manifest).unwrap();

        let result = PackagistAdapter.is_publishable(&path).unwrap();
        assert!(!result);
    }

    #[test]
    fn is_publishable_empty_version() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = r#"{
    "name": "vendor/package",
    "version": ""
}"#;
        let path = temp.path().join("composer.json");
        std::fs::write(&path, manifest).unwrap();

        let result = PackagistAdapter.is_publishable(&path).unwrap();
        assert!(!result);
    }
}
