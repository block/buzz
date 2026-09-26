//! Direct moderation action tests (a child of `tests`, split for the size ratchet).

use super::*;

const SIGNER: &str = "11111111111111111111111111111111111111111111111111111111111111aa";

fn intent(action: DirectAction, expiration_secs: Option<u64>) -> AdminDirectIntent {
    AdminDirectIntent {
        origin: "https://admin.example.com".to_string(),
        expected_relay: "wss://relay.example.com".to_string(),
        expected_pubkey: SIGNER.to_string(),
        community_host: "Team.Example.com".to_string(),
        action,
        target: "ab".repeat(32),
        request_id: uuid::Uuid::nil(),
        reason: Some("spam".to_string()),
        expiration_secs,
    }
}

const BASE: &str = "https://relay.example.com";

#[test]
fn direct_action_sends_the_typed_host_only_as_the_query() {
    let (url, body) =
        direct_action_request(&intent(DirectAction::Timeout, Some(60)), BASE, SIGNER).unwrap();
    let t = "ab".repeat(32);
    assert!(
        url.ends_with(&format!(
            "/api/admin/v1/members/{t}/timeout?communityHost=team.example.com"
        )),
        "{url}"
    );
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "requestId": uuid::Uuid::nil(),
            "reason": "spam",
            "expirationSecs": 60,
        })
    );
    let (url, body) =
        direct_action_request(&intent(DirectAction::Delete, None), BASE, SIGNER).unwrap();
    assert!(url.contains(&format!("/events/{t}/delete?")), "{url}");
    assert!(!String::from_utf8(body).unwrap().contains("expirationSecs"));
}

#[test]
fn direct_action_refuses_a_changed_signer_or_relay() {
    // Confirmed under SIGNER on relay.example.com; each mismatch fails pre-send.
    let other = "22".repeat(32);
    for (signer, base) in [
        (other.as_str(), BASE),
        (SIGNER, "https://relay-b.example.com"),
    ] {
        assert!(direct_action_request(&intent(DirectAction::Ban, None), base, signer).is_err());
    }
    let mut blank = intent(DirectAction::Ban, None);
    blank.expected_pubkey = " ".to_string();
    assert!(direct_action_request(&blank, BASE, SIGNER).is_err());
}

#[test]
fn direct_action_rejects_bad_host_target_and_duration() {
    let mut bad_host = intent(DirectAction::Ban, None);
    bad_host.community_host = "https://team.example.com/x".to_string();
    let mut bad_target = intent(DirectAction::Ban, None);
    bad_target.target = "AB".repeat(32);
    for bad in [
        bad_host,
        bad_target,
        intent(DirectAction::Timeout, None),
        intent(DirectAction::Timeout, Some(0)),
        intent(DirectAction::Ban, Some(60)),
    ] {
        assert!(
            direct_action_request(&bad, BASE, SIGNER).is_err(),
            "{bad:?}"
        );
    }
}

/// AppState signing as `keys` on relay.example.com, plus a direct intent
/// confirmed under `keys` that targets the loopback admin origin at `addr`.
fn direct_state_and_intent(
    keys: &nostr::Keys,
    addr: std::net::SocketAddr,
    reason: &str,
) -> (crate::app_state::AppState, AdminDirectIntent) {
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some("wss://relay.example.com".to_string());
    let mut intent = intent(DirectAction::Ban, None);
    intent.origin = format!("http://127.0.0.1:{}", addr.port());
    intent.expected_pubkey = keys.public_key().to_hex();
    intent.reason = Some(reason.to_string());
    (state, intent)
}

/// Mutation evidence: removing the guard call in `send_admin_mutation` lets
/// both key-backup reasons reach the listener and flips the count RED.
#[tokio::test]
async fn direct_action_refuses_key_backup_reason_before_any_request() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    let addr = serve_sequence_inspect(
        vec![(
            "200 OK",
            "Content-Type: application/json\r\n",
            r#"{"state":"applied"}"#,
        )],
        Some(Arc::new(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        })),
    )
    .await;
    let keys = nostr::Keys::generate();
    for reason in ["see ncryptsec1qgg9947", "see NCRYPTSEC1QGG9947"] {
        let (state, intent) = direct_state_and_intent(&keys, addr, reason);
        let err = send_direct_action(&intent, keys.clone(), &state)
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("key-backup"), "{err:?}");
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "a key backup reached the relay"
    );
    let (state, intent) = direct_state_and_intent(&keys, addr, "spam");
    send_direct_action(&intent, keys.clone(), &state)
        .await
        .expect("an ordinary reason still sends");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

/// The identity is swapped to B after the snapshot is taken; the first send
/// and the 401 retry must both still be signed by the confirmed key A.
///
/// Mutation evidence: signing with a re-read `state.signing_keys()` inside
/// `send_direct_action` (the pre-fix shape) signs as B and flips this RED.
#[tokio::test]
async fn direct_action_signs_only_with_the_validated_snapshot() {
    use std::sync::Mutex;
    let auths: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&auths);
    let addr = serve_sequence_inspect(
        vec![
            ("401 Unauthorized", "WWW-Authenticate: Nostr\r\n", ""),
            (
                "200 OK",
                "Content-Type: application/json\r\n",
                r#"{"state":"applied"}"#,
            ),
        ],
        Some(Arc::new(move |_, bytes: &[u8]| {
            let text = String::from_utf8_lossy(bytes);
            let auth = text
                .lines()
                .find_map(|l| {
                    l.strip_prefix("authorization: ")
                        .or(l.strip_prefix("Authorization: "))
                })
                .expect("NIP-98 header present");
            let pubkey = decode_nip98_event(auth.trim())["pubkey"]
                .as_str()
                .unwrap()
                .to_string();
            rec.lock().unwrap().push(pubkey);
        })),
    )
    .await;
    let a = nostr::Keys::generate();
    let (state, intent) = direct_state_and_intent(&a, addr, "spam");
    let snapshot = state.signing_keys().unwrap();
    *state.keys.lock().unwrap() = nostr::Keys::generate(); // identity import of B
    send_direct_action(&intent, snapshot, &state)
        .await
        .expect("A-confirmed action sends as A");
    assert_eq!(*auths.lock().unwrap(), vec![a.public_key().to_hex(); 2]);
}
