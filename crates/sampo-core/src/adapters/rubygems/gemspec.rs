use crate::adapters::scan::{LazyScan, ScanIndex};
use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{ConstraintCheckResult, PackageInfo, PackageKind};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser, Tree};

/// Bundler installs (`vendor/bundle`), `rake build` output (`pkg/`) and test fixtures hold
/// gemspecs that are not workspace members.
const RUBY_EXCLUDED_DIRS: &[&str] = &["vendor", "pkg", "spec", "test", "tests"];

const DEPENDENCY_METHODS: &[&str] = &[
    "add_dependency",
    "add_runtime_dependency",
    "add_development_dependency",
];

#[cfg(test)]
pub(super) fn can_discover(root: &Path) -> bool {
    can_discover_scanned(&LazyScan::new(root))
}

pub(super) fn can_discover_scanned(scan: &LazyScan) -> bool {
    !gem_dirs(scan.root(), scan.index()).is_empty()
}

#[cfg(test)]
pub(super) fn discover(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    discover_scanned(&LazyScan::new(root))
}

pub(super) fn discover_scanned(
    scan: &LazyScan,
) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    struct Member {
        name: String,
        version: String,
        dir: PathBuf,
        dependencies: Vec<DependencyCall>,
    }

    let mut members: Vec<Member> = Vec::new();
    for (dir, gemspecs) in gem_dirs(scan.root(), scan.index()) {
        let [gemspec_path] = gemspecs.as_slice() else {
            eprintln!(
                "Warning: skipping {}: it holds several gemspecs, and Sampo manages one gem per \
                 directory",
                dir.display()
            );
            continue;
        };
        let text = fs::read_to_string(gemspec_path)
            .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, gemspec_path)))?;
        // Skip with a warning rather than aborting: discovery is shared across ecosystems,
        // so a hard error here would also drop healthy members of other ecosystems.
        let gemspec = Gemspec::parse(&text);
        let Some(name) = gemspec.name.clone() else {
            eprintln!(
                "Warning: skipping {}: it sets no literal `name`",
                gemspec_path.display()
            );
            continue;
        };
        let literal = match locate_version(gemspec_path, &gemspec, &name) {
            Ok(located) => located.literal,
            Err(reason) => {
                warn_skip(&name, gemspec_path, &reason);
                continue;
            }
        };
        let Some(version) = gem_to_semver(&literal.value) else {
            warn_skip(
                &name,
                gemspec_path,
                &format!(
                    "its version `{}` has no SemVer equivalent (up to three numeric segments, \
                     then an optional pre-release)",
                    literal.value
                ),
            );
            continue;
        };
        members.push(Member {
            name,
            version,
            dir,
            dependencies: gemspec.dependencies,
        });
    }

    let member_names: BTreeSet<String> = members.iter().map(|m| m.name.clone()).collect();
    let mut packages = Vec::new();
    for member in members {
        let mut internal_deps = BTreeSet::new();
        let mut dev_deps = BTreeSet::new();
        for dep in &member.dependencies {
            if dep.name == member.name || !member_names.contains(&dep.name) {
                continue;
            }
            let identifier = PackageInfo::dependency_identifier(PackageKind::RubyGems, &dep.name);
            if dep.development {
                dev_deps.insert(identifier);
            } else {
                internal_deps.insert(identifier);
            }
        }
        // Generated gemspecs declare a dependency in both branches of a
        // `respond_to?(:add_runtime_dependency)` check; a runtime declaration wins.
        dev_deps.retain(|id| !internal_deps.contains(id));

        packages.push(PackageInfo {
            identifier: PackageInfo::dependency_identifier(PackageKind::RubyGems, &member.name),
            name: member.name,
            version: member.version,
            path: member.dir,
            internal_deps,
            internal_dev_deps: dev_deps,
            kind: PackageKind::RubyGems,
        });
    }

    Ok(packages)
}

fn gem_dirs(root: &Path, index: &ScanIndex) -> Vec<(PathBuf, Vec<PathBuf>)> {
    index
        .dirs()
        .filter(|facts| {
            !facts.gemspecs.is_empty() && !has_ruby_excluded_component(root, &facts.dir)
        })
        .map(|facts| (facts.dir.clone(), facts.gemspecs.clone()))
        .collect()
}

