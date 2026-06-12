//! NIP-OA — Owner Attestation
//!
//! Computes and verifies `auth` tags that prove an owner key authorized
//! an agent key to publish events under the agent's own authorship.
//!
//! # Tag format
//!
//! ```json
//! ["auth", "<owner-pubkey-hex>", "<conditions>", "<sig-hex>"]
//! ```
//!
//! # Signing preimage
//!
//! ```text
//! preimage = "nostr:agent-auth:" || agent_pubkey_hex || ":" || conditions
//! message  = SHA256(preimage)
//! sig      = BIP-340 Schnorr(message, owner_secret_key)
//! ```

use core::str::FromStr;

use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::hashes::Hash;
use nostr::secp256k1::schnorr::Signature;
use nostr::secp256k1::Message;
use nostr::{Keys, PublicKey, Tag, SECP256K1};
use serde_json::Value;

use crate::SdkError;

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Validate the `conditions` string per the NIP-OA spec.
///
/// Empty string is valid. Non-empty must be `clause` or `clause&clause&...`
/// where each clause is `kind=<0-65535>`, `created_at<<0-4294967295>`, or
/// `created_at><0-4294967295>`. Canonical decimals only (no leading zeros).
fn validate_conditions(conditions: &str) -> Result<(), SdkError> {
    if conditions.is_empty() {
        return Ok(());
    }

    // No whitespace anywhere
    if conditions.bytes().any(|b| b.is_ascii_whitespace()) {
        return Err(SdkError::InvalidInput(
            "conditions must not contain whitespace".into(),
        ));
    }

    // Split on '&' — each part must be non-empty and a valid clause
    for clause in conditions.split('&') {
        if clause.is_empty() {
            return Err(SdkError::InvalidInput(
                "empty clause in conditions (leading/trailing/double '&')".into(),
            ));
        }
        validate_clause(clause)?;
    }

    Ok(())
}

fn validate_clause(clause: &str) -> Result<(), SdkError> {
    if let Some(value) = clause.strip_prefix("kind=") {
        validate_canonical_decimal(value, 0, 65535, "kind")
    } else if let Some(value) = clause.strip_prefix("created_at<") {
        validate_canonical_decimal(value, 0, 4294967295, "created_at<")
    } else if let Some(value) = clause.strip_prefix("created_at>") {
        validate_canonical_decimal(value, 0, 4294967295, "created_at>")
    } else {
        Err(SdkError::InvalidInput(format!(
            "unsupported clause: {clause:?}"
        )))
    }
}

fn validate_canonical_decimal(s: &str, min: u64, max: u64, label: &str) -> Result<(), SdkError> {
    if s.is_empty() {
        return Err(SdkError::InvalidInput(format!(
            "{label} value must not be empty"
        )));
    }

    // No leading zeros except "0" itself
    if s.len() > 1 && s.starts_with('0') {
        return Err(SdkError::InvalidInput(format!(
            "{label} value has leading zero: {s:?}"
        )));
    }

    // Must be all digits
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SdkError::InvalidInput(format!(
            "{label} value is not a valid decimal: {s:?}"
        )));
    }

    let value: u64 = s
        .parse()
        .map_err(|e| SdkError::InvalidInput(format!("{label} value out of range: {e}")))?;

    if value < min || value > max {
        return Err(SdkError::InvalidInput(format!(
            "{label} value {value} out of range [{min}, {max}]"
        )));
    }

    Ok(())
}

fn build_preimage(agent_pubkey: &PublicKey, conditions: &str) -> String {
    format!("nostr:agent-auth:{}:{}", agent_pubkey.to_hex(), conditions)
}

fn hash_preimage(preimage: &str) -> Message {
    let digest = Sha256Hash::hash(preimage.as_bytes());
    Message::from_digest(digest.to_byte_array())
}

/// Check that a character is a lowercase hex digit (`0-9`, `a-f`).
/// Nostr convention requires lowercase hex for pubkeys and signatures.
fn is_lowercase_hex(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, 'a'..='f')
}

