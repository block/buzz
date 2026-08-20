//! Serialize `TeamRecord` ↔ kind:30176 team events and publish via relay.
//!
//! Team events are NIP-33 parameterized replaceable events keyed by
//! `(pubkey, kind, d_tag)` where `d_tag` is the team's stable id. They mirror
//! the persona event flow (see `persona_events`): the same retention store and
//! flush loop publish them — this module only owns the kind-specific
//! projection, build, and tombstone.

use buzz_core_pkg::kind::KIND_TEAM;
use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};

use super::TeamRecord;

/// The JSON body stored in a team event's content field.
///
/// Explicit opt-IN projection of the public team fields. A team carries no
/// secrets, but the projection is still explicit so a future `TeamRecord`
/// field is published only when deliberately added here. Local-only fields
/// (`source_dir`, `is_symlink`, `is_builtin`, timestamps) are intentionally
/// omitted — they describe this client's install, not the shared team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamEventContent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Runtime-layered instructions. `Option` is PERMANENT wire semantics
    /// (not a transitional shim): absent = publisher predates always-publish
    /// (true value unknown — reconcile must preserve local), `null` =
    /// explicitly cleared, a string = set. New clients always publish this
    /// field (outer always `Some`), using `null` for "no instructions" so
    /// that state round-trips instead of being read back as "unknown".
    #[serde(
        default,
        deserialize_with = "crate::util::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub instructions: Option<Option<String>>,
    /// Pack persona members. `Option` is PERMANENT wire semantics, not a
    /// transitional shim: `None` = publisher predates always-publish (its
    /// true membership is unknown — reconcile must preserve local), while
    /// `Some(vec![])` = explicitly emptied. New clients always publish
    /// `Some(...)`, even when empty. "Cleaning up" the Option later
    /// reintroduces the bug where an old client's event silently wipes team
    /// membership (see the Sietch Tabr incident).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_ids: Option<Vec<String>>,
}

/// Project a `TeamRecord` onto the content fields published in team events.
/// Centralizes the field mapping so a new published field is added in exactly
/// one place.
pub fn team_event_content(record: &TeamRecord) -> TeamEventContent {
    TeamEventContent {
        name: record.name.clone(),
        description: record.description.clone(),
        // Always `Some`, even when the inner value is absent — `None` is
        // reserved for events from clients that predate always-publish (see
        // the struct doc comments).
        instructions: Some(record.instructions.clone()),
        persona_ids: Some(record.persona_ids.clone()),
    }
}

/// Build a kind:30176 event from a `TeamRecord`.
///
/// Returns an unsigned `EventBuilder` — the caller signs and submits.
pub fn build_team_event(record: &TeamRecord) -> Result<EventBuilder, String> {
    let content = serde_json::to_string(&team_event_content(record))
        .map_err(|e| format!("failed to serialize team content: {e}"))?;
    let tags =
        vec![Tag::parse(["d", record.id.as_str()]).map_err(|e| format!("invalid d-tag: {e}"))?];
    Ok(EventBuilder::new(Kind::Custom(KIND_TEAM as u16), content).tags(tags))
}

/// Parse a kind:30176 event's content into the projection — the inbound
/// counterpart of [`team_event_content`].
///
/// Returns [`TeamEventContent`], NOT a [`TeamRecord`]: install-specific local
/// fields (`source_dir`, `is_symlink`, `symlink_target`, `is_builtin`,
/// `version`, timestamps) cannot be represented by the return type, so an
/// inbound event can only ever overwrite the three shared fields. The caller
/// patches them onto the local record (see `apply_inbound_team`), matching on
/// the d-tag (the team's id).
pub fn team_content_from_event(event: &nostr::Event) -> Result<TeamEventContent, String> {
    serde_json::from_str(event.content.as_ref())
        .map_err(|e| format!("failed to parse team event content: {e}"))
}

