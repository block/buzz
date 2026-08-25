//! Persona (kind:30175) and team (kind:30176) definition wire shapes.
//!
//! NIP-AP parameterized-replaceable events keyed by `(pubkey, kind, d_tag)`.
//! This module owns the published content projections and the pure
//! build/parse/delete helpers, so every publisher — the desktop app and the
//! CLI — shares one definition of the wire contract. Mapping a client's own
//! record type onto a projection stays with that client.
//!
//! Two things here are load-bearing and easy to "clean up" into silent data
//! loss: [`PersonaEventContent`]'s field order, which pins content bytes and
//! therefore event ids, and [`TeamEventContent`]'s `Option` nesting, which
//! distinguishes "unknown, preserve local" from "explicitly emptied". Read
//! their doc comments before changing either.

use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};

use crate::builders::build_delete_addressable;
use crate::SdkError;
use buzz_core::kind::{KIND_PERSONA, KIND_TEAM};

/// Maximum d-tag length in the NIP-AP slug grammar.
pub const MAX_D_TAG_LEN: usize = 64;

/// Deserialize a field so that "absent" and "present but null" stay distinct.
///
/// `Option<Option<T>>`: outer `None` = key absent, `Some(None)` = explicit
/// `null`, `Some(Some(v))` = set. Serde collapses absent and null together
/// without this.
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

/// The JSON body stored in a persona event's content field.
///
/// Field order MUST match the NIP-AP reference vectors (`docs/nips/NIP-AP.md`
/// content body: `display_name, system_prompt, avatar_url, runtime, model,
/// provider, name_pool`). serde emits fields in declaration order, so this
/// order pins the exact content bytes and therefore the NIP-01 event id — a
/// reorder breaks cross-implementation interop.
///
/// Explicit opt-IN projection: `skip_serializing_if` on every optional field
/// keeps serialized bytes stable against pre-revision publishers, which is what
/// makes content-hash drift comparison meaningful.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaEventContent {
    /// Human-readable name shown in clients.
    pub display_name: String,
    /// Optional since the unified agent model (NIP-AP revision): a definition
    /// can be pure configuration. Writers emit `Some` whenever the record has a
    /// prompt (including the empty string) so pre-revision content bytes are
    /// unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Avatar URL, or a data URL small enough for a reader to render inline.
    /// Desktop's inline bounds are 8 KiB for `image/svg+xml` and 256 KiB for
    /// raster; past those, upload to media storage and publish the URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Preferred agent harness id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Model identifier, interpreted relative to `runtime`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Inference provider, when the runtime supports more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Candidate display names for instances created from this definition.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    /// Definition-level defaults copied onto instances at creation (NIP-AP
    /// behavioral fields). Absent = defer to client defaults;
    /// `skip_serializing_if` keeps pre-revision hashes stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    /// Pubkeys an instance answers when `respond_to` is allowlist-scoped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub respond_to_allowlist: Vec<String>,
    /// Concurrent turn limit copied onto instances at creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
}

/// The JSON body stored in a team event's content field.
///
/// Explicit opt-IN projection of the public team fields. A team carries no
/// secrets, but the projection is still explicit so a future record field is
/// published only when deliberately added here. Fields describing one client's
/// install (source directory, symlink state, builtin flag, timestamps) are
/// intentionally absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamEventContent {
    /// Team name.
    pub name: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Runtime-layered instructions. `Option` is PERMANENT wire semantics (not
    /// a transitional shim): absent = publisher predates always-publish (true
    /// value unknown — reconcile must preserve local), `null` = explicitly
    /// cleared, a string = set. New clients always publish this field (outer
    /// always `Some`), using `null` for "no instructions" so that state
    /// round-trips instead of being read back as "unknown".
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub instructions: Option<Option<String>>,
    /// Persona members, by d-tag. `Option` is PERMANENT wire semantics, not a
    /// transitional shim: `None` = publisher predates always-publish (its true
    /// membership is unknown — reconcile must preserve local), while
    /// `Some(vec![])` = explicitly emptied. New clients always publish
    /// `Some(...)`, even when empty. "Cleaning up" the `Option` reintroduces
    /// the bug where an old client's event silently wipes team membership (see
    /// the Sietch Tabr incident).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_ids: Option<Vec<String>>,
}

