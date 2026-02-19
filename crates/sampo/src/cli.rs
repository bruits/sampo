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

    /// Manage pre-release versions for workspace packages
    Pre(PreArgs),

    /// Update Sampo CLI to the latest version
    Update(UpdateArgs),
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
Examples:\n  sampo publish --dry-run -- --access restricted\n  sampo publish --cargo-args --allow-dirty -- --tag beta\n\nBehavior:\n  - Skips packages whose current version already exists on their registry.\n  - Creates git tags after publishing (<name>-v<version>, or v<version> with git.short_tags).\n\nAll arguments after `--` are forwarded to ALL underlying publish commands (separator required).\nUse --cargo-args, --npm-args, --hex-args, --pypi-args, or --packagist-args to forward arguments\nto a specific ecosystem only.")]
pub struct PublishArgs {
    /// Dry-run: simulate publish without pushing artifacts
    #[arg(long)]
    pub dry_run: bool,

    /// Extra arguments forwarded only to Cargo (e.g. --cargo-args --allow-dirty)
    #[arg(long, num_args = 1.., value_delimiter = ' ', allow_hyphen_values = true)]
    pub cargo_args: Option<Vec<String>>,

    /// Extra arguments forwarded only to npm/pnpm/yarn/bun (e.g. --npm-args --access restricted)
    #[arg(long, num_args = 1.., value_delimiter = ' ', allow_hyphen_values = true)]
    pub npm_args: Option<Vec<String>>,

    /// Extra arguments forwarded only to Hex/Mix
    #[arg(long, num_args = 1.., value_delimiter = ' ', allow_hyphen_values = true)]
    pub hex_args: Option<Vec<String>>,

    /// Extra arguments forwarded only to PyPI
    #[arg(long, num_args = 1.., value_delimiter = ' ', allow_hyphen_values = true)]
    pub pypi_args: Option<Vec<String>>,

    /// Extra arguments forwarded only to Packagist/Composer
    #[arg(long, num_args = 1.., value_delimiter = ' ', allow_hyphen_values = true)]
    pub packagist_args: Option<Vec<String>>,

    /// Extra flags passed through to ALL underlying publish commands (must follow `--`)
    #[arg(last = true, value_name = "PUBLISH_ARG")]
    pub publish_args: Vec<String>,
}

#[derive(Debug, Args, Default)]
pub struct ReleaseArgs {
    /// Dry-run: compute and show changes without modifying files
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args, Default)]
pub struct PreArgs {
    #[command(subcommand)]
    pub command: Option<PreCommands>,
}

#[derive(Debug, Subcommand)]
pub enum PreCommands {
    /// Enter pre-release mode for selected packages
    Enter(PreEnterArgs),

    /// Exit pre-release mode for selected packages
    Exit(PreExitArgs),
}

#[derive(Debug, Args)]
pub struct PreEnterArgs {
    /// Pre-release label to apply (alpha, beta, rc, etc.)
    pub label: Option<String>,

    /// Packages to update (prompted interactively if omitted)
    #[arg(short, long, num_args = 1.., value_name = "PACKAGE")]
    pub package: Vec<String>,
}

#[derive(Debug, Args, Default)]
pub struct PreExitArgs {
    /// Packages to update (prompted interactively if omitted)
    #[arg(short, long, num_args = 1.., value_name = "PACKAGE")]
    pub package: Vec<String>,
}

#[derive(Debug, Args, Default)]
pub struct UpdateArgs {
    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Include pre-release versions (alpha, beta, rc, etc.)
    #[arg(long)]
    pub pre: bool,
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
                assert_eq!(args.publish_args, vec!["--allow-dirty", "--no-verify"]);
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

    #[test]
    fn parses_pre_enter_with_label_and_packages() {
        let cli = Cli::try_parse_from(["sampo", "pre", "enter", "alpha", "-p", "foo"]).unwrap();
        match cli.command {
            Commands::Pre(pre) => match pre.command {
                Some(PreCommands::Enter(args)) => {
                    assert_eq!(args.label.as_deref(), Some("alpha"));
                    assert_eq!(args.package, vec!["foo"]);
                }
                _ => panic!("wrong variant"),
            },
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_pre_exit_with_packages() {
        let cli = Cli::try_parse_from(["sampo", "pre", "exit", "--package", "foo"]).unwrap();
        match cli.command {
            Commands::Pre(pre) => match pre.command {
                Some(PreCommands::Exit(args)) => {
                    assert_eq!(args.package, vec!["foo"]);
                }
                _ => panic!("wrong variant"),
            },
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_pre_without_subcommand() {
        let cli = Cli::try_parse_from(["sampo", "pre"]).unwrap();
        match cli.command {
            Commands::Pre(args) => assert!(args.command.is_none()),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_update() {
        let cli = Cli::try_parse_from(["sampo", "update"]).unwrap();
        match cli.command {
            Commands::Update(args) => assert!(!args.yes),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_update_with_yes() {
        let cli = Cli::try_parse_from(["sampo", "update", "--yes"]).unwrap();
        match cli.command {
            Commands::Update(args) => {
                assert!(args.yes);
                assert!(!args.pre);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_update_with_pre() {
        let cli = Cli::try_parse_from(["sampo", "update", "--pre"]).unwrap();
        match cli.command {
            Commands::Update(args) => {
                assert!(!args.yes);
                assert!(args.pre);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_update_with_yes_and_pre() {
        let cli = Cli::try_parse_from(["sampo", "update", "--yes", "--pre"]).unwrap();
        match cli.command {
            Commands::Update(args) => {
                assert!(args.yes);
                assert!(args.pre);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_publish_cargo_args_with_hyphen_values() {
        let cli = Cli::try_parse_from([
            "sampo",
            "publish",
            "--cargo-args",
            "--allow-dirty",
            "--no-verify",
        ])
        .unwrap();
        match cli.command {
            Commands::Publish(args) => {
                assert_eq!(
                    args.cargo_args,
                    Some(vec!["--allow-dirty".to_string(), "--no-verify".to_string()])
                );
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_publish_ecosystem_args_with_universal() {
        let cli = Cli::try_parse_from([
            "sampo",
            "publish",
            "--cargo-args=--allow-dirty",
            "--npm-args=--access",
            "--",
            "--tag",
            "beta",
        ])
        .unwrap();
        match cli.command {
            Commands::Publish(args) => {
                assert_eq!(args.cargo_args, Some(vec!["--allow-dirty".to_string()]));
                assert_eq!(args.npm_args, Some(vec!["--access".to_string()]));
                assert_eq!(args.publish_args, vec!["--tag", "beta"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_publish_all_ecosystem_args() {
        let cli = Cli::try_parse_from([
            "sampo",
            "publish",
            "--cargo-args=--allow-dirty",
            "--npm-args=--access",
            "--hex-args=--replace",
            "--pypi-args=--skip-existing",
            "--packagist-args=--no-interaction",
        ])
        .unwrap();
        match cli.command {
            Commands::Publish(args) => {
                assert_eq!(args.cargo_args, Some(vec!["--allow-dirty".to_string()]));
                assert_eq!(args.npm_args, Some(vec!["--access".to_string()]));
                assert_eq!(args.hex_args, Some(vec!["--replace".to_string()]));
                assert_eq!(args.pypi_args, Some(vec!["--skip-existing".to_string()]));
                assert_eq!(
                    args.packagist_args,
                    Some(vec!["--no-interaction".to_string()])
                );
            }
            _ => panic!("wrong variant"),
        }
    }
}
