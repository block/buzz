//! NIP-49 encrypted local key backup.
//!
//! Creates a password-encrypted `ncryptsec` backup of the user's identity key
//! for a user-selected local file. The blob is **local-only by contract**: it must
//! never be transmitted to a relay on any path. That contract is enforced at
//! runtime by [`crate::egress_guard`] (wired into every relay event-body
//! constructor and the native websocket send loop) and structurally by the
//! source-allowlist scan in this module's tests.
//!
//! Creation decrypt-verifies the fresh blob against the live identity before
//! returning it. Portable copies use atomic, owner-only file writes.

use nostr::nips::nip49::{EncryptedSecretKey, KeySecurity};
use nostr::{FromBech32, Keys, ToBech32};

/// Bech32 prefix of NIP-49 encrypted secret keys. Import routing is
/// case-insensitive because bech32 permits all-uppercase encodings.
pub const NCRYPTSEC_HRP: &str = "ncryptsec1";

/// scrypt cost for new backups (2^18 — Gossip's desktop default, ~256 MiB).
/// The blob self-describes its cost, so this can be raised later without
/// breaking existing backups.
pub const BACKUP_LOG_N: u8 = 18;

/// Highest scrypt cost accepted when decrypting an untrusted backup.
///
/// NIP-49 intentionally leaves `log_n` client-selected. Capping it at the tier
/// Buzz itself emits keeps generated and upstream-compatible lower-cost backups
/// readable without allowing a crafted payload to request unbounded memory
/// before password authentication.
pub const MAX_VERIFY_LOG_N: u8 = BACKUP_LOG_N;

/// Filename of the app-managed canonical backup inside the app data dir.
pub const BACKUP_FILE_NAME: &str = "identity.ncryptsec";

/// Default number of words in a generated backup passphrase. Three words
/// from a 1296-word list ≈ 31 bits of entropy before the scrypt work factor.
pub const DEFAULT_PASSPHRASE_WORDS: usize = 3;

/// Bounds for the generator's word-count control. At the lower bound a draw
/// can fall below [`MIN_PASSPHRASE_LEN`] (three 3-char words), so
/// [`generate_passphrase`] re-draws until the phrase meets the minimum.
pub const MIN_PASSPHRASE_WORDS: usize = 3;
pub const MAX_PASSPHRASE_WORDS: usize = 10;

/// EFF short wordlist 2.0 (1296 words, one per line).
const WORDLIST: &str = include_str!("assets/eff_short_wordlist_2_0.txt");

/// Minimum length for a user-chosen passphrase.
pub const MIN_PASSPHRASE_LEN: usize = 12;

/// Encrypt the identity secret key under `password` and verify the result.
///
/// Returns the bech32 `ncryptsec1…` string. The fresh blob is decrypted and
/// its derived pubkey compared to the live identity **before** returning, so
/// a returned blob is always provably recoverable with the same password.
pub fn create_backup_blob(keys: &Keys, password: &str, log_n: u8) -> Result<String, String> {
    let secret_key = keys.secret_key();

    let encrypted = EncryptedSecretKey::new(secret_key, password, log_n, KeySecurity::Unknown)
        .map_err(|e| format!("encrypt key backup: {e}"))?;

    let ncryptsec = encrypted
        .to_bech32()
        .map_err(|e| format!("encode ncryptsec: {e}"))?;

    // Integrity check: decrypt the fresh blob and confirm it recovers the
    // exact live identity. A corrupted or mis-encrypted blob must never be
    // shown to the user as a "backup". This is the second, deliberate KDF
    // invocation of the one-artifact-per-action contract.
    verify_backup_blob(&ncryptsec, password, &keys.public_key())?;

    Ok(ncryptsec)
}

/// Decrypt `ncryptsec` with `password` and assert it recovers a key whose
/// public key equals `expected_pubkey`.
pub fn verify_backup_blob(
    ncryptsec: &str,
    password: &str,
    expected_pubkey: &nostr::PublicKey,
) -> Result<(), String> {
    let encrypted = parse_ncryptsec(ncryptsec)?;
    let recovered = encrypted
        .decrypt(password)
        .map_err(|e| format!("verify key backup (decrypt): {e}"))?;
    let recovered_keys = Keys::new(recovered);
    if recovered_keys.public_key() != *expected_pubkey {
        return Err("verify key backup: decrypted key does not match identity".to_string());
    }
    Ok(())
}

/// Parse a bech32 `ncryptsec1…` string, rejecting anything that is not a
/// structurally valid NIP-49 payload.
pub fn parse_ncryptsec(input: &str) -> Result<EncryptedSecretKey, String> {
    EncryptedSecretKey::from_bech32(input.trim()).map_err(|e| format!("invalid ncryptsec: {e}"))
}

