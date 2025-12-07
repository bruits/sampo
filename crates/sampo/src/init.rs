use sampo_core::discover_packages_at;
use sampo_core::errors::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub struct InitReport {
    pub root: PathBuf,
    pub created_dir: bool,
    pub created_readme: bool,
    pub created_config: bool,
}

/// Initialize Sampo in the current working directory.
///
/// Unlike other commands, `init` works directly in `cwd` without walking up
/// the directory tree. The user must run it from their project root.
pub fn init_from_cwd(cwd: &Path) -> Result<InitReport> {
    // Check if there's a manifest in cwd (Cargo.toml, package.json, mix.exs)
    let packages = discover_packages_at(cwd)?;
    if packages.is_empty() {
        return Err(sampo_core::errors::SampoError::Workspace(
            sampo_core::errors::WorkspaceError::NotFound,
        ));
    }
    init_at_root(cwd)
}

fn init_at_root(root: &Path) -> Result<InitReport> {
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
        fs::write(&readme_path, README_SNIPPET)?;
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

const README_SNIPPET: &str = r#"# Sampo

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries.

## Quick links
- Documentation: https://github.com/bruits/sampo/blob/main/crates/sampo/README.md
- Getting started: https://github.com/bruits/sampo/blob/main/crates/sampo/README.md#getting-started
- Configuration: https://github.com/bruits/sampo/blob/main/crates/sampo/README.md#configuration
- Commands: https://github.com/bruits/sampo/blob/main/crates/sampo/README.md#commands
- GitHub Action (CI): https://github.com/bruits/sampo/blob/main/crates/sampo-github-action/README.md
- GitHub Bot: https://github.com/bruits/sampo/blob/main/crates/sampo-github-bot/README.md
"#;

const DEFAULT_CONFIG: &str = r#"# Sampo configuration
version = 1

[github]
# By default, Sampo tries to infer the repository from the git remote.
# You can override or clarify it here if needed.
# repository = "owner/repo"

[changelog]
# Options for release notes generation.
# show_commit_hash = true (default)
# show_acknowledgments = true (default)

[packages]
# Options for package discovery and filtering.
# ignore_unpublished = false (default)
# ignore = ["internal-*", "examples/*"]
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
