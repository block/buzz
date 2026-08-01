//! NIP-IA identity archival event builders.
//!
//! Split out of `events.rs` to keep that module under the desktop file-size
//! ceiling; see `desktop/scripts/check-file-sizes.mjs`.

use buzz_core_pkg::kind::{KIND_IA_ARCHIVE_REQUEST, KIND_IA_UNARCHIVE_REQUEST};
use nostr::{EventBuilder, Kind, Tag};

use crate::events::{check_content, check_pubkey, tag};

// ── NIP-IA identity archival ─────────────────────────────────────────────────
//
// kind:9035 archive request, kind:9036 unarchive request.
// Both protected by NIP-70 (`["-"]`), p-tag the target, and may carry
// optional `reason` (machine-readable code), `replaced-by` (9035 only),
// and a NIP-OA `auth` tag for owner-of-agent requests.
//
// See docs/nips/NIP-IA.md §Event Formats. The relay verifies; the desktop's
// job is to produce a well-formed, signed request — consent path is selected
// by the relay, not declared here.

fn check_reason(reason: &str) -> Result<(), String> {
    // Reason codes are machine-readable strings; the spec doesn't cap length
    // but we keep them short to discourage stuffing prose where `content` goes.
    if reason.len() > 64 {
        return Err(format!(
            "reason code exceeds maximum length of 64 chars (got {})",
            reason.len()
        ));
    }
    if reason.chars().any(|c| c.is_control()) {
        return Err("reason code must not contain control characters".into());
    }
    Ok(())
}

fn identity_archive_tags(
    target_pubkey: &str,
    reason: Option<&str>,
    replaced_by: Option<&str>,
    auth_tag: Option<&[String; 4]>,
) -> Result<Vec<Tag>, String> {
    check_pubkey(target_pubkey)?;
    let target_lower = target_pubkey.to_ascii_lowercase();

    let mut tags = Vec::with_capacity(5);
    // NIP-70: mark as protected administrative state.
    tags.push(tag(vec!["-"])?);
    tags.push(tag(vec!["p", &target_lower])?);

    if let Some(r) = reason {
        check_reason(r)?;
        tags.push(tag(vec!["reason", r])?);
    }

    if let Some(rb) = replaced_by {
        check_pubkey(rb)?;
        let rb_lower = rb.to_ascii_lowercase();
        if rb_lower == target_lower {
            return Err("replaced-by must differ from the target".into());
        }
        tags.push(tag(vec!["replaced-by", &rb_lower])?);
    }

    if let Some(auth) = auth_tag {
        // Structural check only — the relay performs full NIP-OA verification.
        // We require the label, a 64-hex owner pubkey, and a 128-hex signature.
        if auth[0] != "auth" {
            return Err(format!(
                "auth tag label must be \"auth\" (got \"{}\")",
                auth[0]
            ));
        }
        check_pubkey(&auth[1])?;
        if auth[3].len() != 128 || !auth[3].chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("auth tag signature must be 128-character hex".into());
        }
        tags.push(tag(vec!["auth", &auth[1], &auth[2], &auth[3]])?);
    }

    Ok(tags)
}

/// Kind 9035 — NIP-IA archive request.
///
/// `content` is an optional human-readable reason (clients MUST NOT parse
/// authorization semantics from it). `reason` is the machine-readable code
/// (`rotated`, `retired`, `bot-rebuilt`, `left-organization`, `spam`, ...).
/// `replaced_by` is the rotation pointer. `auth` is a NIP-OA owner-attestation
/// tag required only for the owner-of-agent consent path.
///
/// `.allow_self_tagging()` is required: NIP-IA's self path has `actor==target`,
/// which means the request's `["p", target]` matches the signer. nostr 0.44
/// strips matching `p` tags by default — we need the wire form intact.
pub fn build_archive_identity_request(
    target_pubkey: &str,
    content: &str,
    reason: Option<&str>,
    replaced_by: Option<&str>,
    auth: Option<&[String; 4]>,
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let tags = identity_archive_tags(target_pubkey, reason, replaced_by, auth)?;
    Ok(
        EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVE_REQUEST as u16), content)
            .tags(tags)
            .allow_self_tagging(),
    )
}

