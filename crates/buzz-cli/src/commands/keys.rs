//! `buzz keys` subcommands — local Nostr identity operations.
//!
//! These commands run entirely on the machine that invokes them. They make no
//! relay request and, unlike every other subcommand, do not require
//! `BUZZ_PRIVATE_KEY` to already be set — `keys generate` is how that value
//! comes into existence in the first place.
//!
//! This matters for self-hosted agents. An agent that runs on its own machine
//! should mint its own identity there, so the secret is created where it will
//! be used and never has to be transported from somewhere else. Generating the
//! key on an operator workstation and copying it to the agent host inverts
//! that: the secret exists in two places, and the operator's machine becomes a
//! custodian of an identity it does not run.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use nostr::{Keys, ToBech32};

use crate::error::CliError;

/// Permission bits for a freshly written secret-key file: owner read/write only.
#[cfg(unix)]
const SECRET_FILE_MODE: u32 = 0o600;

/// Run `buzz keys generate`.
///
/// Mints a fresh secp256k1 keypair and reports the **public** half on stdout.
/// The secret half is written to `out` and is printed only when `stdout_secret`
/// is set — an explicit opt-in for callers that pipe into their own secret
/// store rather than a file.
///
/// `force` permits overwriting an existing `out` path. Without it an existing
/// file is an error: re-running a connect flow must not be able to silently
/// destroy the identity a live agent is already using, which would orphan
/// every message that agent has ever signed.
pub fn cmd_generate(out: Option<&str>, stdout_secret: bool, force: bool) -> Result<(), CliError> {
    if out.is_none() && !stdout_secret {
        return Err(CliError::Usage(
            "no destination for the generated secret key: pass --out <path> to write \
             it to a file, or --stdout to print it"
                .into(),
        ));
    }

    let keys = Keys::generate();
    let pubkey = keys.public_key();
    let npub = pubkey
        .to_bech32()
        .map_err(|e| CliError::Other(format!("failed to encode npub: {e}")))?;
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| CliError::Other(format!("failed to encode nsec: {e}")))?;

    let written = match out {
        Some(path) => Some(write_secret_file(Path::new(path), &nsec, force)?),
        None => None,
    };

    // Ordering is deliberate: the file is on disk before anything is printed,
    // so a caller that reads stdout and then reads the path can never observe
    // a pubkey whose secret was not persisted.
    let mut report = serde_json::json!({
        "pubkey": pubkey.to_hex(),
        "npub": npub,
    });
    if let Some(path) = &written {
        report["secret_key_path"] = serde_json::json!(path.display().to_string());
    }
    if stdout_secret {
        report["nsec"] = serde_json::json!(nsec);
    }
    println!("{report}");
    Ok(())
}

/// Create `path` and write `nsec` to it with owner-only permissions.
///
/// The file is created with its restrictive mode from the outset via
/// `OpenOptions::mode` rather than being chmod-ed afterwards — a
/// create-then-chmod sequence leaves a window in which the secret is on disk
/// world-readable.
///
/// Returns the canonical path that was written.
fn write_secret_file(path: &Path, nsec: &str, force: bool) -> Result<PathBuf, CliError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(CliError::Usage(format!(
                "directory does not exist: {}",
                parent.display()
            )));
        }
    }

    let mut options = OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        // create_new fails if the path exists, which is the guard we want —
        // and it is atomic, so two concurrent generates cannot both believe
        // they created the file.
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(SECRET_FILE_MODE);
    }

    let mut file = options.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => CliError::Usage(format!(
            "refusing to overwrite existing key file: {} (pass --force to replace it, \
             but note that any agent already using this identity will lose it)",
            path.display()
        )),
        _ => CliError::Other(format!("failed to create {}: {e}", path.display())),
    })?;

    // `--force` reuses an existing inode, whose mode is whatever it already
    // was; `OpenOptions::mode` only applies on creation. Re-assert the mode so
    // the overwrite path cannot leave a permissive file behind.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(SECRET_FILE_MODE))
            .map_err(|e| {
                CliError::Other(format!(
                    "failed to set permissions on {}: {e}",
                    path.display()
                ))
            })?;
    }

    writeln!(file, "{nsec}")
        .map_err(|e| CliError::Other(format!("failed to write {}: {e}", path.display())))?;
    file.sync_all()
        .map_err(|e| CliError::Other(format!("failed to flush {}: {e}", path.display())))?;

    Ok(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}

pub fn dispatch(cmd: crate::KeysCmd) -> Result<(), CliError> {
    use crate::KeysCmd;
    match cmd {
        KeysCmd::Generate { out, stdout, force } => cmd_generate(out.as_deref(), stdout, force),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_a_destination() {
        // Neither --out nor --stdout: the secret would be generated and
        // immediately discarded, which is never what the caller meant.
        let err = cmd_generate(None, false, false).expect_err("expected usage error");
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn writes_secret_file_with_owner_only_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.nsec");
        let written = write_secret_file(&path, "nsec1test", false).unwrap();

        let contents = std::fs::read_to_string(&written).unwrap();
        assert_eq!(contents.trim(), "nsec1test");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&written).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, SECRET_FILE_MODE);
        }
    }

    #[test]
    fn refuses_to_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.nsec");
        write_secret_file(&path, "nsec1original", false).unwrap();

        let err = write_secret_file(&path, "nsec1replacement", false)
            .expect_err("expected overwrite refusal");
        assert!(matches!(err, CliError::Usage(_)));

        // The original identity survives the refused write.
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.trim(), "nsec1original");
    }

    #[test]
    fn force_overwrites_and_keeps_owner_only_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.nsec");
        write_secret_file(&path, "nsec1original", false).unwrap();

        // Loosen the mode so the re-assert has something to correct.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        write_secret_file(&path, "nsec1replacement", true).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.trim(), "nsec1replacement");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, SECRET_FILE_MODE);
        }
    }

    #[test]
    fn rejects_missing_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-such-dir").join("agent.nsec");
        let err = write_secret_file(&path, "nsec1test", false).expect_err("expected usage error");
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn generated_secret_round_trips_to_the_reported_pubkey() {
        // The whole point of the command is that the caller can later load the
        // written secret and arrive at the pubkey that was printed. Prove the
        // encode/parse pair agrees rather than trusting it.
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let reloaded = Keys::parse(&nsec).unwrap();
        assert_eq!(reloaded.public_key(), keys.public_key());
    }
}
