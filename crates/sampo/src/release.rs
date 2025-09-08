use crate::cli::ReleaseArgs;
use sampo_core::run_release;
use std::io;

pub fn run(args: &ReleaseArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    run_in(&cwd, args)
}

pub fn run_in(root: &std::path::Path, args: &ReleaseArgs) -> io::Result<()> {
    run_release(root, args.dry_run)
}