fn has_ruby_excluded_component(root: &Path, dir: &Path) -> bool {
    let Ok(rel) = dir.strip_prefix(root) else {
        return false;
    };
    rel.components().any(|component| match component {
        Component::Normal(name) => name
            .to_str()
            .is_some_and(|name| RUBY_EXCLUDED_DIRS.contains(&name)),
        _ => false,
    })
}

/// Resolved without gitignore context, like rebar3's `.app.src`.
pub(super) fn manifest_path(package_dir: &Path) -> PathBuf {
    crate::adapters::scan::find_gemspecs_matching(package_dir, |_| true)
        .into_iter()
        .next()
        .unwrap_or_else(|| {
            // Unreachable for discovered packages (discovery found the file). Fall back to
            // the `bundle gem` convention, where the gemspec is named after the directory.
            let name = package_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("gem");
            package_dir.join(format!("{name}.gemspec"))
        })
}

pub(super) fn is_publishable(manifest_path: &Path) -> Result<bool> {
    let gemspec = read_gemspec(manifest_path)?;
    if gemspec.name.is_none() {
        return Err(SampoError::Publish(format!(
            "Gemspec {} is missing a literal name",
            manifest_path.display()
        )));
    }
    // `bundle gem` scaffolds `allowed_push_host` with a `TODO:` placeholder that `gem push`
    // cannot use: until it is edited or removed, the gem has no publish target.
    Ok(!gemspec
        .allowed_push_host
        .as_deref()
        .is_some_and(|host| host.trim_start().starts_with("TODO")))
}

/// The `allowed_push_host` metadata, when it routes `gem push` away from rubygems.org.
pub(super) fn private_push_host(manifest_path: &Path) -> Option<String> {
    let text = fs::read_to_string(manifest_path).ok()?;
    let host = Gemspec::parse(&text).allowed_push_host?;
    (!is_rubygems_org(&host)).then_some(host)
}

fn is_rubygems_org(url: &str) -> bool {
    let url = url.trim();
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    host == "rubygems.org"
}

/// The version RubyGems will publish for this gemspec, spelled as rubygems.org stores it.
pub(super) fn published_version(manifest_path: &Path, version: &str) -> String {
    let on_disk = read_gemspec(manifest_path).ok().and_then(|gemspec| {
        let name = gemspec.name.clone()?;
        locate_version(manifest_path, &gemspec, &name).ok()
    });
    match on_disk {
        // RubyGems reads a hyphen as `.pre.`: `1.0.0-rc.1` is published as `1.0.0.pre.rc.1`.
        Some(located) if gem_to_semver(&located.literal.value).as_deref() == Some(version) => {
            located.literal.value.replace('-', ".pre.")
        }
        _ => semver_to_gem(version),
    }
}

pub(super) fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    let gemspec = Gemspec::parse(input);

    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    if let Some(target_version) = new_pkg_version {
        match &gemspec.version {
            VersionSource::Literal(literal) => {
                let gem_version = gem_literal(target_version, &literal.value);
                if literal.value != gem_version {
                    replacements.push((literal.start, literal.end, gem_version));
                }
            }
            // The constant's file is rewritten by `finalize_version_files`, once every
            // gemspec has been written.
            VersionSource::Constant(_) => {}
            VersionSource::Absent | VersionSource::Computed => {
                return Err(SampoError::Release(format!(
                    "Gemspec {} has no literal version or version constant to update",
                    manifest_path.display()
                )));
            }
        }
    }

    let mut applied: Vec<(String, String)> = Vec::new();
    for dep in &gemspec.dependencies {
        let Some(new_version) = new_version_by_name.get(&dep.name) else {
            continue;
        };
        let Some(rewrites) = rewrite_requirements(&dep.requirements, new_version) else {
            continue;
        };
        if rewrites.is_empty() {
            continue;
        }
        replacements.extend(rewrites);
        if !applied.iter().any(|(name, _)| name == &dep.name) {
            applied.push((dep.name.clone(), new_version.clone()));
        }
    }

    // Splice back to front so earlier offsets stay valid.
    replacements.sort_by_key(|(start, _, _)| *start);
    let mut output = input.to_string();
    for (start, end, text) in replacements.into_iter().rev() {
        output.replace_range(start..end, &text);
    }
    Ok((output, applied))
}

