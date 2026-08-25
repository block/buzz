//! The exact-key-set tables that make the no-secret invariant enforceable.

use super::*;

// ── Wire schemas: the enforceable no-secret invariant ───────────────────────

/// The exact wire key set of every args and outcome type, with every optional
/// field populated so nothing escapes the pin by being absent — plus the two
/// envelopes, whose own key sets are now equally enforceable.
///
/// This table *is* the no-secret invariant. Combined with
/// `deny_unknown_fields`, it means no field — secret-bearing or otherwise — can
/// be added to this contract without a reviewer changing a line here. The
/// `agents.create` outcome is the case that matters: public identity only, never
/// the key the host just minted.
///
/// The envelopes are here because a key set nobody pins is a key set a field can
/// be added to. The response envelope in particular admits a *different* exact
/// set per status, which is what its strict deserializer enforces.
#[test]
fn every_payload_has_an_exact_and_secret_free_wire_schema() {
    let signer = Keys::generate();
    let expected: Vec<(&str, Vec<&str>)> = vec![
        // Envelopes. Every optional member present, so the pin covers the
        // widest shape each may take.
        (
            "request/envelope",
            vec![
                "action",
                "actionVersion",
                "args",
                "protocolVersion",
                "requestId",
                "type",
            ],
        ),
        (
            "response/envelope/succeeded",
            vec![
                "action",
                "outcome",
                "protocolVersion",
                "replayed",
                "requestId",
                "status",
                "type",
            ],
        ),
        (
            "response/envelope/failed",
            vec![
                "error",
                "protocolVersion",
                "replayed",
                "requestId",
                "status",
                "type",
            ],
        ),
        (
            "response/envelope/indeterminate",
            vec![
                "error",
                "protocolVersion",
                "replayed",
                "requestId",
                "status",
                "type",
            ],
        ),
        ("error", vec!["code", "message"]),
        // Args, fully populated (optional fields present).
        (
            "channel.read/args",
            vec![
                "channelId",
                "cursor",
                "limit",
                "mentionsOnly",
                "rootEventId",
            ],
        ),
        (
            "message.post/args",
            vec!["channelId", "content", "mentions"],
        ),
        (
            "message.reply/args",
            vec!["channelId", "content", "mentions", "replyToEventId"],
        ),
        (
            "reaction.add/args",
            vec!["channelId", "reaction", "targetEventId"],
        ),
        ("profile.set/args", vec!["about", "displayName", "picture"]),
        ("storage.address/args", vec!["slug"]),
        (
            "agents.create/args",
            vec![
                "channelId",
                "displayName",
                "model",
                "provider",
                "respondTo",
                "runtime",
                "systemPrompt",
            ],
        ),
        (
            "agents.update/args",
            vec![
                "displayName",
                "model",
                "provider",
                "respondTo",
                "runtime",
                "systemPrompt",
                "target",
            ],
        ),
        ("agents.delete/args", vec!["target"]),
        // Outcomes.
        ("channel.read/outcome", vec!["messages", "nextCursor"]),
        ("message.post/outcome", vec!["createdAt", "eventId", "kind"]),
        (
            "message.reply/outcome",
            vec!["createdAt", "eventId", "kind"],
        ),
        ("reaction.add/outcome", vec!["createdAt", "eventId", "kind"]),
        ("profile.set/outcome", vec!["createdAt", "eventId", "kind"]),
        (
            "storage.address/outcome",
            vec!["authorPubkey", "dTag", "kind"],
        ),
        (
            "agents.create/outcome",
            vec!["agentPubkey", "channelId", "displayName"],
        ),
        (
            "agents.update/outcome",
            vec!["agentPubkey", "displayName", "updatedFields"],
        ),
        ("agents.delete/outcome", vec!["agentPubkey", "displayName"]),
    ];

    let mut actual: Vec<(String, Vec<String>)> = Vec::new();
    // Envelopes first, in the same order as the table above. `replayed` is set
    // so the widest shape is what gets pinned.
    let request = BrokerRequest::new(
        "req-1",
        ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)),
    )
    .expect("envelope fixture builds");
    actual.push((
        "request/envelope".to_string(),
        keys_of(&serde_json::to_value(&request).expect("request serializes")),
    ));
    for (name, result) in [
        (
            "succeeded",
            BrokerResult::succeeded(ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
                agent_pubkey: pubkey(),
                display_name: "Gone".into(),
            })),
        ),
        (
            "failed",
            BrokerResult::failed(BrokerError::new(BrokerErrorCode::ActionFailed, "no")),
        ),
        (
            "indeterminate",
            BrokerResult::indeterminate(BrokerError::new(BrokerErrorCode::OutcomeUnknown, "?")),
        ),
    ] {
        let response = BrokerResponse::new("req-1", result).replayed();
        actual.push((
            format!("response/envelope/{name}"),
            keys_of(&serde_json::to_value(&response).expect("response serializes")),
        ));
    }
    actual.push((
        "error".to_string(),
        keys_of(
            &serde_json::to_value(BrokerError::new(BrokerErrorCode::Internal, "?"))
                .expect("error serializes"),
        ),
    ));
    for args in action_fixtures() {
        let json = serde_json::to_value(&args).expect("args serialize");
        actual.push((
            format!("{}/args", args.action().as_str()),
            keys_of(&json["args"]),
        ));
    }
    for outcome in outcome_fixtures(&signer) {
        let json = serde_json::to_value(&outcome).expect("outcome serializes");
        actual.push((
            format!("{}/outcome", outcome.action().as_str()),
            keys_of(&json["outcome"]),
        ));
    }

    let expected: Vec<(String, Vec<String>)> = expected
        .into_iter()
        .map(|(name, keys)| {
            (
                name.to_string(),
                keys.into_iter().map(str::to_string).collect(),
            )
        })
        .collect();
    assert_eq!(
        actual, expected,
        "a payload's wire keys changed — confirm no field can carry key material"
    );

    // And no key anywhere in the contract even *looks* like secret material.
    for (name, keys) in &actual {
        for key in keys {
            let lower = key.to_ascii_lowercase();
            for forbidden in ["secret", "private", "nsec", "seckey", "credential", "token"] {
                assert!(
                    !lower.contains(forbidden),
                    "{name} exposes \"{key}\", which reads as secret material"
                );
            }
        }
    }
}

