//! NIP-AB HKDF-SHA256 key derivation primitives.
//!
//! All functions are pure (no I/O, no side effects) and operate on fixed-size
//! `[u8; 32]` arrays. The underlying HKDF implementation is
//! [`nostr::util::hkdf`], which uses `bitcoin::hashes` internally.
//!
//! # Derivation overview
//!
//! ```text
//! session_secret (32 bytes, random)
//!        │
//!        ├─► derive_session_id  → session_id   (HKDF, salt=[], info="nostr-pair-session-id")
//!        │
//!        ├─► derive_sas(ecdh_shared, …)
//!        │       ├─ sas_input   (HKDF, salt=session_secret, info="nostr-pair-sas-v1")
//!        │       └─ sas_code    = be_u32(sas_input[0..4]) % 1_000_000
//!        │
//!        └─► derive_transcript_hash(session_id, src_pk, tgt_pk, sas_input, …)
//!                └─ transcript_hash (HKDF, salt=session_secret,
//!                                    info="nostr-pair-transcript-v1")
//! ```

use nostr::hashes::Hash as _;
use nostr::util::hkdf;

// ── HKDF info strings ────────────────────────────────────────────────────────

const INFO_SESSION_ID: &[u8] = b"nostr-pair-session-id";
const INFO_SAS: &[u8] = b"nostr-pair-sas-v1";
const INFO_TRANSCRIPT: &[u8] = b"nostr-pair-transcript-v1";

// ── Internal helper ───────────────────────────────────────────────────────────

