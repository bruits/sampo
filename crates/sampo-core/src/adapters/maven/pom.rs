use crate::adapters::{format_command_display, has_flag};
use crate::errors::{Result, SampoError, WorkspaceError};
use crate::process::command;
use crate::types::{PackageInfo, PackageKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser, Tree};

const POM_FILE: &str = "pom.xml";

/// Maven's mutable dev-version marker. Central rejects it on release deployments, and
/// Sampo would otherwise mistake it for one of its own pre-release cycles, so packages
/// carrying it are skipped at discovery.
const SNAPSHOT_SUFFIX: &str = "-SNAPSHOT";

/// Bound the `<parent>` chain walk when resolving inherited properties, so a
/// pathological or cyclic layout cannot spin.
const MAX_PARENT_HOPS: usize = 10;

pub(super) fn can_discover(root: &Path) -> bool {
    root.join(POM_FILE).is_file()
}

pub(super) fn manifest_path(package_dir: &Path) -> PathBuf {
    package_dir.join(POM_FILE)
}

pub(super) fn discover(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    struct Member {
        name: String,
        version: String,
        dir: PathBuf,
        deps: Vec<(String, bool)>,
        parent_key: Option<String>,
    }

    let mut members: Vec<Member> = Vec::new();
    let mut member_keys: BTreeSet<String> = BTreeSet::new();

    for reactor_pom in collect_reactor_poms(root)? {
        let ReactorPom { dir, path, parsed } = reactor_pom;

        // Skip with a warning rather than aborting: discovery is shared across
        // ecosystems, so a hard error here would also drop healthy members of other
        // ecosystems.
        let Some(artifact_id) = parsed
            .artifact_id
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        else {
            eprintln!(
                "Warning: skipping {}: it declares no <artifactId>",
                path.display()
            );
            continue;
        };

        let Some(group_id) = parsed.effective_group_id() else {
            warn_skip(
                artifact_id,
                &path,
                "it declares no <groupId> (own or inherited from <parent>)",
            );
            continue;
        };

        let name = format!("{group_id}/{artifact_id}");

        let version = match parsed.effective_version() {
            EffectiveVersion::Static(v) => v,
            EffectiveVersion::Unmanageable(raw, reason) => {
                warn_skip(&name, &path, &format!("its version `{raw}` {reason}"));
                continue;
            }
            EffectiveVersion::Absent => {
                warn_skip(&name, &path, "it declares no <version> (own or inherited)");
                continue;
            }
        };

        let parent_key = parsed.parent.as_ref().and_then(ParentRef::key);
        let deps = parsed
            .dependency_keys()
            .into_iter()
            .filter(|(key, _)| key != &name)
            .collect();

        member_keys.insert(name.clone());
        members.push(Member {
            name,
            version,
            dir,
            deps,
            parent_key,
        });
    }

    let mut packages = Vec::new();
    for member in members {
        let mut internal = BTreeSet::new();
        let mut internal_dev = BTreeSet::new();

        // The <parent> reference is a real publish-order dependency: consumers cannot
        // resolve a module whose parent POM is not on the registry yet.
        if let Some(parent_key) = &member.parent_key
            && parent_key != &member.name
            && member_keys.contains(parent_key)
        {
            internal.insert(PackageInfo::dependency_identifier(
                PackageKind::Maven,
                parent_key,
            ));
        }

        for (key, ordering_exempt) in &member.deps {
            if !member_keys.contains(key) {
                continue;
            }
            let identifier = PackageInfo::dependency_identifier(PackageKind::Maven, key);
            // Test-scoped deps never ship in the artifact, and <dependencyManagement>
            // pins are declarative — neither requires the referenced artifact to exist
            // at deploy time, so they must not constrain the publish order (a parent
            // pinning its own modules would otherwise cycle with the children's
            // <parent> edges). They still get version rewrites.
            if *ordering_exempt {
                internal_dev.insert(identifier);
            } else {
                internal.insert(identifier);
            }
        }

        packages.push(PackageInfo {
            identifier: PackageInfo::dependency_identifier(PackageKind::Maven, &member.name),
            name: member.name,
            version: member.version,
            path: member.dir,
            internal_deps: internal,
            internal_dev_deps: internal_dev,
            kind: PackageKind::Maven,
        });
    }

    Ok(packages)
}

