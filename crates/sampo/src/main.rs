mod add;
mod cli;
mod init;
mod names;
mod prerelease;
mod publish;
mod release;
mod ui;
mod version_check;

use clap::Parser;
use cli::{Cli, Commands};
use std::process::ExitCode;
use version_check::VersionCheckResult;

fn main() -> ExitCode {
    let cli = Cli::parse();

    check_and_notify_update();

    match cli.command {
        Commands::Init => {
            let cwd = match std::env::current_dir() {
                Ok(dir) => dir,
                Err(e) => {
                    eprintln!("Failed to get current directory: {e}");
                    return ExitCode::from(1);
                }
            };
            match init::init_from_cwd(&cwd) {
                Ok(report) => {
                    println!("Initialized Sampo at {}", report.root.display());
                    let dir = report.root.join(".sampo");
                    if report.created_dir {
                        println!("  created: {}", dir.display());
                    }
                    if report.created_readme {
                        println!("  created: {}", dir.join("README.md").display());
                    }
                    if report.created_config {
                        println!("  created: {}", dir.join("config.toml").display());
                    }
                }
                Err(e) => {
                    eprintln!("init error: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        Commands::Add(args) => {
            if let Err(e) = add::run(&args) {
                eprintln!("Failed to add changeset: {e}");
                return ExitCode::from(1);
            }
        }
        Commands::Publish(args) => {
            if let Err(e) = publish::run(&args) {
                eprintln!("Failed to publish packages: {e}");
                return ExitCode::from(1);
            }
        }
        Commands::Release(args) => {
            if let Err(e) = release::run(&args) {
                eprintln!("Failed to release packages: {e}");
                return ExitCode::from(1);
            }
        }
        Commands::Pre(args) => {
            if let Err(e) = prerelease::run(&args) {
                eprintln!("Failed to manage pre-release versions: {e}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

/// Checks for CLI updates and prints a hint if a newer version is available. Non-blocking, best-effort.
fn check_and_notify_update() {
    if let VersionCheckResult::UpdateAvailable { current, latest } =
        version_check::check_for_updates()
    {
        ui::log_hint(&format!(
            "A new version of Sampo is available: {current} → {latest}. Run `cargo install sampo` to update."
        ));
    }
}
