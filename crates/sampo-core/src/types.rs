use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;

/// Identifies the ecosystem a package belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackageKind {
    Cargo,
    Npm,
    Hex,
    PyPI,
    Packagist,
}

impl PackageKind {
    /// Returns the canonical lowercase string representation (e.g. "cargo").
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Hex => "hex",
            Self::PyPI => "pypi",
            Self::Packagist => "packagist",
        }
    }

    /// Returns a human-friendly display name (e.g. "Cargo").
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Cargo => "Cargo",
            Self::Npm => "npm",
            Self::Hex => "Hex",
            Self::PyPI => "PyPI",
            Self::Packagist => "Packagist",
        }
    }

    /// Formats a package name with the ecosystem when desired.
    pub fn format_name(&self, package_name: &str, include_kind: bool) -> String {
        if include_kind {
            format!("{package_name} ({})", self.display_name())
        } else {
            package_name.to_string()
        }
    }

    /// Parse a kind from a case-insensitive string.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "cargo" => Some(Self::Cargo),
            "npm" => Some(Self::Npm),
            "hex" => Some(Self::Hex),
            "pypi" => Some(Self::PyPI),
            "packagist" => Some(Self::Packagist),
            _ => None,
        }
    }
}

impl std::fmt::Display for PackageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PackageKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(())
    }
}

/// Represents a user-provided package reference (from changesets, config, CLI, etc.)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageSpecifier {
    pub kind: Option<PackageKind>,
    pub name: String,
}

impl PackageSpecifier {
    /// Parse from a raw input string.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("package reference cannot be empty".to_string());
        }

        let unquoted = strip_wrapping_quotes(trimmed);

        if let Some((kind_str, rest)) = unquoted.split_once('/') {
            if rest.is_empty() {
                return Err("package reference is missing a name after '/'".to_string());
            }
            let kind = PackageKind::from_str(kind_str)
                .map_err(|_| format!("unsupported package kind '{}'", kind_str))?;
            Ok(Self {
                kind: Some(kind),
                name: rest.to_string(),
            })
        } else {
            Ok(Self {
                kind: None,
                name: unquoted.to_string(),
            })
        }
    }

    /// Canonical string used when persisting the specifier.
    pub fn to_canonical_string(&self) -> String {
        match self.kind {
            Some(kind) => format!("{}/{}", kind, self.name),
            None => self.name.clone(),
        }
    }

    /// Human-friendly name, optionally including the ecosystem.
    pub fn display_name(&self, include_kind: bool) -> String {
        match self.kind {
            Some(kind) => kind.format_name(&self.name, include_kind),
            None => self.name.clone(),
        }
    }
}

impl std::fmt::Display for PackageSpecifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_canonical_string())
    }
}

/// Information about a dependency update during release
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyUpdate {
    pub name: String,
    pub new_version: String,
}

/// Information about a single released package
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasedPackage {
    pub name: String,
    /// Canonical identifier (e.g. "cargo/sampo-core")
    pub identifier: String,
    pub old_version: String,
    pub new_version: String,
    pub bump: Bump,
}

/// Output information from a release operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseOutput {
    /// Packages that were released
    pub released_packages: Vec<ReleasedPackage>,
    /// Whether this was a dry-run (no files modified)
    pub dry_run: bool,
}

/// Output information from a publish operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishOutput {
    /// Tags that were created (non-dry-run) or would be created (dry-run)
    pub tags: Vec<String>,
    /// Whether this was a dry-run (no packages actually published)
    pub dry_run: bool,
}

/// Information about a package in the workspace
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    pub name: String,
    /// Canonical identifier in the form "<kind>/<name>" (e.g. "cargo/sampo-core")
    pub identifier: String,
    pub version: String,
    pub path: PathBuf,
    pub internal_deps: BTreeSet<String>,
    pub kind: PackageKind,
}

impl PackageInfo {
    /// Returns the canonical identifier for this package.
    pub fn canonical_identifier(&self) -> &str {
        &self.identifier
    }

    /// Human-friendly name for display, optionally including the ecosystem.
    pub fn display_name(&self, include_kind: bool) -> String {
        self.kind.format_name(&self.name, include_kind)
    }

    /// Helper to build a dependency identifier for a given kind/name pair.
    pub fn dependency_identifier(kind: PackageKind, name: &str) -> String {
        format!("{}/{}", kind.as_str(), name)
    }
}

/// Represents a workspace with its package members
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub root: PathBuf,
    pub members: Vec<PackageInfo>,
}