pub(super) fn is_publishable(manifest_path: &Path) -> Result<bool> {
    let text = fs::read_to_string(manifest_path)
        .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
    let parsed = parse_pom(&text).ok_or_else(|| {
        SampoError::Publish(format!(
            "Manifest {} is not a valid Maven POM",
            manifest_path.display()
        ))
    })?;

    if parsed
        .artifact_id
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return Err(SampoError::Publish(format!(
            "Manifest {} is missing an <artifactId>",
            manifest_path.display()
        )));
    }

    match parsed.effective_version() {
        EffectiveVersion::Static(_) => {}
        EffectiveVersion::Absent => {
            return Err(SampoError::Publish(format!(
                "Manifest {} is missing a version field",
                manifest_path.display()
            )));
        }
        // Discovery already skips unmanageable versions, so this is defensive.
        EffectiveVersion::Unmanageable(raw, reason) => {
            return Err(SampoError::Publish(format!(
                "Manifest {} cannot be published because its version `{}` {}",
                manifest_path.display(),
                raw,
                reason
            )));
        }
    }

    // `maven.deploy.skip` is the conventional way to keep a module out of `mvn deploy`;
    // honor it as the package's "private" flag.
    Ok(!resolve_deploy_skip(manifest_path, &parsed))
}

/// Whether the POM routes `mvn deploy` somewhere other than Maven Central, via a
/// `<distributionManagement>` release `<repository>` (own or inherited through the
/// in-tree parent chain). The Central Portal plugin needs no such block, so a
/// non-Central URL there signals a private target. `<snapshotRepository>` is ignored:
/// Sampo never publishes snapshots.
pub(super) fn has_private_deploy_repository(manifest_path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(manifest_path) else {
        return false;
    };
    let Some(parsed) = parse_pom(&text) else {
        return false;
    };
    resolve_through_parents(manifest_path, &parsed, |pom| {
        pom.deploy_repository_url
            .as_ref()
            .map(|url| !is_central_url(url))
    })
    .unwrap_or(false)
}

/// Deploy URLs operated by Maven Central: the Portal, the legacy OSSRH staging shim
/// (both under sonatype domains), and the repository hosts themselves.
fn is_central_url(url: &str) -> bool {
    let url = url.trim();
    url.contains("sonatype.")
        || url.contains("repo.maven.apache.org")
        || url.contains("repo1.maven.org")
}

pub(super) fn publish(manifest_path: &Path, dry_run: bool, extra_args: &[String]) -> Result<()> {
    let manifest_dir = manifest_path.parent().ok_or_else(|| {
        SampoError::Publish(format!(
            "Manifest {} does not have a parent directory",
            manifest_path.display()
        ))
    })?;

    let mut cmd = command("mvn");
    cmd.current_dir(manifest_dir);
    cmd.args(publish_args(dry_run, extra_args));

    println!("Running: {}", format_command_display(&cmd));

    let status = cmd.status().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SampoError::Publish(
                "mvn not found in PATH; ensure Apache Maven is installed to publish packages"
                    .to_string(),
            )
        } else {
            SampoError::Io(err)
        }
    })?;

    if !status.success() {
        let action = if dry_run { "mvn verify" } else { "mvn deploy" };
        return Err(SampoError::Publish(format!(
            "{} failed for {} with status {}",
            action,
            manifest_path.display(),
            status
        )));
    }

    Ok(())
}

