use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::Duration,
};

use buzz_auth::{
    AssertionTransportProfile, HttpHeaderField, ProofTransport, TrustedProxyError,
    TrustedProxyNonceClaim, TrustedProxyNonceReplayReader, TrustedProxyProvenanceVerifier,
    TrustedProxyReplayReadError, TrustedProxyRequest, ASSERTION_HEADER_NAME,
    CLIENT_PEER_HEADER_NAME, PROVENANCE_HEADER_NAME,
};
use buzz_core::CommunityId;
use chrono::{TimeZone, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const NOW: u64 = 1_800_000_000;
const PRIMARY_SECRET: [u8; 32] = [0x53; 32];
const DOMAIN_A: CommunityId = CommunityId::from_uuid(Uuid::from_u128(1));
const DOMAIN_B: CommunityId = CommunityId::from_uuid(Uuid::from_u128(2));

type TestMac = Hmac<Sha256>;

fn proxy_assertion() -> &'static str {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE.get_or_init(|| {
        [
            ["eyJhbGciOiJF", "UzI1NiJ9"].concat(),
            ["eyJzdWIiOiJz", "dWJqZWN0In0"].concat(),
            ["c2lnbmF0", "dXJl"].concat(),
        ]
        .join(".")
    })
}

#[derive(Default)]
struct ReplayReader {
    committed: Mutex<HashSet<(CommunityId, [u8; 32])>>,
    unavailable: AtomicBool,
}

impl ReplayReader {
    fn commit(&self, authorization_domain: CommunityId, claim: &TrustedProxyNonceClaim) {
        self.committed
            .lock()
            .expect("replay test mutex must remain healthy")
            .insert((authorization_domain, *claim.claim_key()));
    }

    fn make_unavailable(&self) {
        self.unavailable.store(true, Ordering::SeqCst);
    }
}

impl TrustedProxyNonceReplayReader for ReplayReader {
    fn is_committed<'a>(
        &'a self,
        authorization_domain: CommunityId,
        claim: &'a TrustedProxyNonceClaim,
    ) -> Pin<Box<dyn Future<Output = Result<bool, TrustedProxyReplayReadError>> + Send + 'a>> {
        Box::pin(async move {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(TrustedProxyReplayReadError);
            }
            Ok(self
                .committed
                .lock()
                .map_err(|_| TrustedProxyReplayReadError)?
                .contains(&(authorization_domain, *claim.claim_key())))
        })
    }
}

fn verifier() -> TrustedProxyProvenanceVerifier {
    TrustedProxyProvenanceVerifier::new(
        vec![PRIMARY_SECRET.to_vec(), vec![0x71; 32]],
        Duration::from_secs(60),
        Duration::from_secs(5),
    )
    .expect("valid verifier policy")
}

struct ProvenanceFreshness<'a> {
    timestamp: u64,
    nonce: &'a [u8],
}

fn sign_provenance(
    assertion: &str,
    method: &str,
    authority: &str,
    path_and_query: &str,
    body: &[u8],
    timestamp: u64,
    nonce: &[u8],
) -> String {
    sign_provenance_in_domain(
        DOMAIN_A,
        assertion,
        method,
        authority,
        path_and_query,
        body,
        ProvenanceFreshness { timestamp, nonce },
    )
}