/// Information describing a user-provided package reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecQuery {
    pub name: String,
    pub identifier: Option<String>,
}

impl SpecQuery {
    pub fn new(name: String, identifier: Option<String>) -> Self {
        Self { name, identifier }
    }

    /// Preferred display value for diagnostics.
    pub fn display(&self) -> &str {
        self.identifier.as_deref().unwrap_or(self.name.as_str())
    }

    /// Returns the raw package name without ecosystem prefix.
    pub fn base_name(&self) -> &str {
        &self.name
    }

    /// Optional canonical identifier supplied with the query.
    pub fn identifier(&self) -> Option<&str> {
        self.identifier.as_deref()
    }
}

/// Classification of how a specifier matches workspace packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecResolution<'a> {
    Match(&'a PackageInfo),
    NotFound {
        query: SpecQuery,
    },
    Ambiguous {
        query: SpecQuery,
        matches: Vec<&'a PackageInfo>,
    },
}

impl Workspace {
    /// Returns the package matching the given canonical identifier, if any.
    pub fn find_by_identifier(&self, identifier: &str) -> Option<&PackageInfo> {
        self.members
            .iter()
            .find(|info| info.identifier == identifier)
    }

    /// Returns all workspace packages matching the provided specifier.
    pub fn match_specifier<'a>(&'a self, spec: &PackageSpecifier) -> Vec<&'a PackageInfo> {
        match spec.kind {
            Some(kind) => self
                .members
                .iter()
                .filter(|info| info.kind == kind && info.name == spec.name)
                .collect(),
            None => self
                .members
                .iter()
                .filter(|info| info.name == spec.name)
                .collect(),
        }
    }

    /// Resolves a specifier to a single package or classifies the failure.
    pub fn resolve_specifier<'a>(&'a self, spec: &PackageSpecifier) -> SpecResolution<'a> {
        if let Some(kind) = spec.kind {
            let identifier = PackageInfo::dependency_identifier(kind, &spec.name);
            match self.find_by_identifier(&identifier) {
                Some(info) => SpecResolution::Match(info),
                None => SpecResolution::NotFound {
                    query: SpecQuery::new(spec.name.clone(), Some(identifier)),
                },
            }
        } else {
            let matches = self.match_specifier(spec);
            match matches.len() {
                0 => SpecResolution::NotFound {
                    query: SpecQuery::new(spec.name.clone(), None),
                },
                1 => SpecResolution::Match(matches[0]),
                _ => SpecResolution::Ambiguous {
                    query: SpecQuery::new(spec.name.clone(), None),
                    matches,
                },
            }
        }
    }

    /// Returns true when the workspace contains packages from multiple ecosystems.
    pub fn has_multiple_package_kinds(&self) -> bool {
        let mut kinds = self.members.iter().map(|info| info.kind);
        if let Some(first) = kinds.next() {
            kinds.any(|kind| kind != first)
        } else {
            false
        }
    }
}

