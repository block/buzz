use nostr::{Keys, PublicKey};

pub(crate) fn identity_npub_for_log(pubkey: &PublicKey) -> String {
    buzz_core_pkg::nostr_identity::public_key_to_npub(pubkey)
        .unwrap_or_else(|error| format!("<npub unavailable: {error}>"))
}

/// Render an internal hex or canonical npub identity for diagnostics and UI errors.
/// Invalid values become a non-key sentinel instead of leaking the input.
pub(crate) fn identity_npub_for_log_str(pubkey: &str) -> String {
    buzz_core_pkg::nostr_identity::parse_public_key_compat(pubkey)
        .map(|(pubkey, _)| identity_npub_for_log(&pubkey))
        .unwrap_or_else(|_| "<invalid npub>".to_string())
}

/// Read the explicit environment identity, accepting secret hex only for migration.
pub(crate) fn identity_from_env() -> Option<Keys> {
    match std::env::var("BUZZ_PRIVATE_KEY") {
        Ok(nsec) => match buzz_core_pkg::nostr_identity::parse_secret_key_compat(nsec.trim()) {
            Ok((secret_key, encoding)) => {
                if encoding == buzz_core_pkg::nostr_identity::KeyInputEncoding::LegacyHex {
                    eprintln!(
                        "buzz-desktop: BUZZ_PRIVATE_KEY uses legacy secret hex; store the canonical nsec form"
                    );
                }
                Some(Keys::new(secret_key))
            }
            Err(_) => {
                eprintln!("buzz-desktop: invalid BUZZ_PRIVATE_KEY; expected a canonical nsec");
                None
            }
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("buzz-desktop: BUZZ_PRIVATE_KEY contains invalid UTF-8");
            None
        }
        Err(std::env::VarError::NotPresent) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_log_display_uses_npub() {
        let pubkey = Keys::generate().public_key();
        let display = identity_npub_for_log(&pubkey);
        assert!(display.starts_with("npub1"));
        assert_ne!(display, pubkey.to_hex());
    }

    #[test]
    fn identity_string_display_normalizes_hex_and_redacts_invalid_values() {
        let pubkey = Keys::generate().public_key();
        let expected = identity_npub_for_log(&pubkey);
        assert_eq!(identity_npub_for_log_str(&pubkey.to_hex()), expected);
        assert_eq!(identity_npub_for_log_str(&expected), expected);
        assert_eq!(
            identity_npub_for_log_str("not-a-public-key"),
            "<invalid npub>"
        );
    }
}
