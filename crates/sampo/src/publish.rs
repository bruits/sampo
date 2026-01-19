use crate::cli::PublishArgs;
use sampo_core::errors::Result;
use sampo_core::run_publish;

/// Runs the publish command.
///
/// Returns `Ok(true)` if packages were published (or would be in dry-run mode),
/// `Ok(false)` if there were no packages to publish.
pub fn run(args: &PublishArgs) -> Result<bool> {
    let cwd = std::env::current_dir()?;
    let output = run_publish(&cwd, args.dry_run, &args.publish_args)?;

    Ok(!output.tags.is_empty())
}
