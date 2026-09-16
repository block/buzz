use super::*;
use crate::active_user_signer::tests::ControlledSigner;

#[tokio::test]
async fn renderer_signing_awaits_backend_and_keeps_requested_fields() {
    let controlled = ControlledSigner::new(false);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let task = tokio::spawn(async move {
        sign_renderer_event(
            &signer,
            9,
            "hello".into(),
            Some(123456),
            vec![vec!["h".into(), "channel".into()]],
        )
        .await
    });
    controlled.wait_entered().await;
    assert!(!task.is_finished());
    controlled.release.notify_one();
    let event = Event::from_json(task.await.unwrap().unwrap()).unwrap();
    let expected = EventBuilder::new(Kind::Custom(9), "hello")
        .custom_created_at(Timestamp::from(123456))
        .tags([Tag::parse(["h", "channel"]).unwrap()])
        .sign_with_keys(&controlled.keys)
        .unwrap();
    assert_eq!(event.id, expected.id);
    event.verify().unwrap();
}

#[tokio::test]
async fn renderer_auth_keeps_literal_relay_and_challenge_and_propagates_failure() {
    for fail in [false, true] {
        let controlled = ControlledSigner::new(fail);
        let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
        let task = tokio::spawn(async move {
            sign_renderer_auth(&signer, "test-challenge", "wss://relay.example/path").await
        });
        controlled.wait_entered().await;
        assert!(!task.is_finished());
        controlled.release.notify_one();
        let result = task.await.unwrap();
        if fail {
            assert!(result.unwrap_err().contains("deliberate signer failure"));
        } else {
            let event = Event::from_json(result.unwrap()).unwrap();
            assert_eq!(event.kind.as_u16(), 22242);
            assert!(event.content.is_empty());
            assert_eq!(
                event.tags.clone().to_vec(),
                vec![
                    Tag::parse(["relay", "wss://relay.example/path"]).unwrap(),
                    Tag::parse(["challenge", "test-challenge"]).unwrap()
                ]
            );
            event.verify().unwrap();
        }
    }
}

#[tokio::test]
async fn observer_control_is_user_authored_and_agent_encrypted() {
    let user = Keys::generate();
    let agent = Keys::generate();
    let signer = ActiveUserSigner::local(user.clone());
    let payload = serde_json::json!({"type": "test-control"});
    let event = Event::from_json(
        build_observer_control_with_signer(&signer, &agent.public_key().to_hex(), &payload)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(event.pubkey, user.public_key());
    assert_ne!(event.pubkey, agent.public_key());
    event.verify().unwrap();
    let decrypted: serde_json::Value =
        buzz_core_pkg::observer::decrypt_observer_payload(&agent, &event).unwrap();
    assert_eq!(decrypted, payload);
    assert_eq!(
        decrypt_observer_event_with_signer(
            &ActiveUserSigner::local(agent.clone()),
            &event.as_json()
        )
        .await
        .unwrap(),
        payload
    );
    let mut tampered = event;
    tampered.content.push('x');
    assert_eq!(
        decrypt_observer_event_with_signer(&ActiveUserSigner::local(agent), &tampered.as_json())
            .await
            .unwrap_err(),
        "observer event has invalid ID"
    );
}

// A crypto-only delay/failure seam: observer tests must await decryption, not signing.
#[derive(Debug)]
struct ObserverDecryptSigner {
    control: std::sync::Arc<ControlledSigner>,
    plaintext: Option<String>,
}

macro_rules! delegate_observer_crypto {
    ($method:ident) => {
        fn $method<'a>(
            &'a self,
            public_key: &'a PublicKey,
            content: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::SignerError>> {
            nostr::NostrSigner::$method(&self.control.keys, public_key, content)
        }
    };
}

impl nostr::NostrSigner for ObserverDecryptSigner {
    fn backend(&self) -> nostr::signer::SignerBackend<'_> {
        nostr::signer::SignerBackend::Custom("observer-test".into())
    }

    fn get_public_key(
        &self,
    ) -> nostr::util::BoxedFuture<'_, Result<PublicKey, nostr::SignerError>> {
        nostr::NostrSigner::get_public_key(self.control.as_ref())
    }

    fn sign_event(
        &self,
        event: nostr::UnsignedEvent,
    ) -> nostr::util::BoxedFuture<'_, Result<Event, nostr::SignerError>> {
        nostr::NostrSigner::sign_event(&self.control.keys, event)
    }

    delegate_observer_crypto!(nip04_encrypt);
    delegate_observer_crypto!(nip04_decrypt);
    delegate_observer_crypto!(nip44_encrypt);

    fn nip44_decrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::SignerError>> {
        Box::pin(async move {
            self.control.entered.notify_one();
            self.control.release.notified().await;
            if self.control.fail {
                return Err(nostr::SignerError::from("deliberate decrypt failure"));
            }
            if let Some(plaintext) = &self.plaintext {
                return Ok(plaintext.clone());
            }
            nostr::NostrSigner::nip44_decrypt(&self.control.keys, public_key, content).await
        })
    }
}

