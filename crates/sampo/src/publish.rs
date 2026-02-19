use crate::cli::PublishArgs;
use sampo_core::PublishExtraArgs;
use sampo_core::errors::Result;
use sampo_core::run_publish;

/// Runs the publish command.
///
/// Returns `Ok(true)` if packages were published (or would be in dry-run mode),
/// `Ok(false)` if there were no packages to publish.
pub fn run(args: &PublishArgs) -> Result<bool> {
    let cwd = std::env::current_dir()?;

    let extra_args = PublishExtraArgs {
        universal: args.publish_args.clone(),
        cargo: args.cargo_args.clone().unwrap_or_default(),
        npm: args.npm_args.clone().unwrap_or_default(),
        hex: args.hex_args.clone().unwrap_or_default(),
        pypi: args.pypi_args.clone().unwrap_or_default(),
        packagist: args.packagist_args.clone().unwrap_or_default(),
    };

    let output = run_publish(&cwd, args.dry_run, &extra_args)?;

    Ok(!output.tags.is_empty())
}