/// `--non-recursive` is injected because Sampo publishes one package at a time in
/// dependency order: without it, deploying an aggregator POM would re-deploy its entire
/// reactor and collide with the per-module publishes. In multi-module workspaces the
/// sibling artifacts must therefore be available locally (`mvn install` before
/// publishing). `mvn` has no deploy dry-run; `verify` runs the same build — packaging
/// and signing included — while stopping short of the registry upload. Each injected
/// flag is skipped when the user forwards its own.
fn publish_args(dry_run: bool, extra_args: &[String]) -> Vec<String> {
    let mut args = Vec::new();

    if !has_flag(extra_args, "--batch-mode") && !has_flag(extra_args, "-B") {
        args.push("--batch-mode".to_string());
    }

    if !has_flag(extra_args, "--non-recursive") && !has_flag(extra_args, "-N") {
        args.push("--non-recursive".to_string());
    }

    args.push(if dry_run { "verify" } else { "deploy" }.to_string());
    args.extend_from_slice(extra_args);
    args
}

pub(super) fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    let spans = locate_pom_spans(input).ok_or_else(|| {
        SampoError::Release(format!(
            "Manifest {} is not a valid Maven POM",
            manifest_path.display()
        ))
    })?;

    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut updated: BTreeSet<String> = BTreeSet::new();

    if let Some(target) = new_pkg_version {
        match &spans.own_version {
            Some(span) if is_static_version(&span.value) => {
                if span.value != target {
                    edits.push((span.range.clone(), target.to_string()));
                }
            }
            // A build-time property is never spliced; discovery skips such packages,
            // so hitting this means the manifest changed under us.
            Some(span) => {
                return Err(SampoError::Release(format!(
                    "Manifest {} has no static <version> to update (found `{}`)",
                    manifest_path.display(),
                    span.value
                )));
            }
            // The version is inherited: it only moves when the parent's own release
            // rewrites the <parent> block below, so both must land in the same batch
            // with the same number.
            None => {
                let parent = spans.parent.as_ref();
                let parent_target = parent
                    .and_then(|p| p.key.as_ref())
                    .and_then(|key| new_version_by_name.get(key));
                match parent_target {
                    Some(parent_version) if parent_version == target => {}
                    Some(parent_version) => {
                        return Err(SampoError::Release(format!(
                            "Manifest {} inherits its version from its parent POM, but is \
                             planned for {} while the parent releases {}; align them via a \
                             fixed group or declare an explicit <version>",
                            manifest_path.display(),
                            target,
                            parent_version
                        )));
                    }
                    None => {
                        return Err(SampoError::Release(format!(
                            "Manifest {} inherits its version from its parent POM; release \
                             the parent to the same version (e.g. via a fixed group) or \
                             declare an explicit <version>",
                            manifest_path.display()
                        )));
                    }
                }
            }
        }
    }

    if let Some(parent) = &spans.parent
        && let Some(key) = &parent.key
        && let Some(new_version) = new_version_by_name.get(key)
        && let Some(span) = &parent.version
        && is_static_version(&span.value)
        && &span.value != new_version
    {
        edits.push((span.range.clone(), new_version.clone()));
        updated.insert(key.clone());
    }

    for dep in &spans.dependencies {
        let Some(key) = &dep.key else { continue };
        let Some(new_version) = new_version_by_name.get(key) else {
            continue;
        };
        let Some(span) = &dep.version else { continue };
        // `${project.version}` and friends track the new version by themselves, and
        // ranges express an intent Sampo should not overwrite.
        if !is_static_version(&span.value) || &span.value == new_version {
            continue;
        }
        edits.push((span.range.clone(), new_version.clone()));
        updated.insert(key.clone());
    }

    if edits.is_empty() {
        return Ok((input.to_string(), Vec::new()));
    }

    // Splice from the end so earlier byte ranges stay valid.
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut output = input.to_string();
    for (range, replacement) in edits {
        output.replace_range(range, &replacement);
    }

    let applied = updated
        .into_iter()
        .filter_map(|key| {
            new_version_by_name
                .get(&key)
                .cloned()
                .map(|version| (key, version))
        })
        .collect();

    Ok((output, applied))
}

