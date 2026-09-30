//! Contract tests for the broker envelope, actions, and client trait.

use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag};

const CHANNEL: &str = "b2c38ca8-9ec3-411e-bab5-f9deab34d52e";
const PUBKEY: &str = "a02c4e0850e5e612b4ddf95dbe2f5c56467cf27c6552203bc833ff438fb31971";
const EVENT: &str = "78d47c4f36a2d048f45b57a31d964a3ce239f0fc46162c5d7c90db2b5aa52bc6";

fn pubkey() -> PubkeyHex {
    PubkeyHex::parse(PUBKEY).expect("fixture pubkey is valid hex")
}

/// A genuinely signed event, so read fixtures exercise real verification rather
/// than a hand-built value that could never verify.
fn signed_message(keys: &Keys) -> BrokerMessage {
    let event = EventBuilder::new(Kind::Custom(9), "hello")
        .tags([
            Tag::parse(["h", CHANNEL]).expect("h tag"),
            Tag::parse(["e", EVENT, "", "root"]).expect("e tag"),
            Tag::parse(["p", PUBKEY]).expect("p tag"),
        ])
        .sign_with_keys(keys)
        .expect("fixture event signs");
    BrokerMessage(event)
}

/// Every [`BrokerErrorCode`] variant, so code-driven tables cannot silently skip
/// one: [`error_codes_have_stable_wire_strings`] pins this list against the enum.
fn all_error_codes() -> [BrokerErrorCode; 11] {
    use BrokerErrorCode as E;
    [
        E::InvalidRequest,
        E::UnsupportedProtocolVersion,
        E::UnknownAction,
        E::UnsupportedActionVersion,
        E::Unsupported,
        E::Unauthenticated,
        E::Unauthorized,
        E::RequestIdConflict,
        E::ActionFailed,
        E::OutcomeUnknown,
        E::Internal,
    ]
}

/// One valid `args` value per action, so table-driven tests cannot silently
/// skip an action: [`fixtures_cover_every_action`] pins the coverage.
fn action_fixtures() -> Vec<ActionArgs> {
    vec![
        ActionArgs::ChannelRead(ChannelReadArgs {
            channel_id: CHANNEL.into(),
            root_event_id: Some(EVENT.into()),
            mentions_only: true,
            cursor: Some("opaque-host-cursor-v1".into()),
            limit: Some(50),
        }),
        ActionArgs::MessagePost(MessagePostArgs {
            channel_id: CHANNEL.into(),
            content: "shipping the contract".into(),
            mentions: vec![pubkey()],
        }),
        ActionArgs::MessageReply(MessageReplyArgs {
            channel_id: CHANNEL.into(),
            reply_to_event_id: EVENT.into(),
            content: "agreed".into(),
            mentions: vec![pubkey()],
        }),
        ActionArgs::ReactionAdd(ReactionAddArgs {
            channel_id: CHANNEL.into(),
            target_event_id: EVENT.into(),
            reaction: "🎉".into(),
        }),
        ActionArgs::ProfileSet(ProfileSetArgs {
            display_name: Some("ss-dev-00".into()),
            about: Some("implementation".into()),
            picture: Some("https://example.invalid/avatar.png".into()),
        }),
        ActionArgs::StorageAddress(StorageAddressArgs {
            slug: "mem/broker-foundation".into(),
        }),
        ActionArgs::AgentsCreate(AgentsCreateArgs {
            channel_id: CHANNEL.into(),
            display_name: "Research helper".into(),
            system_prompt: "Find sources.".into(),
            runtime: Some("buzz-acp".into()),
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet-4-5".into()),
            respond_to: Some("owner-only".into()),
        }),
        ActionArgs::AgentsUpdate(AgentsUpdateArgs {
            target: AgentTarget::Pubkey(pubkey()),
            display_name: Some("Research helper v2".into()),
            system_prompt: Some("Find better sources.".into()),
            runtime: Some("buzz-acp".into()),
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet-4-5".into()),
            respond_to: Some("anyone".into()),
        }),
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Name("Research helper".into()),
        }),
    ]
}

/// One outcome per action, matching the fixture order above.
fn outcome_fixtures(keys: &Keys) -> Vec<ActionOutcome> {
    let page = MessagePage {
        messages: vec![signed_message(keys)],
        next_cursor: Some("opaque-host-cursor-v2".into()),
    };
    let published = EventPublished {
        event_id: EVENT.into(),
        kind: 9,
        created_at: 1_764_000_003,
    };
    vec![
        ActionOutcome::ChannelRead(page),
        ActionOutcome::MessagePost(published.clone()),
        ActionOutcome::MessageReply(published.clone()),
        ActionOutcome::ReactionAdd(published.clone()),
        ActionOutcome::ProfileSet(published),
        ActionOutcome::StorageAddress(StorageAddress {
            author_pubkey: pubkey(),
            kind: 30174,
            d_tag: EVENT.into(),
        }),
        ActionOutcome::AgentsCreate(AgentsCreateOutcome {
            agent_pubkey: pubkey(),
            display_name: "Research helper".into(),
            channel_id: CHANNEL.into(),
        }),
        ActionOutcome::AgentsUpdate(AgentsUpdateOutcome {
            agent_pubkey: pubkey(),
            display_name: "Research helper v2".into(),
            updated_fields: vec!["displayName".into()],
        }),
        ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
            agent_pubkey: pubkey(),
            display_name: "Research helper".into(),
        }),
    ]
}

fn prepared(args: ActionArgs) -> PreparedRequest {
    BrokerRequest::new("req-1", args)
        .expect("fixture request builds")
        .prepare()
        .expect("fixture request prepares")
}

