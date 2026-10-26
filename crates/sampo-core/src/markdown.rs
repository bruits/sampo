//! Utilities to render Markdown consistently across crates.

use crate::types::{Bump, ChangelogCategory};
use std::collections::BTreeMap;

/// A row of the package table that opens the changeset layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSummary {
    pub name: String,
    pub bump: Bump,
    /// Old and new versions, known once the release is planned.
    pub versions: Option<(String, String)>,
}

/// A changelog entry with each package it applies to and that package's category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEntry {
    pub message: String,
    pub packages: Vec<(String, ChangelogCategory)>,
}

/// Format a Markdown list item for a changeset message.
///
/// - Ensures multi-line messages are indented so that subsequent lines are
///   rendered as part of the same list item.
/// - If the message itself contains list items (e.g. lines starting with "- "),
///   they become properly nested under the changeset item.
/// - Always ends with a trailing newline.
pub fn format_markdown_list_item(message: &str) -> String {
    let mut out = String::new();
    let mut lines = message.lines();
    if let Some(first) = lines.next() {
        out.push_str("- ");
        out.push_str(first);
        out.push('\n');
    } else {
        // TODO: should empty messages be allowed? If so, how should they be rendered?
        // For now, render as an empty list item. At some point, this subject will be
        // brought up for discussion.
        out.push_str("- \n");
        return out;
    }

    // Indent continuation lines by two spaces so they remain part of the same
    // list item in Markdown. Nested list markers will be correctly nested.
    for line in lines {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }

    out
}

/// Compose a Markdown message with an optional prefix and suffix.
///
/// Ensures the suffix does not break closing code fences: when the message ends
/// with a triple backtick fence (```), the suffix is put on a new line.
pub fn compose_markdown_with_affixes(message: &str, prefix: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return format!("{}{}", prefix, message);
    }

    let ends_with_fence = message.trim_end().ends_with("```");
    if ends_with_fence {
        format!("{}{}\n{}", prefix, message, suffix)
    } else {
        format!("{}{}{}", prefix, message, suffix)
    }
}

/// Append a `### title` section listing the changes, if there are any.
pub fn append_changes_section(out: &mut String, title: &str, changes: &[String]) {
    if changes.is_empty() {
        return;
    }
    out.push_str(&format!("### {title}\n\n"));
    for change in changes {
        out.push_str(&format_markdown_list_item(change));
    }
    out.push('\n');
}

/// Append a table of the packages, then each entry once, under the section of its
/// highest category and followed by the packages it applies to.
///
/// Packages are listed by name, and a package's category is named only where it differs
/// from the section's. Entries with the same message are merged, as the per-package
/// layout lists such a message once.
pub fn append_changes_by_changeset(
    out: &mut String,
    packages: &[PackageSummary],
    entries: &[ChangeEntry],
) {
    append_package_table(out, packages);

    let mut merged: Vec<ChangeEntry> = Vec::new();
    for entry in entries {
        let index = match merged.iter().position(|m| m.message == entry.message) {
            Some(index) => index,
            None => {
                merged.push(ChangeEntry {
                    message: entry.message.clone(),
                    packages: Vec::new(),
                });
                merged.len() - 1
            }
        };
        let merged_packages = &mut merged[index].packages;
        for (name, category) in &entry.packages {
            match merged_packages.iter_mut().find(|(n, _)| n == name) {
                Some((_, existing)) if category < existing => *existing = category.clone(),
                Some(_) => {}
                None => merged_packages.push((name.clone(), category.clone())),
            }
        }
    }

    let mut sections: BTreeMap<ChangelogCategory, Vec<String>> = BTreeMap::new();
    for mut entry in merged {
        // Changeset frontmatter is parsed into a hash map, so its order isn't kept.
        entry
            .packages
            .sort_by(|(left, _), (right, _)| left.cmp(right));
        let Some(section) = entry.packages.iter().map(|(_, c)| c).min().cloned() else {
            continue;
        };
        let names: Vec<String> = entry
            .packages
            .iter()
            .map(|(name, category)| {
                if *category == section {
                    name.clone()
                } else {
                    format!("{name} ({category})")
                }
            })
            .collect();
        // After a multi-line message, a separate paragraph keeps the packages line from
        // joining the message's last paragraph or nested list item.
        let separator = if entry.message.contains('\n') {
            "\n\n"
        } else {
            "\n"
        };
        sections.entry(section).or_default().push(format!(
            "{}{separator}Packages: {}",
            entry.message,
            names.join(", ")
        ));
    }

    for (category, changes) in &sections {
        append_changes_section(out, &category.heading(), changes);
    }
}