fn sign_provenance_in_domain(
    authorization_domain: CommunityId,
    assertion: &str,
    method: &str,
    authority: &str,
    path_and_query: &str,
    body: &[u8],
    freshness: ProvenanceFreshness<'_>,
) -> String {
    let ProvenanceFreshness { timestamp, nonce } = freshness;
    let canonical_path = if path_and_query.is_empty() {
        "/".to_owned()
    } else if path_and_query.starts_with('?') {
        format!("/{path_and_query}")
    } else {
        path_and_query.to_owned()
    };
    let assertion_digest: [u8; 32] = Sha256::digest(assertion.as_bytes()).into();
    let body_digest: [u8; 32] = Sha256::digest(body).into();
    let transport = [2_u8]; // ProofTransport::Nip98, frozen wire code.
    let mut input = b"NIP-FI-PROXY-1".to_vec();
    for value in [
        timestamp.to_be_bytes().as_slice(),
        nonce,
        &assertion_digest,
        authorization_domain.as_uuid().as_bytes(),
        method.as_bytes(),
        authority.as_bytes(),
        canonical_path.as_bytes(),
        &body_digest,
        &transport,
    ] {
        input.extend_from_slice(&(value.len() as u64).to_be_bytes());
        input.extend_from_slice(value);
    }
    let mut mac = <TestMac as KeyInit>::new_from_slice(&PRIMARY_SECRET)
        .expect("HMAC-SHA-256 accepts the test key");
    mac.update(&input);
    format!(
        "v1.{timestamp}.{}.{}",
        base64url(nonce),
        base64url(&mac.finalize().into_bytes())
    )
}

struct PeerProvenance<'a> {
    timestamp: u64,
    nonce: &'a [u8],
    client_peer: &'a str,
}

fn sign_peer_provenance(
    assertion: &str,
    method: &str,
    authority: &str,
    path_and_query: &str,
    body: &[u8],
    provenance: PeerProvenance<'_>,
) -> String {
    let PeerProvenance {
        timestamp,
        nonce,
        client_peer,
    } = provenance;
    let canonical_path = if path_and_query.is_empty() {
        "/".to_owned()
    } else if path_and_query.starts_with('?') {
        format!("/{path_and_query}")
    } else {
        path_and_query.to_owned()
    };
    let assertion_digest: [u8; 32] = Sha256::digest(assertion.as_bytes()).into();
    let body_digest: [u8; 32] = Sha256::digest(body).into();
    let transport = [2_u8]; // ProofTransport::Nip98, frozen wire code.
    let mut input = b"NIP-FI-PROXY-2".to_vec();
    for value in [
        timestamp.to_be_bytes().as_slice(),
        nonce,
        &assertion_digest,
        DOMAIN_A.as_uuid().as_bytes(),
        method.as_bytes(),
        authority.as_bytes(),
        canonical_path.as_bytes(),
        &body_digest,
        &transport,
        client_peer.as_bytes(),
    ] {
        input.extend_from_slice(&(value.len() as u64).to_be_bytes());
        input.extend_from_slice(value);
    }
    let mut mac = <TestMac as KeyInit>::new_from_slice(&PRIMARY_SECRET)
        .expect("HMAC-SHA-256 accepts the test key");
    mac.update(&input);
    format!(
        "v2.{timestamp}.{}.{}",
        base64url(nonce),
        base64url(&mac.finalize().into_bytes())
    )
}

fn base64url(value: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::new();
    for chunk in value.chunks(3) {
        let first = chunk[0];
        output.push(ALPHABET[(first >> 2) as usize] as char);
        let second_high = chunk.get(1).copied().unwrap_or(0);
        output.push(ALPHABET[(((first & 0x03) << 4) | (second_high >> 4)) as usize] as char);
        if let Some(second) = chunk.get(1).copied() {
            let third_high = chunk.get(2).copied().unwrap_or(0);
            output.push(ALPHABET[(((second & 0x0f) << 2) | (third_high >> 6)) as usize] as char);
        }
        if let Some(third) = chunk.get(2).copied() {
            output.push(ALPHABET[(third & 0x3f) as usize] as char);
        }
    }
    output
}

fn request(method: &str, authority: &str, path: &str, body: &[u8]) -> TrustedProxyRequest {
    request_in_domain(DOMAIN_A, method, authority, path, body)
}

fn request_in_domain(
    authorization_domain: CommunityId,
    method: &str,
    authority: &str,
    path: &str,
    body: &[u8],
) -> TrustedProxyRequest {
    TrustedProxyRequest::from_server_request(
        authorization_domain,
        ProofTransport::Nip98,
        method,
        authority,
        path,
        body,
    )
    .expect("valid server request")
}

