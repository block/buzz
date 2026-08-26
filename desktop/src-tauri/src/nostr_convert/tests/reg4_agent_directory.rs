use super::*;

/// REG-4: owner-attested capability strings and model id published on a
/// kind:30177 record reach the directory, and stay empty/absent for records
/// published by builds that predate the fields.
#[test]
fn managed_agent_directory_surfaces_owner_attested_capabilities_and_model() {
    let agent_keys = Keys::generate();
    let owner_keys = Keys::generate();
    let agent_pubkey = agent_keys.public_key().to_hex();

    let auth_tag_json =
        buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "")
            .expect("compute auth tag");
    let auth_tag_values: Vec<String> =
        serde_json::from_str(&auth_tag_json).expect("parse auth tag json");
    let profile = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Bumble"}"#)
        .tags([Tag::parse(auth_tag_values).expect("parse auth tag")])
        .sign_with_keys(&agent_keys)
        .expect("sign profile");

    let stamped = EventBuilder::new(
        Kind::Custom(30177),
        serde_json::json!({
            "name": "Bumble",
            "parallelism": 1,
            "respond_to": "anyone",
            "capabilities": ["web-search", "code-review"],
            "model": "claude-opus-4",
        })
        .to_string(),
    )
    .tags([Tag::parse(["d", agent_pubkey.as_str()]).expect("parse d tag")])
    .sign_with_keys(&owner_keys)
    .expect("sign managed-agent event");

    let agents = relay_agents_from_managed_agent_events(&[stamped], std::slice::from_ref(&profile));
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].capabilities, vec!["web-search", "code-review"]);
    assert_eq!(agents[0].model.as_deref(), Some("claude-opus-4"));

    // A record from an older build yields empty capabilities and no model —
    // never fabricated values.
    let unstamped = managed_agent_event(&owner_keys, &agent_pubkey, "Bumble", "anyone", &[]);
    let agents =
        relay_agents_from_managed_agent_events(&[unstamped], std::slice::from_ref(&profile));
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].capabilities, Vec::<String>::new());
    assert_eq!(agents[0].model, None);
}

/// REG-4 negative: the validate-or-None rule applies per capability string
/// and to the model — a hostile value degrades alone and never hides a real,
/// reachable agent.
#[test]
fn managed_agent_directory_drops_invalid_capabilities_and_model_but_keeps_the_agent() {
    let agent_keys = Keys::generate();
    let owner_keys = Keys::generate();
    let agent_pubkey = agent_keys.public_key().to_hex();

    let auth_tag_json =
        buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "")
            .expect("compute auth tag");
    let auth_tag_values: Vec<String> =
        serde_json::from_str(&auth_tag_json).expect("parse auth tag json");
    let profile = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Bumble"}"#)
        .tags([Tag::parse(auth_tag_values).expect("parse auth tag")])
        .sign_with_keys(&agent_keys)
        .expect("sign profile");

    // A bidi override in one capability, an over-long second capability, and
    // a bidi-bearing model — all correctly signed.
    let hostile = EventBuilder::new(
        Kind::Custom(30177),
        serde_json::json!({
            "name": "Bumble",
            "parallelism": 1,
            "respond_to": "anyone",
            "capabilities": ["ok-capability", "evil\u{202E}cap", "a".repeat(65).as_str()],
            "model": "evil\u{202E}model",
        })
        .to_string(),
    )
    .tags([Tag::parse(["d", agent_pubkey.as_str()]).expect("parse d tag")])
    .sign_with_keys(&owner_keys)
    .expect("sign managed-agent event");

    let agents = relay_agents_from_managed_agent_events(&[hostile], std::slice::from_ref(&profile));
    assert_eq!(agents.len(), 1, "the agent itself must still be reachable");
    assert_eq!(agents[0].name, "Bumble");
    assert_eq!(
        agents[0].capabilities,
        vec!["ok-capability"],
        "the valid capability survives; bidi and over-long ones are dropped"
    );
    assert_eq!(agents[0].model, None, "bidi-bearing model dropped");
}

/// REG-4 edge cases: boundary and whitespace handling for directory strings.
/// - exactly MAX_DIRECTORY_STRING_CHARS (64) chars survives; 65 does not
/// - a whitespace-only capability is dropped, not rendered
#[test]
fn managed_agent_directory_capability_length_and_whitespace_boundaries() {
    let agent_keys = Keys::generate();
    let owner_keys = Keys::generate();
    let agent_pubkey = agent_keys.public_key().to_hex();

    let auth_tag_json =
        buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "")
            .expect("compute auth tag");
    let auth_tag_values: Vec<String> =
        serde_json::from_str(&auth_tag_json).expect("parse auth tag json");
    let profile = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Bumble"}"#)
        .tags([Tag::parse(auth_tag_values).expect("parse auth tag")])
        .sign_with_keys(&agent_keys)
        .expect("sign profile");

    let boundary = "a".repeat(64);
    let model_boundary = "m".repeat(64);
    let stamped = EventBuilder::new(
        Kind::Custom(30177),
        serde_json::json!({
            "name": "Bumble",
            "parallelism": 1,
            "respond_to": "anyone",
            "capabilities": [boundary.as_str(), "   ", "a".repeat(65).as_str()],
            "model": model_boundary.as_str(),
        })
        .to_string(),
    )
    .tags([Tag::parse(["d", agent_pubkey.as_str()]).expect("parse d tag")])
    .sign_with_keys(&owner_keys)
    .expect("sign managed-agent event");

    let agents = relay_agents_from_managed_agent_events(&[stamped], std::slice::from_ref(&profile));
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].capabilities,
        vec![boundary.as_str()],
        "64-char capability survives; whitespace-only and 65-char ones drop"
    );
    assert_eq!(
        agents[0].model.as_deref(),
        Some(model_boundary.as_str()),
        "64-char model survives the same boundary"
    );
}

/// REG-4 malformed-data edge case: a wrong-typed `capabilities` value (string
/// instead of array) fails the whole managed-agent content parse. The verified
/// coordinate still RESERVES the identity — nothing fabricated takes its
/// place — so the agent is hidden until its owner republishes a well-formed
/// record. This is the documented reservation rule for malformed current
/// policies, not a crash and not a fallback to stale data.
#[test]
fn managed_agent_directory_wrong_typed_capabilities_hides_the_agent_without_fabricating() {
    let agent_keys = Keys::generate();
    let owner_keys = Keys::generate();
    let agent_pubkey = agent_keys.public_key().to_hex();

    let auth_tag_json =
        buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "")
            .expect("compute auth tag");
    let auth_tag_values: Vec<String> =
        serde_json::from_str(&auth_tag_json).expect("parse auth tag json");
    let profile = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Bumble"}"#)
        .tags([Tag::parse(auth_tag_values).expect("parse auth tag")])
        .sign_with_keys(&agent_keys)
        .expect("sign profile");

    let wrong_typed = EventBuilder::new(
        Kind::Custom(30177),
        serde_json::json!({
            "name": "Bumble",
            "parallelism": 1,
            "respond_to": "anyone",
            "capabilities": "web-search",
        })
        .to_string(),
    )
    .tags([Tag::parse(["d", agent_pubkey.as_str()]).expect("parse d tag")])
    .sign_with_keys(&owner_keys)
    .expect("sign managed-agent event");

    let agents =
        relay_agents_from_managed_agent_events(&[wrong_typed], std::slice::from_ref(&profile));
    assert!(
        agents.is_empty(),
        "wrong-typed capabilities fails the content parse; the verified coordinate reserves the identity without fabricating a directory entry"
    );
}
