use buzz_core::cml::{derive_host_id, parse_cml, CmlStatus};

fn valid_cml() -> String {
    serde_json::json!({
        "acceptance": [{"id":"A1","text":"Persisted state round-trips","verified":false}],
        "blockers": [],
        "evidence": [],
        "git": {
            "base_sha": "1111111111111111111111111111111111111111",
            "branch": "feat/cml",
            "head_sha": null,
            "repo": "block/buzz",
            "worktree_alias": "buzz-cml"
        },
        "id": "cdd4722d-7481-4d01-9c0a-423b4454c179",
        "lease": null,
        "objective": "One testable outcome",
        "priority": "P1",
        "protocol": "buzz-cml",
        "review": {"max_rounds":3,"round":0},
        "roles": {"fixer":null,"planner":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","reviewer":null,"worker":null},
        "runtime": {"host_id":null,"last_heartbeat_at":null,"presence":"offline","ttl_seconds":180},
        "status": "proposed",
        "title": "CML core",
        "updated_at": 1787673000,
        "version": 1
    }).to_string()
}

#[test]
fn valid_document_round_trips_to_stable_canonical_bytes() {
    let parsed = parse_cml(&valid_cml()).expect("valid CML");
    assert_eq!(parsed.status, CmlStatus::Proposed);
    let first = parsed.to_canonical_json().expect("serialize");
    let second = parse_cml(&first)
        .expect("canonical form parses")
        .to_canonical_json()
        .expect("reserialize");
    assert_eq!(first, second);
    assert!(first.ends_with('\n'));
    assert!(first.find("\"acceptance\"") < first.find("\"version\""));
}

#[test]
fn unknown_fields_fail_closed() {
    let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
    value["surprise"] = serde_json::json!(true);
    let error = parse_cml(&value.to_string()).unwrap_err().to_string();
    assert!(error.contains("unknown field"), "{error}");
}

#[test]
fn privacy_leaking_runtime_and_worktree_values_are_rejected() {
    let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
    value["runtime"]["host_id"] = serde_json::json!("192.168.1.10");
    assert!(parse_cml(&value.to_string()).is_err());

    value["runtime"]["host_id"] = serde_json::json!("h_0123456789abcdef");
    value["git"]["worktree_alias"] = serde_json::json!("/Users/alice/private/repo");
    assert!(parse_cml(&value.to_string()).is_err());
}

#[test]
fn reviewer_must_be_distinct_and_fourth_round_is_rejected() {
    let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
    let actor = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    value["roles"]["worker"] = serde_json::json!(actor);
    value["roles"]["reviewer"] = serde_json::json!(actor);
    assert!(parse_cml(&value.to_string()).is_err());

    value["roles"]["reviewer"] =
        serde_json::json!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
    value["review"]["round"] = serde_json::json!(4);
    assert!(parse_cml(&value.to_string()).is_err());
}

#[test]
fn host_ids_are_stable_within_scope_and_unlinkable_across_scope_or_agent() {
    let secret = [7_u8; 32];
    let community = uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
    let channel_a = uuid::Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
    let channel_b = uuid::Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
    let agent_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let agent_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    let first = derive_host_id(&secret, community, channel_a, agent_a).unwrap();
    assert_eq!(
        first,
        derive_host_id(&secret, community, channel_a, agent_a).unwrap()
    );
    assert_ne!(
        first,
        derive_host_id(&secret, community, channel_b, agent_a).unwrap()
    );
    assert_ne!(
        first,
        derive_host_id(&secret, community, channel_a, agent_b).unwrap()
    );
    assert!(first.starts_with("h_") && first.len() == 18);
    assert!(!first.contains(agent_a));
}

#[test]
fn duplicate_acceptance_ids_and_noncanonical_shas_are_rejected() {
    let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
    value["acceptance"] = serde_json::json!([
        {"id":"A1","text":"first","verified":false},
        {"id":"A1","text":"second","verified":false}
    ]);
    assert!(parse_cml(&value.to_string()).is_err());

    value["acceptance"] = serde_json::json!([]);
    value["git"]["base_sha"] = serde_json::json!("ABCDEF");
    assert!(parse_cml(&value.to_string()).is_err());
}

#[test]
fn presence_must_be_derived_at_snapshot_time() {
    let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
    value["runtime"]["presence"] = serde_json::json!("online");
    assert!(parse_cml(&value.to_string()).is_err());

    value["runtime"]["last_heartbeat_at"] = serde_json::json!(1787672990_u64);
    assert!(parse_cml(&value.to_string()).is_ok());

    value["runtime"]["presence"] = serde_json::json!("stale");
    assert!(parse_cml(&value.to_string()).is_err());
}

#[test]
fn status_lease_and_holder_must_be_coherent() {
    let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
    value["status"] = serde_json::json!("claimed");
    assert!(parse_cml(&value.to_string()).is_err());

    let worker = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    value["roles"]["worker"] = serde_json::json!(worker);
    value["lease"] = serde_json::json!({
        "id":"lease-1",
        "holder":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "issued_at":1787672900_u64,
        "expires_at":1787673900_u64
    });
    assert!(parse_cml(&value.to_string()).is_err());

    value["lease"]["holder"] = serde_json::json!(worker);
    assert!(parse_cml(&value.to_string()).is_ok());

    value["status"] = serde_json::json!("proposed");
    assert!(parse_cml(&value.to_string()).is_err());
}

#[test]
fn git_identifiers_and_evidence_reject_path_and_url_injection() {
    for branch in ["feat:x", "a~b", "a b", ".hidden/x", "x.lock", "a//b", "@{x"] {
        let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
        value["git"]["branch"] = serde_json::json!(branch);
        assert!(parse_cml(&value.to_string()).is_err(), "branch {branch}");
    }
    for repo in ["owner\\..\\secrets/name", "./name", "owner/."] {
        let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
        value["git"]["repo"] = serde_json::json!(repo);
        assert!(parse_cml(&value.to_string()).is_err(), "repo {repo}");
    }
    for reference in [
        "file:///etc/shadow",
        "http://169.254.169.254/latest/meta-data",
    ] {
        let mut value: serde_json::Value = serde_json::from_str(&valid_cml()).unwrap();
        value["evidence"] = serde_json::json!([{"kind":"runtime","reference":reference}]);
        assert!(
            parse_cml(&value.to_string()).is_err(),
            "reference {reference}"
        );
    }
}
