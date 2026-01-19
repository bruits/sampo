mod add;
mod cli;
mod init;
mod names;
mod prerelease;
mod publish;
mod release;
mod ui;
mod update;
mod version_check;

use clap::Parser;
use cli::{Cli, Commands};
use std::process::ExitCode;
use version_check::VersionCheckResult;

/// Exit codes following Linux conventions.
mod exit {
    use std::process::ExitCode;

    /// Success, changes were made (or would be made in dry-run).
    pub const SUCCESS: ExitCode = ExitCode::SUCCESS;

    /// Error occurred.
    pub const ERROR: ExitCode = ExitCode::FAILURE;

    /// Success, but no changes were needed (no-op).
    /// no_changes() is a function rather than a const because ExitCode::from(2) isn't const-evaluable in stable Rust.
    pub fn no_changes() -> ExitCode {
        ExitCode::from(2)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    check_and_notify_update();

    match cli.command {
        Commands::Init => {
            let cwd = match std::env::current_dir() {
                Ok(dir) => dir,
                Err(e) => {
                    eprintln!("Failed to get current directory: {e}");
                    return exit::ERROR;
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
                    return exit::ERROR;
                }
            }
        }
        Commands::Add(args) => {
            if let Err(e) = add::run(&args) {
                eprintln!("Failed to add changeset: {e}");
                return exit::ERROR;
            }
        }
        Commands::Publish(args) => match publish::run(&args) {
            Ok(true) => {}
            Ok(false) => return exit::no_changes(),
            Err(e) => {
                eprintln!("Failed to publish packages: {e}");
                return exit::ERROR;
            }
        },
        Commands::Release(args) => match release::run(&args) {
            Ok(true) => {}
            Ok(false) => return exit::no_changes(),
            Err(e) => {
                eprintln!("Failed to release packages: {e}");
                return exit::ERROR;
            }
        },
        Commands::Pre(args) => match prerelease::run(&args) {
            Ok(true) => {}
            Ok(false) => return exit::no_changes(),
            Err(e) => {
                eprintln!("Failed to manage pre-release versions: {e}");
                return exit::ERROR;
            }
        },
        Commands::Update(args) => match update::run(&args) {
            Ok(true) => {}
            Ok(false) => return exit::no_changes(),
            Err(e) => {
                eprintln!("Failed to update Sampo: {e}");
                return exit::ERROR;
            }
        },
    }

    exit::SUCCESS
}

/// Checks for CLI updates and prints a hint if a newer version is available. Non-blocking, best-effort.
fn check_and_notify_update() {
    if let VersionCheckResult::UpdateAvailable { current, latest } =
        version_check::check_for_updates()
    {
        ui::log_hint(&format!(
            "A new version of Sampo is available: {current} → {latest}. Run `sampo update` to update."
        ));
    }
}
