//! Synthetic route-to-PostgreSQL proof for the explicitly composed O5 surface.

use std::{str::FromStr, sync::Arc, time::SystemTime};

use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use buzz_audit::authorization::{PseudonymKey, Pseudonymizer};
use buzz_db::operator_lifecycle::OperatorReferenceKey;
use buzz_relay::{
    api::operator::lifecycle_router,
    operator_persistence::PostgresOperatorExecutor,
    operator_runtime::{
        GrantedOperatorCapability, GrantedOperatorReplacement, OpaqueOperatorReference,
        OperatorAuthenticator, OperatorAuthorizationRequest, OperatorCapability, OperatorClock,
        OperatorCredential, OperatorRuntime, OperatorRuntimeError,
    },
};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1
const CREDENTIAL: &str = "synthetic-route-postgres-credential-canary";

struct IsolatedDatabase {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl IsolatedDatabase {
    async fn migrated() -> Self {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let admin_options = PgConnectOptions::from_str(&database_url)
            .expect("O5 route test database URL must be valid PostgreSQL");
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(admin_options.clone())
            .await
            .expect(
                "O5 route PostgreSQL gate requires a database; a zero-assertion success is prohibited",
            );
        let name = format!("o5_route_{}", Uuid::new_v4().simple());
        assert!(name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'));
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
            .execute(&admin)
            .await
            .expect("create isolated O5 route database");
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(admin_options.database(&name))
            .await
            .expect("connect isolated O5 route database");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("apply exact O5 migrations for route test");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await
            .expect("count executed route-test migrations");
        assert_eq!(count, 50, "route test executes the full O5 SQLx chain");
        Self { pool, admin, name }
    }

    async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE \"{}\" WITH (FORCE)",
            self.name
        )))
        .execute(&self.admin)
        .await
        .expect("drop isolated O5 route database");
        self.admin.close().await;
    }
}

struct SystemClock;

impl OperatorClock for SystemClock {
    fn now_unix_seconds(&self) -> Result<u64, OperatorRuntimeError> {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(|_| OperatorRuntimeError::InvalidAuthority)
    }
}

struct Grant {
    domain_id: Uuid,
    operation_id: Uuid,
    fingerprint: [u8; 32],
    authority_id: Uuid,
    approval_ids: Vec<Uuid>,
    replacement: Option<GrantedOperatorReplacement>,
    expires_at: u64,
}

impl GrantedOperatorCapability for Grant {
    fn authority_evidence_id(&self) -> Uuid {
        self.authority_id
    }

    fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    fn intent_fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }

    fn actor_reference(&self) -> OpaqueOperatorReference {
        OpaqueOperatorReference::from_digest([11; 32])
    }

    fn provenance_reference(&self) -> OpaqueOperatorReference {
        OpaqueOperatorReference::from_digest([12; 32])
    }

    fn approval_evidence_ids(&self) -> &[Uuid] {
        &self.approval_ids
    }

    fn replacement(&self) -> Option<GrantedOperatorReplacement> {
        self.replacement
    }

    fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at
    }

    fn permits(&self, _capability: OperatorCapability) -> bool {
        true
    }
}

struct Authenticator;

#[async_trait]
impl OperatorAuthenticator for Authenticator {
    async fn authenticate(
        &self,
        credential: &OperatorCredential,
        request: OperatorAuthorizationRequest,
    ) -> Result<Box<dyn GrantedOperatorCapability>, OperatorRuntimeError> {
        assert_eq!(credential.expose_to_authenticator(), CREDENTIAL.as_bytes());
        let replacement = request
            .replacement_reference()
            .map(|reference| {
                GrantedOperatorReplacement::new(reference, [reference.digest()[0]; 32], [78; 32])
            })
            .transpose()?;
        let now = SystemClock.now_unix_seconds()?;
        Ok(Box::new(Grant {
            domain_id: request.domain_id(),
            operation_id: request.operation_id(),
            fingerprint: request.intent_fingerprint(),
            authority_id: Uuid::new_v4(),
            approval_ids: vec![Uuid::new_v4()],
            replacement,
            expires_at: now + 300,
        }))
    }
}

fn reference(byte: u8) -> String {
    hex::encode([byte; 32])
}

fn common_body(domain_id: Uuid, operation_id: Uuid, revision: u64) -> Value {
    json!({
        "domain_id": domain_id,
        "operation_id": operation_id,
        "correlation_id": Uuid::new_v4(),
        "reason": "planned_rotation",
        "expected_revision": revision,
        "approval_references": [reference(13)],
    })
}

