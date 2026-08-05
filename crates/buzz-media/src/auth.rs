//! Blossom kind:24242 auth verification (BUD-11 compliant).

use crate::error::MediaError;
pub use buzz_auth::blossom::BlossomVerb;

fn map_auth_error(error: buzz_auth::blossom::BlossomAuthError) -> MediaError {
    use buzz_auth::blossom::BlossomAuthError;

    match error {
        BlossomAuthError::InvalidSignature => MediaError::InvalidSignature,
        BlossomAuthError::InvalidAuthKind => MediaError::InvalidAuthKind,
        BlossomAuthError::InvalidAuthEvent => MediaError::InvalidAuthEvent,
        BlossomAuthError::InvalidAuthVerb => MediaError::InvalidAuthVerb,
        BlossomAuthError::MissingTag(tag) => MediaError::MissingTag(tag),
        BlossomAuthError::TokenExpired => MediaError::TokenExpired,
        BlossomAuthError::TimestampOutOfWindow => MediaError::TimestampOutOfWindow,
        BlossomAuthError::ServerMismatch => MediaError::ServerMismatch,
        BlossomAuthError::HashMismatch => MediaError::HashMismatch,
        BlossomAuthError::InsufficientScope => MediaError::InsufficientScope,
    }
}

/// Verify common kind:24242 Blossom auth event validity for one exact verb.
pub fn verify_blossom_auth_event_for_verb(
    auth_event: &nostr::Event,
    verb: BlossomVerb,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), MediaError> {
    buzz_auth::blossom::verify_blossom_auth_event_for_verb(
        auth_event,
        verb,
        server_domain,
        max_age_secs,
    )
    .map_err(map_auth_error)
}

/// Verify common upload auth event validity without checking the blob hash.
pub fn verify_blossom_auth_event(
    auth_event: &nostr::Event,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), MediaError> {
    buzz_auth::blossom::verify_blossom_auth_event(auth_event, server_domain, max_age_secs)
        .map_err(map_auth_error)
}

/// Verify a kind:24242 upload event including the exact `x` tag blob hash.
pub fn verify_blossom_upload_auth(
    auth_event: &nostr::Event,
    sha256: &str,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), MediaError> {
    buzz_auth::blossom::verify_blossom_upload_auth(auth_event, sha256, server_domain, max_age_secs)
        .map_err(map_auth_error)
}

