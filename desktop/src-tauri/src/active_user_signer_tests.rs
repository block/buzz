use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use nostr::signer::SignerBackend;
use nostr::{
    util::BoxedFuture, Event, EventBuilder, Keys, Kind, NostrSigner, PublicKey, SignerError,
    Timestamp, UnsignedEvent,
};
use tokio::sync::Notify;

use super::ActiveUserSigner;

#[derive(Debug)]
pub(crate) struct ControlledSigner {
    pub(crate) keys: Keys,
    pub(crate) entered: Notify,
    pub(crate) release: Notify,
    pub(crate) fail: bool,
    pub(crate) fail_kind: AtomicUsize,
    pub(crate) public_key_reads: AtomicUsize,
}

impl ControlledSigner {
    pub(crate) async fn wait_entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.entered.notified())
            .await
            .unwrap();
    }

    pub(crate) fn new(fail: bool) -> Arc<Self> {
        Arc::new(Self {
            keys: Keys::generate(),
            entered: Notify::new(),
            release: Notify::new(),
            fail,
            // No Nostr kind can equal this sentinel (kind:0 is a real profile).
            fail_kind: AtomicUsize::new(usize::MAX),
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
            if self.fail || self.fail_kind.load(Ordering::SeqCst) == unsigned.kind.as_u16() as usize
            {
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
async fn local_signatures_match_fields_and_identity() {
    let keys = Keys::generate();
    let signer = ActiveUserSigner::local(keys.clone());
    let builder = EventBuilder::new(Kind::TextNote, "fixed template")
        .custom_created_at(Timestamp::from(123456));
    let expected = builder.clone().sign_with_keys(&keys).unwrap();
    let actual = signer.sign_event(builder).await.unwrap();
    assert_eq!(actual.id, expected.id);
    assert_eq!(actual.pubkey, expected.pubkey);
    assert_eq!(actual.created_at, expected.created_at);
    assert_eq!(actual.tags, expected.tags);
    assert_eq!(actual.content, expected.content);
    actual.verify().unwrap();
    let encrypted = signer
        .signer()
        .nip44_encrypt(&signer.public_key(), "private")
        .await
        .unwrap();
    assert_eq!(
        signer
            .signer()
            .nip44_decrypt(&signer.public_key(), &encrypted)
            .await
            .unwrap(),
        "private"
    );
}

#[tokio::test]
async fn caches_identity_and_waits_for_async_signer_failure() {
    let controlled = ControlledSigner::new(true);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    assert_eq!(signer.public_key(), controlled.keys.public_key());
    assert_eq!(
        signer.signer().get_public_key().await.unwrap(),
        controlled.keys.public_key()
    );
    let task = tokio::spawn(async move {
        signer
            .sign_event(EventBuilder::new(Kind::TextNote, "test"))
            .await
    });
    controlled.wait_entered().await;
    assert!(!task.is_finished());
    controlled.release.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap_err(),
        "deliberate signer failure"
    );
    assert_eq!(controlled.public_key_reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn owner_memory_capability_preserves_keyed_address_validation() {
    use buzz_core_pkg::engram::{build_event, Body};
    let owner = Keys::generate();
    let agent = Keys::generate();
    let signer = ActiveUserSigner::local(owner.clone());
    let body = Body::Memory {
        slug: "mem/test-memory".into(),
        value: Some("remember".into()),
    };
    let event = build_event(&agent, &owner.public_key(), &body, 1).unwrap();
    assert_eq!(
        signer
            .read_agent_memory(&event, &agent.public_key())
            .await
            .unwrap()
            .unwrap()
            .to_json_bytes(),
        body.to_json_bytes()
    );

    // Correct signature and decryptable ciphertext do not make a false address valid.
    let invalid = EventBuilder::new(event.kind, event.content.clone())
        .tags([
            nostr::Tag::parse(["d", &"0".repeat(64)]).unwrap(),
            nostr::Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .unwrap();
    invalid.verify().unwrap();
    assert!(signer
        .read_agent_memory(&invalid, &agent.public_key())
        .await
        .unwrap()
        .is_none());
    assert!(signer
        .read_agent_memory(&event, &Keys::generate().public_key())
        .await
        .unwrap()
        .is_none());

    // Envelope, ciphertext, and keyed address alone cannot authenticate a record.
    let mut invalid_signature = event.clone();
    invalid_signature.sig = EventBuilder::new(Kind::TextNote, "unrelated")
        .sign_with_keys(&agent)
        .unwrap()
        .sig;
    assert!(signer
        .read_agent_memory(&invalid_signature, &agent.public_key())
        .await
        .unwrap()
        .is_none());

    // Operational inability must not be transformed into a legitimate empty list.
    let unavailable = ActiveUserSigner::new(Arc::new(owner)).await.unwrap();
    assert!(unavailable
        .read_agent_memory(&event, &agent.public_key())
        .await
        .unwrap_err()
        .contains("keyed agent memory"));
}

#[derive(Debug)]
struct ControlledAgentCapabilities {
    keys: Keys,
    entered: Notify,
    release: Notify,
    fail: bool,
}

impl ControlledAgentCapabilities {
    async fn gate(&self) -> Result<(), String> {
        self.entered.notify_one();
        self.release.notified().await;
        if self.fail {
            return Err("deliberate auxiliary failure".into());
        }
        Ok(())
    }

    async fn wait_entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.entered.notified())
            .await
            .unwrap();
    }
}

impl super::AgentCapabilities for ControlledAgentCapabilities {
    fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    fn authorize_agent<'a>(
        &'a self,
        agent: &'a PublicKey,
        conditions: &'a str,
    ) -> BoxedFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.gate().await?;
            super::AgentCapabilities::authorize_agent(&self.keys, agent, conditions).await
        })
    }

    fn read_agent_memory<'a>(
        &'a self,
        event: &'a Event,
        agent: &'a PublicKey,
    ) -> BoxedFuture<'a, Result<Option<buzz_core_pkg::engram::Body>, String>> {
        Box::pin(async move {
            self.gate().await?;
            super::AgentCapabilities::read_agent_memory(&self.keys, event, agent).await
        })
    }
}

