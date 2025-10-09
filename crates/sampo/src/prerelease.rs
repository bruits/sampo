use crate::cli::{PreArgs, PreCommands, PreEnterArgs, PreExitArgs};
use crate::ui::{prompt_io_error, prompt_nonempty_string, select_packages};
use dialoguer::{Select, console::style, theme::ColorfulTheme};
use sampo_core::{
    Config, VersionChange, discover_workspace, enter_prerelease,
    errors::{Result, SampoError},
    exit_prerelease,
    filters::filter_members,
    restore_preserved_changesets,
    types::PackageSpecifier,
};
use semver::Version;
use std::collections::{BTreeSet, HashMap};

const LABEL_PROMPT: &str = "Pre-release label (alpha, beta, rc, etc.)";

pub fn run(args: &PreArgs) -> Result<()> {
    match &args.command {
        Some(PreCommands::Enter(cmd)) => run_enter(cmd),
        Some(PreCommands::Exit(cmd)) => run_exit(cmd),
        None => run_interactive(),
    }
}

fn run_enter(args: &PreEnterArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let workspace = discover_workspace(&cwd)?;
    let available = visible_packages(&workspace)?;

    let selected_specs = if args.package.is_empty() {
        let labels: Vec<String> = available.iter().map(|(label, _)| label.clone()).collect();
        if labels.is_empty() {
            select_packages(
                &labels,
                "Select packages to enter pre-release mode (space to toggle, enter to confirm)",
            )?;
            Vec::new()
        } else {
            let mut label_map: HashMap<String, PackageSpecifier> = HashMap::new();
            for (label, spec) in available {
                label_map.insert(label.clone(), spec);
            }
            select_packages(
                &labels,
                "Select packages to enter pre-release mode (space to toggle, enter to confirm)",
            )?
            .into_iter()
            .map(|label| {
                label_map.get(&label).cloned().ok_or_else(|| {
                    SampoError::Prerelease(format!(
                        "Selected package '{}' could not be resolved.",
                        label
                    ))
                })
            })
            .collect::<Result<Vec<_>>>()?
        }
    } else {
        resolve_cli_specifiers(&workspace, &args.package)?
    };

    let mut seen = BTreeSet::new();
    let mut normalized_specs = Vec::new();
    for spec in selected_specs {
        if let Some(identifier) = canonical_from_spec(&spec) {
            if !seen.insert(identifier.clone()) {
                continue;
            }
            normalized_specs.push(PackageSpecifier {
                kind: spec.kind,
                name: spec.name,
            });
        } else {
            return Err(SampoError::Prerelease(format!(
                "Package reference '{}' is missing an ecosystem prefix.",
                spec.name
            )));
        }
    }

    if normalized_specs.is_empty() {
        return Err(SampoError::Prerelease("No packages selected.".to_string()));
    }

    let canonical: Vec<String> = normalized_specs
        .iter()
        .map(|spec| spec.to_canonical_string())
        .collect();
    let requested_display: Vec<String> = normalized_specs
        .iter()
        .map(display_label_from_spec)
        .collect();

    let label = resolve_label(args.label.as_deref())?;

    let packages_to_reset = packages_requiring_label_switch(&workspace, &canonical, &label)?;
    if !packages_to_reset.is_empty() {
        let exit_updates = exit_prerelease(&workspace.root, &packages_to_reset)?;
        if !exit_updates.is_empty() {
            let reset_display: Vec<String> = packages_to_reset
                .iter()
                .map(|id| display_name_for_identifier(&workspace, id))
                .collect();
            report_updates(
                "Restored stable versions",
                None,
                &reset_display,
                &exit_updates,
            );
        }

        let restored = restore_preserved_changesets(&workspace.root)?;
        if restored > 0 {
            println!("Restored {restored} preserved changeset(s) from previous pre-release phase.");
        }
    }

    let updates = enter_prerelease(&workspace.root, &canonical, &label)?;
    report_updates(
        "Applied pre-release label",
        Some(label.as_str()),
        &requested_display,
        &updates,
    );
    Ok(())
}

