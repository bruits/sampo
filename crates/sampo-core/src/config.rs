use crate::errors::SampoError;
use rustc_hash::FxHashSet;
use semver::Version;
use std::collections::BTreeSet;
use std::path::Path;

/// Configuration for Sampo
#[derive(Debug, Clone)]
pub struct Config {
    #[allow(dead_code)]
    pub version: u64,
    pub github_repository: Option<String>,
    pub changelog_show_commit_hash: bool,
    pub changelog_show_acknowledgments: bool,
    pub changelog_show_release_date: bool,
    pub changelog_release_date_format: String,
    pub changelog_release_date_timezone: Option<String>,
    /// Custom tags for changelog categorization (e.g., "Added", "Fixed", "Changed").
    /// When set, enables Keep a Changelog style sections instead of bump-level sections.
    pub changesets_tags: Vec<String>,
    pub fixed_dependencies: Vec<Vec<String>>,
    pub linked_dependencies: Vec<Vec<String>>,
    pub ignore_unpublished: bool,
    pub ignore: Vec<String>,
    pub git_default_branch: Option<String>,
    pub git_release_branches: Vec<String>,
    /// Package using short tag format (`v{version}`) for Packagist compatibility.
    pub git_short_tags: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            github_repository: None,
            changelog_show_commit_hash: true,
            changelog_show_acknowledgments: true,
            changelog_show_release_date: true,
            changelog_release_date_format: "%Y-%m-%d".to_string(),
            changelog_release_date_timezone: None,
            changesets_tags: Vec::new(),
            fixed_dependencies: Vec::new(),
            linked_dependencies: Vec::new(),
            ignore_unpublished: false,
            ignore: Vec::new(),
            git_default_branch: None,
            git_release_branches: Vec::new(),
            git_short_tags: None,
        }
    }
}