/// Sorted JSON object keys of `value`, for exact-schema assertions.
fn keys_of(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("expected a JSON object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

// ── Coverage ────────────────────────────────────────────────────────────────

/// The fixture tables are the input to every table-driven test below, so an
/// action added without a fixture would be silently untested. This is the guard.
#[test]
fn fixtures_cover_every_action() {
    let keys = Keys::generate();
    let mut from_args: Vec<&str> = action_fixtures()
        .iter()
        .map(|args| args.action().as_str())
        .collect();
    let mut from_outcomes: Vec<&str> = outcome_fixtures(&keys)
        .iter()
        .map(|outcome| outcome.action().as_str())
        .collect();
    let mut declared: Vec<&str> = Action::ALL.iter().map(|a| a.as_str()).collect();

    from_args.sort_unstable();
    from_outcomes.sort_unstable();
    declared.sort_unstable();

    assert_eq!(from_args, declared, "every action needs an args fixture");
    assert_eq!(
        from_outcomes, declared,
        "every action needs an outcome fixture"
    );

    let mut unique = declared.clone();
    unique.dedup();
    assert_eq!(unique.len(), declared.len(), "wire names must be unique");
}

// ── Envelope round-trip ─────────────────────────────────────────────────────

#[test]
fn every_action_round_trips_through_a_request_envelope() {
    for args in action_fixtures() {
        let action = args.action();
        let request = BrokerRequest::new("req-1", args)
            .unwrap_or_else(|e| panic!("{} fixture must validate: {e}", action.as_str()));

        let json = serde_json::to_value(&request).expect("request serializes");
        assert_eq!(json["type"], BROKER_REQUEST_TYPE);
        assert_eq!(json["protocolVersion"], 1);
        assert_eq!(json["requestId"], "req-1");
        assert_eq!(json["actionVersion"], 1);
        assert_eq!(
            json["action"],
            action.as_str(),
            "{} must name itself on the wire",
            action.as_str()
        );
        assert!(
            json.get("args").is_some(),
            "{} must carry an args object",
            action.as_str()
        );

        let parsed: BrokerRequest = serde_json::from_value(json)
            .unwrap_or_else(|e| panic!("{} must deserialize: {e}", action.as_str()));
        assert_eq!(parsed, request);
        parsed.validated().expect("round-tripped request is valid");
    }
}

#[test]
fn every_outcome_round_trips_through_a_response_envelope() {
    let signer = Keys::generate();
    for outcome in outcome_fixtures(&signer) {
        let action = outcome.action();
        let response = BrokerResponse::new("req-1", BrokerResult::succeeded(outcome.clone()));
        response.validate().expect("response is valid");

        let json = serde_json::to_value(&response).expect("response serializes");
        assert_eq!(json["type"], BROKER_RESULT_TYPE);
        assert_eq!(json["status"], "succeeded");
        assert_eq!(json["action"], action.as_str());
        assert!(json.get("error").is_none(), "a success carries no error");
        // `replayed` is delivery metadata and stays off the wire when false.
        assert!(json.get("replayed").is_none());

        let parsed: BrokerResponse = serde_json::from_value(json)
            .unwrap_or_else(|e| panic!("{} outcome must deserialize: {e}", action.as_str()));
        assert_eq!(parsed, response);
        assert_eq!(parsed.result.outcome(), Some(&outcome));
        assert!(parsed.result.error().is_none());
    }
}

/// Args and outcome share the `action` discriminator, so a payload can never
/// pair one action's name with another's shape.
#[test]
fn an_args_shape_cannot_be_paired_with_another_action_name() {
    let json = serde_json::json!({
        "type": BROKER_REQUEST_TYPE,
        "protocolVersion": 1,
        "requestId": "req-1",
        "actionVersion": 1,
        "action": "agents.delete",
        "args": { "channelId": CHANNEL, "content": "not a delete" },
    });
    assert!(serde_json::from_value::<BrokerRequest>(json).is_err());
}

/// `#[serde(flatten)]` silently disables `deny_unknown_fields`, so the response
/// envelope — the one payload here that needs `flatten` for its wire shape — read
/// as strict while accepting and discarding extra keys. Every rejection below
/// parsed cleanly before the strict intermediary existed.
///
/// The request envelope has the same `flatten` but *not* the same hole: its
/// `ActionArgs` is adjacently tagged, contributing exactly `action` and `args`,
/// so `deny_unknown_fields` still applies to the whole set. That is pinned in
/// [`a_request_envelope_rejects_anything_outside_its_exact_key_set`] rather than
/// assumed.
#[test]
fn a_response_envelope_rejects_anything_outside_its_exact_key_set() {
    let succeeded = || {
        serde_json::json!({
            "type": BROKER_RESULT_TYPE,
            "protocolVersion": 1,
            "requestId": "req-1",
            "status": "succeeded",
            "action": "agents.delete",
            "outcome": { "agentPubkey": PUBKEY, "displayName": "Gone" },
        })
    };
    let failed = || {
        serde_json::json!({
            "type": BROKER_RESULT_TYPE,
            "protocolVersion": 1,
            "requestId": "req-1",
            "status": "failed",
            "error": { "code": "action_failed", "message": "no" },
        })
    };
    assert!(serde_json::from_value::<BrokerResponse>(succeeded()).is_ok());
    assert!(serde_json::from_value::<BrokerResponse>(failed()).is_ok());

    let mut rejected: Vec<(&str, serde_json::Value)> = Vec::new();

    // An unknown top-level key, including one that reads as key material.
    for extra in ["hostNote", "secretKey", "credential"] {
        let mut json = succeeded();
        json[extra] = serde_json::json!("nsec1deadbeef");
        rejected.push((extra, json));
        let mut json = failed();
        json[extra] = serde_json::json!("nsec1deadbeef");
        rejected.push((extra, json));
    }

    // Members the declared status does not admit. Each of these is a
    // contradiction the type system already forbids in Rust, and the envelope
    // used to accept it on the wire and drop the half it could not represent.
    let mut error_beside_success = succeeded();
    error_beside_success["error"] = serde_json::json!({ "code": "internal", "message": "?" });
    rejected.push(("error beside a success", error_beside_success));

    let mut outcome_beside_failure = failed();
    outcome_beside_failure["action"] = serde_json::json!("agents.delete");
    outcome_beside_failure["outcome"] =
        serde_json::json!({ "agentPubkey": PUBKEY, "displayName": "Gone" });
    rejected.push(("outcome beside a failure", outcome_beside_failure));

    let mut outcome_beside_indeterminate = failed();
    outcome_beside_indeterminate["status"] = serde_json::json!("indeterminate");
    outcome_beside_indeterminate["error"] =
        serde_json::json!({ "code": "outcome_unknown", "message": "?" });
    outcome_beside_indeterminate["action"] = serde_json::json!("agents.delete");
    outcome_beside_indeterminate["outcome"] =
        serde_json::json!({ "agentPubkey": PUBKEY, "displayName": "Gone" });
    rejected.push((
        "outcome beside an indeterminate",
        outcome_beside_indeterminate,
    ));

    // Missing the member its status requires.
    let mut no_outcome = succeeded();
    no_outcome.as_object_mut().unwrap().remove("outcome");
    rejected.push(("success with no outcome", no_outcome));
    let mut no_error = failed();
    no_error.as_object_mut().unwrap().remove("error");
    rejected.push(("failure with no error", no_error));

    // An unknown status is not a fourth disposition to ignore.
    for status in ["succeeded_partially", "pending", "SUCCEEDED", ""] {
        let mut json = failed();
        json["status"] = serde_json::json!(status);
        rejected.push(("unknown status", json));
    }

    // Strictness still reaches inside the outcome.
    let mut extra_in_outcome = succeeded();
    extra_in_outcome["outcome"]["secretKey"] = serde_json::json!("nsec1deadbeef");
    rejected.push(("unknown key inside the outcome", extra_in_outcome));

    let mut extra_in_error = failed();
    extra_in_error["error"]["secretKey"] = serde_json::json!("nsec1deadbeef");
    rejected.push(("unknown key inside the error", extra_in_error));

    for (what, json) in rejected {
        assert!(
            serde_json::from_value::<BrokerResponse>(json.clone()).is_err(),
            "{what} must not deserialize: {json}"
        );
    }
}

/// Strict deserialization must not have narrowed what the writer emits: the
/// wire form is still the flattened one, and the strict reader is its inverse for
/// every status, with and without the optional `replayed`.
#[test]
fn the_strict_reader_accepts_exactly_what_the_writer_emits() {
    let signer = Keys::generate();
    let mut results: Vec<BrokerResult> = outcome_fixtures(&signer)
        .into_iter()
        .map(BrokerResult::succeeded)
        .collect();
    results.push(BrokerResult::failed(BrokerError::new(
        BrokerErrorCode::ActionFailed,
        "runtime not installed",
    )));
    results.push(BrokerResult::indeterminate(BrokerError::new(
        BrokerErrorCode::OutcomeUnknown,
        "host restarted mid-execution",
    )));

    for result in results {
        for replayed in [false, true] {
            let response = if replayed {
                BrokerResponse::new("req-1", result.clone()).replayed()
            } else {
                BrokerResponse::new("req-1", result.clone())
            };
            let json = serde_json::to_value(&response).expect("response serializes");
            let parsed: BrokerResponse = serde_json::from_value(json.clone())
                .unwrap_or_else(|e| panic!("strict reader rejected our own bytes {json}: {e}"));
            assert_eq!(parsed, response);
            assert_eq!(parsed.replayed, replayed);
        }
    }
}

/// The request envelope flattens too, so it was checked for the same hole. It
/// does not have one — `ActionArgs` is adjacently tagged and contributes exactly
/// `action` and `args`, leaving `deny_unknown_fields` in force — and this pins
/// that, so the request side cannot regress into the response side's bug.
#[test]
fn a_request_envelope_rejects_anything_outside_its_exact_key_set() {
    let valid = || {
        serde_json::json!({
            "type": BROKER_REQUEST_TYPE,
            "protocolVersion": 1,
            "requestId": "req-1",
            "actionVersion": 1,
            "action": "channel.read",
            "args": { "channelId": CHANNEL },
        })
    };
    assert!(serde_json::from_value::<BrokerRequest>(valid()).is_ok());

    // Unknown top-level key, beside the flattened discriminator, and inside the
    // args — all four positions a smuggled field could take.
    for extra in ["hostNote", "secretKey", "onBehalfOf", "envVars"] {
        let mut json = valid();
        json[extra] = serde_json::json!("nsec1deadbeef");
        assert!(
            serde_json::from_value::<BrokerRequest>(json.clone()).is_err(),
            "a request carrying top-level \"{extra}\" must not deserialize: {json}"
        );

        let mut json = valid();
        json["args"][extra] = serde_json::json!("nsec1deadbeef");
        assert!(
            serde_json::from_value::<BrokerRequest>(json.clone()).is_err(),
            "a request carrying \"{extra}\" inside args must not deserialize: {json}"
        );
    }

    // A second discriminator-shaped key is not a place to hide one either.
    let mut extra_tag = valid();
    extra_tag["outcome"] = serde_json::json!({});
    assert!(serde_json::from_value::<BrokerRequest>(extra_tag).is_err());

    // Missing required members, so the pin cannot pass by accepting anything.
    for missing in [
        "type",
        "protocolVersion",
        "requestId",
        "actionVersion",
        "args",
    ] {
        let mut json = valid();
        json.as_object_mut().unwrap().remove(missing);
        assert!(
            serde_json::from_value::<BrokerRequest>(json).is_err(),
            "a request missing \"{missing}\" must not deserialize"
        );
    }
}

/// Every JSON-pointer path to an object member reachable in `value`, including
/// members nested inside arrays, so a null-injection table cannot miss one.
fn member_paths(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                // Escape per RFC 6901, so a key containing `/` or `~` still
                // addresses the member it names.
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let path = format!("{prefix}/{escaped}");
                out.push(path.clone());
                member_paths(child, &path, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                member_paths(child, &format!("{prefix}/{index}"), out);
            }
        }
        _ => {}
    }
}

/// The bug this guards: `#[serde(default)] Option<T>` maps an explicit `null` to
/// `None`, which is indistinguishable from *absent*. The response envelope decides
/// its shape from absence, so `{"status":"failed","action":null,"outcome":null}`
/// and a succeeded response with `"error":null` both parsed as well-formed and
/// skipped the per-status contradiction check entirely — a malformed envelope
/// validating `Ok`.
///
/// The rule adopted in response is uniform and therefore checkable: **no member
/// anywhere in this contract accepts an explicit `null`.** Nothing here emits one
/// (`skip_serializing_if` omits instead), so `null` is a second spelling of
/// "absent" that the contract simply does not define. One spelling means no layer
/// has to decide what a present-but-null member meant.
///
/// This walks the real fixtures rather than a hand-written list of members, so an
/// optional field added later is covered without anyone remembering to add it
/// here.
#[test]
fn no_member_of_any_payload_accepts_an_explicit_null() {
    let keys = Keys::generate();

    // Requests: every action, with every optional member populated.
    for args in action_fixtures() {
        let request = BrokerRequest::new("req-1", args).expect("fixture request builds");
        let valid = serde_json::to_value(&request).expect("request serializes");
        // The untouched fixture must parse, or nulling members below would
        // "reject" for a reason that has nothing to do with null.
        assert_eq!(
            serde_json::from_value::<BrokerRequest>(valid.clone()).expect("fixture parses"),
            request,
        );

        let mut paths = Vec::new();
        member_paths(&valid, "", &mut paths);
        assert!(
            paths.len() > 1,
            "fixture should expose several members: {valid}"
        );
        for path in paths {
            let mut json = valid.clone();
            *json.pointer_mut(&path).expect("path addresses a member") = serde_json::Value::Null;
            assert!(
                serde_json::from_value::<BrokerRequest>(json.clone()).is_err(),
                "request with null at \"{path}\" must not deserialize: {json}"
            );
        }
    }

    // Responses: every outcome, plus both error-carrying statuses.
    let mut results: Vec<BrokerResult> = outcome_fixtures(&keys)
        .into_iter()
        .map(BrokerResult::succeeded)
        .collect();
    results.push(BrokerResult::failed(BrokerError::new(
        BrokerErrorCode::ActionFailed,
        "runtime not installed",
    )));
    results.push(BrokerResult::indeterminate(BrokerError::new(
        BrokerErrorCode::OutcomeUnknown,
        "host restarted mid-execution",
    )));

    for result in results {
        let response = BrokerResponse::new("req-1", result).replayed();
        let valid = serde_json::to_value(&response).expect("response serializes");
        assert_eq!(
            serde_json::from_value::<BrokerResponse>(valid.clone()).expect("fixture parses"),
            response,
        );

        let mut paths = Vec::new();
        member_paths(&valid, "", &mut paths);
        for path in paths {
            let mut json = valid.clone();
            *json.pointer_mut(&path).expect("path addresses a member") = serde_json::Value::Null;
            assert!(
                serde_json::from_value::<BrokerResponse>(json.clone()).is_err(),
                "response with null at \"{path}\" must not deserialize: {json}"
            );
        }
    }
    // The two `bool` members carry no explicit guard, because `null` already
    // fails as a type error rather than defaulting to `false`. Pin that, so the
    // docs saying so cannot drift and so a later change to `Option<bool>` — which
    // *would* need the guard — fails here.
    let mut json = serde_json::json!({
        "type": BROKER_REQUEST_TYPE,
        "protocolVersion": 1,
        "requestId": "req-1",
        "actionVersion": 1,
        "action": "channel.read",
        "args": { "channelId": CHANNEL, "mentionsOnly": serde_json::Value::Null },
    });
    assert!(
        serde_json::from_value::<BrokerRequest>(json.clone()).is_err(),
        "a null mentionsOnly must not deserialize: {json}"
    );
    json = serde_json::json!({
        "type": BROKER_RESULT_TYPE,
        "protocolVersion": 1,
        "requestId": "req-1",
        "status": "failed",
        "error": { "code": "action_failed", "message": "no" },
        "replayed": serde_json::Value::Null,
    });
    assert!(
        serde_json::from_value::<BrokerResponse>(json.clone()).is_err(),
        "a null replayed must not deserialize: {json}"
    );
}