fn append_package_table(out: &mut String, packages: &[PackageSummary]) {
    if packages.is_empty() {
        return;
    }
    let with_versions = packages.iter().any(|p| p.versions.is_some());
    if with_versions {
        out.push_str("| Package | Bump | Version |\n| --- | --- | --- |\n");
    } else {
        out.push_str("| Package | Bump |\n| --- | --- |\n");
    }
    for package in packages {
        out.push_str(&format!("| {} | {} |", package.name, package.bump));
        if with_versions {
            match &package.versions {
                Some((old, new)) => out.push_str(&format!(" {old} -> {new} |")),
                None => out.push_str("  |"),
            }
        }
        out.push('\n');
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_item_single_line() {
        let out = format_markdown_list_item("feat: add new feature");
        assert_eq!(out, "- feat: add new feature\n");
    }

    #[test]
    fn list_item_multiline_with_nested_list() {
        let msg = "feat: big change\n- add A\n- add B";
        let out = format_markdown_list_item(msg);
        let expected = "- feat: big change\n  - add A\n  - add B\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn list_item_with_empty_message() {
        let out = format_markdown_list_item("");
        assert_eq!(out, "- \n");
    }

    #[test]
    fn compose_affixes_simple() {
        let msg = compose_markdown_with_affixes(
            "feat: add new feature",
            "[abcd](link) ",
            " — Thanks @user!",
        );
        assert_eq!(msg, "[abcd](link) feat: add new feature — Thanks @user!");
    }

    #[test]
    fn compose_affixes_preserves_code_fence() {
        let message = "Here is code:\n```rust\nfn main() {}\n```";
        let result = compose_markdown_with_affixes(message, "[abcd](link) ", " — Thanks @user!");
        let expected = "[abcd](link) Here is code:\n```rust\nfn main() {}\n```\n — Thanks @user!";
        assert_eq!(result, expected);
    }

    fn summary(name: &str, bump: Bump, versions: Option<(&str, &str)>) -> PackageSummary {
        PackageSummary {
            name: name.to_string(),
            bump,
            versions: versions.map(|(old, new)| (old.to_string(), new.to_string())),
        }
    }

    fn entry(message: &str, packages: &[(&str, ChangelogCategory)]) -> ChangeEntry {
        ChangeEntry {
            message: message.to_string(),
            packages: packages
                .iter()
                .map(|(name, category)| (name.to_string(), category.clone()))
                .collect(),
        }
    }

    const MAJOR: ChangelogCategory = ChangelogCategory::Bump(Bump::Major);
    const MINOR: ChangelogCategory = ChangelogCategory::Bump(Bump::Minor);
    const PATCH: ChangelogCategory = ChangelogCategory::Bump(Bump::Patch);

    #[test]
    fn changes_by_changeset_lists_each_entry_once_with_its_packages() {
        let mut out = String::new();
        append_changes_by_changeset(
            &mut out,
            &[
                summary("a", Bump::Minor, None),
                summary("b", Bump::Minor, None),
            ],
            &[
                entry("feat: shared", &[("a", MINOR), ("b", MINOR)]),
                entry("fix: only b", &[("b", PATCH)]),
            ],
        );
        assert_eq!(
            out,
            "| Package | Bump |\n| --- | --- |\n| a | minor |\n| b | minor |\n\n\
             ### Minor changes\n\n- feat: shared\n  Packages: a, b\n\n\
             ### Patch changes\n\n- fix: only b\n  Packages: b\n\n"
        );
    }

    #[test]
    fn changes_by_changeset_uses_highest_bump_and_names_differing_ones() {
        let mut out = String::new();
        append_changes_by_changeset(
            &mut out,
            &[],
            &[entry(
                "feat: mixed",
                &[("a", PATCH), ("b", MAJOR), ("c", MINOR), ("d", MAJOR)],
            )],
        );
        assert_eq!(
            out,
            "### Major changes\n\n- feat: mixed\n  Packages: a (patch), b, c (minor), d\n\n"
        );
    }

    #[test]
    fn changes_by_changeset_shows_versions_in_table() {
        let mut out = String::new();
        append_changes_by_changeset(
            &mut out,
            &[
                summary("a", Bump::Minor, Some(("1.0.0", "1.1.0"))),
                summary("b", Bump::Patch, None),
            ],
            &[],
        );
        assert_eq!(
            out,
            "| Package | Bump | Version |\n| --- | --- | --- |\n| a | minor | 1.0.0 -> 1.1.0 |\n| b | patch |  |\n\n"
        );
    }

    #[test]
    fn changes_by_changeset_separates_packages_from_multiline_messages() {
        let mut out = String::new();
        append_changes_by_changeset(
            &mut out,
            &[],
            &[entry("feat: big change\n- add A\n- add B", &[("a", MINOR)])],
        );
        assert_eq!(
            out,
            "### Minor changes\n\n- feat: big change\n  - add A\n  - add B\n  \n  Packages: a\n\n"
        );
    }

    #[test]
    fn changes_by_changeset_merges_identical_messages() {
        let mut out = String::new();
        append_changes_by_changeset(
            &mut out,
            &[],
            &[
                entry("Updated dependencies: c@1.1.0", &[("a", PATCH)]),
                entry("Updated dependencies: c@1.1.0", &[("b", PATCH)]),
                entry("feat: same text", &[("a", PATCH)]),
                entry("feat: same text", &[("a", MINOR), ("b", PATCH)]),
            ],
        );
        assert_eq!(
            out,
            "### Minor changes\n\n- feat: same text\n  Packages: a, b (patch)\n\n\
             ### Patch changes\n\n- Updated dependencies: c@1.1.0\n  Packages: a, b\n\n"
        );
    }

    #[test]
    fn changes_by_changeset_uses_tags_as_sections() {
        let added = ChangelogCategory::Tag("Added".to_string());
        let fixed = ChangelogCategory::Tag("Fixed".to_string());
        let mut out = String::new();
        append_changes_by_changeset(
            &mut out,
            &[],
            &[
                entry("feat: tagged", &[("a", added.clone()), ("b", fixed)]),
                entry("feat: untagged", &[("a", MINOR)]),
            ],
        );
        assert_eq!(
            out,
            "### Added\n\n- feat: tagged\n  Packages: a, b (Fixed)\n\n\
             ### Minor changes\n\n- feat: untagged\n  Packages: a\n\n"
        );
    }
}