/// Decrypt an `ncryptsec1…` string with `password` into identity keys.
pub fn decrypt_ncryptsec(input: &str, password: &str) -> Result<Keys, String> {
    let encrypted = parse_ncryptsec(input)?;
    let log_n = encrypted.log_n();
    if log_n > MAX_VERIFY_LOG_N {
        return Err(format!(
            "unsupported backup KDF cost: log_n {log_n} exceeds maximum {MAX_VERIFY_LOG_N}"
        ));
    }
    let secret_key = encrypted
        .decrypt(password)
        .map_err(|_| "wrong backup password or damaged key backup".to_string())?;
    Ok(Keys::new(secret_key))
}

/// Recover identity keys from either an encrypted NIP-49 backup or the raw
/// nsec/hex formats accepted before encrypted imports were added.
pub fn recover_keys_from_input(input: &str, password: Option<&str>) -> Result<Keys, String> {
    let trimmed = input.trim();
    let is_ncryptsec = trimmed
        .get(..NCRYPTSEC_HRP.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(NCRYPTSEC_HRP));

    if is_ncryptsec {
        let password = password.ok_or_else(|| "key backup requires a password".to_string())?;
        decrypt_ncryptsec(trimmed, password)
    } else {
        Keys::parse(trimmed).map_err(|e| format!("Invalid private key: {e}"))
    }
}

/// Path of the canonical app-managed backup file.
pub fn backup_file_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(BACKUP_FILE_NAME)
}

/// Atomically write `ncryptsec` to `path` with owner-only permissions, then
/// reread and byte-compare. Same crash-safety pattern as
/// `app_state::save_key_file`.
pub fn write_backup_file(path: &std::path::Path, ncryptsec: &str) -> Result<(), String> {
    use atomic_write_file::AtomicWriteFile;
    use std::io::Write;

    let mut file = AtomicWriteFile::open(path)
        .map_err(|e| format!("open backup file for atomic write: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set backup file permissions: {e}"))?;
    }

    file.write_all(ncryptsec.as_bytes())
        .map_err(|e| format!("write backup file: {e}"))?;
    file.commit()
        .map_err(|e| format!("commit backup file: {e}"))?;

    // Reread and byte-compare: only report success for bytes that are
    // actually on disk.
    let on_disk = std::fs::read_to_string(path).map_err(|e| format!("reread backup file: {e}"))?;
    if on_disk != ncryptsec {
        return Err("backup file verification failed: on-disk bytes differ".to_string());
    }

    Ok(())
}

/// Delete the app-managed backup if present. Missing files are already clean.
pub fn delete_backup_file(data_dir: &std::path::Path) -> Result<(), String> {
    let path = backup_file_path(data_dir);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("delete stale backup file: {e}")),
    }
}

/// Remove the app-managed backup only when an import changes identities.
pub fn cleanup_stale_backup(
    previous: &nostr::PublicKey,
    new: &nostr::PublicKey,
    data_dir: &std::path::Path,
) -> Result<(), String> {
    if previous != new {
        delete_backup_file(data_dir)?;
    }
    Ok(())
}

/// Generate a passphrase of `word_count` EFF short-wordlist words joined by
/// `separator`, using OS entropy.
///
/// `word_count` is clamped to `MIN_PASSPHRASE_WORDS..=MAX_PASSPHRASE_WORDS`.
/// Because a low-word-count draw can land under [`MIN_PASSPHRASE_LEN`]
/// (e.g. three 3-char words), whole phrases below the minimum are rejected
/// and re-drawn — the result always passes the same length gate applied to
/// user-chosen passphrases. Uses rejection sampling for a uniform
/// distribution over the 1296 words.
pub fn generate_passphrase(word_count: usize, separator: &str) -> Result<String, String> {
    let word_count = word_count.clamp(MIN_PASSPHRASE_WORDS, MAX_PASSPHRASE_WORDS);
    let words: Vec<&str> = WORDLIST.lines().filter(|l| !l.is_empty()).collect();
    if words.len() != 1296 {
        return Err(format!(
            "wordlist corrupted: expected 1296 words, found {}",
            words.len()
        ));
    }

    // At 3 words the under-length probability per draw is small, so a few
    // attempts always suffice; the cap only guards against a logic bug
    // becoming an infinite loop.
    for _ in 0..128 {
        let mut chosen: Vec<&str> = Vec::with_capacity(word_count);
        while chosen.len() < word_count {
            let mut buf = [0u8; 2];
            getrandom::getrandom(&mut buf).map_err(|e| format!("entropy source: {e}"))?;
            let value = u16::from_le_bytes(buf);
            // Rejection sampling: accept only values below the largest
            // multiple of 1296 that fits in u16 (65536 - 65536 % 1296 = 64800).
            if value < 64800 {
                chosen.push(words[(value as usize) % 1296]);
            }
        }
        let phrase = chosen.join(separator);
        if phrase.chars().count() >= MIN_PASSPHRASE_LEN {
            return Ok(phrase);
        }
    }
    Err("could not generate a passphrase meeting the minimum length".to_string())
}

#[cfg(test)]
#[path = "key_backup_tests.rs"]
mod tests;
