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
        // Empty message still yields a bullet
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
