mod cli;
mod workspace;

use cli::{Cli, Commands};
use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => {
            // TODO: initialize sampo in this repository
        }
        Commands::Add(_args) => {
            // TODO: create a new changeset
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