/// Write each released gem's version into the file holding its version constant, after
/// the gemspecs themselves were rewritten.
pub(super) fn finalize_version_files(
    members: &[PackageInfo],
    new_version_by_id: &BTreeMap<String, String>,
) -> Result<()> {
    for member in members
        .iter()
        .filter(|member| member.kind == PackageKind::RubyGems)
    {
        let Some(new_version) = new_version_by_id.get(member.canonical_identifier()) else {
            continue;
        };
        let gemspec_path = manifest_path(&member.path);
        let gemspec = read_gemspec(&gemspec_path)?;
        if !matches!(gemspec.version, VersionSource::Constant(_)) {
            continue;
        }
        let located = locate_version(&gemspec_path, &gemspec, &member.name).map_err(|reason| {
            SampoError::Release(format!(
                "Cannot write version {new_version} for '{}': {reason}",
                member.name
            ))
        })?;
        let literal = &located.literal;
        let gem_version = gem_literal(new_version, &literal.value);
        if literal.value == gem_version {
            continue;
        }
        let mut text = fs::read_to_string(&located.path)
            .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, &located.path)))?;
        text.replace_range(literal.start..literal.end, &gem_version);
        fs::write(&located.path, text)
            .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, &located.path)))?;
    }
    Ok(())
}

pub(super) fn check_dependency_constraint(
    manifest_path: &Path,
    dep_name: &str,
    new_version: &str,
) -> Result<ConstraintCheckResult> {
    let gemspec = read_gemspec(manifest_path)?;
    let calls: Vec<&DependencyCall> = gemspec
        .dependencies
        .iter()
        .filter(|dep| dep.name == dep_name)
        .collect();
    if calls.is_empty() {
        return Ok(ConstraintCheckResult::Skipped {
            reason: format!("dependency '{dep_name}' not found in gemspec"),
        });
    }
    if is_prerelease(new_version) {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pre-release version".to_string(),
        });
    }

    let new_gem_version = semver_to_gem(new_version);
    let mut pinned = false;
    for call in calls {
        let mut texts = Vec::new();
        for requirement in &call.requirements {
            let text = match requirement {
                RequirementArg::Literal(literal) => literal.value.clone(),
                RequirementArg::Opaque(source) => {
                    return Ok(ConstraintCheckResult::Unverifiable {
                        constraint: source.clone(),
                    });
                }
            };
            texts.push(text);
        }
        let mut unsatisfied_range = false;
        for text in &texts {
            match requirement_satisfied(text, &new_gem_version) {
                Some(true) => {}
                // The release rewrites a pin (`= x` or bare `x`) to the new version.
                Some(false) if matches!(split_requirement(text), Some((None | Some("="), _))) => {
                    pinned = true;
                }
                Some(false) => unsatisfied_range = true,
                None => {
                    return Ok(ConstraintCheckResult::Skipped {
                        reason: format!("unsupported requirement '{text}'"),
                    });
                }
            }
        }
        if unsatisfied_range {
            return Ok(ConstraintCheckResult::NotSatisfied {
                constraint: texts.join(", "),
                new_version: new_version.to_string(),
            });
        }
    }
    if pinned {
        return Ok(ConstraintCheckResult::Skipped {
            reason: "pinned version".to_string(),
        });
    }
    Ok(ConstraintCheckResult::Satisfied)
}

pub(super) fn gemfile_lock_dirs(workspace_root: &Path) -> Vec<PathBuf> {
    let scan = LazyScan::new(workspace_root);
    let mut dirs = vec![workspace_root.to_path_buf()];
    dirs.extend(
        gem_dirs(workspace_root, scan.index())
            .into_iter()
            .map(|(dir, _)| dir)
            .filter(|dir| dir != workspace_root),
    );
    dirs.retain(|dir| dir.join("Gemfile.lock").is_file() && dir.join("Gemfile").is_file());
    dirs
}

fn warn_skip(name: &str, gemspec_path: &Path, reason: &str) {
    eprintln!(
        "Warning: skipping '{}' ({}): {}; set a literal `VERSION = \"x.y.z\"` in \
         lib/<gem>/version.rb, or a literal `spec.version`, for Sampo to manage it",
        name,
        gemspec_path.display(),
        reason
    );
}

fn read_gemspec(manifest_path: &Path) -> Result<Gemspec> {
    let text = fs::read_to_string(manifest_path)
        .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
    Ok(Gemspec::parse(&text))
}

