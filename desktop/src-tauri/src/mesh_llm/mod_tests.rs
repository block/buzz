//! Unit tests for `mesh_llm/mod.rs` private helpers (kept in a sibling file so
//! `mod.rs` stays under the 500-line budget; `#[path]`-included from there).
use super::find_progressish_reason;
use serde_json::json;

#[test]
fn progressish_reads_typed_phase_not_whole_tree() {
    assert_eq!(
        find_progressish_reason(&json!({"phase": "downloading weights"})),
        Some("downloading model".to_string())
    );
    // Regression (Sami N1): an unrelated field mentioning a progress word must
    // not trip the badge — only the typed phase field counts.
    assert_eq!(
        find_progressish_reason(&json!({
            "phase": "ready",
            "model_name": "prepared-qwen-preparing"
        })),
        None
    );
    assert_eq!(find_progressish_reason(&json!({"foo": "bar"})), None);
}

#[test]
fn sdk_ready_models_are_parsed_from_real_status_shape() {
    let payload = json!({
        "node_state": "serving",
        "llama_ready": true,
        "hosted_models": ["Qwen/Qwen3-0.6B-GGUF:Q8_0"],
        "serving_models": ["Qwen/Qwen3-0.6B-GGUF:Q8_0"],
        "runtime": {
            "models": [{
                "name": "Qwen/Qwen3-0.6B-GGUF:Q8_0",
                "status": "ready"
            }]
        }
    });

    assert_eq!(
        super::models_from_status_payload(Some(&payload)),
        vec![super::MeshModelOption {
            id: "Qwen/Qwen3-0.6B-GGUF:Q8_0".to_string(),
            name: None,
        }]
    );
    assert_eq!(
        super::node_state_from_payload(
            super::MeshNodeMode::Serve,
            &super::MeshHealth::ok(),
            &payload,
        ),
        super::MeshNodeState::Running
    );
}

#[test]
fn requested_model_is_not_ready_while_sdk_is_in_standby() {
    let payload = json!({
        "node_state": "standby",
        "llama_ready": false,
        "hosted_models": [],
        "serving_models": ["Qwen/Qwen3-0.6B-GGUF:Q8_0"],
        "runtime": {"models": []}
    });

    assert!(super::models_from_status_payload(Some(&payload)).is_empty());
    assert_eq!(
        super::node_state_from_payload(
            super::MeshNodeMode::Serve,
            &super::MeshHealth::ok(),
            &payload,
        ),
        super::MeshNodeState::Starting
    );
}

#[test]
fn iroh_relay_mode_defaults_to_enabled() {
    // Default is ON: unset, empty, "1", and "default" all enable the SDK's
    // default iroh relays, so members connect regardless of NAT. Relays are
    // transport-only (ciphertext forwarding) — admission is a separate layer.
    use super::IrohRelayMode;
    assert_eq!(
        super::iroh_relay_mode_from(None).unwrap(),
        IrohRelayMode::Default
    );
    assert_eq!(
        super::iroh_relay_mode_from(Some("")).unwrap(),
        IrohRelayMode::Default
    );
    assert_eq!(
        super::iroh_relay_mode_from(Some("  ")).unwrap(),
        IrohRelayMode::Default
    );
    assert_eq!(
        super::iroh_relay_mode_from(Some("1")).unwrap(),
        IrohRelayMode::Default
    );
    assert_eq!(
        super::iroh_relay_mode_from(Some("default")).unwrap(),
        IrohRelayMode::Default
    );
}

#[test]
fn iroh_relay_mode_opt_out_and_custom() {
    use super::IrohRelayMode;
    // "0" is the explicit opt-out for metadata-conscious deployments.
    assert_eq!(
        super::iroh_relay_mode_from(Some("0")).unwrap(),
        IrohRelayMode::Disabled
    );
    // Anything else is a comma-separated custom relay list.
    assert_eq!(
        super::iroh_relay_mode_from(Some("https://relay1.example, https://relay2.example ,"))
            .unwrap(),
        IrohRelayMode::Custom(vec![
            "https://relay1.example".parse().unwrap(),
            "https://relay2.example".parse().unwrap(),
        ])
    );
}