/// The exact repro that reached `Ok`: a member the declared status does not admit,
/// supplied as `null` rather than as a value. The fixtures above cannot cover this
/// — a serialized response never contains the member its status forbids — so each
/// status-incompatible member is injected here by name.
///
/// This is the case that makes the null hole a contract bug rather than a
/// tidiness one: these envelopes contradict themselves, and before the fix
/// `validate()` returned `Ok(())` on all of them.
#[test]
fn a_status_incompatible_member_is_rejected_as_null_not_only_as_a_value() {
    let succeeded = || {
        serde_json::json!({
            "type": BROKER_RESULT_TYPE,
            "protocolVersion": 1,
            "requestId": "req-1",
            "status": "succeeded",
            "action": "agents.delete",
            "outcome": { "agentPubkey": PUBKEY, "displayName": "Gone" },
        })
    };
    let failed = || {
        serde_json::json!({
            "type": BROKER_RESULT_TYPE,
            "protocolVersion": 1,
            "requestId": "req-1",
            "status": "failed",
            "error": { "code": "action_failed", "message": "no" },
        })
    };

    let mut cases: Vec<(String, serde_json::Value)> = Vec::new();

    // `error` is the member a success does not admit.
    let mut json = succeeded();
    json["error"] = serde_json::Value::Null;
    cases.push(("null error beside a success".into(), json));

    // `action` and `outcome` are the members the two failure statuses do not
    // admit — individually and together, since the original report showed both.
    for status in ["failed", "indeterminate"] {
        let base = || {
            let mut json = failed();
            json["status"] = serde_json::json!(status);
            if status == "indeterminate" {
                json["error"] = serde_json::json!({ "code": "outcome_unknown", "message": "?" });
            }
            json
        };
        for member in ["action", "outcome"] {
            let mut json = base();
            json[member] = serde_json::Value::Null;
            cases.push((format!("null {member} beside a {status}"), json));
        }
        let mut json = base();
        json["action"] = serde_json::Value::Null;
        json["outcome"] = serde_json::Value::Null;
        cases.push((format!("null action and outcome beside a {status}"), json));
    }

    for (what, json) in cases {
        let parsed = serde_json::from_value::<BrokerResponse>(json.clone());
        // Assert on the parse, not on `validate()`: a response that parses and
        // then fails validation would still have to be *reported* by a caller
        // that remembered to validate. Rejecting at the boundary means a
        // malformed envelope never becomes a value at all.
        assert!(
            parsed.is_err(),
            "{what} must not deserialize, but parsed as {:?} which validates {:?}: {json}",
            parsed.as_ref().ok(),
            parsed.as_ref().map(BrokerResponse::validate).ok(),
        );
    }
}

// ── Envelope rejection ──────────────────────────────────────────────────────

/// Unknown names must not resolve, and neither must the *mechanism* names this
/// contract deliberately refuses to expose: an interface that can sign arbitrary
/// bytes is a signing oracle.
#[test]
fn only_declared_action_names_resolve() {
    for action in Action::ALL {
        assert_eq!(Action::parse(action.as_str()).unwrap(), action);
    }
    for rejected in [
        "channel.write",
        "agents.exfiltrate",
        "",
        "channel.read ",
        "sign",
        "sign_event",
        "publish",
        "nip44.encrypt",
        "nip44.decrypt",
        "nip42.auth",
        "nip98.auth",
        "keys.export",
        "identity.nsec",
        "presence.set",
        "typing.set",
    ] {
        assert!(
            Action::parse(rejected).is_err(),
            "\"{rejected}\" must not parse as an action"
        );
    }

    let json = serde_json::json!({
        "type": BROKER_REQUEST_TYPE,
        "protocolVersion": 1,
        "requestId": "req-1",
        "actionVersion": 1,
        "action": "agents.exfiltrate",
        "args": {},
    });
    assert!(serde_json::from_value::<BrokerRequest>(json).is_err());
}

#[test]
fn envelope_metadata_must_match_this_protocol_version() {
    let args = || {
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Pubkey(pubkey()),
        })
    };
    let failed = || BrokerResult::failed(BrokerError::unsupported("no"));

    for bad in [0_u16, 2, 999] {
        let mut request = BrokerRequest::new("req-1", args()).unwrap();
        request.protocol_version = bad;
        let error = request.validated().unwrap_err().to_string();
        assert!(error.contains("protocolVersion"), "unexpected: {error}");

        let mut response = BrokerResponse::new("req-1", failed());
        response.protocol_version = bad;
        assert!(response.validate().is_err());
    }

    let mut wrong_action_version = BrokerRequest::new("req-1", args()).unwrap();
    wrong_action_version.action_version = 7;
    let error = wrong_action_version.validated().unwrap_err().to_string();
    assert!(error.contains("actionVersion"), "unexpected: {error}");

    let mut wrong_request_type = BrokerRequest::new("req-1", args()).unwrap();
    wrong_request_type.r#type = BROKER_RESULT_TYPE.into();
    assert!(wrong_request_type.validated().is_err());

    let mut wrong_response_type = BrokerResponse::new("req-1", failed());
    wrong_response_type.r#type = BROKER_REQUEST_TYPE.into();
    assert!(wrong_response_type.validate().is_err());
}

#[test]
fn request_id_must_be_present_bounded_and_printable() {
    let args = || {
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Pubkey(pubkey()),
        })
    };
    for (id, valid) in [
        ("", false),
        ("has space", false),
        ("has\nnewline", false),
        ("has\u{7f}del", false),
        ("req/1-a.b:c", true),
    ] {
        assert_eq!(
            BrokerRequest::new(id, args()).is_ok(),
            valid,
            "requestId {id:?} validity"
        );
    }
    assert!(BrokerRequest::new("a".repeat(MAX_REQUEST_ID_LEN), args()).is_ok());
    assert!(BrokerRequest::new("a".repeat(MAX_REQUEST_ID_LEN + 1), args()).is_err());
}

