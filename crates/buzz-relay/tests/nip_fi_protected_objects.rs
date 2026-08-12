//! Cross-crate security regressions for NIP-FI protected objects.
//!
//! The PostgreSQL and object-store cases are ignored by default so ordinary
//! unit runs remain hermetic. The S4 release gate runs them explicitly against
//! disposable services.

use std::{
    future::Future,
    io::Write,
    path::PathBuf,
    pin::Pin,
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
    time::Duration,
};

use buzz_auth::{
    AuthoritativeAuthorizationRecheck, CanonicalFederatedAssertionVerifier,
    CanonicalVerifierKeySet, CanonicalVerifierPolicy, HttpHeaderField, LocalAuthorizationPolicy,
    PreparedAuthorizationRecheck, ProofTransport, RouteCapability, TrustedProxyNonceClaim,
    TrustedProxyNonceReplayReader, TrustedProxyProvenanceVerifier, TrustedProxyReplayReadError,
    TrustedProxyRequest, VerifiedFederatedAssertion, VerifiedNostrProof, VerifierKeyGeneration,
    ASSERTION_HEADER_NAME, PROVENANCE_HEADER_NAME,
};
use buzz_core::{AuthorizationLeaseFence, CommunityId, TenantContext};
use buzz_db::authorization_admission::{
    AdmissionApplicationEffect, AdmissionApplicationResult, AdmissionCommitDigests,
    AdmissionCommitError, AdmissionCommitOutcome, AdmissionCommitReceipt, AdmissionCommitRequest,
    AdmissionEnrollmentEvidence, AdmissionFinalRechecker, AdmissionObject,
    CanonicalAdmissionCommitter, PostgresCanonicalAdmissionCommitter,
};
use buzz_media::upload_record::UploadEventFacts;
use buzz_media::{
    stage_upload, stage_upload_record, MediaConfig, MediaStorage, S3AddressingStyle,
    UploadAttribution, UploadNetworkInfo,
};
use buzz_relay::api::media::{
    ProtectedPublicationBinding, ProtectedPublicationPlan, ProtectedPublicationReceipt,
    ProtectedPublicationTarget,
};
use buzz_relay::handlers::moderation_commands::prepare_moderation_application_effect;
use bytes::Bytes;
use chrono::{TimeDelta, TimeZone, Utc};
use hmac::{Hmac, KeyInit, Mac};
use nostr::{EventBuilder, Keys, Kind, RelayUrl, Tag};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

const PROXY_NOW: u64 = 1_800_000_000;
const PROXY_SECRET: [u8; 32] = [0x53; 32];

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
struct EmptyReplayReader;

impl TrustedProxyNonceReplayReader for EmptyReplayReader {
    fn is_committed<'a>(
        &'a self,
        _authorization_domain: CommunityId,
        _claim: &'a TrustedProxyNonceClaim,
    ) -> Pin<Box<dyn Future<Output = Result<bool, TrustedProxyReplayReadError>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}

struct ApplicationReplayRechecker;

impl AdmissionFinalRechecker for ApplicationReplayRechecker {
    fn authoritative_recheck<'a, 'transaction>(
        &'a self,
        transaction: &'a mut Transaction<'transaction, Postgres>,
        request: &'a PreparedAuthorizationRecheck,
        _object: AdmissionObject,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<AuthoritativeAuthorizationRecheck, AdmissionCommitError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            // This harness isolates canonical application/outbox replay. The
            // production rechecker performs the full dependency reads; here we
            // still use authoritative transaction time and the exact sealed
            // snapshot so the real committer/finalizer executes unchanged.
            let authoritative_now = sqlx::query_scalar("SELECT transaction_timestamp()")
                .fetch_one(&mut **transaction)
                .await
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            Ok(AuthoritativeAuthorizationRecheck::from_authoritative_parts(
                request.lease_dependencies(),
                request.verifier_stamp(),
                authoritative_now,
            ))
        })
    }
}

fn base64_url(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(ALPHABET[(first >> 2) as usize] as char);
        output.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[(third & 0x3f) as usize] as char);
        }
    }
    output
}

struct EphemeralRsaKey {
    path: PathBuf,
}

impl EphemeralRsaKey {
    fn generate(kid: &str, prefix: &str) -> (Self, serde_json::Value) {
        let key = Self {
            path: std::env::temp_dir().join(format!("{prefix}-{}.pem", Uuid::new_v4())),
        };
        let generated = Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
                "-pkeyopt",
                "rsa_keygen_pubexp:65537",
                "-out",
            ])
            .arg(&key.path)
            .output()
            .expect("generate ephemeral RSA test key");
        assert!(
            generated.status.success(),
            "OpenSSL RSA key generation failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let modulus = Command::new("openssl")
            .args(["rsa", "-in"])
            .arg(&key.path)
            .args(["-noout", "-modulus"])
            .output()
            .expect("read ephemeral RSA test modulus");
        assert!(
            modulus.status.success(),
            "OpenSSL RSA modulus extraction failed: {}",
            String::from_utf8_lossy(&modulus.stderr)
        );
        let modulus = std::str::from_utf8(&modulus.stdout)
            .expect("UTF-8 RSA test modulus")
            .trim()
            .strip_prefix("Modulus=")
            .expect("OpenSSL RSA modulus prefix");
        let modulus = hex::decode(modulus).expect("hex RSA test modulus");
        let jwks = serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": kid,
                "n": base64_url(&modulus),
                "e": "AQAB",
            }],
        });
        (key, jwks)
    }
}