async fn post(runtime: Arc<OperatorRuntime>, path: &str, body: Value) -> (StatusCode, Value) {
    let response = lifecycle_router(runtime)
        .oneshot(
            Request::post(path)
                .header(header::AUTHORIZATION, CREDENTIAL)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("operator route request"),
        )
        .await
        .expect("operator route response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("operator route response body");
    let body = serde_json::from_slice(&bytes).expect("operator route JSON response");
    (status, body)
}

#[tokio::test]
async fn explicitly_composed_routes_reach_real_postgres_list_preview_revoke_and_rotate() {
    let fixture = IsolatedDatabase::migrated().await;
    let domain = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
        .bind(domain)
        .bind(format!("{domain}.route.o5.test"))
        .execute(&fixture.pool)
        .await
        .expect("insert synthetic route domain");
    for (binding_id, issuer, subject, key) in [
        (
            Uuid::new_v4(),
            "https://issuer-a.invalid",
            "subject-a",
            [31_u8; 32],
        ),
        (
            Uuid::new_v4(),
            "https://issuer-b.invalid",
            "subject-b",
            [32_u8; 32],
        ),
    ] {
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,issuer,uid,pubkey,source,binding_id,creation_attribution_kind) \
             VALUES ($1,$2,$3,$4,'db_binding',$5,'legacy_unknown')",
        )
        .bind(domain)
        .bind(issuer)
        .bind(subject)
        .bind(key.as_slice())
        .bind(binding_id)
        .execute(&fixture.pool)
        .await
        .expect("insert synthetic route binding");
    }
    let db = buzz_db::Db::from_pool(fixture.pool.clone());
    let executor = Arc::new(PostgresOperatorExecutor::new(
        db,
        OperatorReferenceKey::new([41; 32], 1).expect("operator reference key"),
        Pseudonymizer::new(PseudonymKey::new([42; 32]).expect("pseudonym key"), 1),
    ));
    let runtime = Arc::new(OperatorRuntime::new(
        Arc::new(Authenticator),
        executor,
        Arc::new(SystemClock),
    ));
    let mut scenarios = 0_u32;

    let mut list = common_body(domain, Uuid::new_v4(), 1);
    list["limit"] = json!(10);
    let (status, listed) = post(runtime.clone(), "/operator/v1/lifecycle/list", list).await;
    assert_eq!(status, StatusCode::OK, "list response: {listed}");
    let records = listed["records"].as_array().expect("redacted list records");
    assert_eq!(records.len(), 2);
    let first = records[0]["reference"].as_str().expect("first reference");
    let second = records[1]["reference"].as_str().expect("second reference");
    scenarios += 1;

    let mut preview = common_body(domain, Uuid::new_v4(), 1);
    preview["target"] = json!(first);
    preview["replacement"] = json!(reference(70));
    let (status, previewed) =
        post(runtime.clone(), "/operator/v1/lifecycle/preview", preview).await;
    assert_eq!(status, StatusCode::OK, "preview response: {previewed}");
    scenarios += 1;

    let mut revoke = common_body(domain, Uuid::new_v4(), 1);
    revoke["target"] = json!(first);
    revoke["reason"] = json!("emergency_containment");
    let (status, revoked) = post(runtime.clone(), "/operator/v1/lifecycle/revoke", revoke).await;
    assert_eq!(status, StatusCode::OK, "revoke response: {revoked}");
    assert_eq!(revoked["lifecycle_revision"], 2);
    scenarios += 1;

    let mut rotate = common_body(domain, Uuid::new_v4(), 2);
    rotate["target"] = json!(second);
    rotate["replacement"] = json!(reference(71));
    let (status, rotated) = post(runtime.clone(), "/operator/v1/lifecycle/rotate", rotate).await;
    assert_eq!(status, StatusCode::OK, "rotate response: {rotated}");
    assert_eq!(rotated["lifecycle_revision"], 3);
    scenarios += 1;

    let mut current = common_body(domain, Uuid::new_v4(), 3);
    current["limit"] = json!(10);
    let (status, current) = post(runtime.clone(), "/operator/v1/lifecycle/list", current).await;
    assert_eq!(status, StatusCode::OK, "current list response: {current}");
    let current_target = current["records"]
        .as_array()
        .expect("current redacted list records")
        .iter()
        .find(|record| record["state"] == "active")
        .and_then(|record| record["reference"].as_str())
        .expect("current active replacement reference");
    scenarios += 1;

    let mut retired_preview = common_body(domain, Uuid::new_v4(), 3);
    retired_preview["target"] = json!(current_target);
    retired_preview["replacement"] = json!(reference(32));
    let (status, denied_preview) = post(
        runtime.clone(),
        "/operator/v1/lifecycle/preview",
        retired_preview,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "retired-key preview must fail closed: {denied_preview}"
    );
    scenarios += 1;

    let mut rotate_back = common_body(domain, Uuid::new_v4(), 3);
    rotate_back["target"] = json!(current_target);
    rotate_back["replacement"] = json!(reference(32));
    let (status, denied_rotate) = post(runtime, "/operator/v1/lifecycle/rotate", rotate_back).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "retired-key rotation must fail closed: {denied_rotate}"
    );
    scenarios += 1;

    let receipts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM authorization_operator_operation_receipts WHERE community_id=$1",
    )
    .bind(domain)
    .fetch_one(&fixture.pool)
    .await
    .expect("count reachable operator receipts");
    let effects: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM authorization_operator_effects WHERE community_id=$1",
    )
    .bind(domain)
    .fetch_one(&fixture.pool)
    .await
    .expect("count reachable operator effects");
    let previews: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM authorization_lifecycle_previews WHERE community_id=$1",
    )
    .bind(domain)
    .fetch_one(&fixture.pool)
    .await
    .expect("count reachable operator previews");
    assert_eq!(receipts, 7);
    assert_eq!(effects, 2);
    assert_eq!(previews, 1, "denied preview cannot persist an impact plan");
    assert_eq!(scenarios, 7, "every reachable route scenario executed");

    fixture.cleanup().await;
}