/// The SemVer form of a gem version, which Sampo bumps: `1.0.0.rc1` → `1.0.0-rc1`.
pub(super) fn gem_to_semver(version: &str) -> Option<String> {
    let version = version.trim();
    if let Some((core, pre)) = version.split_once('-') {
        let core_segments: Vec<&str> = core.split('.').collect();
        let numeric = core_segments
            .iter()
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
        if !numeric || core_segments.len() > 3 || pre.is_empty() {
            return None;
        }
        semver::Prerelease::new(pre).ok()?;
        return Some(version.to_string());
    }
    let segments: Vec<&str> = version.split('.').collect();
    let numeric = segments
        .iter()
        .take_while(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .count();
    if numeric == 0 || numeric > 3 {
        return None;
    }
    let core = segments[..numeric].join(".");
    if numeric == segments.len() {
        return Some(core);
    }
    // `1.0.0a` is `1.0.0.a` to RubyGems, which splits digits from letters; a dotted
    // split would misread it as core `1.0` with pre-release `0a`.
    if segments[numeric].starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let pre = segments[numeric..].join(".");
    semver::Prerelease::new(&pre).ok()?;
    Some(format!("{core}-{pre}"))
}

/// The gem spelling Sampo writes for a SemVer version: `1.1.0-rc.1` → `1.1.0.rc.1`, the
/// dotted form RubyGems keeps verbatim (a hyphen would be published as `.pre.`).
pub(super) fn semver_to_gem(version: &str) -> String {
    match version.split_once('-') {
        Some((core, pre)) => format!("{core}.{}", pre.replace('-', ".")),
        None => version.to_string(),
    }
}

/// The literal replacing `current` with `version`. A hyphenated literal stays hyphenated:
/// RubyGems publishes it as `.pre.`, and `1.0.0.alpha.2` would sort below the
/// `1.0.0.pre.alpha.1` already published.
fn gem_literal(version: &str, current: &str) -> String {
    if current.contains('-') {
        version.to_string()
    } else {
        semver_to_gem(version)
    }
}

fn is_prerelease(version: &str) -> bool {
    version.bytes().any(|b| b.is_ascii_alphabetic())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Number(u64),
    Text(String),
}

/// `Gem::Version#segments`: runs of digits or letters, after RubyGems' `-` → `.pre.`.
fn gem_segments(version: &str) -> Option<Vec<Segment>> {
    let version = version.trim().replace('-', ".pre.");
    if version.is_empty()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.')
    {
        return None;
    }
    let mut segments = Vec::new();
    let bytes = version.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'.' {
            i += 1;
            continue;
        }
        let digits = bytes[i].is_ascii_digit();
        let start = i;
        while i < bytes.len() && bytes[i] != b'.' && bytes[i].is_ascii_digit() == digits {
            i += 1;
        }
        let run = &version[start..i];
        segments.push(if digits {
            Segment::Number(run.parse().ok()?)
        } else {
            Segment::Text(run.to_string())
        });
    }
    Some(segments)
}

/// `Gem::Version#canonical_segments`: trailing zeros dropped from the release and the
/// pre-release parts, so `1.0` equals `1.0.0`.
fn canonical_segments(segments: &[Segment]) -> Vec<Segment> {
    let split = segments
        .iter()
        .position(|s| matches!(s, Segment::Text(_)))
        .unwrap_or(segments.len());
    let mut out = Vec::new();
    for part in [&segments[..split], &segments[split..]] {
        let mut part = part.to_vec();
        while part.last() == Some(&Segment::Number(0)) {
            part.pop();
        }
        out.extend(part);
    }
    out
}

