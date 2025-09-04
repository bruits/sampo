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

    /// Publish packages to registries (creates tags on success)
    Publish(PublishArgs),

    /// Consume changesets, bump versions, and update changelogs to prepare for release.
    Release(ReleaseArgs),
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
#[command(after_long_help = "\
Examples:\n  sampo publish --dry-run -- --allow-dirty\n  sampo publish -- --no-verify\n\nBehavior:\n  - Publishes only crates that have a git tag of the form <name>-v<version> for their current version.\n  - Skips crates whose current version already exists on crates.io.\n\nAll arguments after `--` are forwarded to `cargo publish` (separator required).")]
pub struct PublishArgs {
    /// Dry-run: simulate publish without pushing artifacts
    #[arg(long)]
    pub dry_run: bool,

    /// Extra flags passed through to `cargo publish` (must follow `--`)
    #[arg(last = true, value_name = "CARGO_ARG")]
    pub cargo_args: Vec<String>,
}

#[derive(Debug, Args, Default)]
pub struct ReleaseArgs {
    /// Dry-run: compute and show changes without modifying files
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
    fn parses_publish_dry_run() {
        let cli = Cli::try_parse_from(["sampo", "publish", "--dry-run"]).unwrap();
        match cli.command {
            Commands::Publish(args) => assert!(args.dry_run),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_publish_passthrough_flags() {
        let cli = Cli::try_parse_from([
            "sampo",
            "publish",
            "--dry-run",
            "--",
            "--allow-dirty",
            "--no-verify",
        ])
        .unwrap();
        match cli.command {
            Commands::Publish(args) => {
                assert!(args.dry_run);
                assert_eq!(args.cargo_args, vec!["--allow-dirty", "--no-verify"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn publish_rejects_passthrough_without_separator() {
        let res = Cli::try_parse_from(["sampo", "publish", "--dry-run", "--allow-dirty"]);
        assert!(res.is_err(), "should require `--` before cargo flags");
    }

    #[test]
    fn parses_release_dry_run() {
        let cli = Cli::try_parse_from(["sampo", "release", "--dry-run"]).unwrap();
        match cli.command {
            Commands::Release(args) => assert!(args.dry_run),
            _ => panic!("wrong variant"),
        }
    }
}