/// Duplicate object keys are rejected everywhere the envelope reads, including
/// inside the `outcome` object.
///
/// serde's derived readers reject a repeated field, so most of this contract got
/// duplicate rejection for free. `outcome` did not: the strict intermediary held
/// it as a `serde_json::Value` before re-deserializing it under its action tag,
/// and buffering through `Value` silently collapses duplicates last-wins. That
/// made `outcome` the one place where a reader could see a value the envelope's
/// own strictness never vetted — so it now re-parses the original bytes via
/// `RawValue`.
///
/// Each case asserts the de-duplicated form parses first, so a rejection cannot
/// be a rejection of the surrounding fixture.
#[test]
fn a_duplicate_object_key_is_rejected_at_every_depth() {
    let outcome = format!(r#"{{"agentPubkey":"{PUBKEY}","displayName":"n"}}"#);
    let response = |body: &str| {
        format!(
            r#"{{"type":"{BROKER_RESULT_TYPE}","protocolVersion":1,"requestId":"r","status":"succeeded","action":"agents.delete","outcome":{body}}}"#
        )
    };

    serde_json::from_str::<BrokerResponse>(&response(&outcome))
        .expect("the de-duplicated response parses");
    let cases = [
        (
            "inside the outcome object",
            response(&format!(
                r#"{{"agentPubkey":"{PUBKEY}","displayName":"first","displayName":"second"}}"#
            )),
        ),
        (
            "a top-level envelope member",
            format!(
                r#"{{"type":"{BROKER_RESULT_TYPE}","protocolVersion":1,"requestId":"r","requestId":"evil","status":"succeeded","action":"agents.delete","outcome":{outcome}}}"#
            ),
        ),
        (
            "a flattened member",
            format!(
                r#"{{"type":"{BROKER_RESULT_TYPE}","protocolVersion":1,"requestId":"r","status":"succeeded","action":"agents.delete","action":"agents.update","outcome":{outcome}}}"#
            ),
        ),
        (
            "inside a typed error payload",
            format!(
                r#"{{"type":"{BROKER_RESULT_TYPE}","protocolVersion":1,"requestId":"r","status":"failed","error":{{"code":"unauthorized","message":"a","message":"b"}}}}"#
            ),
        ),
    ];
    for (where_, json) in cases {
        assert!(
            serde_json::from_str::<BrokerResponse>(&json).is_err(),
            "a duplicate key {where_} must not deserialize"
        );
    }

    // The request envelope too, where `args` is typed rather than buffered.
    let request = |args: &str| {
        format!(
            r#"{{"type":"{BROKER_REQUEST_TYPE}","protocolVersion":1,"requestId":"r","actionVersion":1,"action":"agents.delete","args":{args}}}"#
        )
    };
    serde_json::from_str::<BrokerRequest>(&request(r#"{"target":{"name":"good"}}"#))
        .expect("the de-duplicated request parses");
    assert!(
        serde_json::from_str::<BrokerRequest>(&request(
            r#"{"target":{"name":"good"},"target":{"name":"evil"}}"#
        ))
        .is_err(),
        "a duplicate key inside args must not deserialize"
    );
}

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

/// 64 hex characters is a *shape*; a public key is a point on secp256k1. Most
/// 32-byte values are not one, so accepting shape alone let this type's name
/// promise something it never checked, and deferred the first real rejection to
/// whichever consumer eventually converted the string to a key — by which point
/// the request had already been accepted.
///
/// The fixtures are ordered the way the type is used: the real key must pass
/// untouched first, so a rejection below is about the curve check and not about a
/// probe that would have failed for any input.
#[test]
fn a_pubkey_must_be_a_point_on_the_curve_not_merely_hex() {
    /// The check this type now delegates to, spelled out independently: a value
    /// is a key only if it converts to an x-only key, which is what `xonly`
    /// does. `from_hex` alone is a hex decode and answers nothing — asking only
    /// it is how the gap survived a `nostr`-backed check in the first place.
    fn is_a_point(hex: &str) -> bool {
        nostr::PublicKey::from_hex(hex)
            .and_then(|key| key.xonly().map(|_| ()))
            .is_ok()
    }

    // A real key passes, in both spellings, and is unchanged by the new check.
    assert!(is_a_point(PUBKEY), "fixture must be a real point");
    assert_eq!(
        PubkeyHex::parse(PUBKEY).expect("a real key parses"),
        pubkey()
    );
    assert_eq!(
        PubkeyHex::parse(PUBKEY.to_ascii_uppercase()).expect("case is still normalized"),
        pubkey()
    );

    // Well-formed hex that is not on the curve. Each is asserted to be a
    // non-point first, so the rejection cannot be for an unrelated reason.
    //
    // The last fixture is the important one: `x = 5` is a perfectly in-range
    // field element, so it is not rejected for overflowing the field the way the
    // first three are — there is simply no y with y² = x³ + 7. A check that
    // only bounds the value against the field prime would accept it, so this is
    // what pins the test to a real curve check rather than a range check.
    for junk in [
        "f".repeat(64),
        "0".repeat(64),
        // The field prime p itself: out of range by exactly one.
        "fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f".into(),
        format!("{:0>64}", 5),
    ] {
        assert!(
            !is_a_point(&junk),
            "fixture \"{junk}\" must not be a valid point"
        );
        let error = PubkeyHex::parse(&junk)
            .expect_err("64 hex characters that are not a point must be rejected")
            .to_string();
        assert!(
            error.contains("x-only"),
            "rejection must name the curve, not the shape: {error}"
        );
        // The serde door takes the same check: `PubkeyHex` deserializes through
        // `parse`, so a host cannot ship a non-point where a constructor would
        // have refused one.
        assert!(
            serde_json::from_value::<PubkeyHex>(serde_json::json!(junk)).is_err(),
            "the wire door must reject the non-point \"{junk}\" too"
        );
        // And through a payload, since that is the shape a host actually sends.
        // The honest form parses, so this rejects for the key and not the shape.
        let target = |value: &str| {
            serde_json::json!({
                "action": "agents.delete",
                "args": { "target": { "pubkey": value } },
            })
        };
        serde_json::from_value::<ActionArgs>(target(PUBKEY))
            .expect("a real key in a target payload must parse");
        assert!(
            serde_json::from_value::<ActionArgs>(target(&junk)).is_err(),
            "an agents.delete target must reject the non-point \"{junk}\""
        );
    }
}

// ── Argument validation ─────────────────────────────────────────────────────

/// Boundaries of every shared validator, in one table.
#[test]
fn validators_accept_and_reject_at_their_boundaries() {
    let read = |mutate: fn(&mut ChannelReadArgs)| {
        let mut args = ChannelReadArgs::channel(CHANNEL);
        mutate(&mut args);
        args.validated().is_ok()
    };
    let post = |content: String, mentions: Vec<PubkeyHex>| {
        MessagePostArgs {
            channel_id: CHANNEL.into(),
            content,
            mentions,
        }
        .validated()
    };
    let react = |reaction: String| {
        ReactionAddArgs {
            channel_id: CHANNEL.into(),
            target_event_id: EVENT.into(),
            reaction,
        }
        .validated()
    };
    let slug = |slug: &str| StorageAddressArgs { slug: slug.into() }.validated().is_ok();

    // Channel UUID, thread id, limit, and opaque cursor.
    assert!(ChannelReadArgs::channel("not-a-uuid").validated().is_err());
    assert!(!read(|a| a.root_event_id = Some("nothex".into())));
    assert!(read(|a| a.root_event_id = Some(EVENT.into())));
    assert!(!read(|a| a.limit = Some(0)));
    assert!(read(|a| a.limit = Some(actions::MAX_PAGE_LIMIT)));
    assert!(!read(|a| a.limit = Some(actions::MAX_PAGE_LIMIT + 1)));
    assert!(!read(|a| a.cursor = Some(String::new())));
    assert!(!read(|a| a.cursor = Some("has space".into())));
    assert!(read(
        |a| a.cursor = Some("a".repeat(actions::MAX_CURSOR_LEN))
    ));
    assert!(!read(
        |a| a.cursor = Some("a".repeat(actions::MAX_CURSOR_LEN + 1))
    ));

    // Content, mentions, reaction payload.
    assert!(post("   ".into(), vec![]).is_err());
    assert!(matches!(
        post("x".repeat(actions::MAX_CONTENT_BYTES + 1), vec![]).unwrap_err(),
        SdkError::ContentTooLarge { .. }
    ));
    assert!(post("hi".into(), vec![pubkey(); actions::MAX_MENTIONS]).is_ok());
    assert!(matches!(
        post("hi".into(), vec![pubkey(); actions::MAX_MENTIONS + 1]).unwrap_err(),
        SdkError::TooManyMentions
    ));
    assert!(react(" ".into()).is_err());
    assert!(react(":shipit:".into()).is_ok());
    assert!(matches!(
        react("a".repeat(actions::MAX_EMOJI_CHARS + 1)).unwrap_err(),
        SdkError::EmojiTooLong
    ));

    // NIP-AE slug grammar for encrypted-memory addressing.
    assert!(slug("core"));
    assert!(slug("mem/broker-foundation"));
    assert!(!slug(""));
    assert!(!slug("Core"));
    assert!(!slug("secrets"));
    assert!(!slug("mem/Bad Slug"));

    // Patch-shaped writes must change something, and reject unknown modes.
    let profile_error = ProfileSetArgs {
        display_name: None,
        about: None,
        picture: None,
    }
    .validated()
    .unwrap_err()
    .to_string();
    assert!(profile_error.contains("at least one"), "{profile_error}");
    let update = |respond_to: Option<&str>, name: Option<&str>| {
        AgentsUpdateArgs {
            target: AgentTarget::Pubkey(pubkey()),
            display_name: name.map(str::to_string),
            system_prompt: None,
            runtime: None,
            provider: None,
            model: None,
            respond_to: respond_to.map(str::to_string),
        }
        .validated()
    };
    assert!(update(None, None)
        .unwrap_err()
        .to_string()
        .contains("at least one field"));
    assert!(update(Some("anyone"), None).is_ok());
    assert!(update(Some("allowlist"), Some("A")).is_err());
    assert!(AgentsDeleteArgs {
        target: AgentTarget::Name("  ".into()),
    }
    .validated()
    .is_err());
}

/// Validation must be **inseparable from normalization**: there must be no way
/// to learn that a request is valid while still holding the un-normalized value.
///
/// The bug this pins: `validate(&self)` called the arguments' `validated()`,
/// which *computes* a normalized copy, then dropped the copy and returned
/// `Ok(())`. A hand-built request targeting `"  helper  "` therefore passed
/// validation and still carried the padding, so a host that trusted the verdict
/// and executed the struct looked up a name the validator never approved.
/// `prepare()` was safe, but a host cannot force its callers through the client's
/// outgoing path.
///
/// The fix is typed, so this test asserts the *shape* of the API and not just one
/// call's behaviour: the only route to a verdict is `validated()`, which consumes
/// the request and hands back a `ValidatedRequest` whose arguments are already
/// normalized. The un-normalized value is gone rather than sitting beside its
/// approved copy. That the old method no longer exists is enforced at compile
/// time by every other caller in this file having had to change.
#[test]
fn a_request_cannot_be_validated_without_being_normalized() {
    let padded = || {
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Name("  helper  ".into()),
        })
    };
    let trimmed = ActionArgs::AgentsDelete(AgentsDeleteArgs {
        target: AgentTarget::Name("helper".into()),
    });

    // A request built by hand, bypassing `new` — the shape the reviewer used.
    let hand_built = BrokerRequest {
        r#type: BROKER_REQUEST_TYPE.into(),
        protocol_version: BROKER_PROTOCOL_VERSION,
        request_id: "req-trap".into(),
        action_version: 1,
        action: padded(),
    };
    let validated = hand_built
        .validated()
        .expect("a padded name is valid, just not canonical");

    // The verdict and the normalized value are the same object, so an executor
    // holding the verdict cannot be holding the padding.
    assert_eq!(
        validated.args(),
        &trimmed,
        "a validated request must carry the normalized arguments"
    );
    assert_eq!(validated.action(), Action::AgentsDelete);
    assert_eq!(validated.request_id(), "req-trap");

    // Freezing from the verdict carries the normalized value onto the wire.
    let body = String::from_utf8(
        validated
            .clone()
            .prepare()
            .expect("prepares")
            .body()
            .to_vec(),
    )
    .expect("body is utf8");
    assert!(
        body.contains(r#""name":"helper""#) && !body.contains("  helper  "),
        "frozen body still carries the unnormalized name: {body}"
    );

    // Moving the envelope onward yields the normalized request, not the input.
    assert_eq!(validated.into_request().action, trimmed);

    // The envelope is still checked, so `validated` is not merely a normalizer:
    // an invalid envelope produces no verdict at all.
    let bad_version = BrokerRequest {
        r#type: BROKER_REQUEST_TYPE.into(),
        protocol_version: 99,
        request_id: "req-trap".into(),
        action_version: 1,
        action: padded(),
    };
    assert!(bad_version.validated().is_err());

    // And arguments that cannot be normalized are rejected rather than
    // normalized to something the caller did not ask for.
    let empty_name = BrokerRequest {
        r#type: BROKER_REQUEST_TYPE.into(),
        protocol_version: BROKER_PROTOCOL_VERSION,
        request_id: "req-trap".into(),
        action_version: 1,
        action: ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Name("   ".into()),
        }),
    };
    assert!(empty_name.validated().is_err());
}

/// Validation normalizes, so the frozen body must carry the normalized value —
/// not the caller's. Otherwise a padded selector passes validation and the host
/// executes something the validator never approved: it looks up `"  helper  "`,
/// or publishes a padded reaction.
///
/// Both construction paths are checked, because `BrokerRequest`'s fields are
/// public and it is `Deserialize`, so `prepare` is reachable without ever going
/// through `new`.
#[test]
fn the_frozen_body_carries_exactly_what_validation_approved() {
    // Path 1: through `new`, which stores the normalized action.
    let request = BrokerRequest::new(
        "req-normalize",
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Name("  helper  ".into()),
        }),
    )
    .expect("a padded name is valid, just not canonical");
    assert_eq!(
        request.action,
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Name("helper".into()),
        }),
        "`new` must store the normalized copy"
    );
    let body = String::from_utf8(request.prepare().expect("prepares").body().to_vec())
        .expect("body is utf8");
    assert!(
        body.contains(r#""name":"helper""#) && !body.contains("  helper  "),
        "frozen body still carries the unnormalized name: {body}"
    );

    // Path 2: a struct literal that bypasses `new` entirely.
    let bypassed = BrokerRequest {
        r#type: BROKER_REQUEST_TYPE.to_string(),
        protocol_version: BROKER_PROTOCOL_VERSION,
        request_id: "req-bypass".into(),
        action_version: 1,
        action: ActionArgs::ReactionAdd(ReactionAddArgs {
            channel_id: CHANNEL.into(),
            target_event_id: EVENT.into(),
            reaction: "  \u{1f41d}  ".into(),
        }),
    };
    let body = String::from_utf8(bypassed.prepare().expect("prepares").body().to_vec())
        .expect("body is utf8");
    assert!(
        body.contains("\"reaction\":\"\u{1f41d}\""),
        "frozen body did not normalize a padded reaction: {body}"
    );

    // Normalization is idempotent, so a second freeze is byte-identical: the
    // retry contract still holds through the new path.
    let once = BrokerRequest::new(
        "req-idem",
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Name(" helper ".into()),
        }),
    )
    .unwrap()
    .prepare()
    .unwrap();
    let twice = BrokerRequest::new(
        "req-idem",
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Name("helper".into()),
        }),
    )
    .unwrap()
    .prepare()
    .unwrap();
    assert_eq!(
        once.body(),
        twice.body(),
        "a padded and a pre-trimmed request must freeze to the same bytes"
    );
}