fn parse_json_array(s: &str) -> Result<Vec<Value>, SdkError> {
    let v: Value = serde_json::from_str(s)
        .map_err(|e| SdkError::InvalidInput(format!("invalid JSON: {e}")))?;
    match v {
        Value::Array(arr) => Ok(arr),
        _ => Err(SdkError::InvalidInput(
            "auth tag must be a JSON array".into(),
        )),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Compute a NIP-OA `auth` tag authorizing `agent_pubkey` under `conditions`.
///
/// Signs the preimage with `owner_keys` using BIP-340 Schnorr.
///
/// Returns a JSON string of the form:
/// `["auth","<owner_pubkey_hex>","<conditions>","<sig_hex>"]`
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] if `owner_pubkey == agent_pubkey`
/// (self-attestation is meaningless and rejected).
pub fn compute_auth_tag(
    owner_keys: &Keys,
    agent_pubkey: &PublicKey,
    conditions: &str,
) -> Result<String, SdkError> {
    let owner_pubkey = owner_keys.public_key();
    if owner_pubkey == *agent_pubkey {
        return Err(SdkError::InvalidInput(
            "owner and agent pubkeys must differ (self-attestation rejected)".into(),
        ));
    }

    validate_conditions(conditions)?;

    let preimage = build_preimage(agent_pubkey, conditions);
    let message = hash_preimage(&preimage);
    let sig = owner_keys.sign_schnorr(&message);

    let tag_json = serde_json::json!(["auth", owner_pubkey.to_hex(), conditions, sig.to_string(),]);
    Ok(tag_json.to_string())
}

/// Verify a NIP-OA `auth` tag JSON string against the given `agent_pubkey`.
///
/// Reconstructs the preimage, hashes it, and verifies the Schnorr signature
/// against the owner pubkey embedded in the tag.
///
/// Returns the owner's [`PublicKey`] on success.
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] for malformed JSON, wrong element count,
/// bad hex, self-attestation, or signature verification failure.
pub fn verify_auth_tag(
    auth_tag_json: &str,
    agent_pubkey: &PublicKey,
) -> Result<PublicKey, SdkError> {
    let arr = parse_json_array(auth_tag_json)?;

    if arr.len() != 4 {
        return Err(SdkError::InvalidInput(format!(
            "auth tag must have 4 elements, got {}",
            arr.len()
        )));
    }

    let label = arr[0]
        .as_str()
        .ok_or_else(|| SdkError::InvalidInput("element 0 must be a string".into()))?;
    if label != "auth" {
        return Err(SdkError::InvalidInput(format!(
            "first element must be \"auth\", got \"{label}\""
        )));
    }

    let owner_pubkey_hex = arr[1].as_str().ok_or_else(|| {
        SdkError::InvalidInput("element 1 (owner pubkey) must be a string".into())
    })?;
    let conditions = arr[2]
        .as_str()
        .ok_or_else(|| SdkError::InvalidInput("element 2 (conditions) must be a string".into()))?;
    let sig_hex = arr[3]
        .as_str()
        .ok_or_else(|| SdkError::InvalidInput("element 3 (signature) must be a string".into()))?;

    let owner_pubkey = PublicKey::from_hex(owner_pubkey_hex)
        .map_err(|e| SdkError::InvalidInput(format!("invalid owner pubkey: {e}")))?;

    validate_conditions(conditions)?;

    if owner_pubkey == *agent_pubkey {
        return Err(SdkError::InvalidInput(
            "owner and agent pubkeys must differ (self-attestation rejected)".into(),
        ));
    }

    let sig = Signature::from_str(sig_hex)
        .map_err(|e| SdkError::InvalidInput(format!("invalid signature hex: {e}")))?;

    let preimage = build_preimage(agent_pubkey, conditions);
    let message = hash_preimage(&preimage);

    let xonly = owner_pubkey.xonly().map_err(|e| {
        SdkError::InvalidInput(format!("owner pubkey xonly conversion failed: {e}"))
    })?;
    SECP256K1
        .verify_schnorr(&sig, &message, &xonly)
        .map_err(|e| SdkError::InvalidInput(format!("signature verification failed: {e}")))?;

    Ok(owner_pubkey)
}

