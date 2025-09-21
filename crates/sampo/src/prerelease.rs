use crate::cli::{PreCommands, PreEnterArgs, PreExitArgs};
use crate::ui::select_packages;
use sampo_core::{
    Config, VersionChange, discover_workspace, enter_prerelease,
    errors::{Result, SampoError},
    exit_prerelease,
    filters::list_visible_packages,
};
use std::collections::BTreeSet;

pub fn run(command: &PreCommands) -> Result<()> {
    match command {
        PreCommands::Enter(args) => run_enter(args),
        PreCommands::Exit(args) => run_exit(args),
    }
}

fn run_enter(args: &PreEnterArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let workspace = discover_workspace(&cwd)?;
    let available = visible_packages(&workspace)?;

    let selections = normalize_packages(if args.package.is_empty() {
        select_packages(
            &available,
            "Select packages to enter pre-release mode (space to toggle, enter to confirm)",
        )?
    } else {
        args.package.clone()
    });

    if selections.is_empty() {
        return Err(SampoError::Prerelease("No packages selected.".to_string()));
    }

    let label = args.label.trim();
    let updates = enter_prerelease(&workspace.root, &selections, label)?;
    report_updates(
        "Applied pre-release label",
        Some(label),
        &selections,
        &updates,
    );
    Ok(())
}

fn run_exit(args: &PreExitArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let workspace = discover_workspace(&cwd)?;
    let available = visible_packages(&workspace)?;

    let pre_release_set: BTreeSet<&str> = workspace
        .members
        .iter()
        .filter(|info| info.version.contains('-'))
        .map(|info| info.name.as_str())
        .collect();

    let filtered: Vec<String> = available
        .into_iter()
        .filter(|name| pre_release_set.contains(name.as_str()))
        .collect();

    let selections = normalize_packages(if args.package.is_empty() {
        if filtered.is_empty() {
            println!("All workspace packages are already stable.");
            return Ok(());
        }
        select_packages(
            &filtered,
            "Select packages to exit pre-release mode (space to toggle, enter to confirm)",
        )?
    } else {
        args.package.clone()
    });

    if selections.is_empty() {
        return Err(SampoError::Prerelease("No packages selected.".to_string()));
    }

    let updates = exit_prerelease(&workspace.root, &selections)?;
    report_updates("Restored stable versions", None, &selections, &updates);
    Ok(())
}

fn visible_packages(workspace: &sampo_core::Workspace) -> Result<Vec<String>> {
    let config = Config::load(&workspace.root)?;
    Ok(
        list_visible_packages(workspace, &config).unwrap_or_else(|_| {
            workspace
                .members
                .iter()
                .map(|info| info.name.clone())
                .collect()
        }),
    )
}

fn normalize_packages(names: Vec<String>) -> Vec<String> {
    let mut set = BTreeSet::new();
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        set.insert(trimmed.to_string());
    }
    set.into_iter().collect()
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
