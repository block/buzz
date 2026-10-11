//! Real terminal backend: a PTY running the user's normal shell, or `herdr session attach <name>` when the
//! desktop's terminal setting is herdr and the web asked for a named session. Plain-shell terminals are always
//! brand-new PTYs owned by this bridge (never attached to an agent's own terminal).
use std::io::{Read, Write};
use std::path::PathBuf;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::mpsc;

use crate::server::{TermInput, TerminalBackend, TerminalHandle, TerminalLaunch};

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
    /// Overrides $SHELL (tests).
    pub shell: Option<PathBuf>,
}

impl PtyBackend {
    pub fn new(mode: TerminalMode) -> Self {
        Self { mode, shell: None }
    }
    fn command(&self, l: &TerminalLaunch) -> CommandBuilder {
        let shell = self
            .shell
            .clone()
            .or_else(|| std::env::var_os("SHELL").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
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
        if let Some(home) = std::env::var_os("HOME") {
            cmd.cwd(home);
        }
        cmd
    }
}

impl TerminalBackend for PtyBackend {
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
        Ok(TerminalHandle { output: orx, input: itx })
    }
}