pub(super) fn find_dependency_constraint_value(
    manifest_path: &Path,
    dep_name: &str,
) -> Result<Option<String>> {
    let text = fs::read_to_string(manifest_path)
        .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
    let Some(spans) = locate_pom_spans(&text) else {
        return Ok(None);
    };

    if let Some(parent) = &spans.parent
        && parent.key.as_deref() == Some(dep_name)
    {
        return Ok(parent.version.as_ref().map(|span| span.value.clone()));
    }

    for dep in &spans.dependencies {
        if dep.key.as_deref() == Some(dep_name) {
            return Ok(dep.version.as_ref().map(|span| span.value.clone()));
        }
    }
    Ok(None)
}

/// The version a POM resolves to, before deciding whether Sampo can manage it.
enum EffectiveVersion {
    /// A release literal Sampo can read and bump, e.g. `<version>1.2.3</version>`.
    Static(String),
    /// A version Sampo must not touch; the reason completes "its version `x` …",
    /// phrased for a user-facing warning.
    Unmanageable(String, &'static str),
    /// No `<version>` element, own or in the `<parent>` block.
    Absent,
}

fn classify_version(raw: &str) -> EffectiveVersion {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return EffectiveVersion::Absent;
    }
    if trimmed.contains("${") {
        return EffectiveVersion::Unmanageable(
            trimmed.to_string(),
            "is resolved at build time; pin a static <version> for Sampo to manage it",
        );
    }
    if trimmed.ends_with(SNAPSHOT_SUFFIX) {
        return EffectiveVersion::Unmanageable(
            trimmed.to_string(),
            "is a -SNAPSHOT; Sampo manages static release versions, remove the suffix \
             for Sampo to manage it",
        );
    }
    EffectiveVersion::Static(trimmed.to_string())
}

/// A version literal Sampo may splice: not a `${…}` property, not a `[…]`/`(…]` range.
fn is_static_version(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && !trimmed.contains("${")
        && !trimmed.starts_with('[')
        && !trimmed.starts_with('(')
}

struct ParentRef {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
    /// `<relativePath>` as written: `None` when absent (Maven defaults to
    /// `../pom.xml`), `Some("")` when explicitly emptied (repository-only resolution).
    relative_path: Option<String>,
}

impl ParentRef {
    /// The parent's `group/artifact` key, when both coordinates are literal.
    fn key(&self) -> Option<String> {
        let group = self.group_id.as_deref()?.trim();
        let artifact = self.artifact_id.as_deref()?.trim();
        if group.is_empty() || artifact.is_empty() || group.contains("${") {
            return None;
        }
        Some(format!("{group}/{artifact}"))
    }

    /// The parent's literal groupId, for resolving `${project.parent.groupId}`.
    fn literal_group_id(&self) -> Option<&str> {
        self.group_id
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty() && !g.contains("${"))
    }
}

struct PomDep {
    group_id: Option<String>,
    artifact_id: Option<String>,
    scope: Option<String>,
    /// True for `<dependencyManagement>` entries (declarative pins, no ordering).
    managed: bool,
}

struct ParsedPom {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
    parent: Option<ParentRef>,
    modules: Vec<String>,
    dependencies: Vec<PomDep>,
    /// `maven.deploy.skip` from this POM's own `<properties>`; `None` when unset
    /// (the property may still inherit from a parent POM).
    deploy_skip: Option<bool>,
    /// The `<distributionManagement>` release repository URL declared by this POM;
    /// `None` when the block is absent (it may still inherit from a parent POM).
    deploy_repository_url: Option<String>,
}