/// Correlation must reject an outcome that echoes a different identity than the
/// request supplied — `requestId` plus action is not enough, because a host
/// routing bug can return a well-formed success for the wrong subject.
///
/// Table-driven over every request/outcome identity pair, so the enumeration in
/// `correlate_identities`' doc table is pinned by a test rather than asserted in
/// prose. Each case builds the *matching* response first and requires it to pass,
/// so a case cannot "reject" for an unrelated reason.
#[test]
fn correlation_rejects_an_outcome_naming_a_different_subject() {
    let requested = pubkey();
    let other =
        PubkeyHex::parse("b02c4e0850e5e612b4ddf95dbe2f5c56467cf27c6552203bc833ff438fb31971")
            .expect("valid hex");
    let other_channel = "c2c38ca8-9ec3-411e-bab5-f9deab34d52e";

    // (action, matching outcome, mismatched outcome or None when nothing is
    // comparable). A `None` documents an inherent gap, not an oversight.
    let cases: Vec<(&str, ActionArgs, ActionOutcome, Option<ActionOutcome>)> = vec![
        (
            "agents.create echoes channelId",
            ActionArgs::AgentsCreate(AgentsCreateArgs {
                channel_id: CHANNEL.into(),
                display_name: "Helper".into(),
                system_prompt: "be useful".into(),
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
            }),
            ActionOutcome::AgentsCreate(AgentsCreateOutcome {
                agent_pubkey: requested.clone(),
                display_name: "Helper".into(),
                channel_id: CHANNEL.into(),
            }),
            Some(ActionOutcome::AgentsCreate(AgentsCreateOutcome {
                agent_pubkey: requested.clone(),
                display_name: "Helper".into(),
                channel_id: other_channel.into(),
            })),
        ),
        (
            "agents.update targeted by pubkey echoes agentPubkey",
            ActionArgs::AgentsUpdate(AgentsUpdateArgs {
                target: AgentTarget::Pubkey(requested.clone()),
                display_name: Some("Renamed".into()),
                system_prompt: None,
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
            }),
            ActionOutcome::AgentsUpdate(AgentsUpdateOutcome {
                agent_pubkey: requested.clone(),
                display_name: "Renamed".into(),
                updated_fields: vec!["displayName".into()],
            }),
            Some(ActionOutcome::AgentsUpdate(AgentsUpdateOutcome {
                agent_pubkey: other.clone(),
                display_name: "Renamed".into(),
                updated_fields: vec!["displayName".into()],
            })),
        ),
        (
            "agents.delete targeted by pubkey echoes agentPubkey",
            ActionArgs::AgentsDelete(AgentsDeleteArgs {
                target: AgentTarget::Pubkey(requested.clone()),
            }),
            ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
                agent_pubkey: requested.clone(),
                display_name: "Gone".into(),
            }),
            Some(ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
                agent_pubkey: other.clone(),
                display_name: "Gone".into(),
            })),
        ),
        (
            // Inherent gap: the host resolves the name, and the rename may be
            // exactly what this call performed, so no pubkey is comparable.
            "agents.delete targeted by name compares nothing",
            ActionArgs::AgentsDelete(AgentsDeleteArgs {
                target: AgentTarget::Name("helper".into()),
            }),
            ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
                agent_pubkey: other.clone(),
                display_name: "helper".into(),
            }),
            None,
        ),
        (
            // Host-minted identifiers only; nothing the request supplied is echoed.
            "message.post echoes no requested identity",
            ActionArgs::MessagePost(MessagePostArgs {
                channel_id: CHANNEL.into(),
                content: "hi".into(),
                mentions: vec![],
            }),
            ActionOutcome::MessagePost(EventPublished {
                event_id: EVENT.into(),
                kind: 9,
                created_at: 1,
            }),
            None,
        ),
    ];

    for (label, args, matching, mismatched) in cases {
        let request = BrokerRequest::new("req-correlate", args)
            .expect("fixture args validate")
            .prepare()
            .expect("fixture prepares");
        BrokerResponse::new("req-correlate", BrokerResult::succeeded(matching))
            .validate_for(&request)
            .unwrap_or_else(|e| panic!("{label}: the matching outcome must pass, got {e}"));
        if let Some(mismatched) = mismatched {
            let err = BrokerResponse::new("req-correlate", BrokerResult::succeeded(mismatched))
                .validate_for(&request)
                .expect_err(&format!("{label}: a mismatched identity must be rejected"));
            assert!(
                matches!(err, SdkError::InvalidInput(_)),
                "{label}: expected InvalidInput, got {err:?}"
            );
        }
    }
}

// ── Identities have one spelling ────────────────────────────────────────────

/// Every legal spelling of a channel UUID names one channel, so a request and a
/// response that spell it differently must still correlate.
///
/// The bug: `Uuid::parse_str` accepts uppercase, unhyphenated, braced, and
/// `urn:uuid:` forms, `channel()` returned the caller's spelling untouched, and
/// correlation compared bytes — so an uppercase request against a host's
/// canonical lowercase echo of the *same* channel failed `validate_for`. That is
/// worse than the mismatch the check exists to catch: it makes a correct host
/// unusable.
///
/// Two independent guards close it, and each is asserted separately below so
/// neither can be the only thing holding: canonicalize where a value enters, and
/// compare parsed identities rather than bytes.
#[test]
fn one_identity_spelled_two_ways_still_correlates() {
    let spellings = [
        CHANNEL.to_ascii_uppercase(),
        CHANNEL.replace('-', ""),
        format!("{{{CHANNEL}}}"),
        format!("urn:uuid:{CHANNEL}"),
        CHANNEL.to_string(),
    ];

    let create = |channel_id: &str| {
        ActionArgs::AgentsCreate(AgentsCreateArgs {
            channel_id: channel_id.into(),
            display_name: "Helper".into(),
            system_prompt: "be useful".into(),
            runtime: None,
            provider: None,
            model: None,
            respond_to: None,
        })
    };
    let echo = |channel_id: &str| {
        BrokerResult::succeeded(ActionOutcome::AgentsCreate(AgentsCreateOutcome {
            agent_pubkey: pubkey(),
            display_name: "Helper".into(),
            channel_id: channel_id.into(),
        }))
    };

    for spelling in &spellings {
        // Guard 1: the frozen body carries the canonical spelling, not the
        // caller's, so what the host receives is what correlation will compare.
        let request = BrokerRequest::new("req-spelling", create(spelling))
            .expect("every legal UUID spelling validates");
        let body = String::from_utf8(request.prepare().expect("prepares").body().to_vec())
            .expect("body is utf8");
        assert!(
            body.contains(&format!("\"channelId\":\"{CHANNEL}\"")),
            "frozen body did not canonicalize \"{spelling}\": {body}"
        );

        // And through the wire door too, which no `validated()` covers: a parsed
        // request reaches a caller canonical.
        let parsed: BrokerRequest = serde_json::from_value(serde_json::json!({
            "type": BROKER_REQUEST_TYPE,
            "protocolVersion": 1,
            "requestId": "req-spelling",
            "actionVersion": 1,
            "action": "channel.read",
            "args": { "channelId": spelling },
        }))
        .unwrap_or_else(|e| panic!("\"{spelling}\" must parse: {e}"));
        assert_eq!(
            parsed.action,
            ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)),
            "the wire door did not canonicalize \"{spelling}\""
        );

        // Guard 2: correlation compares parsed identities, so every spelling on
        // either side correlates even if guard 1 were absent.
        let prepared = BrokerRequest::new("req-spelling", create(spelling))
            .expect("validates")
            .prepare()
            .expect("prepares");
        for returned in &spellings {
            BrokerResponse::new("req-spelling", echo(returned))
                .validate_for(&prepared)
                .unwrap_or_else(|e| {
                    panic!("request \"{spelling}\" vs echo \"{returned}\" must correlate: {e}")
                });
        }
    }

    // A genuinely different channel is still rejected, so the fix widened what
    // counts as equal without weakening the check.
    let prepared = BrokerRequest::new("req-spelling", create(CHANNEL))
        .expect("validates")
        .prepare()
        .expect("prepares");
    let err = BrokerResponse::new("req-spelling", echo("c2c38ca8-9ec3-411e-bab5-f9deab34d52e"))
        .validate_for(&prepared)
        .expect_err("a different channel must still be rejected");
    assert!(matches!(err, SdkError::InvalidInput(_)), "{err:?}");
}

/// The same treatment for the contract's other multi-spelling identities: hex.
///
/// A pubkey was already canonicalized by `PubkeyHex::parse`, which is also its
/// serde path — this pins that it is, so the `agentPubkey` rows of the
/// correlation table cannot regress into a byte comparison of two cases. Event
/// ids and `d` tags are plain `String`s and were *not* normalized on the wire,
/// only in `validated()`, so those are the ones this changes.
#[test]
fn hex_identities_are_canonical_through_every_door() {
    // Pubkey: mixed-case target vs lowercase echo correlates, both directions.
    let upper = PubkeyHex::parse(PUBKEY.to_ascii_uppercase()).expect("valid hex");
    let request = BrokerRequest::new(
        "req-hex",
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Pubkey(upper),
        }),
    )
    .expect("validates")
    .prepare()
    .expect("prepares");
    BrokerResponse::new(
        "req-hex",
        BrokerResult::succeeded(ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
            agent_pubkey: pubkey(),
            display_name: "Gone".into(),
        })),
    )
    .validate_for(&request)
    .expect("two cases of one pubkey are one identity");

    // Event ids and d tags: the wire door lowercases, so a parsed value equals a
    // constructed one and neither carries the sender's case.
    let parsed: ActionArgs = serde_json::from_value(serde_json::json!({
        "action": "reaction.add",
        "args": {
            "channelId": CHANNEL,
            "targetEventId": EVENT.to_ascii_uppercase(),
            "reaction": "\u{1f41d}",
        },
    }))
    .expect("an uppercase event id parses");
    assert_eq!(
        parsed,
        ActionArgs::ReactionAdd(ReactionAddArgs {
            channel_id: CHANNEL.into(),
            target_event_id: EVENT.into(),
            reaction: "\u{1f41d}".into(),
        }),
        "the wire door did not lowercase targetEventId"
    );

    let parsed: ActionOutcome = serde_json::from_value(serde_json::json!({
        "action": "storage.address",
        "outcome": {
            "authorPubkey": PUBKEY.to_ascii_uppercase(),
            "kind": 30078,
            "dTag": EVENT.to_ascii_uppercase(),
        },
    }))
    .expect("an uppercase d tag parses");
    assert_eq!(
        parsed,
        ActionOutcome::StorageAddress(StorageAddress {
            author_pubkey: pubkey(),
            kind: 30078,
            d_tag: EVENT.into(),
        }),
        "the wire door did not lowercase dTag or authorPubkey"
    );

    // The optional identity member takes the same door, and still rejects null.
    let read: ActionArgs = serde_json::from_value(serde_json::json!({
        "action": "channel.read",
        "args": { "channelId": CHANNEL, "rootEventId": EVENT.to_ascii_uppercase() },
    }))
    .expect("an uppercase root event id parses");
    assert_eq!(
        read,
        ActionArgs::ChannelRead(ChannelReadArgs {
            channel_id: CHANNEL.into(),
            root_event_id: Some(EVENT.into()),
            ..ChannelReadArgs::default()
        }),
    );
    assert!(
        serde_json::from_value::<ActionArgs>(serde_json::json!({
            "action": "channel.read",
            "args": { "channelId": CHANNEL, "rootEventId": serde_json::Value::Null },
        }))
        .is_err(),
        "canonicalizing must not have replaced the null guard"
    );

    // A malformed identity is still a parse failure, so the new doors reject
    // rather than merely normalize.
    for bad in ["nothex", "", &EVENT[..40], &format!("{EVENT}00")] {
        assert!(
            serde_json::from_value::<ActionArgs>(serde_json::json!({
                "action": "channel.read",
                "args": { "channelId": CHANNEL, "rootEventId": bad },
            }))
            .is_err(),
            "rootEventId \"{bad}\" must not deserialize"
        );
    }
    assert!(
        serde_json::from_value::<ActionArgs>(serde_json::json!({
            "action": "channel.read",
            "args": { "channelId": "not-a-uuid" },
        }))
        .is_err(),
        "a non-UUID channelId must not deserialize"
    );
}

