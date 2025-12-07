use crate::error::{BotError, Result};
use octocrab::models::repos::{DiffEntry, DiffEntryStatus};
use sampo_core::changeset::{ChangesetInfo, parse_changeset};
use sampo_core::types::{Bump, PackageSpecifier};
use std::collections::BTreeMap;
use std::path::Path;

pub struct ChangesetAnalysis {
    pub has_changeset: bool,
    pub comment_markdown: String,
}

struct ChangesetFile {
    path: String,
    content: String,
}

struct ParsedChangesets {
    valid: Vec<ChangesetInfo>,
    issues: Vec<ChangesetIssue>,
}

#[derive(Clone)]
struct ChangesetIssue {
    path: String,
    reason: String,
}

struct PackagePreview {
    spec: PackageSpecifier,
    highest_bump: Bump,
    major_changes: Vec<String>,
    minor_changes: Vec<String>,
    patch_changes: Vec<String>,
}

impl PackagePreview {
    fn new(spec: PackageSpecifier) -> Self {
        Self {
            spec,
            highest_bump: Bump::Patch,
            major_changes: Vec::new(),
            minor_changes: Vec::new(),
            patch_changes: Vec::new(),
        }
    }

    fn register_change(&mut self, bump: Bump, message: &str) {
        if bump_priority(bump) > bump_priority(self.highest_bump) {
            self.highest_bump = bump;
        }
        let target = match bump {
            Bump::Major => &mut self.major_changes,
            Bump::Minor => &mut self.minor_changes,
            Bump::Patch => &mut self.patch_changes,
        };
        if !target.iter().any(|existing| existing == message) {
            target.push(message.to_string());
        }
    }
}

pub async fn analyze_pr_changesets(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
    head_ref: &str,
) -> Result<ChangesetAnalysis> {
    let files = collect_changeset_files(octo, owner, repo, pr, head_ref).await?;

    if files.is_empty() {
        let comment = build_missing_changeset_comment(&[]);
        return Ok(ChangesetAnalysis {
            has_changeset: false,
            comment_markdown: comment,
        });
    }

    let parsed = parse_changeset_files(&files);

    if parsed.valid.is_empty() {
        let comment = build_missing_changeset_comment(&parsed.issues);
        return Ok(ChangesetAnalysis {
            has_changeset: false,
            comment_markdown: comment,
        });
    }

    let packages = summarize_packages(&parsed.valid);
    let comment = build_present_changeset_comment(&packages, &parsed.issues);

    Ok(ChangesetAnalysis {
        has_changeset: true,
        comment_markdown: comment,
    })
}

fn build_missing_changeset_comment(issues: &[ChangesetIssue]) -> String {
    let mut out = String::new();
    out.push_str("## ⚠️ No changeset detected\n\n");
    out.push_str("No new `.sampo/changesets/*.md` files were detected in this PR. To add one:\n\n");
    out.push_str("- run `sampo add`\n");
    out.push_str("- follow the prompts to pick the affected packages and describe the change\n");
    out.push_str(
        "- commit the generated file to this pull request, and it will be detected automatically\n\n",
    );
    out.push_str("If this PR isn't supposed to introduce any changes (e.g., documentation updates), you can ignore this message.\n");

    append_issue_section(&mut out, issues);
    out
}

fn build_present_changeset_comment(
    packages: &BTreeMap<String, PackagePreview>,
    issues: &[ChangesetIssue],
) -> String {
    let mut out = String::new();
    out.push_str("## 🧭 Changeset detected\n\n");
    out.push_str("Merging this PR will release the following updates:\n\n");

    for package in packages.values() {
        append_package_preview(&mut out, package);
    }

    append_issue_section(&mut out, issues);

    out
}

fn append_package_preview(out: &mut String, package: &PackagePreview) {
    let include_kind = package.spec.kind.is_some();
    let display_name = package.spec.display_name(include_kind);
    let bump_label = match package.highest_bump {
        Bump::Major => "major version bump",
        Bump::Minor => "minor version bump",
        Bump::Patch => "patch version bump",
    };
    out.push_str(&format!("## {display_name} — {bump_label}\n\n"));

    append_changes_section(out, "Major changes", &package.major_changes);
    append_changes_section(out, "Minor changes", &package.minor_changes);
    append_changes_section(out, "Patch changes", &package.patch_changes);
}

