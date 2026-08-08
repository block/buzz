use url::Url;

use super::{
    parse_add_community_deep_link, parse_import_claim_deep_link, parse_join_deep_link,
    parse_join_slack_deep_link, parse_message_deep_link, parse_nostr_bind_deep_link,
    PendingCommunityDeepLink, PendingCommunityDeepLinks, PendingImportClaimDeepLink,
    PendingImportClaimDeepLinks,
};

#[test]
fn parse_join_slack_extracts_relay_and_service() {
    let url = Url::parse(
        "buzz://join-slack?relay=wss%3A%2F%2Frelay.example&service=https%3A%2F%2Fmig.example",
    )
    .unwrap();
    let p = parse_join_slack_deep_link(&url).unwrap();
    assert_eq!(p.relay_url, "wss://relay.example");
    assert_eq!(p.service, "https://mig.example");
}

#[test]
fn parse_join_slack_rejects_missing_relay_or_bad_service() {
    // missing relay
    assert!(parse_join_slack_deep_link(
        &Url::parse("buzz://join-slack?service=https%3A%2F%2Fmig.example").unwrap()
    )
    .is_err());
    // missing service
    assert!(parse_join_slack_deep_link(
        &Url::parse("buzz://join-slack?relay=wss%3A%2F%2Frelay.example").unwrap()
    )
    .is_err());
    // non-http service
    assert!(parse_join_slack_deep_link(
        &Url::parse("buzz://join-slack?relay=wss%3A%2F%2Fr.example&service=file%3A%2F%2Fx")
            .unwrap()
    )
    .is_err());
    // Remote services must use TLS; local development may use loopback HTTP.
    assert!(parse_join_slack_deep_link(
        &Url::parse(
            "buzz://join-slack?relay=wss%3A%2F%2Fr.example&service=http%3A%2F%2Fmig.example"
        )
        .unwrap()
    )
    .is_err());
    assert!(parse_join_slack_deep_link(
        &Url::parse(
            "buzz://join-slack?relay=ws%3A%2F%2Flocalhost%3A3000&service=http%3A%2F%2Flocalhost%3A8787"
        )
        .unwrap()
    )
    .is_ok());
    // The service is an origin; appending endpoints to a path prefix is unsafe.
    assert!(parse_join_slack_deep_link(
        &Url::parse(
            "buzz://join-slack?relay=wss%3A%2F%2Fr.example&service=https%3A%2F%2Fmig.example%2Fprefix"
        )
        .unwrap()
    )
    .is_err());
}

#[test]
fn parse_import_claim_email_channel() {
    let url = Url::parse(
            "buzz://import-claim?subject=slack:U060&token=v1.aa.bb.cc.dd&service=https%3A%2F%2Fmig.example",
        )
        .unwrap();
    let p = parse_import_claim_deep_link(&url).unwrap();
    assert_eq!(p.subject, "slack:U060");
    assert_eq!(p.token.as_deref(), Some("v1.aa.bb.cc.dd"));
    assert_eq!(p.service.as_deref(), Some("https://mig.example"));
    assert_eq!(p.via, None);
    assert_eq!(p.relay_url, None);
}

#[test]
fn parse_import_claim_oidc_channel() {
    let url = Url::parse(
            "buzz://import-claim?subject=slack:U060&via=oidc&code=abc123&relay=wss%3A%2F%2Frelay.example&service=https%3A%2F%2Fmig.example",
        )
        .unwrap();
    let p = parse_import_claim_deep_link(&url).unwrap();
    assert_eq!(p.subject, "slack:U060");
    assert_eq!(p.via.as_deref(), Some("oidc"));
    assert_eq!(p.token, None);
    assert_eq!(p.service.as_deref(), Some("https://mig.example"));
    assert_eq!(p.relay_url.as_deref(), Some("wss://relay.example"));
    assert_eq!(p.code.as_deref(), Some("abc123"));
}