impl Drop for EphemeralRsaKey {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn signed_test_jwt(subject: &str, event_author: [u8; 32]) -> (String, serde_json::Value) {
    let issued_at = Utc::now().timestamp() - 1;
    let claims = serde_json::json!({
        "iss": "https://s4-verifier.test",
        "aud": "buzz-s4-test",
        "sub": subject,
        "event_author": hex::encode(event_author),
        "iat": issued_at,
        "nbf": issued_at,
        "exp": issued_at + 600,
    });
    let header = serde_json::json!({ "alg": "RS256", "kid": "s4-test", "typ": "JWT" });
    let signing_input = format!(
        "{}.{}",
        base64_url(&serde_json::to_vec(&header).expect("serialize JWT header")),
        base64_url(&serde_json::to_vec(&claims).expect("serialize JWT claims")),
    );
    let (key, jwks) = EphemeralRsaKey::generate("s4-test", "buzz-s4-jwt");
    let mut child = Command::new("openssl")
        .args(["dgst", "-sha256", "-sign"])
        .arg(&key.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn OpenSSL test signer");
    child
        .stdin
        .as_mut()
        .expect("OpenSSL signer stdin")
        .write_all(signing_input.as_bytes())
        .expect("write JWT signing input");
    let signed = child.wait_with_output().expect("wait for JWT signer");
    assert!(signed.status.success(), "OpenSSL JWT signer failed");
    (
        format!("{signing_input}.{}", base64_url(&signed.stdout)),
        jwks,
    )
}

fn verified_authorization_evidence(
    domain: CommunityId,
    keys: &Keys,
    target: [u8; 32],
    request: [u8; 32],
    transport_context: [u8; 32],
) -> (VerifiedFederatedAssertion, VerifiedNostrProof) {
    let (token, jwks) = signed_test_jwt(
        "canonical-application-subject",
        keys.public_key().to_bytes(),
    );
    let verifier = CanonicalFederatedAssertionVerifier::new(
        CanonicalVerifierPolicy::new(
            "https://s4-verifier.test".to_owned(),
            "buzz-s4-test".to_owned(),
            "sub".to_owned(),
            Some("event_author".to_owned()),
            5,
            600,
        )
        .expect("canonical verifier policy"),
    );
    let key_set = CanonicalVerifierKeySet::new(
        VerifierKeyGeneration::new(1).expect("positive verifier generation"),
        serde_json::from_value(jwks).expect("parse generated test JWKS"),
    );
    let assertion = verifier
        .verify(
            &token,
            &key_set,
            domain,
            ProofTransport::Nip42,
            target,
            request,
            transport_context,
        )
        .expect("verify test assertion");
    let challenge = hex::encode([81_u8; 32]);
    let relay = "wss://s4-admission.test";
    let event = EventBuilder::auth(
        challenge.clone(),
        RelayUrl::parse(relay).expect("test relay URL"),
    )
    .sign_with_keys(keys)
    .expect("sign NIP-42 test proof");
    let assertion_fingerprint: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let proof = buzz_auth::verify_nip42_authorization_proof(
        &event,
        &challenge,
        relay,
        domain,
        request,
        target,
        transport_context,
        Some(assertion_fingerprint),
        None,
        Utc::now() + TimeDelta::minutes(5),
    )
    .expect("verify NIP-42 authorization proof");
    (assertion, proof)
}

async fn enrollment_request_with_effect(
    pool: &PgPool,
    domain: CommunityId,
    authorization_evidence: (VerifiedFederatedAssertion, VerifiedNostrProof),
    object: AdmissionObject,
    capability: RouteCapability,
    authoritative_now: chrono::DateTime<Utc>,
    application_effect: Box<dyn AdmissionApplicationEffect>,
) -> AdmissionCommitRequest {
    let (assertion, proof) = authorization_evidence;
    let prepared = buzz_db::identity_enrollment::prepare_direct_enrollment(
        pool,
        assertion.clone(),
        proof.clone(),
        authoritative_now,
    )
    .await
    .expect("prepare direct application enrollment");
    let policy = LocalAuthorizationPolicy::from_database(
        domain,
        Uuid::from_u128(411),
        1,
        0,
        1,
        AuthorizationLeaseFence::from_bytes([82; 32]).expect("authorization fence"),
        capability,
        authoritative_now + TimeDelta::minutes(5),
        None,
        None,
    )
    .expect("media authorization policy");
    let evidence = AdmissionEnrollmentEvidence::new(prepared, assertion, proof, policy)
        .expect("seal media enrollment evidence");
    AdmissionCommitRequest::enrollment(
        Uuid::new_v4(),
        Uuid::from_u128(412),
        object,
        capability,
        evidence,
    )
    .expect("construct application enrollment request")
    .with_application_effect(application_effect)
    .expect("attach canonical application effect")
}

#[test]
fn receipt_for_one_repository_cannot_substitute_for_another_target() {
    let domain = CommunityId::from_uuid(Uuid::from_u128(1));
    let owner = "a".repeat(64);
    let target_a =
        ProtectedPublicationTarget::repository(domain, &owner, "repo-a").expect("target A");
    let target_b =
        ProtectedPublicationTarget::repository(domain, &owner, "repo-b").expect("target B");
    let artifact_digest = [7_u8; 32];
    let application_result = target_a
        .application_result(artifact_digest)
        .expect("typed publication result");
    let semantic_fingerprint = [9; 32];
    let application_intent_digest = [12; 32];
    let application_result_digest = canonical_application_result_digest(
        target_a,
        semantic_fingerprint,
        application_intent_digest,
        &application_result,
    );
    let receipt = AdmissionCommitReceipt::from_storage(
        domain,
        target_a.admission_object(),
        Uuid::from_u128(10),
        [8; 32],
        semantic_fingerprint,
        AdmissionCommitDigests::new([10; 32], Some(application_result_digest))
            .expect("receipt digests"),
        Uuid::from_u128(11),
    )
    .expect("storage receipt");

    assert!(target_a
        .validate_receipt(
            artifact_digest,
            application_intent_digest,
            receipt,
            &application_result,
        )
        .is_ok());
    assert_eq!(
        target_b.validate_receipt(
            artifact_digest,
            application_intent_digest,
            receipt,
            &application_result,
        ),
        Err(AdmissionCommitError::IntentConflict)
    );
}

fn canonical_application_result_digest(
    target: ProtectedPublicationTarget,
    semantic_fingerprint: [u8; 32],
    application_intent_digest: [u8; 32],
    result: &AdmissionApplicationResult,
) -> [u8; 32] {
    let schema = result.schema();
    let domain = b"buzz:canonical-application-result:v1";
    let authorization_domain = target.authorization_domain();
    let object = target.admission_object();
    let object_kind = object.kind().database_code().to_be_bytes();
    let schema_version = schema.version().to_be_bytes();
    let result_code = result.code().to_be_bytes();
    let fields: [&[u8]; 9] = [
        authorization_domain.as_uuid().as_bytes(),
        &object_kind,
        object.key(),
        &semantic_fingerprint,
        &application_intent_digest,
        schema.type_key(),
        &schema_version,
        &result_code,
        result.payload(),
    ];
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

#[tokio::test]
async fn same_domain_evidence_from_another_request_has_a_distinct_exact_binding() {
    let domain = CommunityId::from_uuid(Uuid::from_u128(2));
    let verifier = TrustedProxyProvenanceVerifier::new(
        vec![PROXY_SECRET.to_vec()],
        Duration::from_secs(60),
        Duration::from_secs(5),
    )
    .expect("proxy verifier");
    let request_a = TrustedProxyRequest::from_server_request(
        domain,
        ProofTransport::Nip98,
        "POST",
        "relay.example.com:443",
        "/upload?slot=a",
        b"body-a",
    )
    .expect("request A");
    let request_b = TrustedProxyRequest::from_server_request(
        domain,
        ProofTransport::Blossom,
        "POST",
        "relay.example.com:443",
        "/upload?slot=b",
        b"body-b",
    )
    .expect("request B");
    let evidence_a = verify_proxy_request(
        &verifier,
        &request_a,
        ProofTransport::Nip98,
        "/upload?slot=a",
        b"body-a",
        &[1_u8; 16],
    )
    .await;
    let evidence_b = verify_proxy_request(
        &verifier,
        &request_b,
        ProofTransport::Blossom,
        "/upload?slot=b",
        b"body-b",
        &[2_u8; 16],
    )
    .await;

    assert_eq!(evidence_a.authorization_domain(), domain);
    assert_eq!(evidence_b.authorization_domain(), domain);
    assert_ne!(
        evidence_a.request_fingerprint(),
        evidence_b.request_fingerprint(),
        "same-domain evidence must retain exact request binding"
    );
    assert_ne!(
        evidence_a.transport_context_fingerprint(),
        evidence_b.transport_context_fingerprint(),
        "same-domain evidence must retain exact transport context"
    );
    assert_ne!(evidence_a.transport(), evidence_b.transport());
}

#[test]
fn moderation_command_produces_only_the_reviewed_canonical_effect() {
    fn assert_canonical_effect<T: buzz_db::authorization_admission::AdmissionApplicationEffect>(
        _effect: &T,
    ) {
    }

    let community = CommunityId::from_uuid(Uuid::new_v4());
    let tenant = TenantContext::resolved(community, "effect.example");
    let actor = Keys::generate();
    let target = Keys::generate().public_key().to_bytes();
    let event = EventBuilder::new(Kind::from(9040_u16), "")
        .tag(Tag::parse(["p", &hex::encode(target)]).expect("p tag"))
        .sign_with_keys(&actor)
        .expect("signed moderation event");
    let effect = prepare_moderation_application_effect(&tenant, &event).expect("effect");
    assert_canonical_effect(&effect);
    assert_eq!(
        effect.admission_object().kind(),
        buzz_db::authorization_admission::AdmissionObjectKind::ModerationTarget
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL via BUZZ_TEST_DATABASE_URL"]
async fn repeated_same_target_publications_deliver_in_sequence_and_replay_is_idempotent() {
    let pool = postgres_pool().await;
    buzz_db::migration::run_migrations(&pool)
        .await
        .expect("run migrations");
    let community = insert_community(&pool, "outbox").await;
    let object_key = nonzero_32(10);
    let projection_key = format!("repos/{}/pointer", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO authorization_event_capacity \
         (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
         VALUES ($1, 16, 1048576, 16384)",
    )
    .bind(community.as_uuid())
    .execute(&pool)
    .await
    .expect("install outbox audit capacity");

    let seed = insert_publication(&pool, community, object_key, &projection_key, 1, false)
        .await
        .expect("seed same-epoch authority");
    sqlx::query(
        "INSERT INTO authorization_authority_epochs \
         (community_id, object_kind, object_key, authority_epoch, fence, \
          operation_id, request_fingerprint) \
         VALUES ($1, 3, $2, 7, $3, $4, $5)",
    )
    .bind(community.as_uuid())
    .bind(object_key.to_vec())
    .bind(nonzero_32(11).to_vec())
    .bind(seed.operation)
    .bind(seed.request.to_vec())
    .execute(&pool)
    .await
    .expect("install unchanged authority epoch");

    let first = insert_publication(&pool, community, object_key, &projection_key, 2, true);
    let second = insert_publication(&pool, community, object_key, &projection_key, 3, true);
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first concurrent publication");
    let second = second.expect("second concurrent publication");
    let duplicate = sqlx::query(
        "INSERT INTO protected_publication_projection_outbox \
         (community_id, operation_id, request_fingerprint, publication_result_digest, \
          object_kind, object_key, application_type, application_version, \
          application_effect_digest, projection_kind, projection_key, staged_object_key, \
          payload_digest) \
         VALUES ($1, $2, $3, $4, 3, $5, $6, 1, $7, 3, $8, $9, $10)",
    )
    .bind(community.as_uuid())
    .bind(first.operation)
    .bind(first.request.to_vec())
    .bind(first.result.to_vec())
    .bind(object_key.to_vec())
    .bind(first.application_type.to_vec())
    .bind(first.application_effect.to_vec())
    .bind(&projection_key)
    .bind("manifests/exact-replay")
    .bind(first.payload.to_vec())
    .execute(&pool)
    .await;
    assert!(
        duplicate.is_err(),
        "exact replay cannot create a second effect"
    );

    let rows = sqlx::query(
        "SELECT operation_id, delivery_state \
         FROM protected_publication_projection_outbox \
         WHERE community_id = $1 AND object_key = $2 AND projection_key = $3 \
         ORDER BY publication_sequence",
    )
    .bind(community.as_uuid())
    .bind(object_key.to_vec())
    .bind(&projection_key)
    .fetch_all(&pool)
    .await
    .expect("read concurrent ordered outbox");
    assert_eq!(rows.len(), 2);
    let earlier = rows[0].get::<Uuid, _>("operation_id");
    let later = rows[1].get::<Uuid, _>("operation_id");
    assert_ne!(earlier, later);
    assert!([first.operation, second.operation].contains(&earlier));
    assert!([first.operation, second.operation].contains(&later));

    let early = mark_delivered(&pool, community, later).await;
    assert!(
        early.is_err(),
        "newer projection cannot pass pending predecessor"
    );
    mark_delivered(&pool, community, earlier)
        .await
        .expect("deliver first");
    mark_delivered(&pool, community, later)
        .await
        .expect("deliver second");

    let delivered = sqlx::query(
        "SELECT operation_id, delivery_state \
         FROM protected_publication_projection_outbox \
         WHERE community_id = $1 AND object_key = $2 AND projection_key = $3 \
         ORDER BY publication_sequence",
    )
    .bind(community.as_uuid())
    .bind(object_key.to_vec())
    .bind(&projection_key)
    .fetch_all(&pool)
    .await
    .expect("read ordered outbox");
    assert_eq!(delivered.len(), 2);
    assert!(delivered
        .iter()
        .all(|row| row.get::<i16, _>("delivery_state") == 2));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL via BUZZ_TEST_DATABASE_URL"]
async fn uncommitted_lower_sequence_serializes_later_publication_and_delivery() {
    let pool = postgres_pool().await;
    buzz_db::migration::run_migrations(&pool)
        .await
        .expect("run migrations");
    let community = insert_community(&pool, "outbox-interleaving").await;
    let object_key = nonzero_32(15);
    let projection_key = format!("repos/{}/pointer", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO authorization_event_capacity \
         (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
         VALUES ($1, 16, 1048576, 16384)",
    )
    .bind(community.as_uuid())
    .execute(&pool)
    .await
    .expect("install interleaving audit capacity");

    let mut lower_transaction = pool.begin().await.expect("begin lower publication");
    let lower = stage_publication(
        &mut lower_transaction,
        community,
        object_key,
        &projection_key,
        4,
        true,
    )
    .await
    .expect("stage lower publication without committing");

    let later_pool = pool.clone();
    let later_projection_key = projection_key.clone();
    let mut later_task = tokio::spawn(async move {
        let later = insert_publication(
            &later_pool,
            community,
            object_key,
            &later_projection_key,
            5,
            true,
        )
        .await?;
        let delivery = mark_delivered(&later_pool, community, later.operation).await;
        Ok::<_, sqlx::Error>((later, delivery))
    });

    assert!(
        tokio::time::timeout(Duration::from_millis(250), &mut later_task)
            .await
            .is_err(),
        "later publication must wait for the lower allocator to commit or abort"
    );
    let visible_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM protected_publication_projection_outbox \
         WHERE community_id = $1 AND object_key = $2 AND projection_key = $3",
    )
    .bind(community.as_uuid())
    .bind(object_key.to_vec())
    .bind(&projection_key)
    .fetch_one(&pool)
    .await
    .expect("count externally visible interleaving rows");
    assert_eq!(
        visible_rows, 0,
        "the lower row is uncommitted and the later row is still serialized"
    );

    lower_transaction
        .commit()
        .await
        .expect("commit lower publication");
    let (later, premature_delivery) = tokio::time::timeout(Duration::from_secs(5), later_task)
        .await
        .expect("later publication unblocks after lower commit")
        .expect("later task joins")
        .expect("later publication commits");
    assert!(
        premature_delivery.is_err(),
        "later delivery must observe and reject the committed pending predecessor"
    );

    let ordered = sqlx::query(
        "SELECT operation_id, publication_sequence \
         FROM protected_publication_projection_outbox \
         WHERE community_id = $1 AND object_key = $2 AND projection_key = $3 \
         ORDER BY publication_sequence",
    )
    .bind(community.as_uuid())
    .bind(object_key.to_vec())
    .bind(&projection_key)
    .fetch_all(&pool)
    .await
    .expect("read interleaved publication order");
    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].get::<Uuid, _>("operation_id"), lower.operation);
    assert_eq!(ordered[1].get::<Uuid, _>("operation_id"), later.operation);
    assert!(
        ordered[0].get::<i64, _>("publication_sequence")
            < ordered[1].get::<i64, _>("publication_sequence")
    );

    mark_delivered(&pool, community, lower.operation)
        .await
        .expect("deliver committed predecessor");
    mark_delivered(&pool, community, later.operation)
        .await
        .expect("deliver later publication after predecessor");
}

#[tokio::test]
#[ignore = "requires live MinIO/S3 configured by BUZZ_S3_* variables"]
async fn changed_retry_attribution_reuses_the_first_exact_media_result() {
    let storage = MediaStorage::new(&media_config()).expect("media storage");
    let community = CommunityId::from_uuid(Uuid::new_v4());
    let first_tenant = TenantContext::resolved(community, "first.example");
    let retry_tenant = TenantContext::resolved(community, "alias.example");
    let uploader = Keys::generate().public_key();
    let request_event_id = hex::encode(nonzero_32(20));
    let sha256 = hex::encode(nonzero_32(21));
    let facts = UploadEventFacts {
        sha256: &sha256,
        ext: "png",
        mime: "image/png",
        size: 42,
        uploaded_at: 1_800_000_000,
    };
    let first = stage_upload_record(
        &storage,
        &first_tenant,
        &uploader,
        &request_event_id,
        &UploadAttribution {
            uploader_name: Some("first-name".into()),
            net: UploadNetworkInfo {
                ip: Some("8.8.8.8".parse().expect("test IP")),
                port: Some(443),
            },
        },
        facts,
    )
    .await
    .expect("stage first result");
    let retry = stage_upload_record(
        &storage,
        &retry_tenant,
        &uploader,
        &request_event_id,
        &UploadAttribution {
            uploader_name: Some("changed-name".into()),
            net: UploadNetworkInfo {
                ip: Some("1.1.1.1".parse().expect("test IP")),
                port: Some(8443),
            },
        },
        facts,
    )
    .await
    .expect("stage retry result");

    assert_eq!(retry.digest(), first.digest());
    assert_eq!(retry.canonical_bytes(), first.canonical_bytes());
    assert_eq!(retry.projection_key(), first.projection_key());
    assert_eq!(retry.record().community_host, "first.example");
    assert_eq!(retry.record().uploader_name.as_deref(), Some("first-name"));
    assert_eq!(retry.record().ip.as_deref(), Some("8.8.8.8"));
    assert_eq!(retry.record().port, Some(443));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL, live MinIO, and OpenSSL"]
async fn canonical_media_commit_then_exact_replay_has_one_outbox_and_no_replay_plan() {
    let pool = postgres_pool().await;
    buzz_db::migration::run_migrations(&pool)
        .await
        .expect("run migrations");
    let community = insert_community(&pool, "canonical-media-replay").await;
    sqlx::query(
        "INSERT INTO authorization_event_capacity \
         (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
         VALUES ($1, 32, 2097152, 16384)",
    )
    .bind(community.as_uuid())
    .execute(&pool)
    .await
    .expect("install canonical media audit capacity");
    sqlx::query(
        "INSERT INTO identity_enrollment_policies \
         (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
         VALUES ($1, 1, 1, $2, transaction_timestamp() - interval '1 second')",
    )
    .bind(community.as_uuid())
    .bind([85_u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect("install canonical media enrollment policy");
    sqlx::query(
        "INSERT INTO authorization_invalidation_domains (community_id, current_generation) \
         VALUES ($1, 0)",
    )
    .bind(community.as_uuid())
    .execute(&pool)
    .await
    .expect("activate canonical media invalidation domain");

    let mut config = media_config();
    config.max_image_bytes = 2 * 1024 * 1024;
    config.max_file_bytes = 2 * 1024 * 1024;
    let storage = MediaStorage::new(&config).expect("media storage");
    let tenant = TenantContext::resolved(community, "canonical-media.example");
    let keys = Keys::generate();
    let body = Bytes::from_static(include_bytes!(
        "../../buzz-media/tests/fixtures/ios/uikit-sanitized.png"
    ));
    let sha256 = hex::encode(Sha256::digest(&body));
    let expires = (nostr::Timestamp::now().as_secs() + 300).to_string();
    let auth_event = EventBuilder::new(Kind::from(24242), "Upload protected media")
        .tags(vec![
            Tag::parse(["t", "upload"]).expect("upload tag"),
            Tag::parse(["x", &sha256]).expect("hash tag"),
            Tag::parse(["expiration", &expires]).expect("expiration tag"),
            Tag::parse(["server", tenant.host()]).expect("server tag"),
        ])
        .sign_with_keys(&keys)
        .expect("sign Blossom upload");

    let first_staged = stage_upload(
        &storage,
        &config,
        &tenant,
        &auth_event,
        body.clone(),
        Some(UploadAttribution {
            uploader_name: Some("first-name".into()),
            net: UploadNetworkInfo {
                ip: Some("8.8.8.8".parse().expect("first IP")),
                port: Some(443),
            },
        }),
    )
    .await
    .expect("stage first canonical media upload");
    assert_eq!(
        first_staged
            .moderation_projection()
            .expect("first moderation projection")
            .record()
            .uploader_name
            .as_deref(),
        Some("first-name")
    );
    let first_plan = ProtectedPublicationPlan::from(first_staged);
    let target = first_plan.target().expect("first publication target");
    let first_effect = first_plan
        .application_effect()
        .expect("first application effect");

    let request_fingerprint = [83; 32];
    let transport_context = [84; 32];
    let (assertion, proof) = verified_authorization_evidence(
        community,
        &keys,
        *target.admission_object().key(),
        request_fingerprint,
        transport_context,
    );
    let authoritative_now = Utc::now();
    let first_request = enrollment_request_with_effect(
        &pool,
        community,
        (assertion.clone(), proof.clone()),
        target.admission_object(),
        RouteCapability::MediaWrite,
        authoritative_now,
        Box::new(first_effect),
    )
    .await;
    let operation_id = first_request.operation_id();
    let committer = PostgresCanonicalAdmissionCommitter::new(
        pool.clone(),
        Arc::new(ApplicationReplayRechecker),
    );
    let first_outcome = committer
        .commit(first_request)
        .await
        .expect("commit canonical media publication");
    let first_result = match &first_outcome {
        AdmissionCommitOutcome::Committed {
            application_result: Some(result),
            ..
        } => result.clone(),
        _ => panic!("first canonical media operation must commit with a typed result"),
    };
    let first_binding = ProtectedPublicationReceipt::bind(first_plan, &first_outcome)
        .expect("bind fresh canonical publication");
    let ProtectedPublicationBinding::Committed(fresh_receipt) = first_binding else {
        panic!("first canonical publication unexpectedly replayed");
    };
    let _fresh_projection_plan = fresh_receipt.into_plan();
    let first_outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM protected_publication_projection_outbox \
         WHERE community_id = $1 AND operation_id = $2",
    )
    .bind(community.as_uuid())
    .bind(operation_id)
    .fetch_one(&pool)
    .await
    .expect("count first canonical outbox rows");
    assert!(first_outbox_count > 0);

    let retry_staged = stage_upload(
        &storage,
        &config,
        &tenant,
        &auth_event,
        body,
        Some(UploadAttribution {
            uploader_name: Some("changed-name".into()),
            net: UploadNetworkInfo {
                ip: Some("1.1.1.1".parse().expect("retry IP")),
                port: Some(8443),
            },
        }),
    )
    .await
    .expect("stage exact retry with changed attribution");
    let retry_record = retry_staged
        .moderation_projection()
        .expect("retry moderation projection")
        .record();
    assert_eq!(retry_record.uploader_name.as_deref(), Some("first-name"));
    assert_eq!(retry_record.ip.as_deref(), Some("8.8.8.8"));
    assert_eq!(retry_record.port, Some(443));
    let retry_plan = ProtectedPublicationPlan::from(retry_staged);
    let retry_effect = retry_plan
        .application_effect()
        .expect("retry application effect");
    let retry_request = enrollment_request_with_effect(
        &pool,
        community,
        (assertion, proof),
        target.admission_object(),
        RouteCapability::MediaWrite,
        authoritative_now,
        Box::new(retry_effect),
    )
    .await;
    assert_eq!(retry_request.operation_id(), operation_id);
    let replay_outcome = committer
        .commit(retry_request)
        .await
        .expect("read exact canonical replay");
    let replay_binding = ProtectedPublicationReceipt::bind(retry_plan, &replay_outcome)
        .expect("bind exact replay without projection authority");
    let ProtectedPublicationBinding::ExactReplay(replay) = replay_binding else {
        panic!("retry repeated canonical application DML");
    };
    assert_eq!(replay.application_result(), &first_result);
    assert_eq!(replay.admission().operation_id(), operation_id);
    let replay_outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM protected_publication_projection_outbox \
         WHERE community_id = $1 AND operation_id = $2",
    )
    .bind(community.as_uuid())
    .bind(operation_id)
    .fetch_one(&pool)
    .await
    .expect("count exact-replay outbox rows");
    assert_eq!(replay_outbox_count, first_outbox_count);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and OpenSSL"]
async fn canonical_moderation_is_atomic_target_bound_and_fresh_only() {
    let pool = postgres_pool().await;
    buzz_db::migration::run_migrations(&pool)
        .await
        .expect("run migrations");
    let committer = PostgresCanonicalAdmissionCommitter::new(
        pool.clone(),
        Arc::new(ApplicationReplayRechecker),
    );
    let keys = Keys::generate();
    let actor = keys.public_key().to_bytes();

    let committed_domain = insert_community(&pool, "canonical-moderation-commit").await;
    install_enrollment_domain(&pool, committed_domain, true).await;
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'owner', NULL)",
    )
    .bind(committed_domain.as_uuid())
    .bind(hex::encode(actor))
    .execute(&pool)
    .await
    .expect("install canonical moderation owner");

    let committed_tenant =
        TenantContext::resolved(committed_domain, "canonical-moderation.example");
    let committed_target = Keys::generate().public_key().to_bytes();
    let committed_target_hex = hex::encode(committed_target);
    let committed_event = EventBuilder::new(Kind::from(9040_u16), "")
        .tag(Tag::parse(["p", &committed_target_hex]).expect("moderation target tag"))
        .sign_with_keys(&keys)
        .expect("sign canonical moderation command");
    let committed_effect =
        prepare_moderation_application_effect(&committed_tenant, &committed_event)
            .expect("prepare canonical moderation effect");
    let committed_object = committed_effect.admission_object();
    let committed_now = Utc::now();
    let (committed_assertion, committed_proof) = verified_authorization_evidence(
        committed_domain,
        &keys,
        *committed_object.key(),
        [87; 32],
        [88; 32],
    );
    let committed_request = enrollment_request_with_effect(
        &pool,
        committed_domain,
        (committed_assertion.clone(), committed_proof.clone()),
        committed_object,
        RouteCapability::Moderation,
        committed_now,
        Box::new(committed_effect),
    )
    .await;
    let committed_operation = committed_request.operation_id();
    let committed_outcome = committer
        .commit(committed_request)
        .await
        .expect("commit canonical moderation operation");
    let (committed_receipt, committed_result) = match &committed_outcome {
        AdmissionCommitOutcome::Committed {
            receipt,
            application_result: Some(result),
            ..
        } => (*receipt, result.clone()),
        _ => panic!("canonical moderation operation did not freshly commit"),
    };
    assert_eq!(committed_receipt.authorization_domain(), committed_domain);
    assert_eq!(committed_receipt.object(), committed_object);
    assert_eq!(committed_receipt.operation_id(), committed_operation);
    let persisted_result_digest = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT application_result_digest FROM authorization_admission_results \
         WHERE community_id = $1 AND operation_id = $2",
    )
    .bind(committed_domain.as_uuid())
    .bind(committed_operation)
    .fetch_one(&pool)
    .await
    .expect("read persisted moderation result digest");
    let persisted_result_digest: [u8; 32] = persisted_result_digest
        .try_into()
        .expect("32-byte moderation result digest");
    assert_eq!(
        committed_receipt.application_result_digest(),
        Some(&persisted_result_digest)
    );
    let decoded: serde_json::Value = serde_json::from_slice(committed_result.payload())
        .expect("decode canonical moderation application result");
    assert_eq!(
        decoded["object_key"],
        serde_json::to_value(committed_object.key()).expect("encode expected moderation object")
    );
    assert_eq!(
        decoded["action"]["Ban"]["target"],
        serde_json::to_value(committed_target).expect("encode expected moderation target")
    );
    let restriction =
        buzz_db::moderation::restriction_state(&pool, committed_domain, &committed_target)
            .await
            .expect("read committed moderation restriction");
    assert!(restriction.banned);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM moderation_actions \
             WHERE community_id = $1 AND target_pubkey = $2",
        )
        .bind(committed_domain.as_uuid())
        .bind(committed_target.as_slice())
        .fetch_one(&pool)
        .await
        .expect("count committed moderation actions"),
        1
    );

    let replay_effect = prepare_moderation_application_effect(&committed_tenant, &committed_event)
        .expect("prepare exact moderation replay effect");
    let replay_request = enrollment_request_with_effect(
        &pool,
        committed_domain,
        (committed_assertion, committed_proof),
        committed_object,
        RouteCapability::Moderation,
        committed_now,
        Box::new(replay_effect),
    )
    .await;
    assert_eq!(replay_request.operation_id(), committed_operation);
    let replay_outcome = committer
        .commit(replay_request)
        .await
        .expect("load exact canonical moderation replay");
    match replay_outcome {
        AdmissionCommitOutcome::ExactReplay {
            receipt,
            application_result: Some(result),
        } => {
            assert_eq!(receipt, committed_receipt);
            assert_eq!(result, committed_result);
        }
        _ => panic!("exact moderation replay was allowed to yield fresh post-commit work"),
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM moderation_actions \
             WHERE community_id = $1 AND target_pubkey = $2",
        )
        .bind(committed_domain.as_uuid())
        .bind(committed_target.as_slice())
        .fetch_one(&pool)
        .await
        .expect("count replay-suppressed moderation actions"),
        1
    );

