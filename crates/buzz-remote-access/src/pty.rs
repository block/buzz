//! Real terminal backend: a PTY running the user's normal shell, or `herdr session attach <name>` when the
//! desktop's terminal setting is herdr and the web asked for a named session. Plain-shell terminals are always
//! brand-new PTYs owned by this bridge (never attached to an agent's own terminal).
use std::io::{Read, Write};
use std::path::PathBuf;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::mpsc;

use crate::server::{SessionInfo, TermInput, TerminalBackend, TerminalHandle, TerminalLaunch};

#[derive(Clone, Debug)]
pub enum TerminalMode {
    /// The user's login shell.
    Shell,
    /// herdr: attach (or create) the named session; without a name a plain shell is used.
    Herdr { bin: PathBuf },
}

#[derive(Clone, Debug)]
pub struct PtyBackend {
    pub mode: TerminalMode,
    /// Overrides $SHELL (Settings → Buzz Term → Shell, and tests).
    pub shell: Option<PathBuf>,
    /// Starting directory (Settings → Buzz Term → Default directory). Must exist; otherwise the home directory is used.
    pub cwd: Option<PathBuf>,
    /// Reported to the web so it keeps that many lines of history (Settings → Buzz Term → Scrollback).
    pub scrollback: Option<u32>,
}

/// The shell to run: the configured one only if it is an absolute path to an executable file, else `$SHELL`, else zsh.
pub fn resolve_shell(configured: Option<&std::path::Path>, env_shell: Option<PathBuf>) -> PathBuf {
    if let Some(p) = configured {
        if p.is_absolute() && is_executable_file(p) {
            return p.to_path_buf();
        }
    }
    env_shell.unwrap_or_else(|| PathBuf::from("/bin/zsh"))
}

fn is_executable_file(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// The starting directory: `~`/`~/x` expanded, must be an existing directory, else the home directory.
pub fn resolve_cwd(configured: Option<&std::path::Path>, home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(c) = configured {
        let s = c.to_string_lossy();
        let expanded = if s == "~" {
            home.clone()
        } else if let Some(rest) = s.strip_prefix("~/") {
            home.clone().map(|h| h.join(rest))
        } else {
            Some(c.to_path_buf())
        };
        if let Some(d) = expanded {
            if d.is_absolute() && d.is_dir() {
                return Some(d);
            }
        }
    }
    home
}

impl PtyBackend {
    pub fn new(mode: TerminalMode) -> Self {
        Self { mode, shell: None, cwd: None, scrollback: None }
    }
    fn command(&self, l: &TerminalLaunch) -> CommandBuilder {
        let shell = resolve_shell(self.shell.as_deref(), std::env::var_os("SHELL").map(PathBuf::from));
        let mut cmd = match (&self.mode, &l.session) {
            (TerminalMode::Herdr { bin }, Some(name)) => {
                let mut c = CommandBuilder::new(bin);
                c.args(["session", "attach", name]);
                c
            }
            _ => {
                let mut c = CommandBuilder::new(&shell);
                c.arg("-l");
                c
            }
        };
        // The app's environment may hold secrets: start clean with only what a shell needs.
        cmd.env_clear();
        for k in ["HOME", "USER", "LOGNAME", "PATH", "LANG", "LC_ALL", "SHELL", "TMPDIR"] {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Some(dir) = resolve_cwd(self.cwd.as_deref(), std::env::var_os("HOME").map(PathBuf::from)) {
            cmd.cwd(dir);
        }
        cmd
    }
}

/// Parses `herdr session list` (whitespace columns: name status directory socket; first line is the header).
pub fn parse_herdr_sessions(out: &str) -> Vec<SessionInfo> {
    out.lines()
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (name, status) = (it.next()?, it.next()?);
            let ok = name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
            ok.then(|| SessionInfo { name: name.into(), status: status.into() })
        })
        .collect()
}

impl TerminalBackend for PtyBackend {
    fn list_sessions(&self) -> Vec<SessionInfo> {
        let TerminalMode::Herdr { bin } = &self.mode else { return vec![] };
        std::process::Command::new(bin)
            .args(["session", "list"])
            .stdin(std::process::Stdio::null())
            .output()
            .map(|o| parse_herdr_sessions(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    }

    fn open(&self, l: TerminalLaunch) -> Result<TerminalHandle, String> {
        let pair = native_pty_system()
            .openpty(PtySize { rows: l.rows, cols: l.cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| e.to_string())?;
        let mut child = pair.slave.spawn_command(self.command(&l)).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let (otx, orx) = mpsc::channel::<Vec<u8>>(64);
        let (itx, mut irx) = mpsc::channel::<TermInput>(64);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || otx.blocking_send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut killer = child.clone_killer();
        tokio::spawn(async move {
            let master = pair.master;
            while let Some(i) = irx.recv().await {
                match i {
                    TermInput::Data(d) => {
                        if writer.write_all(&d).and_then(|_| writer.flush()).is_err() {
                            break;
                        }
                    }
                    TermInput::Resize(c, r) => {
                        let _ = master.resize(PtySize { rows: r, cols: c, pixel_width: 0, pixel_height: 0 });
                    }
                    TermInput::Close => break,
                }
            }
            let _ = killer.kill();
            drop(writer);
            drop(master);
        });
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(TerminalHandle { output: orx, input: itx, scrollback: self.scrollback })
    }
}
