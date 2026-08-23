//! NIP-IA identity archival requests — kind:9035 archive, kind:9036 unarchive.
//!
//! Split out of `events.rs` to keep that module under the file-size ratchet.

use buzz_core_pkg::kind::{KIND_IA_ARCHIVE_REQUEST, KIND_IA_UNARCHIVE_REQUEST};
use nostr::{EventBuilder, Kind, Tag};

use super::{check_content, message_tags::check_pubkey, tag};

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
pub(crate) fn build_archive_identity_request(
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
pub(crate) fn build_unarchive_identity_request(
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
