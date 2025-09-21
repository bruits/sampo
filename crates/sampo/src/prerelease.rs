use crate::cli::{PreArgs, PreCommands, PreEnterArgs, PreExitArgs};
use crate::ui::{prompt_io_error, prompt_nonempty_string, select_packages};
use dialoguer::{Select, console::style, theme::ColorfulTheme};
use sampo_core::{
    Config, VersionChange, discover_workspace, enter_prerelease,
    errors::{Result, SampoError},
    exit_prerelease,
    filters::list_visible_packages,
};
use std::collections::BTreeSet;

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

    let label = resolve_label(args.label.as_deref())?;
    let updates = enter_prerelease(&workspace.root, &selections, &label)?;
    report_updates(
        "Applied pre-release label",
        Some(label.as_str()),
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

fn resolve_label(existing: Option<&str>) -> Result<String> {
    if let Some(value) = existing {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    prompt_nonempty_string(LABEL_PROMPT)
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