/// `Gem::Version#<=>`: a missing segment counts as zero, and text sorts before numbers.
fn compare_gem_versions(a: &[Segment], b: &[Segment]) -> Ordering {
    let (a, b) = (canonical_segments(a), canonical_segments(b));
    let zero = Segment::Number(0);
    for i in 0..a.len().max(b.len()) {
        let (lhs, rhs) = (a.get(i).unwrap_or(&zero), b.get(i).unwrap_or(&zero));
        let ordering = match (lhs, rhs) {
            (Segment::Number(l), Segment::Number(r)) => l.cmp(r),
            (Segment::Text(l), Segment::Text(r)) => l.cmp(r),
            (Segment::Text(_), Segment::Number(_)) => Ordering::Less,
            (Segment::Number(_), Segment::Text(_)) => Ordering::Greater,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

/// `Gem::Version#bump`, the exclusive upper bound of `~>`: drop the pre-release and the
/// last segment, then increment the new last one (`1.2.3` → `1.3`, `1` → `2`).
fn pessimistic_upper_bound(segments: &[Segment]) -> Vec<Segment> {
    let mut release: Vec<Segment> = segments
        .iter()
        .take_while(|s| matches!(s, Segment::Number(_)))
        .cloned()
        .collect();
    if release.len() > 1 {
        release.pop();
    }
    if let Some(Segment::Number(last)) = release.last_mut() {
        *last += 1;
    }
    release
}

/// Split a requirement such as `~> 1.2` into its operator (`None` for a bare version,
/// which RubyGems reads as `=`) and its version text.
fn split_requirement(requirement: &str) -> Option<(Option<&'static str>, &str)> {
    let trimmed = requirement.trim();
    for op in ["~>", ">=", "<=", "!=", ">", "<", "="] {
        if let Some(rest) = trimmed.strip_prefix(op) {
            return Some((Some(op), rest.trim()));
        }
    }
    (!trimmed.is_empty()).then_some((None, trimmed))
}

fn requirement_satisfied(requirement: &str, version: &str) -> Option<bool> {
    let (op, required) = split_requirement(requirement)?;
    let required = gem_segments(required)?;
    let version = gem_segments(version)?;
    let ordering = compare_gem_versions(&version, &required);
    Some(match op {
        None | Some("=") => ordering == Ordering::Equal,
        Some("!=") => ordering != Ordering::Equal,
        Some(">") => ordering == Ordering::Greater,
        Some("<") => ordering == Ordering::Less,
        Some(">=") => ordering != Ordering::Less,
        Some("<=") => ordering != Ordering::Greater,
        Some("~>") => {
            let release: Vec<Segment> = version
                .iter()
                .take_while(|s| matches!(s, Segment::Number(_)))
                .cloned()
                .collect();
            ordering != Ordering::Less
                && compare_gem_versions(&release, &pessimistic_upper_bound(&required))
                    == Ordering::Less
        }
        Some(_) => return None,
    })
}

/// The splices that move one dependency's requirements to `new_version`, or `None` when
/// they must stay as written.
///
/// A requirement the new version satisfies is kept, so `~> 1.2` survives a bump to 1.3.0.
/// An unsatisfied `~>` keeps its precision (`~> 1.2` → `~> 2.0`). Pins and `~>` also
/// follow a pre-release, as RubyGems installs one only when the requirement names it.
fn rewrite_requirements(
    requirements: &[RequirementArg],
    new_version: &str,
) -> Option<Vec<(usize, usize, String)>> {
    let gem_version = semver_to_gem(new_version);
    let mut rewrites = Vec::new();
    for requirement in requirements {
        let RequirementArg::Literal(literal) = requirement else {
            return None;
        };
        let (op, current) = split_requirement(&literal.value)?;
        let satisfied = requirement_satisfied(&literal.value, &gem_version)?;
        let tracks_prerelease = is_prerelease(new_version) || is_prerelease(current);
        let rewritten = match op {
            None | Some("=" | "~>") if satisfied && !tracks_prerelease => continue,
            None => gem_version.clone(),
            Some("=") => format!("= {gem_version}"),
            Some("~>") => format!("~> {}", pessimistic_target(&gem_version, current)?),
            _ if satisfied => continue,
            _ => return None,
        };
        if rewritten != literal.value {
            rewrites.push((literal.start, literal.end, rewritten));
        }
    }
    Some(rewrites)
}

/// `version` cut to the precision of `template` for a `~>` requirement, keeping any
/// pre-release: `1.3.0.rc.1` against `1.2` gives `1.3.rc.1`, still below 2.
fn pessimistic_target(version: &str, template: &str) -> Option<String> {
    let release = with_precision(version, template)?;
    let numeric = version
        .split('.')
        .take_while(|s| s.bytes().all(|b| b.is_ascii_digit()))
        .count();
    let pre: Vec<&str> = version.split('.').skip(numeric).collect();
    if pre.is_empty() {
        Some(release)
    } else {
        Some(format!("{release}.{}", pre.join(".")))
    }
}

fn with_precision(version: &str, template: &str) -> Option<String> {
    // Before any hyphen: `1.2.0-rc.1` has the precision of `1.2.0`.
    let release = template.split('-').next().unwrap_or(template);
    let precision = release
        .split('.')
        .take_while(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .count()
        .max(1);
    let mut release: Vec<&str> = version
        .split('.')
        .take_while(|s| s.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    if release.is_empty() {
        return None;
    }
    while release.len() < precision {
        release.push("0");
    }
    release.truncate(precision);
    Some(release.join("."))
}

/// A string literal's content and its byte span between the delimiters, so a splice
/// keeps the quote style (`"…"`, `'…'`, `%q<…>`) and any `.freeze` around it.
#[derive(Debug, Clone)]
pub(super) struct Literal {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) value: String,
}

#[derive(Debug)]
pub(super) struct VersionLiteral {
    pub(super) path: PathBuf,
    pub(super) literal: Literal,
}

#[derive(Debug)]
enum VersionSource {
    Absent,
    /// `spec.version = "1.2.3"`, or the second argument of `Gem::Specification.new`.
    Literal(Literal),
    /// `spec.version = Foo::VERSION` (with optional `.dup`/`.freeze`), as path segments.
    Constant(Vec<String>),
    /// Anything else: a method call, a file read, an interpolated string.
    Computed,
}

#[derive(Debug)]
enum RequirementArg {
    Literal(Literal),
    /// Not a string literal (a variable, an interpolation): kept as source text.
    Opaque(String),
}

#[derive(Debug)]
struct DependencyCall {
    name: String,
    development: bool,
    requirements: Vec<RequirementArg>,
}

#[derive(Debug)]
struct Gemspec {
    name: Option<String>,
    version: VersionSource,
    /// Files the gemspec loads, as written: `require_relative` targets are relative to the
    /// gemspec, `require` targets to its `lib/`.
    required: Vec<(bool, String)>,
    dependencies: Vec<DependencyCall>,
    allowed_push_host: Option<String>,
}

fn ruby_language() -> &'static Language {
    static LANGUAGE: OnceLock<Language> = OnceLock::new();
    LANGUAGE.get_or_init(|| tree_sitter_ruby::LANGUAGE.into())
}

fn parse_ruby(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(ruby_language()).ok()?;
    parser.parse(source, None)
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.start_byte()..node.end_byte()).unwrap_or("")
}

fn descendants(tree: &Tree) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        out.push(node);
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out
}

/// Strip `.freeze`, `.dup` and `.to_s`, which leave a value unchanged.
fn unwrap_value<'tree>(mut node: Node<'tree>, source: &str) -> Node<'tree> {
    while node.child_by_field_name("arguments").is_none()
        && matches!(method_name(node, source), Some("freeze" | "dup" | "to_s"))
    {
        match node.child_by_field_name("receiver") {
            Some(receiver) => node = receiver,
            None => break,
        }
    }
    node
}

