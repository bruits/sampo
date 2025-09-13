use crate::cli::PublishArgs;
use sampo_core::errors::Result;
use sampo_core::run_publish;

pub fn run(args: &PublishArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    run_publish(&cwd, args.dry_run, &args.cargo_args)
}
