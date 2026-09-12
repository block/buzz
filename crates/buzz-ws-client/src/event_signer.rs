//! Event signing only: construction, credentials, lifecycle and publication belong to callers.

use std::{future::Future, pin::Pin};

use nostr::{Event, Keys, PublicKey, UnsignedEvent};

/// An identity capable of signing an exact, already-constructed Nostr event.
///
/// This deliberately exposes no secret export, encryption, login or relay operations.
/// Callers must capture one signer for the lifetime of an operation.
pub trait EventSigner: Send + Sync {
    /// The public key bound to this signer snapshot.
    fn public_key(&self) -> PublicKey;

    /// Sign without changing the event's public key, timestamp, tags, kind or content.
    fn sign(
        &self,
        event: UnsignedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<Event, String>> + Send + '_>>;
}

/// An in-process signer backed by local keys. Key management remains outside the signer.
#[derive(Clone)]
pub struct LocalEventSigner {
    keys: Keys,
}

impl LocalEventSigner {
    /// Capture the supplied local identity, without generating or looking up keys.
    pub fn new(keys: Keys) -> Self {
        Self { keys }
    }
}

impl EventSigner for LocalEventSigner {
    fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    fn sign(
        &self,
        event: UnsignedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<Event, String>> + Send + '_>> {
        Box::pin(async move {
            if event.pubkey != self.public_key() {
                return Err("unsigned event does not match the signing identity".to_string());
            }
            event.sign_with_keys(&self.keys).map_err(|e| e.to_string())
        })
    }
}

// Compatibility for explicit-key callers. Generic consumers see only EventSigner;
// operations requiring a local secret continue to require Keys explicitly.
impl EventSigner for Keys {
    fn public_key(&self) -> PublicKey {
        Keys::public_key(self)
    }

    fn sign(
        &self,
        event: UnsignedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<Event, String>> + Send + '_>> {
        Box::pin(async move { LocalEventSigner::new(self.clone()).sign(event).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag, Timestamp};

    #[tokio::test]
    async fn signs_exact_event_like_existing_local_path() {
        let keys = Keys::generate();
        let signer = LocalEventSigner::new(keys.clone());
        let builder = EventBuilder::new(Kind::Custom(40002), "exact 🐝\ncontent")
            .tags([
                Tag::parse(["h", "channel"]).unwrap(),
                Tag::parse(["x", "a", "b"]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(1700000000));
        let legacy = builder.clone().sign_with_keys(&keys).unwrap();
        let event = signer
            .sign(builder.build(signer.public_key()))
            .await
            .unwrap();
        assert_eq!(event.id, legacy.id);
        assert_eq!(event.pubkey, legacy.pubkey);
        assert_eq!(event.created_at, legacy.created_at);
        assert_eq!(event.kind, legacy.kind);
        assert_eq!(event.tags, legacy.tags);
        assert_eq!(event.content, legacy.content);
        event.verify().unwrap();
    }

    #[tokio::test]
    async fn rejects_another_author_without_rewriting() {
        let signer = LocalEventSigner::new(Keys::generate());
        let event =
            EventBuilder::new(Kind::TextNote, "unchanged").build(Keys::generate().public_key());
        assert!(signer.sign(event).await.is_err());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::NostrWsConnection;
    use futures_util::{SinkExt, StreamExt};
    use nostr::Tag;
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    struct AsyncSigner(LocalEventSigner);
    impl EventSigner for AsyncSigner {
        fn public_key(&self) -> PublicKey {
            self.0.public_key()
        }
        fn sign(
            &self,
            event: UnsignedEvent,
        ) -> Pin<Box<dyn Future<Output = Result<Event, String>> + Send + '_>> {
            Box::pin(async move {
                tokio::task::yield_now().await;
                self.0.sign(event).await
            })
        }
    }

    #[tokio::test]
    async fn nip42_awaits_signer_and_preserves_auth_wire_shape() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let expected_url = url.clone();
        let signer = AsyncSigner(LocalEventSigner::new(Keys::generate()));
        let public_key = signer.public_key();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    json!(["AUTH", "challenge"]).to_string().into(),
                ))
                .await
                .unwrap();
            let frame = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            assert_eq!(frame[0], "AUTH"); // Not EVENT: the signer never publishes.
            let event: Event = serde_json::from_value(frame[1].clone()).unwrap();
            event.verify().unwrap();
            assert_eq!(event.pubkey, public_key);
            assert_eq!(event.kind.as_u16(), 22242);
            assert_eq!(event.content, "");
            let tags: Vec<_> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
            assert_eq!(
                tags,
                vec![
                    vec!["challenge".to_string(), "challenge".to_string()],
                    vec!["relay".to_string(), expected_url],
                    vec!["auth".to_string(), "test-token".to_string()]
                ]
            );
            socket
                .send(Message::Text(
                    json!(["OK", event.id.to_hex(), true, ""])
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        });
        let auth = Tag::parse(["auth", "test-token"]).unwrap();
        let connection = NostrWsConnection::connect_authenticated(&url, &signer, Some(&auth))
            .await
            .unwrap();
        server.await.unwrap();
        drop(connection);
    }
}
