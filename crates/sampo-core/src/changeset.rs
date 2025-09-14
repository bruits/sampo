use crate::types::Bump;
use changesets::{Change, ChangeType};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Information about a changeset file
#[derive(Debug, Clone)]
pub struct ChangesetInfo {
    pub path: PathBuf,
    /// (package, bump) pairs parsed from frontmatter
    pub entries: Vec<(String, Bump)>,
    pub message: String,
}

/// Parse a changeset from its markdown content.
/// Uses Knope's `changesets` crate to parse the frontmatter.
///
/// # Example
/// ```rust,ignore
/// let text = "---\nmy-package: minor\n---\n\nfeat: new feature\n";
/// let info = parse_changeset(text, &Path::new("test.md")).unwrap();
/// assert_eq!(info.entries, vec![("my-package".into(), Bump::Minor)]);
/// ```
pub fn parse_changeset(text: &str, path: &Path) -> Option<ChangesetInfo> {
    let file_name = path.file_name()?.to_string_lossy().to_string();
    let change = Change::from_file_name_and_content(&file_name, text).ok()?;

    // Convert Change.versioning -> Vec<(String, Bump)>, rejecting non-semver change types.
    let mut entries: Vec<(String, Bump)> = Vec::new();
    for (pkg, ct) in change.versioning.iter() {
        let bump = match ct {
            ChangeType::Patch => Bump::Patch,
            ChangeType::Minor => Bump::Minor,
            ChangeType::Major => Bump::Major,
            ChangeType::Custom(_) => return None,
        };
        entries.push((pkg.clone(), bump));
    }
    if entries.is_empty() {
        return None;
    }

    let message = change.summary.trim().to_string();
    if message.is_empty() {
        return None;
    }

    Some(ChangesetInfo {
        path: path.to_path_buf(),
        entries,
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
        let text =
            fs::read_to_string(&path).map_err(|e| crate::errors::io_error_with_path(e, &path))?;
        if let Some(cs) = parse_changeset(&text, &path) {
            out.push(cs);
        }
    }
    Ok(out)
}

/// Render a changeset as markdown with YAML mapping frontmatter
pub fn render_changeset_markdown(packages: &[String], bump: Bump, message: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    out.push_str("---\n");
    for p in packages {
        let _ = writeln!(out, "{}: {}", p, bump);
    }
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
        let text = "---\na: minor\nb: minor\n---\n\nfeat: message\n";
        let p = Path::new("/tmp/x.md");
        let cs = parse_changeset(text, p).unwrap();
        let mut entries = cs.entries.clone();
        entries.sort_by(|l, r| l.0.cmp(&r.0));
        assert_eq!(
            entries,
            vec![("a".into(), Bump::Minor), ("b".into(), Bump::Minor)]
        );
        assert_eq!(cs.message, "feat: message");
    }

    #[test]
    fn render_changeset_markdown_test() {
        let s = render_changeset_markdown(&["a".into(), "b".into()], Bump::Minor, "feat: x");
        assert!(s.starts_with("---\n"));
        assert!(s.contains("a: minor\n"));
        assert!(s.contains("b: minor\n"));
        assert!(s.contains("---\n\nfeat: x\n"));
    }

    // Test from sampo/changeset.rs - ensure compatibility
    #[test]
    fn render_changeset_markdown_compatibility() {
        let s = render_changeset_markdown(&["a".into(), "b".into()], Bump::Minor, "feat: x");
        assert!(s.starts_with("---\n"));
        assert!(s.contains("a: minor\n"));
        assert!(s.contains("b: minor\n"));
        assert!(s.ends_with("feat: x\n"));
    }

    #[test]
    fn parse_major_changeset() {
        let text = "---\nmypackage: major\n---\n\nBREAKING: API change\n";
        let p = Path::new("/tmp/major.md");
        let cs = parse_changeset(text, p).unwrap();
        assert_eq!(cs.entries, vec![("mypackage".into(), Bump::Major)]);
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

    // Additional tests for comprehensive coverage
    #[test]
    fn load_changesets_filters_non_md_files() {
        let temp = tempfile::tempdir().unwrap();
        let changeset_dir = temp.path().join("changesets");
        fs::create_dir_all(&changeset_dir).unwrap();

        // Create a non-markdown file
        fs::write(changeset_dir.join("not-a-changeset.txt"), "invalid content").unwrap();

        // Create a valid changeset
        let valid_content = "---\ntest: patch\n---\n\nTest changeset\n";
        fs::write(changeset_dir.join("valid.md"), valid_content).unwrap();

        let changesets = load_changesets(&changeset_dir).unwrap();
        assert_eq!(changesets.len(), 1);
        assert_eq!(changesets[0].entries, vec![("test".into(), Bump::Patch)]);
    }

    #[test]
    fn parse_changeset_with_invalid_frontmatter() {
        let text = "packages:\n  - test\nrelease: patch\n---\n\nNo frontmatter delimiter\n";
        let p = Path::new("/tmp/invalid.md");
        assert!(parse_changeset(text, p).is_none());
    }

    #[test]
    fn parse_changeset_missing_packages() {
        let text = "---\n---\n\nNo packages defined\n";
        let p = Path::new("/tmp/no-packages.md");
        assert!(parse_changeset(text, p).is_none());
    }

    #[test]
    fn parse_changeset_missing_release() {
        // Non-semver change type should be rejected by our wrapper
        let text = "---\n\"test\": none\n---\n\nNo release type\n";
        let p = Path::new("/tmp/no-release.md");
        assert!(parse_changeset(text, p).is_none());
    }

    #[test]
    fn parse_changeset_empty_message() {
        let text = "---\ntest: patch\n---\n\n";
        let p = Path::new("/tmp/empty-message.md");
        assert!(parse_changeset(text, p).is_none());
    }
}