    let rollback_domain = insert_community(&pool, "canonical-moderation-rollback").await;
    install_enrollment_domain(&pool, rollback_domain, false).await;
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'owner', NULL)",
    )
    .bind(rollback_domain.as_uuid())
    .bind(hex::encode(actor))
    .execute(&pool)
    .await
    .expect("install rollback moderation owner");
    let rollback_tenant = TenantContext::resolved(rollback_domain, "rollback-moderation.example");
    let rollback_target = Keys::generate().public_key().to_bytes();
    let rollback_target_hex = hex::encode(rollback_target);
    let rollback_event = EventBuilder::new(Kind::from(9040_u16), "")
        .tag(Tag::parse(["p", &rollback_target_hex]).expect("rollback target tag"))
        .sign_with_keys(&keys)
        .expect("sign rollback moderation command");
    let rollback_effect = prepare_moderation_application_effect(&rollback_tenant, &rollback_event)
        .expect("prepare rollback moderation effect");
    let rollback_object = rollback_effect.admission_object();
    let rollback_now = Utc::now();
    let (rollback_assertion, rollback_proof) = verified_authorization_evidence(
        rollback_domain,
        &keys,
        *rollback_object.key(),
        [89; 32],
        [90; 32],
    );
    let rollback_request = enrollment_request_with_effect(
        &pool,
        rollback_domain,
        (rollback_assertion, rollback_proof),
        rollback_object,
        RouteCapability::Moderation,
        rollback_now,
        Box::new(rollback_effect),
    )
    .await;
    let rollback_operation = rollback_request.operation_id();
    assert!(matches!(
        committer.commit(rollback_request).await,
        Err(AdmissionCommitError::AuditUnavailable)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM community_bans \
             WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(rollback_domain.as_uuid())
        .bind(rollback_target.as_slice())
        .fetch_one(&pool)
        .await
        .expect("count rolled-back restrictions"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM moderation_actions WHERE community_id = $1",
        )
        .bind(rollback_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count rolled-back moderation actions"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM authorization_operation_receipts \
             WHERE community_id = $1 AND operation_id = $2",
        )
        .bind(rollback_domain.as_uuid())
        .bind(rollback_operation)
        .fetch_one(&pool)
        .await
        .expect("count rolled-back admission receipts"),
        0
    );
}

