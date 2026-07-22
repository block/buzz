//! `buzz capabilities` — machine-readable security-contract report.
//!
//! This command runs entirely locally: it makes no relay connection, reads no
//! private key material, and does not inspect `BUZZ_PRIVATE_KEY`,
//! `BUZZ_AUTH_TAG`, or any other environment variable. It exists so that
//! downstream automated verifiers (e.g. a "doctor" tool checking that a given
//! `buzz` build actually implements the `--private-key-fd` security
//! properties) have something more reliable to check than grepping
//! `--help` text.
//!
//! The emitted JSON is a **stable, versioned attestation contract** for
//! downstream tooling:
//!
//! - `schema_version` identifies the *shape* of the object. Bump it (and
//!   never repurpose or remove an existing field's meaning) if the shape of
//!   this JSON changes — additive-only evolution should still bump it so
//!   strict downstream parsers can detect the change.
//! - `capability_revision` identifies the specific implementation revision of
//!   the `--private-key-fd` feature being attested to
//!   (`secure_private_key_fd_v1`). If the underlying security properties
//!   change (e.g. a guarantee is weakened, removed, or a new one is added),
//!   mint a new revision string rather than silently redefining what the old
//!   one means.

use serde::Serialize;

use crate::error::CliError;
use crate::{MAX_PRIVATE_KEY_FD, MIN_PRIVATE_KEY_FD, PRIVATE_KEY_FD_MAX_LEN};

/// `schema_version` of the JSON object emitted by [`cmd_show`].
const SCHEMA_VERSION: u32 = 1;

/// Stable identifier for the exact set of `--private-key-fd` security
/// properties this report attests to. Only bump this if the contract's
/// meaning changes (e.g. one of the guarantees below is weakened, removed, or
/// a new one is added) — not on every unrelated code change.
const CAPABILITY_REVISION: &str = "secure_private_key_fd_v1";

/// Whether `--private-key-fd` is actually usable on this build. This must
/// fail closed: it is only `true` when compiled for Unix, where
/// [`crate::read_key_from_fd`] has a real `/dev/fd`-backed implementation. On
/// any other target (e.g. Windows) the flag is parsed by clap but the
/// underlying read always errors out, so this reports `false` rather than
/// claiming a working feature.
#[cfg(unix)]
const PRIVATE_KEY_FD_SUPPORTED: bool = true;

#[cfg(not(unix))]
const PRIVATE_KEY_FD_SUPPORTED: bool = false;

/// Security-relevant details of the `--private-key-fd` implementation.
///
/// All fields other than `supported` describe the implementation as it
/// exists in the Unix build and stay fixed regardless of target platform —
/// `supported` alone is what tells a downstream consumer whether this
/// platform's build actually backs those properties with a real
/// implementation.
#[derive(Serialize)]
struct PrivateKeyFdCapabilities {
    /// `true` only on Unix targets, where `--private-key-fd` has a real
    /// `/dev/fd`-backed implementation. Fails closed on other platforms.
    supported: bool,
    /// How the key material is handed to the process.
    transport: &'static str,
    /// Minimum accepted file descriptor number (inclusive).
    min_fd: u32,
    /// Maximum accepted file descriptor number (inclusive).
    max_fd: u32,
    /// Maximum number of bytes read from the fd for the key.
    max_key_bytes: usize,
    /// Whether the original fd passed via `--private-key-fd` is closed after
    /// being read.
    closes_original_fd: bool,
    /// Whether the in-memory buffer holding the key is zeroized.
    zeroizes_input: bool,
    /// Whether `--help` hides the live `BUZZ_PRIVATE_KEY` env value.
    help_env_value_redacted: bool,
}

/// Top-level `buzz capabilities` report.
#[derive(Serialize)]
struct CapabilitiesReport {
    /// Shape version of this JSON object. Bump on any shape change.
    schema_version: u32,
    /// Implementation revision identifier for the `--private-key-fd`
    /// security properties attested to below.
    capability_revision: &'static str,
    private_key_fd: PrivateKeyFdCapabilities,
}

impl Default for CapabilitiesReport {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            capability_revision: CAPABILITY_REVISION,
            private_key_fd: PrivateKeyFdCapabilities {
                supported: PRIVATE_KEY_FD_SUPPORTED,
                transport: "inherited_fd",
                min_fd: MIN_PRIVATE_KEY_FD,
                max_fd: MAX_PRIVATE_KEY_FD,
                max_key_bytes: PRIVATE_KEY_FD_MAX_LEN,
                closes_original_fd: true,
                zeroizes_input: true,
                help_env_value_redacted: true,
            },
        }
    }
}

/// Run `buzz capabilities`.
///
/// Prints one JSON object to stdout describing the security properties of
/// the `--private-key-fd` implementation and returns `Ok(())` (exit code 0).
/// Reads no environment variables and performs no I/O beyond stdout.
pub fn cmd_show() -> Result<(), CliError> {
    let report = CapabilitiesReport::default();
    let json = serde_json::to_string(&report)
        .map_err(|e| CliError::Other(format!("failed to serialize capabilities report: {e}")))?;
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_and_revision_are_stable() {
        assert_eq!(SCHEMA_VERSION, 1);
        assert_eq!(CAPABILITY_REVISION, "secure_private_key_fd_v1");
    }

    #[test]
    fn private_key_fd_supported_matches_platform() {
        // Fails closed: only true on Unix, where read_key_from_fd has a real
        // /dev/fd-backed implementation.
        assert_eq!(PRIVATE_KEY_FD_SUPPORTED, cfg!(unix));
    }

    #[test]
    fn cmd_show_succeeds() {
        assert!(cmd_show().is_ok());
    }

    #[test]
    fn report_matches_expected_contract() {
        let report = CapabilitiesReport::default();
        assert_eq!(report.schema_version, 1);
        assert_eq!(report.capability_revision, "secure_private_key_fd_v1");
        assert_eq!(report.private_key_fd.transport, "inherited_fd");
        assert_eq!(report.private_key_fd.min_fd, 3);
        assert_eq!(report.private_key_fd.max_fd, 1024);
        assert_eq!(report.private_key_fd.max_key_bytes, 256);
        assert!(report.private_key_fd.closes_original_fd);
        assert!(report.private_key_fd.zeroizes_input);
        assert!(report.private_key_fd.help_env_value_redacted);
        assert_eq!(report.private_key_fd.supported, cfg!(unix));
    }

    #[test]
    fn serializes_to_expected_json_shape() {
        let report = CapabilitiesReport::default();
        let value: serde_json::Value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["capability_revision"], "secure_private_key_fd_v1");
        assert_eq!(value["private_key_fd"]["transport"], "inherited_fd");
        assert_eq!(value["private_key_fd"]["min_fd"], 3);
        assert_eq!(value["private_key_fd"]["max_fd"], 1024);
        assert_eq!(value["private_key_fd"]["max_key_bytes"], 256);
        assert_eq!(value["private_key_fd"]["closes_original_fd"], true);
        assert_eq!(value["private_key_fd"]["zeroizes_input"], true);
        assert_eq!(value["private_key_fd"]["help_env_value_redacted"], true);
        assert_eq!(value["private_key_fd"]["supported"], cfg!(unix));
    }
}