impl ParsedPom {
    /// The module's groupId, falling back to the `<parent>` block when absent.
    fn effective_group_id(&self) -> Option<String> {
        self.group_id
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty() && !g.contains("${"))
            .map(str::to_string)
            .or_else(|| {
                self.parent
                    .as_ref()
                    .and_then(ParentRef::literal_group_id)
                    .map(str::to_string)
            })
    }

    /// The module's version, falling back to the `<parent>` block when absent.
    fn effective_version(&self) -> EffectiveVersion {
        let raw = self
            .version
            .as_deref()
            .or_else(|| self.parent.as_ref().and_then(|p| p.version.as_deref()));
        match raw {
            Some(raw) => classify_version(raw),
            None => EffectiveVersion::Absent,
        }
    }

    /// The `group/artifact` keys of the declared dependencies, each with an
    /// ordering-exemption flag (test scope or `<dependencyManagement>` pin).
    fn dependency_keys(&self) -> Vec<(String, bool)> {
        let own_group = self.effective_group_id();
        let parent_group = self
            .parent
            .as_ref()
            .and_then(ParentRef::literal_group_id)
            .map(str::to_string);
        let mut keys = Vec::new();
        for dep in &self.dependencies {
            let Some(key) = dependency_key(
                dep.group_id.as_deref(),
                dep.artifact_id.as_deref(),
                own_group.as_deref(),
                parent_group.as_deref(),
            ) else {
                continue;
            };
            // Import-scoped BOMs inside dependencyManagement are exempt too: a parent
            // importing its own child BOM must not cycle, and the documented
            // `mvn install` prerequisite covers local resolution during the publish run.
            let ordering_exempt =
                dep.managed || dep.scope.as_deref().map(str::trim) == Some("test");
            keys.push((key, ordering_exempt));
        }
        keys
    }

    /// This POM's own `group/artifact` key, used to validate a `<relativePath>`
    /// candidate against the child's declared `<parent>` coordinates.
    fn own_key(&self) -> Option<String> {
        let group = self.effective_group_id()?;
        let artifact = self.artifact_id.as_deref().map(str::trim)?;
        if artifact.is_empty() || artifact.contains("${") {
            return None;
        }
        Some(format!("{group}/{artifact}"))
    }
}

/// Where the in-tree parent POM lives, per `<relativePath>` (default `../pom.xml`).
/// `None` when there is no parent or its resolution is repository-only.
fn parent_pom_location(parsed: &ParsedPom) -> Option<PathBuf> {
    let parent = parsed.parent.as_ref()?;
    match parent.relative_path.as_deref().map(str::trim) {
        Some("") => None,
        Some(rel) => Some(PathBuf::from(rel)),
        None => Some(PathBuf::from("..").join(POM_FILE)),
    }
}

/// Resolve `maven.deploy.skip` for a module: its own `<properties>` win, otherwise the
/// property inherits through the in-tree `<parent>` chain (nearest definition wins).
fn resolve_deploy_skip(manifest_path: &Path, parsed: &ParsedPom) -> bool {
    resolve_through_parents(manifest_path, parsed, |pom| pom.deploy_skip).unwrap_or(false)
}

/// Walk a module's in-tree `<parent>` chain — starting with the module itself — and
/// return the first `Some` produced by `visit` (nearest definition wins, matching
/// Maven's property inheritance).
///
/// Each `<relativePath>` candidate (default `../pom.xml`) is validated against the
/// declared `<parent>` coordinates, as Maven's model builder does: on mismatch the
/// parent is external (repository-resolved) and the walk stops, so an unrelated POM
/// sitting outside the repository is never consulted.
fn resolve_through_parents<T>(
    manifest_path: &Path,
    parsed: &ParsedPom,
    visit: impl Fn(&ParsedPom) -> Option<T>,
) -> Option<T> {
    if let Some(found) = visit(parsed) {
        return Some(found);
    }

    let mut visited: BTreeSet<PathBuf> = BTreeSet::new();
    let mut dir = manifest_path.parent().map(Path::to_path_buf);
    let mut next = parent_pom_location(parsed);
    let mut expected_key = parsed.parent.as_ref().and_then(ParentRef::key);

    for _ in 0..MAX_PARENT_HOPS {
        let (Some(current_dir), Some(rel)) = (dir.as_ref(), next.take()) else {
            break;
        };
        let mut candidate = normalize_path(&current_dir.join(rel));
        // `<relativePath>` may point at the parent's directory instead of its POM.
        if candidate.is_dir() {
            candidate = candidate.join(POM_FILE);
        }
        if !visited.insert(candidate.clone()) {
            break;
        }
        let Ok(text) = fs::read_to_string(&candidate) else {
            break;
        };
        let Some(parent) = parse_pom(&text) else {
            break;
        };
        if expected_key.is_none() || parent.own_key() != expected_key {
            break;
        }
        if let Some(found) = visit(&parent) {
            return Some(found);
        }
        expected_key = parent.parent.as_ref().and_then(ParentRef::key);
        dir = candidate.parent().map(Path::to_path_buf);
        next = parent_pom_location(&parent);
    }

    None
}

