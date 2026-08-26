//! Buzz trusted-operation broker — host implementation.
//!
//! The broker performs a small set of named business operations that need the
//! owner's credentials, on behalf of a requester who cannot hold them. Desktop
//! is host #1; the boundary is drawn so a hosted authority can replace it.
//!
//! # Why this lives entirely in Rust
//!
//! Rust already owns every primitive this depends on: signature verification
//! and NIP-44 decrypt (`commands::identity`), owner key custody
//! (`AppState::signing_keys`), the authoritative agent roster
//! (`managed_agents::storage`), and every agent mutation
//! (`commands::agents`, `commands::agent_models_update`). Routing authority
//! through the frontend would move the trust boundary away from the credentials
//! and split one authority operation across IPC — where the inputs are
//! spoofable and a crash can land between the mutation and its record. The
//! frontend may forward frames and render diagnostics; it is not the security
//! principal.
//!
//! # Guarantee
//!
//! **At-most-once execution plus `indeterminate`** — not exactly-once, and not
//! durable completion. See [`store`] for why, and for what a capability must
//! own to do better.
//!
//! # Delivery is ephemeral
//!
//! Requests arrive as kind-24200 observer frames, which the relay routes to
//! connected subscribers without storing. Execution while the host is offline
//! is therefore unsupported: no queue holds the request, and the requester's
//! wait expires. That is a deliberate limit of this transport, not an oversight.

use sha2::{Digest, Sha256};

pub mod agents_policy;
pub mod desktop_agents;
pub mod handlers;
pub mod host;
pub mod ingress;
pub mod pipeline;
pub mod store;

/// Maximum accepted size of a decrypted broker request payload, in bytes.
///
/// Bounded before hashing or parsing so an oversized frame cannot force
/// unbounded work. Generous next to the largest legitimate request (a 20k-char
/// system prompt) and far below the observer plaintext ceiling.
pub const MAX_REQUEST_BYTES: usize = 96 * 1024;

/// Hash the exact decrypted request bytes.
///
/// The digest is computed **only here**, on the host, from the bytes as
/// received. Nothing re-serializes the request first, so there is no canonical
/// encoding to agree on and no second implementation that could drift from this
/// one. A retry that resends identical bytes hashes identically; a retry that
/// changes anything hashes differently and is refused as a conflict rather than
/// silently answered with the first request's outcome.
///
/// # Errors
///
/// Returns an error when the payload exceeds [`MAX_REQUEST_BYTES`].
pub fn request_digest(payload: &[u8]) -> Result<String, String> {
    if payload.len() > MAX_REQUEST_BYTES {
        return Err(format!(
            "broker request exceeds {MAX_REQUEST_BYTES} bytes (got {})",
            payload.len()
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(payload);
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests;
