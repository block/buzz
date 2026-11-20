//! Public-API interoperability journeys for strict NIP-AB code entry.
use buzz_core::pairing::{PairingError, PairingSession, PayloadType, SessionState};
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use zeroize::Zeroizing;

fn setup() -> (PairingSession, PairingSession) {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    let (target, offer) = PairingSession::new_target_with_desktop_code(&qr).unwrap();
    assert!(source.handle_offer_with_confirmation(&offer).unwrap().1);
    (source, target)
}

fn input(code: &str) -> Zeroizing<String> {
    Zeroizing::new(code.into())
}

#[test]
fn both_devices_bind_authorization_to_the_same_live_exchange() {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    assert!(source.binding().is_err());
    let (mut target, offer) = PairingSession::new_target_with_desktop_code(&qr).unwrap();
    source.handle_offer_with_confirmation(&offer).unwrap();
    let expected = source.binding().unwrap();
    assert_eq!(expected, target.binding().unwrap());
    assert_eq!(expected.source_pubkey, source.pubkey());
    assert_eq!(expected.target_pubkey, target.pubkey());
    let (other, _) = PairingSession::new_target_with_desktop_code(&qr).unwrap();
    assert_ne!(
        expected.transcript_hash,
        other.binding().unwrap().transcript_hash
    );
    let (code, challenge) = source.start_desktop_code().unwrap();
    target.handle_desktop_code(&challenge).unwrap();
    let submission = target.submit_desktop_code(input(&code)).unwrap();
    let (proof, _) = source.handle_target_code(&submission).unwrap();
    target.handle_sas_confirm(&proof).unwrap();
    assert_eq!(expected, source.binding().unwrap());
    assert_eq!(expected, target.binding().unwrap());
    target.confirm_target_sas().unwrap();
    let payload = source
        .send_payload(PayloadType::Custom, input("test"))
        .unwrap();
    assert!(source.binding().is_err());
    target.handle_payload(&payload).unwrap();
    assert!(target.binding().is_err());
    source
        .handle_complete(&target.send_complete().unwrap())
        .unwrap();
    assert!(source.binding().is_err());
    assert!(target.binding().is_err());
}

#[test]
fn cancelled_exchange_cannot_produce_an_authorization_binding() {
    let (mut source, mut target) = setup();
    let abort = source
        .abort(buzz_core::pairing::AbortReason::UserDenied)
        .unwrap()
        .unwrap();
    target.handle_abort(&abort).unwrap();
    assert!(source.binding().is_err());
    assert!(target.binding().is_err());
}

fn wrong(code: &str) -> &'static str {
    if code == "000000" {
        "000001"
    } else {
        "000000"
    }
}

fn resign(source: &PairingSession, event: &Event) -> Event {
    source
        .sign_event(
            EventBuilder::new(event.kind, &event.content)
                .tags(event.tags.iter().cloned())
                .custom_created_at(Timestamp::from(event.created_at.as_secs() + 1)),
        )
        .unwrap()
}

#[test]
fn code_entry_retries_then_import_requires_explicit_intent_and_durable_ack() {
    let (mut source, mut target) = setup();
    let (code, challenge) = source.start_desktop_code().unwrap();
    assert!(target.submit_desktop_code(input(&code)).is_err());
    target.handle_desktop_code(&challenge).unwrap();
    assert!(target.handle_desktop_code(&challenge).is_err());
    for invalid in ["", "12345", "1234567", "abcdef", "１２３４５６"] {
        assert!(target.submit_desktop_code(input(invalid)).is_err());
    }
    let submission = target.submit_desktop_code(input(wrong(&code))).unwrap();
    assert!(target.submit_desktop_code(input(&code)).is_err());
    let (rejection, accepted) = source.handle_target_code(&submission).unwrap();
    assert!(!accepted);
    assert_eq!(target.handle_code_rejected(&rejection).unwrap(), 4);
    let correct = target.submit_desktop_code(input(&code)).unwrap();
    // Even re-signed stale content must not resolve the new pending request.
    assert!(target
        .handle_code_rejected(&resign(&source, &rejection))
        .is_err());
    assert!(target.submit_desktop_code(input(&code)).is_err());
    let (proof, accepted) = source.handle_target_code(&correct).unwrap();
    assert!(accepted);
    target.handle_sas_confirm(&proof).unwrap();
    let payload = source
        .send_payload(PayloadType::Nsec, input("identity-secret"))
        .unwrap();
    assert!(
        target.handle_payload(&payload).is_err(),
        "local import approval is mandatory"
    );
    assert!(
        target.send_complete().is_err(),
        "cannot acknowledge an unreceived secret"
    );
    target.confirm_target_sas().unwrap();
    let (kind, received) = target.handle_payload(&payload).unwrap();
    assert_eq!(kind, PayloadType::Nsec);
    assert_eq!(&*received, "identity-secret");
    assert!(
        target.handle_payload(&payload).is_err(),
        "import is single use"
    );
    let complete = target.send_complete_with_result(true).unwrap();
    source.handle_complete(&complete).unwrap();
    assert_eq!(source.state(), SessionState::Completed);
    assert_eq!(target.state(), SessionState::Completed);
}