struct ReactorPom {
    dir: PathBuf,
    path: PathBuf,
    parsed: ParsedPom,
}

/// Walk the reactor from the root POM, following `<modules>` recursively. Modules only
/// added by build profiles are not walked: they are conditional by design, and Sampo
/// cannot know which profiles apply.
fn collect_reactor_poms(root: &Path) -> std::result::Result<Vec<ReactorPom>, WorkspaceError> {
    let mut out = Vec::new();
    let mut visited: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: Vec<PathBuf> = vec![root.join(POM_FILE)];

    while let Some(pom_path) = queue.pop() {
        let pom_path = normalize_path(&pom_path);
        if !visited.insert(pom_path.clone()) {
            continue;
        }

        let text = match fs::read_to_string(&pom_path) {
            Ok(text) => text,
            Err(err) => {
                // The root POM gated discovery, so it must read; a missing module POM
                // only drops that module.
                if out.is_empty() && queue.is_empty() {
                    return Err(WorkspaceError::Io(crate::errors::io_error_with_path(
                        err, &pom_path,
                    )));
                }
                eprintln!("Warning: skipping module {}: {}", pom_path.display(), err);
                continue;
            }
        };

        let Some(parsed) = parse_pom(&text) else {
            eprintln!(
                "Warning: skipping {}: it is not a valid Maven POM",
                pom_path.display()
            );
            continue;
        };

        let dir = pom_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());

        for module in &parsed.modules {
            let module = module.trim();
            if module.is_empty() {
                continue;
            }
            let target = dir.join(module);
            let module_pom = if target.is_file() {
                // Maven allows a module entry to name the POM file itself, but every
                // Sampo consumer re-resolves the manifest as `<dir>/pom.xml` from
                // `PackageInfo.path` — a custom-named POM would be lost (or worse,
                // shadowed by an unrelated `pom.xml` in the same directory).
                if target.file_name().and_then(|n| n.to_str()) != Some(POM_FILE) {
                    eprintln!(
                        "Warning: skipping module {}: Sampo only manages modules whose \
                         manifest is named `pom.xml`",
                        target.display()
                    );
                    continue;
                }
                target
            } else {
                target.join(POM_FILE)
            };
            queue.push(module_pom);
        }

        out.push(ReactorPom {
            dir,
            path: pom_path,
            parsed,
        });
    }

    // The queue is depth-first in reverse module order; sort for a stable result.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn warn_skip(name: &str, pom_path: &Path, reason: &str) {
    eprintln!(
        "Warning: skipping '{}' ({}): {}",
        name,
        pom_path.display(),
        reason
    );
}

/// The text content of a leaf element, with the byte span Sampo splices on writes.
struct TextSpan {
    range: std::ops::Range<usize>,
    value: String,
}

struct ParentSpans {
    key: Option<String>,
    version: Option<TextSpan>,
}

struct DepSpans {
    key: Option<String>,
    version: Option<TextSpan>,
}

/// Everything `update_manifest_versions` may splice, located in a single parse.
struct PomSpans {
    own_version: Option<TextSpan>,
    parent: Option<ParentSpans>,
    dependencies: Vec<DepSpans>,
}

fn xml_language() -> &'static Language {
    static LANGUAGE: OnceLock<Language> = OnceLock::new();
    LANGUAGE.get_or_init(|| tree_sitter_xml::LANGUAGE_XML.into())
}

fn parse_xml(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(xml_language()).ok()?;
    parser.parse(source, None)
}

/// The document's root element (`<project>` in a POM), skipping the prolog.
fn root_element(tree: &Tree) -> Option<Node<'_>> {
    let mut cursor = tree.root_node().walk();
    tree.root_node()
        .named_children(&mut cursor)
        .find(|n| n.kind() == "element")
}