/// Verify a kind:24242 download event for one exact blob and server.
pub fn verify_blossom_get_auth(
    auth_event: &nostr::Event,
    sha256: &str,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), MediaError> {
    buzz_auth::blossom::verify_blossom_get_auth(auth_event, sha256, server_domain, max_age_secs)
        .map_err(map_auth_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn build_valid_auth(keys: &Keys, sha256: &str) -> nostr::Event {
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn test_verify_valid() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        assert!(verify_blossom_upload_auth(&event, &sha256, None, 600).is_ok());
    }

    #[test]
    fn test_verify_auth_event_valid() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        assert!(verify_blossom_auth_event(&event, None, 600).is_ok());
    }

    fn build_get_auth(keys: &Keys, tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::from(24242), "Get buzz-media")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn test_verify_get_accepts_matching_x_without_server_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["x", &sha256]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );

        assert!(verify_blossom_get_auth(&event, &sha256, Some("relay.example"), 600).is_ok());
    }

    #[test]
    fn test_verify_get_accepts_matching_server_without_x_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["server", "https://Relay.Example./media/ignored"]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );

        assert!(verify_blossom_get_auth(&event, &sha256, Some("relay.example"), 600).is_ok());
    }

    #[test]
    fn test_verify_get_rejects_upload_verb() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);

        assert!(matches!(
            verify_blossom_get_auth(&event, &sha256, Some("relay.example"), 600),
            Err(MediaError::InvalidAuthVerb)
        ));
    }

    #[test]
    fn test_verify_get_requires_x_or_server_scope() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let other_hash = "b".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["x", &other_hash]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );

        assert!(matches!(
            verify_blossom_get_auth(&event, &sha256, Some("relay.example"), 600),
            Err(MediaError::InsufficientScope)
        ));
    }

    #[test]
    fn test_verify_get_rejects_wrong_server_scope() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["server", "other.example"]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );

        assert!(matches!(
            verify_blossom_get_auth(&event, &sha256, Some("relay.example"), 600),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_verify_hash_mismatch() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        let wrong_hash = "b".repeat(64);
        assert!(matches!(
            verify_blossom_upload_auth(&event, &wrong_hash, None, 600),
            Err(MediaError::HashMismatch)
        ));
    }

    #[test]
    fn test_verify_wrong_kind() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(27235), "wrong kind")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_upload_auth(&event, &sha256, None, 600),
            Err(MediaError::InvalidAuthKind)
        ));
    }

    #[test]
    fn test_verify_multi_x_tags() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let other_hash = "b".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &other_hash]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload multi-x")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        // Should pass because at least one x tag matches
        assert!(verify_blossom_upload_auth(&event, &sha256, None, 600).is_ok());
    }

    #[test]
    fn test_server_tag_enforcement() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "other.example.com"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload scoped")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        // Should fail — server tag present but doesn't match our domain
        assert!(matches!(
            verify_blossom_upload_auth(&event, &sha256, Some("buzz.example.com"), 600),
            Err(MediaError::ServerMismatch)
        ));
        // Should pass when our domain matches
        assert!(
            verify_blossom_upload_auth(&event, &sha256, Some("other.example.com"), 600).is_ok()
        );
        // Should fail when server_domain is None — fail closed
        assert!(matches!(
            verify_blossom_upload_auth(&event, &sha256, None, 600),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_no_server_tags_always_passes() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        // No server tags → passes regardless of our domain
        assert!(verify_blossom_upload_auth(&event, &sha256, Some("any.domain.com"), 600).is_ok());
    }

    /// A `server` tag is matched against the *bound tenant host* under the
    /// shared `normalize_host` rule, so equivalent host spellings agree — the
    /// stock CLI's bare `host:port`, an explicit default port, a trailing dot,
    /// mixed case, and a full URL all match the same bound host. This is the
    /// regression guard for the multi-tenant media blocker: a non-primary
    /// tenant must accept its own server-tagged client.
    #[test]
    fn test_server_tag_normalized_against_bound_host() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let build = |server: &str| {
            let tags = vec![
                Tag::parse(["t", "upload"]).unwrap(),
                Tag::parse(["x", &sha256]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
                Tag::parse(["server", server]).unwrap(),
            ];
            EventBuilder::new(Kind::from(24242), "Upload scoped")
                .tags(tags)
                .sign_with_keys(&keys)
                .unwrap()
        };

        // Non-primary tenant host with explicit non-default port (the live
        // repro: tenant B on 127.0.0.1:3100). Stock CLI tags `host:port`.
        assert!(verify_blossom_upload_auth(
            &build("127.0.0.1:3100"),
            &sha256,
            Some("127.0.0.1:3100"),
            600
        )
        .is_ok());

        // Equivalence under normalize_host: explicit default port, trailing
        // dot, mixed case, and a full URL all collapse to the bound host.
        for tag in [
            "Relay.Example:443",
            "relay.example.",
            "RELAY.EXAMPLE",
            "https://relay.example/",
        ] {
            assert!(
                verify_blossom_upload_auth(&build(tag), &sha256, Some("relay.example"), 600)
                    .is_ok(),
                "server tag {tag:?} should match bound host relay.example"
            );
        }

        // A different tenant host still fails closed.
        assert!(matches!(
            verify_blossom_upload_auth(
                &build("127.0.0.1:3100"),
                &sha256,
                Some("127.0.0.1:3200"),
                600
            ),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_empty_content_rejected() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        // Empty content — BUD-11 requires a human-readable string
        let event = EventBuilder::new(Kind::from(24242), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_auth_event(&event, None, 600),
            Err(MediaError::InvalidAuthEvent)
        ));
    }
}