/// Normalize a raw slug to the NIP-AP grammar `^[a-z0-9][a-z0-9_-]{0,63}$`
/// (`docs/nips/NIP-AP.md:27`).
///
/// The relay enforces this grammar, so an un-normalized slug like
/// `CodeReviewer` or `_ops` is signed locally but REJECTED — pending forever.
/// The builders here deliberately do NOT call this: a d-tag is a coordinate,
/// and rewriting it under a caller would silently move an existing definition
/// to a new address. [`build_persona_event`] instead rejects a d-tag that is
/// not already normalized. Callers route both the outbound publish and the
/// inbound match key through this fn so the two cannot drift.
///
/// - ASCII-lowercase every char.
/// - Map any char outside `[a-z0-9_-]` to `-`.
/// - If the first char is not `[a-z0-9]` (a leading `_`/`-`), prepend `a`
///   rather than trimming — trimming `_ops`→`ops` would collide with a real
///   `ops` slug, whereas the prefix keeps distinct inputs distinct.
/// - Truncate to [`MAX_D_TAG_LEN`] bytes.
///
/// Deterministic but NOT globally injective: two slugs differing only in case
/// collapse to one d-tag. That is the correct NIP-33 behavior — same logical
/// definition, one coordinate.
pub fn normalize_d_tag(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if !out
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        out.insert(0, 'a');
    }
    out.truncate(MAX_D_TAG_LEN);
    out
}

/// Build a kind:30175 persona event from an already-projected content body.
///
/// `d_tag` is written verbatim but must already match the NIP-AP slug grammar
/// — pass raw input through [`normalize_d_tag`] first. The relay enforces the
/// grammar on persona d-tags, so accepting anything else would hand back a
/// signable event that can never publish; the builder rejects rather than
/// normalizes because rewriting a coordinate would silently move an existing
/// definition. (Team ids are the opposite; see [`build_team_event`].) `shared`
/// adds the `["shared", "true"]` tag that marks a persona for catalog
/// discovery. Returns an unsigned builder; the caller signs and submits.
pub fn build_persona_event(
    d_tag: &str,
    content: &PersonaEventContent,
    shared: bool,
) -> Result<EventBuilder, SdkError> {
    check_persona_d_tag(d_tag)?;
    let body = serde_json::to_string(content)
        .map_err(|e| SdkError::InvalidInput(format!("failed to serialize persona content: {e}")))?;
    let mut tags = vec![d_tag_of(d_tag)?];
    if shared {
        tags.push(
            Tag::parse(["shared", "true"])
                .map_err(|e| SdkError::InvalidTag(format!("invalid shared tag: {e}")))?,
        );
    }
    Ok(EventBuilder::new(Kind::Custom(KIND_PERSONA as u16), body).tags(tags))
}

/// Build a kind:30176 team event from an already-projected content body.
///
/// `d_tag` is the team's stable id, used verbatim. Do NOT pass it through
/// [`normalize_d_tag`]: the relay only length-bounds team ids, and Buzz Desktop
/// writes raw UUIDs and ids like `builtin-team:welcome`, so normalizing would
/// address a different coordinate than the one Desktop published. Returns an
/// unsigned builder; the caller signs and submits.
pub fn build_team_event(d_tag: &str, content: &TeamEventContent) -> Result<EventBuilder, SdkError> {
    let body = serde_json::to_string(content)
        .map_err(|e| SdkError::InvalidInput(format!("failed to serialize team content: {e}")))?;
    Ok(EventBuilder::new(Kind::Custom(KIND_TEAM as u16), body).tags(vec![d_tag_of(d_tag)?]))
}

/// Build a NIP-09 deletion targeting a persona's kind:30175 coordinate.
///
/// Delegates to [`build_delete_addressable`], which validates and
/// lowercase-normalizes the owner pubkey and rejects an empty d-tag — the
/// relay hex-decodes the coordinate owner, so a malformed owner builds a
/// deletion that signs fine but never lands. The single `a`-tag (deliberately
/// no `e`-tag) removes the parameterized-replaceable coordinate for every
/// client; an `e`-tag would route to the event-id deletion path and leave the
/// coordinate live.
pub fn build_persona_delete(d_tag: &str, owner_pubkey_hex: &str) -> Result<EventBuilder, SdkError> {
    build_delete_addressable(KIND_PERSONA, owner_pubkey_hex, d_tag)
}

