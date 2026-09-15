use nostr::signer::SignerBackend;
use nostr::{util::BoxedFuture, Event, Keys, NostrSigner, PublicKey, SignerError, UnsignedEvent};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Notify;

#[derive(Debug)]
pub(crate) struct ControlledSigner {
    pub(crate) keys: Keys,
    pub(crate) entered: Notify,
    pub(crate) release: Notify,
    pub(crate) fail: bool,
    pub(crate) public_key_reads: AtomicUsize,
}

impl ControlledSigner {
    pub(crate) fn new(fail: bool) -> Arc<Self> {
        Arc::new(Self {
            keys: Keys::generate(),
            entered: Notify::new(),
            release: Notify::new(),
            fail,
            public_key_reads: AtomicUsize::new(0),
        })
    }
}

macro_rules! delegate_crypto {
    ($method:ident) => {
        fn $method<'a>(
            &'a self,
            public_key: &'a PublicKey,
            content: &'a str,
        ) -> BoxedFuture<'a, Result<String, SignerError>> {
            self.keys.$method(public_key, content)
        }
    };
}

impl NostrSigner for ControlledSigner {
    fn backend(&self) -> SignerBackend<'_> {
        SignerBackend::Custom("controlled".into())
    }
    fn get_public_key(&self) -> BoxedFuture<'_, Result<PublicKey, SignerError>> {
        self.public_key_reads.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(self.keys.public_key()) })
    }
    fn sign_event(&self, unsigned: UnsignedEvent) -> BoxedFuture<'_, Result<Event, SignerError>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            if self.fail {
                return Err(SignerError::from("deliberate signer failure"));
            }
            NostrSigner::sign_event(&self.keys, unsigned).await
        })
    }
    delegate_crypto!(nip04_encrypt);
    delegate_crypto!(nip04_decrypt);
    delegate_crypto!(nip44_encrypt);
    delegate_crypto!(nip44_decrypt);
}

#[tokio::test]
async fn authenticated_connection_awaits_signer_and_preserves_auth_template() {
    use futures_util::{SinkExt, StreamExt};
    use nostr::Tag;
    use tokio_tungstenite::tungstenite::Message;
    for fail in [false, true] {
        let signer = ControlledSigner::new(fail);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let expected_relay = nostr::RelayUrl::parse(&url).unwrap().to_string();
        let expected_pubkey = signer.keys.public_key();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::Text(r#"["AUTH","test-challenge"]"#.into()))
                .await
                .unwrap();
            let frame = ws.next().await;
            if fail {
                assert!(!matches!(frame, Some(Ok(Message::Text(_)))));
                return;
            }
            let frame = frame.unwrap().unwrap().into_text().unwrap();
            let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
            assert_eq!(value[0], "AUTH");
            let event: Event = serde_json::from_value(value[1].clone()).unwrap();
            assert_eq!(event.pubkey, expected_pubkey);
            assert_eq!(event.kind.as_u16(), 22242);
            assert!(event.content.is_empty());
            assert_eq!(
                event.tags.clone().to_vec(),
                vec![
                    Tag::parse(["challenge", "test-challenge"]).unwrap(),
                    Tag::parse(["relay", &expected_relay]).unwrap(),
                    Tag::parse(["auth", "test-token"]).unwrap()
                ]
            );
            event.verify().unwrap();
            ws.send(Message::Text(
                serde_json::json!(["OK", event.id.to_hex(), true, ""])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        });
        let task_signer = signer.clone();
        let client = tokio::spawn(async move {
            crate::NostrWsConnection::connect_authenticated_with_signer(
                &url,
                task_signer.as_ref(),
                Some(&Tag::parse(["auth", "test-token"]).unwrap()),
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), signer.entered.notified())
            .await
            .unwrap();
        assert!(!client.is_finished());
        signer.release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), client)
            .await
            .unwrap()
            .unwrap();
        if fail {
            assert!(
                matches!(result, Err(crate::WsClientError::EventBuilder(message)) if message.contains("deliberate signer failure"))
            );
        } else {
            assert!(result.is_ok());
        }
        assert_eq!(signer.public_key_reads.load(Ordering::SeqCst), 1);
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }
}
