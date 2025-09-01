mod add;
mod cli;
mod config;
mod init;
mod names;
mod workspace;

use clap::Parser;
use cli::{Cli, Commands};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => {
            let cwd = std::env::current_dir().unwrap();
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
                    eprintln!("init error: {}", e);
                    return ExitCode::from(1);
                }
            }
        }
        Commands::Add(args) => {
            if let Err(e) = add::run(&args) {
                eprintln!("add error: {}", e);
                return ExitCode::from(1);
            }
        }
        Commands::Status => {
            // TODO: show pending releases/changesets
        }
        Commands::List => {
            let cwd = std::env::current_dir().unwrap();
            match workspace::Workspace::discover_from(&cwd) {
                Ok(ws) => {
                    for krate in ws.members {
                        println!(
                            "{} {}\n  path: {}\n  internal_deps: {}\n",
                            krate.name,
                            krate.version,
                            krate.path.display(),
                            display_set(&krate.internal_deps)
                        );
                    }
                }
                Err(e) => {
                    eprintln!("workspace error: {}", e);
                    return ExitCode::from(1);
                }
            }
        }
        Commands::Version(_args) => {
            // TODO: apply version bumps from changesets
        }
        Commands::Publish(_args) => {
            // TODO: publish packages to registries
        }
    }
    ExitCode::SUCCESS
}

fn display_set(s: &std::collections::BTreeSet<String>) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.iter().cloned().collect::<Vec<_>>().join(", ")
}