fn append_changes_section(out: &mut String, title: &str, changes: &[String]) {
    if changes.is_empty() {
        return;
    }
    out.push_str(&format!("### {title}\n"));
    for change in changes {
        out.push_str("- ");
        out.push_str(change);
        out.push('\n');
    }
    out.push('\n');
}

fn append_issue_section(out: &mut String, issues: &[ChangesetIssue]) {
    if issues.is_empty() {
        return;
    }
    out.push_str("Detected issues with these files:\n");
    for issue in issues {
        out.push_str(&format!("- `{}`: {}\n", issue.path, issue.reason));
    }
}

fn summarize_packages(changesets: &[ChangesetInfo]) -> BTreeMap<String, PackagePreview> {
    let mut packages = BTreeMap::new();
    for cs in changesets {
        for (spec, bump, _tag) in &cs.entries {
            let key = spec.to_canonical_string();
            let preview = packages
                .entry(key)
                .or_insert_with(|| PackagePreview::new(spec.clone()));
            preview.register_change(*bump, &cs.message);
        }
    }
    packages
}

async fn collect_changeset_files(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
    head_ref: &str,
) -> Result<Vec<ChangesetFile>> {
    let mut files = Vec::new();
    let mut page = octo
        .pulls(owner, repo)
        .list_files(pr)
        .await
        .map_err(BotError::from_pr_files)?;

    let dir_prefix = ".sampo/changesets/";

    loop {
        for entry in &page {
            if !is_new_changeset(entry, dir_prefix) {
                continue;
            }
            let content = extract_changeset_content(octo, owner, repo, entry, head_ref).await?;
            files.push(ChangesetFile {
                path: entry.filename.clone(),
                content,
            });
        }

        if let Some(next_page) = octo
            .get_page::<DiffEntry>(&page.next)
            .await
            .map_err(BotError::from_pr_files)?
        {
            page = next_page;
        } else {
            break;
        }
    }

    Ok(files)
}

fn is_new_changeset(entry: &DiffEntry, dir_prefix: &str) -> bool {
    entry.filename.starts_with(dir_prefix)
        && entry.filename.ends_with(".md")
        && matches!(entry.status, DiffEntryStatus::Added)
}

async fn extract_changeset_content(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    entry: &DiffEntry,
    head_ref: &str,
) -> Result<String> {
    if let Some(patch) = &entry.patch
        && let Some(content) = content_from_patch(patch)
    {
        return Ok(content);
    }

    let mut response = octo
        .repos(owner, repo)
        .get_content()
        .path(&entry.filename)
        .r#ref(head_ref)
        .send()
        .await
        .map_err(|err| BotError::Internal(format!("failed to fetch changeset content: {err}")))?;

    let items = response.take_items();
    let content = items
        .first()
        .and_then(|item| item.decoded_content())
        .ok_or_else(|| {
            BotError::Internal(format!(
                "failed to decode content for changeset {}",
                entry.filename
            ))
        })?;
    Ok(content)
}

fn content_from_patch(patch: &str) -> Option<String> {
    let mut content = String::new();
    let mut has_lines = false;
    for line in patch.lines() {
        if line.starts_with("@@") || line.starts_with("---") || line.starts_with("+++") {
            continue;
        }
        if line.starts_with('\\') {
            continue;
        }
        if let Some(stripped) = line.strip_prefix('+') {
            content.push_str(stripped);
            content.push('\n');
            has_lines = true;
        }
    }
    if has_lines { Some(content) } else { None }
}

