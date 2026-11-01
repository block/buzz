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

/// Resolve the private key string, applying preference order and deprecation.
pub fn resolve_private_key(inputs: PrivateKeyInputs) -> Result<String, CliError> {
    let mut sources = 0u8;
    if inputs.private_key_file.is_some() {
        sources += 1;
    }
    if inputs.private_key_stdin {
        sources += 1;
    }
    if inputs.private_key.is_some() {
        sources += 1;
    }
    if sources > 1 {
        return Err(CliError::Usage(
            "specify only one of --private-key-file, --private-key-stdin, or BUZZ_PRIVATE_KEY/--private-key"
                .into(),
        ));
    }

    if let Some(path) = inputs.private_key_file.as_deref() {
        return read_private_key_file(path);
    }

    if inputs.private_key_stdin {
        return read_private_key_stdin();
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
        return Ok(trimmed);
    }

    Err(CliError::Auth(
        "BUZZ_PRIVATE_KEY is required (prefer --private-key-file / --private-key-stdin / env; \
--private-key is deprecated)".into(),
    ))
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

/// Detect whether `--private-key` was present on argv (vs env-only).
pub fn private_key_flag_on_argv<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|a| {
        let a = a.as_ref();
        a == "--private-key" || a.starts_with("--private-key=")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn argv_detector_matches_flag_forms() {
        assert!(private_key_flag_on_argv(["buzz", "--private-key", "nsec1x"]));
        assert!(private_key_flag_on_argv(["buzz", "--private-key=nsec1x"]));
        assert!(!private_key_flag_on_argv(["buzz", "channels", "list"]));
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
        assert_eq!(key, "nsec1filekey");
    }

    #[test]
    fn rejects_multiple_sources() {
        let err = resolve_private_key(PrivateKeyInputs {
            private_key: Some("nsec1a".into()),
            private_key_stdin: true,
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn env_value_without_argv_does_not_require_file() {
        let key = resolve_private_key(PrivateKeyInputs {
            private_key: Some("nsec1env".into()),
            private_key_from_argv: false,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(key, "nsec1env");
    }
}
