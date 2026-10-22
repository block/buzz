use super::*;
use nostr::{JsonUtil, Kind};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
use tokio_tungstenite::accept_hdr_async;

const TOKEN: &str = "eyJhbGciOiJFUzI1NiJ9.e30.AAECAw";
const BUDGET: Duration = Duration::from_secs(2);

#[derive(Clone, Default)]
struct CrateTrace(Arc<Mutex<String>>);

fn crate_trace() -> &'static CrateTrace {
    static TRACE: OnceLock<CrateTrace> = OnceLock::new();
    TRACE.get_or_init(|| {
        let trace = CrateTrace::default();
        tracing::subscriber::set_global_default(trace.clone()).unwrap();
        trace
    })
}

impl tracing::Subscriber for CrateTrace {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        // Other tests can reach the same callsites before this subscriber.
        // Evaluate the active dispatch instead of caching a global decision.
        tracing::subscriber::Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }

    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        // This asserts this crate's logging contract, not raw wire tracing in
        // dependencies. Hosts must not enable credential-bearing wire traces.
        metadata.target().starts_with("buzz_ws_client")
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields<'a>(&'a mut String);
        impl tracing::field::Visit for Fields<'_> {
            fn record_debug(&mut self, _: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{value:?}"));
            }
        }
        event.record(&mut Fields(&mut self.0.lock().unwrap()));
    }
}

fn keys() -> Keys {
    // Fixed public test identity, never a generated or real employee key.
    Keys::parse(&format!("{:064x}", 1)).unwrap()
}

fn assertion() -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap()
}

async fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("wss://{}/tenant", listener.local_addr().unwrap());
    (listener, relay)
}

async fn connect(relay: &str, budget: Duration) -> Result<NostrWsConnection, NipFiConnectError> {
    let assertion = assertion();
    let mut request = nip_fi_request(relay, &assertion).unwrap();
    // Private test seam: identical production request/authentication path,
    // with plaintext transport only for this loopback fixture.
    *request.uri_mut() = relay.replacen("wss:", "ws:", 1).parse().unwrap();
    NostrWsConnection::connect_nip_fi_request(request, relay, &keys(), &assertion, budget).await
}

