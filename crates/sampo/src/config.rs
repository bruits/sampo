use std::io;
use std::path::{Path, PathBuf};

pub struct Config {
    #[allow(dead_code)]
    pub version: u64,
    pub changesets_dir: PathBuf,
}

impl Config {
    pub fn load(root: &Path) -> io::Result<Self> {
        let base = root.join(".sampo");
        let path = base.join("config.toml");
        if !path.exists() {
            // default config
            return Ok(Self {
                version: 1,
                changesets_dir: base.join("changesets"),
            });
        }

        let text = std::fs::read_to_string(&path)?;
        let value: toml::Value = text.parse().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid config.toml: {}", e),
            )
        })?;

        let version = value
            .get("version")
            .and_then(|v| v.as_integer())
            .unwrap_or(1) as u64;

        let dir_str = value
            .get("changesets")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("dir"))
            .and_then(|v| v.as_str())
            .unwrap_or("changesets");

        Ok(Self {
            version,
            changesets_dir: base.join(dir_str),
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
        assert!(cfg.changesets_dir.ends_with(".sampo/changesets"));
    }

    #[test]
    fn reads_changesets_dir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".sampo")).unwrap();
        fs::write(
            tmp.path().join(".sampo/config.toml"),
            "version=1\n[changesets]\ndir=\"notes\"\n",
        )
        .unwrap();
        let cfg = Config::load(tmp.path()).unwrap();
        assert!(cfg.changesets_dir.ends_with(".sampo/notes"));
    }
}
