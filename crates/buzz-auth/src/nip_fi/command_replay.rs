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
//!
//! The same atomic step also records the deny entry durably, so a restarted
//! pod can restore it (see [`CommandReplayGuard::load_denies`]).  Every check
//! (replay, shared capacity) runs before any write: a `Replay` or `Full`
//! outcome leaves the store unchanged.

use std::{future::Future, pin::Pin};

use chrono::{DateTime, Utc};
use nostr::PublicKey;

use sha2::{Digest, Sha256};

use crate::error::AuthError;

/// One authenticated command, as presented to the shared store.
#[derive(Debug, Clone, Copy)]
pub struct CommandClaim<'a> {
    /// Exact `iss` of the command.
    pub issuer: &'a str,
    /// The command's `jti`.
    pub jti: &'a str,
    /// Claim TTL; implementations clamp it as the NIP-98 guard does.
    pub ttl_secs: u64,
    /// The signed target key.
    pub target: &'a PublicKey,
    /// The signed `until`, exact.
    pub until: DateTime<Utc>,
    /// This issuer's `deny_set_capacity`, enforced across every pod.
    pub capacity: usize,
    /// Verifier clock: entries with `until <= now` are expired.
    pub now: DateTime<Utc>,
}

/// Outcome of [`CommandReplayGuard::try_claim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The claim is new; the deny entry (if `until > now`) is recorded.
    Claimed,
    /// `(iss, jti)` was already claimed.  Nothing was written.
    Replay,
    /// The issuer's shared deny set is full.  Nothing was written, so the
    /// same command may be retried.
    Full,
}

/// Shared seen-set and durable deny store for NIP-FI commands.
///
/// The production implementation lives in `buzz-pubsub` (one Redis script).
/// An in-memory implementation is provided behind
/// `cfg(any(test, feature = "test-utils"))`.
pub trait CommandReplayGuard: Send + Sync {
    /// Atomically check replay and shared capacity, then claim `(iss, jti)`
    /// and record the deny entry.
    ///
    /// On `Err` the outcome is unknown (the write may have applied and only
    /// its reply been lost); callers MUST fail closed.
    fn try_claim<'a>(
        &'a self,
        claim: CommandClaim<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ClaimOutcome, AuthError>> + Send + 'a>>;

    /// Every recorded deny entry for `issuer` with `until > now`, at its exact
    /// signed `until`.
    fn load_denies<'a>(&'a self, issuer: &'a str, now: DateTime<Utc>) -> LoadDenies<'a>;
}

/// The future [`CommandReplayGuard::load_denies`] returns.
pub type LoadDenies<'a> = Pin<
    Box<dyn Future<Output = Result<Vec<(PublicKey, DateTime<Utc>)>, LoadDeniesError>> + Send + 'a>,
>;

/// Why [`CommandReplayGuard::load_denies`] failed.
#[derive(Debug, thiserror::Error)]
pub enum LoadDeniesError {
    /// The store could not be read.
    #[error(transparent)]
    Store(#[from] AuthError),
    /// A stored deny record could not be decoded.  `readable` holds every
    /// active record that did decode, so the caller can still apply them;
    /// the load is still a failure.
    #[error("unreadable deny record: {record}")]
    Unreadable {
        /// The first record that could not be decoded.
        record: String,
        /// The active records that decoded.
        readable: Vec<(PublicKey, DateTime<Utc>)>,
    },
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
    namespaced_command_replay_key(COMMAND_REPLAY_PREFIX, issuer, jti)
}

/// Key prefix for enforce-mode command replay claims.
pub const COMMAND_REPLAY_PREFIX: &str = "buzz:nip-fi:command";

/// Key prefix for shadow-mode claims, so a shadow accept never uses up the
/// enforce claim for the same `(iss, jti)` in a shared store.
pub const SHADOW_COMMAND_REPLAY_PREFIX: &str = "buzz:nip-fi:shadow-command";

/// [`command_replay_key`] under an explicit prefix.
pub fn namespaced_command_replay_key(prefix: &str, issuer: &str, jti: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [issuer, jti] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{prefix}:{}", hex::encode(hasher.finalize()))
}

/// Process-local store for tests.  Instances shared via `Arc` model pods
/// sharing one Redis.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Default)]
pub struct InMemoryCommandReplayGuard(std::sync::Mutex<InMemoryStore>);

#[cfg(any(test, feature = "test-utils"))]
#[derive(Default)]
struct InMemoryStore {
    claims: std::collections::HashSet<String>,
    denies: std::collections::HashMap<String, std::collections::HashMap<PublicKey, DateTime<Utc>>>,
}

#[cfg(any(test, feature = "test-utils"))]
impl CommandReplayGuard for InMemoryCommandReplayGuard {
    fn try_claim<'a>(
        &'a self,
        claim: CommandClaim<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ClaimOutcome, AuthError>> + Send + 'a>> {
        let key = command_replay_key(claim.issuer, claim.jti);
        let mut store = self.0.lock().expect("replay store");
        let outcome = if store.claims.contains(&key) {
            ClaimOutcome::Replay
        } else {
            let active = claim.until > claim.now;
            let denies = store.denies.entry(claim.issuer.to_owned()).or_default();
            denies.retain(|_, until| *until > claim.now);
            if active && !denies.contains_key(claim.target) && denies.len() >= claim.capacity {
                ClaimOutcome::Full
            } else {
                if active {
                    let until = denies.entry(*claim.target).or_insert(claim.until);
                    *until = (*until).max(claim.until);
                }
                store.claims.insert(key);
                ClaimOutcome::Claimed
            }
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn load_denies<'a>(&'a self, issuer: &'a str, now: DateTime<Utc>) -> LoadDenies<'a> {
        let denies = self
            .0
            .lock()
            .expect("replay store")
            .denies
            .get(issuer)
            .map(|d| {
                d.iter()
                    .filter(|(_, u)| **u > now)
                    .map(|(k, u)| (*k, *u))
                    .collect()
            })
            .unwrap_or_default();
        Box::pin(async move { Ok(denies) })
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
