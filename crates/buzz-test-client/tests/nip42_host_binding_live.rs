//! Live two-host proof for the NIP-42 host-binding fix.
//!
//! Sibling of the NIP-98 row 44 obligation: AUTH event's `relay` tag must match
//! the per-tenant host of the connection it arrives on, not the deployment-wide
//! `config.relay_url`.
//!
//! Requires a running multi-tenant relay with TWO seeded communities. Bring-up:
//!
//! ```sh
//! # Compose up infra, schema, then seed:
//! INSERT INTO communities (id, host) VALUES
//!   ('11111111-1111-4111-8111-111111111111', 'a.localhost:3100'),
//!   ('22222222-2222-4222-8222-222222222222', 'b.localhost:3100');
//! # Run one binary, BUZZ_HEALTH_PORT=8180 BUZZ_METRICS_PORT=9202,
//! # BUZZ_RECONCILE_CHANNELS=false, BUZZ_GIT_CONFORMANCE_PROBE=false
//! ```
//!
//! Each test is `#[ignore]` so it only runs explicitly:
//! `cargo test -p buzz-test-client --test nip42_host_binding_live -- --ignored --test-threads=1`

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, RelayUrl};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const HOST_A: &str = "ws://a.localhost:3100";
const HOST_B: &str = "ws://b.localhost:3100";

/// Connect, wait for AUTH challenge, send a kind:22242 with `relay_tag_url`,
/// return the OK response's accepted flag and message.
async fn do_auth_with_relay_tag(
    connect_url: &str,
    relay_tag_url: &str,
) -> Result<(bool, String), String> {
    let (mut ws, _) = connect_async(connect_url)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    // Wait for AUTH challenge.
    let challenge = loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .map_err(|_| "timeout waiting for AUTH challenge".to_string())?
            .ok_or_else(|| "ws closed before challenge".to_string())?
            .map_err(|e| format!("ws read: {e}"))?;
        let text = match msg {
            Message::Text(t) => t,
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => continue,
            other => return Err(format!("unexpected ws frame: {other:?}")),
        };
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("json: {e}"))?;
        if v.get(0).and_then(|s| s.as_str()) == Some("AUTH") {
            break v
                .get(1)
                .and_then(|s| s.as_str())
                .ok_or_else(|| "AUTH msg missing challenge".to_string())?
                .to_string();
        }
        // ignore NOTICE etc.
    };

    // Sign a NIP-42 AUTH event with chosen relay tag.
    let keys = Keys::generate();
    let parsed: RelayUrl = relay_tag_url
        .parse()
        .map_err(|e| format!("parse relay tag url {relay_tag_url}: {e}"))?;
    let event = EventBuilder::auth(&challenge, parsed)
        .sign_with_keys(&keys)
        .map_err(|e| format!("sign: {e}"))?;
    let event_id_hex = event.id.to_hex();

    let send = json!(["AUTH", event]);
    ws.send(Message::Text(send.to_string().into()))
        .await
        .map_err(|e| format!("ws send: {e}"))?;

    // Wait for OK with matching event id.
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .map_err(|_| "timeout waiting for OK".to_string())?
            .ok_or_else(|| "ws closed before OK".to_string())?
            .map_err(|e| format!("ws read OK: {e}"))?;
        let text = match msg {
            Message::Text(t) => t,
            _ => continue,
        };
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("json OK: {e}"))?;
        if v.get(0).and_then(|s| s.as_str()) == Some("OK")
            && v.get(1).and_then(|s| s.as_str()) == Some(&event_id_hex)
        {
            let accepted = v.get(2).and_then(|s| s.as_bool()).unwrap_or(false);
            let message = v.get(3).and_then(|s| s.as_str()).unwrap_or("").to_string();
            return Ok((accepted, message));
        }
    }
}

/// AUTH event signed for host A's URL on a connection to host A → ACCEPT.
#[tokio::test]
#[ignore = "requires two-host multi-tenant relay"]
async fn nip42_matching_host_accepted_a() {
    let (accepted, msg) = do_auth_with_relay_tag(HOST_A, HOST_A)
        .await
        .expect("auth flow on host A");
    assert!(
        accepted,
        "matching-host AUTH on host A must be ACCEPTED; relay said: {msg}"
    );
}

/// AUTH event signed for host B's URL on a connection to host B → ACCEPT.
#[tokio::test]
#[ignore = "requires two-host multi-tenant relay"]
async fn nip42_matching_host_accepted_b() {
    let (accepted, msg) = do_auth_with_relay_tag(HOST_B, HOST_B)
        .await
        .expect("auth flow on host B");
    assert!(
        accepted,
        "matching-host AUTH on host B must be ACCEPTED; relay said: {msg}"
    );
}

/// Cross-host attack on the B-bound connection: forge AUTH with `relay` tag
/// pointing at host A. Pre-fix this passed (verified against
/// `state.config.relay_url`). Post-fix the per-tenant host check rejects it.
#[tokio::test]
#[ignore = "requires two-host multi-tenant relay"]
async fn nip42_cross_host_rejected_a_relay_tag_on_b_connection() {
    let (accepted, msg) = do_auth_with_relay_tag(HOST_B, HOST_A)
        .await
        .expect("auth flow with cross-host relay tag");
    assert!(
        !accepted,
        "cross-host AUTH (relay-tag=A on connection=B) must be REJECTED; relay said: {msg}"
    );
    // Must be the host-binding rejection, not some other error.
    assert!(
        msg.contains("auth-required") || msg.contains("verification"),
        "rejection must be the NIP-42 verification-failure signal; relay said: {msg}"
    );
}

/// Mirror of the cross-host test in the opposite direction: B-relay-tag on
/// A-connection → REJECT.
#[tokio::test]
#[ignore = "requires two-host multi-tenant relay"]
async fn nip42_cross_host_rejected_b_relay_tag_on_a_connection() {
    let (accepted, msg) = do_auth_with_relay_tag(HOST_A, HOST_B)
        .await
        .expect("auth flow with mirror cross-host relay tag");
    assert!(
        !accepted,
        "cross-host AUTH (relay-tag=B on connection=A) must be REJECTED; relay said: {msg}"
    );
}
