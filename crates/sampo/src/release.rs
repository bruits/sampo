use crate::cli::ReleaseArgs;
use sampo_core::errors::Result;
use sampo_core::run_release;

/// Runs the release command.
///
/// Returns `Ok(true)` if changes were made (or would be made in dry-run mode),
/// `Ok(false)` if there were no changesets to process.
pub fn run(args: &ReleaseArgs) -> Result<bool> {
    let cwd = std::env::current_dir()?;
    let output = run_release(&cwd, args.dry_run)?;

    Ok(!output.released_packages.is_empty())
}
