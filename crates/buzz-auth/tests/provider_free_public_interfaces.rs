use buzz_auth::{
    generate_challenge, verify_nip42_authorization_proof, ActiveLocalBinding, AuthError,
    Nip42AuthorizationProofError, ProofTransport,
};
use buzz_core::CommunityId;
use chrono::{Duration, Utc};
use nostr::{EventBuilder, Keys, RelayUrl};
use uuid::Uuid;

const TEST_RELAY: &str = "wss://relay.example.com";

#[test]
fn storage_adapter_is_checked_public_and_redacted() {
    let domain = CommunityId::from_uuid(Uuid::from_u128(1));
    let binding_id = Uuid::from_u128(2);
    let event_author = Keys::generate().public_key();
    let binding = ActiveLocalBinding::from_storage_parts(
        domain,
        "https://issuer.example".to_owned(),
        "opaque-subject".to_owned(),
        binding_id,
        1,
        event_author,
        None,
    )
    .expect("valid authoritative storage row");

    let debug = format!("{binding:?}");
    assert_eq!(debug, "ActiveLocalBinding([REDACTED])");
    assert!(!debug.contains("issuer"));
    assert!(!debug.contains("opaque-subject"));
    assert!(!debug.contains(&binding_id.to_string()));

    for invalid in [
        ActiveLocalBinding::from_storage_parts(
            CommunityId::from_uuid(Uuid::nil()),
            "https://issuer.example".to_owned(),
            "opaque-subject".to_owned(),
            binding_id,
            1,
            event_author,
            None,
        ),
        ActiveLocalBinding::from_storage_parts(
            domain,
            String::new(),
            "opaque-subject".to_owned(),
            binding_id,
            1,
            event_author,
            None,
        ),
        ActiveLocalBinding::from_storage_parts(
            domain,
            "https://issuer.example".to_owned(),
            String::new(),
            binding_id,
            1,
            event_author,
            None,
        ),
        ActiveLocalBinding::from_storage_parts(
            domain,
            "i".repeat(2_049),
            "opaque-subject".to_owned(),
            binding_id,
            1,
            event_author,
            None,
        ),
        ActiveLocalBinding::from_storage_parts(
            domain,
            "https://issuer.example".to_owned(),
            "s".repeat(2_049),
            binding_id,
            1,
            event_author,
            None,
        ),
        ActiveLocalBinding::from_storage_parts(
            domain,
            "https://issuer.example".to_owned(),
            "opaque-subject".to_owned(),
            Uuid::nil(),
            1,
            event_author,
            None,
        ),
        ActiveLocalBinding::from_storage_parts(
            domain,
            "https://issuer.example".to_owned(),
            "opaque-subject".to_owned(),
            binding_id,
            0,
            event_author,
            None,
        ),
    ] {
        assert!(invalid.is_none());
    }
}

#[test]
fn nip42_verifier_mints_exact_redacted_proof() {
    let keys = Keys::generate();
    let challenge = generate_challenge();
    let relay_url = RelayUrl::parse(TEST_RELAY).expect("valid relay URL");
    let event = EventBuilder::auth(&challenge, relay_url)
        .sign_with_keys(&keys)
        .expect("sign NIP-42 event");
    let domain = CommunityId::from_uuid(Uuid::from_u128(3));
    let expires_at = Utc::now() + Duration::minutes(1);

    let proof = verify_nip42_authorization_proof(
        &event,
        &challenge,
        TEST_RELAY,
        domain,
        [4; 32],
        [5; 32],
        [6; 32],
        Some([7; 32]),
        Some([8; 32]),
        expires_at,
    )
    .expect("verified NIP-42 event mints sealed proof");

    assert_eq!(proof.authorization_domain(), domain);
    assert_eq!(proof.actor_pubkey(), keys.public_key());
    assert_eq!(proof.transport(), ProofTransport::Nip42);
    assert_eq!(proof.request_fingerprint(), &[4; 32]);
    assert_eq!(proof.target_fingerprint(), &[5; 32]);
    assert_eq!(proof.transport_context_fingerprint(), &[6; 32]);
    assert_eq!(proof.bound_assertion_fingerprint(), Some(&[7; 32]));
    assert_eq!(proof.delegation_conditions_fingerprint(), Some(&[8; 32]));
    assert_eq!(proof.expires_at(), expires_at);
    assert_eq!(format!("{proof:?}"), "VerifiedNostrProof([REDACTED])");
}

#[test]
fn nip42_verifier_rejects_protocol_and_binding_mismatches() {
    let keys = Keys::generate();
    let challenge = generate_challenge();
    let relay_url = RelayUrl::parse(TEST_RELAY).expect("valid relay URL");
    let event = EventBuilder::auth(&challenge, relay_url)
        .sign_with_keys(&keys)
        .expect("sign NIP-42 event");
    let domain = CommunityId::from_uuid(Uuid::from_u128(3));
    let future = Utc::now() + Duration::minutes(1);
    let verify = |challenge: &str,
                  domain: CommunityId,
                  request: [u8; 32],
                  target: [u8; 32],
                  transport: [u8; 32],
                  assertion: Option<[u8; 32]>,
                  delegation: Option<[u8; 32]>,
                  expiry| {
        verify_nip42_authorization_proof(
            &event, challenge, TEST_RELAY, domain, request, target, transport, assertion,
            delegation, expiry,
        )
    };

    assert!(matches!(
        verify("wrong", domain, [4; 32], [5; 32], [6; 32], None, None, future),
        Err(Nip42AuthorizationProofError::Authentication(
            AuthError::ChallengeMismatch
        ))
    ));
    for invalid in [
        verify(
            &challenge,
            CommunityId::from_uuid(Uuid::nil()),
            [4; 32],
            [5; 32],
            [6; 32],
            None,
            None,
            future,
        ),
        verify(
            &challenge, domain, [0; 32], [5; 32], [6; 32], None, None, future,
        ),
        verify(
            &challenge, domain, [4; 32], [0; 32], [6; 32], None, None, future,
        ),
        verify(
            &challenge, domain, [4; 32], [5; 32], [0; 32], None, None, future,
        ),
        verify(
            &challenge,
            domain,
            [4; 32],
            [5; 32],
            [6; 32],
            Some([0; 32]),
            None,
            future,
        ),
        verify(
            &challenge,
            domain,
            [4; 32],
            [5; 32],
            [6; 32],
            None,
            Some([0; 32]),
            future,
        ),
        verify(
            &challenge,
            domain,
            [4; 32],
            [5; 32],
            [6; 32],
            None,
            None,
            Utc::now() - Duration::seconds(1),
        ),
    ] {
        assert!(matches!(
            invalid,
            Err(Nip42AuthorizationProofError::InvalidBinding)
        ));
    }
}