/// The tag name of an element (from its `STag`, or `EmptyElemTag` for `<empty/>`).
fn element_name<'a>(element: Node<'_>, source: &'a str) -> Option<&'a str> {
    let mut cursor = element.walk();
    let tag = element
        .children(&mut cursor)
        .find(|n| matches!(n.kind(), "STag" | "EmptyElemTag"))?;
    let mut tag_cursor = tag.walk();
    let name = tag.children(&mut tag_cursor).find(|n| n.kind() == "Name")?;
    source.get(name.start_byte()..name.end_byte())
}

/// The child elements of an element (the `element` nodes inside its `content`).
fn child_elements(element: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = element.walk();
    let Some(content) = element
        .children(&mut cursor)
        .find(|n| n.kind() == "content")
    else {
        return Vec::new();
    };
    let mut content_cursor = content.walk();
    content
        .children(&mut content_cursor)
        .filter(|n| n.kind() == "element")
        .collect()
}

fn find_child<'tree>(element: Node<'tree>, source: &str, name: &str) -> Option<Node<'tree>> {
    child_elements(element)
        .into_iter()
        .find(|child| element_name(*child, source) == Some(name))
}

/// The trimmed text of a leaf element. `None` when the content holds anything other
/// than character data (nested elements, comments, entity references): those are not
/// the plain literals Sampo reads or splices.
fn text_span(element: Node<'_>, source: &str) -> Option<TextSpan> {
    let mut cursor = element.walk();
    let content = element
        .children(&mut cursor)
        .find(|n| n.kind() == "content")?;
    let mut content_cursor = content.walk();
    if content
        .children(&mut content_cursor)
        .any(|n| n.kind() != "CharData")
    {
        return None;
    }

    let raw = source.get(content.start_byte()..content.end_byte())?;
    // The content span includes surrounding whitespace; narrow to the trimmed value so
    // a splice preserves the original padding.
    let value = raw.trim();
    let start = content.start_byte() + (raw.len() - raw.trim_start().len());
    Some(TextSpan {
        range: start..start + value.len(),
        value: value.to_string(),
    })
}

fn text_value(element: Node<'_>, source: &str) -> Option<String> {
    text_span(element, source).map(|span| span.value)
}

fn parse_pom(source: &str) -> Option<ParsedPom> {
    let tree = parse_xml(source)?;
    let project = root_element(&tree)?;
    parse_pom_node(project, source)
}

fn parse_pom_node(project: Node<'_>, source: &str) -> Option<ParsedPom> {
    if element_name(project, source) != Some("project") {
        return None;
    }

    let field = |name: &str| find_child(project, source, name).and_then(|n| text_value(n, source));

    let parent = find_child(project, source, "parent").map(|parent| ParentRef {
        group_id: find_child(parent, source, "groupId").and_then(|n| text_value(n, source)),
        artifact_id: find_child(parent, source, "artifactId").and_then(|n| text_value(n, source)),
        version: find_child(parent, source, "version").and_then(|n| text_value(n, source)),
        relative_path: find_child(parent, source, "relativePath")
            .map(|n| text_value(n, source).unwrap_or_default()),
    });

    let modules = find_child(project, source, "modules")
        .map(|modules| {
            child_elements(modules)
                .into_iter()
                .filter(|n| element_name(*n, source) == Some("module"))
                .filter_map(|n| text_value(n, source))
                .collect()
        })
        .unwrap_or_default();

    let dependencies = dependency_elements(project, source)
        .into_iter()
        .map(|(dep, managed)| PomDep {
            group_id: find_child(dep, source, "groupId").and_then(|n| text_value(n, source)),
            artifact_id: find_child(dep, source, "artifactId").and_then(|n| text_value(n, source)),
            scope: find_child(dep, source, "scope").and_then(|n| text_value(n, source)),
            managed,
        })
        .collect();

    let deploy_skip = find_child(project, source, "properties")
        .and_then(|properties| find_child(properties, source, "maven.deploy.skip"))
        .and_then(|n| text_value(n, source))
        .map(|v| v.trim().eq_ignore_ascii_case("true"));

    let deploy_repository_url = find_child(project, source, "distributionManagement")
        .and_then(|management| find_child(management, source, "repository"))
        .map(|repository| {
            find_child(repository, source, "url")
                .and_then(|n| text_value(n, source))
                .unwrap_or_default()
        });

    Some(ParsedPom {
        group_id: field("groupId"),
        artifact_id: field("artifactId"),
        version: field("version"),
        parent,
        modules,
        dependencies,
        deploy_skip,
        deploy_repository_url,
    })
}

