//! One spelling per identity, at every door, plus the result type's absent reader.
//!
//! Split from `validation` on size alone: with these guards inline that module
//! lands over the repo's 1,000-line ceiling, which is the cap this split exists
//! to satisfy.

use super::schema::member_paths;
use super::*;

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