fn observer_test_event(sender: &Keys, content: String) -> Event {
    EventBuilder::new(Kind::Custom(21420), content)
        .sign_with_keys(sender)
        .unwrap()
}

#[tokio::test]
async fn observer_local_decrypt_matches_prior_core_results_and_errors() {
    use buzz_core_pkg::observer::{decrypt_observer_payload, NIP44_MAX_CONTENT_LEN};
    let sender = Keys::generate();
    let recipient = Keys::generate();
    let signer = ActiveUserSigner::local(recipient.clone());
    let mut contents = vec![
        String::new(),
        "x".repeat(132),
        "x".repeat(NIP44_MAX_CONTENT_LEN + 1),
    ];
    for plaintext in ["{\"hello\":\"world\"}", "null", "[1,true]", "not JSON"] {
        contents.push(
            nostr::nips::nip44::encrypt(
                sender.secret_key(),
                &recipient.public_key(),
                plaintext,
                nostr::nips::nip44::Version::V2,
            )
            .unwrap(),
        );
    }
    // A valid envelope encrypted for a different identity must also preserve errors.
    contents.push(
        nostr::nips::nip44::encrypt(
            sender.secret_key(),
            &Keys::generate().public_key(),
            "null",
            nostr::nips::nip44::Version::V2,
        )
        .unwrap(),
    );
    for content in contents {
        let event = observer_test_event(&sender, content);
        let expected = decrypt_observer_payload::<serde_json::Value>(&recipient, &event)
            .map_err(|error| format!("decrypt observer event failed: {error}"));
        assert_eq!(
            decrypt_observer_event_with_signer(&signer, &event.as_json()).await,
            expected
        );
    }
}

#[tokio::test]
async fn observer_decryption_awaits_backend_and_propagates_failure_and_plaintext_bounds() {
    use buzz_core_pkg::observer::OBSERVER_MAX_PLAINTEXT_LEN;
    for (fail, plaintext) in [
        (false, None),
        (true, None),
        (false, Some("x".repeat(OBSERVER_MAX_PLAINTEXT_LEN + 1))),
        (
            false,
            Some(" ".repeat(OBSERVER_MAX_PLAINTEXT_LEN - 4) + "null"),
        ),
    ] {
        let control = ControlledSigner::new(fail);
        let sender = Keys::generate();
        let content = buzz_core_pkg::observer::encrypt_observer_payload(
            &sender,
            &control.keys.public_key(),
            &serde_json::json!({"ok": true}),
        )
        .unwrap();
        let event = observer_test_event(&sender, content);
        let expected = match plaintext.as_ref() {
            Some(value) if value.len() > OBSERVER_MAX_PLAINTEXT_LEN => Err(format!(
                "decrypt observer event failed: observer plaintext exceeds {OBSERVER_MAX_PLAINTEXT_LEN} bytes (got {})", value.len()
            )),
            Some(_) => Ok(serde_json::Value::Null),
            None if fail => Err("decrypt observer event failed: NIP-44 error: deliberate decrypt failure".into()),
            None => Ok(serde_json::json!({"ok": true})),
        };
        let signer = ActiveUserSigner::new(std::sync::Arc::new(ObserverDecryptSigner {
            control: control.clone(),
            plaintext,
        }))
        .await
        .unwrap();
        let task = tokio::spawn(async move {
            decrypt_observer_event_with_signer(&signer, &event.as_json()).await
        });
        control.wait_entered().await;
        assert!(!task.is_finished());
        control.release.notify_one();
        assert_eq!(task.await.unwrap(), expected);
        assert_eq!(
            control
                .public_key_reads
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }
}

#[tokio::test]
async fn observer_invalid_events_are_rejected_before_backend_decryption() {
    let control = ControlledSigner::new(true);
    let signer = ActiveUserSigner::new(std::sync::Arc::new(ObserverDecryptSigner {
        control: control.clone(),
        plaintext: None,
    }))
    .await
    .unwrap();
    let event = observer_test_event(&Keys::generate(), "x".repeat(132));
    let mut invalid_id = serde_json::to_value(&event).unwrap();
    invalid_id["content"] = "tampered".into();
    let mut invalid_signature = serde_json::to_value(&event).unwrap();
    invalid_signature["sig"] = "0".repeat(128).into();
    for (json, expected) in [
        ("not JSON".to_owned(), "invalid event:"),
        (invalid_id.to_string(), "observer event has invalid ID"),
        (
            invalid_signature.to_string(),
            "observer event has invalid signature",
        ),
        (
            observer_test_event(&Keys::generate(), String::new()).as_json(),
            "decrypt observer event failed: invalid NIP-44 ciphertext length: 0",
        ),
    ] {
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            decrypt_observer_event_with_signer(&signer, &json),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.starts_with(expected), "{error}");
    }
    // Notify retains a permit if any invalid input incorrectly reached the signer.
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(10),
        control.entered.notified()
    )
    .await
    .is_err());
}