/// `BrokerResult` must have **no wire door of its own**, so the strict envelope is
/// the only way to read a result.
///
/// The bug: the exported result type derived its own reader, which accepted and
/// dropped arbitrary siblings — `status: failed` beside an `error` and a
/// `secretKey`, or a succeeded result beside an `error`. A consumer parsing the
/// result type directly therefore got an `Ok` value whose complete wire shape had
/// never been vetted, while the identical bytes failed through the envelope.
///
/// Removing the door is checked at compile time, because a runtime test cannot
/// call a `Deserialize` impl that does not exist. `absence_of_a_reader` resolves to
/// the inherent function only when the bound holds, so this is a genuine negative
/// assertion rather than a comment.
#[test]
fn the_result_type_has_no_deserializer_of_its_own() {
    struct Probe<T>(std::marker::PhantomData<T>);

    trait NoReader {
        fn absence_of_a_reader() -> bool {
            true
        }
    }
    impl<T> NoReader for Probe<T> {}

    impl<T: serde::de::DeserializeOwned> Probe<T> {
        fn absence_of_a_reader() -> bool {
            false
        }
    }

    // The probe must be able to see a reader that *is* there, or its `true`
    // means nothing.
    assert!(
        !Probe::<BrokerResponse>::absence_of_a_reader(),
        "probe is broken: it reports no reader for a type that has one"
    );
    assert!(
        !Probe::<ActionOutcome>::absence_of_a_reader(),
        "probe is broken: it reports no reader for a type that has one"
    );
    assert!(
        Probe::<BrokerResult>::absence_of_a_reader(),
        "BrokerResult must not be Deserialize: it is a second, lax wire door"
    );

    // And the exact byte sequences the old direct reader accepted are rejected
    // through the one door that remains. Each is the envelope form of what
    // bugs-00 reported, since a bare result object is no longer parseable at all.
    let envelope = |extra: serde_json::Value| {
        let mut json = serde_json::json!({
            "type": BROKER_RESULT_TYPE,
            "protocolVersion": 1,
            "requestId": "req-1",
        });
        for (key, value) in extra.as_object().expect("object").clone() {
            json[key] = value;
        }
        json
    };
    let reported = [
        (
            "failed with an error and a secretKey",
            serde_json::json!({
                "status": "failed",
                "error": { "code": "action_failed", "message": "no" },
                "secretKey": "nsec1deadbeef",
            }),
        ),
        (
            "succeeded beside an error",
            serde_json::json!({
                "status": "succeeded",
                "action": "agents.delete",
                "outcome": { "agentPubkey": PUBKEY, "displayName": "Gone" },
                "error": { "code": "action_failed", "message": "no" },
            }),
        ),
    ];
    for (what, body) in reported {
        let json = envelope(body);
        assert!(
            serde_json::from_value::<BrokerResponse>(json.clone()).is_err(),
            "{what} must not deserialize through the envelope either: {json}"
        );
    }
}

/// Derived coverage for the canonicalization rule, so a *newly added* identity
/// member is covered without anyone remembering to extend a list.
///
/// The two tests above name the members that exist today. This one walks the real
/// fixtures — requests *and* responses, since both directions carry identities
/// through separate code — finds every member whose name marks it as an identity,
/// re-spells its value, and requires the payload to parse back to the canonical
/// value. A field added later with the wrong (or no) `deserialize_with` fails here.
///
/// Matching on the member *name* is the point: the naming convention is what a
/// reviewer sees, so if a member is named like an identity it is held to the
/// identity rule. A member holding an identity under some other name would escape
/// this, which is why the audit above is by type as well.
///
/// The suffix match is case-insensitive on purpose. An earlier revision matched
/// `"EventId"` exactly, which silently skipped the outcome member spelled
/// `eventId` and left every response-side door unpinned — a mutation removing
/// that door survived. Matching how a *reader* groups these names, rather than
/// how one of them happens to be capitalized, is what closes that gap.
#[test]
fn every_identity_shaped_member_is_canonicalized_on_the_wire() {
    /// A member-name suffix and how a sender might legally re-spell its value.
    type Respelling = (&'static str, fn(&str) -> String);

    // Every identity in this contract is hex or a UUID, so case is the
    // re-spelling they all admit; `channelId` additionally admits the forms
    // covered by `one_identity_spelled_two_ways_still_correlates`.
    let respellings: [Respelling; 4] = [
        ("channelid", |v| v.to_ascii_uppercase()),
        ("eventid", |v| v.to_ascii_uppercase()),
        ("pubkey", |v| v.to_ascii_uppercase()),
        ("dtag", |v| v.to_ascii_uppercase()),
    ];

    /// Re-spell every identity-named member of `valid` in turn and require the
    /// payload to parse back to `original`. Returns how many members it checked.
    fn respell_each<T>(valid: &serde_json::Value, original: &T, respellings: &[Respelling]) -> usize
    where
        T: serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let mut checked = 0;
        let mut paths = Vec::new();
        member_paths(valid, "", &mut paths);
        for path in paths {
            let Some(name) = path.rsplit('/').next() else {
                continue;
            };
            let lowered = name.to_ascii_lowercase();
            let Some((_, respell)) = respellings
                .iter()
                .find(|(suffix, _)| lowered.ends_with(suffix))
            else {
                continue;
            };
            let Some(current) = valid
                .pointer(&path)
                .expect("path addresses a member")
                .as_str()
            else {
                continue;
            };
            let respelled = respell(current);
            if respelled == current {
                continue;
            }

            let mut json = valid.clone();
            *json.pointer_mut(&path).expect("path addresses a member") =
                serde_json::Value::String(respelled.clone());
            let parsed: T = serde_json::from_value(json)
                .unwrap_or_else(|e| panic!("\"{respelled}\" at {path} must parse: {e}"));
            assert_eq!(
                &parsed, original,
                "member {path} did not canonicalize \"{respelled}\" back to \"{current}\""
            );
            checked += 1;
        }
        checked
    }

    let mut request_members = 0;
    for args in action_fixtures() {
        let request = BrokerRequest::new("req-canon", args).expect("fixture request builds");
        let valid = serde_json::to_value(&request).expect("request serializes");
        request_members += respell_each(&valid, &request, &respellings);
    }

    // The response side carries identities too — `agents.create` echoes a
    // `channelId`, `storage.address` a `dTag`, the publishing outcomes an
    // `eventId` and an `authorPubkey` — and those doors are separate code from
    // the request side's.
    let keys = Keys::generate();
    let mut response_members = 0;
    for outcome in outcome_fixtures(&keys) {
        let response = BrokerResponse::new("req-canon", BrokerResult::succeeded(outcome));
        let valid = serde_json::to_value(&response).expect("response serializes");
        response_members += respell_each(&valid, &response, &respellings);
    }

    // Guard the guard: a rule that silently matched nothing would pass forever.
    // The two directions are floored *separately* on purpose — one combined
    // total would be satisfied by the request side alone, which is exactly the
    // blind spot that let a response-side door go unpinned.
    assert!(
        request_members >= 8,
        "expected identity members across the request fixtures, checked {request_members}"
    );
    assert!(
        response_members >= 6,
        "expected identity members across the response fixtures, checked {response_members}"
    );
}

// ── Reads carry verifiable provenance ───────────────────────────────────────

/// A read returns the signed event, so a keyless caller can check authorship
/// itself. A host that tampered with content fails verification locally, with no
/// relay involved — which is why this contract does not settle for a projection.
#[test]
fn read_results_are_signed_events_a_keyless_caller_can_verify() {
    let signer = Keys::generate();
    let message = signed_message(&signer);
    message.verify().expect("a genuinely signed event verifies");
    assert_eq!(
        message.author().unwrap().as_str(),
        signer.public_key().to_hex()
    );
    assert_eq!(message.thread().root.as_deref(), Some(EVENT));
    assert_eq!(message.mentions(), vec![PUBKEY.to_string()]);

    // Tamper with the content: the id no longer matches, so verification fails
    // even though every other field is untouched.
    let mut json = serde_json::to_value(&message).unwrap();
    json["content"] = serde_json::json!("a message the author never wrote");
    let tampered: BrokerMessage =
        serde_json::from_value(json).expect("a tampered event still parses");
    assert!(
        tampered.verify().is_err(),
        "tampering must be locally detectable"
    );

    // The wire form is the event's own JSON — no wrapper of its own to disagree
    // with the signed bytes.
    let wire = serde_json::to_value(&message).unwrap();
    assert_eq!(
        keys_of(&wire),
        vec![
            "content",
            "created_at",
            "id",
            "kind",
            "pubkey",
            "sig",
            "tags"
        ]
    );
}

/// The one type here the contract does not own. `nostr`'s `Event` deserializer
/// accepts and discards unknown members, so a genuinely signed event could carry
/// an extra `secretKey` and parse clean — the no-secret rule stopping at the
/// envelope boundary instead of reaching inside it. Deserializing through a
/// `deny_unknown_fields` intermediary closes that, and this drives the injection
/// on a real signed event so nothing is rejected for a bad signature instead.
#[test]
fn an_event_object_cannot_smuggle_a_member_past_the_seven_canonical_ones() {
    let signer = Keys::generate();
    let message = signed_message(&signer);
    let wire = serde_json::to_value(&message).expect("event serializes");

    // The baseline: untouched, this same JSON parses and verifies.
    let parsed: BrokerMessage =
        serde_json::from_value(wire.clone()).expect("a signed event round-trips");
    parsed.verify().expect("and still verifies");

    for extra in ["secretKey", "nsec", "seckey", "credential", "hostNote"] {
        let mut smuggled = wire.clone();
        smuggled[extra] = serde_json::json!("nsec1deadbeef");
        assert!(
            serde_json::from_value::<BrokerMessage>(smuggled.clone()).is_err(),
            "an event carrying \"{extra}\" must not deserialize: {smuggled}"
        );

        // And not through the outcome or the envelope either — the rejection has
        // to hold at every depth a read result travels.
        let outcome = serde_json::json!({
            "action": "channel.read",
            "outcome": { "messages": [smuggled.clone()] },
        });
        assert!(
            serde_json::from_value::<ActionOutcome>(outcome).is_err(),
            "an outcome holding an event with \"{extra}\" must not deserialize"
        );
        let envelope = serde_json::json!({
            "type": BROKER_RESULT_TYPE,
            "protocolVersion": 1,
            "requestId": "req-1",
            "status": "succeeded",
            "action": "channel.read",
            "outcome": { "messages": [smuggled] },
        });
        assert!(
            serde_json::from_value::<BrokerResponse>(envelope).is_err(),
            "a response holding an event with \"{extra}\" must not deserialize"
        );
    }

    // Dropping a canonical member is a parse failure too, not a default.
    for missing in [
        "id",
        "pubkey",
        "created_at",
        "kind",
        "tags",
        "content",
        "sig",
    ] {
        let mut json = wire.clone();
        json.as_object_mut().unwrap().remove(missing);
        assert!(
            serde_json::from_value::<BrokerMessage>(json).is_err(),
            "an event missing \"{missing}\" must not deserialize"
        );
    }
}