async fn verify_proxy_request(
    verifier: &TrustedProxyProvenanceVerifier,
    request: &TrustedProxyRequest,
    transport: ProofTransport,
    path_and_query: &str,
    body: &[u8],
    nonce: &[u8],
) -> buzz_auth::SealedTransportEvidence {
    let assertion = format!("Bearer {}", proxy_assertion());
    let provenance = sign_proxy_provenance(transport, path_and_query, body, nonce);
    verifier
        .verify(
            &[
                HttpHeaderField::new(ASSERTION_HEADER_NAME, assertion.as_bytes()),
                HttpHeaderField::new(PROVENANCE_HEADER_NAME, provenance.as_bytes()),
            ],
            request,
            Utc.timestamp_opt(PROXY_NOW as i64, 0)
                .single()
                .expect("test time"),
            &EmptyReplayReader,
        )
        .await
        .expect("valid proxy evidence")
}

fn sign_proxy_provenance(
    transport: ProofTransport,
    path_and_query: &str,
    body: &[u8],
    nonce: &[u8],
) -> String {
    let assertion_digest: [u8; 32] = Sha256::digest(proxy_assertion().as_bytes()).into();
    let body_digest: [u8; 32] = Sha256::digest(body).into();
    let transport_code = match transport {
        ProofTransport::Nip42 => 1,
        ProofTransport::Nip98 => 2,
        ProofTransport::GitSmartHttpSession => 3,
        ProofTransport::Blossom => 4,
    };
    let mut input = b"NIP-FI-PROXY-1".to_vec();
    for field in [
        PROXY_NOW.to_be_bytes().as_slice(),
        nonce,
        &assertion_digest,
        b"POST".as_slice(),
        b"relay.example.com:443".as_slice(),
        path_and_query.as_bytes(),
        &body_digest,
        &[transport_code],
    ] {
        input.extend_from_slice(&(field.len() as u64).to_be_bytes());
        input.extend_from_slice(field);
    }
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&PROXY_SECRET).expect("HMAC key");
    mac.update(&input);
    format!(
        "v1.{PROXY_NOW}.{}.{}",
        base64url(nonce),
        base64url(&mac.finalize().into_bytes())
    )
}

