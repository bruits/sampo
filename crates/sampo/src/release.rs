use crate::cli::ReleaseArgs;
use sampo_core::errors::Result;
use sampo_core::run_release;

pub fn run(args: &ReleaseArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let _ = run_release(&cwd, args.dry_run)?;

    Ok(())
}