/// A plain string literal: no interpolation and no escape sequence, so the bytes between
/// the delimiters are its value.
fn string_literal(node: Node, source: &str) -> Option<Literal> {
    let node = unwrap_value(node, source);
    if node.kind() != "string" || node.child_count() < 2 {
        return None;
    }
    let mut cursor = node.walk();
    if node
        .named_children(&mut cursor)
        .any(|child| child.kind() != "string_content")
    {
        return None;
    }
    let open = node.child(0)?;
    let close = node.child(node.child_count() as u32 - 1)?;
    let (start, end) = (open.end_byte(), close.start_byte());
    Some(Literal {
        start,
        end,
        value: source.get(start..end)?.to_string(),
    })
}

/// The segments of a constant reference: `Foo::Bar::VERSION` → `[Foo, Bar, VERSION]`.
fn constant_path(node: Node, source: &str) -> Option<Vec<String>> {
    match node.kind() {
        "constant" => Some(vec![node_text(node, source).to_string()]),
        "scope_resolution" => {
            let name = node.child_by_field_name("name")?;
            let mut path = match node.child_by_field_name("scope") {
                Some(scope) => constant_path(scope, source)?,
                None => Vec::new(),
            };
            path.push(node_text(name, source).to_string());
            Some(path)
        }
        _ => None,
    }
}

fn method_name<'a>(call: Node, source: &'a str) -> Option<&'a str> {
    if call.kind() != "call" {
        return None;
    }
    call.child_by_field_name("method")
        .map(|method| node_text(method, source))
}

