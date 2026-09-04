use super::*;
use crate::placement::{PlacementProjection, TargetIntent};
use nostr::JsonUtil;

fn payload(owner: &Keys) -> Payload {
    Payload {
        v: 1,
        community: "wss://relay.example".into(),
        owner: owner.public_key(),
        agent: Keys::generate().public_key(),
        host: Keys::generate().public_key(),
        request: Uuid::new_v4(),
        action: Action::Start,
    }
}

fn decode(event: &Event, owner: &Keys, p: &Payload) -> Result<DecodedIntent, Error> {
    decode_event(event, owner, &p.community, p.agent, &[p.host])
}

// Deliberately bypass the producer's validation to exercise the receive boundary.
fn raw(owner: &Keys, text: &str) -> Event {
    let content = nip44::encrypt(
        owner.secret_key(),
        &owner.public_key(),
        text,
        nip44::Version::V2,
    )
    .unwrap();
    resign(
        owner,
        Kind::Custom(KIND_PLACEMENT_INTENT as u16),
        content,
        vec![Tag::parse(["L", NAMESPACE]).unwrap()],
    )
}

fn resign(owner: &Keys, kind: Kind, content: String, tags: Vec<Tag>) -> Event {
    EventBuilder::new(kind, content)
        .tags(tags)
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(owner)
        .unwrap()
}

#[test]
fn owner_desktops_observe_the_same_cross_host_identity_without_host_keys() {
    let owner = Keys::generate();
    let x = Keys::generate();
    let y = Keys::generate();
    let mut p = payload(&owner);
    p.host = x.public_key();
    let start_x = build_event(&owner, &p, 100).unwrap();
    p.host = y.public_key();
    p.request = Uuid::new_v4();
    let start_y = build_event(&owner, &p, 101).unwrap();
    p.host = x.public_key();
    p.request = Uuid::new_v4();
    p.action = Action::Stop;
    let stop_x = build_event(&owner, &p, 102).unwrap();
    let hosts = [x.public_key(), y.public_key()];
    let read = |event: &Event| {
        decode_event(event, &owner, &p.community, p.agent, &hosts)
            .unwrap()
            .placement()
    };
    let a = [read(&start_x), read(&start_y), read(&stop_x)];
    let b = [read(&stop_x), read(&start_y), read(&start_x)];
    for intents in [&a, &b] {
        let projection = PlacementProjection::new(intents);
        assert_eq!(
            projection.desired(),
            Some((y.public_key(), EventOrder::from_event(&start_y)))
        );
        assert!(matches!(
            projection.target(x.public_key()),
            TargetIntent::Stopped(_)
        ));
        assert!(projection.retains_start(y.public_key(), EventOrder::from_event(&start_y)));
    }
    // Exact persisted bytes survive retry; no per-recipient re-signing.
    let retry = Event::from_json(start_y.as_json()).unwrap();
    assert_eq!(read(&retry), read(&start_y));
    for host in [&x, &y] {
        assert_eq!(decode(&start_y, host, &p), Err(Error::Scope));
        assert!(nip44::decrypt(host.secret_key(), &owner.public_key(), &start_y.content).is_err());
    }
}

#[test]
fn rejects_each_foreign_scope_and_unbound_target() {
    let owner = Keys::generate();
    let p = payload(&owner);
    let event = build_event(&owner, &p, 100).unwrap();
    let decoded = decode(&event, &owner, &p).unwrap();
    assert_eq!(decoded.payload(), &p);
    assert_eq!(decoded.placement().order.event_id(), event.id);
    assert_eq!(decoded.placement().host, p.host);
    assert_eq!(decoded.placement().action, PlacementAction::Start);
    let foreign = Keys::generate();
    assert_eq!(build_event(&foreign, &p, 100), Err(Error::Scope));
    assert_eq!(decode(&event, &foreign, &p), Err(Error::Scope));
    assert_eq!(
        decode_event(&event, &owner, "wss://other.example", p.agent, &[p.host]),
        Err(Error::Scope)
    );
    assert_eq!(
        decode_event(
            &event,
            &owner,
            &p.community,
            foreign.public_key(),
            &[p.host]
        ),
        Err(Error::Scope)
    );
    assert_eq!(
        decode_event(&event, &owner, &p.community, p.agent, &[]),
        Err(Error::Scope)
    );
    assert_eq!(
        decode_event(
            &event,
            &owner,
            &p.community,
            p.agent,
            &[foreign.public_key()]
        ),
        Err(Error::Scope)
    );
    let mut spoof = p.clone();
    spoof.owner = foreign.public_key();
    assert_eq!(
        decode(
            &raw(&owner, &serde_json::to_string(&spoof).unwrap()),
            &owner,
            &p
        ),
        Err(Error::Scope)
    );
}

