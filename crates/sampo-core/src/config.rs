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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            github_repository: None,
            changelog_show_commit_hash: true,
            changelog_show_acknowledgments: true,
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

        Ok(Self {
            version,
            github_repository,
            changelog_show_commit_hash,
            changelog_show_acknowledgments,
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
}
