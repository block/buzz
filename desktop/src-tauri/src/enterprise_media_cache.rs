//! A single bounded read proof per corporate login. A new session gets a new cache.
use buzz_ws_client_pkg::event_signer::EventSigner;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(crate) struct MediaReadProofCache {
    entry: Mutex<Option<Entry>>,
}
struct Entry {
    pubkey: nostr::PublicKey,
    origin: String,
    header: String,
    expires_at: u64,
}
impl MediaReadProofCache {
    pub(crate) async fn clear(&self) {
        *self.entry.lock().await = None;
    }

    pub(crate) async fn get(
        &self,
        signer: &dyn EventSigner,
        origin: &str,
        cancelled: &CancellationToken,
    ) -> Result<String, String> {
        // Serializes avatar-grid bursts without unbounded in-flight work/cache entries.
        tokio::select! {
            biased;
            _ = cancelled.cancelled() => Err("Corporate login changed; media proof cancelled".into()),
            result = async {
                let mut entry = self.entry.lock().await;
                let now = nostr::Timestamp::now().as_secs();
                if let Some(cached) = entry.as_ref() {
                    if cached.pubkey == signer.public_key() && cached.origin == origin
                        && cached.expires_at > now + 10 {
                        return Ok(cached.header.clone());
                    }
                }
                // Capture expiry before signing: signer latency must not extend cache validity.
                *entry = None;
                let header = crate::commands::media::sign_blossom_get_auth_header(
                    signer, origin, 120,
                ).await?;
                if cancelled.is_cancelled() {
                    return Err("Corporate login changed; media proof cancelled".into());
                }
                if now + 120 <= nostr::Timestamp::now().as_secs() + 10 {
                    return Err("Corporate media proof expired while signing".into());
                }
                *entry = Some(Entry {
                    pubkey: signer.public_key(), origin: origin.into(),
                    header: header.clone(), expires_at: now + 120,
                });
                Ok(header)
            } => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Signer {
        keys: nostr::Keys,
        calls: AtomicUsize,
        cancel: Option<CancellationToken>,
    }
    impl EventSigner for Signer {
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
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                if let Some(cancel) = &self.cancel {
                    cancel.cancel();
                }
                event.sign_with_keys(&self.keys).map_err(|e| e.to_string())
            })
        }
    }
    fn signer() -> Signer {
        Signer {
            keys: nostr::Keys::generate(),
            calls: AtomicUsize::new(0),
            cancel: None,
        }
    }
    #[tokio::test]
    async fn production_cache_singleflights_and_fences_host_identity_expiry_and_new_session() {
        let cache = MediaReadProofCache::default();
        let signer = signer();
        let cancel = CancellationToken::new();
        let (a, b, c) = tokio::join!(
            cache.get(&signer, "https://a.example", &cancel),
            cache.get(&signer, "https://a.example", &cancel),
            cache.get(&signer, "https://a.example", &cancel)
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert!(c.is_ok());
        assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
        cache
            .get(&signer, "https://a.example:8443", &cancel)
            .await
            .unwrap();
        assert_eq!(signer.calls.load(Ordering::SeqCst), 2);
        cache.entry.lock().await.as_mut().unwrap().expires_at = 0;
        cache
            .get(&signer, "https://a.example:8443", &cancel)
            .await
            .unwrap();
        assert_eq!(signer.calls.load(Ordering::SeqCst), 3);
        let other = super::tests::signer();
        cache
            .get(&other, "https://a.example:8443", &cancel)
            .await
            .unwrap();
        assert_eq!(other.calls.load(Ordering::SeqCst), 1);
        cancel.cancel();
        assert!(cache
            .get(&other, "https://a.example:8443", &cancel)
            .await
            .is_err());
        cache.clear().await;
        assert!(cache.entry.lock().await.is_none());
        MediaReadProofCache::default()
            .get(&other, "https://a.example:8443", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(other.calls.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn cancellation_during_sign_never_populates_cache() {
        let cache = MediaReadProofCache::default();
        let cancel = CancellationToken::new();
        let mut signer = signer();
        signer.cancel = Some(cancel.clone());
        assert!(cache
            .get(&signer, "https://a.example", &cancel)
            .await
            .is_err());
        assert!(cache.entry.lock().await.is_none());
    }
}