fn base64url(value: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value)
}

async fn postgres_pool() -> PgPool {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .expect("BUZZ_TEST_DATABASE_URL must name a disposable PostgreSQL database");
    PgPool::connect(&url).await.expect("connect PostgreSQL")
}

async fn insert_community(pool: &PgPool, label: &str) -> CommunityId {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("s4-{label}-{}.example", id.simple()))
        .execute(pool)
        .await
        .expect("insert community");
    CommunityId::from_uuid(id)
}

async fn install_enrollment_domain(
    pool: &PgPool,
    community: CommunityId,
    install_audit_capacity: bool,
) {
    sqlx::query(
        "INSERT INTO identity_enrollment_policies \
         (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
         VALUES ($1, 1, 1, $2, transaction_timestamp() - interval '1 second')",
    )
    .bind(community.as_uuid())
    .bind([85_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("install enrollment policy");
    sqlx::query(
        "INSERT INTO authorization_invalidation_domains (community_id, current_generation) \
         VALUES ($1, 0)",
    )
    .bind(community.as_uuid())
    .execute(pool)
    .await
    .expect("activate invalidation domain");
    if install_audit_capacity {
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 32, 2097152, 16384)",
        )
        .bind(community.as_uuid())
        .execute(pool)
        .await
        .expect("install authorization audit capacity");
    }
}

