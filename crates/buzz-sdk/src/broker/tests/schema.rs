//! Wire schema: action coverage, envelope round trips, and envelope rejection.
//!
//! The exact-key-set tables that make the no-secret invariant enforceable live
//! in the sibling `wire_schema` module.

use super::*;

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
        parsed.validate().expect("round-tripped request is valid");
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
pub(super) fn member_paths(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
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
        let error = request.validate().unwrap_err().to_string();
        assert!(error.contains("protocolVersion"), "unexpected: {error}");

        let mut response = BrokerResponse::new("req-1", failed());
        response.protocol_version = bad;
        assert!(response.validate().is_err());
    }

    let mut wrong_action_version = BrokerRequest::new("req-1", args()).unwrap();
    wrong_action_version.action_version = 7;
    let error = wrong_action_version.validate().unwrap_err().to_string();
    assert!(error.contains("actionVersion"), "unexpected: {error}");

    let mut wrong_request_type = BrokerRequest::new("req-1", args()).unwrap();
    wrong_request_type.r#type = BROKER_RESULT_TYPE.into();
    assert!(wrong_request_type.validate().is_err());

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
