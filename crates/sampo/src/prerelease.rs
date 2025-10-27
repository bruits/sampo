use crate::cli::{PreArgs, PreCommands, PreEnterArgs, PreExitArgs};
use crate::ui::{
    format_package_label, log_success_list, log_success_value, normalize_nonempty_string,
    prompt_io_error, prompt_nonempty_string, prompt_theme, select_packages,
};
use dialoguer::Select;
use sampo_core::{
    Config, VersionChange, discover_workspace, enter_prerelease,
    errors::{Result, SampoError},
    exit_prerelease,
    filters::filter_members,
    restore_preserved_changesets,
    types::{PackageSpecifier, SpecResolution, format_ambiguity_options},
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
    let include_kind = workspace.has_multiple_package_kinds();
    let available = visible_packages(&workspace, include_kind)?;

    let from_cli_packages = !args.package.is_empty();
    let selected_specs = if args.package.is_empty() {
        let labels: Vec<String> = available.iter().map(|(label, _)| label.clone()).collect();
        if labels.is_empty() {
            select_packages(
                &labels,
                "Select packages to enter pre-release mode (space to toggle, enter to confirm)",
                "Packages",
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
                "Packages",
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

    let requested_display: Vec<String> = normalized_specs
        .iter()
        .map(|spec| display_label_from_spec(spec, include_kind))
        .collect();
    if from_cli_packages {
        log_success_list("Packages", &requested_display);
    }
    let canonical: Vec<String> = normalized_specs
        .iter()
        .map(|spec| spec.to_canonical_string())
        .collect();

    let label = resolve_label(args.label.as_deref())?;

    let packages_to_reset = packages_requiring_label_switch(&workspace, &canonical, &label)?;
    if !packages_to_reset.is_empty() {
        let exit_updates = exit_prerelease(&workspace.root, &packages_to_reset)?;
        if !exit_updates.is_empty() {
            let reset_display: Vec<String> = packages_to_reset
                .iter()
                .map(|id| display_name_for_identifier(&workspace, id, include_kind))
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
    let include_kind = workspace.has_multiple_package_kinds();
    let prerelease_ids: BTreeSet<String> = workspace
        .members
        .iter()
        .filter(|info| info.version.contains('-'))
        .map(|info| info.canonical_identifier().to_string())
        .collect();

    let available = visible_packages(&workspace, include_kind)?;
    let available: Vec<(String, PackageSpecifier)> = available
        .into_iter()
        .filter(|(_, spec)| {
            canonical_from_spec(spec)
                .map(|id| prerelease_ids.contains(&id))
                .unwrap_or(false)
        })
        .collect();

    let from_cli_packages = !args.package.is_empty();
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
            "Packages",
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

    let requested_display: Vec<String> = normalized_specs
        .iter()
        .map(|spec| display_label_from_spec(spec, include_kind))
        .collect();
    if from_cli_packages {
        log_success_list("Packages", &requested_display);
    }
    let canonical: Vec<String> = normalized_specs
        .iter()
        .map(|spec| spec.to_canonical_string())
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

fn visible_packages(
    workspace: &sampo_core::Workspace,
    include_kind: bool,
) -> Result<Vec<(String, PackageSpecifier)>> {
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
        let label = format_package_label(&info.name, info.kind, include_kind);
        out.push((label, spec));
    }
    Ok(out)
}

fn resolve_label(existing: Option<&str>) -> Result<String> {
    if let Some(value) = normalize_nonempty_string(existing) {
        log_success_value("Pre-release label", &value);
        return Ok(value);
    }
    let value = prompt_nonempty_string(LABEL_PROMPT)?;
    log_success_value("Pre-release label", &value);
    Ok(value)
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

        let info = match workspace.resolve_specifier(&spec) {
            SpecResolution::Match(info) => info,
            SpecResolution::NotFound { query } => {
                return Err(SampoError::Prerelease(format!(
                    "Package '{}' not found in workspace",
                    query.display()
                )));
            }
            SpecResolution::Ambiguous { query, matches } => {
                let options = format_ambiguity_options(&matches);
                return Err(SampoError::Prerelease(format!(
                    "Package '{}' is ambiguous. Disambiguate using one of: {}.",
                    query.base_name(),
                    options
                )));
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
    spec.kind.map(|_| spec.to_canonical_string())
}

fn display_label_from_spec(spec: &PackageSpecifier, include_kind: bool) -> String {
    spec.kind
        .map(|kind| format_package_label(&spec.name, kind, include_kind))
        .unwrap_or_else(|| spec.name.clone())
}

fn display_name_for_identifier(
    workspace: &sampo_core::Workspace,
    identifier: &str,
    include_kind: bool,
) -> String {
    workspace
        .find_by_identifier(identifier)
        .map(|info| format_package_label(&info.name, info.kind, include_kind))
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
    let theme = prompt_theme();
    let options = ["Enter pre-release mode", "Exit pre-release mode"];
    let selection = Select::with_theme(&theme)
        .with_prompt("Choose pre-release action")
        .items(&options)
        .default(0)
        .report(false)
        .interact()
        .map_err(prompt_io_error)?;
    let mode = match selection {
        0 => InteractiveMode::Enter,
        1 => InteractiveMode::Exit,
        _ => InteractiveMode::Enter,
    };
    let summary = match mode {
        InteractiveMode::Enter => "Enter pre-release mode",
        InteractiveMode::Exit => "Exit pre-release mode",
    };
    log_success_value("Action", summary);
    Ok(mode)
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

#[cfg(test)]
mod tests {
    use crate::ui::normalize_nonempty_string;

    #[test]
    fn normalized_label_arg_trims_and_accepts_value() {
        assert_eq!(
            normalize_nonempty_string(Some("  beta  ")).as_deref(),
            Some("beta")
        );
    }

    #[test]
    fn normalized_label_arg_rejects_empty_value() {
        assert!(normalize_nonempty_string(Some("   ")).is_none());
        assert!(normalize_nonempty_string(None).is_none());
    }
}