struct PublicationFixture {
    operation: Uuid,
    request: [u8; 32],
    application_type: [u8; 32],
    application_effect: [u8; 32],
    result: [u8; 32],
    payload: [u8; 32],
}

async fn insert_publication(
    pool: &PgPool,
    community: CommunityId,
    object_key: [u8; 32],
    projection_key: &str,
    sequence: u8,
    insert_outbox: bool,
) -> Result<PublicationFixture, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let fixture = stage_publication(
        &mut transaction,
        community,
        object_key,
        projection_key,
        sequence,
        insert_outbox,
    )
    .await?;
    transaction.commit().await?;
    Ok(fixture)
}

async fn stage_publication(
    connection: &mut PgConnection,
    community: CommunityId,
    object_key: [u8; 32],
    projection_key: &str,
    sequence: u8,
    insert_outbox: bool,
) -> Result<PublicationFixture, sqlx::Error> {
    let fixture = PublicationFixture {
        operation: Uuid::new_v4(),
        request: nonzero_32(sequence.saturating_add(30)),
        application_type: nonzero_32(sequence.saturating_add(35)),
        application_effect: nonzero_32(sequence.saturating_add(37)),
        result: nonzero_32(sequence.saturating_add(40)),
        payload: nonzero_32(sequence.saturating_add(50)),
    };
    let actor = nonzero_32(60);
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id, operation_id, request_fingerprint, operation_kind, \
          actor_fingerprint, outcome_code, result_digest) \
         VALUES ($1, $2, $3, 11, $4, 1, $5)",
    )
    .bind(community.as_uuid())
    .bind(fixture.operation)
    .bind(fixture.request.to_vec())
    .bind(actor.to_vec())
    .bind(fixture.result.to_vec())
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        "INSERT INTO authorization_events \
         (community_id, event_id, event_kind, outcome_code, reason_code, actor_kind, \
          actor_fingerprint, operation_id, request_fingerprint, correlation_id, attempt_id, \
          occurred_at, canonical_envelope, envelope_digest) \
         VALUES ($1, $2, 10, 1, 1, 1, $3, $4, $5, $6, $7, \
                 transaction_timestamp(), $8, $9)",
    )
    .bind(community.as_uuid())
    .bind(Uuid::new_v4())
    .bind(actor.to_vec())
    .bind(fixture.operation)
    .bind(fixture.request.to_vec())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![sequence])
    .bind(nonzero_32(sequence.saturating_add(70)).to_vec())
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        "INSERT INTO authorization_admission_results \
         (community_id, operation_id, request_fingerprint, semantic_fingerprint, \
          object_kind, object_key, application_type, application_version, \
          application_code, application_payload, application_intent_digest, \
          application_effect_digest, application_result_digest) \
         VALUES ($1, $2, $3, $4, 3, $5, $6, 1, 1, $7, $8, $9, $10)",
    )
    .bind(community.as_uuid())
    .bind(fixture.operation)
    .bind(fixture.request.to_vec())
    .bind(nonzero_32(sequence.saturating_add(80)).to_vec())
    .bind(object_key.to_vec())
    .bind(fixture.application_type.to_vec())
    .bind(vec![sequence])
    .bind(nonzero_32(sequence.saturating_add(90)).to_vec())
    .bind(fixture.application_effect.to_vec())
    .bind(fixture.result.to_vec())
    .execute(&mut *connection)
    .await?;
    if insert_outbox {
        sqlx::query(
            "INSERT INTO protected_publication_projection_outbox \
             (community_id, operation_id, request_fingerprint, publication_result_digest, \
              object_kind, object_key, application_type, application_version, \
              application_effect_digest, projection_kind, projection_key, staged_object_key, \
              payload_digest) \
             VALUES ($1, $2, $3, $4, 3, $5, $6, 1, $7, 3, $8, $9, $10)",
        )
        .bind(community.as_uuid())
        .bind(fixture.operation)
        .bind(fixture.request.to_vec())
        .bind(fixture.result.to_vec())
        .bind(object_key.to_vec())
        .bind(fixture.application_type.to_vec())
        .bind(fixture.application_effect.to_vec())
        .bind(projection_key)
        .bind(format!("manifests/{}/{}", fixture.operation, sequence))
        .bind(fixture.payload.to_vec())
        .execute(&mut *connection)
        .await?;
    }
    Ok(fixture)
}

