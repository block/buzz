use super::*;

fn setup() -> (PairingSession, PairingSession, String) {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    let (target, _) = PairingSession::new_target(&qr).expect("target");
    let offer = target
        .build_event(&PairingMessage::Offer {
            session_id: hex::encode(target.session_id),
            version: 1,
            confirmation: Some("desktop-code-v1".into()),
        })
        .expect("offer");
    assert!(
        source
            .handle_offer_with_confirmation(&offer)
            .expect("offer")
            .1
    );
    let (code, challenge) = source.start_desktop_code().expect("challenge");
    assert_eq!(
        target.decrypt_message(&challenge).expect("decrypt"),
        PairingMessage::DesktopCode {}
    );
    assert_eq!(code.len(), 6);
    (source, target, code)
}

fn submission(target: &PairingSession, code: &str, attempt: u8) -> Event {
    target
        .build_event(&PairingMessage::CodeSubmit {
            code: code.into(),
            request_id: attempt.to_string(),
        })
        .expect("submission")
}

#[test]
fn only_source_only_code_releases_identity() {
    let (mut source, mut target, code) = setup();
    assert!(source
        .send_payload(PayloadType::Custom, Zeroizing::new("secret".into()))
        .is_err());
    let event = submission(&target, &code, 1);
    let (proof, accepted) = source.handle_target_code(&event).expect("verify");
    assert!(accepted);
    assert!(source.handle_target_code(&event).is_err());
    target.handle_sas_confirm(&proof).expect("source proof");
    target.confirm_target_sas().expect("user approves import");
    let payload = source
        .send_payload(PayloadType::Custom, Zeroizing::new("secret".into()))
        .expect("payload");
    assert_eq!(
        &*target.handle_payload(&payload).expect("import").1,
        "secret"
    );
}

#[test]
fn qr_derived_transcript_cannot_authorize_release() {
    let (mut source, target, _) = setup();
    let hash = derive_transcript_hash(
        &target.session_id,
        &source.pubkey().to_bytes(),
        &target.pubkey().to_bytes(),
        &target.sas_input.expect("sas"),
        &target.session_secret,
    );
    let event = target
        .build_event(&PairingMessage::SasConfirm {
            transcript_hash: hex::encode(hash),
        })
        .expect("proof");
    assert!(source.handle_target_code(&event).is_err());
    assert_eq!(source.state(), SessionState::Confirming);
    assert!(source
        .send_payload(PayloadType::Custom, Zeroizing::new("secret".into()))
        .is_err());
}

#[test]
fn five_wrong_guesses_abort_without_reset_or_replay_bypass() {
    let (mut source, target, code) = setup();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for attempt in 1..=5 {
        let event = submission(&target, wrong, attempt);
        let (reply, accepted) = source.handle_target_code(&event).expect("reject");
        assert!(!accepted);
        assert_eq!(
            target.decrypt_message(&reply).expect("reply"),
            PairingMessage::CodeRejected {
                request_id: attempt.to_string(),
                remaining_attempts: 5 - attempt
            }
        );
        assert!(
            source.handle_target_code(&event).is_err(),
            "duplicate submission"
        );
        assert!(
            source.start_desktop_code().is_err(),
            "cannot regenerate code/budget"
        );
        assert!(source
            .send_payload(PayloadType::Custom, Zeroizing::new("secret".into()))
            .is_err());
    }
    assert_eq!(source.state(), SessionState::Aborted);
    assert!(source
        .handle_target_code(&submission(&target, &code, 6))
        .is_err());
}

#[test]
fn wrong_then_correct_code_succeeds() {
    let (mut source, target, code) = setup();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    assert!(
        !source
            .handle_target_code(&submission(&target, wrong, 1))
            .expect("reject")
            .1
    );
    assert!(
        source
            .handle_target_code(&submission(&target, &code, 2))
            .expect("accept")
            .1
    );
}

#[test]
fn wrong_peer_tampering_and_expiry_cannot_authorize() {
    let (mut source, target, code) = setup();
    let (other, _) = PairingSession::new_source("wss://relay.test".into());
    let mut bad = submission(&target, &code, 1);
    bad.pubkey = other.pubkey();
    assert!(source.handle_target_code(&bad).is_err());
    let mut tampered = submission(&target, &code, 2);
    tampered.content.push('x');
    assert!(source.handle_target_code(&tampered).is_err());
    assert_eq!(source.code_attempts, 0);
    source.created_at = Instant::now() - source.timeout - Duration::from_secs(1);
    assert!(source
        .handle_target_code(&submission(&target, &code, 3))
        .is_err());
}

#[test]
fn legacy_capabilities_never_enable_automatic_release() {
    for capability in [
        None,
        Some("code-entry"),
        Some("unknown"),
        Some("desktop-code-v1"),
    ] {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (target, _) = PairingSession::new_target(&qr).expect("target");
        let event = target
            .build_event(&PairingMessage::Offer {
                session_id: hex::encode(target.session_id),
                version: 1,
                confirmation: capability.map(str::to_string),
            })
            .expect("offer");
        assert_eq!(event.tags.len(), 1);
        let (_, enabled) = source
            .handle_offer_with_confirmation(&event)
            .expect("offer");
        assert_eq!(enabled, capability == Some("desktop-code-v1"));
        assert_eq!(source.start_desktop_code().is_ok(), enabled);
    }
}

#[test]
fn readiness_delay_and_late_scan_share_the_original_deadline() {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    // Simulate the maximum 35-second readiness wait without sleeping.
    source.created_at = Instant::now() - Duration::from_secs(35);
    let deadline = source.deadline();
    let remaining = deadline.saturating_duration_since(Instant::now());
    assert!(remaining <= Duration::from_secs(85));
    assert!(remaining > Duration::from_secs(84));
    // A scan 84 seconds after readiness still has one second to be accepted.
    source.created_at -= Duration::from_secs(84);
    let (_, offer) = PairingSession::new_target(&qr).unwrap();
    assert!(source.handle_offer(&offer).is_ok());
    assert!(!source.is_expired());
    // The transport deadline and protocol expiry both end the original window.
    source.created_at -= Duration::from_secs(2);
    assert!(source.deadline() < Instant::now());
    assert!(source.is_expired());
    assert!(matches!(
        source.confirm_sas(),
        Err(PairingError::SessionExpired)
    ));
}