#[test]
fn rejects_hash_signature_ciphertext_and_envelope_tampering() {
    let owner = Keys::generate();
    let p = payload(&owner);
    let event = build_event(&owner, &p, 100).unwrap();
    let mut bad_hash = event.clone();
    bad_hash.created_at = Timestamp::from(101);
    assert!(bad_hash.verify_signature()); // signature alone is insufficient
    assert_eq!(decode(&bad_hash, &owner, &p), Err(Error::Envelope));
    let mut bad_sig = event.clone();
    bad_sig.sig = build_event(&owner, &p, 102).unwrap().sig;
    assert!(bad_sig.verify_id());
    assert_eq!(decode(&bad_sig, &owner, &p), Err(Error::Envelope));
    for (kind, tags) in [
        (Kind::TextNote, event.tags.clone().to_vec()),
        (event.kind, vec![]),
        (
            event.kind,
            vec![Tag::parse(["L", "buzz.host.execution.v1"]).unwrap()],
        ),
        (
            event.kind,
            vec![Tag::parse(["L", NAMESPACE, "extra"]).unwrap()],
        ),
        (event.kind, vec![Tag::parse(["L", NAMESPACE]).unwrap(); 2]),
    ] {
        assert_eq!(
            decode(
                &resign(&owner, kind, event.content.clone(), tags),
                &owner,
                &p
            ),
            Err(Error::Envelope)
        );
    }
    let forged_ciphertext = resign(
        &owner,
        event.kind,
        "not ciphertext".into(),
        event.tags.clone().to_vec(),
    );
    assert_eq!(decode(&forged_ciphertext, &owner, &p), Err(Error::Crypto));
    let oversized = resign(
        &owner,
        event.kind,
        "a".repeat(MAX_CIPHERTEXT + 1),
        event.tags.clone().to_vec(),
    );
    assert_eq!(decode(&oversized, &owner, &p), Err(Error::Envelope));
}

#[test]
fn rejects_legacy_versions_duplicates_unknown_fields_and_invalid_payloads() {
    let owner = Keys::generate();
    let p = payload(&owner);
    let json = serde_json::to_string(&p).unwrap();
    let cases = [
        json.replacen("\"v\":1", "\"v\":2", 1),
        json.replacen("\"v\":1", "\"v\":1,\"v\":1", 1),
        json.replacen("\"v\":1", "\"v\":1,\"run\":\"old-generation\"", 1),
        json.replace("\"start\"", "{\"action\":\"stop\",\"run\":\"old\"}"),
        json.replace("\"start\"", "\"restart\""),
        json.replace("wss://relay.example", "WSS://relay.example/"),
        json.replace("wss://relay.example", "wss://user@relay.example"),
        json.replace(&p.request.to_string(), &Uuid::nil().to_string()),
        json.replace(&p.agent.to_hex(), "invalid-key"),
        format!("{json} trailing"),
        " ".repeat(MAX_PLAINTEXT + 1),
    ];
    for text in cases {
        assert_eq!(decode(&raw(&owner, &text), &owner, &p), Err(Error::Payload));
    }
    for invalid in [
        Payload { v: 2, ..p.clone() },
        Payload {
            community: "https://relay.example".into(),
            ..p.clone()
        },
        Payload {
            community: format!("wss://relay.example/{}", "x".repeat(512)),
            ..p.clone()
        },
        Payload {
            request: Uuid::nil(),
            ..p.clone()
        },
    ] {
        assert_eq!(build_event(&owner, &invalid, 100), Err(Error::Payload));
    }
}

#[test]
fn signed_seconds_and_lower_id_not_arrival_or_request_id_order() {
    let owner = Keys::generate();
    let p = payload(&owner);
    let first = build_event(&owner, &p, 100).unwrap();
    let second = build_event(&owner, &p, 100).unwrap();
    assert_ne!(first.id, second.id); // rebuilding is NOT a retry
    let a = decode(&first, &owner, &p).unwrap().placement();
    let b = decode(&second, &owner, &p).unwrap().placement();
    assert_eq!(a.order > b.order, first.id < second.id);
    let future = build_event(&owner, &p, u64::MAX).unwrap();
    let f = decode(&future, &owner, &p).unwrap().placement();
    assert!(f.order > a.order); // desired state is not expired command admission
    let intents = [f, b, a, f];
    assert_eq!(
        PlacementProjection::new(&intents).desired(),
        Some((p.host, f.order))
    );
    assert!(!crate::kind::is_replaceable(KIND_PLACEMENT_INTENT));
    assert!(!crate::kind::is_parameterized_replaceable(
        KIND_PLACEMENT_INTENT
    ));
    assert!(!crate::kind::is_ephemeral(KIND_PLACEMENT_INTENT));
}