#[test]
fn parse_import_claim_rejects_incomplete_and_malformed() {
    // Neither channel identifiable (subject only).
    assert!(parse_import_claim_deep_link(
        &Url::parse("buzz://import-claim?subject=slack:U060").unwrap()
    )
    .is_err());
    assert!(parse_import_claim_deep_link(
        &Url::parse("buzz://import-claim?subject=slack:U060&via=oidc").unwrap()
    )
    .is_err());
    // OIDC with relay + service but no finalize code.
    assert!(parse_import_claim_deep_link(
        &Url::parse(
            "buzz://import-claim?subject=slack:U060&via=oidc&relay=wss%3A%2F%2Fr.example&service=https%3A%2F%2Fmig.example"
        )
        .unwrap()
    )
    .is_err());
    // token without service.
    assert!(parse_import_claim_deep_link(
        &Url::parse("buzz://import-claim?subject=slack:U060&token=t").unwrap()
    )
    .is_err());
    // Malformed subject (no source).
    assert!(parse_import_claim_deep_link(
        &Url::parse("buzz://import-claim?subject=U060&via=oidc").unwrap()
    )
    .is_err());
    // Non-http service.
    assert!(parse_import_claim_deep_link(
        &Url::parse("buzz://import-claim?subject=slack:U060&token=t&service=file%3A%2F%2Fx")
            .unwrap()
    )
    .is_err());
    // A query or fragment would make the app append /oidc/start or
    // /email/complete at the wrong location.
    for service in [
        "https%3A%2F%2Fmig.example%3Fnext%3Devil",
        "https%3A%2F%2Fmig.example%23fragment",
        "https%3A%2F%2Fmig.example%2Fprefix",
        "http%3A%2F%2Fmig.example",
    ] {
        assert!(parse_import_claim_deep_link(
            &Url::parse(&format!(
                "buzz://import-claim?subject=slack:U060&token=t&service={service}"
            ))
            .unwrap()
        )
        .is_err());
    }
}

#[test]
fn pending_import_claims_dedupe_and_acknowledge() {
    let queue = PendingImportClaimDeepLinks::default();
    let payload = parse_import_claim_deep_link(
            &Url::parse(
                "buzz://import-claim?subject=slack:U060&via=oidc&code=abc123&relay=wss%3A%2F%2Frelay.example&service=https%3A%2F%2Fmig.example",
            )
            .unwrap(),
        )
        .unwrap();
    queue.enqueue(PendingImportClaimDeepLink {
        request_id: "first".into(),
        payload: payload.clone(),
    });
    queue.enqueue(PendingImportClaimDeepLink {
        request_id: "duplicate".into(),
        payload,
    });

    assert_eq!(queue.first().unwrap().request_id, "first");
    assert!(!queue.acknowledge("wrong"));
    assert!(queue.acknowledge("first"));
    assert!(queue.first().is_none());
}

fn pending(id: &str, relay_url: &str, code: Option<&str>) -> PendingCommunityDeepLink {
    PendingCommunityDeepLink {
        id: id.to_owned(),
        kind: if code.is_some() { "join" } else { "connect" }.to_owned(),
        relay_url: relay_url.to_owned(),
        code: code.map(str::to_owned),
        policy_receipt: None,
        name: None,
        service: None,
    }
}

#[test]
fn pending_join_serializes_policy_receipt_for_cold_launch_recovery() {
    let mut link = pending("join", "wss://relay.example", Some("invite"));
    link.policy_receipt = Some("relay-signed-receipt".to_owned());

    let payload = serde_json::to_value(link).unwrap();
    assert_eq!(payload["policyReceipt"], "relay-signed-receipt");
}

#[test]
fn pending_community_links_are_fifo_and_acknowledged_in_order() {
    let queue = PendingCommunityDeepLinks::default();
    queue.enqueue(pending("first", "wss://one.example", Some("one")));
    queue.enqueue(pending("second", "wss://two.example", Some("two")));
    assert_eq!(queue.first().unwrap().id, "first");
    assert!(!queue.acknowledge("second"));
    assert!(queue.acknowledge("first"));
    assert_eq!(queue.first().unwrap().id, "second");
}

#[test]
fn pending_community_links_dedupe_exact_intents() {
    let queue = PendingCommunityDeepLinks::default();
    queue.enqueue(pending("first", "wss://one.example", Some("one")));
    queue.enqueue(pending("duplicate", "wss://one.example", Some("one")));
    assert!(queue.acknowledge("first"));
    assert!(queue.first().is_none());
}

fn valid_nostr_bind_url() -> Url {
    Url::parse(
            "buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard",
        )
        .unwrap()
}