async fn mark_delivered(
    pool: &PgPool,
    community: CommunityId,
    operation: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "UPDATE protected_publication_projection_outbox \
         SET delivery_state = 2, attempt_count = 1, failure_code = 0 \
         WHERE community_id = $1 AND operation_id = $2 AND projection_kind = 3",
    )
    .bind(community.as_uuid())
    .bind(operation)
    .execute(pool)
    .await
}

fn media_config() -> MediaConfig {
    MediaConfig {
        s3_endpoint: std::env::var("BUZZ_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:9000".into()),
        s3_access_key: std::env::var("BUZZ_S3_ACCESS_KEY").unwrap_or_else(|_| "buzz_dev".into()),
        s3_secret_key: std::env::var("BUZZ_S3_SECRET_KEY")
            .unwrap_or_else(|_| "buzz_dev_secret".into()),
        s3_bucket: std::env::var("BUZZ_S3_BUCKET").unwrap_or_else(|_| "buzz-media".into()),
        s3_region: std::env::var("BUZZ_S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
        s3_addressing_style: S3AddressingStyle::Path,
        max_image_bytes: 1,
        max_gif_bytes: 1,
        max_video_bytes: 1,
        max_file_bytes: 1,
        public_base_url: "http://localhost:3000/media".into(),
        upload_records_enabled: true,
        upload_ip_header: None,
        upload_port_header: None,
    }
}

fn nonzero_32(seed: u8) -> [u8; 32] {
    [seed.max(1); 32]
}
