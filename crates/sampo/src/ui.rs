use dialoguer::{MultiSelect, theme::ColorfulTheme};
use sampo_core::errors::{Result, SampoError};
use std::io;

pub fn select_packages(available: &[String], prompt: &str) -> Result<Vec<String>> {
    if available.is_empty() {
        return Err(SampoError::InvalidData(
            "No workspace packages detected. Run this command inside a Cargo workspace.".into(),
        ));
    }

    let theme = ColorfulTheme {
        prompt_prefix: dialoguer::console::style("🧭".to_string()),
        ..ColorfulTheme::default()
    };

    loop {
        let selections = MultiSelect::with_theme(&theme)
            .with_prompt(prompt)
            .items(available)
            .report(false)
            .interact()
            .map_err(prompt_io_error)?;

        if selections.is_empty() {
            eprintln!("Select at least one package to continue.");
            continue;
        }

        return Ok(selections
            .into_iter()
            .map(|index| available[index].clone())
            .collect());
    }
}

pub fn prompt_io_error(error: dialoguer::Error) -> io::Error {
    match error {
        dialoguer::Error::IO(err) => err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_packages_requires_non_empty_workspace() {
        let err = select_packages(&[], "prompt").unwrap_err();
        match err {
            SampoError::InvalidData(msg) => {
                assert!(msg.contains("No workspace packages"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
