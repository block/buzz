//! Synthetic tests drive the actual delayed signer -> HTTP submit -> ACK path.
use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Notify;

struct DelayedSigner {
    keys: nostr::Keys,
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
}
impl EventSigner for DelayedSigner {
    fn public_key(&self) -> nostr::PublicKey {
        self.keys.public_key()
    }
    fn sign(
        &self,
        event: nostr::UnsignedEvent,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<nostr::Event, String>> + Send + '_>,
    > {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            event.sign_with_keys(&self.keys).map_err(|e| e.to_string())
        })
    }
}

#[tokio::test]
async fn delayed_signer_preserves_captured_relay_and_identity_and_requires_exact_ack_id() {
    use crate::relay_admission::{reset_rate_limit_gate, TEST_SERIAL};
    use axum::{routing::post, Json, Router};
    use nostr::JsonUtil;
    let _serial = TEST_SERIAL.lock().await;
    reset_rate_limit_gate();
    for wrong_ack in [false, true] {
        let captured = Arc::new(std::sync::Mutex::new(Vec::<nostr::Event>::new()));
        let received = captured.clone();
        let router = Router::new().route(
            "/events",
            post(move |Json(event): Json<nostr::Event>| {
                let received = received.clone();
                async move {
                    event.verify().unwrap();
                    let id = if wrong_ack {
                        "f".repeat(64)
                    } else {
                        event.id.to_hex()
                    };
                    received.lock().unwrap().push(event);
                    Json(serde_json::json!({"event_id":id,"accepted":true,"message":""}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async {
            axum::serve(listener, router).await.unwrap();
        });
        let state = Arc::new(crate::app_state::build_app_state());
        let signer = Arc::new(DelayedSigner {
            keys: nostr::Keys::generate(),
            calls: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let task_state = state.clone();
        let task_signer = signer.clone();
        let task = tokio::spawn(async move {
            submit_event_at_with_keys(
                nostr::EventBuilder::new(nostr::Kind::TextNote, "immutable message"),
                &task_state,
                &base,
                &task_signer,
            )
            .await
        });
        signer.entered.notified().await;
        *state.relay_url_override.lock().unwrap() = Some("ws://127.0.0.1:1".into());
        *state.keys.lock().unwrap() = Some(nostr::Keys::generate());
        signer.release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        let events = captured.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].pubkey, signer.public_key());
        assert_eq!(events[0].content, "immutable message");
        let saved = events[0].as_json();
        assert_eq!(nostr::Event::from_json(saved).unwrap().id, events[0].id);
        if wrong_ack {
            assert!(result.unwrap_err().contains("ID mismatch"));
        } else {
            assert_eq!(result.unwrap().event_id, events[0].id.to_hex());
        }
        server.abort();
    }
    reset_rate_limit_gate();
}