fn now() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(NOW as i64, 0)
        .single()
        .expect("valid test time")
}

fn assertion_header(assertion: &str) -> String {
    format!("Bearer {assertion}")
}

fn headers<'a>(assertion: &'a str, provenance: &'a str) -> Vec<HttpHeaderField<'a>> {
    vec![
        HttpHeaderField::new(ASSERTION_HEADER_NAME, assertion.as_bytes()),
        HttpHeaderField::new(PROVENANCE_HEADER_NAME, provenance.as_bytes()),
    ]
}

fn peer_headers<'a>(
    assertion: &'a str,
    provenance: &'a str,
    client_peer: &'a str,
) -> Vec<HttpHeaderField<'a>> {
    let mut headers = headers(assertion, provenance);
    headers.push(HttpHeaderField::new(
        CLIENT_PEER_HEADER_NAME,
        client_peer.as_bytes(),
    ));
    headers
}

#[tokio::test]
async fn valid_provenance_seals_redacted_move_only_evidence() {
    let verifier = verifier();
    let request = request("POST", "relay.example.com:443", "/events?a=1&a=2", b"body");
    let provenance = sign_provenance(
        proxy_assertion(),
        "POST",
        "relay.example.com:443",
        "/events?a=1&a=2",
        b"body",
        NOW,
        &[0x19; 16],
    );
    let assertion = assertion_header(proxy_assertion());
    let replay = ReplayReader::default();

    let evidence = verifier
        .verify(&headers(&assertion, &provenance), &request, now(), &replay)
        .await
        .expect("valid provenance");

    assert_eq!(evidence.confidential_assertion(), proxy_assertion());
    assert_eq!(evidence.authorization_domain(), DOMAIN_A);
    assert_eq!(
        evidence.profile(),
        AssertionTransportProfile::TrustedProxyHmacV1
    );
    assert_eq!(evidence.transport(), ProofTransport::Nip98);
    assert!(evidence.authenticated_client_peer().is_none());
    assert_eq!(evidence.proxy_expires_at().timestamp(), (NOW + 60) as i64);
    assert_eq!(
        evidence.nonce_claim().retain_until().timestamp(),
        (NOW + 60) as i64
    );
    assert_ne!(evidence.assertion_digest(), &[0; 32]);
    assert_ne!(evidence.request_fingerprint(), &[0; 32]);
    assert_ne!(evidence.transport_context_fingerprint(), &[0; 32]);
    assert_eq!(
        format!("{evidence:?}"),
        "SealedTransportEvidence([REDACTED])"
    );
    assert_eq!(
        format!("{:?}", evidence.nonce_claim()),
        "TrustedProxyNonceClaim([REDACTED])"
    );
}

