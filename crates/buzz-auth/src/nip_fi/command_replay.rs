//! NIP-FI command replay protection — shared, deployment-wide `(iss, jti)` claim.
//!
//! [`super::command::CommandVerifier`] reserves `(iss, jti)` in the receiving
//! pod's deny map, but a command accepted by one pod is not visible to another
//! pod's map.  Like NIP-98 replay protection ([`crate::nip98_replay`]), the
//! cross-pod fence is an atomic set-if-absent in shared state (Redis).
//!
//! The claim is taken after the command is fully authenticated (so a forgery
//! cannot burn a legitimate `jti`) and after this pod has reserved its local
//! deny-entry slot (so a `503 deny set full` never consumes the command).

use std::{future::Future, pin::Pin};

use sha2::{Digest, Sha256};

use crate::error::AuthError;

/// Shared seen-set for NIP-FI command `(iss, jti)` pairs.
///
/// The production implementation lives in `buzz-pubsub` (Redis `SET NX EX`).
/// An in-memory implementation is provided behind
/// `cfg(any(test, feature = "test-utils"))`.
pub trait CommandReplayGuard: Send + Sync {
    /// Atomically claim `(issuer, jti)`.
    ///
    /// Returns `Ok(true)` when newly claimed (proceed) and `Ok(false)` when
    /// already claimed (the caller MUST reject the command as replay).  On
    /// `Err` callers MUST fail closed.  Implementations MUST use an atomic
    /// set-if-absent and clamp `ttl_secs` to
    /// [`crate::DEFAULT_REPLAY_TTL_SECS`]..=[`crate::MAX_REPLAY_TTL_SECS`],
    /// as the NIP-98 guard does.
    fn try_claim<'a>(
        &'a self,
        issuer: &'a str,
        jti: &'a str,
        ttl_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>>;
}

/// Redis key for a command replay claim:
/// `buzz:nip-fi:command:{sha256(len(iss) || iss || len(jti) || jti)}`.
///
/// `iss` is an arbitrary URI and `jti` is up to 512 bytes, so the pair is
/// hashed with big-endian `u64` length prefixes: the key is fixed-length and
/// no two distinct pairs share an encoding.  Commands are deployment-wide
/// (they close sessions in every community), so the key has no community
/// scope.  The `nip-fi:command` segment never matches a NIP-98 key, whose
/// final segments are always `:nip98:{event_id_hex}`.
pub fn command_replay_key(issuer: &str, jti: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [issuer, jti] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("buzz:nip-fi:command:{}", hex::encode(hasher.finalize()))
}

/// Process-local seen-set for tests.  Instances shared via `Arc` model pods
/// sharing one Redis.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Default)]
pub struct InMemoryCommandReplayGuard(std::sync::Mutex<std::collections::HashSet<String>>);

#[cfg(any(test, feature = "test-utils"))]
impl CommandReplayGuard for InMemoryCommandReplayGuard {
    fn try_claim<'a>(
        &'a self,
        issuer: &'a str,
        jti: &'a str,
        _ttl_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
        let key = command_replay_key(issuer, jti);
        Box::pin(async move { Ok(self.0.lock().expect("replay set").insert(key)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_length_prefixed_and_fixed_length() {
        // Concatenation-ambiguous pairs must map to distinct keys.
        assert_ne!(command_replay_key("ab", "c"), command_replay_key("a", "bc"));
        let long = command_replay_key("https://issuer.example", &"j".repeat(512));
        assert_eq!(long.len(), "buzz:nip-fi:command:".len() + 64);
        assert!(!long.contains(":nip98:"));
    }
}