/// Build a NIP-09 deletion (kind:5) targeting a team's kind:30176 event.
///
/// Carries a single `a`-tag with the NIP-33 coordinate `30176:<owner>:<d_tag>`
/// and no `e`-tag: an `e`-tag routes the relay to the event-id deletion path,
/// leaving the parameterized-replaceable coordinate live. The coordinate
/// delete removes the team for every client and across reboots.
pub fn build_team_delete(d_tag: &str, owner_pubkey_hex: &str) -> Result<EventBuilder, String> {
    let coord = format!("{KIND_TEAM}:{owner_pubkey_hex}:{d_tag}");
    let tag = Tag::parse(["a", coord.as_str()]).map_err(|e| format!("invalid a-tag: {e}"))?;
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(vec![tag]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_team() -> TeamRecord {
        TeamRecord {
            id: "team-123".to_string(),
            name: "Test Team".to_string(),
            description: Some("A test team".to_string()),
            instructions: Some("Coordinate carefully.".to_string()),
            persona_ids: vec!["p1".to_string(), "p2".to_string()],
            is_builtin: false,
            source_dir: Some(PathBuf::from("/local/only/path")),
            is_symlink: true,
            symlink_target: Some("/somewhere".to_string()),
            version: Some("1.0".to_string()),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn build_team_event_produces_correct_kind() {
        let builder = build_team_event(&sample_team()).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(event.kind.as_u16() as u32, KIND_TEAM);
    }

    #[test]
    fn d_tag_is_team_id() {
        let builder = build_team_event(&sample_team()).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        let d = event
            .tags
            .iter()
            .find_map(|t| {
                let v: Vec<&str> = t.as_slice().iter().map(|s| s.as_str()).collect();
                (v.first() == Some(&"d"))
                    .then(|| v.get(1).map(|s| s.to_string()))
                    .flatten()
            })
            .unwrap();
        assert_eq!(d, "team-123");
    }

    #[test]
    fn content_omits_local_only_fields() {
        let event_content = team_event_content(&sample_team());
        let json = serde_json::to_string(&event_content).unwrap();
        // Published fields present.
        assert!(json.contains("\"name\""));
        assert!(json.contains("\"persona_ids\""));
        assert!(json.contains("\"instructions\""));
        // Local-only / install-specific fields never published.
        assert!(!json.contains("source_dir"));
        assert!(!json.contains("is_symlink"));
        assert!(!json.contains("symlink_target"));
        assert!(!json.contains("is_builtin"));
        assert!(!json.contains("created_at"));
        assert!(!json.contains("version"));
    }

    #[test]
    fn content_round_trips() {
        let event_content = team_event_content(&sample_team());
        let json = serde_json::to_string(&event_content).unwrap();
        let restored: TeamEventContent = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, event_content);
    }

    // ── persona_ids wire semantics (omission must not wipe membership) ────

    #[test]
    fn content_from_old_clients_without_persona_ids_parses_none() {
        // Events published before always-publish must still parse. The
        // field reads back `None` — NOT an empty list — so the reconcile
        // can distinguish "publisher predates always-publish" from
        // "explicitly emptied" and preserve local membership (see
        // apply_inbound_team). This is the exact shape of the Sietch Tabr
        // wipe event.
        let legacy = r#"{"name":"Old Team"}"#;
        let restored: TeamEventContent = serde_json::from_str(legacy).unwrap();
        assert_eq!(restored.name, "Old Team");
        assert_eq!(restored.persona_ids, None);
    }

    #[test]
    fn content_publishes_some_even_when_persona_ids_empty() {
        // New clients must always publish `Some`, even for an empty list —
        // `Some(vec![])` is the explicit "no persona members" signal that a
        // pre-fix client can never produce.
        let mut team = sample_team();
        team.persona_ids = vec![];
        let event_content = team_event_content(&team);
        assert_eq!(event_content.persona_ids, Some(vec![]));
        let json = serde_json::to_string(&event_content).unwrap();
        assert!(json.contains("\"persona_ids\":[]"));
    }

    // ── instructions wire semantics (tri-state: absent/null/value) ────────
    //
    // Deserialized from raw JSON strings, not constructed enum states
    // directly — the bug is specifically in how serde maps JSON shapes onto
    // the tri-state, so the test must exercise that mapping (mirrors
    // `double_option_tristate` in `util.rs`).

    #[test]
    fn content_from_old_clients_without_instructions_parses_none() {
        let legacy = r#"{"name":"Old Team","persona_ids":["p1"]}"#;
        let restored: TeamEventContent = serde_json::from_str(legacy).unwrap();
        assert_eq!(restored.instructions, None);
    }

    #[test]
    fn content_with_explicit_null_instructions_parses_some_none() {
        let json = r#"{"name":"Team","persona_ids":["p1"],"instructions":null}"#;
        let restored: TeamEventContent = serde_json::from_str(json).unwrap();
        assert_eq!(restored.instructions, Some(None));
    }

    #[test]
    fn content_with_instructions_value_parses_some_some() {
        let json = r#"{"name":"Team","persona_ids":["p1"],"instructions":"Coordinate."}"#;
        let restored: TeamEventContent = serde_json::from_str(json).unwrap();
        assert_eq!(restored.instructions, Some(Some("Coordinate.".to_string())));
    }

    #[test]
    fn content_publishes_some_none_when_instructions_absent_locally() {
        // New clients must always publish the field — `null` is the explicit
        // "no instructions" signal that a pre-fix client can never produce.
        let mut team = sample_team();
        team.instructions = None;
        let event_content = team_event_content(&team);
        assert_eq!(event_content.instructions, Some(None));
        let json = serde_json::to_string(&event_content).unwrap();
        assert!(json.contains("\"instructions\":null"));
    }

    #[test]
    fn build_team_delete_has_single_a_tag_no_e_tag() {
        const OWNER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        let builder = build_team_delete("team-123", OWNER).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();

        assert_eq!(event.kind, Kind::Custom(5));

        let a_tags: Vec<&[String]> = event
            .tags
            .iter()
            .map(|t| t.as_slice())
            .filter(|v| v.first().map(String::as_str) == Some("a"))
            .collect();
        assert_eq!(a_tags.len(), 1);
        assert_eq!(a_tags[0][1], format!("{KIND_TEAM}:{OWNER}:team-123"));

        // An e-tag would route to the event-id deletion path and leave the
        // replaceable coordinate live — the tombstone must carry none.
        assert!(event
            .tags
            .iter()
            .all(|t| t.as_slice().first().map(String::as_str) != Some("e")));
    }
}
