//! Utilities to render Markdown consistently across crates.

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
}
