use crate::cli::AddArgs;
use crate::names;
use crate::ui::{format_package_label, prompt_io_error, select_packages};
use dialoguer::{Input, MultiSelect, theme::ColorfulTheme};
use sampo_core::{
    Bump, Config, Workspace, discover_workspace,
    errors::{Result, SampoError},
    filters::filter_members,
    render_changeset_markdown,
    types::PackageSpecifier,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn run(args: &AddArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;

    let workspace = discover_workspace(&cwd).ok();
    let include_kind = workspace
        .as_ref()
        .map(|ws| ws.has_multiple_package_kinds())
        .unwrap_or(false);
    let (root, available_packages) = if let Some(ref ws) = workspace {
        let cfg = Config::load(&ws.root).unwrap_or_default();
        let visible =
            filter_members(ws, &cfg).unwrap_or_else(|_| ws.members.iter().collect::<Vec<_>>());
        let mut out = Vec::new();
        for info in visible {
            let label = format_package_label(&info.name, info.kind, include_kind);
            let spec = PackageSpecifier {
                kind: Some(info.kind),
                name: info.name.clone(),
            };
            out.push((label, spec));
        }
        (ws.root.clone(), out)
    } else {
        (cwd.clone(), Vec::new())
    };

    // Create changesets directory if it doesn't exist
    let changesets_dir = root.join(".sampo").join("changesets");
    ensure_dir(&changesets_dir)?;

    // Collect inputs, prefilling from CLI args if provided
    let selected_specs = if args.package.is_empty() {
        let labels: Vec<String> = available_packages
            .iter()
            .map(|(label, _)| label.clone())
            .collect();
        if labels.is_empty() {
            select_packages(
                &labels,
                "Select packages impacted by this changeset (space to toggle, enter to confirm)",
            )?; // This will yield a consistent error message for empty workspaces.
            Vec::new()
        } else {
            let selections = select_packages(
                &labels,
                "Select packages impacted by this changeset (space to toggle, enter to confirm)",
            )?;
            let map: HashMap<&str, &PackageSpecifier> = available_packages
                .iter()
                .map(|(label, spec)| (label.as_str(), spec))
                .collect();
            selections
                .into_iter()
                .map(|label| {
                    map.get(label.as_str())
                        .cloned()
                        .ok_or_else(|| {
                            SampoError::InvalidData(format!(
                                "Selected package '{}' could not be resolved.",
                                label
                            ))
                        })
                        .map(|spec| (*spec).clone())
                })
                .collect::<Result<Vec<_>>>()?
        }
    } else {
        resolve_cli_packages(workspace.as_ref(), &args.package)?
    };

    if selected_specs.is_empty() {
        return Err(SampoError::InvalidData(
            "No packages selected for the changeset.".to_string(),
        ));
    }

    let labels_for_bumps: Vec<String> = selected_specs
        .iter()
        .map(|spec| package_display_label(spec, include_kind))
        .collect();
    let package_bumps_display = prompt_package_bumps(&labels_for_bumps)?;
    let mut package_bumps: Vec<(PackageSpecifier, Bump)> =
        Vec::with_capacity(package_bumps_display.len());
    for (idx, (_label, bump)) in package_bumps_display.into_iter().enumerate() {
        let spec = selected_specs.get(idx).cloned().ok_or_else(|| {
            SampoError::InvalidData(
                "Bump selections did not match the selected packages.".to_string(),
            )
        })?;
        package_bumps.push((spec, bump));
    }

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

fn package_display_label(spec: &PackageSpecifier, include_kind: bool) -> String {
    spec.kind
        .map(|kind| format_package_label(&spec.name, kind, include_kind))
        .unwrap_or_else(|| spec.name.clone())
}

fn resolve_cli_packages(
    workspace: Option<&Workspace>,
    inputs: &[String],
) -> Result<Vec<PackageSpecifier>> {
    let mut resolved = Vec::new();
    for raw in inputs {
        let spec = PackageSpecifier::parse(raw).map_err(|reason| {
            SampoError::InvalidData(format!("Invalid package reference '{}': {}", raw, reason))
        })?;

        if let Some(ws) = workspace {
            if let Some(kind) = spec.kind {
                let identifier = format!("{}:{}", kind.as_str(), spec.name);
                if let Some(info) = ws.find_by_identifier(&identifier) {
                    resolved.push(PackageSpecifier {
                        kind: Some(info.kind),
                        name: info.name.clone(),
                    });
                } else {
                    return Err(SampoError::InvalidData(format!(
                        "Package '{}' not found in the workspace.",
                        spec.to_canonical_string()
                    )));
                }
            } else {
                let matches = ws.match_specifier(&spec);
                match matches.len() {
                    0 => {
                        return Err(SampoError::InvalidData(format!(
                            "Package '{}' not found in the workspace.",
                            spec.name
                        )));
                    }
                    1 => {
                        let info = matches[0];
                        resolved.push(PackageSpecifier {
                            kind: Some(info.kind),
                            name: info.name.clone(),
                        });
                    }
                    _ => {
                        let options = matches
                            .iter()
                            .map(|info| format!("{}:{}", info.kind.as_str(), info.name))
                            .collect::<Vec<_>>()
                            .join(", ");
                        return Err(SampoError::InvalidData(format!(
                            "Package '{}' is ambiguous. Disambiguate using one of: {}.",
                            spec.name, options
                        )));
                    }
                }
            }
        } else {
            resolved.push(spec);
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn render_has_frontmatter() {
        let md = render_changeset_markdown(
            &[
                (PackageSpecifier::parse("a").unwrap(), Bump::Minor),
                (PackageSpecifier::parse("b").unwrap(), Bump::Minor),
            ],
            "feat: add stuff",
        );
        assert!(md.starts_with("---\n"));
        assert!(md.contains("a: minor\n"));
        assert!(md.contains("b: minor\n"));
        assert!(md.ends_with("feat: add stuff\n"));
    }

    #[test]
    fn render_single_package() {
        let md = render_changeset_markdown(
            &[(PackageSpecifier::parse("single").unwrap(), Bump::Patch)],
            "fix: bug",
        );
        assert!(md.contains("single: patch\n"));
        assert!(md.ends_with("fix: bug\n"));
    }

    #[test]
    fn render_major_release() {
        let md = render_changeset_markdown(
            &[(PackageSpecifier::parse("pkg").unwrap(), Bump::Major)],
            "breaking: api change",
        );
        assert!(md.contains("pkg: major\n"));
        assert!(md.ends_with("breaking: api change\n"));
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
