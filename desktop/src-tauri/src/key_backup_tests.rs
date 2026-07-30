use super::*;

/// NIP-49 spec vector (same as rust-nostr's upstream test): decrypts with
/// password "nostr" at our call sites.
const SPEC_NCRYPTSEC: &str = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";
const SPEC_SECRET_HEX: &str = "3501454135014541350145413501453fefb02227e449e57cf4d3a3ce05378683";

/// Fast scrypt tier for tests. log_n 18 is exercised once in
/// `round_trip_at_production_cost`.
const FAST_LOG_N: u8 = 16;

// ── Codec ─────────────────────────────────────────────────────────────────────

#[test]
fn spec_vector_decrypts_at_our_call_site() {
    let keys = decrypt_ncryptsec(SPEC_NCRYPTSEC, "nostr").unwrap();
    assert_eq!(keys.secret_key().to_secret_hex(), SPEC_SECRET_HEX);
}

#[test]
fn round_trip_fast_tier() {
    let keys = Keys::generate();
    let blob = create_backup_blob(&keys, "correct horse battery", FAST_LOG_N).unwrap();
    assert!(blob.starts_with("ncryptsec1"));
    let recovered = decrypt_ncryptsec(&blob, "correct horse battery").unwrap();
    assert_eq!(recovered.public_key(), keys.public_key());
}

#[test]
fn round_trip_at_production_cost() {
    // One log_n 18 round trip: proves the production constant works end to
    // end (slow — several seconds — but deliberate; see plan D5).
    let keys = Keys::generate();
    let blob = create_backup_blob(&keys, "production cost tier check", BACKUP_LOG_N).unwrap();
    let recovered = decrypt_ncryptsec(&blob, "production cost tier check").unwrap();
    assert_eq!(recovered.public_key(), keys.public_key());
}

#[test]
fn wrong_password_is_a_friendly_error() {
    let keys = Keys::generate();
    let blob = create_backup_blob(&keys, "right password", FAST_LOG_N).unwrap();
    let err = decrypt_ncryptsec(&blob, "wrong password").unwrap_err();
    assert_eq!(err, "wrong backup password or damaged key backup");
}

#[test]
fn nfkc_cross_form_passphrase_round_trips() {
    // "é" composed (U+00E9) vs decomposed (e + U+0301): NIP-49 mandates NFKC
    // normalization, so a passphrase entered in either form must decrypt.
    let keys = Keys::generate();
    let composed = "caf\u{00e9} passphrase";
    let decomposed = "cafe\u{0301} passphrase";
    assert_ne!(composed, decomposed);
    let blob = create_backup_blob(&keys, composed, FAST_LOG_N).unwrap();
    let recovered = decrypt_ncryptsec(&blob, decomposed).unwrap();
    assert_eq!(recovered.public_key(), keys.public_key());
}

#[test]
fn parse_rejects_garbage_and_wrong_hrp() {
    assert!(parse_ncryptsec("garbage").is_err());
    assert!(parse_ncryptsec("").is_err());
    // Valid bech32, wrong HRP (an nsec is not an encrypted backup).
    let nsec = Keys::generate().secret_key().to_bech32().unwrap();
    assert!(parse_ncryptsec(&nsec).is_err());
    // Truncated blob.
    assert!(parse_ncryptsec(&SPEC_NCRYPTSEC[..SPEC_NCRYPTSEC.len() - 10]).is_err());
}

#[test]
fn verify_backup_blob_catches_pubkey_mismatch() {
    // Corrupted-blob simulation: the blob decrypts fine but recovers a key
    // that is not the live identity — verification must fail.
    let other = Keys::generate();
    let blob = create_backup_blob(&other, "some password", FAST_LOG_N).unwrap();
    let live = Keys::generate();
    let err = verify_backup_blob(&blob, "some password", &live.public_key()).unwrap_err();
    assert!(err.contains("does not match identity"), "{err}");
}

// ── Import key recovery ───────────────────────────────────────────────────────

#[test]
fn recover_keys_ncryptsec_happy_path() {
    let keys = recover_keys_from_input(&format!("  {SPEC_NCRYPTSEC}\n"), Some("nostr")).unwrap();
    assert_eq!(keys.secret_key().to_secret_hex(), SPEC_SECRET_HEX);
}

#[test]
fn recover_keys_ncryptsec_requires_password() {
    let err = recover_keys_from_input(SPEC_NCRYPTSEC, None).unwrap_err();
    assert_eq!(err, "key backup requires a password");
}

#[test]
fn recover_keys_ncryptsec_wrong_password() {
    let err = recover_keys_from_input(SPEC_NCRYPTSEC, Some("wrong")).unwrap_err();
    assert_eq!(err, "wrong backup password or damaged key backup");
}