fn arguments<'tree>(call: Node<'tree>) -> Vec<Node<'tree>> {
    let Some(list) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut cursor = list.walk();
    list.named_children(&mut cursor)
        .filter(|n| n.kind() != "comment")
        .collect()
}

fn classify_version(value: Node, source: &str) -> VersionSource {
    if let Some(literal) = string_literal(value, source) {
        return VersionSource::Literal(literal);
    }
    match constant_path(unwrap_value(value, source), source) {
        Some(path) => VersionSource::Constant(path),
        None => VersionSource::Computed,
    }
}

impl Gemspec {
    /// tree-sitter recovers from syntax errors, so a broken gemspec still yields what it
    /// can read; one without a literal name is then skipped at discovery.
    fn parse(source: &str) -> Self {
        let mut gemspec = Gemspec {
            name: None,
            version: VersionSource::Absent,
            required: Vec::new(),
            dependencies: Vec::new(),
            allowed_push_host: None,
        };
        let Some(tree) = parse_ruby(source) else {
            return gemspec;
        };

        for node in descendants(&tree) {
            match node.kind() {
                "assignment" => gemspec.read_assignment(node, source),
                "call" => gemspec.read_call(node, source),
                _ => {}
            }
        }
        gemspec
    }

    fn read_assignment(&mut self, node: Node, source: &str) {
        let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) else {
            return;
        };
        match left.kind() {
            // `spec.name = "foo"` / `spec.version = Foo::VERSION`
            "call" if left.child_by_field_name("receiver").is_some() => {
                match method_name(left, source) {
                    Some("name") if self.name.is_none() => {
                        self.name = string_literal(right, source).map(|l| l.value);
                    }
                    Some("version") if matches!(self.version, VersionSource::Absent) => {
                        self.version = classify_version(right, source);
                    }
                    // `spec.metadata = { "allowed_push_host" => "https://…" }`
                    Some("metadata") if right.kind() == "hash" => {
                        let mut cursor = right.walk();
                        for pair in right.named_children(&mut cursor) {
                            let key = pair
                                .child_by_field_name("key")
                                .and_then(|key| string_literal(key, source));
                            if key.is_some_and(|k| k.value == "allowed_push_host") {
                                self.allowed_push_host = pair
                                    .child_by_field_name("value")
                                    .and_then(|value| string_literal(value, source))
                                    .map(|l| l.value);
                            }
                        }
                    }
                    _ => {}
                }
            }
            // `spec.metadata["allowed_push_host"] = "https://…"`
            "element_reference" => {
                let is_metadata = left
                    .child_by_field_name("object")
                    .and_then(|object| method_name(object, source))
                    == Some("metadata");
                let mut cursor = left.walk();
                let key = left
                    .named_children(&mut cursor)
                    .skip(1)
                    .find_map(|n| string_literal(n, source));
                if is_metadata && key.is_some_and(|k| k.value == "allowed_push_host") {
                    self.allowed_push_host = string_literal(right, source).map(|l| l.value);
                }
            }
            _ => {}
        }
    }

    fn read_call(&mut self, node: Node, source: &str) {
        let Some(method) = method_name(node, source) else {
            return;
        };
        let args = arguments(node);
        match method {
            "require_relative" | "require" => {
                if let Some(target) = args.first().and_then(|a| string_literal(*a, source)) {
                    self.required
                        .push((method == "require_relative", target.value));
                }
            }
            // `Gem::Specification.new "name", "1.0" do |s| … end`
            "new" => {
                let is_specification = node
                    .child_by_field_name("receiver")
                    .and_then(|r| constant_path(r, source))
                    .is_some_and(|path| path.ends_with(&["Gem".into(), "Specification".into()]));
                if !is_specification {
                    return;
                }
                if self.name.is_none() {
                    self.name = args
                        .first()
                        .and_then(|a| string_literal(*a, source))
                        .map(|l| l.value);
                }
                if let Some(version) = args.get(1)
                    && matches!(self.version, VersionSource::Absent)
                {
                    self.version = classify_version(*version, source);
                }
            }
            method if DEPENDENCY_METHODS.contains(&method) => {
                let Some(name) = args.first().and_then(|a| string_literal(*a, source)) else {
                    return;
                };
                let mut requirements = Vec::new();
                for arg in &args[1..] {
                    if arg.kind() == "array" {
                        let mut cursor = arg.walk();
                        for element in arg.named_children(&mut cursor) {
                            requirements.push(requirement_arg(element, source));
                        }
                    } else {
                        requirements.push(requirement_arg(*arg, source));
                    }
                }
                self.dependencies.push(DependencyCall {
                    name: name.value,
                    development: method == "add_development_dependency",
                    requirements,
                });
            }
            _ => {}
        }
    }
}

