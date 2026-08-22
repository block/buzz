//! Digest tests.

use super::*;

#[test]
fn identical_bytes_hash_identically_and_any_change_does_not() {
    let payload = br#"{"type":"broker_request","requestId":"req-1"}"#;
    assert_eq!(
        request_digest(payload).unwrap(),
        request_digest(payload).unwrap()
    );

    // Byte-level difference, including key order, is a different request.
    let reordered = br#"{"requestId":"req-1","type":"broker_request"}"#;
    assert_ne!(
        request_digest(payload).unwrap(),
        request_digest(reordered).unwrap()
    );
}

#[test]
fn digest_is_hex_sha256_of_the_exact_bytes() {
    // Pinned against an independent SHA-256 of the empty input, so a change to
    // the hashing scheme fails loudly rather than silently invalidating every
    // recorded digest.
    assert_eq!(
        request_digest(b"").unwrap(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        request_digest(b"abc").unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn an_oversized_payload_is_refused_before_hashing() {
    let too_big = vec![b'x'; MAX_REQUEST_BYTES + 1];
    let error = request_digest(&too_big).unwrap_err();
    assert!(error.contains("exceeds"), "unexpected: {error}");

    assert!(request_digest(&vec![b'x'; MAX_REQUEST_BYTES]).is_ok());
}