#[test]
fn strict_target_rejects_legacy_proof_before_challenge_or_code_submission() {
    for challenge_first in [false, true] {
        let (mut source, mut target) = setup();
        if challenge_first {
            let (_, challenge) = source.start_desktop_code().unwrap();
            target.handle_desktop_code(&challenge).unwrap();
        }
        let legacy_proof = source.confirm_sas().unwrap();
        assert!(target.handle_sas_confirm(&legacy_proof).is_err());
        assert_eq!(target.state(), SessionState::Confirming);
        assert!(target.confirm_target_sas().is_err());
    }
    // The legacy constructor still negotiates the existing explicit SAS flow.
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    let (mut target, offer) = PairingSession::new_target(&qr).unwrap();
    assert!(!source.handle_offer_with_confirmation(&offer).unwrap().1);
    assert!(target.submit_desktop_code(input("123456")).is_err());
    target
        .handle_sas_confirm(&source.confirm_sas().unwrap())
        .unwrap();
    target.confirm_target_sas().unwrap();
}

#[test]
fn five_rejections_end_both_peers_and_replays_cannot_reset_the_budget() {
    let (mut source, mut target) = setup();
    let (code, challenge) = source.start_desktop_code().unwrap();
    target.handle_desktop_code(&challenge).unwrap();
    for remaining in (0..5).rev() {
        let submission = target.submit_desktop_code(input(wrong(&code))).unwrap();
        let (rejection, accepted) = source.handle_target_code(&submission).unwrap();
        assert!(!accepted);
        assert!(source.handle_target_code(&submission).is_err());
        assert_eq!(target.handle_code_rejected(&rejection).unwrap(), remaining);
        assert!(target.handle_code_rejected(&rejection).is_err());
        assert!(target
            .handle_desktop_code(&resign(&source, &challenge))
            .is_err());
        assert!(source.start_desktop_code().is_err());
    }
    assert_eq!(source.state(), SessionState::Aborted);
    assert_eq!(target.state(), SessionState::Aborted);
    assert!(target.submit_desktop_code(input(&code)).is_err());
    assert!(target.confirm_target_sas().is_err());
}

#[test]
fn authenticated_peer_and_recipient_checks_preserve_a_valid_challenge() {
    let (mut source, mut target) = setup();
    let (code, challenge) = source.start_desktop_code().unwrap();
    let attacker = Keys::generate();
    let ciphertext = nip44::encrypt(
        attacker.secret_key(),
        &target.pubkey(),
        "{\"type\":\"desktop-code\"}",
        nip44::Version::V2,
    )
    .unwrap();
    let wrong_peer = EventBuilder::new(Kind::Custom(24134), ciphertext)
        .tags([Tag::public_key(target.pubkey())])
        .sign_with_keys(&attacker)
        .unwrap();
    let wrong_recipient = source
        .sign_event(
            EventBuilder::new(challenge.kind, &challenge.content)
                .tags([Tag::public_key(attacker.public_key())]),
        )
        .unwrap();
    let wrong_kind = source
        .sign_event(
            EventBuilder::new(Kind::TextNote, &challenge.content)
                .tags([Tag::public_key(target.pubkey())]),
        )
        .unwrap();
    let mut tampered = challenge.clone();
    tampered.content.push('x');
    for candidate in [wrong_peer.clone(), wrong_recipient, wrong_kind, tampered] {
        assert!(target.handle_desktop_code(&candidate).is_err());
    }
    target.handle_desktop_code(&challenge).unwrap();
    let submission = target.submit_desktop_code(input(&code)).unwrap();
    assert!(target.handle_code_rejected(&wrong_peer).is_err());
    let (proof, accepted) = source.handle_target_code(&submission).unwrap();
    assert!(accepted);
    target.handle_sas_confirm(&proof).unwrap();
}

#[test]
fn failed_secure_import_is_reported_without_success_or_retry() {
    let (mut source, mut target) = setup();
    let (code, challenge) = source.start_desktop_code().unwrap();
    target.handle_desktop_code(&challenge).unwrap();
    let submission = target.submit_desktop_code(input(&code)).unwrap();
    let (proof, _) = source.handle_target_code(&submission).unwrap();
    target.handle_sas_confirm(&proof).unwrap();
    target.confirm_target_sas().unwrap();
    let payload = source
        .send_payload(PayloadType::Nsec, input("identity-secret"))
        .unwrap();
    target.handle_payload(&payload).unwrap();
    let failed = target.send_complete_with_result(false).unwrap();
    assert!(matches!(
        source.handle_complete(&failed),
        Err(PairingError::UnexpectedMessage { .. })
    ));
    assert_eq!(source.state(), SessionState::Aborted);
    assert_eq!(target.state(), SessionState::Aborted);
    assert!(target.send_complete().is_err());
    assert!(source
        .send_payload(PayloadType::Nsec, input("identity-secret"))
        .is_err());
}