#[test]
fn recover_keys_uppercase_ncryptsec_classifies_as_encrypted() {
    let upper = SPEC_NCRYPTSEC.to_ascii_uppercase();
    assert_eq!(
        recover_keys_from_input(&upper, None).unwrap_err(),
        "key backup requires a password"
    );
    let keys = recover_keys_from_input(&upper, Some("nostr")).unwrap();
    assert_eq!(keys.secret_key().to_secret_hex(), SPEC_SECRET_HEX);

    let mut mixed = SPEC_NCRYPTSEC.to_string();
    mixed.replace_range(0..1, "N");
    let err = recover_keys_from_input(&mixed, Some("nostr")).unwrap_err();
    assert!(err.contains("invalid ncryptsec"), "{err}");
}

#[test]
fn recover_keys_raw_nsec_path_unchanged() {
    let keys = Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let recovered = recover_keys_from_input(&nsec, Some("ignored")).unwrap();
    assert_eq!(recovered.public_key(), keys.public_key());
    let recovered = recover_keys_from_input(&nsec, None).unwrap();
    assert_eq!(recovered.public_key(), keys.public_key());
    assert!(recover_keys_from_input("garbage", None).is_err());
}

// ── File lifecycle ────────────────────────────────────────────────────────────

#[test]
fn write_backup_file_persists_0600_and_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let path = backup_file_path(dir.path());
    write_backup_file(&path, SPEC_NCRYPTSEC).unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, SPEC_NCRYPTSEC);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "backup file must be owner-only");
    }
}

#[test]
fn write_backup_file_overwrites_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = backup_file_path(dir.path());
    write_backup_file(&path, "ncryptsec1old").unwrap();
    write_backup_file(&path, SPEC_NCRYPTSEC).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_NCRYPTSEC);
    // No leftover temp files from the atomic write.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries, vec![std::ffi::OsString::from(BACKUP_FILE_NAME)]);
}

#[test]
fn delete_backup_file_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    delete_backup_file(dir.path()).unwrap();
    let path = backup_file_path(dir.path());
    write_backup_file(&path, SPEC_NCRYPTSEC).unwrap();
    delete_backup_file(dir.path()).unwrap();
    assert!(!path.exists());
}

#[test]
fn cleanup_stale_backup_removes_only_on_identity_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = backup_file_path(dir.path());
    let a = Keys::generate().public_key();
    let b = Keys::generate().public_key();

    write_backup_file(&path, SPEC_NCRYPTSEC).unwrap();
    cleanup_stale_backup(&a, &a, dir.path()).unwrap();
    assert!(path.exists(), "same identity must keep the backup");

    cleanup_stale_backup(&a, &b, dir.path()).unwrap();
    assert!(
        !path.exists(),
        "identity change must remove the stale backup"
    );
}

#[test]
fn generated_passphrase_respects_word_count_and_separator() {
    let words: std::collections::HashSet<&str> =
        WORDLIST.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(words.len(), 1296, "EFF short wordlist 2.0 has 1296 words");

    for (count, separator) in [(3, "-"), (4, "-"), (6, " "), (5, "."), (10, "")] {
        let phrase = generate_passphrase(count, separator).unwrap();
        if separator.is_empty() {
            // No separator to split on; length gate below still applies.
        } else {
            let parts: Vec<&str> = phrase.split(separator).collect();
            assert_eq!(parts.len(), count);
            for w in &parts {
                assert!(words.contains(w), "unknown word {w:?}");
            }
        }
        assert!(phrase.chars().count() >= MIN_PASSPHRASE_LEN);
    }
}

#[test]
fn generated_passphrase_clamps_word_count() {
    // Below the floor: clamped up to MIN_PASSPHRASE_WORDS, never shorter.
    let phrase = generate_passphrase(1, "-").unwrap();
    assert_eq!(phrase.split('-').count(), MIN_PASSPHRASE_WORDS);
    // Above the ceiling: clamped down to MAX_PASSPHRASE_WORDS.
    let phrase = generate_passphrase(50, "-").unwrap();
    assert_eq!(phrase.split('-').count(), MAX_PASSPHRASE_WORDS);
}

#[test]
fn generated_passphrases_are_not_repeated() {
    // 3 words × ~10.3 bits each — a collision across 8 draws would indicate a
    // broken entropy source, not bad luck.
    let mut seen = std::collections::HashSet::new();
    for _ in 0..8 {
        assert!(seen.insert(generate_passphrase(DEFAULT_PASSPHRASE_WORDS, "-").unwrap()));
    }
}