fn test_endpoint_token() -> String {
    super::transport_policy::endpoint_token_for_test([iroh::TransportAddr::Ip(
        "192.168.1.20:47916".parse().unwrap(),
    )])
}

fn add_test_owner_bindings(
    payload: &mut serde_json::Value,
    owner: &mesh_llm_host_runtime::crypto::OwnerKeypair,
    member_pubkey: &str,
) {
    let endpoints = super::identity::advertised_endpoint_tokens(payload).unwrap();
    payload["ownerBindingSig"] = serde_json::Value::String(hex::encode(
        owner.sign_bytes(&super::identity::member_binding_bytes(member_pubkey)),
    ));
    payload["ownerEndpointBindingSig"] = serde_json::Value::String(hex::encode(owner.sign_bytes(
        &super::identity::member_endpoint_binding_bytes(member_pubkey, &endpoints),
    )));
}

#[test]
fn normalized_roster_none_means_no_enforcement() {
    let identity = super::identity::OwnerIdentity {
        keystore_path: std::path::PathBuf::from("/tmp/ks.json"),
        owner_id: "owner-self".to_string(),
        verifying_key_hex: String::new(),
    };
    assert_eq!(super::normalized_roster(&None, &identity), None);
}

#[test]
fn normalized_roster_always_includes_self_and_dedupes() {
    let identity = super::identity::OwnerIdentity {
        keystore_path: std::path::PathBuf::from("/tmp/ks.json"),
        owner_id: "owner-self".to_string(),
        verifying_key_hex: String::new(),
    };
    // Empty roster (fresh relay, nobody published yet) still admits self —
    // otherwise the first sharer locks themselves out.
    assert_eq!(
        super::normalized_roster(&Some(vec![]), &identity),
        Some(vec!["owner-self".to_string()])
    );
    // Dedup + trim + sorted, self merged in.
    assert_eq!(
        super::normalized_roster(
            &Some(vec![
                "owner-b".to_string(),
                " owner-a ".to_string(),
                "owner-b".to_string(),
                "".to_string(),
                "owner-self".to_string(),
            ]),
            &identity
        ),
        Some(vec![
            "owner-a".to_string(),
            "owner-b".to_string(),
            "owner-self".to_string(),
        ])
    );
}

fn signed_reporter_status(reporter_secret: &str, _label: &str) -> nostr::Event {
    use mesh_llm_host_runtime::crypto::OwnerKeypair;

    let keys = nostr::Keys::parse(reporter_secret).expect("valid reporter secret");
    let owner = OwnerKeypair::generate();
    let member_pubkey = keys.public_key().to_hex();
    let mut payload = json!({
        "ownerId": owner.owner_id(),
        "ownerVerifyingKey": hex::encode(owner.verifying_key().as_bytes()),
        "serveTargets": []
    });
    add_test_owner_bindings(&mut payload, &owner, &member_pubkey);
    super::coordinator::build_status_report_event(payload)
        .expect("status builder")
        .sign_with_keys(&keys)
        .expect("test event signs")
}

fn signed_membership_event(members: &[String]) -> nostr::Event {
    let keys = nostr::Keys::generate();
    let tags = members
        .iter()
        .map(|member| nostr::Tag::parse(["member", member]).expect("valid member tag"))
        .collect::<Vec<_>>();
    nostr::EventBuilder::new(nostr::Kind::Custom(13_534), "")
        .tags(tags)
        .sign_with_keys(&keys)
        .expect("test membership event signs")
}

