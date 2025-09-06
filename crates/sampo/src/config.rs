use std::io;
use std::path::Path;

pub struct Config {
    #[allow(dead_code)]
    pub version: u64,
    pub github_repository: Option<String>,
    pub changelog_show_commit_hash: bool,
    pub changelog_show_acknowledgments: bool,
}

impl Config {
    pub fn load(root: &Path) -> io::Result<Self> {
        let base = root.join(".sampo");
        let path = base.join("config.toml");
        if !path.exists() {
            // default config
            return Ok(Self {
                version: 1,
                github_repository: None,
                changelog_show_commit_hash: true,
                changelog_show_acknowledgments: true,
            });
        }

        let text = std::fs::read_to_string(&path)?;
        let value: toml::Value = text.parse().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid config.toml: {e}"),
            )
        })?;

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
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config::load(tmp.path()).unwrap();
        assert_eq!(cfg.version, 1);
        assert!(cfg.changelog_show_commit_hash);
        assert!(cfg.changelog_show_acknowledgments);
        assert_eq!(cfg.github_repository, None);
    }

    #[test]
    fn reads_changelog_options() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".sampo")).unwrap();
        fs::write(
            tmp.path().join(".sampo/config.toml"),
            "version=1\n[changelog]\nshow_commit_hash=false\nshow_acknowledgments=false\n",
        )
        .unwrap();
        let cfg = Config::load(tmp.path()).unwrap();
        assert!(!cfg.changelog_show_commit_hash);
        assert!(!cfg.changelog_show_acknowledgments);
        assert_eq!(cfg.github_repository, None);
    }

    #[test]
    fn reads_github_repository() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".sampo")).unwrap();
        fs::write(
            tmp.path().join(".sampo/config.toml"),
            "version=1\n[github]\nrepository=\"owner/repo\"\n",
        )
        .unwrap();
        let cfg = Config::load(tmp.path()).unwrap();
        assert_eq!(cfg.github_repository, Some("owner/repo".to_string()));
        assert!(cfg.changelog_show_commit_hash); // default
        assert!(cfg.changelog_show_acknowledgments); // default
    }

    #[test]
    fn reads_both_changelog_and_github() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".sampo")).unwrap();
        fs::write(
            tmp.path().join(".sampo/config.toml"),
            "version=1\n[changelog]\nshow_commit_hash=false\n[github]\nrepository=\"owner/repo\"\n",
        )
        .unwrap();
        let cfg = Config::load(tmp.path()).unwrap();
        assert!(!cfg.changelog_show_commit_hash);
        assert!(cfg.changelog_show_acknowledgments); // default when not specified
        assert_eq!(cfg.github_repository, Some("owner/repo".to_string()));
    }
}