fn requirement_arg(node: Node, source: &str) -> RequirementArg {
    match string_literal(node, source) {
        Some(literal) => RequirementArg::Literal(literal),
        None => RequirementArg::Opaque(node_text(node, source).to_string()),
    }
}

/// The string literal behind the gemspec's version, in the gemspec itself or the file
/// defining the referenced constant. `Err` is the reason it cannot be managed.
fn locate_version(
    gemspec_path: &Path,
    gemspec: &Gemspec,
    name: &str,
) -> std::result::Result<VersionLiteral, String> {
    let constant = match &gemspec.version {
        VersionSource::Literal(literal) => {
            return Ok(VersionLiteral {
                path: gemspec_path.to_path_buf(),
                literal: literal.clone(),
            });
        }
        VersionSource::Constant(path) => path,
        VersionSource::Absent => return Err("it sets no `version`".to_string()),
        VersionSource::Computed => {
            return Err(
                "its version is computed (only a literal or a constant can be managed)".to_string(),
            );
        }
    };

    let gem_dir = gemspec_path.parent().unwrap_or(Path::new("."));
    let mut candidates: Vec<PathBuf> = Vec::new();
    for (relative, target) in &gemspec.required {
        let base = if *relative {
            gem_dir.to_path_buf()
        } else {
            gem_dir.join("lib")
        };
        let mut path = base.join(target);
        if path.extension().is_none() {
            path.set_extension("rb");
        }
        candidates.push(path);
    }
    // The `bundle gem` layout: `foo-bar` keeps its constant in `lib/foo/bar/version.rb`.
    candidates.push(
        gem_dir
            .join("lib")
            .join(name.replace('-', "/"))
            .join("version.rb"),
    );
    candidates.push(gem_dir.join("lib").join(name).join("version.rb"));
    candidates.push(gemspec_path.to_path_buf());

    let reference = constant.join("::");
    let mut seen = BTreeSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.clone()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&candidate) else {
            continue;
        };
        match find_constant(&text, constant) {
            Some(Some(literal)) => {
                return Ok(VersionLiteral {
                    path: candidate,
                    literal,
                });
            }
            Some(None) => {
                return Err(format!(
                    "`{reference}` in {} is not a string literal",
                    candidate.display()
                ));
            }
            None => {}
        }
    }
    Err(format!(
        "neither the gemspec, a file it loads, nor lib/{}/version.rb assigns `{reference}`",
        name.replace('-', "/")
    ))
}

/// The assignment of the constant at `path` in a Ruby file: `Some(Some(_))` for a string
/// literal, `Some(None)` for any other value, `None` when the file does not assign it.
fn find_constant(source: &str, path: &[String]) -> Option<Option<Literal>> {
    let tree = parse_ruby(source)?;
    find_constant_in(tree.root_node(), source, &mut Vec::new(), path)
}

fn find_constant_in(
    node: Node,
    source: &str,
    nesting: &mut Vec<String>,
    path: &[String],
) -> Option<Option<Literal>> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    for child in children {
        match child.kind() {
            "module" | "class" => {
                let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|n| constant_path(n, source))
                else {
                    continue;
                };
                let depth = nesting.len();
                nesting.extend(name);
                let found = child
                    .child_by_field_name("body")
                    .and_then(|body| find_constant_in(body, source, nesting, path));
                nesting.truncate(depth);
                if found.is_some() {
                    return found;
                }
            }
            "assignment" => {
                let (Some(left), Some(right)) = (
                    child.child_by_field_name("left"),
                    child.child_by_field_name("right"),
                ) else {
                    continue;
                };
                let Some(assigned) = constant_path(left, source) else {
                    continue;
                };
                let full: Vec<&str> = nesting
                    .iter()
                    .map(String::as_str)
                    .chain(assigned.iter().map(String::as_str))
                    .collect();
                if full.iter().copied().eq(path.iter().map(String::as_str)) {
                    return Some(string_literal(right, source));
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod gemspec_tests;