#[tokio::test]
async fn auxiliary_operations_await_captured_backend_without_local_fallback() {
    use buzz_core_pkg::engram::{build_event, Body};

    for fail in [false, true] {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let agent_key = agent.public_key();
        let conditions = "kind=1&created_at<2000000000";
        let body = Body::Memory {
            slug: "mem/captured-owner".into(),
            value: Some("captured memory".into()),
        };
        let event = build_event(&agent, &owner.public_key(), &body, 1).unwrap();
        let controlled = Arc::new(ControlledAgentCapabilities {
            keys: owner.clone(),
            entered: Notify::new(),
            release: Notify::new(),
            fail,
        });
        // The ordinary signer still has usable local keys: failure must not
        // fall back to them. Only the auxiliary capability is replaced.
        let mut active =
            ActiveUserSigner::local(owner.clone()).with_test_agent_capabilities(controlled.clone());
        let captured = active.clone();
        let task = tokio::spawn(async move {
            let authorization = captured.authorize_agent(&agent_key, conditions).await;
            let memory = captured.read_agent_memory(&event, &agent_key).await;
            (captured.public_key(), authorization, memory)
        });

        controlled.wait_entered().await;
        assert!(!task.is_finished());
        active = ActiveUserSigner::local(Keys::generate());
        assert_ne!(active.public_key(), owner.public_key());
        controlled.release.notify_one();
        controlled.wait_entered().await;
        assert!(!task.is_finished());
        controlled.release.notify_one();

        let (identity, authorization, memory) =
            tokio::time::timeout(std::time::Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(identity, owner.public_key());
        if fail {
            assert_eq!(authorization.unwrap_err(), "deliberate auxiliary failure");
            assert_eq!(memory.unwrap_err(), "deliberate auxiliary failure");
        } else {
            let authorization = authorization.unwrap();
            assert_eq!(
                buzz_sdk_pkg::nip_oa::verify_auth_tag(&authorization, &agent_key).unwrap(),
                owner.public_key()
            );
            let fields: serde_json::Value = serde_json::from_str(&authorization).unwrap();
            assert_eq!(fields[2].as_str(), Some(conditions));
            assert_eq!(
                memory.unwrap().unwrap().to_json_bytes(),
                body.to_json_bytes()
            );
        }
    }
}

#[test]
#[should_panic(expected = "test capabilities must match signer identity")]
fn auxiliary_capability_must_match_captured_identity() {
    ActiveUserSigner::local(Keys::generate())
        .with_test_agent_capabilities(Arc::new(Keys::generate()));
}

fn remote_validity() -> crate::builderlab::session::SessionValidity {
    crate::builderlab::session::SessionValidity {
        generation: 42,
        cancel: tokio_util::sync::CancellationToken::new(),
        expires: chrono::Utc::now() + chrono::Duration::minutes(5),
    }
}

#[tokio::test]
async fn remote_memory_is_explicitly_unsupported_and_session_fenced() {
    use crate::remote_signer::{RemoteSigner, RemoteSignerSession};
    use buzz_core_pkg::engram::{build_event, Body};

    let owner = Keys::generate();
    let agent = Keys::generate();
    let event = build_event(
        &agent,
        &owner.public_key(),
        &Body::Memory {
            slug: "mem/remote".into(),
            value: Some("not an empty listing".into()),
        },
        1,
    )
    .unwrap();
    let validity = remote_validity();
    // No endpoint is contacted: keyed memory validation has no remote API.
    let remote = RemoteSigner::new(
        "https://signer.invalid/api/goose",
        RemoteSignerSession::new("test-session", owner.public_key()).unwrap(),
    )
    .unwrap();
    let remote = Arc::new(remote);
    let signer = ActiveUserSigner::new(remote.clone())
        .await
        .unwrap()
        .with_agent_capabilities(remote)
        .unwrap()
        .with_lifetime(Arc::new(validity.clone()));
    assert!(!signer.has_local_crypto());
    assert_eq!(signer.generation(), Some(42));
    assert_eq!(
        signer
            .read_agent_memory(&event, &agent.public_key())
            .await
            .unwrap_err(),
        "remote signer: unsupported capability"
    );
    validity.cancel.cancel();
    assert!(signer
        .read_agent_memory(&event, &agent.public_key())
        .await
        .unwrap_err()
        .contains("canceled or expired"));
    assert!(signer
        .authorize_agent(&agent.public_key(), "kind=1")
        .await
        .unwrap_err()
        .contains("canceled or expired"));
    assert!(signer.signer().get_public_key().await.is_err());
    tokio::time::timeout(std::time::Duration::from_secs(1), signer.canceled())
        .await
        .unwrap();
}

#[tokio::test]
async fn captured_session_cancels_pending_auxiliary_operations() {
    for memory in [false, true] {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let controlled = Arc::new(ControlledAgentCapabilities {
            keys: owner.clone(),
            entered: Notify::new(),
            release: Notify::new(),
            fail: false,
        });
        let mut signer =
            ActiveUserSigner::local(owner).with_test_agent_capabilities(controlled.clone());
        let validity = remote_validity();
        signer.validity = Some(Arc::new(validity.clone()));
        let task = tokio::spawn(async move {
            if memory {
                let event = EventBuilder::new(Kind::TextNote, "unused")
                    .sign_with_keys(&agent)
                    .unwrap();
                signer
                    .read_agent_memory(&event, &agent.public_key())
                    .await
                    .map(|_| ())
            } else {
                signer
                    .authorize_agent(&agent.public_key(), "kind=1")
                    .await
                    .map(|_| ())
            }
        });
        controlled.wait_entered().await;
        validity.cancel.cancel();
        // Never release the backend: cancellation must drop the pending work.
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.contains("canceled or expired"));
    }
}