#[tokio::test]
#[allow(clippy::result_large_err)] // tungstenite fixes the handshake callback's error type.
async fn upgrade_has_one_sensitive_header_and_same_key_nip42_proof() {
    let trace = crate_trace();
    let (listener, relay) = listener().await;
    let expected_relay = relay.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_hdr_async(socket, |request: &Request, response| {
            assert_eq!(
                request
                    .headers()
                    .get_all("nostr-federated-identity")
                    .iter()
                    .count(),
                1
            );
            assert_eq!(
                request.headers()["nostr-federated-identity"],
                format!("Bearer {TOKEN}")
            );
            assert!(request.headers().get("authorization").is_none());
            assert!(request.headers().get("x-bb-session-credential").is_none());
            assert_eq!(request.uri().path(), "/tenant");
            assert!(request.uri().query().is_none());
            Ok(response)
        })
        .await
        .unwrap();
        ws.send(Message::text(r#"["AUTH","fixture-challenge"]"#))
            .await
            .unwrap();
        let text = ws.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(!text.contains(TOKEN));
        let frame: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(frame[0], "AUTH");
        let proof = Event::from_json(frame[1].to_string()).unwrap();
        proof.verify().unwrap();
        assert_eq!(proof.pubkey, keys().public_key());
        assert_eq!(proof.kind, Kind::Authentication);
        let tag = |name: &str| {
            proof
                .tags
                .iter()
                .find(|t| t.as_slice()[0] == name)
                .unwrap()
                .as_slice()[1]
                .clone()
        };
        assert_eq!(tag("relay"), expected_relay);
        assert_eq!(tag("challenge"), "fixture-challenge");
        assert_eq!(proof.tags.len(), 2);
        ws.send(Message::text(json!(["OK", proof.id, true, ""]).to_string()))
            .await
            .unwrap();
    });
    let request = nip_fi_request(&relay, &assertion()).unwrap();
    assert!(request.headers()["nostr-federated-identity"].is_sensitive());
    assert!(!format!("{request:?}").contains(TOKEN));
    let connection = connect(&relay, BUDGET).await.unwrap();
    let logged = trace.0.lock().unwrap().clone();
    assert!(logged.contains("NIP-42 authentication successful"));
    assert!(!logged.contains(TOKEN));
    assert!(!logged.contains("fixture-challenge"));
    assert!(connection.auth_buffer_remaining.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn invalid_public_inputs_and_zero_budget_never_open_a_socket() {
    let (listener, relay) = listener().await;
    for url in [
        relay.replacen("wss:", "ws:", 1),
        format!("{relay}?secret=credential"),
        format!("{relay}#fragment"),
        relay.replacen("wss://", "wss://user:password@", 1),
    ] {
        let result = NostrWsConnection::connect_authenticated_with_nip_fi(
            &url,
            &keys(),
            &assertion(),
            BUDGET,
        )
        .await;
        assert!(matches!(
            result,
            Err(NipFiConnectError::InvalidConfiguration)
        ));
    }
    for value in [
        "",
        "Bearer ",
        "Basic abc",
        "Bearer a.b",
        "Bearer a.b.c.d",
        "Bearer a..c",
        "Bearer a.b.c,other",
        "Bearer a.b.c ",
        "Bearer a.b.c=",
    ] {
        let result = NostrWsConnection::connect_authenticated_with_nip_fi(
            &relay,
            &keys(),
            &HeaderValue::from_str(value).unwrap(),
            BUDGET,
        )
        .await;
        assert!(matches!(
            result,
            Err(NipFiConnectError::InvalidConfiguration)
        ));
    }
    let oversized =
        HeaderValue::from_str(&format!("Bearer {}.b.c", "a".repeat(16 * 1024))).unwrap();
    assert!(matches!(
        NostrWsConnection::connect_authenticated_with_nip_fi(&relay, &keys(), &oversized, BUDGET)
            .await,
        Err(NipFiConnectError::InvalidConfiguration)
    ));
    assert!(matches!(
        NostrWsConnection::connect_authenticated_with_nip_fi(
            &relay,
            &keys(),
            &assertion(),
            Duration::ZERO
        )
        .await,
        Err(NipFiConnectError::InvalidConfiguration)
    ));
    assert!(timeout(Duration::from_millis(30), listener.accept())
        .await
        .is_err());
}

#[tokio::test]
async fn upgrade_rejections_are_redacted_and_never_redirect_or_fall_back() {
    for (status, expected) in [
        (401, NipFiConnectError::AuthenticationRejected),
        (403, NipFiConnectError::PermissionDenied),
        (302, NipFiConnectError::Unavailable),
        (503, NipFiConnectError::Unavailable),
    ] {
        let (listener, relay) = listener().await;
        let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let location = format!("ws://{}/forbidden", destination.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(socket.read(&mut bytes).await.unwrap() > 0);
            socket.write_all(format!("HTTP/1.1 {status} Refused\r\nLocation: {location}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{TOKEN}", TOKEN.len()).as_bytes()).await.unwrap();
            // A fallback/retry would create a second socket on this listener.
            assert!(timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err());
        });
        let error = connect(&relay, BUDGET).await.err().unwrap();
        assert_eq!(error, expected);
        assert!(!format!("{error:?} {error}").contains(TOKEN));
        assert!(timeout(Duration::from_millis(30), destination.accept())
            .await
            .is_err());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn reflected_challenge_is_rejected_without_signing_an_auth_event() {
    let (listener, relay) = listener().await;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        ws.send(Message::text(
            json!(["AUTH", format!("echo-{TOKEN}")]).to_string(),
        ))
        .await
        .unwrap();
        let received = timeout(BUDGET, ws.next()).await.unwrap();
        assert!(!matches!(received, Some(Ok(Message::Text(_)))));
    });
    let error = connect(&relay, BUDGET).await.err().unwrap();
    assert_eq!(error, NipFiConnectError::AuthenticationRejected);
    assert!(!format!("{error:?} {error}").contains(TOKEN));
    server.await.unwrap();
}

#[tokio::test]
async fn reflected_auth_refusal_is_redacted() {
    let (listener, relay) = listener().await;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        ws.send(Message::text(r#"["AUTH","fixture"]"#))
            .await
            .unwrap();
        let frame: Value =
            serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        ws.send(Message::text(
            json!(["OK", frame[1]["id"], false, TOKEN]).to_string(),
        ))
        .await
        .unwrap();
    });
    let error = connect(&relay, BUDGET).await.err().unwrap();
    assert_eq!(error, NipFiConnectError::AuthenticationRejected);
    assert!(!format!("{error:?} {error}").contains(TOKEN));
    server.await.unwrap();
}

#[tokio::test]
async fn complete_budget_bounds_stalled_upgrade_and_closes_socket() {
    let (listener, relay) = listener().await;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        timeout(BUDGET, socket.read_to_end(&mut request))
            .await
            .unwrap()
            .unwrap();
        assert!(!request.is_empty());
    });
    assert!(matches!(
        connect(&relay, Duration::from_millis(50)).await,
        Err(NipFiConnectError::Timeout)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn complete_budget_bounds_stalled_authentication_and_closes_socket() {
    let (listener, relay) = listener().await;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        // Never issue an AUTH challenge. The caller's budget, rather than the
        // longer inner challenge deadline, must stop this connection.
        assert!(!matches!(
            timeout(BUDGET, ws.next()).await.unwrap(),
            Some(Ok(Message::Text(_)))
        ));
    });
    assert!(matches!(
        connect(&relay, Duration::from_millis(50)).await,
        Err(NipFiConnectError::Timeout)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn host_cancellation_drops_socket_during_upgrade_and_authentication() {
    for upgrade in [false, true] {
        let (listener, relay) = listener().await;
        let (arrived, ready) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            if upgrade {
                let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                arrived.send(()).unwrap();
                assert!(!matches!(
                    timeout(BUDGET, ws.next()).await.unwrap(),
                    Some(Ok(Message::Text(_)))
                ));
            } else {
                let mut buffer = [0; 4096];
                assert!(socket.read(&mut buffer).await.unwrap() > 0);
                arrived.send(()).unwrap();
                let mut remaining = Vec::new();
                timeout(BUDGET, socket.read_to_end(&mut remaining))
                    .await
                    .unwrap()
                    .unwrap();
            }
        });
        let client = tokio::spawn(async move { connect(&relay, BUDGET).await });
        ready.await.unwrap();
        client.abort();
        assert!(matches!(client.await, Err(error) if error.is_cancelled()));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn pre_auth_message_flood_is_bounded_by_count_and_bytes() {
    for oversized in [false, true] {
        let (listener, relay) = listener().await;
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            let notice = if oversized {
                "x".repeat(64 * 1024)
            } else {
                "x".into()
            };
            for _ in 0..65 {
                if ws
                    .send(Message::text(json!(["NOTICE", notice]).to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        assert!(matches!(
            connect(&relay, BUDGET).await,
            Err(NipFiConnectError::Unavailable)
        ));
        server.await.unwrap();
    }
}

#[tokio::test]
#[allow(clippy::result_large_err)] // tungstenite fixes the handshake callback's error type.
async fn ordinary_connector_keeps_its_header_free_nip42_path() {
    let (listener, relay) = listener().await;
    let url = relay.replacen("wss:", "ws:", 1);
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_hdr_async(socket, |request: &Request, response| {
            assert!(request.headers().get("nostr-federated-identity").is_none());
            Ok(response)
        })
        .await
        .unwrap();
        ws.send(Message::text(r#"["AUTH","legacy-fixture"]"#))
            .await
            .unwrap();
        let frame: Value =
            serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        let proof = Event::from_json(frame[1].to_string()).unwrap();
        proof.verify().unwrap();
        ws.send(Message::text(json!(["OK", proof.id, true, ""]).to_string()))
            .await
            .unwrap();
    });
    NostrWsConnection::connect_authenticated(&url, &keys(), None)
        .await
        .unwrap();
    server.await.unwrap();
}
