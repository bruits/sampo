use crate::errors::{Result, SampoError};
use crate::types::{Bump, PackageSpecifier, ParsedChangeType};
use changesets::Change;
use std::fs;
use std::path::{Path, PathBuf};

/// Information about a changeset file
#[derive(Debug, Clone)]
pub struct ChangesetInfo {
    pub path: PathBuf,
    /// (package, bump, optional_tag) tuples parsed from frontmatter.
    /// The tag is used for changelog categorization when custom tags are enabled.
    pub entries: Vec<(PackageSpecifier, Bump, Option<String>)>,
    pub message: String,
}

/// Parse a changeset from its markdown content.
/// Uses Knope's `changesets` crate to parse the frontmatter.
///
/// When `allowed_tags` is empty, only standard bump levels (patch, minor, major) are accepted.
/// When `allowed_tags` is non-empty, the format `bump (Tag)` is also accepted, where the tag
/// must be one of the allowed tags (case-insensitive).
///
/// # Example
/// ```rust,ignore
/// let text = "---\nmy-package: minor\n---\n\nfeat: new feature\n";
/// let info = parse_changeset(text, &Path::new("test.md"), &[]).unwrap();
/// assert_eq!(info.entries[0].0.to_canonical_string(), "my-package");
/// assert_eq!(info.entries[0].1, Bump::Minor);
/// assert_eq!(info.entries[0].2, None);
///
/// // With custom tags:
/// let text = "---\nmy-package: minor (Added)\n---\n\nfeat: new feature\n";
/// let allowed = vec!["Added".to_string()];
/// let info = parse_changeset(text, &Path::new("test.md"), &allowed).unwrap();
/// assert_eq!(info.entries[0].1, Bump::Minor);
/// assert_eq!(info.entries[0].2, Some("Added".to_string()));
/// ```
pub fn parse_changeset(
    text: &str,
    path: &Path,
    allowed_tags: &[String],
) -> Result<Option<ChangesetInfo>> {
    let file_name = path
        .file_name()
        .ok_or_else(|| SampoError::Changeset("Invalid file path".to_string()))?
        .to_string_lossy()
        .to_string();

    let change = Change::from_file_name_and_content(&file_name, text)
        .map_err(|err| SampoError::Changeset(format!("Failed to parse changeset: {}", err)))?;

    // Convert Change.versioning -> Vec<(PackageSpecifier, Bump, Option<String>)>
    let mut entries: Vec<(PackageSpecifier, Bump, Option<String>)> = Vec::new();
    for (package_name, change_type) in change.versioning.iter() {
        let (bump, tag) = parse_change_type(change_type, package_name, allowed_tags)?;
        let spec = PackageSpecifier::parse(package_name).map_err(|reason| {
            SampoError::Changeset(format!(
                "Invalid package reference '{}': {reason}",
                package_name
            ))
        })?;
        entries.push((spec, bump, tag));
    }
    if entries.is_empty() {
        return Ok(None);
    }

    let message = change.summary.trim().to_string();
    if message.is_empty() {
        return Ok(None);
    }

    Ok(Some(ChangesetInfo {
        path: path.to_path_buf(),
        entries,
        message,
    }))
}

/// Parse a changesets::ChangeType into (Bump, Option<String>).
///
/// For standard types (Patch, Minor, Major), returns the corresponding bump with no tag.
/// For Custom types, attempts to parse as "bump (Tag)" format.
fn parse_change_type(
    change_type: &changesets::ChangeType,
    package_name: &str,
    allowed_tags: &[String],
) -> Result<(Bump, Option<String>)> {
    match change_type {
        changesets::ChangeType::Patch => Ok((Bump::Patch, None)),
        changesets::ChangeType::Minor => Ok((Bump::Minor, None)),
        changesets::ChangeType::Major => Ok((Bump::Major, None)),
        changesets::ChangeType::Custom(custom_str) => {
            // Try to parse as "bump (Tag)" format
            match ParsedChangeType::parse(custom_str, allowed_tags) {
                Ok(parsed) => Ok((parsed.bump, parsed.tag)),
                Err(parse_err) => {
                    // If custom tags are enabled, give a helpful error
                    if !allowed_tags.is_empty() {
                        Err(SampoError::Changeset(format!(
                            "Invalid change type '{}' for package '{}': {}",
                            custom_str, package_name, parse_err
                        )))
                    } else {
                        // No custom tags configured, reject custom type
                        Err(SampoError::Changeset(format!(
                            "Unsupported change type '{}' for package '{}'. Only 'patch', 'minor', and 'major' are supported. \
                             To use custom tags like 'minor (Added)', configure changesets.tags in .sampo/config.toml.",
                            custom_str, package_name
                        )))
                    }
                }
            }
        }
    }
}

