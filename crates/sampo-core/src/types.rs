use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;

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

/// Information about a crate in the workspace
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateInfo {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub internal_deps: BTreeSet<String>,
}

/// Represents a Cargo workspace with its members
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub root: PathBuf,
    pub members: Vec<CrateInfo>,
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