/// The `<dependency>` element nodes from `<dependencies>` and
/// `<dependencyManagement><dependencies>`, flagging the managed ones.
fn dependency_elements<'tree>(project: Node<'tree>, source: &str) -> Vec<(Node<'tree>, bool)> {
    let mut lists = Vec::new();
    if let Some(deps) = find_child(project, source, "dependencies") {
        lists.push((deps, false));
    }
    if let Some(management) = find_child(project, source, "dependencyManagement")
        && let Some(deps) = find_child(management, source, "dependencies")
    {
        lists.push((deps, true));
    }

    let mut out = Vec::new();
    for (list, managed) in lists {
        for dep in child_elements(list) {
            if element_name(dep, source) == Some("dependency") {
                out.push((dep, managed));
            }
        }
    }
    out
}

/// Locate every splice target in one parse: the project's own `<version>`, the
/// `<parent>` block, and each dependency's `<version>`.
fn locate_pom_spans(source: &str) -> Option<PomSpans> {
    let tree = parse_xml(source)?;
    let project = root_element(&tree)?;
    // Dependency keys need the parsed identity to resolve `${project.groupId}`.
    let parsed = parse_pom_node(project, source)?;
    let own_group = parsed.effective_group_id();
    let parent_group = parsed
        .parent
        .as_ref()
        .and_then(ParentRef::literal_group_id)
        .map(str::to_string);

    let own_version = find_child(project, source, "version").and_then(|n| text_span(n, source));

    let parent = find_child(project, source, "parent").map(|parent| ParentSpans {
        key: parsed.parent.as_ref().and_then(ParentRef::key),
        version: find_child(parent, source, "version").and_then(|n| text_span(n, source)),
    });

    let dependencies = dependency_elements(project, source)
        .into_iter()
        .map(|(dep, _)| {
            let group = find_child(dep, source, "groupId").and_then(|n| text_value(n, source));
            let artifact =
                find_child(dep, source, "artifactId").and_then(|n| text_value(n, source));
            DepSpans {
                key: dependency_key(
                    group.as_deref(),
                    artifact.as_deref(),
                    own_group.as_deref(),
                    parent_group.as_deref(),
                ),
                version: find_child(dep, source, "version").and_then(|n| text_span(n, source)),
            }
        })
        .collect();

    Some(PomSpans {
        own_version,
        parent,
        dependencies,
    })
}

/// The `group/artifact` key of a dependency. `${project.groupId}` — the common way to
/// reference a sibling — resolves against the module's effective groupId, and
/// `${project.parent.groupId}` (plus its legacy `${parent.groupId}` alias, which Maven
/// still resolves) against the `<parent>` block's; any other property is unresolvable
/// and dropped.
fn dependency_key(
    group: Option<&str>,
    artifact: Option<&str>,
    own_group: Option<&str>,
    parent_group: Option<&str>,
) -> Option<String> {
    let artifact = artifact.map(str::trim).filter(|a| !a.is_empty())?;
    if artifact.contains("${") {
        return None;
    }
    let group = match group.map(str::trim) {
        Some("${project.groupId}") => own_group?.to_string(),
        Some("${project.parent.groupId}") | Some("${parent.groupId}") => parent_group?.to_string(),
        Some(group) if !group.is_empty() && !group.contains("${") => group.to_string(),
        _ => return None,
    };
    Some(format!("{group}/{artifact}"))
}

/// Resolve `.` and `..` components without touching the filesystem, so module paths
/// like `../sibling` compare equal regardless of how they were spelled.
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

#[cfg(test)]
mod pom_tests;