#[test]
fn has_membership_snapshot_distinguishes_empty_from_missing() {
    // A zero-member community still publishes an explicit kind:13534 event, so
    // its presence — not the member count — is what makes an empty roster
    // authoritative. No snapshot at all means the query was incomplete.
    let zero_member_snapshot = signed_membership_event(&[]);
    assert!(
        super::has_membership_snapshot(std::slice::from_ref(&zero_member_snapshot)),
        "an explicit zero-member snapshot counts as present"
    );

    let member = nostr::Keys::parse(&"1".repeat(64))
        .unwrap()
        .public_key()
        .to_hex();
    let populated = signed_membership_event(std::slice::from_ref(&member));
    assert!(super::has_membership_snapshot(std::slice::from_ref(
        &populated
    )));

    // A response with only status events (or nothing) has no snapshot.
    assert!(!super::has_membership_snapshot(&[]));
    let status_only = signed_reporter_status(&"2".repeat(64), "owner-x");
    assert!(!super::has_membership_snapshot(std::slice::from_ref(
        &status_only
    )));
}

#[test]
fn owner_ids_from_events_collects_sorted_deduped_roster() {
    let secret_a = "1".repeat(64);
    let secret_b = "2".repeat(64);
    let member_a = nostr::Keys::parse(&secret_a).unwrap().public_key().to_hex();
    let member_b = nostr::Keys::parse(&secret_b).unwrap().public_key().to_hex();
    let events = vec![
        signed_reporter_status(&secret_b, "owner-b"),
        signed_reporter_status(&secret_a, "owner-a"),
        signed_membership_event(&[member_a, member_b]),
    ];
    let owners = super::owner_ids_from_events(&events);
    assert_eq!(owners.len(), 2);
    assert!(owners.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(super::owner_ids_from_events(&[]), Vec::<String>::new());
}

#[test]
fn owner_roster_intersects_status_reporters_with_current_members() {
    let current_secret = "1".repeat(64);
    let removed_secret = "2".repeat(64);
    let nonmember_secret = "3".repeat(64);
    let current_member = nostr::Keys::parse(&current_secret)
        .unwrap()
        .public_key()
        .to_hex();
    let events = vec![
        signed_reporter_status(&current_secret, "owner-current"),
        signed_reporter_status(&removed_secret, "owner-removed"),
        signed_reporter_status(&nonmember_secret, "owner-nonmember"),
        signed_membership_event(std::slice::from_ref(&current_member)),
    ];

    assert_eq!(super::owner_ids_from_events(&events).len(), 1);
}

#[test]
fn owner_roster_rejects_spoofed_owner_id_and_cross_member_binding() {
    use mesh_llm_host_runtime::crypto::OwnerKeypair;

    let member_secret = "4".repeat(64);
    let other_secret = "5".repeat(64);
    let member_keys = nostr::Keys::parse(&member_secret).unwrap();
    let other_keys = nostr::Keys::parse(&other_secret).unwrap();
    let member_pubkey = member_keys.public_key().to_hex();
    let owner = OwnerKeypair::generate();
    let verifying_key = hex::encode(owner.verifying_key().as_bytes());
    let endpoint = test_endpoint_token();

    let sign_status = |owner_id: String, binding_pubkey: &str| {
        let mut payload = json!({
            "ownerId": owner_id,
            "ownerVerifyingKey": verifying_key.clone(),
            "serveTargets": [{
                "modelId": "spoofed-model",
                "modelName": null,
                "endpointAddr": endpoint.clone(),
                "nodeName": null,
                "capacity": null
            }]
        });
        add_test_owner_bindings(&mut payload, &owner, binding_pubkey);
        super::coordinator::build_status_report_event(payload)
            .unwrap()
            .sign_with_keys(&member_keys)
            .unwrap()
    };

    let spoofed_owner = sign_status("0".repeat(64), &member_pubkey);
    let cross_member_binding = sign_status(owner.owner_id(), &other_keys.public_key().to_hex());
    let membership = signed_membership_event(std::slice::from_ref(&member_pubkey));

    let events = vec![spoofed_owner, cross_member_binding, membership];
    assert!(
        super::owner_ids_from_events(&events).is_empty(),
        "a Buzz member must not be able to advertise an unproven MeshLLM owner identity"
    );
    assert!(
        super::availability_from_events(events)
            .serve_targets
            .is_empty(),
        "an unproven owner identity must not contribute a selectable target"
    );
}

#[test]
fn owner_roster_rejects_endpoint_substitution_without_owner_signature() {
    use mesh_llm_host_runtime::crypto::OwnerKeypair;

    let keys = nostr::Keys::parse(&"b".repeat(64)).unwrap();
    let member_pubkey = keys.public_key().to_hex();
    let owner = OwnerKeypair::generate();
    let signed_endpoint = test_endpoint_token();
    let substituted_endpoint = test_endpoint_token();
    let mut payload = json!({
        "ownerId": owner.owner_id(),
        "ownerVerifyingKey": hex::encode(owner.verifying_key().as_bytes()),
        "serveTargets": [{
            "modelId": "model-a",
            "modelName": null,
            "endpointAddr": signed_endpoint,
            "nodeName": null,
            "capacity": null
        }]
    });
    add_test_owner_bindings(&mut payload, &owner, &member_pubkey);
    payload["serveTargets"][0]["endpointAddr"] = serde_json::Value::String(substituted_endpoint);
    let status = super::coordinator::build_status_report_event(payload)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    let membership = signed_membership_event(std::slice::from_ref(&member_pubkey));

    assert_eq!(
        super::owner_ids_from_events(&[status.clone(), membership.clone()]).len(),
        1,
        "endpoint substitution must not erase a valid member-to-owner admission binding"
    );
    assert!(super::availability_from_events(vec![status, membership])
        .serve_targets
        .is_empty());
}

#[test]
fn availability_excludes_removed_member_status() {
    let current_secret = "6".repeat(64);
    let removed_secret = "7".repeat(64);
    let current_member = nostr::Keys::parse(&current_secret)
        .unwrap()
        .public_key()
        .to_hex();
    let current_endpoint = test_endpoint_token();
    let events = vec![
        signed_reporter_target(&current_secret, "model-current", &current_endpoint),
        signed_reporter_target(&removed_secret, "model-removed", &test_endpoint_token()),
        signed_membership_event(std::slice::from_ref(&current_member)),
    ];

    let availability = super::availability_from_events(events);
    assert_eq!(availability.models.len(), 1);
    assert_eq!(availability.models[0].id, "model-current");
    assert_eq!(availability.serve_targets.len(), 1);
    assert_eq!(
        availability.serve_targets[0].endpoint_addr,
        current_endpoint
    );
}

fn signed_reporter_target(reporter_secret: &str, model: &str, endpoint: &str) -> nostr::Event {
    use mesh_llm_host_runtime::crypto::OwnerKeypair;

    let keys = nostr::Keys::parse(reporter_secret).unwrap();
    let owner = OwnerKeypair::generate();
    let member_pubkey = keys.public_key().to_hex();
    let mut payload = json!({
        "ownerId": owner.owner_id(),
        "ownerVerifyingKey": hex::encode(owner.verifying_key().as_bytes()),
        "models": [{"id": model, "name": null}],
        "serveTargets": [{
            "modelId": model,
            "modelName": null,
            "endpointAddr": endpoint,
            "nodeName": null,
            "capacity": null
        }]
    });
    add_test_owner_bindings(&mut payload, &owner, &member_pubkey);
    super::coordinator::build_status_report_event(payload)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap()
}

#[test]
fn stale_status_keeps_member_admitted_but_excluded_from_routing() {
    let secret = "8".repeat(64);
    let member = nostr::Keys::parse(&secret).unwrap().public_key().to_hex();
    let stale = signed_reporter_target_at(&secret, "stale-model", &test_endpoint_token(), 1_000);
    // Membership is the trust boundary: a current member whose device went
    // offline (stale status) must stay admitted, otherwise every app
    // open/close in the community churns the allowlist and restarts serving
    // nodes. Freshness still gates routing: a stale node is never selected as
    // a serve target.
    let membership = signed_membership_event_at(std::slice::from_ref(&member), 900);
    let events = vec![stale, membership];

    assert_eq!(super::owner_ids_from_events(&events).len(), 1);
    let availability = super::availability_from_events(events);
    assert!(availability.serve_targets.is_empty());
}

#[test]
fn removed_member_is_dropped_from_admission_despite_fresh_status() {
    // Revocation path: freshness must never resurrect trust. A reporter with a
    // perfectly fresh status who is absent from the latest NIP-43 roster gets
    // no admission entry.
    let member_secret = "8".repeat(64);
    let outsider_secret = "9".repeat(64);
    let member = nostr::Keys::parse(&member_secret)
        .unwrap()
        .public_key()
        .to_hex();
    let now = nostr::Timestamp::now().as_secs();
    let fresh_outsider =
        signed_reporter_target_at(&outsider_secret, "model", &test_endpoint_token(), now);
    // Latest roster lists only `member`; the outsider was removed (or never
    // admitted).
    let membership = signed_membership_event_at(std::slice::from_ref(&member), now);
    let events = vec![fresh_outsider, membership];

    assert!(super::owner_ids_from_events(&events).is_empty());
}

#[test]
fn one_endpoint_can_advertise_multiple_models() {
    let secret = "a".repeat(64);
    let member = nostr::Keys::parse(&secret).unwrap().public_key().to_hex();
    let endpoint = test_endpoint_token();
    let availability = super::availability_from_events(vec![
        signed_reporter_target(&secret, "model-a", &endpoint),
        signed_reporter_target(&secret, "model-b", &endpoint),
        signed_membership_event(std::slice::from_ref(&member)),
    ]);

    assert_eq!(availability.serve_targets.len(), 2);
    assert!(availability
        .serve_targets
        .iter()
        .any(|target| target.model_id == "model-a"));
    assert!(availability
        .serve_targets
        .iter()
        .any(|target| target.model_id == "model-b"));
}

#[test]
fn same_member_can_publish_multiple_owner_scoped_devices() {
    let secret = "9".repeat(64);
    let member = nostr::Keys::parse(&secret).unwrap().public_key().to_hex();
    let first = signed_reporter_target(&secret, "model-a", &test_endpoint_token());
    let second = signed_reporter_target(&secret, "model-b", &test_endpoint_token());
    let first_d = first
        .tags
        .iter()
        .find_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("d"))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .unwrap();
    let second_d = second
        .tags
        .iter()
        .find_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("d"))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .unwrap();
    assert_ne!(
        first_d, second_d,
        "device status coordinates must not overwrite"
    );

    let availability = super::availability_from_events(vec![
        first,
        second,
        signed_membership_event(std::slice::from_ref(&member)),
    ]);
    assert_eq!(availability.serve_targets.len(), 2);
}