/// Kind 9036 — NIP-IA unarchive request.
///
/// Same shape as 9035 minus `replaced-by` (which has no defined meaning on
/// unarchive per spec). `auth` is used for owner-of-agent unarchive paths.
/// See `build_archive_identity_request` for the rationale on
/// `.allow_self_tagging()`.
pub fn build_unarchive_identity_request(
    target_pubkey: &str,
    content: &str,
    reason: Option<&str>,
    auth: Option<&[String; 4]>,
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let tags = identity_archive_tags(target_pubkey, reason, None, auth)?;
    Ok(
        EventBuilder::new(Kind::Custom(KIND_IA_UNARCHIVE_REQUEST as u16), content)
            .tags(tags)
            .allow_self_tagging(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    /// Builder layout regression for the NIP-IA owner-of-agent archive flow.
    /// Compares against `docs/nips/NIP-IA.md` §Vector 1.
    #[test]
    fn archive_identity_request_matches_spec_vector_1_layout() {
        const OWNER_HEX: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        const TARGET_HEX: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        const CONDITIONS: &str = "kind=1&created_at<1713957000";
        const SIG: &str = "8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369";

        let auth: [String; 4] = [
            "auth".into(),
            OWNER_HEX.into(),
            CONDITIONS.into(),
            SIG.into(),
        ];
        let builder = build_archive_identity_request(
            TARGET_HEX,
            "Archiving zombie agent after rebuild.",
            Some("bot-rebuilt"),
            None,
            Some(&auth),
        )
        .expect("build_archive_identity_request");

        let owner_secret = nostr::SecretKey::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
        let owner_keys = Keys::new(owner_secret);
        let event = builder.sign_with_keys(&owner_keys).unwrap();

        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();

        assert_eq!(event.kind, Kind::Custom(KIND_IA_ARCHIVE_REQUEST as u16));
        // Spec layout: ["-"], ["p", target], ["reason", code], ["auth", ...]
        assert_eq!(tags[0], vec!["-"]);
        assert_eq!(tags[1], vec!["p", TARGET_HEX]);
        assert_eq!(tags[2], vec!["reason", "bot-rebuilt"]);
        assert_eq!(tags[3], vec!["auth", OWNER_HEX, CONDITIONS, SIG]);
    }

    #[test]
    fn archive_request_rejects_replaced_by_equal_target() {
        const TARGET_HEX: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        let err = build_archive_identity_request(TARGET_HEX, "", None, Some(TARGET_HEX), None)
            .unwrap_err();
        assert!(err.contains("replaced-by"));
    }

    #[test]
    fn unarchive_request_layout_self_path() {
        const TARGET_HEX: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        let builder = build_unarchive_identity_request(
            TARGET_HEX,
            "I am active again.",
            Some("returned"),
            None,
        )
        .unwrap();
        let target_secret = nostr::SecretKey::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000002",
        )
        .unwrap();
        let event = builder.sign_with_keys(&Keys::new(target_secret)).unwrap();
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(event.kind, Kind::Custom(KIND_IA_UNARCHIVE_REQUEST as u16));
        // Self-unarchive: the `p` tag MUST point at the signer. Verifies our
        // `.allow_self_tagging()` call survives nostr 0.44's default scrub.
        assert_eq!(tags[0], vec!["-"]);
        assert_eq!(tags[1], vec!["p", TARGET_HEX]);
        assert_eq!(tags[2], vec!["reason", "returned"]);
        assert_eq!(tags.len(), 3, "self unarchive must not carry auth tag");
        assert_eq!(event.pubkey.to_hex(), TARGET_HEX);
    }
}