#[tokio::test]
async fn v2_provenance_preserves_clients_behind_proxy_fan_in() {
    let verifier = verifier();
    let request = request("GET", "relay.example.com:443", "/", b"");
    let assertion = assertion_header(proxy_assertion());
    let replay = ReplayReader::default();
    let peer_a = "192.0.2.10";
    let peer_b = "192.0.2.11";
    let provenance_a = sign_peer_provenance(
        proxy_assertion(),
        "GET",
        "relay.example.com:443",
        "/",
        b"",
        PeerProvenance {
            timestamp: NOW,
            nonce: &[0x1a; 16],
            client_peer: peer_a,
        },
    );
    let provenance_b = sign_peer_provenance(
        proxy_assertion(),
        "GET",
        "relay.example.com:443",
        "/",
        b"",
        PeerProvenance {
            timestamp: NOW,
            nonce: &[0x1b; 16],
            client_peer: peer_b,
        },
    );
    let provenance_a_again = sign_peer_provenance(
        proxy_assertion(),
        "GET",
        "relay.example.com:443",
        "/",
        b"",
        PeerProvenance {
            timestamp: NOW,
            nonce: &[0x1d; 16],
            client_peer: peer_a,
        },
    );

    let evidence_a = verifier
        .verify(
            &peer_headers(&assertion, &provenance_a, peer_a),
            &request,
            now(),
            &replay,
        )
        .await
        .expect("authenticated peer A");
    let evidence_b = verifier
        .verify(
            &peer_headers(&assertion, &provenance_b, peer_b),
            &request,
            now(),
            &replay,
        )
        .await
        .expect("authenticated peer B");
    let evidence_a_again = verifier
        .verify(
            &peer_headers(&assertion, &provenance_a_again, peer_a),
            &request,
            now(),
            &replay,
        )
        .await
        .expect("authenticated peer A with a new nonce");

    assert_eq!(
        evidence_a.profile(),
        AssertionTransportProfile::TrustedProxyHmacV2
    );
    let authenticated_a = evidence_a.authenticated_client_peer().expect("v2 peer key");
    let authenticated_b = evidence_b.authenticated_client_peer().expect("v2 peer key");
    let authenticated_a_again = evidence_a_again
        .authenticated_client_peer()
        .expect("stable v2 peer key");
    assert_eq!(
        authenticated_a.admission_key(),
        authenticated_a_again.admission_key()
    );
    assert_ne!(
        authenticated_a.admission_key(),
        authenticated_b.admission_key()
    );
    assert_eq!(
        format!("{authenticated_a:?}"),
        "AuthenticatedClientPeer([REDACTED])"
    );
    assert!(!format!("{evidence_a:?}").contains(peer_a));
}

#[tokio::test]
async fn v2_peer_is_required_canonical_and_mac_bound() {
    let verifier = verifier();
    let request = request("GET", "relay.example.com:443", "/", b"");
    let assertion = assertion_header(proxy_assertion());
    let replay = ReplayReader::default();
    let peer = "198.51.100.20";
    let provenance = sign_peer_provenance(
        proxy_assertion(),
        "GET",
        "relay.example.com:443",
        "/",
        b"",
        PeerProvenance {
            timestamp: NOW,
            nonce: &[0x1c; 16],
            client_peer: peer,
        },
    );

    assert_eq!(
        verifier
            .verify(&headers(&assertion, &provenance), &request, now(), &replay)
            .await
            .unwrap_err(),
        TrustedProxyError::MissingClientPeer
    );
    assert_eq!(
        verifier
            .verify(
                &peer_headers(&assertion, &provenance, "198.51.100.21"),
                &request,
                now(),
                &replay,
            )
            .await
            .unwrap_err(),
        TrustedProxyError::InvalidMac
    );
    assert_eq!(
        verifier
            .verify(
                &peer_headers(&assertion, &provenance, "::ffff:198.51.100.20"),
                &request,
                now(),
                &replay,
            )
            .await
            .unwrap_err(),
        TrustedProxyError::MalformedClientPeer
    );

    let mut duplicate = peer_headers(&assertion, &provenance, peer);
    duplicate.push(HttpHeaderField::new(
        "Nostr-Federated-Identity-Client-Peer",
        peer.as_bytes(),
    ));
    assert_eq!(
        verifier
            .verify(&duplicate, &request, now(), &replay)
            .await
            .unwrap_err(),
        TrustedProxyError::AmbiguousHeader
    );
}

