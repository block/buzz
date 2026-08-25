//! Argument validation, read provenance, and result semantics.
//!
//! The identity-canonicalization guards live in the sibling `identities` module.

use super::*;

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
