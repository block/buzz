use std::{ffi::OsString, io::IsTerminal, path::PathBuf, process::Command};

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "buzz",
    about = "Buzz terminal — a fresh private channel on every launch",
    args_conflicts_with_subcommands = true
)]
struct TerminalArgs {
    /// Preview the terminal offline, without creating a channel.
    #[arg(long)]
    demo: bool,
    #[command(subcommand)]
    command: Option<TerminalCommand>,
}

#[derive(Subcommand)]
enum TerminalCommand {
    /// Open an existing channel. Never creates a replacement channel.
    Join { channel_id: uuid::Uuid },
}

pub(super) fn requested(args: &[OsString]) -> bool {
    args.len() == 1
        || args
            .get(1)
            .is_some_and(|arg| arg == "join" || arg == "--demo")
}

pub(super) fn run(args: Vec<OsString>) -> i32 {
    let parsed = match TerminalArgs::try_parse_from(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            let code = if error.use_stderr() { 1 } else { 0 };
            let _ = error.print();
            return code;
        }
    };
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!(
            "Buzz TUI needs an interactive terminal. Use buzz --help for scriptable commands."
        );
        return 1;
    }
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Cannot locate Buzz executable: {error}");
            return 1;
        }
    };
    let entry = std::env::var_os("BUZZ_TERMINAL_ENTRY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let installed = executable.with_file_name("../share/buzz/terminal/src/main.ts");
            if installed.is_file() {
                installed
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../terminal/src/main.ts")
            }
        });
    if !entry.is_file() {
        eprintln!("Terminal assets are not installed. Build from source with just terminal-build, or set BUZZ_TERMINAL_ENTRY to terminal/src/main.ts (Node 24 and pnpm dependencies required).");
        return 1;
    }
    let mut command = Command::new("node");
    command.arg(entry);
    if std::env::var_os("BUZZ_TERMINAL_HOST").is_none() {
        command.env(
            "BUZZ_TERMINAL_HOST",
            executable.with_file_name("buzz-terminal-host"),
        );
    }
    if parsed.demo {
        command.arg("--demo");
    }
    if let Some(TerminalCommand::Join { channel_id }) = parsed.command {
        command.arg("join").arg(channel_id.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        eprintln!(
            "Could not start Buzz terminal (Node 24 required): {}",
            command.exec()
        );
        1
    }
    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("Could not start Buzz terminal (Node 24 required): {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_human_entrypoints_launch_terminal() {
        for argv in [
            vec!["buzz"],
            vec!["buzz", "join", "id"],
            vec!["buzz", "--demo"],
        ] {
            assert!(requested(
                &argv.into_iter().map(OsString::from).collect::<Vec<_>>()
            ));
        }
        for argv in [
            vec!["buzz", "--help"],
            vec!["buzz", "channels", "join", "id"],
            vec!["buzz", "--format", "compact", "messages", "list"],
        ] {
            assert!(!requested(
                &argv.into_iter().map(OsString::from).collect::<Vec<_>>()
            ));
        }
        assert!(TerminalArgs::try_parse_from(["buzz"]).is_ok());
        assert!(TerminalArgs::try_parse_from(["buzz", "join"]).is_err());
        assert!(TerminalArgs::try_parse_from(["buzz", "join", "wrong"]).is_err());
        let parsed =
            TerminalArgs::try_parse_from(["buzz", "join", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"])
                .expect("join");
        assert!(matches!(parsed.command, Some(TerminalCommand::Join { .. })));
    }
}
