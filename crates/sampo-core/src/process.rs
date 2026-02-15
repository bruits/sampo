use std::process::Command;

/// Creates a `Command` that can resolve `.cmd` and `.bat` scripts on Windows.
///
/// On Windows, tools like npm, pnpm, yarn, composer, and mix are installed
/// as `.cmd`/`.bat` batch scripts. Rust's `std::process::Command` only
/// auto-resolves `.exe` extensions, not `.cmd`/`.bat` (see rust-lang/rust#37519).
/// This function wraps the invocation through `cmd.exe /C` on Windows
/// to ensure proper resolution via PATHEXT.
pub fn command(program: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", program]);
        cmd
    } else {
        Command::new(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_creates_valid_command() {
        let cmd = command("test-program");

        if cfg!(windows) {
            assert_eq!(cmd.get_program(), "cmd");
            let args: Vec<_> = cmd.get_args().collect();
            assert_eq!(args, ["/C", "test-program"]);
        } else {
            assert_eq!(cmd.get_program(), "test-program");
            assert_eq!(cmd.get_args().count(), 0);
        }
    }
}
