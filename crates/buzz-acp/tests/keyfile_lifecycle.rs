//! Process-level regression for the git-identity keyfile leak Gurney found:
//! on an error/timeout exit `buzz-acp` calls `shutdown().await`, which now
//! deletes the 0600 nostr keyfile tempdir explicitly (relying on `TempDir`'s
//! `Drop` right before `std::process::exit` leaked it ~80% of the time). This
//! exercises the REAL binary on its error-exit path and asserts the keyfile
//! dir does not survive.

use std::path::Path;
use std::process::Command;

use nostr::ToBech32;

/// Count `buzz-acp-git-*` identity tempdirs under `tmp`.
fn identity_dirs(tmp: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(tmp)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("buzz-acp-git-"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `buzz-acp models` against an agent that exits immediately hits the
/// init-error exit path. With `NOSTR_PRIVATE_KEY` set, the identity keyfile is
/// installed at spawn; `shutdown()` must delete it before the destructor-less
/// exit. Repeated because the pre-fix leak was ~80% probabilistic — a single
/// run could pass spuriously against the broken code.
#[test]
fn error_exit_removes_identity_keyfile() {
    for attempt in 0..8 {
        // Isolate the tempdir so we only observe this run's identity dirs.
        let tmp = tempfile::tempdir().unwrap();

        // A generated key so `install_git_identity` actually writes a keyfile.
        let nsec = nostr::Keys::generate().secret_key().to_bech32().unwrap();

        // `false` spawns then exits, so `initialize()` fails fast and the
        // harness takes the error exit through `shutdown_and_exit`.
        let output = Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
            .args(["models", "--agent-command", "false", "--agent-args", ""])
            .env("TMPDIR", tmp.path())
            .env("NOSTR_PRIVATE_KEY", &nsec)
            .output()
            .expect("run buzz-acp models");

        // The error exit is expected — we assert on the side effect, not success.
        assert!(
            !output.status.success(),
            "attempt {attempt}: expected non-zero exit on the error path; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let leaked = identity_dirs(tmp.path());
        assert!(
            leaked.is_empty(),
            "attempt {attempt}: identity keyfile dir leaked on the error-exit path: \
             {leaked:?}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