/// Parse a NIP-OA `auth` tag JSON string into a [`Tag`] without verifying the
/// signature.
///
/// Validates structure only:
/// - Exactly 4 elements
/// - First element is `"auth"`
/// - Second element is a 64-character hex string (owner pubkey)
/// - Fourth element is a 128-character hex string (signature)
///
/// This is the fast path used at MCP startup — no crypto is performed.
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] for any structural violation.
pub fn parse_auth_tag(json_str: &str) -> Result<Tag, SdkError> {
    let arr = parse_json_array(json_str)?;

    if arr.len() != 4 {
        return Err(SdkError::InvalidInput(format!(
            "auth tag must have 4 elements, got {}",
            arr.len()
        )));
    }

    let label = arr[0]
        .as_str()
        .ok_or_else(|| SdkError::InvalidInput("element 0 must be a string".into()))?;
    if label != "auth" {
        return Err(SdkError::InvalidInput(format!(
            "first element must be \"auth\", got \"{label}\""
        )));
    }

    let owner_pubkey_hex = arr[1].as_str().ok_or_else(|| {
        SdkError::InvalidInput("element 1 (owner pubkey) must be a string".into())
    })?;
    if owner_pubkey_hex.len() != 64 || !owner_pubkey_hex.chars().all(is_lowercase_hex) {
        return Err(SdkError::InvalidInput(format!(
            "owner pubkey must be 64 hex chars, got {:?}",
            owner_pubkey_hex
        )));
    }

    let conditions = arr[2]
        .as_str()
        .ok_or_else(|| SdkError::InvalidInput("element 2 (conditions) must be a string".into()))?;

    validate_conditions(conditions)?;

    let sig_hex = arr[3]
        .as_str()
        .ok_or_else(|| SdkError::InvalidInput("element 3 (signature) must be a string".into()))?;
    if sig_hex.len() != 128 || !sig_hex.chars().all(is_lowercase_hex) {
        return Err(SdkError::InvalidInput(format!(
            "signature must be 128 hex chars, got length {}",
            sig_hex.len()
        )));
    }

    Tag::parse(["auth", owner_pubkey_hex, conditions, sig_hex])
        .map_err(|e| SdkError::InvalidInput(format!("failed to construct Tag: {e}")))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER_PUBKEY_HEX: &str =
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const AGENT_PUBKEY_HEX: &str =
        "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
    const CONDITIONS: &str = "kind=1&created_at<1713957000";
    const SPEC_SIG_HEX: &str =
        "8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369";

    /// Verify the spec's provided signature against the spec's known preimage.
    #[test]
    fn test_verify_spec_test_vector() {
        let agent_pubkey = PublicKey::from_hex(AGENT_PUBKEY_HEX).unwrap();
        let owner_pubkey = PublicKey::from_hex(OWNER_PUBKEY_HEX).unwrap();

        let preimage = build_preimage(&agent_pubkey, CONDITIONS);
        assert_eq!(
            preimage,
            format!("nostr:agent-auth:{}:{}", AGENT_PUBKEY_HEX, CONDITIONS)
        );

        let message = hash_preimage(&preimage);

        let sig = Signature::from_str(SPEC_SIG_HEX).expect("spec sig must parse");
        let xonly = owner_pubkey.xonly().expect("valid test pubkey");
        SECP256K1
            .verify_schnorr(&sig, &message, &xonly)
            .expect("spec test vector signature must verify");
    }

    /// Sign with generated keys, then verify — round-trip without byte comparison.
    #[test]
    fn test_sign_then_verify_round_trip() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key();

        let tag_json = compute_auth_tag(&owner_keys, &agent_pubkey, "kind=9")
            .expect("compute_auth_tag must succeed");

        let recovered =
            verify_auth_tag(&tag_json, &agent_pubkey).expect("verify_auth_tag must succeed");

        assert_eq!(recovered, owner_keys.public_key());
    }

    /// Empty conditions string is valid.
    #[test]
    fn test_empty_conditions() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key();

        let tag_json = compute_auth_tag(&owner_keys, &agent_pubkey, "")
            .expect("empty conditions must succeed");

        let recovered = verify_auth_tag(&tag_json, &agent_pubkey)
            .expect("verify with empty conditions must succeed");

        assert_eq!(recovered, owner_keys.public_key());
    }

    /// Self-attestation (owner == agent) must be rejected at both sign and verify.
    #[test]
    fn test_reject_self_attestation() {
        let keys = Keys::generate();
        let pubkey = keys.public_key();

        let err = compute_auth_tag(&keys, &pubkey, "kind=9")
            .expect_err("self-attestation must be rejected");
        assert!(
            matches!(err, SdkError::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );

        // Craft a self-attesting tag and verify it's rejected.
        let fake_json =
            serde_json::json!(["auth", pubkey.to_hex(), "kind=9", "a".repeat(128),]).to_string();
        let err = verify_auth_tag(&fake_json, &pubkey)
            .expect_err("self-attestation must be rejected at verify");
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    /// Various malformed inputs to verify_auth_tag must return errors.
    #[test]
    fn test_reject_malformed_tag() {
        let agent_pubkey = PublicKey::from_hex(AGENT_PUBKEY_HEX).unwrap();

        assert!(verify_auth_tag("not json", &agent_pubkey).is_err());
        assert!(verify_auth_tag(r#"{"auth":"x"}"#, &agent_pubkey).is_err());
        assert!(verify_auth_tag(r#"["auth","a","b"]"#, &agent_pubkey).is_err());

        let bad_label =
            serde_json::json!(["notauth", OWNER_PUBKEY_HEX, CONDITIONS, "a".repeat(128)])
                .to_string();
        assert!(verify_auth_tag(&bad_label, &agent_pubkey).is_err());

        let bad_pk =
            serde_json::json!(["auth", "notahex", CONDITIONS, "a".repeat(128)]).to_string();
        assert!(verify_auth_tag(&bad_pk, &agent_pubkey).is_err());

        let bad_sig =
            serde_json::json!(["auth", OWNER_PUBKEY_HEX, CONDITIONS, "notasig"]).to_string();
        assert!(verify_auth_tag(&bad_sig, &agent_pubkey).is_err());

        // Valid structure but wrong signature (won't verify).
        let wrong_sig =
            serde_json::json!(["auth", OWNER_PUBKEY_HEX, CONDITIONS, "0".repeat(128)]).to_string();
        assert!(verify_auth_tag(&wrong_sig, &agent_pubkey).is_err());
    }

    /// parse_auth_tag with a well-formed JSON array returns a Tag.
    #[test]
    fn test_parse_auth_tag_valid() {
        let sig_hex = "a".repeat(128);
        let json = serde_json::json!(["auth", OWNER_PUBKEY_HEX, CONDITIONS, sig_hex,]).to_string();

        let tag = parse_auth_tag(&json).expect("valid tag must parse");
        let slice = tag.as_slice();
        assert_eq!(slice[0], "auth");
        assert_eq!(slice[1], OWNER_PUBKEY_HEX);
        assert_eq!(slice[2], CONDITIONS);
        assert_eq!(slice[3], "a".repeat(128));
    }

    /// Various malformed inputs to parse_auth_tag must return errors.
    #[test]
    fn test_parse_auth_tag_malformed() {
        assert!(parse_auth_tag("not json").is_err());
        assert!(parse_auth_tag(r#"{"auth":"x"}"#).is_err());
        assert!(parse_auth_tag(r#"["auth","a","b"]"#).is_err());

        let bad_label =
            serde_json::json!(["notauth", OWNER_PUBKEY_HEX, CONDITIONS, "a".repeat(128)])
                .to_string();
        assert!(parse_auth_tag(&bad_label).is_err());

        let short_pk = serde_json::json!(["auth", "abcd", CONDITIONS, "a".repeat(128)]).to_string();
        assert!(parse_auth_tag(&short_pk).is_err());

        let non_hex_pk =
            serde_json::json!(["auth", "z".repeat(64), CONDITIONS, "a".repeat(128)]).to_string();
        assert!(parse_auth_tag(&non_hex_pk).is_err());

        let short_sig =
            serde_json::json!(["auth", OWNER_PUBKEY_HEX, CONDITIONS, "abcd"]).to_string();
        assert!(parse_auth_tag(&short_sig).is_err());

        let non_hex_sig =
            serde_json::json!(["auth", OWNER_PUBKEY_HEX, CONDITIONS, "z".repeat(128)]).to_string();
        assert!(parse_auth_tag(&non_hex_sig).is_err());
    }

    /// Uppercase hex must be rejected — Nostr convention is lowercase only.
    #[test]
    fn test_parse_auth_tag_rejects_uppercase_hex() {
        // Uppercase in owner pubkey
        let upper_pk = OWNER_PUBKEY_HEX.to_uppercase();
        let json = serde_json::json!(["auth", upper_pk, CONDITIONS, "a".repeat(128)]).to_string();
        assert!(
            parse_auth_tag(&json).is_err(),
            "uppercase owner pubkey must be rejected"
        );

        // Mixed case in owner pubkey
        let mut mixed_pk = OWNER_PUBKEY_HEX.to_string();
        mixed_pk.replace_range(0..1, "A");
        let json = serde_json::json!(["auth", mixed_pk, CONDITIONS, "a".repeat(128)]).to_string();
        assert!(
            parse_auth_tag(&json).is_err(),
            "mixed-case owner pubkey must be rejected"
        );

        // Uppercase in signature
        let json =
            serde_json::json!(["auth", OWNER_PUBKEY_HEX, CONDITIONS, "A".repeat(128)]).to_string();
        assert!(
            parse_auth_tag(&json).is_err(),
            "uppercase signature must be rejected"
        );
    }

    /// Verify the spec's SHA-256 hash of the preimage matches the expected value.
    #[test]
    fn test_spec_sha256_hash() {
        let agent_pubkey = PublicKey::from_hex(AGENT_PUBKEY_HEX).unwrap();
        let preimage = build_preimage(&agent_pubkey, CONDITIONS);
        let digest = Sha256Hash::hash(preimage.as_bytes());
        let expected = "08cdecd55af4c28d3801fd69615dcf5cc04fab3bc134b38a840bf157197069a6";
        assert_eq!(format!("{digest:x}"), expected);
    }

    // ── Conditions validation ─────────────────────────────────────────────

    #[test]
    fn test_valid_conditions() {
        // These should all pass through validate_conditions
        assert!(validate_conditions("").is_ok());
        assert!(validate_conditions("kind=1").is_ok());
        assert!(validate_conditions("kind=0").is_ok());
        assert!(validate_conditions("kind=65535").is_ok());
        assert!(validate_conditions("created_at<1713957000").is_ok());
        assert!(validate_conditions("created_at>0").is_ok());
        assert!(validate_conditions("created_at>4294967295").is_ok());
        assert!(validate_conditions("kind=1&created_at<1713957000").is_ok());
        assert!(validate_conditions("kind=9&created_at>100&created_at<200").is_ok());
    }

    #[test]
    fn test_reject_trailing_ampersand() {
        let err = validate_conditions("kind=1&").unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn test_reject_leading_ampersand() {
        assert!(validate_conditions("&kind=1").is_err());
    }

    #[test]
    fn test_reject_double_ampersand() {
        assert!(validate_conditions("kind=1&&created_at<100").is_err());
    }

    #[test]
    fn test_reject_leading_zero() {
        assert!(validate_conditions("kind=01").is_err());
        assert!(validate_conditions("created_at<01713957000").is_err());
    }

    #[test]
    fn test_reject_whitespace() {
        assert!(validate_conditions("kind=1 ").is_err());
        assert!(validate_conditions(" kind=1").is_err());
        assert!(validate_conditions("kind= 1").is_err());
        assert!(validate_conditions("kind=1\t").is_err());
    }

    #[test]
    fn test_reject_unknown_clause() {
        assert!(validate_conditions("foo=1").is_err());
        assert!(validate_conditions("Kind=1").is_err()); // case-sensitive
        assert!(validate_conditions("CREATED_AT<100").is_err());
        assert!(validate_conditions("created_at=100").is_err()); // wrong operator
    }

    #[test]
    fn test_reject_out_of_range() {
        assert!(validate_conditions("kind=65536").is_err());
        assert!(validate_conditions("created_at<4294967296").is_err());
    }

    #[test]
    fn test_reject_non_decimal() {
        assert!(validate_conditions("kind=abc").is_err());
        assert!(validate_conditions("kind=-1").is_err());
        assert!(validate_conditions("kind=1.0").is_err());
    }

    #[test]
    fn test_reject_empty_value() {
        assert!(validate_conditions("kind=").is_err());
        assert!(validate_conditions("created_at<").is_err());
        assert!(validate_conditions("created_at>").is_err());
    }

    // Verify conditions validation is wired into compute and verify
    #[test]
    fn test_compute_rejects_invalid_conditions() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let err = compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=01")
            .expect_err("leading zero must be rejected");
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn test_verify_rejects_invalid_conditions() {
        let agent_pubkey = PublicKey::from_hex(AGENT_PUBKEY_HEX).unwrap();
        // Craft a tag with invalid conditions but valid structure
        let bad_conditions =
            serde_json::json!(["auth", OWNER_PUBKEY_HEX, "kind=01", "a".repeat(128)]).to_string();
        let err = verify_auth_tag(&bad_conditions, &agent_pubkey)
            .expect_err("leading zero must be rejected at verify");
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn test_parse_rejects_invalid_conditions() {
        let bad =
            serde_json::json!(["auth", OWNER_PUBKEY_HEX, "kind=1&", "a".repeat(128)]).to_string();
        assert!(parse_auth_tag(&bad).is_err());
    }
}
