//! Deriving the agent's public identity from the key it was minted with.
//!
//! `docs/remote-agents.md` §Deploy, **Step 0 — derive and verify identity**:
//!
//! > The payload carries the nsec, not the pubkey. Before any substrate read or
//! > mutation, the provider MUST parse `private_key_nsec` and derive the public
//! > key from it; a malformed or undecodable key is an immediate in-band error.
//! > Every selector, name, and comparison below uses the *derived* pubkey —
//! > never a caller-supplied one.
//!
//! That is the strict reading, and it is the one this module implements. The
//! payload's `pubkey` field is retained on the wire, but it is demoted from
//! *identity* to *assertion*: [`derive_pubkey`] produces the identity, and a
//! payload pubkey that disagrees with it is a fatal in-band error rather than a
//! value that wins. The reason the field cannot simply be trusted is the same
//! reason the spec says "never a caller-supplied one": the unit slug, the env
//! file name and `backend_agent_id` are all keyed on it, so a payload whose
//! pubkey and nsec disagree would name a unit after one identity and run it
//! under another — an agent that looks deployed and is permanently unreachable,
//! which is exactly the failure the fail-closed nsec check already guards.
//!
//! Key material is handled as `Zeroizing` throughout: the bech32 decode yields
//! the 32 secret bytes, and they are wiped when this module is done with them.

use bech32::primitives::decode::CheckedHrpstring;
use bech32::Bech32;
use secp256k1::{Secp256k1, SecretKey};
use zeroize::Zeroizing;

use crate::protocol::Secret;

/// The human-readable part of a Nostr private key (NIP-19).
const NSEC_HRP: &str = "nsec";

/// Derive the agent's 64-character lowercase hex x-only public key from its
/// bech32 `nsec`.
///
/// The error strings are deliberately shape-only. A decode failure must not
/// echo any part of the key back into a response the desktop persists in
/// `last_error`.
pub fn derive_pubkey(nsec: &Secret) -> Result<String, String> {
    let raw = nsec.expose().trim();

    let checked = CheckedHrpstring::new::<Bech32>(raw).map_err(|_| {
        "'private_key_nsec' is not a valid bech32 Nostr private key: the agent's identity \
         cannot be derived from it"
            .to_string()
    })?;
    if checked.hrp().to_lowercase() != NSEC_HRP {
        return Err(format!(
            "'private_key_nsec' has bech32 prefix '{}', expected '{NSEC_HRP}': the payload \
             carries the wrong kind of key",
            checked.hrp().to_lowercase()
        ));
    }

    let bytes = Zeroizing::new(checked.byte_iter().collect::<Vec<u8>>());
    if bytes.len() != 32 {
        return Err(format!(
            "'private_key_nsec' decodes to {} bytes, expected 32",
            bytes.len()
        ));
    }

    let secret = SecretKey::from_byte_array(
        <[u8; 32]>::try_from(bytes.as_slice()).expect("length checked above"),
    )
    .map_err(|_| {
        "'private_key_nsec' decodes to bytes that are not a valid secp256k1 secret key".to_string()
    })?;

    let (x_only, _parity) = secret.x_only_public_key(&Secp256k1::new());
    Ok(hex_lower(&x_only.serialize()))
}

/// Reconcile the derived identity against a pubkey the payload also carried.
///
/// The derived value always wins — this never returns the asserted one. A
/// mismatch is fatal because the two identities would be used for different
/// things: the slug and every host-side name would follow the assertion while
/// the running harness authenticated as the derived key.
pub fn reconcile(derived: String, asserted: Option<&str>) -> Result<String, String> {
    match asserted {
        Some(claimed) if claimed.to_lowercase() != derived => Err(format!(
            "deploy payload's 'pubkey' does not match the identity derived from \
             'private_key_nsec': the unit would be named for {} while the harness ran as {}. \
             Refusing to deploy an agent no desktop surface could reach.",
            fragment(&claimed.to_lowercase()),
            fragment(&derived)
        )),
        _ => Ok(derived),
    }
}

/// A short, non-secret prefix of a public key, for error messages that must
/// distinguish two identities without printing two 64-character strings.
fn fragment(pubkey: &str) -> String {
    let end = pubkey
        .char_indices()
        .nth(12)
        .map_or(pubkey.len(), |(i, _)| i);
    format!("{}…", &pubkey[..end])
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NIP-19 test vector: the nsec for the all-`0x01` secret key, and the
    /// x-only public key secp256k1 derives from it. Fixed so a change in how
    /// the key is decoded cannot silently move every agent's unit slug.
    const NSEC: &str = "nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftw";
    const PUBKEY: &str = "1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f";

    fn secret(value: &str) -> Secret {
        serde_json::from_value(serde_json::json!(value)).unwrap()
    }

    #[test]
    fn a_valid_nsec_derives_its_known_pubkey() {
        assert_eq!(derive_pubkey(&secret(NSEC)).unwrap(), PUBKEY);
    }

    #[test]
    fn derivation_is_stable_across_calls_and_surrounding_whitespace() {
        let first = derive_pubkey(&secret(NSEC)).unwrap();
        let padded = derive_pubkey(&secret(&format!("  {NSEC}\n"))).unwrap();
        assert_eq!(first, padded);
        assert_eq!(first.len(), 64);
        assert!(first
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn a_malformed_nsec_is_rejected() {
        for bad in [
            "",
            "not-bech32-at-all",
            // Valid bech32 shape, wrong checksum.
            "nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftx",
            // Hex, not bech32 — the shape a caller might paste by mistake.
            "1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f",
        ] {
            let error = derive_pubkey(&secret(bad)).unwrap_err();
            assert!(
                error.contains("private_key_nsec"),
                "{bad:?} produced {error:?}"
            );
            assert!(
                !error.contains(bad) || bad.is_empty(),
                "error echoed the key"
            );
        }
    }

    #[test]
    fn an_npub_is_rejected_as_the_wrong_kind_of_key() {
        // A public key in the private key's field: correct bech32, wrong hrp.
        let npub = "npub1rwzv24nmzfjypx2a8m264ws9vht3uxp5vpypnluuzl67n4waq78suk0wul";
        let error = derive_pubkey(&secret(npub)).unwrap_err();
        assert!(error.contains("expected 'nsec'"), "{error}");
    }

    #[test]
    fn reconcile_accepts_a_matching_assertion_and_returns_the_derived_value() {
        let derived = derive_pubkey(&secret(NSEC)).unwrap();
        assert_eq!(
            reconcile(derived.clone(), Some(PUBKEY)).unwrap(),
            derived,
            "the derived value is the identity"
        );
        // Case is not identity: the desktop mints lowercase hex, but an
        // uppercase assertion still describes the same key.
        assert_eq!(
            reconcile(derived.clone(), Some(&PUBKEY.to_uppercase())).unwrap(),
            derived
        );
        // No assertion at all is fine — the nsec is sufficient.
        assert_eq!(reconcile(derived.clone(), None).unwrap(), derived);
    }

    #[test]
    fn reconcile_rejects_a_mismatched_assertion() {
        let derived = derive_pubkey(&secret(NSEC)).unwrap();
        let other = "f".repeat(64);
        let error = reconcile(derived, Some(&other)).unwrap_err();
        assert!(error.contains("does not match"), "{error}");
        // Both identities are named, neither in full.
        assert!(error.contains("ffffffffffff…"), "{error}");
        assert!(error.contains("1b84c5567b12…"), "{error}");
    }
}
