use crate::errors::SampoError;
use rustc_hash::FxHashSet;
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
    pub fixed_dependencies: Vec<Vec<String>>,
    pub linked_dependencies: Vec<Vec<String>>,
    pub ignore_unpublished: bool,
    pub ignore: Vec<String>,
    pub git_default_branch: Option<String>,
    pub git_release_branches: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            github_repository: None,
            changelog_show_commit_hash: true,
            changelog_show_acknowledgments: true,
            fixed_dependencies: Vec::new(),
            linked_dependencies: Vec::new(),
            ignore_unpublished: false,
            ignore: Vec::new(),
            git_default_branch: None,
            git_release_branches: Vec::new(),
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

        let changelog_show_commit_hash = value
            .get("changelog")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("show_commit_hash"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let changelog_show_acknowledgments = value
            .get("changelog")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("show_acknowledgments"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

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

        let (git_default_branch, git_release_branches) = value
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

                (default_branch, release_branches)
            })
            .unwrap_or((None, Vec::new()));

        Ok(Self {
            version,
            github_repository,
            changelog_show_commit_hash,
            changelog_show_acknowledgments,
            fixed_dependencies,
            linked_dependencies,
            ignore_unpublished,
            ignore,
            git_default_branch,
            git_release_branches,
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
        assert_eq!(config.default_branch(), "main");
        assert!(config.is_release_branch("main"));
        assert_eq!(config.git_release_branches, Vec::<String>::new());
    }

    #[test]
    fn reads_changelog_options() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[changelog]\nshow_commit_hash = false\nshow_acknowledgments = false\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert!(!config.changelog_show_commit_hash);
        assert!(!config.changelog_show_acknowledgments);
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
}