/// The envelope must not carry requester, owner, or scope: those are derived by
/// the host from the credential. A body that could name its own subject would
/// let any caller act as anyone — the same reason `agents.create` has no owner.
#[test]
fn no_payload_can_name_its_own_authority() {
    let request = serde_json::to_value(
        BrokerRequest::new(
            "req-1",
            ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        keys_of(&request),
        vec![
            "action",
            "actionVersion",
            "args",
            "protocolVersion",
            "requestId",
            "type"
        ]
    );

    // Authority-naming fields are rejected, not ignored, wherever they appear.
    for rejected in [
        serde_json::json!({
            "channelId": CHANNEL, "displayName": "A", "systemPrompt": "B",
            "ownerPubkey": PUBKEY,
        }),
        serde_json::json!({
            "channelId": CHANNEL, "displayName": "A", "systemPrompt": "B",
            "onBehalfOf": PUBKEY,
        }),
        serde_json::json!({
            "channelId": CHANNEL, "displayName": "A", "systemPrompt": "B",
            "envVars": { "ANTHROPIC_API_KEY": "sk-live" },
        }),
        serde_json::json!({
            "channelId": CHANNEL, "displayName": "A", "systemPrompt": "B",
            "secretKey": "nsec1deadbeef",
        }),
    ] {
        assert!(
            serde_json::from_value::<AgentsCreateArgs>(rejected.clone()).is_err(),
            "must reject: {rejected}"
        );
    }

    // A read cannot ask about someone else's mentions, and a profile write
    // cannot name a subject.
    assert!(serde_json::from_value::<ChannelReadArgs>(
        serde_json::json!({ "channelId": CHANNEL, "mentionsOf": PUBKEY })
    )
    .is_err());
    assert!(serde_json::from_value::<ProfileSetArgs>(
        serde_json::json!({ "displayName": "A", "pubkey": PUBKEY })
    )
    .is_err());

    // An outcome cannot smuggle a minted secret past the schema either.
    for extra in ["nsec", "secretKey", "seckey", "credential"] {
        let mut outcome = serde_json::json!({
            "agentPubkey": PUBKEY, "displayName": "A", "channelId": CHANNEL,
        });
        outcome[extra] = serde_json::json!("nsec1deadbeef");
        let json = serde_json::json!({ "action": "agents.create", "outcome": outcome });
        assert!(
            serde_json::from_value::<ActionOutcome>(json).is_err(),
            "an outcome carrying \"{extra}\" must not deserialize"
        );
    }
}

/// The nested action enums are strict about their own key set, not just about
/// the payload inside it.
///
/// `ActionArgs`/`ActionOutcome` are adjacently tagged, so their wire form is the
/// two-key object `{action, args}` / `{action, outcome}`. Without
/// `deny_unknown_fields` on the enum itself, a *sibling* of those two keys is
/// silently ignored — and these types are public and wire-facing, so a host
/// author can deserialize one directly rather than through the envelope. The
/// envelope's own strictness does not cover that door.
#[test]
fn a_nested_action_object_rejects_siblings_of_its_two_keys() {
    // The valid two-key forms must pass untouched, so a rejection below cannot
    // be a rejection of the fixture itself.
    let args = serde_json::json!({
        "action": "agents.delete", "args": { "target": { "name": "helper" } },
    });
    let outcome = serde_json::json!({
        "action": "agents.delete",
        "outcome": { "agentPubkey": PUBKEY, "displayName": "Gone" },
    });
    serde_json::from_value::<ActionArgs>(args.clone()).expect("the exact args shape deserializes");
    serde_json::from_value::<ActionOutcome>(outcome.clone())
        .expect("the exact outcome shape deserializes");

    for extra in ["secretKey", "nsec", "outcome", "unexpected"] {
        let mut probe = args.clone();
        probe[extra] = serde_json::json!("x");
        assert!(
            serde_json::from_value::<ActionArgs>(probe).is_err(),
            "ActionArgs must reject the sibling key \"{extra}\""
        );
    }
    for extra in ["secretKey", "nsec", "args", "unexpected"] {
        let mut probe = outcome.clone();
        probe[extra] = serde_json::json!("x");
        assert!(
            serde_json::from_value::<ActionOutcome>(probe).is_err(),
            "ActionOutcome must reject the sibling key \"{extra}\""
        );
    }
}

#[test]
fn pubkey_hex_rejects_anything_but_a_public_key() {
    assert!(PubkeyHex::parse("nothex").is_err());
    assert!(PubkeyHex::parse(&PUBKEY[..40]).is_err());
    assert!(PubkeyHex::parse(format!("{PUBKEY}00")).is_err());
    assert!(PubkeyHex::parse("nsec1deadbeef").is_err());
    // Normalizes case, so two spellings of one key cannot look like two keys.
    assert_eq!(
        PubkeyHex::parse(PUBKEY.to_ascii_uppercase()).unwrap(),
        pubkey()
    );
    // And it enforces that through serde, not only through the constructor.
    assert!(serde_json::from_value::<PubkeyHex>(serde_json::json!("nothex")).is_err());
}
