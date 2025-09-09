use crate::cli::ReleaseArgs;
use sampo_core::run_release;
use std::io;

pub fn run(args: &ReleaseArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    let _ = run_release(&cwd, args.dry_run)?;

    Ok(())
}
