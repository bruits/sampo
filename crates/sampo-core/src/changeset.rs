use crate::types::Bump;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Information about a changeset file
#[derive(Debug, Clone)]
pub struct ChangesetInfo {
    pub path: PathBuf,
    pub packages: Vec<String>,
    pub bump: Bump,
    pub message: String,
}

/// Parse a changeset from its markdown content
pub fn parse_changeset(text: &str, path: &Path) -> Option<ChangesetInfo> {
    // Expect frontmatter delimited by --- lines, with keys: packages (list), release (string)
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut packages: Vec<String> = Vec::new();
    let mut bump: Option<Bump> = None;
    let mut in_packages = false;
    for line in &mut lines {
        let l = line.trim();
        if l == "---" {
            break;
        }
        if l.starts_with("packages:") {
            in_packages = true;
            continue;
        }
        if in_packages {
            // list items like "- name"
            if let Some(rest) = l.strip_prefix('-') {
                let name = rest.trim().to_string();
                if !name.is_empty() {
                    packages.push(name);
                }
                continue;
            } else if !l.is_empty() {
                // a non-list line ends the packages block
                in_packages = false;
            }
        }
        if let Some(v) = l.strip_prefix("release:")
            && let Some(b) = Bump::parse(v.trim())
        {
            bump = Some(b);
        }
    }

    // The remainder after the second --- is the message
    let remainder: String = lines.collect::<Vec<_>>().join("\n");
    let message = remainder.trim().to_string();
    if packages.is_empty() || bump.is_none() || message.is_empty() {
        return None;
    }
    Some(ChangesetInfo {
        path: path.to_path_buf(),
        packages,
        bump: bump.unwrap(),
        message,
    })
}

/// Load all changesets from a directory
pub fn load_changesets(dir: &Path) -> io::Result<Vec<ChangesetInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        if let Some(cs) = parse_changeset(&text, &path) {
            out.push(cs);
        }
    }
    Ok(out)
}

/// Detect the changesets directory, respecting custom configuration
pub fn detect_changesets_dir(workspace: &Path) -> PathBuf {
    let base = workspace.join(".sampo");
    let cfg_path = base.join("config.toml");
    if cfg_path.exists()
        && let Ok(text) = std::fs::read_to_string(&cfg_path)
        && let Ok(value) = text.parse::<toml::Value>()
        && let Some(dir) = value
            .get("changesets")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("dir"))
            .and_then(|v| v.as_str())
    {
        return base.join(dir);
    }
    base.join("changesets")
}

/// Render a changeset as markdown with frontmatter
pub fn render_changeset_markdown(packages: &[String], bump: Bump, message: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("packages:\n");
    for p in packages {
        let _ = writeln!(out, "  - {}", p);
    }
    let _ = writeln!(out, "release: {}", bump);
    out.push_str("---\n\n");
    out.push_str(message);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_changeset() {
        let text = "---\npackages:\n  - a\n  - b\nrelease: minor\n---\n\nfeat: message\n";
        let p = Path::new("/tmp/x.md");
        let cs = parse_changeset(text, p).unwrap();
        assert_eq!(cs.packages, vec!["a", "b"]);
        assert_eq!(cs.bump, Bump::Minor);
        assert_eq!(cs.message, "feat: message");
    }

    #[test]
    fn render_changeset_markdown_test() {
        let s = render_changeset_markdown(&["a".into(), "b".into()], Bump::Minor, "feat: x");
        assert!(s.starts_with("---\n"));
        assert!(s.contains("packages:\n  - a\n  - b\n"));
        assert!(s.contains("release: minor\n"));
        assert!(s.contains("---\n\nfeat: x\n"));
    }

    // Test from sampo/changeset.rs - ensure compatibility
    #[test]
    fn render_changeset_markdown_compatibility() {
        let s = render_changeset_markdown(&["a".into(), "b".into()], Bump::Minor, "feat: x");
        assert!(s.starts_with("---\n"));
        assert!(s.contains("packages:\n  - a\n  - b\n"));
        assert!(s.contains("release: minor\n"));
        assert!(s.ends_with("feat: x\n"));
    }

    #[test]
    fn parse_major_changeset() {
        let text = "---\npackages:\n  - mypackage\nrelease: major\n---\n\nBREAKING: API change\n";
        let p = Path::new("/tmp/major.md");
        let cs = parse_changeset(text, p).unwrap();
        assert_eq!(cs.packages, vec!["mypackage"]);
        assert_eq!(cs.bump, Bump::Major);
        assert_eq!(cs.message, "BREAKING: API change");
    }

    #[test]
    fn parse_empty_returns_none() {
        let text = "";
        let p = Path::new("/tmp/empty.md");
        assert!(parse_changeset(text, p).is_none());
    }

    #[test]
    fn load_changesets_empty_dir() {
        let temp = tempfile::tempdir().unwrap();
        let changesets = load_changesets(temp.path()).unwrap();
        assert!(changesets.is_empty());
    }

    #[test]
    fn detect_changesets_dir_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let dir = detect_changesets_dir(temp.path());
        assert_eq!(dir, temp.path().join(".sampo/changesets"));
    }

    #[test]
    fn detect_changesets_dir_custom() {
        let temp = tempfile::tempdir().unwrap();
        let sampo_dir = temp.path().join(".sampo");
        fs::create_dir_all(&sampo_dir).unwrap();
        fs::write(
            sampo_dir.join("config.toml"),
            "[changesets]\ndir = \"custom-changesets\"\n",
        )
        .unwrap();

        let dir = detect_changesets_dir(temp.path());
        assert_eq!(dir, temp.path().join(".sampo/custom-changesets"));
    }

    // Additional tests for comprehensive coverage
    #[test]
    fn load_changesets_filters_non_md_files() {
        let temp = tempfile::tempdir().unwrap();
        let changeset_dir = temp.path().join("changesets");
        fs::create_dir_all(&changeset_dir).unwrap();

        // Create a non-markdown file
        fs::write(changeset_dir.join("not-a-changeset.txt"), "invalid content").unwrap();

        // Create a valid changeset
        let valid_content = "---\npackages:\n  - test\nrelease: patch\n---\n\nTest changeset\n";
        fs::write(changeset_dir.join("valid.md"), valid_content).unwrap();

        let changesets = load_changesets(&changeset_dir).unwrap();
        assert_eq!(changesets.len(), 1);
        assert_eq!(changesets[0].packages, vec!["test"]);
    }

    #[test]
    fn parse_changeset_with_invalid_frontmatter() {
        let text = "packages:\n  - test\nrelease: patch\n---\n\nNo frontmatter delimiter\n";
        let p = Path::new("/tmp/invalid.md");
        assert!(parse_changeset(text, p).is_none());
    }

    #[test]
    fn parse_changeset_missing_packages() {
        let text = "---\nrelease: patch\n---\n\nNo packages defined\n";
        let p = Path::new("/tmp/no-packages.md");
        assert!(parse_changeset(text, p).is_none());
    }

    #[test]
    fn parse_changeset_missing_release() {
        let text = "---\npackages:\n  - test\n---\n\nNo release type\n";
        let p = Path::new("/tmp/no-release.md");
        assert!(parse_changeset(text, p).is_none());
    }

    #[test]
    fn parse_changeset_empty_message() {
        let text = "---\npackages:\n  - test\nrelease: patch\n---\n\n";
        let p = Path::new("/tmp/empty-message.md");
        assert!(parse_changeset(text, p).is_none());
    }
}