/// Formats ambiguous matches for error messaging.
pub fn format_ambiguity_options(matches: &[&PackageInfo]) -> String {
    matches
        .iter()
        .map(|info| format!("{}/{}", info.kind.as_str(), info.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Strip wrapping single or double quotes from a string.
pub fn strip_wrapping_quotes(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|s| s.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// Semantic version bump types, ordered by impact
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl FromStr for Bump {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "patch" => Ok(Self::Patch),
            "minor" => Ok(Self::Minor),
            "major" => Ok(Self::Major),
            _ => Err(()),
        }
    }
}

impl Bump {
    /// Parse a bump type from a string (convenient method that returns Option)
    pub fn parse(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    /// Convert bump to string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

impl std::fmt::Display for Bump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<changesets::ChangeType> for Bump {
    type Error = ();

    fn try_from(change_type: changesets::ChangeType) -> Result<Self, Self::Error> {
        match change_type {
            changesets::ChangeType::Patch => Ok(Self::Patch),
            changesets::ChangeType::Minor => Ok(Self::Minor),
            changesets::ChangeType::Major => Ok(Self::Major),
            changesets::ChangeType::Custom(_) => Err(()),
        }
    }
}

/// Represents how a changelog entry should be categorized.
///
/// When using custom tags (e.g., Keep a Changelog style), the tag determines
/// the section heading. Otherwise, the bump level determines the heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangelogCategory {
    /// Categorize by semver bump level (default behavior)
    Bump(Bump),
    /// Categorize by custom tag (e.g., "Added", "Fixed", "Changed")
    Tag(String),
}

impl ChangelogCategory {
    /// Returns the underlying bump level for version calculation.
    pub fn bump(&self) -> Bump {
        match self {
            Self::Bump(b) => *b,
            Self::Tag(_) => Bump::Patch, // Tags don't affect bump calculation
        }
    }

    /// Returns the heading text to use in changelogs.
    pub fn heading(&self) -> String {
        match self {
            Self::Bump(bump) => match bump {
                Bump::Major => "Major changes".to_string(),
                Bump::Minor => "Minor changes".to_string(),
                Bump::Patch => "Patch changes".to_string(),
            },
            Self::Tag(tag) => tag.clone(),
        }
    }

    /// Returns a sort key for ordering categories.
    /// Tags are sorted alphabetically first, then bump types by severity (Major, Minor, Patch).
    pub fn sort_key(&self) -> (u8, String) {
        match self {
            Self::Tag(tag) => (0, tag.to_lowercase()),
            Self::Bump(Bump::Major) => (1, String::new()),
            Self::Bump(Bump::Minor) => (2, String::new()),
            Self::Bump(Bump::Patch) => (3, String::new()),
        }
    }
}

impl Ord for ChangelogCategory {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl PartialOrd for ChangelogCategory {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for ChangelogCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bump(bump) => write!(f, "{}", bump),
            Self::Tag(tag) => write!(f, "{}", tag),
        }
    }
}

/// Result of parsing a change type string that may include a custom tag.
///
/// Parses formats like:
/// - `minor` -> bump=Minor, tag=None
/// - `minor (Added)` -> bump=Minor, tag=Some("Added")
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChangeType {
    pub bump: Bump,
    pub tag: Option<String>,
}

impl ParsedChangeType {
    /// Parse a change type string, optionally extracting a custom tag.
    ///
    /// Supports formats:
    /// - `patch`, `minor`, `major` - standard semver bumps
    /// - `minor (Added)` - semver bump with custom tag for changelog categorization
    ///
    /// The tag must be enclosed in parentheses at the end of the string.
    /// Tags are only allowed if `allowed_tags` is non-empty (configured via changesets.tags).
    pub fn parse(input: &str, allowed_tags: &[String]) -> Result<Self, String> {
        let trimmed = input.trim();

        // Check for tag format: "bump (Tag)"
        if let Some(paren_start) = trimmed.rfind('(')
            && let Some(paren_end) = trimmed.rfind(')')
            && paren_end > paren_start
            && paren_end == trimmed.len() - 1
        {
            let bump_part = trimmed[..paren_start].trim();
            let tag_part = trimmed[paren_start + 1..paren_end].trim();

            let bump = Bump::parse(bump_part).ok_or_else(|| {
                format!(
                    "Invalid bump level '{}'. Expected 'patch', 'minor', or 'major'.",
                    bump_part
                )
            })?;

            if !tag_part.is_empty() {
                // Tags require configuration via changesets.tags
                if allowed_tags.is_empty() {
                    return Err(format!(
                        "Tag '{}' found, but no tags are configured. Please configure changesets.tags in your config file.",
                        tag_part
                    ));
                }

                // Find matching configured tag (case-insensitive), use its casing
                let configured_tag = allowed_tags
                    .iter()
                    .find(|t| t.eq_ignore_ascii_case(tag_part));

                match configured_tag {
                    Some(tag) => {
                        return Ok(Self {
                            bump,
                            tag: Some(tag.clone()),
                        });
                    }
                    None => {
                        return Err(format!(
                            "Tag '{}' is not in the configured changesets.tags list. Allowed tags: {:?}",
                            tag_part, allowed_tags
                        ));
                    }
                }
            }
        }

        // Standard bump format without tag
        let bump = Bump::parse(trimmed).ok_or_else(|| {
            format!(
                "Invalid change type '{}'. Expected 'patch', 'minor', 'major', or 'bump (Tag)' format.",
                trimmed
            )
        })?;

        Ok(Self { bump, tag: None })
    }

    /// Convert to a ChangelogCategory based on whether a tag is present.
    pub fn to_category(&self) -> ChangelogCategory {
        match &self.tag {
            Some(tag) => ChangelogCategory::Tag(tag.clone()),
            None => ChangelogCategory::Bump(self.bump),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn make_package(name: &str) -> PackageInfo {
        PackageInfo {
            name: name.to_string(),
            identifier: PackageInfo::dependency_identifier(PackageKind::Cargo, name),
            version: "0.1.0".to_string(),
            path: PathBuf::from(format!("crates/{name}")),
            internal_deps: BTreeSet::new(),
            kind: PackageKind::Cargo,
        }
    }

    #[test]
    fn resolve_specifier_matches_prefixed_identifier() {
        let workspace = Workspace {
            root: PathBuf::new(),
            members: vec![make_package("core")],
        };
        let spec = PackageSpecifier::parse("cargo/core").unwrap();
        let outcome = workspace.resolve_specifier(&spec);
        assert!(matches!(outcome, SpecResolution::Match(info) if info.name == "core"));
    }

    #[test]
    fn resolve_specifier_not_found_reports_identifier() {
        let workspace = Workspace {
            root: PathBuf::new(),
            members: vec![make_package("core")],
        };
        let spec = PackageSpecifier::parse("cargo/missing").unwrap();
        let outcome = workspace.resolve_specifier(&spec);
        match outcome {
            SpecResolution::NotFound { query } => {
                assert_eq!(query.identifier(), Some("cargo/missing"));
                assert_eq!(query.display(), "cargo/missing");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn resolve_specifier_detects_ambiguity() {
        let pkg_a = make_package("shared");
        let mut pkg_b = make_package("shared");
        pkg_b.identifier = "cargo/shared-alt".to_string();
        let workspace = Workspace {
            root: PathBuf::new(),
            members: vec![pkg_a, pkg_b],
        };
        let spec = PackageSpecifier::parse("shared").unwrap();
        let outcome = workspace.resolve_specifier(&spec);
        match outcome {
            SpecResolution::Ambiguous { query, matches } => {
                assert_eq!(query.base_name(), "shared");
                assert_eq!(matches.len(), 2);
                let listing = format_ambiguity_options(&matches);
                assert!(listing.contains("cargo/shared"));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn parsed_change_type_simple_bump() {
        let result = ParsedChangeType::parse("minor", &[]).unwrap();
        assert_eq!(result.bump, Bump::Minor);
        assert_eq!(result.tag, None);
    }

    #[test]
    fn parsed_change_type_with_tag() {
        let allowed = vec!["Added".to_string()];
        let result = ParsedChangeType::parse("minor (Added)", &allowed).unwrap();
        assert_eq!(result.bump, Bump::Minor);
        assert_eq!(result.tag, Some("Added".to_string()));
    }

    #[test]
    fn parsed_change_type_validates_tag_when_configured() {
        let allowed = vec!["Added".to_string(), "Fixed".to_string()];
        let result = ParsedChangeType::parse("patch (Fixed)", &allowed).unwrap();
        assert_eq!(result.bump, Bump::Patch);
        assert_eq!(result.tag, Some("Fixed".to_string()));

        let err = ParsedChangeType::parse("patch (Unknown)", &allowed).unwrap_err();
        assert!(err.contains("not in the configured changesets.tags list"));
    }

    #[test]
    fn parsed_change_type_case_insensitive_tag_validation() {
        let allowed = vec!["Added".to_string()];
        // User writes "added" but we normalize to configured casing "Added"
        let result = ParsedChangeType::parse("minor (added)", &allowed).unwrap();
        assert_eq!(result.tag, Some("Added".to_string()));

        // Also test uppercase input
        let result = ParsedChangeType::parse("minor (ADDED)", &allowed).unwrap();
        assert_eq!(result.tag, Some("Added".to_string()));
    }

    #[test]
    fn changelog_category_heading() {
        assert_eq!(
            ChangelogCategory::Bump(Bump::Major).heading(),
            "Major changes"
        );
        assert_eq!(
            ChangelogCategory::Bump(Bump::Minor).heading(),
            "Minor changes"
        );
        assert_eq!(
            ChangelogCategory::Bump(Bump::Patch).heading(),
            "Patch changes"
        );
        assert_eq!(
            ChangelogCategory::Tag("Added".to_string()).heading(),
            "Added"
        );
        assert_eq!(
            ChangelogCategory::Tag("Fixed".to_string()).heading(),
            "Fixed"
        );
    }

    #[test]
    fn changelog_category_bump_extraction() {
        assert_eq!(ChangelogCategory::Bump(Bump::Major).bump(), Bump::Major);
        assert_eq!(
            ChangelogCategory::Tag("Added".to_string()).bump(),
            Bump::Patch
        );
    }
}