#[tokio::test]
async fn provenance_binds_authorization_domain_and_transport_context() {
    let verifier = verifier();
    let request_a = request("POST", "relay.example.com:443", "/events", b"body");
    let request_b = request_in_domain(
        DOMAIN_B,
        "POST",
        "relay.example.com:443",
        "/events",
        b"body",
    );
    let provenance_a = sign_provenance(
        proxy_assertion(),
        "POST",
        "relay.example.com:443",
        "/events",
        b"body",
        NOW,
        &[0x29; 16],
    );
    let assertion = assertion_header(proxy_assertion());
    let replay = ReplayReader::default();
    let evidence_a = verifier
        .verify(
            &headers(&assertion, &provenance_a),
            &request_a,
            now(),
            &replay,
        )
        .await
        .expect("domain A provenance");

    let unavailable = ReplayReader::default();
    unavailable.make_unavailable();
    assert_eq!(
        verifier
            .verify(
                &headers(&assertion, &provenance_a),
                &request_b,
                now(),
                &unavailable,
            )
            .await
            .unwrap_err(),
        TrustedProxyError::InvalidMac
    );

    let provenance_b = sign_provenance_in_domain(
        DOMAIN_B,
        proxy_assertion(),
        "POST",
        "relay.example.com:443",
        "/events",
        b"body",
        ProvenanceFreshness {
            timestamp: NOW,
            nonce: &[0x29; 16],
        },
    );
    let evidence_b = verifier
        .verify(
            &headers(&assertion, &provenance_b),
            &request_b,
            now(),
            &replay,
        )
        .await
        .expect("domain B provenance");
    assert_ne!(
        evidence_a.transport_context_fingerprint(),
        evidence_b.transport_context_fingerprint()
    );
}

#[tokio::test]
async fn direct_origin_and_injected_headers_fail_closed() {
    let verifier = verifier();
    let request = request("GET", "relay.example.com:443", "/media/hash", b"");
    let assertion = assertion_header(proxy_assertion());
    let replay = ReplayReader::default();

    let direct = [HttpHeaderField::new(
        ASSERTION_HEADER_NAME,
        assertion.as_bytes(),
    )];
    assert_eq!(
        verifier
            .verify(&direct, &request, now(), &replay)
            .await
            .unwrap_err(),
        TrustedProxyError::MissingProvenance
    );

    let provenance = sign_provenance(
        proxy_assertion(),
        "GET",
        "relay.example.com:443",
        "/media/hash",
        b"",
        NOW,
        &[0x2a; 16],
    );
    let injected_assertion = assertion_header("e30.e30.aW5qZWN0ZWQ");
    assert_eq!(
        verifier
            .verify(
                &headers(&injected_assertion, &provenance),
                &request,
                now(),
                &replay,
            )
            .await
            .unwrap_err(),
        TrustedProxyError::InvalidMac
    );
}

#[tokio::test]
async fn duplicate_combined_and_whitespace_fields_are_rejected() {
    let verifier = verifier();
    let request = request("GET", "relay.example.com:443", "/", b"");
    let assertion = assertion_header(proxy_assertion());
    let provenance = sign_provenance(
        proxy_assertion(),
        "GET",
        "relay.example.com:443",
        "/",
        b"",
        NOW,
        &[0x3b; 16],
    );
    let replay = ReplayReader::default();

    let duplicate = [
        HttpHeaderField::new(ASSERTION_HEADER_NAME, assertion.as_bytes()),
        HttpHeaderField::new("Nostr-Federated-Identity", assertion.as_bytes()),
        HttpHeaderField::new(PROVENANCE_HEADER_NAME, provenance.as_bytes()),
    ];
    assert_eq!(
        verifier
            .verify(&duplicate, &request, now(), &replay)
            .await
            .unwrap_err(),
        TrustedProxyError::AmbiguousHeader
    );

    for malformed in [
        format!("{provenance},other"),
        format!(" {provenance}"),
        format!("{provenance} "),
    ] {
        assert_eq!(
            verifier
                .verify(&headers(&assertion, &malformed), &request, now(), &replay)
                .await
                .unwrap_err(),
            TrustedProxyError::AmbiguousHeader
        );
    }
}

