//! Resolve the CLI identity secret without requiring it on argv.
//!
//! Prefer `BUZZ_PRIVATE_KEY`, `--private-key-file`, or `--private-key-stdin`.
//! Bare `--private-key` remains accepted but warns: argv values land in shell
//! history and process listings (see block/buzz#4032).

use crate::error::CliError;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Sources that can supply the Nostr private key for relay commands.
#[derive(Debug, Clone, Default)]
pub struct PrivateKeyInputs {
    /// Value from `--private-key` / `BUZZ_PRIVATE_KEY` (clap merges both).
    pub private_key: Option<String>,
    /// Path from `--private-key-file`.
    pub private_key_file: Option<std::path::PathBuf>,
    /// When true, read a single line/key from stdin (`--private-key-stdin`).
    pub private_key_stdin: bool,
    /// True when `--private-key` appeared on argv (not only via the env var).
    pub private_key_from_argv: bool,
}

/// Resolve the private key string, or leave identity selection to the desktop when no key was supplied.
pub fn resolve_private_key(inputs: PrivateKeyInputs) -> Result<Option<String>, CliError> {
    // An explicit flag overrides BUZZ_PRIVATE_KEY. The env var is injected for
    // managed agents, so it must not reject --private-key-file or --private-key-stdin.
    // Those flags still conflict with each other and with argv --private-key.
    let explicit = u8::from(inputs.private_key_file.is_some())
        + u8::from(inputs.private_key_stdin)
        + u8::from(inputs.private_key_from_argv);
    if explicit > 1 {
        return Err(CliError::Usage(
            "specify only one of --private-key, --private-key-file, or --private-key-stdin"
                .into(),
        ));
    }

    if let Some(path) = inputs.private_key_file.as_deref() {
        return read_private_key_file(path).map(Some);
    }

    if inputs.private_key_stdin {
        return read_private_key_stdin().map(Some);
    }

    if let Some(key) = inputs.private_key {
        if inputs.private_key_from_argv {
            eprintln!(
                "warning: --private-key puts the secret in shell history and process listings; \
prefer BUZZ_PRIVATE_KEY, --private-key-file, or --private-key-stdin (see https://github.com/block/buzz/issues/4032)"
            );
        }
        let trimmed = key.trim().to_owned();
        if trimmed.is_empty() {
            return Err(CliError::Auth(
                "BUZZ_PRIVATE_KEY is empty (use --private-key-file, --private-key-stdin, or set env var)"
                    .into(),
            ));
        }
        return Ok(Some(trimmed));
    }

    Ok(None)
}

fn read_private_key_file(path: &Path) -> Result<String, CliError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                eprintln!(
                    "warning: private key file {} is group/world-accessible (mode {:o}); prefer chmod 600",
                    path.display(),
                    mode
                );
            }
        }
    }

    let contents = fs::read_to_string(path).map_err(|e| {
        CliError::Auth(format!(
            "failed to read --private-key-file {}: {e}",
            path.display()
        ))
    })?;
    let trimmed = contents.trim().to_owned();
    if trimmed.is_empty() {
        return Err(CliError::Auth(format!(
            "--private-key-file {} is empty",
            path.display()
        )));
    }
    Ok(trimmed)
}

fn read_private_key_stdin() -> Result<String, CliError> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| CliError::Auth(format!("failed to read --private-key-stdin: {e}")))?;
    let trimmed = buf.trim().to_owned();
    if trimmed.is_empty() {
        return Err(CliError::Auth(
            "--private-key-stdin produced an empty key".into(),
        ));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn no_key_leaves_desktop_credential_selection_available() {
        assert_eq!(
            resolve_private_key(PrivateKeyInputs::default()).unwrap(),
            None
        );
    }

    #[test]
    fn prefers_file_contents() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "  nsec1filekey  ").unwrap();
        let key = resolve_private_key(PrivateKeyInputs {
            private_key_file: Some(file.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(key.as_deref(), Some("nsec1filekey"));
    }

    #[test]
    fn rejects_argv_key_combined_with_another_flag() {
        let err = resolve_private_key(PrivateKeyInputs {
            private_key: Some("nsec1a".into()),
            private_key_stdin: true,
            private_key_from_argv: true,
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn file_overrides_env_key() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "nsec1fromfile").unwrap();
        let key = resolve_private_key(PrivateKeyInputs {
            private_key: Some("nsec1env".into()),
            private_key_file: Some(file.path().to_path_buf()),
            private_key_from_argv: false,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(key.as_deref(), Some("nsec1fromfile"));
    }

    #[test]
    fn env_value_without_argv_does_not_require_file() {
        let key = resolve_private_key(PrivateKeyInputs {
            private_key: Some("nsec1env".into()),
            private_key_from_argv: false,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(key.as_deref(), Some("nsec1env"));
    }
}