#[derive(Debug)]
struct TestLifetime(tokio_util::sync::CancellationToken);
impl super::CapabilityLifetime for TestLifetime {
    fn generation(&self) -> u64 {
        17
    }
    fn check(&self) -> Result<(), String> {
        if self.0.is_cancelled() {
            Err("retired capability".into())
        } else {
            Ok(())
        }
    }
    fn canceled(&self) -> nostr::util::BoxedFuture<'_, ()> {
        Box::pin(self.0.cancelled())
    }
}

#[tokio::test]
async fn captured_lifetime_cancels_pending_work_without_replacement() {
    let lifetime = Arc::new(TestLifetime(tokio_util::sync::CancellationToken::new()));
    let signer = ActiveUserSigner::local(Keys::generate()).with_lifetime(lifetime.clone());
    assert_eq!(signer.generation(), Some(17));
    let entered = Arc::new(tokio::sync::Notify::new());
    let started = entered.clone();
    let task = tokio::spawn(async move {
        signer
            .run(async move {
                started.notify_one();
                std::future::pending::<Result<(), String>>().await
            })
            .await
    });
    entered.notified().await;
    lifetime.0.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}

#[tokio::test]
async fn local_lifetime_does_not_change_success_or_error() {
    let signer = ActiveUserSigner::local(Keys::generate());
    assert_eq!(signer.generation(), None);
    assert_eq!(signer.run(async { Ok(19) }).await, Ok(19));
    assert_eq!(
        signer.run(async { Err::<(), _>("original".into()) }).await,
        Err("original".into())
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), signer.canceled())
            .await
            .is_err()
    );
}
