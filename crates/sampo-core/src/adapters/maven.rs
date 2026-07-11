use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

mod pom;

const MAVEN_REPO_BASE: &str = "https://repo1.maven.org/maven2";

// repo1.maven.org has no documented request quota (429s only target sustained
// high-volume consumers); keep the same courtesy delay the other registries use.
const MAVEN_RATE_LIMIT: Duration = Duration::from_millis(200);

static MAVEN_LAST_CALL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Stateless adapter for Maven workspaces.
///
/// Maven Central hosts several build tools (Maven, Gradle, sbt, …); this adapter
/// currently supports Maven itself (`pom.xml`), with room for other build tools to
/// slot in as sibling submodules.
pub(super) struct MavenAdapter;

impl MavenAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        pom::can_discover(root)
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        pom::discover(root)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        pom::manifest_path(package_dir)
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        pom::is_publishable(manifest_path)
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
                "Package name cannot be empty when checking the Maven registry".into(),
            ));
        }

        // A package deployed to a private repository isn't on Central; querying there
        // risks a false positive from a same-named public artifact. Let `mvn deploy`
        // own the idempotent re-run.
        if let Some(path) = manifest_path
            && pom::has_private_deploy_repository(path)
        {
            return Ok(false);
        }

        let (group_id, artifact_id) = split_coordinates(name)?;

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(crate::USER_AGENT)
            .build()
            .map_err(|e| {
                SampoError::Publish(format!("failed to build HTTP client for Maven: {}", e))
            })?;

        let url = registry_url(group_id, artifact_id, version);
        enforce_maven_rate_limit();

        let response = client.get(&url).send().map_err(|e| {
            SampoError::Publish(format!(
                "failed to query the Maven registry for '{}': {}",
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
                    "Maven registry returned 429 Too Many Requests for '{}@{}'.{}",
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
                    "Maven registry returned {} for '{}@{}'{}",
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
        pom::publish(manifest_path, dry_run, extra_args)
    }

    pub(super) fn regenerate_lockfile(&self, _workspace_root: &Path) -> Result<()> {
        // Maven has no lockfile; dependency versions live in the POMs themselves.
        Ok(())
    }
}

/// Split a Sampo Maven package name (`groupId/artifactId`) into its coordinates.
fn split_coordinates(name: &str) -> Result<(&str, &str)> {
    match name.split_once('/') {
        Some((group_id, artifact_id)) if !group_id.is_empty() && !artifact_id.is_empty() => {
            Ok((group_id, artifact_id))
        }
        _ => Err(SampoError::Publish(format!(
            "Invalid Maven package name '{}': expected 'groupId/artifactId'",
            name
        ))),
    }
}

/// The public URL of a release's POM on Maven Central: a 200/404 on this file is the
/// cheapest authoritative "does this version exist" signal.
fn registry_url(group_id: &str, artifact_id: &str, version: &str) -> String {
    format!(
        "{MAVEN_REPO_BASE}/{}/{artifact_id}/{version}/{artifact_id}-{version}.pom",
        group_id.replace('.', "/")
    )
}

fn enforce_maven_rate_limit() {
    let lock = MAVEN_LAST_CALL.get_or_init(|| Mutex::new(None));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    if let Some(last_call) = *guard {
        let elapsed = now.saturating_duration_since(last_call);
        if elapsed < MAVEN_RATE_LIMIT {
            thread::sleep(MAVEN_RATE_LIMIT - elapsed);
        }
    }
    *guard = Some(now);
}

pub(super) fn publish_dry_run(
    packages: &[(&PackageInfo, &Path)],
    extra_args: &[String],
) -> Result<()> {
    for (package, manifest) in packages {
        MavenAdapter
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

/// Update a Maven POM with a new package version and refreshed dependency references.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    pom::update_manifest_versions(manifest_path, input, new_pkg_version, new_version_by_name)
}

pub(super) fn check_dependency_constraint(
    manifest_path: &Path,
    dep_name: &str,
    _current_constraint: &str,
    _new_version: &str,
) -> Result<crate::types::ConstraintCheckResult> {
    use crate::types::ConstraintCheckResult;

    let dependency_value = pom::find_dependency_constraint_value(manifest_path, dep_name)?;
    let Some(value) = dependency_value else {
        return Ok(ConstraintCheckResult::Skipped {
            reason: format!("dependency '{}' not found in manifest", dep_name),
        });
    };

    let trimmed = value.trim();
    // Maven dependency versions are not constraints: a plain version is a "soft"
    // requirement resolved by mediation, `${…}` tracks a property, and ranges express
    // an intent Sampo should not second-guess. The release rewrite keeps literal pins
    // current, so there is nothing to validate here.
    let reason = if trimmed.contains("${") {
        "property-managed version"
    } else if trimmed.starts_with('[') || trimmed.starts_with('(') {
        "version range"
    } else {
        "pinned version"
    };
    Ok(ConstraintCheckResult::Skipped {
        reason: reason.to_string(),
    })
}

/// Version-coupling groups derived from the POM tree: a module inheriting its `<version>`
/// from a parent POM is locked to that parent's version, so the two must release together.
/// Emitted as `[child, parent]` pairs for the caller to union into clusters.
pub(super) fn implicit_fixed_groups(members: &[&PackageInfo]) -> Vec<Vec<String>> {
    let member_names: BTreeSet<&str> = members.iter().map(|m| m.name.as_str()).collect();

    let mut groups = Vec::new();
    for member in members {
        let manifest = pom::manifest_path(&member.path);
        let Some(link) = pom::version_link(&manifest) else {
            continue;
        };
        // A module with its own `<version>` is released independently, even with a parent.
        if !link.inherits {
            continue;
        }
        let Some(parent_key) = link.parent_key else {
            continue;
        };
        if member_names.contains(parent_key.as_str()) {
            let parent_id = PackageInfo::dependency_identifier(PackageKind::Maven, &parent_key);
            groups.push(vec![member.identifier.clone(), parent_id]);
        }
    }
    groups
}

/// Fail before any manifest is written when a version-inheriting module cannot be
/// released consistently: its parent must be in the same batch at the same version.
pub(super) fn validate_release_plan(
    members: &[PackageInfo],
    new_version_by_id: &BTreeMap<String, String>,
) -> Result<()> {
    let member_names: BTreeSet<&str> = members
        .iter()
        .filter(|m| m.kind == PackageKind::Maven)
        .map(|m| m.name.as_str())
        .collect();

    for member in members.iter().filter(|m| m.kind == PackageKind::Maven) {
        let Some(target) = new_version_by_id.get(&member.identifier) else {
            continue;
        };
        let manifest = pom::manifest_path(&member.path);
        let Some(link) = pom::version_link(&manifest) else {
            continue;
        };
        if !link.inherits {
            continue;
        }

        let parent_key = link
            .parent_key
            .as_deref()
            .filter(|k| member_names.contains(k));
        let Some(parent_key) = parent_key else {
            return Err(SampoError::Release(format!(
                "'{}' inherits its version from a parent POM outside this workspace; declare \
                 an explicit <version> to release it independently",
                member.name
            )));
        };
        let parent_id = PackageInfo::dependency_identifier(PackageKind::Maven, parent_key);
        match new_version_by_id.get(&parent_id) {
            Some(parent_version) if parent_version == target => {}
            Some(parent_version) => {
                return Err(SampoError::Release(format!(
                    "'{}' inherits its version from '{}', but is planned for {} while the \
                     parent releases {}; align them (e.g. via a fixed group) or declare an \
                     explicit <version>",
                    member.name, parent_key, target, parent_version
                )));
            }
            None => {
                return Err(SampoError::Release(format!(
                    "'{}' inherits its version from '{}', which is not part of this release \
                     (unchanged or ignored); release the parent together with it or declare \
                     an explicit <version>",
                    member.name, parent_key
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod maven_tests;
