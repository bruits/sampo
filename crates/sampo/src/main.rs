mod cli;

use cli::{Cli, Commands};
use clap::Parser;

fn main() {
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
        Commands::Version(_args) => {
            // TODO: apply version bumps from changesets
        }
        Commands::Publish(_args) => {
            // TODO: publish packages to registries
        }
    }
}