#[test]
fn a_page_is_bounded_and_its_cursor_opaque() {
    let signer = Keys::generate();
    let page = |messages: Vec<BrokerMessage>, next_cursor: Option<&str>| {
        ActionOutcome::ChannelRead(MessagePage {
            messages,
            next_cursor: next_cursor.map(str::to_string),
        })
        .validate()
    };
    assert!(page(vec![], None).is_ok());
    assert!(page(vec![signed_message(&signer)], Some("c1")).is_ok());
    assert!(page(vec![], Some("")).is_err());
    assert!(page(vec![], Some("has space")).is_err());
    assert!(page(
        vec![signed_message(&signer); actions::MAX_PAGE_LIMIT as usize + 1],
        None
    )
    .is_err());
}

/// The protocol cap is not the caller's limit. `ActionOutcome::validate` never
/// sees the request, so on its own it would let a host answer a one-message read
/// with five hundred — within the cap, and still an overrun of what was asked.
/// The request's own number is therefore enforced where both halves are in
/// scope, and an absent `limit` is held to [`actions::DEFAULT_PAGE_LIMIT`]
/// rather than treated as consent to an unbounded page.
#[test]
fn a_read_page_is_bounded_by_the_limit_its_own_request_asked_for() {
    let signer = Keys::generate();
    let page = |count: usize| {
        BrokerResult::succeeded(ActionOutcome::ChannelRead(MessagePage {
            messages: vec![signed_message(&signer); count],
            next_cursor: None,
        }))
    };

    // Explicit limits, and the absent case — which is the one a host could
    // otherwise read as "as many as you like".
    for limit in [Some(1_u32), Some(2), Some(actions::MAX_PAGE_LIMIT), None] {
        let args = ChannelReadArgs {
            channel_id: CHANNEL.into(),
            limit,
            ..ChannelReadArgs::default()
        };
        let allowed = limit.unwrap_or(actions::DEFAULT_PAGE_LIMIT) as usize;
        assert_eq!(
            args.effective_limit() as usize,
            allowed,
            "effective_limit must not diverge from the documented default"
        );
        let request = prepared(ActionArgs::ChannelRead(args));

        BrokerResponse::new(request.request_id(), page(allowed))
            .validate_for(&request)
            .unwrap_or_else(|e| panic!("a page exactly at a limit of {allowed} is allowed: {e}"));
        BrokerResponse::new(request.request_id(), page(allowed - 1))
            .validate_for(&request)
            .unwrap_or_else(|e| panic!("a short page is allowed: {e}"));

        // One over is rejected — including one over the default, which is the
        // case an unlimited request would have smuggled through. At the
        // protocol cap the outcome's own bound fires first, which is a rejection
        // for a different (and also correct) reason, so only the message below
        // the cap is pinned to the request's number.
        let over =
            BrokerResponse::new(request.request_id(), page(allowed + 1)).validate_for(&request);
        let error = over.unwrap_err().to_string();
        if allowed < actions::MAX_PAGE_LIMIT as usize {
            assert!(
                error.contains(&format!("limit of {allowed}")),
                "unexpected error for a limit of {allowed}: {error}"
            );
        }
    }

    // The default is a real bound, not the cap under another name: a host that
    // answers an unlimited read with a cap-sized page is still overrunning it.
    const {
        assert!(actions::DEFAULT_PAGE_LIMIT < actions::MAX_PAGE_LIMIT);
    }
    let unlimited = prepared(ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)));
    assert!(BrokerResponse::new(
        unlimited.request_id(),
        page(actions::MAX_PAGE_LIMIT as usize)
    )
    .validate_for(&unlimited)
    .is_err());
}

// ── Results ─────────────────────────────────────────────────────────────────

#[test]
fn failed_and_indeterminate_are_distinct_and_carry_no_outcome() {
    let failed = BrokerResult::failed(BrokerError::new(
        BrokerErrorCode::ActionFailed,
        "runtime not installed",
    ));
    let failed_json = serde_json::to_value(BrokerResponse::new("r", failed.clone())).unwrap();
    assert_eq!(failed_json["status"], "failed");
    assert_eq!(failed_json["error"]["code"], "action_failed");
    assert!(failed_json.get("outcome").is_none());

    let indeterminate = BrokerResult::indeterminate(BrokerError::new(
        BrokerErrorCode::OutcomeUnknown,
        "host restarted mid-execution",
    ));
    let json = serde_json::to_value(BrokerResponse::new("r", indeterminate.clone())).unwrap();
    assert_eq!(json["status"], "indeterminate");
    assert_eq!(json["error"]["code"], "outcome_unknown");
    assert!(json.get("outcome").is_none());

    assert_ne!(failed, indeterminate);
    assert!(failed.outcome().is_none());
    assert!(indeterminate.outcome().is_none());
}

/// A code and a status are two statements about the same fact — whether side
/// effects landed — so the contract fixes which pairings are meaningful and
/// rejects the rest. Driven across every code × both statuses, so adding a code
/// forces a decision here.
#[test]
fn status_and_error_code_must_agree_about_side_effects() {
    use BrokerErrorCode as E;
    for code in all_error_codes() {
        let failed =
            BrokerResponse::new("req-1", BrokerResult::failed(BrokerError::new(code, "?")))
                .validate();
        let indeterminate = BrokerResponse::new(
            "req-1",
            BrokerResult::indeterminate(BrokerError::new(code, "?")),
        )
        .validate();

        // The table, spelled out independently of the predicates it checks: a
        // second copy is the point, since a test that asked `may_be_failed()`
        // would pass for any implementation of it. Exhaustive with no wildcard,
        // so a new code cannot inherit an answer — it must be decided here too.
        let (failed_ok, indeterminate_ok) = match code {
            E::InvalidRequest
            | E::UnsupportedProtocolVersion
            | E::UnknownAction
            | E::UnsupportedActionVersion
            | E::Unsupported
            | E::Unauthenticated
            | E::Unauthorized
            | E::RequestIdConflict
            | E::ActionFailed => (true, false),
            E::OutcomeUnknown => (false, true),
            E::Internal => (true, true),
        };

        assert_eq!(
            failed.is_ok(),
            failed_ok,
            "{} with a failed status: {failed:?}",
            code.as_str()
        );
        assert_eq!(
            indeterminate.is_ok(),
            indeterminate_ok,
            "{} with an indeterminate status: {indeterminate:?}",
            code.as_str()
        );
        assert_eq!(code.may_be_failed(), failed_ok);
        assert_eq!(code.may_be_indeterminate(), indeterminate_ok);
    }

    // The two directions review found, named: a rejected credential is a
    // known-fate refusal and cannot claim not to know, and `outcome_unknown`
    // cannot claim a clean failure.
    let error = BrokerResponse::new(
        "req-1",
        BrokerResult::indeterminate(BrokerError::new(E::Unauthenticated, "credential rejected")),
    )
    .validate()
    .unwrap_err()
    .to_string();
    assert!(error.contains("unauthenticated"), "unexpected: {error}");
    let error = BrokerResponse::new(
        "req-1",
        BrokerResult::failed(BrokerError::new(E::OutcomeUnknown, "?")),
    )
    .validate()
    .unwrap_err()
    .to_string();
    assert!(error.contains("outcome_unknown"), "unexpected: {error}");
}

#[test]
fn replay_metadata_rides_the_response_not_the_result() {
    let result = BrokerResult::succeeded(ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
        agent_pubkey: pubkey(),
        display_name: "Gone".into(),
    }));
    let fresh = BrokerResponse::new("req-9", result.clone());
    let replayed = BrokerResponse::new("req-9", result.clone()).replayed();

    // The domain outcome is identical; only the delivery metadata differs.
    assert_eq!(fresh.result, replayed.result);
    assert!(!fresh.replayed);
    assert!(replayed.replayed);
    assert_eq!(
        serde_json::to_value(&replayed).unwrap()["replayed"],
        serde_json::json!(true)
    );

    // `replayed` is not part of the stored result encoding.
    assert!(serde_json::to_value(&result)
        .unwrap()
        .get("replayed")
        .is_none());
}

/// A response that validates in isolation can still be the wrong answer. This is
/// the check that makes a mismatched outcome unusable rather than merely
/// surprising.
#[test]
fn response_validation_is_request_aware() {
    let signer = Keys::generate();
    let request = prepared(ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)));
    let page = ActionOutcome::ChannelRead(MessagePage {
        messages: vec![signed_message(&signer)],
        next_cursor: None,
    });

    BrokerResponse::new(request.request_id(), BrokerResult::succeeded(page.clone()))
        .validate_for(&request)
        .expect("the right outcome for the right request");

    // Wrong action: a post receipt is not an answer to a read.
    let wrong_action = BrokerResponse::new(
        request.request_id(),
        BrokerResult::succeeded(ActionOutcome::MessagePost(EventPublished {
            event_id: EVENT.into(),
            kind: 9,
            created_at: 1,
        })),
    );
    wrong_action
        .validate()
        .expect("it is well-formed on its own — that is the point");
    let error = wrong_action.validate_for(&request).unwrap_err().to_string();
    assert!(error.contains("message.post"), "unexpected: {error}");

    // Wrong correlation id.
    let error = BrokerResponse::new("req-other", BrokerResult::succeeded(page))
        .validate_for(&request)
        .unwrap_err()
        .to_string();
    assert!(error.contains("requestId"), "unexpected: {error}");

    // Malformed identifiers inside an otherwise well-shaped outcome.
    let bad_id = BrokerResponse::new(
        request.request_id(),
        BrokerResult::succeeded(ActionOutcome::ChannelRead(MessagePage {
            messages: vec![],
            next_cursor: Some("not a cursor".into()),
        })),
    );
    assert!(bad_id.validate_for(&request).is_err());

    let post = prepared(ActionArgs::MessagePost(MessagePostArgs {
        channel_id: CHANNEL.into(),
        content: "hi".into(),
        mentions: vec![],
    }));
    let bad_event_id = BrokerResponse::new(
        post.request_id(),
        BrokerResult::succeeded(ActionOutcome::MessagePost(EventPublished {
            event_id: "nothex".into(),
            kind: 9,
            created_at: 1,
        })),
    );
    assert!(bad_event_id.validate_for(&post).is_err());

    // A failure needs no outcome to match, only correlation.
    BrokerResponse::new(
        request.request_id(),
        BrokerResult::failed(BrokerError::unauthorized("not your channel")),
    )
    .validate_for(&request)
    .expect("a refusal answers any action");
}

// ── Retry is identical bytes ────────────────────────────────────────────────

