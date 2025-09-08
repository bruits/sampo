use crate::errors::SampoError;
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            github_repository: None,
            changelog_show_commit_hash: true,
            changelog_show_acknowledgments: true,
            fixed_dependencies: Vec::new(),
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

        let text = std::fs::read_to_string(&path)?;
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
            .and_then(|t| t.get("fixed_dependencies"))
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
                            "fixed_dependencies must be an array of arrays, found mixed format with: {}. Use [[\"a\", \"b\"]] instead of [\"a\", \"b\"]",
                            non_array
                        ));
                    } else {
                        // All strings (flat format)
                        return Err(
                            "fixed_dependencies must be an array of arrays. Use [[\"a\", \"b\"]] instead of [\"a\", \"b\"]".to_string()
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
                let mut seen_packages = std::collections::HashSet::new();
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

        Ok(Self {
            version,
            github_repository,
            changelog_show_commit_hash,
            changelog_show_acknowledgments,
            fixed_dependencies,
        })
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
    fn reads_fixed_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nfixed_dependencies = [[\"pkg-a\", \"pkg-b\"]]\n",
        )
        .unwrap();

        let config = Config::load(temp.path()).unwrap();
        assert_eq!(
            config.fixed_dependencies,
            vec![vec!["pkg-a".to_string(), "pkg-b".to_string()]]
        );
    }

    #[test]
    fn reads_fixed_dependencies_groups() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".sampo")).unwrap();
        fs::write(
            temp.path().join(".sampo/config.toml"),
            "[packages]\nfixed_dependencies = [[\"pkg-a\", \"pkg-b\"], [\"pkg-c\", \"pkg-d\", \"pkg-e\"]]\n",
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
            "[packages]\nfixed_dependencies = [\"pkg-a\", \"pkg-b\"]\n",
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
            "[packages]\nfixed_dependencies = [[\"pkg-a\", \"pkg-b\"], [\"pkg-b\", \"pkg-c\"]]\n",
        )
        .unwrap();

        let result = Config::load(temp.path());
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("Package 'pkg-b' appears in multiple fixed dependency groups"));
    }
}