#[tokio::test]
async fn provenance_cannot_cross_assertion_method_authority_path_query_or_body() {
    let verifier = verifier();
    let assertion = assertion_header(proxy_assertion());
    let provenance = sign_provenance(
        proxy_assertion(),
        "POST",
        "relay.example.com:443",
        "/query?a=1&a=2",
        b"body-one",
        NOW,
        &[0x4c; 16],
    );
    let replay = ReplayReader::default();

    let changed = [
        request(
            "PUT",
            "relay.example.com:443",
            "/query?a=1&a=2",
            b"body-one",
        ),
        request(
            "POST",
            "other.example.com:443",
            "/query?a=1&a=2",
            b"body-one",
        ),
        request(
            "POST",
            "relay.example.com:443",
            "/query?a=2&a=1",
            b"body-one",
        ),
        request(
            "POST",
            "relay.example.com:443",
            "/query?a=1&a=2",
            b"body-two",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Blossom,
            "POST",
            "relay.example.com:443",
            "/query?a=1&a=2",
            b"body-one",
        )
        .expect("valid alternate server transport"),
    ];
    for changed_request in changed {
        assert_eq!(
            verifier
                .verify(
                    &headers(&assertion, &provenance),
                    &changed_request,
                    now(),
                    &replay,
                )
                .await
                .unwrap_err(),
            TrustedProxyError::InvalidMac
        );
    }
}

#[tokio::test]
async fn timestamp_boundaries_and_canonical_encoding_are_exact() {
    let verifier = verifier();
    let request = request("GET", "[2001:db8::1]:443", "?x=%2F", b"");
    let assertion = assertion_header(proxy_assertion());
    let replay = ReplayReader::default();

    let fresh = sign_provenance(
        proxy_assertion(),
        "GET",
        "[2001:db8::1]:443",
        "?x=%2F",
        b"",
        NOW - 59,
        &[0x5d; 16],
    );
    verifier
        .verify(&headers(&assertion, &fresh), &request, now(), &replay)
        .await
        .expect("one second before exclusive age bound");

    let expired = sign_provenance(
        proxy_assertion(),
        "GET",
        "[2001:db8::1]:443",
        "?x=%2F",
        b"",
        NOW - 60,
        &[0x5e; 16],
    );
    assert_eq!(
        verifier
            .verify(&headers(&assertion, &expired), &request, now(), &replay)
            .await
            .unwrap_err(),
        TrustedProxyError::Expired
    );

    let future = sign_provenance(
        proxy_assertion(),
        "GET",
        "[2001:db8::1]:443",
        "?x=%2F",
        b"",
        NOW + 6,
        &[0x5f; 16],
    );
    assert_eq!(
        verifier
            .verify(&headers(&assertion, &future), &request, now(), &replay)
            .await
            .unwrap_err(),
        TrustedProxyError::FutureDated
    );

    let canonical = sign_provenance(
        proxy_assertion(),
        "GET",
        "[2001:db8::1]:443",
        "?x=%2F",
        b"",
        NOW,
        &[0x60; 16],
    );
    let leading_zero = canonical.replacen(&format!("v1.{NOW}."), &format!("v1.0{NOW}."), 1);
    assert_eq!(
        verifier
            .verify(
                &headers(&assertion, &leading_zero),
                &request,
                now(),
                &replay
            )
            .await
            .unwrap_err(),
        TrustedProxyError::MalformedProvenance
    );
    let padded = format!("{canonical}=");
    assert_eq!(
        verifier
            .verify(&headers(&assertion, &padded), &request, now(), &replay)
            .await
            .unwrap_err(),
        TrustedProxyError::MalformedProvenance
    );
}

