use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub struct InitReport {
    pub root: PathBuf,
    pub created_dir: bool,
    pub created_readme: bool,
    pub created_config: bool,
}

pub fn init_from_cwd(cwd: &Path) -> io::Result<InitReport> {
    let root = match crate::workspace::Workspace::discover_from(cwd) {
        Ok(ws) => ws.root,
        Err(_) => cwd.to_path_buf(),
    };
    init_at_root(&root)
}

fn init_at_root(root: &Path) -> io::Result<InitReport> {
    let dir = root.join(".sampo");

    let mut created_dir = false;
    let mut created_readme = false;
    let mut created_config = false;

    if !dir.exists() {
        fs::create_dir_all(&dir)?;
        created_dir = true;
    }

    let readme_path = dir.join("README.md");
    if !readme_path.exists() {
        fs::write(&readme_path, CRATE_README)?;
        created_readme = true;
    }

    let config_path = dir.join("config.toml");
    if !config_path.exists() {
        fs::write(&config_path, DEFAULT_CONFIG)?;
        created_config = true;
    }

    Ok(InitReport {
        root: root.to_path_buf(),
        created_dir,
        created_readme,
        created_config,
    })
}

// Embed the crate's README so `sampo init` can copy it into `.sampo/README.md`
// regardless of how the binary is installed.
const CRATE_README: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"));

const DEFAULT_CONFIG: &str = r#"# Sampo configuration
version = 1

[changesets]
# Relative to the `.sampo` directory
dir = "changesets"
"#;

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn init_creates_dir_and_files_idempotently() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Add a minimal workspace file so discovery would pass if used
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();

        let r1 = super::init_at_root(root).unwrap();
        assert!(r1.created_dir);
        assert!(r1.created_readme);
        assert!(r1.created_config);

        // Running again should not recreate existing files
        let r2 = super::init_at_root(root).unwrap();
        assert!(!r2.created_dir);
        assert!(!r2.created_readme);
        assert!(!r2.created_config);

        assert!(root.join(".sampo/README.md").exists());
        assert!(root.join(".sampo/config.toml").exists());
    }
}