/// The retry contract is byte identity, so the client takes frozen bytes rather
/// than a typed value it would have to reserialize. Preparing once and reading
/// `body()` twice is the only way to send the same request twice.
#[test]
fn preparing_a_request_freezes_the_bytes_every_attempt_sends() {
    let request = BrokerRequest::new(
        "req-idem",
        ActionArgs::MessagePost(MessagePostArgs {
            channel_id: CHANNEL.into(),
            content: "exactly once".into(),
            mentions: vec![pubkey()],
        }),
    )
    .unwrap();
    let prepared = request.clone().prepare().expect("valid request prepares");

    assert_eq!(
        prepared.body(),
        prepared.body(),
        "body is frozen, not re-rendered"
    );
    // Correlation metadata is all a transport gets. There is deliberately no
    // accessor for the typed request: one would let an implementation serialize
    // the value a second time, which is the possibility freezing removes.
    assert_eq!(prepared.request_id(), "req-idem");
    assert_eq!(prepared.action(), Action::MessagePost);

    // The frozen bytes are the envelope, and they parse back to the same value.
    let parsed: BrokerRequest =
        serde_json::from_slice(prepared.body()).expect("frozen body is the envelope");
    assert_eq!(parsed, request);

    // Preparing validates, so an invalid request never reaches a transport.
    let invalid = BrokerRequest {
        r#type: BROKER_REQUEST_TYPE.into(),
        protocol_version: 99,
        request_id: "req-bad".into(),
        action_version: 1,
        action: ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Pubkey(pubkey()),
        }),
    };
    assert!(invalid.prepare().is_err());
}

/// The hand-written [`BrokerErrorCode::as_str`] and serde's derived name are two
/// encodings of one wire string, so each is pinned against the other and the
/// whole set is pinned against this literal — a rename in either fails here.
/// This is also what pins [`all_error_codes`] against the enum: a new variant
/// missing from that fixture changes the joined string and fails here.
#[test]
fn error_codes_have_stable_wire_strings() {
    let codes = all_error_codes();
    for code in codes {
        assert_eq!(
            serde_json::to_value(code).unwrap(),
            serde_json::json!(code.as_str()),
            "as_str and the serde name must not drift"
        );
    }
    assert_eq!(
        codes.map(BrokerErrorCode::as_str).join(","),
        "invalid_request,unsupported_protocol_version,unknown_action,\
unsupported_action_version,unsupported,unauthenticated,unauthorized,\
request_id_conflict,action_failed,outcome_unknown,internal"
    );
}

// ── Client trait ────────────────────────────────────────────────────────────

/// A test double, and the only implementation in this crate. It exists to prove
/// the trait is object-safe and usable behind `dyn`, which is what lets an
/// in-process host and an HTTP client be interchangeable.
///
/// Note what it does *not* do: it never calls `validate_for`. It cannot — it has
/// no way to build a [`ValidatedResponse`] except through the blanket
/// [`BrokerClientExt::execute`], which is the whole point of splitting the
/// trait. A deliberately hostile implementation is still forced through the
/// same check.
struct DoubleBroker {
    response: Result<BrokerResponse, BrokerTransportError>,
}

impl BrokerClient for DoubleBroker {
    fn send<'a>(&'a self, request: &'a PreparedRequest, _: Dispatch) -> BrokerFuture<'a> {
        // A real implementation sends `request.body()` verbatim. The double
        // stands in for a host that answers under the id it was asked with, and
        // returns the envelope unjudged.
        let response = self.response.clone().map(|mut response| {
            response.request_id = request.request_id().to_string();
            response
        });
        Box::pin(async move { response })
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    // A hand-rolled park-free executor: the double's future is always ready, so
    // one poll suffices and pulling in a runtime would be the heavier choice.
    use std::task::{Context, Poll, Wake, Waker};
    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: std::sync::Arc<Self>) {}
    }
    let waker = Waker::from(std::sync::Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("test double must not park"),
    }
}

#[test]
fn the_client_trait_is_object_safe_and_returns_a_validated_host_verdict() {
    let request = prepared(ActionArgs::ChannelRead(ChannelReadArgs {
        channel_id: CHANNEL.into(),
        mentions_only: true,
        ..ChannelReadArgs::default()
    }));

    let succeeded: Box<dyn BrokerClient> = Box::new(DoubleBroker {
        response: Ok(BrokerResponse::new(
            "placeholder",
            BrokerResult::succeeded(ActionOutcome::ChannelRead(MessagePage {
                messages: vec![],
                next_cursor: None,
            })),
        )),
    });
    // `execute` is available on `dyn BrokerClient` and is the only way to a
    // `ValidatedResponse` — the caller does no correlation of its own.
    let response = block_on(succeeded.execute(&request)).expect("double answers");
    assert_eq!(response.request_id(), "req-1");
    assert!(response.result().outcome().is_some());
    assert!(!response.replayed());

    // A refusal — including a rejected credential — is still an answer: `Ok`
    // with the verdict in the envelope.
    for code in [
        BrokerErrorCode::Unauthorized,
        BrokerErrorCode::Unauthenticated,
    ] {
        let refused: Box<dyn BrokerClient> = Box::new(DoubleBroker {
            response: Ok(BrokerResponse::new(
                "placeholder",
                BrokerResult::failed(BrokerError::new(code, "no")),
            )),
        });
        let response =
            block_on(refused.execute(&request)).expect("a refusal is not a transport error");
        assert_eq!(response.result().error().map(|e| e.code), Some(code));
    }

    // No usable answer at all is a transport error, and says nothing about side
    // effects. An intermediary's status is operator detail, not a verdict.
    for error in [
        BrokerTransportError::Unreachable("connection reset".into()),
        BrokerTransportError::NoEnvelope {
            status: 401,
            detail: "proxy denied".into(),
        },
        BrokerTransportError::MalformedResponse("not json".into()),
    ] {
        let broken: Box<dyn BrokerClient> = Box::new(DoubleBroker {
            response: Err(error.clone()),
        });
        assert_eq!(block_on(broken.execute(&request)).unwrap_err(), error);
    }
}

/// The double returns whatever it is given, unvalidated — a hostile client
/// cannot do otherwise. `execute` is still the only door, so the mismatch
/// surfaces as a transport failure and never reaches a caller as `Ok`.
#[test]
fn a_client_cannot_hand_back_a_response_that_answers_a_different_request() {
    let request = prepared(ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)));

    // Wrong action for this request.
    let confused: Box<dyn BrokerClient> = Box::new(DoubleBroker {
        response: Ok(BrokerResponse::new(
            "placeholder",
            BrokerResult::succeeded(ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
                agent_pubkey: pubkey(),
                display_name: "Gone".into(),
            })),
        )),
    });
    // The envelope is well-formed in isolation — that is exactly why `send`
    // cannot be the caller's door. `execute` is the only reachable one (a
    // `Dispatch` token cannot be built outside the client module), and it
    // rejects the mismatch rather than passing it on.
    assert!(matches!(
        block_on(confused.execute(&request)).unwrap_err(),
        BrokerTransportError::MalformedResponse(_)
    ));

    // Malformed identifiers inside an otherwise well-shaped outcome, too.
    let bad_cursor: Box<dyn BrokerClient> = Box::new(DoubleBroker {
        response: Ok(BrokerResponse::new(
            "placeholder",
            BrokerResult::succeeded(ActionOutcome::ChannelRead(MessagePage {
                messages: vec![],
                next_cursor: Some("not a cursor".into()),
            })),
        )),
    });
    assert!(matches!(
        block_on(bad_cursor.execute(&request)).unwrap_err(),
        BrokerTransportError::MalformedResponse(_)
    ));

    // A status contradicting its own code, which is how the review reached this:
    // `unauthenticated` is a known pre-dispatch refusal, so claiming not to know
    // the fate is not a verdict `execute` may pass on as `Ok`.
    let contradictory: Box<dyn BrokerClient> = Box::new(DoubleBroker {
        response: Ok(BrokerResponse::new(
            "placeholder",
            BrokerResult::indeterminate(BrokerError::new(
                BrokerErrorCode::Unauthenticated,
                "credential rejected",
            )),
        )),
    });
    assert!(matches!(
        block_on(contradictory.execute(&request)).unwrap_err(),
        BrokerTransportError::MalformedResponse(_)
    ));
}

/// A second double, parsing bytes the way a real HTTP client does, because the
/// strict-envelope and strict-event guards live in `Deserialize` and the typed
/// double above can never exercise them: it hands back a value that was never on
/// a wire.
///
/// This is the shape the bug actually had — bytes arriving from a host — and what
/// the caller sees now is [`BrokerTransportError::MalformedResponse`], not an
/// `Ok` whose extra members were quietly dropped.
struct WireBroker {
    body: Vec<u8>,
}

impl BrokerClient for WireBroker {
    fn send<'a>(&'a self, _: &'a PreparedRequest, _: Dispatch) -> BrokerFuture<'a> {
        // Exactly a transport's job: parse an envelope, and report the absence
        // of one as a transport failure.
        let parsed = serde_json::from_slice::<BrokerResponse>(&self.body)
            .map_err(|e| BrokerTransportError::MalformedResponse(e.to_string()));
        Box::pin(async move { parsed })
    }
}

#[test]
fn bytes_carrying_more_than_the_contract_declares_never_reach_a_caller_as_ok() {
    let signer = Keys::generate();
    let request = prepared(ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)));
    let event = serde_json::to_value(signed_message(&signer)).expect("event serializes");
    let envelope = || {
        serde_json::json!({
            "type": BROKER_RESULT_TYPE,
            "protocolVersion": 1,
            "requestId": request.request_id(),
            "status": "succeeded",
            "action": "channel.read",
            "outcome": { "messages": [event.clone()] },
        })
    };

    // The honest bytes are accepted, so the rejections below are about the
    // smuggled members and not about this fixture being unparseable.
    let client = WireBroker {
        body: serde_json::to_vec(&envelope()).unwrap(),
    };
    let response = block_on(client.execute(&request)).expect("honest bytes are an answer");
    assert!(response.result().outcome().is_some());

    // A key at each depth: on the envelope, inside the outcome, and inside the
    // signed event — the last being the one `nostr` would have discarded.
    let mut on_envelope = envelope();
    on_envelope["secretKey"] = serde_json::json!("nsec1deadbeef");
    let mut in_outcome = envelope();
    in_outcome["outcome"]["secretKey"] = serde_json::json!("nsec1deadbeef");
    let mut in_event = envelope();
    in_event["outcome"]["messages"][0]["secretKey"] = serde_json::json!("nsec1deadbeef");
    // And the contradiction the envelope could previously hold on the wire.
    let mut error_beside_success = envelope();
    error_beside_success["error"] = serde_json::json!({ "code": "internal", "message": "?" });

    for (what, json) in [
        ("on the envelope", on_envelope),
        ("inside the outcome", in_outcome),
        ("inside the event", in_event),
        ("an error beside a success", error_beside_success),
    ] {
        let client = WireBroker {
            body: serde_json::to_vec(&json).unwrap(),
        };
        assert!(
            matches!(
                block_on(client.execute(&request)),
                Err(BrokerTransportError::MalformedResponse(_))
            ),
            "{what}: must not reach the caller as Ok"
        );
    }
}