#[test]
fn parse_add_community_deep_link_extracts_relay_and_name() {
    let url = Url::parse(
            "buzz://add-community?relay=wss%3A%2F%2Facme.communities.buzz.xyz&name=Acme%20Team&ignored=value",
        )
        .unwrap();
    let payload = parse_add_community_deep_link(&url).unwrap();
    assert_eq!(payload.relay_url, "wss://acme.communities.buzz.xyz");
    assert_eq!(payload.name.as_deref(), Some("Acme Team"));
}

#[test]
fn parse_add_community_deep_link_accepts_an_omitted_or_empty_name() {
    for raw in [
        "buzz://add-community?relay=wss%3A%2F%2Facme.example",
        "buzz://add-community?relay=wss%3A%2F%2Facme.example&name=",
    ] {
        assert!(parse_add_community_deep_link(&Url::parse(raw).unwrap())
            .unwrap()
            .name
            .is_none());
    }
}

#[test]
fn parse_add_community_deep_link_rejects_invalid_relays() {
    for raw in [
        "buzz://add-community",
        "buzz://add-community?relay=",
        "buzz://add-community?relay=not-a-url",
        "buzz://add-community?relay=https%3A%2F%2Facme.example",
        "buzz://add-community?relay=wss%3A%2F%2F",
    ] {
        assert!(parse_add_community_deep_link(&Url::parse(raw).unwrap()).is_none());
    }
}

#[test]
fn parse_message_deep_link_extracts_required_params() {
    let url = Url::parse("buzz://message?channel=abc&id=xyz").unwrap();
    let payload = parse_message_deep_link(&url).expect("required params present");
    assert_eq!(payload["channelId"], "abc");
    assert_eq!(payload["messageId"], "xyz");
    assert!(payload["threadRootId"].is_null());
}

#[test]
fn parse_message_deep_link_accepts_buzz_scheme() {
    let url = Url::parse("buzz://message?channel=abc&id=xyz").unwrap();
    let payload = parse_message_deep_link(&url).expect("required params present");
    assert_eq!(payload["channelId"], "abc");
    assert_eq!(payload["messageId"], "xyz");
}

#[test]
fn parse_message_deep_link_includes_thread_root() {
    let url = Url::parse("buzz://message?channel=abc&id=xyz&thread=root1").unwrap();
    let payload = parse_message_deep_link(&url).expect("required params present");
    assert_eq!(payload["threadRootId"], "root1");
}

#[test]
fn parse_message_deep_link_rejects_missing_id() {
    let url = Url::parse("buzz://message?channel=abc").unwrap();
    assert!(parse_message_deep_link(&url).is_none());
}

#[test]
fn parse_message_deep_link_rejects_empty_channel() {
    // Regression: `channel=&id=foo` previously produced channelId: "".
    let url = Url::parse("buzz://message?channel=&id=foo").unwrap();
    assert!(parse_message_deep_link(&url).is_none());
}

#[test]
fn parse_message_deep_link_rejects_empty_id() {
    let url = Url::parse("buzz://message?channel=abc&id=").unwrap();
    assert!(parse_message_deep_link(&url).is_none());
}

#[test]
fn parse_message_deep_link_treats_empty_thread_as_absent() {
    let url = Url::parse("buzz://message?channel=abc&id=xyz&thread=").unwrap();
    let payload = parse_message_deep_link(&url).expect("required params present");
    assert!(payload["threadRootId"].is_null());
}

#[test]
fn parse_join_deep_link_extracts_relay_and_code() {
    let url = Url::parse("buzz://join?relay=wss%3A%2F%2Frelay.example&code=abc.def").unwrap();
    let payload = parse_join_deep_link(&url).expect("required params present");
    assert_eq!(payload["relayUrl"], "wss://relay.example");
    assert_eq!(payload["code"], "abc.def");
    assert!(payload["policyReceipt"].is_null());
}

#[test]
fn parse_join_deep_link_extracts_policy_receipt() {
    let url = Url::parse(
        "buzz://join?relay=wss%3A%2F%2Frelay.example&code=abc.def&policy_receipt=receipt.value",
    )
    .unwrap();
    let payload = parse_join_deep_link(&url).expect("required params present");
    assert_eq!(payload["policyReceipt"], "receipt.value");
}

