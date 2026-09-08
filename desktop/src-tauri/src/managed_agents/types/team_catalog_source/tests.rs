use super::{TeamCatalogSource, TeamMemberCatalogSource};

fn source(owner_pubkey: &str, team_d_tag: &str) -> TeamCatalogSource {
    TeamCatalogSource {
        owner_pubkey: owner_pubkey.to_string(),
        team_d_tag: team_d_tag.to_string(),
    }
}

#[test]
fn normalized_lowercases_and_trims_the_owner_pubkey() {
    // "Already added" compares this against a publication's author hex, which
    // is always lowercase — a mixed-case value from the UI must not miss.
    let normalized = source(&format!("  {}  ", "A".repeat(64)), " team-abc ")
        .normalized()
        .expect("64 hex chars with surrounding space is valid");
    assert_eq!(normalized.owner_pubkey, "a".repeat(64));
    assert_eq!(normalized.team_d_tag, "team-abc");
}

#[test]
fn normalized_rejects_a_short_owner_pubkey() {
    let err = source("abc123", "team-abc").normalized().unwrap_err();
    assert!(err.contains("64 hex"), "error must name the rule: {err}");
}

#[test]
fn normalized_rejects_a_non_hex_owner_pubkey() {
    let err = source(&"z".repeat(64), "team-abc")
        .normalized()
        .unwrap_err();
    assert!(err.contains("64 hex"), "error must name the rule: {err}");
}

#[test]
fn normalized_rejects_a_blank_team_d_tag() {
    let err = source(&"a".repeat(64), "   ").normalized().unwrap_err();
    assert!(err.contains("d-tag"), "error must name the field: {err}");
}

#[test]
fn deserializes_the_camel_case_payload_the_frontend_sends() {
    let parsed: TeamCatalogSource =
        serde_json::from_str(r#"{"ownerPubkey":"abc","teamDTag":"team-abc"}"#)
            .expect("camelCase payload from TS should deserialize");
    assert_eq!(parsed, source("abc", "team-abc"));
}

#[test]
fn round_trips_persisted_snake_case() {
    let value = source(&"a".repeat(64), "team-abc");
    let json = serde_json::to_string(&value).unwrap();
    assert!(json.contains("owner_pubkey"), "persisted shape: {json}");
    assert_eq!(
        serde_json::from_str::<TeamCatalogSource>(&json).unwrap(),
        value,
        "the camelCase alias must not break the stored-record round trip"
    );
}

#[test]
fn member_provenance_round_trips_all_four_components() {
    // Reuse safety depends on every component surviving a store round trip:
    // a dropped `projection_hash` would silently widen a version-pinned match
    // into a version-agnostic one.
    let value = TeamMemberCatalogSource {
        owner_pubkey: "a".repeat(64),
        team_d_tag: "team-abc".to_string(),
        member_key: "member-1".to_string(),
        projection_hash: "b".repeat(64),
    };
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        serde_json::from_str::<TeamMemberCatalogSource>(&json).unwrap(),
        value
    );
}

#[test]
fn member_provenance_differs_when_only_the_projection_hash_differs() {
    // The equality that gates copy reuse must treat two versions of one
    // published member as distinct records.
    let base = TeamMemberCatalogSource {
        owner_pubkey: "a".repeat(64),
        team_d_tag: "team-abc".to_string(),
        member_key: "member-1".to_string(),
        projection_hash: "b".repeat(64),
    };
    let newer = TeamMemberCatalogSource {
        projection_hash: "c".repeat(64),
        ..base.clone()
    };
    assert_ne!(base, newer);
}