fn signed_reporter_target_at(
    reporter_secret: &str,
    model: &str,
    endpoint: &str,
    created_at: u64,
) -> nostr::Event {
    use mesh_llm_host_runtime::crypto::OwnerKeypair;

    let keys = nostr::Keys::parse(reporter_secret).unwrap();
    let owner = OwnerKeypair::generate();
    let member_pubkey = keys.public_key().to_hex();
    let mut payload = json!({
        "ownerId": owner.owner_id(),
        "ownerVerifyingKey": hex::encode(owner.verifying_key().as_bytes()),
        "models": [{"id": model, "name": null}],
        "serveTargets": [{
            "modelId": model,
            "modelName": null,
            "endpointAddr": endpoint,
            "nodeName": null,
            "capacity": null
        }]
    });
    add_test_owner_bindings(&mut payload, &owner, &member_pubkey);
    super::coordinator::build_status_report_event(payload)
        .unwrap()
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(&keys)
        .unwrap()
}

fn signed_membership_event_at(members: &[String], created_at: u64) -> nostr::Event {
    let keys = nostr::Keys::generate();
    let tags = members
        .iter()
        .map(|member| nostr::Tag::parse(["member", member]).unwrap())
        .collect::<Vec<_>>();
    nostr::EventBuilder::new(nostr::Kind::Custom(13_534), "")
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(&keys)
        .unwrap()
}

#[test]
fn owner_roster_without_membership_list_fails_closed() {
    let events = vec![
        signed_reporter_status(&"1".repeat(64), "owner-a"),
        signed_reporter_status(&"2".repeat(64), "owner-b"),
    ];

    assert!(super::owner_ids_from_events(&events).is_empty());
}
