use dialoguer::{
    Input, MultiSelect,
    console::{Style, style},
    theme::ColorfulTheme,
};
use sampo_core::{
    errors::{Result, SampoError},
    types::PackageKind,
};
use std::io;

pub const SUCCESS_PREFIX: &str = "✔";
pub const WARNING_PREFIX: &str = "⚠";
pub const HINT_PREFIX: &str = "💡";
const EMPTY_SELECTION_PLACEHOLDER: &str = "(none)";

pub fn log_success_value(label: &str, value: &str) {
    let theme = success_output_theme();
    let line = format!(
        "{} {}{} {}",
        theme.success_prefix.clone(),
        theme.prompt_style.apply_to(label),
        theme.success_suffix.clone(),
        theme.values_style.apply_to(value),
    );
    println!("{line}");
}

pub fn log_success_list(label: &str, items: &[String]) {
    let theme = success_output_theme();
    let display = if items.is_empty() {
        EMPTY_SELECTION_PLACEHOLDER.to_string()
    } else {
        items.join(", ")
    };
    let line = format!(
        "{} {}{} {}",
        theme.success_prefix.clone(),
        theme.prompt_style.apply_to(label),
        theme.success_suffix.clone(),
        theme.values_style.apply_to(display.as_str()),
    );
    println!("{line}");
}

pub fn log_warning(message: &str) {
    let mut theme = prompt_theme();
    theme.error_prefix = style(WARNING_PREFIX.to_string()).for_stderr().yellow();
    theme.error_style = Style::new().for_stderr().yellow();

    let line = format!(
        "{} {}",
        theme.error_prefix.clone(),
        theme.error_style.apply_to(message)
    );
    eprintln!("{line}");
}

/// Prints a hint message to stderr with a distinct visual style.
///
/// Used for non-critical suggestions like update notifications.
pub fn log_hint(message: &str) {
    let prefix = style(HINT_PREFIX.to_string()).for_stderr().yellow();
    let message_style = Style::new().for_stderr().yellow();

    let line = format!("{} {}", prefix, message_style.apply_to(message));
    eprintln!("{line}");
}

pub fn prompt_theme() -> ColorfulTheme {
    ColorfulTheme {
        prompt_prefix: style("🧭".to_string()).cyan(),
        prompt_style: Style::new().for_stderr(),
        success_prefix: style(SUCCESS_PREFIX.to_string()).for_stderr(),
        success_suffix: style(":".to_string()).for_stderr(),
        values_style: Style::new().for_stderr(),
        ..ColorfulTheme::default()
    }
}

fn success_output_theme() -> ColorfulTheme {
    let mut theme = prompt_theme();
    theme.success_prefix = theme.success_prefix.clone().for_stdout();
    theme.success_suffix = theme.success_suffix.clone().for_stdout();
    theme.prompt_style = theme.prompt_style.clone().for_stdout();
    theme.values_style = theme.values_style.clone().for_stdout();
    theme
}

pub fn select_packages(
    available: &[String],
    prompt: &str,
    summary_label: &str,
) -> Result<Vec<String>> {
    if available.is_empty() {
        return Err(SampoError::InvalidData(
            "No packages detected in the current directory.".into(),
        ));
    }

    let theme = prompt_theme();

    loop {
        let selections = MultiSelect::with_theme(&theme)
            .with_prompt(prompt)
            .items(available)
            .report(false)
            .interact()
            .map_err(prompt_io_error)?;

        if selections.is_empty() {
            log_warning("Select at least one package to continue.");
            continue;
        }

        let selected = selections
            .into_iter()
            .map(|index| available[index].clone())
            .collect::<Vec<_>>();
        log_success_list(summary_label, &selected);
        return Ok(selected);
    }
}

pub fn prompt_nonempty_string(prompt: &str) -> Result<String> {
    let theme = prompt_theme();
    loop {
        let value: String = Input::with_theme(&theme)
            .with_prompt(prompt)
            .report(false)
            .allow_empty(false)
            .interact_text()
            .map_err(prompt_io_error)?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            log_warning("Enter a non-empty value.");
            continue;
        }
        return Ok(trimmed.to_string());
    }
}

/// Normalize an optional string input by trimming whitespace.
/// Returns None if the input is None or if the trimmed value is empty.
pub fn normalize_nonempty_string(input: Option<&str>) -> Option<String> {
    input.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub fn prompt_io_error(error: dialoguer::Error) -> io::Error {
    match error {
        dialoguer::Error::IO(err) => err,
    }
}

pub fn format_package_label(name: &str, kind: PackageKind, include_kind: bool) -> String {
    kind.format_name(name, include_kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_packages_requires_non_empty_workspace() {
        let err = select_packages(&[], "prompt", "Packages").unwrap_err();
        match err {
            SampoError::InvalidData(msg) => {
                assert!(msg.contains("No packages detected"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn normalize_nonempty_string_trims_and_accepts_value() {
        assert_eq!(
            normalize_nonempty_string(Some("  value  ")).as_deref(),
            Some("value")
        );
    }

    #[test]
    fn normalize_nonempty_string_rejects_empty_value() {
        assert!(normalize_nonempty_string(Some("   ")).is_none());
        assert!(normalize_nonempty_string(None).is_none());
    }
}
