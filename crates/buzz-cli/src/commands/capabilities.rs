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
//! The emitted JSON is a **stable, versioned contract**: `schema_version`
//! identifies the shape of the object, and `capability_revision` identifies
//! the specific set of documented behaviors it attests to. Both fields must
//! be bumped (and the change called out in review) if the meaning of any
//! field changes — additive-only schema evolution should bump
//! `schema_version`, and a change to what `secure_private_key_fd_v1` actually
//! asserts should get a new `capability_revision` value instead of silently
//! redefining the old one.

use crate::error::CliError;
use crate::{MAX_PRIVATE_KEY_FD, MIN_PRIVATE_KEY_FD, PRIVATE_KEY_FD_MAX_LEN};

/// `schema_version` of the JSON object emitted by [`cmd_capabilities`].
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

/// Run `buzz capabilities`.
///
/// Prints one JSON object to stdout describing the security properties of
/// the `--private-key-fd` implementation and returns `Ok(())` (exit code 0).
/// Reads no environment variables and performs no I/O beyond stdout.
pub fn cmd_capabilities() -> Result<(), CliError> {
    let report = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "capability_revision": CAPABILITY_REVISION,
        "private_key_fd": {
            "supported": PRIVATE_KEY_FD_SUPPORTED,
            "transport": "inherited_fd",
            "min_fd": MIN_PRIVATE_KEY_FD,
            "max_fd": MAX_PRIVATE_KEY_FD,
            "max_key_bytes": PRIVATE_KEY_FD_MAX_LEN,
            "closes_original_fd": true,
            "zeroizes_input": true,
            "help_env_value_redacted": true,
        },
    });
    println!("{report}");
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
    fn cmd_capabilities_succeeds() {
        assert!(cmd_capabilities().is_ok());
    }
}
