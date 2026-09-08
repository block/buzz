//! Small synchronous launcher entry point. No relay connection or async runtime.
use super::{parse_id, LAUNCH_MODE};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn try_run() -> io::Result<bool> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(LAUNCH_MODE)) {
        return Ok(false);
    }
    let directory = args
        .next()
        .ok_or_else(|| io::Error::other("missing Pi launcher directory"))?;
    let args: Vec<_> = args.collect();
    let mut command = pi_command()?;
    // pi-acp also invokes the launcher without RPC arguments for terminal login.
    if args
        .windows(2)
        .any(|pair| pair[0] == "--mode" && pair[1] == "rpc")
    {
        command
            .arg("--system-prompt")
            .arg(select_prompt(Path::new(&directory), &args)?);
    }
    command
        .arg("--skill")
        .arg(std::env::current_dir()?.join(".agents/skills"))
        .args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec())
    }
    #[cfg(not(unix))]
    {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn pi_command() -> io::Result<Command> {
    #[cfg(windows)]
    {
        // Rust does not search PATHEXT for an extensionless command. npm's Pi
        // installation provides pi.cmd; let std handle its command-line quoting.
        for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            for name in ["pi.exe", "pi.cmd", "pi.bat"] {
                let path = directory.join(name);
                if path.is_file() {
                    return Ok(Command::new(path));
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Pi was not found on PATH",
        ))
    }
    #[cfg(not(windows))]
    {
        Ok(Command::new("pi"))
    }
}

/// Restore IDs come from Pi's session header, not a filename convention.
/// Both pointer and header reads are bounded; missing snapshots fail closed.
pub(super) fn select_prompt(directory: &Path, args: &[OsString]) -> io::Result<PathBuf> {
    let mut restored = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--session" {
            restored = Some(
                iter.next()
                    .ok_or_else(|| io::Error::other("missing Pi session path"))?,
            );
        }
    }
    let pointer = if let Some(path) = restored {
        let mut header = String::new();
        BufReader::new(File::open(path)?.take(64 * 1024)).read_line(&mut header)?;
        if !header.ends_with('\n') {
            return Err(io::Error::other("Pi session header missing or too large"));
        }
        let header: serde_json::Value = serde_json::from_str(&header).map_err(io::Error::other)?;
        if header["type"] != "session" {
            return Err(io::Error::other("invalid Pi session header"));
        }
        let id = parse_id(
            header["id"]
                .as_str()
                .ok_or_else(|| io::Error::other("missing Pi session ID"))?,
        )?;
        directory.join(format!("session-{id}"))
    } else {
        directory.join("pending")
    };
    let mut token = String::new();
    File::open(pointer)?.take(128).read_to_string(&mut token)?;
    let token = parse_id(&token)?;
    let path = directory.join(format!("{token}.md"));
    // Pi treats a nonexistent --system-prompt path as literal prompt text.
    if !path.is_file() {
        return Err(io::Error::other("Pi system prompt snapshot is missing"));
    }
    Ok(path)
}