impl Config {
    /// Load configuration from .sampo/config.toml
    pub fn load(root: &Path) -> Result<Self, SampoError> {
        let base = root.join(".sampo");
        let path = base.join("config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }

        let text = std::fs::read_to_string(&path)
            .map_err(|e| SampoError::Config(format!("failed to read {}: {e}", path.display())))?;
        let value: toml::Value = text
            .parse()
            .map_err(|e| SampoError::Config(format!("invalid config.toml: {e}")))?;

        let version = value
            .get("version")
            .and_then(toml::Value::as_integer)
            .unwrap_or(1);

        let version = u64::try_from(version).unwrap_or(1);

        let github_repository = value
            .get("github")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("repository"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let changelog_table = value.get("changelog").and_then(|v| v.as_table());

        let changelog_show_commit_hash = changelog_table
            .and_then(|t| t.get("show_commit_hash"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let changelog_show_acknowledgments = changelog_table
            .and_then(|t| t.get("show_acknowledgments"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let changelog_show_release_date = changelog_table
            .and_then(|t| t.get("show_release_date"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let changelog_release_date_format = changelog_table
            .and_then(|t| t.get("release_date_format"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "%Y-%m-%d".to_string());

        let changelog_release_date_timezone = changelog_table
            .and_then(|t| t.get("release_date_timezone"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let changesets_table = value.get("changesets").and_then(|v| v.as_table());

        let changesets_tags = changesets_table
            .and_then(|t| t.get("tags"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let fixed_dependencies = value
            .get("packages")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("fixed"))
            .and_then(|v| v.as_array())
            .map(|outer_arr| -> Result<Vec<Vec<String>>, String> {
                // Check if all elements are arrays (groups format)
                let all_arrays = outer_arr.iter().all(|item| item.is_array());
                let any_arrays = outer_arr.iter().any(|item| item.is_array());

                if !all_arrays {
                    if any_arrays {
                        // Mixed format
                        let non_array = outer_arr.iter().find(|item| !item.is_array()).unwrap();
                        return Err(format!(
                            "packages.fixed must be an array of arrays, found mixed format with: {}. Use [[\"a\", \"b\"]] instead of [\"a\", \"b\"]",
                            non_array
                        ));
                    } else {
                        // All strings (flat format)
                        return Err(
                            "packages.fixed must be an array of arrays. Use [[\"a\", \"b\"]] instead of [\"a\", \"b\"]".to_string()
                        );
                    }
                }

                let groups: Vec<Vec<String>> = outer_arr.iter()
                    .filter_map(|inner| inner.as_array())
                    .map(|inner_arr| {
                        inner_arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .collect();

                // Check for overlapping groups
                let mut seen_packages = FxHashSet::default();
                for group in &groups {
                    for package in group {
                        if seen_packages.contains(package) {
                            return Err(format!(
                                "Package '{}' appears in multiple fixed dependency groups. Each package can only belong to one group.",
                                package
                            ));
                        }
                        seen_packages.insert(package.clone());
                    }
                }

                Ok(groups)
            })
            .transpose()
            .map_err(SampoError::Config)?
            .unwrap_or_default();

        let ignore_unpublished = value
            .get("packages")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("ignore_unpublished"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let ignore = value
            .get("packages")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("ignore"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let linked_dependencies = value
            .get("packages")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("linked"))
            .and_then(|v| v.as_array())
            .map(|outer_arr| -> Result<Vec<Vec<String>>, String> {
                // Check if all elements are arrays (groups format)
                let all_arrays = outer_arr.iter().all(|item| item.is_array());
                let any_arrays = outer_arr.iter().any(|item| item.is_array());

                if !all_arrays {
                    if any_arrays {
                        // Mixed format
                        let non_array = outer_arr.iter().find(|item| !item.is_array()).unwrap();
                        return Err(format!(
                            "packages.linked must be an array of arrays, found mixed format with: {}. Use [[\"a\", \"b\"]] instead of [\"a\", \"b\"]",
                            non_array
                        ));
                    } else {
                        // All strings (flat format)
                        return Err(
                            "packages.linked must be an array of arrays. Use [[\"a\", \"b\"]] instead of [\"a\", \"b\"]".to_string()
                        );
                    }
                }

                let groups: Vec<Vec<String>> = outer_arr.iter()
                    .filter_map(|inner| inner.as_array())
                    .map(|inner_arr| {
                        inner_arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .collect();

                // Check for overlapping groups within linked_dependencies
                let mut seen_packages = FxHashSet::default();
                for group in &groups {
                    for package in group {
                        if seen_packages.contains(package) {
                            return Err(format!(
                                "Package '{}' appears in multiple linked dependency groups. Each package can only belong to one group.",
                                package
                            ));
                        }
                        seen_packages.insert(package.clone());
                    }
                }

                Ok(groups)
            })
            .transpose()
            .map_err(SampoError::Config)?
            .unwrap_or_default();

        // Check for overlapping packages between fixed and linked dependencies
        let mut all_fixed_packages = FxHashSet::default();
        for group in &fixed_dependencies {
            for package in group {
                all_fixed_packages.insert(package.clone());
            }
        }

        for group in &linked_dependencies {
            for package in group {
                if all_fixed_packages.contains(package) {
                    return Err(SampoError::Config(format!(
                        "Package '{}' cannot appear in both packages.fixed and packages.linked",
                        package
                    )));
                }
            }
        }

        let (git_default_branch, git_release_branches, git_short_tags) = value
            .get("git")
            .and_then(|v| v.as_table())
            .map(|git_table| {
                let default_branch = git_table
                    .get("default_branch")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                let release_branches = git_table
                    .get("release_branches")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();

                let short_tags = git_table
                    .get("short_tags")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                (default_branch, release_branches, short_tags)
            })
            .unwrap_or((None, Vec::new(), None));

        Ok(Self {
            version,
            github_repository,
            changelog_show_commit_hash,
            changelog_show_acknowledgments,
            changelog_show_release_date,
            changelog_release_date_format,
            changelog_release_date_timezone,
            changesets_tags,
            fixed_dependencies,
            linked_dependencies,
            ignore_unpublished,
            ignore,
            git_default_branch,
            git_release_branches,
            git_short_tags,
        })
    }

    pub fn default_branch(&self) -> &str {
        self.git_default_branch.as_deref().unwrap_or("main")
    }

    pub fn release_branches(&self) -> BTreeSet<String> {
        let mut branches: BTreeSet<String> = BTreeSet::new();
        branches.insert(self.default_branch().to_string());
        for name in &self.git_release_branches {
            if !name.is_empty() {
                branches.insert(name.clone());
            }
        }
        branches
    }

    pub fn is_release_branch(&self, branch: &str) -> bool {
        self.release_branches().contains(branch)
    }

    /// Returns true if the given package should use short tag format (`v{version}`).
    pub fn uses_short_tags(&self, package_name: &str) -> bool {
        self.git_short_tags
            .as_ref()
            .is_some_and(|name| name == package_name)
    }

    /// Builds a git tag name for the given package and version.
    pub fn build_tag_name(&self, package_name: &str, version: &str) -> String {
        if self.uses_short_tags(package_name) {
            format!("v{}", version)
        } else {
            format!("{}-v{}", package_name, version)
        }
    }

    /// Parses a tag and returns (package_name, version).
    pub fn parse_tag(&self, tag: &str) -> Option<(String, String)> {
        if let Some(short_pkg) = self
            .git_short_tags
            .as_ref()
            .filter(|_| tag.starts_with('v'))
        {
            let version_str = tag.trim_start_matches('v');
            if Version::parse(version_str).is_ok() {
                return Some((short_pkg.clone(), version_str.to_string()));
            }
        }

        // Iterate over all "-v" positions to handle prereleases containing "-v" (e.g., "pkg-v1.2.3-v1").
        for (idx, _) in tag.match_indices("-v") {
            let name = &tag[..idx];
            let version = &tag[idx + 2..];
            if name.is_empty() || version.is_empty() {
                continue;
            }
            if Version::parse(version).is_ok() {
                return Some((name.to_string(), version.to_string()));
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn defaults_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::load(temp.path()).unwrap();
        assert_eq!(config.version, 1);
        assert!(config.github_repository.is_none());
        assert!(config.changelog_show_commit_hash);
        assert!(config.changelog_show_acknowledgments);
        assert!(config.changelog_show_release_date);
        assert_eq!(config.changelog_release_date_format, "%Y-%m-%d");
        assert!(config.changelog_release_date_timezone.is_none());
        assert!(config.changesets_tags.is_empty());
        assert_eq!(config.default_branch(), "main");
        assert!(config.is_release_branch("main"));
        assert_eq!(config.git_release_branches, Vec::<String>::new());
        assert!(config.git_short_tags.is_none());
    }

    #[test]
    fn reads_changelog_options() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[changelog]\nshow_commit_hash = false\nshow_acknowledgments = false\nshow_release_date = false\nrelease_date_format = \"%d/%m/%Y\"\nrelease_date_timezone = \"+02:30\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert!(!config.changelog_show_commit_hash);
        assert!(!config.changelog_show_acknowledgments);
        assert!(!config.changelog_show_release_date);
        assert_eq!(config.changelog_release_date_format, "%d/%m/%Y");
        assert_eq!(
            config.changelog_release_date_timezone.as_deref(),
            Some("+02:30")
        );
    }

    #[test]
    fn reads_changesets_tags() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[changesets]\ntags = [\"Added\", \"Changed\", \"Fixed\", \"Deprecated\", \"Removed\", \"Security\"]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(
            config.changesets_tags,
            vec![
                "Added",
                "Changed",
                "Fixed",
                "Deprecated",
                "Removed",
                "Security"
            ]
        );
    }

    #[test]
    fn reads_github_repository() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[github]\nrepository = \"owner/repo\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(config.github_repository.as_deref(), Some("owner/repo"));
    }

    #[test]
    fn reads_both_changelog_and_github() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[changelog]\nshow_commit_hash = false\n[github]\nrepository = \"owner/repo\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert!(!config.changelog_show_commit_hash);
        assert!(config.changelog_show_release_date);
        assert_eq!(config.changelog_release_date_format, "%Y-%m-%d");
        assert!(config.changelog_release_date_timezone.is_none());
        assert_eq!(config.github_repository.as_deref(), Some("owner/repo"));
    }

    #[test]
    fn reads_git_section() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[git]\ndefault_branch = \"release\"\nrelease_branches = [\"release\", \"3.x\"]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(config.default_branch(), "release");
        assert!(config.is_release_branch("release"));
        assert!(config.is_release_branch("3.x"));
        assert!(!config.is_release_branch("main"));
    }

    #[test]
    fn reads_fixed_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nfixed = [[\"pkg-a\", \"pkg-b\"]]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(
            config.fixed_dependencies,
            vec![vec!["pkg-a".to_string(), "pkg-b".to_string()]]
        );
    }

    #[test]
    fn reads_ignore_unpublished_and_ignore_list() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nignore_unpublished = true\nignore = [\"internal-*\", \"examples/*\"]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert!(config.ignore_unpublished);
        assert_eq!(config.ignore, vec!["internal-*", "examples/*"]);
    }

    #[test]
    fn defaults_ignore_options() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::load(temp.path()).unwrap();
        assert!(!config.ignore_unpublished);
        assert!(config.ignore.is_empty());
    }

    #[test]
    fn reads_fixed_dependencies_groups() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nfixed = [[\"pkg-a\", \"pkg-b\"], [\"pkg-c\", \"pkg-d\", \"pkg-e\"]]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(
            config.fixed_dependencies,
            vec![
                vec!["pkg-a".to_string(), "pkg-b".to_string()],
                vec![
                    "pkg-c".to_string(),
                    "pkg-d".to_string(),
                    "pkg-e".to_string()
                ]
            ]
        );
    }

    #[test]
    fn defaults_empty_fixed_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::load(temp.path()).unwrap();
        assert!(config.fixed_dependencies.is_empty());
    }

    #[test]
    fn rejects_flat_array_format() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nfixed = [\"pkg-a\", \"pkg-b\"]\n",
        )
        .unwrap();

        let result = Config::load(temp.path());
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("must be an array of arrays"));
        assert!(error_msg.contains("Use [[\"a\", \"b\"]] instead of [\"a\", \"b\"]"));
    }

    #[test]
    fn rejects_overlapping_groups() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nfixed = [[\"pkg-a\", \"pkg-b\"], [\"pkg-b\", \"pkg-c\"]]\n",
        )
        .unwrap();

        let result = Config::load(temp.path());
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("Package 'pkg-b' appears in multiple fixed dependency groups"));
    }

    #[test]
    fn reads_linked_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nlinked = [[\"pkg-a\", \"pkg-b\"]]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(
            config.linked_dependencies,
            vec![vec!["pkg-a".to_string(), "pkg-b".to_string()]]
        );
    }

    #[test]
    fn reads_linked_dependencies_groups() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nlinked = [[\"pkg-a\", \"pkg-b\"], [\"pkg-c\", \"pkg-d\", \"pkg-e\"]]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(
            config.linked_dependencies,
            vec![
                vec!["pkg-a".to_string(), "pkg-b".to_string()],
                vec![
                    "pkg-c".to_string(),
                    "pkg-d".to_string(),
                    "pkg-e".to_string()
                ]
            ]
        );
    }

    #[test]
    fn defaults_empty_linked_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::load(temp.path()).unwrap();
        assert!(config.linked_dependencies.is_empty());
    }

    #[test]
    fn rejects_overlapping_linked_groups() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nlinked = [[\"pkg-a\", \"pkg-b\"], [\"pkg-b\", \"pkg-c\"]]\n",
        )
        .unwrap();

        let result = Config::load(temp.path());
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("Package 'pkg-b' appears in multiple linked dependency groups"));
    }

    #[test]
    fn rejects_packages_in_both_fixed_and_linked() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nfixed = [[\"pkg-a\", \"pkg-b\"]]\nlinked = [[\"pkg-b\", \"pkg-c\"]]\n",
        )
        .unwrap();

        let result = Config::load(temp.path());
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(
            error_msg.contains(
                "Package 'pkg-b' cannot appear in both packages.fixed and packages.linked"
            )
        );
    }

    #[test]
    fn allows_separate_fixed_and_linked_groups() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nfixed = [[\"pkg-a\", \"pkg-b\"]]\nlinked = [[\"pkg-c\", \"pkg-d\"]]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(
            config.fixed_dependencies,
            vec![vec!["pkg-a".to_string(), "pkg-b".to_string()]]
        );
        assert_eq!(
            config.linked_dependencies,
            vec![vec!["pkg-c".to_string(), "pkg-d".to_string()]]
        );
    }

    #[test]
    fn reads_short_tags() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[git]\nshort_tags = \"my-package\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(config.git_short_tags.as_deref(), Some("my-package"));
    }

    #[test]
    fn defaults_short_tags_to_none() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::load(temp.path()).unwrap();
        assert!(config.git_short_tags.is_none());
    }

    #[test]
    fn uses_short_tags_returns_true_for_matching_package() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[git]\nshort_tags = \"my-package\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert!(config.uses_short_tags("my-package"));
        assert!(!config.uses_short_tags("other-package"));
    }

    #[test]
    fn build_tag_name_uses_short_format_for_configured_package() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[git]\nshort_tags = \"my-package\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(config.build_tag_name("my-package", "1.2.3"), "v1.2.3");
        assert_eq!(
            config.build_tag_name("other-package", "1.2.3"),
            "other-package-v1.2.3"
        );
    }

    #[test]
    fn parse_tag_handles_short_format() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[git]\nshort_tags = \"my-package\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(
            config.parse_tag("v1.2.3"),
            Some(("my-package".to_string(), "1.2.3".to_string()))
        );
        assert_eq!(
            config.parse_tag("v1.2.3-alpha.1"),
            Some(("my-package".to_string(), "1.2.3-alpha.1".to_string()))
        );
        // Standard format still works
        assert_eq!(
            config.parse_tag("other-package-v1.2.3"),
            Some(("other-package".to_string(), "1.2.3".to_string()))
        );
    }

    #[test]
    fn parse_tag_short_format_with_v_in_prerelease() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[git]\nshort_tags = \"my-package\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();

        // Prerelease containing -v (the bug case)
        assert_eq!(
            config.parse_tag("v1.2.3-v1"),
            Some(("my-package".to_string(), "1.2.3-v1".to_string()))
        );
        assert_eq!(
            config.parse_tag("v1.0.0-preview1"),
            Some(("my-package".to_string(), "1.0.0-preview1".to_string()))
        );
        assert_eq!(
            config.parse_tag("v2.0.0-v2-beta"),
            Some(("my-package".to_string(), "2.0.0-v2-beta".to_string()))
        );
        assert_eq!(
            config.parse_tag("v1.2.3+build.123"),
            Some(("my-package".to_string(), "1.2.3+build.123".to_string()))
        );
        assert_eq!(
            config.parse_tag("v1.2.3-alpha.1+build.456"),
            Some((
                "my-package".to_string(),
                "1.2.3-alpha.1+build.456".to_string()
            ))
        );
    }

    #[test]
    fn parse_tag_rejects_invalid_short_tags() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[git]\nshort_tags = \"my-package\"\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();

        assert_eq!(config.parse_tag("v1.2"), None);
        assert_eq!(config.parse_tag("vfoo"), None);
        assert_eq!(config.parse_tag("v01.2.3"), None);
        assert_eq!(config.parse_tag("v"), None);
    }

    #[test]
    fn parse_tag_without_short_tags_config() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::load(temp.path()).unwrap();

        assert_eq!(config.parse_tag("v1.2.3"), None);
        assert_eq!(
            config.parse_tag("my-package-v1.2.3"),
            Some(("my-package".to_string(), "1.2.3".to_string()))
        );
        assert_eq!(
            config.parse_tag("my-package-v1.2.3-alpha.1"),
            Some(("my-package".to_string(), "1.2.3-alpha.1".to_string()))
        );
        // -v in prerelease requires semver validation to parse correctly
        assert_eq!(
            config.parse_tag("my-package-v1.2.3-v1"),
            Some(("my-package".to_string(), "1.2.3-v1".to_string()))
        );
        assert_eq!(config.parse_tag("my-package-vfoo"), None);
        assert_eq!(config.parse_tag("my-package-v1.2"), None);
    }
}
