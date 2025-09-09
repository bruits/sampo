mod add;
mod cli;
mod init;
mod names;
mod publish;
mod release;

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
                    eprintln!("init error: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        Commands::Add(args) => {
            if let Err(e) = add::run(&args) {
                eprintln!("add error: {e}");
                return ExitCode::from(1);
            }
        }
        Commands::Publish(args) => {
            if let Err(e) = publish::run(&args) {
                eprintln!("publish error: {e}");
                return ExitCode::from(1);
            }
        }
        Commands::Release(args) => {
            if let Err(e) = release::run(&args) {
                eprintln!("release error: {e}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}