#[tokio::test]
async fn committed_nonce_and_unavailable_replay_state_fail_closed() {
    let verifier = verifier();
    let git_request = request(
        "GET",
        "relay.example.com:443",
        "/git/info/refs?service=git-upload-pack",
        b"",
    );
    let assertion = assertion_header(proxy_assertion());
    let provenance = sign_provenance(
        proxy_assertion(),
        "GET",
        "relay.example.com:443",
        "/git/info/refs?service=git-upload-pack",
        b"",
        NOW,
        &[0x6a; 16],
    );
    let replay = ReplayReader::default();

    let evidence = verifier
        .verify(
            &headers(&assertion, &provenance),
            &git_request,
            now(),
            &replay,
        )
        .await
        .expect("uncommitted nonce");
    replay.commit(evidence.authorization_domain(), evidence.nonce_claim());
    assert_eq!(
        verifier
            .verify(
                &headers(&assertion, &provenance),
                &git_request,
                now(),
                &replay,
            )
            .await
            .unwrap_err(),
        TrustedProxyError::Replay
    );

    let other_authority_request = request("GET", "other.example.com:443", "/same-nonce", b"");
    let same_nonce_other_domain = sign_provenance_in_domain(
        DOMAIN_B,
        proxy_assertion(),
        "GET",
        "other.example.com:443",
        "/same-nonce",
        b"",
        ProvenanceFreshness {
            timestamp: NOW,
            nonce: &[0x6a; 16],
        },
    );
    let other_domain_request =
        request_in_domain(DOMAIN_B, "GET", "other.example.com:443", "/same-nonce", b"");
    verifier
        .verify(
            &headers(&assertion, &same_nonce_other_domain),
            &other_domain_request,
            now(),
            &replay,
        )
        .await
        .expect("the frozen replay key is isolated by authorization domain");
    let same_nonce_other_authority = sign_provenance(
        proxy_assertion(),
        "GET",
        "other.example.com:443",
        "/same-nonce",
        b"",
        NOW,
        &[0x6a; 16],
    );
    assert_eq!(
        verifier
            .verify(
                &headers(&assertion, &same_nonce_other_authority),
                &other_authority_request,
                now(),
                &replay,
            )
            .await
            .unwrap_err(),
        TrustedProxyError::Replay
    );

    let unavailable = ReplayReader::default();
    unavailable.make_unavailable();
    let other = sign_provenance(
        proxy_assertion(),
        "GET",
        "relay.example.com:443",
        "/git/info/refs?service=git-upload-pack",
        b"",
        NOW,
        &[0x6b; 16],
    );
    assert_eq!(
        verifier
            .verify(
                &headers(&assertion, &other),
                &git_request,
                now(),
                &unavailable,
            )
            .await
            .unwrap_err(),
        TrustedProxyError::ReplayUnavailable
    );
}

#[test]
fn request_and_verifier_configuration_reject_ambiguous_inputs() {
    for invalid in [
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "get",
            "relay.example.com:443",
            "/",
            b"",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "GET",
            "Relay.example.com:443",
            "/",
            b"",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "GET",
            "relay.example.com",
            "/",
            b"",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "GET",
            "relay.example.com:0443",
            "/",
            b"",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "GET",
            "127.000.0.1:443",
            "/",
            b"",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "GET",
            "[2001:0db8::1]:443",
            "/",
            b"",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "GET",
            "relay.example.com:443",
            "relative",
            b"",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "GET",
            "relay.example.com:443",
            "/path#fragment",
            b"",
        ),
        TrustedProxyRequest::from_server_request(
            DOMAIN_A,
            ProofTransport::Nip98,
            "GET",
            "relay.example.com:443",
            "/bad%2",
            b"",
        ),
    ] {
        assert_eq!(invalid.unwrap_err(), TrustedProxyError::InvalidRequest);
    }

    assert_eq!(
        TrustedProxyProvenanceVerifier::new(
            vec![vec![0x01; 31]],
            Duration::from_secs(60),
            Duration::ZERO,
        )
        .unwrap_err(),
        TrustedProxyError::InvalidConfiguration
    );
    assert_eq!(
        TrustedProxyProvenanceVerifier::new(vec![vec![0x01; 32]], Duration::ZERO, Duration::ZERO,)
            .unwrap_err(),
        TrustedProxyError::InvalidConfiguration
    );
    assert_eq!(
        TrustedProxyError::ReplayUnavailable.code(),
        "nip_fi_proxy_replay_unavailable"
    );
}
