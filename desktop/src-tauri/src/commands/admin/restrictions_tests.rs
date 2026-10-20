//! Explicit-host restriction and lift tests.

use super::super::read_stub::{response, serve_raw, signer_of};
use super::*;

const RELAY: &str = "https://relay.example.com";

fn intent(origin: String, signer: &str) -> AdminLiftIntent {
    AdminLiftIntent {
        origin,
        community_host: "Team.Example.com".to_string(),
        expected_relay: "wss://relay.example.com".to_string(),
        expected_signer: signer.to_string(),
        kind: LiftKind::Timeout,
        pubkey: "ab".repeat(32),
    }
}

#[test]
fn lift_url_sends_the_page_host_not_the_active_relay() {
    let signer = "11".repeat(32);
    let url = lift_url(
        &intent("https://admin.example.com".into(), &signer),
        RELAY,
        &signer,
    )
    .unwrap();
    assert!(
        url.ends_with(&format!(
            "/members/{}/timeout?communityHost=team.example.com",
            "ab".repeat(32)
        )),
        "{url}"
    );
}

#[test]
fn list_url_carries_host_and_cursor_and_refuses_a_changed_relay() {
    let url = restrictions_url(
        "https://admin.example.com",
        &routes::AdminRoute::MemberRestrictionsList,
        "other.example.com",
        Some("tok".to_string()),
        "wss://relay.example.com",
        RELAY,
    )
    .unwrap();
    assert!(url.contains("communityHost=other.example.com"), "{url}");
    assert!(url.contains("cursor=tok"), "{url}");
    for expected in ["wss://relay-b.example.com", "", "  "] {
        let err = restrictions_url(
            "https://admin.example.com",
            &routes::AdminRoute::MemberRestrictionsList,
            "other.example.com",
            None,
            expected,
            RELAY,
        )
        .unwrap_err();
        assert_eq!(err, RELAY_SCOPE_CHANGED);
    }
}

#[tokio::test]
async fn lift_refuses_before_sending_on_signer_change_or_bad_host() {
    // Nothing listens on this origin: any request would be a transport error,
    // not the `notSent` refusal asserted here.
    let keys = nostr::Keys::generate();
    let other = "22".repeat(32);
    let mut bad_host = intent("http://127.0.0.1:9".into(), &keys.public_key().to_hex());
    bad_host.community_host = "https://team.example.com/x".to_string();
    for bad in [
        intent("http://127.0.0.1:9".into(), &other),
        intent("http://127.0.0.1:9".into(), " "),
        bad_host,
    ] {
        let err = send_lift(&bad, &keys, RELAY).await.unwrap_err();
        assert!(err.not_sent, "{err:?}");
    }
}

#[tokio::test]
async fn lift_signs_the_send_and_401_retry_with_the_checked_keys() {
    let keys = nostr::Keys::generate();
    let (port, seen) = serve_raw(vec![
        response("401 Unauthorized", ""),
        response("204 No Content", ""),
    ]);
    let lift = intent(
        format!("http://127.0.0.1:{port}"),
        &keys.public_key().to_hex(),
    );
    send_lift(&lift, &keys, RELAY).await.unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for request in seen.iter() {
        assert!(request.starts_with("DELETE "), "{request}");
        assert!(
            request.contains("communityHost=team.example.com"),
            "{request}"
        );
        assert_eq!(signer_of(request), keys.public_key().to_hex());
    }
}
