use crate::cli::ReleaseArgs;
use sampo_core::run_release;
use std::io;

pub fn run(args: &ReleaseArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    let _output = run_release(&cwd, args.dry_run)?;
    // For now, we just ignore the output in the CLI since it already prints
    // the necessary information. In the future, this could be used for
    // additional reporting or integration with other tools.
    Ok(())
}