/// Build a NIP-09 deletion targeting a team's kind:30176 coordinate.
///
/// Same validation and tag shape as [`build_persona_delete`].
pub fn build_team_delete(d_tag: &str, owner_pubkey_hex: &str) -> Result<EventBuilder, SdkError> {
    build_delete_addressable(KIND_TEAM, owner_pubkey_hex, d_tag)
}

/// Parse a kind:30175 event's content into its projection.
pub fn persona_content_from_event(event: &nostr::Event) -> Result<PersonaEventContent, SdkError> {
    serde_json::from_str(event.content.as_ref())
        .map_err(|e| SdkError::InvalidInput(format!("failed to parse persona event content: {e}")))
}

/// Parse a kind:30176 event's content into its projection.
///
/// Returns the projection, NOT a client record: install-specific local fields
/// cannot be represented, so an inbound event can only overwrite the shared
/// fields. The caller patches them onto its local record, matching on the
/// d-tag.
pub fn team_content_from_event(event: &nostr::Event) -> Result<TeamEventContent, SdkError> {
    serde_json::from_str(event.content.as_ref())
        .map_err(|e| SdkError::InvalidInput(format!("failed to parse team event content: {e}")))
}

/// Read an event's `d` tag value.
pub fn event_d_tag(event: &nostr::Event) -> Option<&str> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        match (values.first().map(String::as_str), values.get(1)) {
            (Some("d"), Some(value)) => Some(value.as_str()),
            _ => None,
        }
    })
}

fn d_tag_of(d_tag: &str) -> Result<Tag, SdkError> {
    Tag::parse(["d", d_tag]).map_err(|e| SdkError::InvalidTag(format!("invalid d-tag: {e}")))
}