fn run_exit(args: &PreExitArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let workspace = discover_workspace(&cwd)?;
    let prerelease_ids: BTreeSet<String> = workspace
        .members
        .iter()
        .filter(|info| info.version.contains('-'))
        .map(|info| info.canonical_identifier().to_string())
        .collect();

    let available = visible_packages(&workspace)?;
    let available: Vec<(String, PackageSpecifier)> = available
        .into_iter()
        .filter(|(_, spec)| {
            canonical_from_spec(spec)
                .map(|id| prerelease_ids.contains(&id))
                .unwrap_or(false)
        })
        .collect();

    let selected_specs = if args.package.is_empty() {
        if available.is_empty() {
            println!("All workspace packages are already stable.");
            return Ok(());
        }
        let labels: Vec<String> = available.iter().map(|(label, _)| label.clone()).collect();
        let mut label_map: HashMap<String, PackageSpecifier> = HashMap::new();
        for (label, spec) in available {
            label_map.insert(label.clone(), spec);
        }
        select_packages(
            &labels,
            "Select packages to exit pre-release mode (space to toggle, enter to confirm)",
        )?
        .into_iter()
        .map(|label| {
            label_map.get(&label).cloned().ok_or_else(|| {
                SampoError::Prerelease(format!(
                    "Selected package '{}' could not be resolved.",
                    label
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?
    } else {
        resolve_cli_specifiers(&workspace, &args.package)?
    };

    let mut seen = BTreeSet::new();
    let mut normalized_specs = Vec::new();
    for spec in selected_specs {
        if let Some(identifier) = canonical_from_spec(&spec) {
            if !seen.insert(identifier.clone()) {
                continue;
            }
            normalized_specs.push(PackageSpecifier {
                kind: spec.kind,
                name: spec.name,
            });
        }
    }

    if normalized_specs.is_empty() {
        return Err(SampoError::Prerelease("No packages selected.".to_string()));
    }

    let canonical: Vec<String> = normalized_specs
        .iter()
        .map(|spec| spec.to_canonical_string())
        .collect();
    let requested_display: Vec<String> = normalized_specs
        .iter()
        .map(display_label_from_spec)
        .collect();

    let updates = exit_prerelease(&workspace.root, &canonical)?;
    report_updates(
        "Restored stable versions",
        None,
        &requested_display,
        &updates,
    );
    Ok(())
}

fn run_interactive() -> Result<()> {
    match prompt_mode()? {
        InteractiveMode::Enter => {
            let label = prompt_nonempty_string(LABEL_PROMPT)?;
            let args = PreEnterArgs {
                label: Some(label),
                package: Vec::new(),
            };
            run_enter(&args)
        }
        InteractiveMode::Exit => {
            let args = PreExitArgs {
                package: Vec::new(),
            };
            run_exit(&args)
        }
    }
}

fn visible_packages(workspace: &sampo_core::Workspace) -> Result<Vec<(String, PackageSpecifier)>> {
    let config = Config::load(&workspace.root)?;
    let members = filter_members(workspace, &config)
        .unwrap_or_else(|_| workspace.members.iter().collect::<Vec<_>>());

    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for info in members {
        let identifier = info.canonical_identifier().to_string();
        if !seen.insert(identifier) {
            continue;
        }
        let spec = PackageSpecifier {
            kind: Some(info.kind),
            name: info.name.clone(),
        };
        let label = format!("{} ({})", info.name, info.kind.as_str());
        out.push((label, spec));
    }
    Ok(out)
}

fn resolve_label(existing: Option<&str>) -> Result<String> {
    if let Some(value) = existing {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    prompt_nonempty_string(LABEL_PROMPT)
}

fn packages_requiring_label_switch(
    workspace: &sampo_core::Workspace,
    selections: &[String],
    desired_label: &str,
) -> Result<Vec<String>> {
    let desired_base = normalize_label(desired_label);
    let mut to_reset = Vec::new();

    for identifier in selections {
        let info = workspace.find_by_identifier(identifier).ok_or_else(|| {
            SampoError::Prerelease(format!("Package '{}' not found in workspace", identifier))
        })?;

        let version = Version::parse(&info.version).map_err(|err| {
            SampoError::Prerelease(format!(
                "Invalid semantic version for package '{}': {}",
                info.name, err
            ))
        })?;

        if version.pre.is_empty() {
            continue;
        }

        let current_base = normalize_label(version.pre.as_str());
        if !current_base.is_empty() && current_base != desired_base {
            to_reset.push(identifier.clone());
        }
    }

    Ok(to_reset)
}

fn resolve_cli_specifiers(
    workspace: &sampo_core::Workspace,
    inputs: &[String],
) -> Result<Vec<PackageSpecifier>> {
    let mut resolved = Vec::new();
    for raw in inputs {
        let spec = PackageSpecifier::parse(raw).map_err(|reason| {
            SampoError::Prerelease(format!("Invalid package reference '{}': {}", raw, reason))
        })?;

        let info = if let Some(kind) = spec.kind {
            let identifier = format!("{}:{}", kind.as_str(), spec.name);
            workspace.find_by_identifier(&identifier).ok_or_else(|| {
                SampoError::Prerelease(format!("Package '{}' not found in workspace", identifier))
            })?
        } else {
            let matches = workspace.match_specifier(&spec);
            match matches.len() {
                0 => {
                    return Err(SampoError::Prerelease(format!(
                        "Package '{}' not found in workspace",
                        spec.name
                    )));
                }
                1 => matches[0],
                _ => {
                    let options = matches
                        .iter()
                        .map(|info| format!("{}:{}", info.kind.as_str(), info.name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(SampoError::Prerelease(format!(
                        "Package '{}' is ambiguous. Disambiguate using one of: {}.",
                        spec.name, options
                    )));
                }
            }
        };

        resolved.push(PackageSpecifier {
            kind: Some(info.kind),
            name: info.name.clone(),
        });
    }
    Ok(resolved)
}

fn canonical_from_spec(spec: &PackageSpecifier) -> Option<String> {
    spec.kind
        .map(|kind| format!("{}:{}", kind.as_str(), spec.name))
}

fn display_label_from_spec(spec: &PackageSpecifier) -> String {
    match spec.kind {
        Some(kind) => format!("{} ({})", spec.name, kind.as_str()),
        None => spec.name.clone(),
    }
}

fn display_name_for_identifier(workspace: &sampo_core::Workspace, identifier: &str) -> String {
    workspace
        .find_by_identifier(identifier)
        .map(|info| format!("{} ({})", info.name, info.kind.as_str()))
        .unwrap_or_else(|| identifier.to_string())
}

fn normalize_label(label: &str) -> String {
    label
        .split('.')
        .find(|segment| segment.chars().any(|ch| !ch.is_ascii_digit()))
        .unwrap_or(label)
        .to_ascii_lowercase()
}

enum InteractiveMode {
    Enter,
    Exit,
}

fn prompt_mode() -> Result<InteractiveMode> {
    let theme = ColorfulTheme {
        prompt_prefix: style("🧭".to_string()),
        ..ColorfulTheme::default()
    };
    let options = ["Enter pre-release mode", "Exit pre-release mode"];
    let selection = Select::with_theme(&theme)
        .with_prompt("Choose pre-release action")
        .items(&options)
        .default(0)
        .report(false)
        .interact()
        .map_err(prompt_io_error)?;
    Ok(match selection {
        0 => InteractiveMode::Enter,
        1 => InteractiveMode::Exit,
        _ => InteractiveMode::Enter,
    })
}
fn report_updates(
    action: &str,
    label: Option<&str>,
    requested: &[String],
    changes: &[VersionChange],
) {
    if changes.is_empty() {
        if let Some(label) = label {
            println!(
                "No version changes applied; selected packages already use pre-release label '{}'.",
                label
            );
        } else {
            println!("No version changes applied; selected packages are already stable.");
        }
        return;
    }

    if let Some(label) = label {
        println!("{} '{}' for {} package(s):", action, label, changes.len());
    } else {
        println!("{} for {} package(s):", action, changes.len());
    }

    for change in changes {
        println!(
            "  {}: {} -> {}",
            change.name, change.old_version, change.new_version
        );
    }

    let changed: BTreeSet<&str> = changes.iter().map(|change| change.name.as_str()).collect();
    let skipped: Vec<&str> = requested
        .iter()
        .map(|name| name.as_str())
        .filter(|name| !changed.contains(*name))
        .collect();

    if !skipped.is_empty() {
        if label.is_some() {
            println!("No change needed for: {}", skipped.join(", "));
        } else {
            println!("Already stable: {}", skipped.join(", "));
        }
    }
}
