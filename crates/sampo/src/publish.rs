use crate::cli::PublishArgs;
use sampo_core::run_publish;
use std::io;

pub fn run(args: &PublishArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    run_publish(&cwd, args.dry_run, &args.cargo_args)
}