/// Load all changesets from a directory.
///
/// When `allowed_tags` is empty, only standard bump levels are accepted.
/// When non-empty, the "bump (Tag)" format is also valid.
pub fn load_changesets(dir: &Path, allowed_tags: &[String]) -> Result<Vec<ChangesetInfo>> {
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
        if let Some(changeset) = parse_changeset(&text, &path, allowed_tags)? {
            out.push(changeset);
        }
    }
    Ok(out)
}

/// Render a changeset as markdown with YAML mapping frontmatter.
///
/// This renders the standard bump format without tags.
/// For tagged changesets, use `render_changeset_markdown_with_tags`.
pub fn render_changeset_markdown(entries: &[(PackageSpecifier, Bump)], message: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    out.push_str("---\n");
    for (package, bump) in entries {
        let canonical = package.to_canonical_string();
        let _ = writeln!(out, "{}: {}", canonical, bump);
    }
    out.push_str("---\n\n");
    out.push_str(message);
    out.push('\n');
    out
}

/// Render a changeset as markdown with YAML mapping frontmatter, including optional tags.
///
/// Entries with tags are rendered as `package: bump (Tag)`.
pub fn render_changeset_markdown_with_tags(
    entries: &[(PackageSpecifier, Bump, Option<String>)],
    message: &str,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    out.push_str("---\n");
    for (package, bump, tag) in entries {
        let canonical = package.to_canonical_string();
        match tag {
            Some(t) => {
                let _ = writeln!(out, "{}: {} ({})", canonical, bump, t);
            }
            None => {
                let _ = writeln!(out, "{}: {}", canonical, bump);
            }
        }
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
        let path = Path::new("/tmp/x.md");
        let changeset = parse_changeset(text, path, &[]).unwrap().unwrap();
        let mut entries = changeset.entries.clone();
        entries.sort_by(|left, right| {
            left.0
                .to_canonical_string()
                .cmp(&right.0.to_canonical_string())
        });
        let collected: Vec<(String, Bump, Option<String>)> = entries
            .into_iter()
            .map(|(spec, bump, tag)| (spec.to_canonical_string(), bump, tag))
            .collect();
        assert_eq!(
            collected,
            vec![
                ("a".into(), Bump::Minor, None),
                ("b".into(), Bump::Minor, None)
            ]
        );
        assert_eq!(changeset.message, "feat: message");
    }

    #[test]
    fn parse_changeset_strips_wrapping_quotes() {
        let text = "---\n\"sampo-core\": minor\n'sampo-cli': patch\n---\n\nfeat: message\n";
        let path = Path::new("/tmp/quotes.md");
        let changeset = parse_changeset(text, path, &[]).unwrap().unwrap();
        let mut entries = changeset.entries;
        entries.sort_by(|left, right| {
            left.0
                .to_canonical_string()
                .cmp(&right.0.to_canonical_string())
        });
        let collected: Vec<(String, Bump, Option<String>)> = entries
            .into_iter()
            .map(|(spec, bump, tag)| (spec.to_canonical_string(), bump, tag))
            .collect();
        assert_eq!(
            collected,
            vec![
                ("sampo-cli".into(), Bump::Patch, None),
                ("sampo-core".into(), Bump::Minor, None)
            ]
        );
    }

    #[test]
    fn parse_changeset_accepts_canonical_identifiers_with_slash() {
        let text = "---\ncargo/example: minor\n---\n\nfeat: canonical id\n";
        let path = Path::new("/tmp/canonical-slash.md");
        let changeset = parse_changeset(text, path, &[]).unwrap().unwrap();
        let collected: Vec<(String, Bump, Option<String>)> = changeset
            .entries
            .iter()
            .map(|(spec, bump, tag)| (spec.to_canonical_string(), *bump, tag.clone()))
            .collect();
        assert_eq!(collected, vec![("cargo/example".into(), Bump::Minor, None)]);
    }

    #[test]
    fn parse_changeset_with_tags() {
        let allowed = vec!["Added".to_string(), "Fixed".to_string()];
        let text = "---\nmy-pkg: minor (Added)\n---\n\nfeat: new feature\n";
        let path = Path::new("/tmp/tagged.md");
        let changeset = parse_changeset(text, path, &allowed).unwrap().unwrap();
        assert_eq!(changeset.entries.len(), 1);
        assert_eq!(changeset.entries[0].1, Bump::Minor);
        assert_eq!(changeset.entries[0].2, Some("Added".to_string()));
    }

    #[test]
    fn parse_changeset_normalizes_tag_casing() {
        // User writes "added" but config has "Added" - should normalize to "Added"
        let allowed = vec!["Added".to_string(), "Fixed".to_string()];
        let text = "---\nmy-pkg: minor (added)\n---\n\nfeat: new feature\n";
        let path = Path::new("/tmp/lowercase-tag.md");
        let changeset = parse_changeset(text, path, &allowed).unwrap().unwrap();
        assert_eq!(changeset.entries[0].2, Some("Added".to_string()));

        // Also test uppercase
        let text = "---\nmy-pkg: patch (FIXED)\n---\n\nbug fix\n";
        let changeset = parse_changeset(text, path, &allowed).unwrap().unwrap();
        assert_eq!(changeset.entries[0].2, Some("Fixed".to_string()));
    }

    #[test]
    fn parse_changeset_rejects_unknown_tag_when_configured() {
        let allowed = vec!["Added".to_string()];
        let text = "---\nmy-pkg: minor (Unknown)\n---\n\nfeat: something\n";
        let path = Path::new("/tmp/unknown-tag.md");
        let result = parse_changeset(text, path, &allowed);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not in the configured changesets.tags list"));
    }

    #[test]
    fn parse_changeset_rejects_custom_type_without_config() {
        let text = "---\nmy-pkg: minor (Added)\n---\n\nfeat: something\n";
        let path = Path::new("/tmp/no-config.md");
        let result = parse_changeset(text, path, &[]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("configure changesets.tags"));
    }

    #[test]
    fn render_changeset_markdown_test() {
        let markdown = render_changeset_markdown(
            &[
                (PackageSpecifier::parse("a").unwrap(), Bump::Minor),
                (PackageSpecifier::parse("b").unwrap(), Bump::Minor),
            ],
            "feat: x",
        );
        assert!(markdown.starts_with("---\n"));
        assert!(markdown.contains("a: minor\n"));
        assert!(markdown.contains("b: minor\n"));
        assert!(markdown.contains("---\n\nfeat: x\n"));
    }

    #[test]
    fn render_changeset_markdown_with_canonical_identifier() {
        let markdown = render_changeset_markdown(
            &[(
                PackageSpecifier::parse("cargo/example").unwrap(),
                Bump::Minor,
            )],
            "feat: canonical",
        );
        assert!(markdown.contains("cargo/example: minor\n"));
    }

    // Test from sampo/changeset.rs - ensure compatibility
    #[test]
    fn render_changeset_markdown_compatibility() {
        let markdown = render_changeset_markdown(
            &[
                (PackageSpecifier::parse("a").unwrap(), Bump::Minor),
                (PackageSpecifier::parse("b").unwrap(), Bump::Minor),
            ],
            "feat: x",
        );
        assert!(markdown.starts_with("---\n"));
        assert!(markdown.contains("a: minor\n"));
        assert!(markdown.contains("b: minor\n"));
        assert!(markdown.ends_with("feat: x\n"));
    }

    #[test]
    fn render_changeset_markdown_strips_quoted_names() {
        let markdown = render_changeset_markdown(
            &[(
                PackageSpecifier::parse("\"sampo-core\"").unwrap(),
                Bump::Minor,
            )],
            "feat: sanitized",
        );
        assert!(markdown.contains("sampo-core: minor\n"));
        assert!(!markdown.contains("\"sampo-core\""));
    }

    #[test]
    fn parse_major_changeset() {
        let text = "---\nmypackage: major\n---\n\nBREAKING: API change\n";
        let path = Path::new("/tmp/major.md");
        let changeset = parse_changeset(text, path, &[]).unwrap().unwrap();
        let collected: Vec<(String, Bump)> = changeset
            .entries
            .iter()
            .map(|(spec, bump, _)| (spec.to_canonical_string(), *bump))
            .collect();
        assert_eq!(collected, vec![("mypackage".into(), Bump::Major)]);
        assert_eq!(changeset.message, "BREAKING: API change");
    }

    #[test]
    fn parse_empty_returns_error() {
        let text = "";
        let path = Path::new("/tmp/empty.md");
        assert!(parse_changeset(text, path, &[]).is_err());
    }

    #[test]
    fn load_changesets_empty_dir() {
        let temp = tempfile::tempdir().unwrap();
        let changesets = load_changesets(temp.path(), &[]).unwrap();
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

        let changesets = load_changesets(&changeset_dir, &[]).unwrap();
        assert_eq!(changesets.len(), 1);
        let entry = &changesets[0].entries[0];
        assert_eq!(entry.0.to_canonical_string(), "test");
        assert_eq!(entry.1, Bump::Patch);
    }

    #[test]
    fn parse_changeset_with_invalid_frontmatter() {
        let text = "packages:\n  - test\nrelease: patch\n---\n\nNo frontmatter delimiter\n";
        let path = Path::new("/tmp/invalid.md");
        assert!(parse_changeset(text, path, &[]).is_err());
    }

    #[test]
    fn parse_changeset_missing_packages() {
        let text = "---\n---\n\nNo packages defined\n";
        let path = Path::new("/tmp/no-packages.md");
        assert!(parse_changeset(text, path, &[]).is_err());
    }

    #[test]
    fn parse_changeset_missing_release() {
        // Non-semver change type should be rejected by our wrapper
        let text = "---\n\"test\": none\n---\n\nNo release type\n";
        let path = Path::new("/tmp/no-release.md");
        assert!(parse_changeset(text, path, &[]).is_err());
    }

    #[test]
    fn parse_changeset_empty_message() {
        let text = "---\ntest: patch\n---\n\n";
        let path = Path::new("/tmp/empty-message.md");
        assert!(parse_changeset(text, path, &[]).unwrap().is_none());
    }

    #[test]
    fn render_changeset_markdown_with_tags_test() {
        let markdown = render_changeset_markdown_with_tags(
            &[
                (
                    PackageSpecifier::parse("a").unwrap(),
                    Bump::Minor,
                    Some("Added".to_string()),
                ),
                (PackageSpecifier::parse("b").unwrap(), Bump::Patch, None),
            ],
            "feat: mixed",
        );
        assert!(markdown.contains("a: minor (Added)\n"));
        assert!(markdown.contains("b: patch\n"));
    }

    #[test]
    fn try_from_change_type_to_bump() {
        use changesets::ChangeType;

        // Test successful conversions
        assert_eq!(Bump::try_from(ChangeType::Patch), Ok(Bump::Patch));
        assert_eq!(Bump::try_from(ChangeType::Minor), Ok(Bump::Minor));
        assert_eq!(Bump::try_from(ChangeType::Major), Ok(Bump::Major));

        // Test rejection of custom types
        assert!(Bump::try_from(ChangeType::Custom("custom".to_string())).is_err());
    }
}