/// Run HKDF-SHA256(IKM=`ikm`, salt=`salt`, info=`info`) and return 32 bytes.
///
/// Uses `nostr::util::hkdf::{extract, expand}` directly so we don't pull in
/// an extra `hkdf` crate dependency.
fn hkdf32(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let prk = hkdf::extract(salt, ikm);
    let okm = hkdf::expand(&prk.to_byte_array(), info, 32);
    // HKDF-Expand with L=32 and SHA-256 (HashLen=32) always produces exactly
    // 32 bytes (one iteration, truncated to L). Copy into a fixed-size array
    // without expect/unwrap.
    let mut out = [0u8; 32];
    out.copy_from_slice(&okm[..32]);
    out
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Derive the session ID from the session secret.
///
/// ```text
/// session_id = HKDF-SHA256(IKM=session_secret, salt=[], info="nostr-pair-session-id", L=32)
/// ```
///
/// The session ID is safe to share publicly (e.g., in the QR code or as a
/// Nostr event tag). It uniquely identifies the pairing session without
/// revealing the secret.
pub fn derive_session_id(session_secret: &[u8; 32]) -> [u8; 32] {
    hkdf32(b"", session_secret, INFO_SESSION_ID)
}

/// Derive the Short Authentication String (SAS) code and the raw SAS input.
///
/// ```text
/// sas_input = HKDF-SHA256(IKM=ecdh_shared, salt=session_secret, info="nostr-pair-sas-v1", L=32)
/// sas_code  = be_u32(sas_input[0..4]) mod 1_000_000
/// ```
///
/// Returns `(sas_code, sas_input)`. The caller needs `sas_input` to compute
/// the transcript hash — see [`derive_transcript_hash`].
///
/// `ecdh_shared` is the raw 32-byte x-coordinate from
/// `nostr::util::generate_shared_key(own_secret, other_pubkey)`.
pub fn derive_sas(ecdh_shared: &[u8; 32], session_secret: &[u8; 32]) -> (u32, [u8; 32]) {
    let sas_input = hkdf32(session_secret, ecdh_shared, INFO_SAS);
    let sas_code =
        u32::from_be_bytes([sas_input[0], sas_input[1], sas_input[2], sas_input[3]]) % 1_000_000;
    (sas_code, sas_input)
}

/// Derive the transcript hash that binds all session parameters together.
///
/// ```text
/// transcript     = session_id ‖ source_pubkey ‖ target_pubkey ‖ sas_input  (128 bytes)
/// transcript_hash = HKDF-SHA256(IKM=transcript, salt=session_secret,
///                               info="nostr-pair-transcript-v1", L=32)
/// ```
///
/// Both parties must independently compute this value and compare it before
/// exchanging the actual payload. A mismatch means the session is compromised.
///
/// `sas_input` is the second return value of [`derive_sas`].
pub fn derive_transcript_hash(
    session_id: &[u8; 32],
    source_pubkey: &[u8; 32],
    target_pubkey: &[u8; 32],
    sas_input: &[u8; 32],
    session_secret: &[u8; 32],
) -> [u8; 32] {
    // Concatenate into a 128-byte transcript.
    let mut transcript = [0u8; 128];
    transcript[0..32].copy_from_slice(session_id);
    transcript[32..64].copy_from_slice(source_pubkey);
    transcript[64..96].copy_from_slice(target_pubkey);
    transcript[96..128].copy_from_slice(sas_input);

    hkdf32(session_secret, &transcript, INFO_TRANSCRIPT)
}

/// Format a SAS code as a zero-padded 6-digit string.
///
/// # Examples
/// ```
/// use sprout_core::pairing::crypto::format_sas;
/// assert_eq!(format_sas(291),    "000291");
/// assert_eq!(format_sas(47291),  "047291");
/// assert_eq!(format_sas(999999), "999999");
/// assert_eq!(format_sas(0),      "000000");
/// ```
pub fn format_sas(code: u32) -> String {
    format!("{code:06}")
}

/// Constant-time comparison of two 32-byte arrays.
///
/// Returns `true` iff all bytes are equal. Uses [`subtle::ConstantTimeEq`]
/// to guarantee the comparison is not optimized into a short-circuit by the
/// compiler, preventing timing side-channels on secret-derived values like
/// transcript hashes and session IDs.
pub fn ct_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test vector inputs (from NIP-AB spec) ─────────────────────────────────

    /// session_secret = 0xa1b2c3d4…
    fn session_secret() -> [u8; 32] {
        hex_to_32("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2")
    }

    /// source ephemeral private key bytes (used to derive pubkey for transcript test)
    fn source_privkey_bytes() -> [u8; 32] {
        hex_to_32("7f4c11a9c9d1e3b5a7f2e4d6c8b0a2f4e6d8c0b2a4f6e8d0c2b4a6f8e0d2c4b5")
    }

    /// target ephemeral private key bytes
    fn target_privkey_bytes() -> [u8; 32] {
        hex_to_32("3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7d9e1f3a5b")
    }

    fn hex_to_32(s: &str) -> [u8; 32] {
        let bytes = hex::decode(s).expect("valid hex");
        bytes.try_into().expect("32 bytes")
    }

    fn bytes_to_hex(b: &[u8]) -> String {
        hex::encode(b)
    }

    // ── session_id derivation ─────────────────────────────────────────────────

    #[test]
    fn session_id_is_deterministic() {
        let secret = session_secret();
        let id1 = derive_session_id(&secret);
        let id2 = derive_session_id(&secret);
        assert_eq!(id1, id2, "session_id must be deterministic");
    }

    #[test]
    fn session_id_is_32_bytes() {
        let id = derive_session_id(&session_secret());
        assert_eq!(id.len(), 32);
    }

    #[test]
    fn session_id_differs_from_secret() {
        let secret = session_secret();
        let id = derive_session_id(&secret);
        assert_ne!(id, secret, "session_id must not equal the raw secret");
    }

    #[test]
    fn session_id_test_vector() {
        let id = derive_session_id(&session_secret());
        assert_eq!(
            bytes_to_hex(&id),
            "fb357d0f8e8d5a5ba3b2a91cb18c119e1567b07ffa38cdebb73e68df78f5a380",
            "session_id must match NIP-AB spec test vector"
        );
    }

    // ── SAS derivation ────────────────────────────────────────────────────────

    #[test]
    fn sas_code_is_six_digits() {
        // Use a synthetic ECDH shared secret (just some fixed bytes).
        let ecdh = hex_to_32("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20");
        let (code, _) = derive_sas(&ecdh, &session_secret());
        assert!(code < 1_000_000, "SAS code must be < 1_000_000, got {code}");
    }

    #[test]
    fn sas_is_deterministic() {
        let ecdh = hex_to_32("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20");
        let (code1, input1) = derive_sas(&ecdh, &session_secret());
        let (code2, input2) = derive_sas(&ecdh, &session_secret());
        assert_eq!(code1, code2);
        assert_eq!(input1, input2);
    }

    #[test]
    fn sas_changes_with_different_ecdh() {
        let ecdh1 = hex_to_32("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20");
        let ecdh2 = hex_to_32("ff02030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20");
        let (code1, _) = derive_sas(&ecdh1, &session_secret());
        let (code2, _) = derive_sas(&ecdh2, &session_secret());
        assert_ne!(
            code1, code2,
            "different ECDH inputs must produce different SAS codes"
        );
    }

    #[test]
    fn sas_with_real_ecdh_keys() {
        use nostr::{Keys, SecretKey};

        let src_sk = SecretKey::from_slice(&source_privkey_bytes()).expect("valid key");
        let tgt_sk = SecretKey::from_slice(&target_privkey_bytes()).expect("valid key");
        let src_keys = Keys::new(src_sk);
        let tgt_keys = Keys::new(tgt_sk);

        // ECDH: source computes shared key with target's pubkey
        let ecdh_from_src =
            nostr::util::generate_shared_key(src_keys.secret_key(), &tgt_keys.public_key())
                .unwrap();
        // ECDH: target computes shared key with source's pubkey (must match)
        let ecdh_from_tgt =
            nostr::util::generate_shared_key(tgt_keys.secret_key(), &src_keys.public_key())
                .unwrap();

        assert_eq!(ecdh_from_src, ecdh_from_tgt, "ECDH must be symmetric");

        let (code, sas_input) = derive_sas(&ecdh_from_src, &session_secret());
        println!("sas_code  = {}", format_sas(code));
        println!("sas_input = {}", bytes_to_hex(&sas_input));

        assert!(code < 1_000_000);
    }

    // ── transcript_hash derivation ────────────────────────────────────────────

    #[test]
    fn transcript_hash_is_deterministic() {
        use nostr::{Keys, SecretKey};

        let src_sk = SecretKey::from_slice(&source_privkey_bytes()).expect("valid key");
        let tgt_sk = SecretKey::from_slice(&target_privkey_bytes()).expect("valid key");
        let src_keys = Keys::new(src_sk);
        let tgt_keys = Keys::new(tgt_sk);

        let session_id = derive_session_id(&session_secret());
        let ecdh = nostr::util::generate_shared_key(src_keys.secret_key(), &tgt_keys.public_key())
            .unwrap();
        let (_, sas_input) = derive_sas(&ecdh, &session_secret());

        let src_pk: [u8; 32] = src_keys.public_key().to_bytes();
        let tgt_pk: [u8; 32] = tgt_keys.public_key().to_bytes();

        let h1 =
            derive_transcript_hash(&session_id, &src_pk, &tgt_pk, &sas_input, &session_secret());
        let h2 =
            derive_transcript_hash(&session_id, &src_pk, &tgt_pk, &sas_input, &session_secret());
        assert_eq!(h1, h2);
    }

    /// Full test vector suite — all values pinned against the NIP-AB spec.
    #[test]
    fn all_test_vectors() {
        use nostr::{Keys, SecretKey};

        let src_sk = SecretKey::from_slice(&source_privkey_bytes()).expect("valid key");
        let tgt_sk = SecretKey::from_slice(&target_privkey_bytes()).expect("valid key");
        let src_keys = Keys::new(src_sk);
        let tgt_keys = Keys::new(tgt_sk);

        // Pubkeys
        assert_eq!(
            bytes_to_hex(&src_keys.public_key().to_bytes()),
            "199e64ca60662cb2d6e91d16cb065be51ad74a6ee5f8c5b0fdc53d246611ed9a"
        );
        assert_eq!(
            bytes_to_hex(&tgt_keys.public_key().to_bytes()),
            "89a9fa762105d0aee2b19678246fe7b823aabbc4f4bf691a1ce8a70fcd36d6e4"
        );

        // ECDH
        let ecdh = nostr::util::generate_shared_key(src_keys.secret_key(), &tgt_keys.public_key())
            .unwrap();
        assert_eq!(
            bytes_to_hex(&ecdh),
            "9b4b6d6990713d89d6d9982e506ee1bbcde6f05c54d9d2978696e8a7274d4408"
        );

        // Session ID
        let session_id = derive_session_id(&session_secret());
        assert_eq!(
            bytes_to_hex(&session_id),
            "fb357d0f8e8d5a5ba3b2a91cb18c119e1567b07ffa38cdebb73e68df78f5a380"
        );

        // SAS
        let (sas_code, sas_input) = derive_sas(&ecdh, &session_secret());
        assert_eq!(
            bytes_to_hex(&sas_input),
            "e8b03a329f3a0ac37fe7fbe929171e14b72812be67e33c5d6e193543c41798d3"
        );
        assert_eq!(format_sas(sas_code), "863346");

        // Transcript hash
        let src_pk = src_keys.public_key().to_bytes();
        let tgt_pk = tgt_keys.public_key().to_bytes();
        let transcript_hash =
            derive_transcript_hash(&session_id, &src_pk, &tgt_pk, &sas_input, &session_secret());
        assert_eq!(
            bytes_to_hex(&transcript_hash),
            "d662818ff8911fc60a2d025f8b8b4756107104e85888dd202d28db5ca2cf28d3"
        );
    }

    #[test]
    fn transcript_hash_sensitive_to_pubkey_order() {
        use nostr::{Keys, SecretKey};

        let src_sk = SecretKey::from_slice(&source_privkey_bytes()).expect("valid key");
        let tgt_sk = SecretKey::from_slice(&target_privkey_bytes()).expect("valid key");
        let src_keys = Keys::new(src_sk);
        let tgt_keys = Keys::new(tgt_sk);

        let session_id = derive_session_id(&session_secret());
        let ecdh = nostr::util::generate_shared_key(src_keys.secret_key(), &tgt_keys.public_key())
            .unwrap();
        let (_, sas_input) = derive_sas(&ecdh, &session_secret());

        let src_pk: [u8; 32] = src_keys.public_key().to_bytes();
        let tgt_pk: [u8; 32] = tgt_keys.public_key().to_bytes();

        let h_correct =
            derive_transcript_hash(&session_id, &src_pk, &tgt_pk, &sas_input, &session_secret());
        // Swap source and target — must produce a different hash.
        let h_swapped =
            derive_transcript_hash(&session_id, &tgt_pk, &src_pk, &sas_input, &session_secret());
        assert_ne!(
            h_correct, h_swapped,
            "transcript_hash must be sensitive to pubkey order"
        );
    }

    // ── format_sas ────────────────────────────────────────────────────────────

    #[test]
    fn format_sas_zero_padding() {
        assert_eq!(format_sas(0), "000000");
        assert_eq!(format_sas(1), "000001");
        assert_eq!(format_sas(291), "000291");
        assert_eq!(format_sas(47291), "047291");
        assert_eq!(format_sas(999999), "999999");
    }

    #[test]
    fn format_sas_always_six_chars() {
        for code in [0u32, 1, 99, 1000, 99999, 100000, 999999] {
            let s = format_sas(code);
            assert_eq!(s.len(), 6, "format_sas({code}) = {s:?} (expected 6 chars)");
            assert!(s.chars().all(|c| c.is_ascii_digit()), "all digits: {s}");
        }
    }

    // ── Full round-trip consistency ───────────────────────────────────────────

    #[test]
    fn full_derivation_round_trip() {
        use nostr::{Keys, SecretKey};

        // Simulate both sides of the pairing independently deriving the same values.
        let src_sk = SecretKey::from_slice(&source_privkey_bytes()).expect("valid key");
        let tgt_sk = SecretKey::from_slice(&target_privkey_bytes()).expect("valid key");
        let src_keys = Keys::new(src_sk);
        let tgt_keys = Keys::new(tgt_sk);
        let secret = session_secret();

        // Both sides derive the same session_id.
        let session_id = derive_session_id(&secret);

        // Both sides compute ECDH (symmetric).
        let ecdh_src =
            nostr::util::generate_shared_key(src_keys.secret_key(), &tgt_keys.public_key())
                .unwrap();
        let ecdh_tgt =
            nostr::util::generate_shared_key(tgt_keys.secret_key(), &src_keys.public_key())
                .unwrap();
        assert_eq!(ecdh_src, ecdh_tgt, "ECDH must be symmetric");

        // Both sides derive the same SAS.
        let (code_src, sas_input_src) = derive_sas(&ecdh_src, &secret);
        let (code_tgt, sas_input_tgt) = derive_sas(&ecdh_tgt, &secret);
        assert_eq!(code_src, code_tgt, "SAS codes must match");
        assert_eq!(sas_input_src, sas_input_tgt, "sas_input must match");

        // Both sides derive the same transcript hash (using the agreed pubkey ordering).
        let src_pk: [u8; 32] = src_keys.public_key().to_bytes();
        let tgt_pk: [u8; 32] = tgt_keys.public_key().to_bytes();

        let th_src = derive_transcript_hash(&session_id, &src_pk, &tgt_pk, &sas_input_src, &secret);
        let th_tgt = derive_transcript_hash(&session_id, &src_pk, &tgt_pk, &sas_input_tgt, &secret);
        assert_eq!(th_src, th_tgt, "transcript hashes must match");

        println!(
            "✅ Round-trip OK: sas={} transcript={}",
            format_sas(code_src),
            bytes_to_hex(&th_src)
        );
    }
}
