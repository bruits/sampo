use crate::cli::AddArgs;
use crate::names;
use dialoguer::{Input, MultiSelect, theme::ColorfulTheme};
use sampo_core::{
    Bump, Config, discover_workspace,
    errors::{Result, SampoError},
    filters::list_visible_packages,
    render_changeset_markdown,
};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn run(args: &AddArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;

    // Discover workspace (optional but helps list packages)
    let (root, packages) = match discover_workspace(&cwd) {
        Ok(ws) => {
            // Load config to respect ignore rules when listing packages
            let cfg = Config::load(&ws.root).unwrap_or_default();
            let names = match list_visible_packages(&ws, &cfg) {
                Ok(v) => v,
                Err(_) => ws.members.into_iter().map(|c| c.name).collect::<Vec<_>>(),
            };
            (ws.root, names)
        }
        Err(_) => (cwd.clone(), Vec::new()),
    };

    // Create changesets directory if it doesn't exist
    let changesets_dir = root.join(".sampo").join("changesets");
    ensure_dir(&changesets_dir)?;

    // Collect inputs, prefilling from CLI args if provided
    let selected_packages = if args.package.is_empty() {
        prompt_packages(&packages)?
    } else {
        args.package.clone()
    };

    let package_bumps = prompt_package_bumps(&selected_packages)?;

    let message = match &args.message {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => prompt_message()?,
    };

    // Compose file contents
    let contents = render_changeset_markdown(&package_bumps, &message);
    let path = unique_changeset_path(&changesets_dir);
    fs::write(&path, contents)?;

    println!("Created: {}", path.display());
    Ok(())
}

fn ensure_dir(dir: &PathBuf) -> Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

fn prompt_packages(available: &[String]) -> Result<Vec<String>> {
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
            .with_prompt(
                "Select packages impacted by this changeset (space to toggle, enter to confirm)",
            )
            .items(available)
            .report(false)
            .interact()
            .map_err(prompt_io_error)?;

        if selections.is_empty() {
            eprintln!("Select at least one package to continue.");
            continue;
        }

        let chosen = selections
            .into_iter()
            .map(|index| available[index].clone())
            .collect();
        return Ok(chosen);
    }
}

fn prompt_package_bumps(packages: &[String]) -> Result<Vec<(String, Bump)>> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }

    let mut remaining: Vec<String> = packages.to_vec();
    let mut assignments: HashMap<String, Bump> = HashMap::new();
    let theme = ColorfulTheme::default();

    let patch = prompt_bump_level(&theme, "Which packages receive a PATCH bump?", &remaining)?;
    for name in patch {
        assignments.insert(name.clone(), Bump::Patch);
    }
    remaining.retain(|name| !assignments.contains_key(name));

    if !remaining.is_empty() {
        let minor = prompt_bump_level(&theme, "Which packages receive a MINOR bump?", &remaining)?;
        for name in minor {
            assignments.insert(name.clone(), Bump::Minor);
        }
        remaining.retain(|name| !assignments.contains_key(name));
    }

    if !remaining.is_empty() {
        let major = prompt_bump_level(&theme, "Which packages receive a MAJOR bump?", &remaining)?;
        for name in major {
            assignments.insert(name.clone(), Bump::Major);
        }
        remaining.retain(|name| !assignments.contains_key(name));
    }

    if !remaining.is_empty() {
        eprintln!(
            "No bump level selected for: {} — defaulting to PATCH.",
            remaining.join(", "),
        );
        for name in &remaining {
            assignments.insert(name.clone(), Bump::Patch);
        }
    }

    let mut ordered = Vec::with_capacity(packages.len());
    for name in packages {
        let bump = assignments.get(name).copied().unwrap_or(Bump::Patch);
        ordered.push((name.clone(), bump));
    }
    Ok(ordered)
}

fn prompt_bump_level(
    theme: &ColorfulTheme,
    prompt: &str,
    choices: &[String],
) -> Result<Vec<String>> {
    if choices.is_empty() {
        return Ok(Vec::new());
    }

    let selections = MultiSelect::with_theme(theme)
        .with_prompt(prompt)
        .items(choices)
        .report(false)
        .interact()
        .map_err(prompt_io_error)?;

    Ok(selections
        .into_iter()
        .map(|index| choices[index].clone())
        .collect())
}

fn prompt_message() -> Result<String> {
    let theme = ColorfulTheme::default();
    loop {
        let message: String = Input::with_theme(&theme)
            .with_prompt("Changeset message")
            .allow_empty(false)
            .interact_text()
            .map_err(prompt_io_error)?;
        let trimmed = message.trim();
        if trimmed.is_empty() {
            eprintln!("Enter a non-empty message.");
            continue;
        }
        return Ok(trimmed.to_string());
    }
}

fn prompt_io_error(error: dialoguer::Error) -> io::Error {
    match error {
        dialoguer::Error::IO(err) => err,
    }
}

fn unique_changeset_path(dir: &Path) -> PathBuf {
    let mut rng = rand::thread_rng();
    let base = names::generate_file_name(&mut rng);
    let mut candidate = dir.join(format!("{base}.md"));
    // If somehow exists, add counter suffix
    let mut i = 1u32;
    while candidate.exists() {
        let name_with_counter = format!("{base}-{i}");
        candidate = dir.join(format!("{name_with_counter}.md"));
        i += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn render_has_frontmatter() {
        let md = render_changeset_markdown(
            &[("a".into(), Bump::Minor), ("b".into(), Bump::Minor)],
            "feat: add stuff",
        );
        assert!(md.starts_with("---\n"));
        assert!(md.contains("a: minor\n"));
        assert!(md.contains("b: minor\n"));
        assert!(md.ends_with("feat: add stuff\n"));
    }

    #[test]
    fn render_single_package() {
        let md = render_changeset_markdown(&[("single".into(), Bump::Patch)], "fix: bug");
        assert!(md.contains("single: patch\n"));
        assert!(md.ends_with("fix: bug\n"));
    }

    #[test]
    fn render_major_release() {
        let md = render_changeset_markdown(&[("pkg".into(), Bump::Major)], "breaking: api change");
        assert!(md.contains("pkg: major\n"));
        assert!(md.ends_with("breaking: api change\n"));
    }

    #[test]
    fn prompt_packages_without_workspace_errors() {
        let err = prompt_packages(&[]).unwrap_err();
        match err {
            SampoError::InvalidData(msg) => {
                assert!(msg.contains("No workspace packages detected"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn unique_changeset_path_creates_md_files() {
        let temp_dir = std::env::temp_dir().join("sampo-test");
        fs::create_dir_all(&temp_dir).unwrap();

        let path = unique_changeset_path(&temp_dir);
        assert!(path.starts_with(&temp_dir));
        assert!(path.extension().unwrap() == "md");

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn unique_changeset_path_avoids_conflicts() {
        let temp_dir = std::env::temp_dir().join("sampo-test-conflict");
        fs::create_dir_all(&temp_dir).unwrap();

        let path1 = unique_changeset_path(&temp_dir);
        let path2 = unique_changeset_path(&temp_dir);

        // Should generate different paths
        assert_ne!(path1, path2);
        assert!(path1.extension().unwrap() == "md");
        assert!(path2.extension().unwrap() == "md");

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
