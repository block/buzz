use super::CatalogSource;

fn source(owner_pubkey: &str, persona_id: &str) -> CatalogSource {
    CatalogSource {
        owner_pubkey: owner_pubkey.to_string(),
        persona_id: persona_id.to_string(),
    }
}

#[test]
fn normalized_lowercases_and_trims_the_owner_pubkey() {
    // "Already added" compares this against a publication's author hex, which
    // is always lowercase — a mixed-case value from the UI must not miss.
    let normalized = source(&format!("  {}  ", "A".repeat(64)), " helper ")
        .normalized()
        .expect("64 hex chars with surrounding space is valid");
    assert_eq!(normalized.owner_pubkey, "a".repeat(64));
    assert_eq!(normalized.persona_id, "helper");
}

#[test]
fn normalized_rejects_a_short_owner_pubkey() {
    let err = source("abc123", "helper").normalized().unwrap_err();
    assert!(err.contains("64 hex"), "error must name the rule: {err}");
}

#[test]
fn normalized_rejects_a_non_hex_owner_pubkey() {
    let err = source(&"z".repeat(64), "helper").normalized().unwrap_err();
    assert!(err.contains("64 hex"), "error must name the rule: {err}");
}

#[test]
fn normalized_rejects_a_blank_persona_id() {
    let err = source(&"a".repeat(64), "   ").normalized().unwrap_err();
    assert!(
        err.contains("persona id"),
        "error must name the field: {err}"
    );
}

#[test]
fn deserializes_the_camel_case_payload_the_frontend_sends() {
    // `rename_all` on CreatePersonaRequest does not recurse into this struct,
    // so without the aliases the copy request fails at the Tauri boundary.
    let parsed: CatalogSource =
        serde_json::from_str(r#"{"ownerPubkey":"abc","personaId":"helper"}"#)
            .expect("camelCase payload from TS should deserialize");
    assert_eq!(parsed, source("abc", "helper"));
}

#[test]
fn round_trips_persisted_snake_case() {
    let value = source(&"a".repeat(64), "helper");
    let json = serde_json::to_string(&value).unwrap();
    assert!(json.contains("owner_pubkey"), "persisted shape: {json}");
    assert_eq!(
        serde_json::from_str::<CatalogSource>(&json).unwrap(),
        value,
        "the camelCase alias must not break the stored-record round trip"
    );
}