fn parse_changeset_files(files: &[ChangesetFile]) -> ParsedChangesets {
    let mut valid = Vec::new();
    let mut issues = Vec::new();

    // Bot doesn't have access to config, so we pass empty allowed_tags
    // Tags will be validated later by the actual release process
    let allowed_tags: Vec<String> = Vec::new();

    for file in files {
        match parse_changeset(&file.content, Path::new(&file.path), &allowed_tags) {
            Ok(Some(info)) => valid.push(info),
            Ok(None) => issues.push(ChangesetIssue {
                path: file.path.clone(),
                reason: "changeset missing package entries or summary message".to_string(),
            }),
            Err(err) => issues.push(ChangesetIssue {
                path: file.path.clone(),
                reason: err.to_string(),
            }),
        }
    }

    ParsedChangesets { valid, issues }
}

fn bump_priority(bump: Bump) -> u8 {
    match bump {
        Bump::Patch => 0,
        Bump::Minor => 1,
        Bump::Major => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sampo_core::types::PackageKind;

    #[test]
    fn content_from_patch_extracts_new_file() {
        let patch = "\
diff --git a/.sampo/changesets/example.md b/.sampo/changesets/example.md
new file mode 100644
index 0000000..f1c2d3e
--- /dev/null
+++ b/.sampo/changesets/example.md
@@ -0,0 +1,5 @@
+---
+cargo/example: minor
+---
+
+feat: add new feature
";
        let content = content_from_patch(patch).expect("content expected");
        assert!(content.contains("cargo/example: minor"));
        assert!(content.contains("feat: add new feature"));
    }

    #[test]
    fn summarize_packages_deduplicates_messages() {
        let spec = PackageSpecifier {
            kind: Some(PackageKind::Cargo),
            name: "example".to_string(),
        };
        let info = ChangesetInfo {
            path: Path::new(".sampo/changesets/example.md").to_path_buf(),
            entries: vec![(spec.clone(), Bump::Minor, None)],
            message: "feat: add new feature".to_string(),
        };
        let packages = summarize_packages(&[info.clone(), info]);
        let preview = packages
            .get(&spec.to_canonical_string())
            .expect("preview present");
        assert_eq!(preview.highest_bump, Bump::Minor);
        assert_eq!(preview.minor_changes.len(), 1);
    }

    #[test]
    fn append_package_preview_includes_kind() {
        let mut out = String::new();
        let mut package =
            PackagePreview::new(PackageSpecifier::parse("cargo/example").expect("valid specifier"));
        package.register_change(Bump::Patch, "fix: bug");
        append_package_preview(&mut out, &package);
        assert!(out.contains("example (Cargo)"));
        assert!(out.contains("patch version bump"));
        assert!(out.contains("- fix: bug"));
    }

    #[test]
    fn missing_changeset_comment_includes_instructions() {
        let comment = build_missing_changeset_comment(&[]);
        assert!(comment.contains("No changeset detected"));
        assert!(comment.contains("run `sampo add`"));
        assert!(comment.contains("commit the generated file to this pull request"));
    }

    #[test]
    fn present_changeset_comment_lists_packages() {
        let info = ChangesetInfo {
            path: Path::new(".sampo/changesets/example.md").to_path_buf(),
            entries: vec![(
                PackageSpecifier::parse("cargo/example").expect("valid specifier"),
                Bump::Minor,
                None,
            )],
            message: "feat: add new capability".to_string(),
        };
        let packages = summarize_packages(&[info]);
        let comment = build_present_changeset_comment(&packages, &[]);

        assert!(comment.contains("Changeset detected"));
        assert!(comment.contains("## example (Cargo) — minor version bump"));
        assert!(comment.contains("### Minor changes"));
        assert!(comment.contains("- feat: add new capability"));
    }

    #[test]
    fn issue_section_mentions_problematic_files() {
        let issues = vec![ChangesetIssue {
            path: ".sampo/changesets/broken.md".to_string(),
            reason: "invalid frontmatter".to_string(),
        }];
        let comment = build_present_changeset_comment(&BTreeMap::new(), &issues);
        assert!(comment.contains("issues with these files"));
        assert!(comment.contains("`.sampo/changesets/broken.md`"));
        assert!(comment.contains("invalid frontmatter"));
    }
}
