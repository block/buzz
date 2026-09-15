use super::*;
use crate::active_user_signer::{tests::ControlledSigner, ActiveUserSigner};
use base64::Engine;
use nostr::{Event, JsonUtil, Keys};
use std::sync::Arc;

#[tokio::test]
async fn mesh_status_submit_preserves_author_auth_and_failure() {
    use axum::{body::Bytes, http::HeaderMap, routing::post, Router};
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    for fail in [false, true] {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let router = Router::new().route("/events", post(move |headers: HeaderMap, body: Bytes| {
            let tx = tx.clone();
            async move {
                let event = Event::from_json(&body).unwrap();
                tx.send((headers, event.clone())).await.unwrap();
                axum::Json(serde_json::json!({"event_id":event.id.to_hex(),"accepted":true,"message":""}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let controlled = ControlledSigner::new(fail);
        let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
        let state = Arc::new(crate::app_state::build_app_state());
        let task_state = state.clone();
        let task_base = base.clone();
        let task = tokio::spawn(async move {
            publish_status_report_at(
                &task_state,
                &task_base,
                serde_json::json!({"ownerId":"machine", "serveTargets":[], "models":[]}),
                &signer,
            )
            .await
        });
        controlled.wait_entered().await;
        state.replace_local_identity_keys(Keys::generate()).unwrap();
        *state.relay_url_override.lock().unwrap() = Some("http://127.0.0.1:1".into());
        controlled.release.notify_one();
        if fail {
            assert!(task
                .await
                .unwrap()
                .unwrap_err()
                .contains("deliberate signer failure"));
            assert!(rx.try_recv().is_err());
        } else {
            controlled.wait_entered().await;
            controlled.release.notify_one();
            task.await.unwrap().unwrap();
            let (headers, event) = rx.recv().await.unwrap();
            event.verify().unwrap();
            assert_eq!(event.pubkey, controlled.keys.public_key());
            assert_eq!(event.kind.as_u16(), KIND_BUZZ_MESH_MEMBER_STATUS);
            let auth = Event::from_json(
                base64::engine::general_purpose::STANDARD
                    .decode(
                        headers["authorization"]
                            .to_str()
                            .unwrap()
                            .strip_prefix("Nostr ")
                            .unwrap(),
                    )
                    .unwrap(),
            )
            .unwrap();
            auth.verify().unwrap();
            assert_eq!(auth.pubkey, event.pubkey);
            assert!(auth
                .tags
                .iter()
                .any(|t| t.as_slice() == ["u", &format!("{base}/events")]));
        }
        server.abort();
    }
}
