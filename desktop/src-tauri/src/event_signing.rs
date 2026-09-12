//! Event construction adapter. Signers receive only the completed unsigned event.
use buzz_ws_client_pkg::event_signer::EventSigner;
use nostr::{Event, EventBuilder};

pub(crate) trait EventBuilderSigning {
    async fn sign_with_event_signer(
        self,
        signer: &(impl EventSigner + ?Sized),
    ) -> Result<Event, String>;
}
impl EventBuilderSigning for EventBuilder {
    async fn sign_with_event_signer(
        self,
        signer: &(impl EventSigner + ?Sized),
    ) -> Result<Event, String> {
        let unsigned = self.build(signer.public_key());
        signer.sign(unsigned).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, PublicKey, UnsignedEvent};
    use std::{future::Future, pin::Pin};

    struct UnavailableSigner(PublicKey);
    impl EventSigner for UnavailableSigner {
        fn public_key(&self) -> PublicKey {
            self.0
        }
        fn sign(
            &self,
            event: UnsignedEvent,
        ) -> Pin<Box<dyn Future<Output = Result<Event, String>> + Send + '_>> {
            Box::pin(async move {
                tokio::task::yield_now().await;
                assert_eq!(event.pubkey, self.0);
                assert_eq!(event.created_at.as_secs(), 1700000000);
                assert_eq!(event.content, "exact");
                Err("signer unavailable".to_string())
            })
        }
    }

    #[tokio::test]
    async fn construction_adapter_awaits_and_propagates_signer_failure() {
        let signer = UnavailableSigner(Keys::generate().public_key());
        let result = EventBuilder::new(nostr::Kind::TextNote, "exact")
            .custom_created_at(nostr::Timestamp::from(1700000000))
            .sign_with_event_signer(&signer)
            .await;
        assert_eq!(result.unwrap_err(), "signer unavailable");
    }
}