#[test]
fn parse_join_deep_link_rejects_missing_code() {
    let url = Url::parse("buzz://join?relay=wss%3A%2F%2Frelay.example").unwrap();
    assert!(parse_join_deep_link(&url).is_none());
}

#[test]
fn parse_join_deep_link_rejects_empty_code() {
    let url = Url::parse("buzz://join?relay=wss%3A%2F%2Frelay.example&code=").unwrap();
    assert!(parse_join_deep_link(&url).is_none());
}

#[test]
fn parse_join_deep_link_rejects_missing_relay() {
    let url = Url::parse("buzz://join?code=abc.def").unwrap();
    assert!(parse_join_deep_link(&url).is_none());
}

#[test]
fn parse_join_deep_link_rejects_non_websocket_relay() {
    let url = Url::parse("buzz://join?relay=https%3A%2F%2Frelay.example&code=abc.def").unwrap();
    assert!(parse_join_deep_link(&url).is_none());
}

#[test]
fn parse_nostr_bind_deep_link_accepts_valid_url() {
    let payload = parse_nostr_bind_deep_link(&valid_nostr_bind_url()).unwrap();
    assert_eq!(payload.challenge_id, "550e8400-e29b-41d4-a716-446655440000");
    assert_eq!(payload.nonce, "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567");
    assert_eq!(payload.verification_code, "123456");
    assert_eq!(payload.audience, "buzz:nostr-identity");
    assert_eq!(payload.action, "bind_nostr_identity");
    assert_eq!(payload.protocol, "buzz-nostr-identity");
    assert_eq!(payload.version, "1");
    assert_eq!(payload.origin, "https://example.com");
    assert_eq!(payload.expires_at, "2999-01-01T00:00:00Z");
    assert_eq!(payload.return_mode, "clipboard");
    assert_eq!(payload.callback_url, None);
}

#[test]
fn parse_nostr_bind_deep_link_accepts_same_origin_callback_url() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard&callback_url=https%3A%2F%2Fexample.com%2Fbuzz%3FmockSession%3D1").unwrap();
    let payload = parse_nostr_bind_deep_link(&url).unwrap();
    assert_eq!(
        payload.callback_url.as_deref(),
        Some("https://example.com/buzz?mockSession=1")
    );
}

#[test]
fn parse_nostr_bind_deep_link_accepts_browser_fragment_return() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=browser_fragment_v1&callback_url=https%3A%2F%2Fexample.com%2Fbuzz").unwrap();
    let payload = parse_nostr_bind_deep_link(&url).unwrap();

    assert_eq!(payload.return_mode, "browser_fragment_v1");
    assert_eq!(
        payload.callback_url.as_deref(),
        Some("https://example.com/buzz")
    );
}

#[test]
fn parse_nostr_bind_deep_link_requires_callback_for_browser_fragment_return() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=browser_fragment_v1").unwrap();

    assert_eq!(
        parse_nostr_bind_deep_link(&url).unwrap_err(),
        "browser_fragment_v1 requires callback_url"
    );
}

#[test]
fn parse_nostr_bind_deep_link_rejects_cross_origin_callback_url() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard&callback_url=https%3A%2F%2Fevil.example%2Fbuzz").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_http_callback_url() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard&callback_url=http%3A%2F%2Fexample.com%2Fbuzz").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_missing_challenge_id() {
    let url = Url::parse("buzz://nostr-bind?nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_empty_nonce() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_missing_verification_code() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_short_verification_code() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=12345&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_long_verification_code() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=1234567&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_non_digit_verification_code() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=12345a&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_wrong_action() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=wrong&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_wrong_audience() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=other&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_non_https_origin() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=http%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_origin_with_path() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com%2Fbind&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_origin_with_credentials() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fuser%40example.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_rejects_unsupported_return_mode() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=callback").unwrap();
    assert!(parse_nostr_bind_deep_link(&url).is_err());
}

#[test]
fn parse_nostr_bind_deep_link_accepts_expired_link_for_user_facing_error() {
    let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2000-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
    let payload = parse_nostr_bind_deep_link(&url).unwrap();
    assert_eq!(payload.expires_at, "2000-01-01T00:00:00Z");
}
