use clap::{Args, Parser, Subcommand};

/// Sampo CLI – automate changelogs, versioning, and publishing
#[derive(Debug, Parser)]
#[command(name = "sampo", version, about, long_about = None)]
pub struct Cli {
    /// Command to run
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Initialize Sampo in the current repository
    Init,

    /// Create a new changeset
    Add(AddArgs),

    /// Show pending changesets and planned releases
    Status,

    /// List workspace crates and internal dependencies
    List,

    /// Apply version bumps based on changesets
    Version(VersionArgs),

    /// Publish packages to registries
    Publish(PublishArgs),
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Optional package names to scope the changeset
    #[arg(short, long, num_args = 1.., value_name = "PACKAGE")]
    pub package: Vec<String>,

    /// Optional summary message for the changeset
    #[arg(short, long)]
    pub message: Option<String>,
}

#[derive(Debug, Args, Default)]
pub struct VersionArgs {
    /// Dry-run: compute versions without modifying files
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args, Default)]
pub struct PublishArgs {
    /// Dry-run: simulate publish without pushing artifacts
    #[arg(long)]
    pub dry_run: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_init() {
        let cli = Cli::try_parse_from(["sampo", "init"]).unwrap();
        match cli.command {
            Commands::Init => {}
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_add_with_packages_and_message() {
        let cli = Cli::try_parse_from([
            "sampo",
            "add",
            "-p",
            "pkg-a",
            "--package",
            "pkg-b",
            "-m",
            "feat: message",
        ])
        .unwrap();
        match cli.command {
            Commands::Add(args) => {
                assert_eq!(args.package, vec!["pkg-a", "pkg-b"]);
                assert_eq!(args.message.as_deref(), Some("feat: message"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_status() {
        let cli = Cli::try_parse_from(["sampo", "status"]).unwrap();
        match cli.command {
            Commands::Status => {}
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_list() {
        let cli = Cli::try_parse_from(["sampo", "list"]).unwrap();
        match cli.command {
            Commands::List => {}
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_version_dry_run() {
        let cli = Cli::try_parse_from(["sampo", "version", "--dry-run"]).unwrap();
        match cli.command {
            Commands::Version(args) => assert!(args.dry_run),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_publish_dry_run() {
        let cli = Cli::try_parse_from(["sampo", "publish", "--dry-run"]).unwrap();
        match cli.command {
            Commands::Publish(args) => assert!(args.dry_run),
            _ => panic!("wrong variant"),
        }
    }
}