/// Reject a persona d-tag not already in the NIP-AP slug grammar
/// `^[a-z0-9][a-z0-9_-]{0,63}$`. Validity is "fixed point of
/// [`normalize_d_tag`]", so the validator and the normalizer cannot drift.
fn check_persona_d_tag(d_tag: &str) -> Result<(), SdkError> {
    if normalize_d_tag(d_tag) != d_tag {
        return Err(SdkError::InvalidInput(format!(
            "persona d-tag {d_tag:?} must match ^[a-z0-9][a-z0-9_-]{{0,63}}$; \
             pass raw input through normalize_d_tag first"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::KIND_DELETION;
    use nostr::Keys;

    const OWNER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    fn persona() -> PersonaEventContent {
        PersonaEventContent {
            display_name: "Herring".into(),
            system_prompt: Some("You own output quality.".into()),
            avatar_url: None,
            runtime: Some("buzz-agent".into()),
            model: Some("databricks-kimi-3".into()),
            provider: None,
            name_pool: vec!["Herring".into()],
            respond_to: Some("owner-only".into()),
            respond_to_allowlist: Vec::new(),
            parallelism: Some(24),
        }
    }

    fn team() -> TeamEventContent {
        TeamEventContent {
            name: "Red team".into(),
            description: Some("Three skeptics.".into()),
            instructions: Some(None),
            persona_ids: Some(vec!["quinby".into(), "herring".into()]),
        }
    }

    fn sign(builder: EventBuilder) -> nostr::Event {
        builder.sign_with_keys(&Keys::generate()).unwrap()
    }

    #[test]
    fn persona_event_carries_kind_d_tag_and_content() {
        let event = sign(build_persona_event("herring", &persona(), false).unwrap());
        assert_eq!(event.kind.as_u16() as u32, KIND_PERSONA);
        assert_eq!(event_d_tag(&event), Some("herring"));
        assert_eq!(persona_content_from_event(&event).unwrap(), persona());
    }

    /// A d-tag is a coordinate: the builders validate it but never rewrite
    /// it, since normalizing inside the builder would silently move an
    /// existing definition to a new address behind the caller's back.
    #[test]
    fn builders_use_the_d_tag_verbatim() {
        let persona_event = sign(build_persona_event("code-reviewer", &persona(), false).unwrap());
        assert_eq!(event_d_tag(&persona_event), Some("code-reviewer"));
        // Team ids are not slugs: Desktop publishes raw UUIDs and ids like
        // `builtin-team:welcome`, so mixed case passes through untouched.
        let team_event = sign(build_team_event("Sietch-Tabr", &team()).unwrap());
        assert_eq!(event_d_tag(&team_event), Some("Sietch-Tabr"));
    }

    /// The relay rejects persona d-tags outside `^[a-z0-9][a-z0-9_-]{0,63}$`,
    /// so an unvalidated builder hands the caller a signable event that can
    /// never publish.
    #[test]
    fn persona_builder_rejects_d_tags_outside_the_slug_grammar() {
        let overlong = "z".repeat(MAX_D_TAG_LEN + 1);
        for bad in [
            "",
            "CodeReviewer",
            "_ops",
            "-x",
            "a b",
            "caf\u{e9}",
            overlong.as_str(),
        ] {
            assert!(
                matches!(
                    build_persona_event(bad, &persona(), false),
                    Err(SdkError::InvalidInput(_))
                ),
                "d_tag={bad:?} should be rejected"
            );
        }
    }

    /// The relay hex-decodes the coordinate owner when handling a deletion, so
    /// a malformed owner builds a deletion that signs fine but never lands.
    #[test]
    fn delete_builders_reject_malformed_owners_and_empty_d_tags() {
        let truncated = &OWNER[..63];
        for owner in ["", "not-a-pubkey", truncated, "zz"] {
            assert!(
                matches!(
                    build_persona_delete("herring", owner),
                    Err(SdkError::InvalidInput(_))
                ),
                "persona owner={owner:?} should be rejected"
            );
            assert!(
                matches!(
                    build_team_delete("red-team", owner),
                    Err(SdkError::InvalidInput(_))
                ),
                "team owner={owner:?} should be rejected"
            );
        }
        assert!(build_persona_delete("", OWNER).is_err());
        assert!(build_team_delete("", OWNER).is_err());
    }

    /// Coordinate owners are lowercase on the wire; an uppercase caller input
    /// must normalize rather than produce a coordinate the relay can't match.
    #[test]
    fn delete_builders_lowercase_the_owner_pubkey() {
        let upper = OWNER.to_ascii_uppercase();
        for (builder, kind, d_tag) in [
            (
                build_persona_delete("herring", &upper).unwrap(),
                KIND_PERSONA,
                "herring",
            ),
            (
                build_team_delete("red-team", &upper).unwrap(),
                KIND_TEAM,
                "red-team",
            ),
        ] {
            let event = sign(builder);
            let a_tag = event
                .tags
                .iter()
                .map(|t| t.as_slice())
                .find(|v| v.first().map(String::as_str) == Some("a"))
                .expect("a-tag");
            assert_eq!(a_tag[1], format!("{kind}:{OWNER}:{d_tag}"));
        }
    }

    #[test]
    fn shared_flag_controls_the_shared_tag() {
        for shared in [false, true] {
            let event = sign(build_persona_event("herring", &persona(), shared).unwrap());
            let has = event
                .tags
                .iter()
                .any(|t| t.as_slice().first().map(String::as_str) == Some("shared"));
            assert_eq!(has, shared, "shared={shared}");
        }
    }

    /// Field order pins the content bytes and therefore the event id; a reorder
    /// breaks cross-implementation interop and every stored content hash.
    #[test]
    fn persona_content_field_order_matches_nip_ap() {
        // Byte-exact, every field populated: a substring-order check would miss
        // a field appearing in the wrong place among the ones it doesn't name.
        let full = PersonaEventContent {
            display_name: "Herring".into(),
            system_prompt: Some("Ask the annoying question.".into()),
            avatar_url: Some("https://example.invalid/h.png".into()),
            runtime: Some("claude".into()),
            model: Some("claude-opus-5".into()),
            provider: Some("anthropic".into()),
            name_pool: vec!["Herring".into()],
            respond_to: Some("allowlist".into()),
            respond_to_allowlist: vec!["ab".repeat(32)],
            parallelism: Some(2),
        };
        let allowlisted = "ab".repeat(32);
        let expected = concat!(
            r#"{"display_name":"Herring","system_prompt":"Ask the annoying question.","#,
            r#""avatar_url":"https://example.invalid/h.png","runtime":"claude","#,
            r#""model":"claude-opus-5","provider":"anthropic","name_pool":["Herring"],"#,
            r#""respond_to":"allowlist","respond_to_allowlist":["{ALLOWLISTED}"],"parallelism":2}"#,
        )
        .replace("{ALLOWLISTED}", &allowlisted);
        assert_eq!(serde_json::to_string(&full).unwrap(), expected);

        // Absent optionals stay absent so pre-revision bytes are unchanged.
        let json = serde_json::to_string(&persona()).unwrap();
        assert!(!json.contains("avatar_url"), "{json}");
        assert!(!json.contains("provider"), "{json}");
        assert!(!json.contains("respond_to_allowlist"), "{json}");
    }

    #[test]
    fn team_event_round_trips_through_content() {
        let event = sign(build_team_event("red-team", &team()).unwrap());
        assert_eq!(event.kind.as_u16() as u32, KIND_TEAM);
        assert_eq!(event_d_tag(&event), Some("red-team"));
        assert_eq!(team_content_from_event(&event).unwrap(), team());
    }

    /// Absent vs explicitly-null must stay distinguishable: absent means
    /// "publisher predates always-publish, preserve local", null means
    /// "cleared". Collapsing them wipes team state.
    #[test]
    fn absent_and_null_instructions_are_distinct() {
        let absent: TeamEventContent =
            serde_json::from_str(r#"{"name":"T","persona_ids":[]}"#).unwrap();
        assert_eq!(absent.instructions, None);

        let cleared: TeamEventContent =
            serde_json::from_str(r#"{"name":"T","instructions":null,"persona_ids":[]}"#).unwrap();
        assert_eq!(cleared.instructions, Some(None));

        let set: TeamEventContent =
            serde_json::from_str(r#"{"name":"T","instructions":"go","persona_ids":[]}"#).unwrap();
        assert_eq!(set.instructions, Some(Some("go".into())));

        // A cleared value must serialize as null, not vanish.
        let json = serde_json::to_string(&cleared).unwrap();
        assert!(json.contains(r#""instructions":null"#), "{json}");
    }

    /// Absent membership means "unknown, preserve local"; an empty array means
    /// "deliberately emptied". These must not serialize identically — the
    /// absent shape is the Sietch Tabr wipe event.
    #[test]
    fn absent_and_empty_persona_ids_are_distinct() {
        let absent: TeamEventContent = serde_json::from_str(r#"{"name":"T"}"#).unwrap();
        assert_eq!(absent.persona_ids, None);
        assert!(!serde_json::to_string(&absent)
            .unwrap()
            .contains("persona_ids"));

        let emptied = TeamEventContent {
            persona_ids: Some(Vec::new()),
            ..absent
        };
        assert!(serde_json::to_string(&emptied)
            .unwrap()
            .contains(r#""persona_ids":[]"#));
    }

    #[test]
    fn d_tag_normalization_follows_the_relay_grammar() {
        assert_eq!(normalize_d_tag("CodeReviewer"), "codereviewer");
        assert_eq!(normalize_d_tag("_ops"), "a_ops", "leading _ is prefixed");
        assert_eq!(normalize_d_tag("-x"), "a-x", "leading - is prefixed");
        assert_eq!(normalize_d_tag("a b.c"), "a-b-c");
        assert_eq!(normalize_d_tag(&"z".repeat(80)).len(), MAX_D_TAG_LEN);
        assert_eq!(normalize_d_tag(""), "a", "empty stays grammar-valid");
        // Already-valid slugs pass through untouched.
        assert_eq!(normalize_d_tag("red-team"), "red-team");
    }

    #[test]
    fn delete_uses_one_coordinate_tag_and_no_e_tag() {
        for (builder, kind, d_tag) in [
            (
                build_persona_delete("herring", OWNER).unwrap(),
                KIND_PERSONA,
                "herring",
            ),
            (
                build_team_delete("red-team", OWNER).unwrap(),
                KIND_TEAM,
                "red-team",
            ),
        ] {
            let event = sign(builder);
            assert_eq!(event.kind, Kind::Custom(KIND_DELETION as u16));

            let a_tags: Vec<&[String]> = event
                .tags
                .iter()
                .map(|t| t.as_slice())
                .filter(|v| v.first().map(String::as_str) == Some("a"))
                .collect();
            assert_eq!(a_tags.len(), 1);
            assert_eq!(a_tags[0][1], format!("{kind}:{OWNER}:{d_tag}"));

            // An e-tag would route to the event-id deletion path and leave the
            // replaceable coordinate live.
            assert!(event
                .tags
                .iter()
                .all(|t| t.as_slice().first().map(String::as_str) != Some("e")));
        }
    }
}
