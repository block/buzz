//! Real command against PostgreSQL and a read-only S3 protocol fixture.
use axum::{
    extract::State,
    http::{Method, StatusCode, Uri},
    response::IntoResponse,
    routing::any,
    Router,
};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use tokio::process::Command;
use uuid::Uuid;

#[derive(Clone)]
struct S3Fixture {
    hash: String,
    upload_key: String,
    upload_record: String,
    methods: Arc<Mutex<Vec<Method>>>,
}
async fn storage(State(state): State<S3Fixture>, method: Method, uri: Uri) -> impl IntoResponse {
    state.methods.lock().unwrap().push(method.clone());
    if method != Method::GET {
        return (StatusCode::FORBIDDEN, "unexpected storage operation".into());
    }
    if uri.query().unwrap_or_default().contains("versions") {
        return (StatusCode::OK,format!("<ListVersionsResult><IsTruncated>false</IsTruncated><Version><Key>{}.jpg</Key><VersionId>fixture-v1</VersionId><Size>4</Size><IsLatest>true</IsLatest></Version><Version><Key>{}</Key><VersionId>upload-v1</VersionId><Size>{}</Size><IsLatest>false</IsLatest></Version></ListVersionsResult>",state.hash,state.upload_key,state.upload_record.len()));
    }
    if uri.path().ends_with(&state.upload_key)
        && uri
            .query()
            .unwrap_or_default()
            .contains("versionId=upload-v1")
    {
        return (StatusCode::OK, state.upload_record);
    }
    (
        StatusCode::FORBIDDEN,
        "exact upload version required".into(),
    )
}
fn binary() -> std::ffi::OsString {
    std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into())
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn preview_cli_saves_inventory_resumes_and_never_calls_delete() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let human = nostr::Keys::generate().public_key().to_hex();
    let community = Uuid::new_v4();
    let run = Uuid::new_v4();
    let hash = "a".repeat(64);
    sqlx::query("INSERT INTO communities (id,host) VALUES ($1,'preview-fixture.example.org')")
        .bind(community)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO media_uploaders (community_id,sha256,pubkey) VALUES ($1,$2,decode($3,'hex'))",
    )
    .bind(community)
    .bind(&hash)
    .bind(&human)
    .execute(&pool)
    .await
    .unwrap();
    let state = S3Fixture {
        hash: hash.clone(),
        upload_key: format!(
            "_uploads/{community}/{}/01ARZ3NDEKTSV4RRFFQ69G5FAV.json",
            "b".repeat(64)
        ),
        upload_record: serde_json::json!({
            "version":1,"community_id":community,"sha256":"b".repeat(64),
            "event_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","uploader_id":human
        })
        .to_string(),
        methods: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .fallback(any(storage))
        .with_state(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let invoke = |run_id: Uuid, max_source_rows: usize| {
        let mut command = Command::new(binary());
        command
            .args([
                "user-deletions",
                "media",
                "--pubkey",
                &human,
                "--run-id",
                &run_id.to_string(),
                "--mode",
                "print-only",
                "--max-source-rows",
                &max_source_rows.to_string(),
                "--scan-timeout-secs",
                "900",
            ])
            .env("DATABASE_URL", &url)
            .env("BUZZ_MEDIA_SOURCE_DATABASE_URL", &url)
            .env("BUZZ_S3_ENDPOINT", format!("http://{addr}"))
            .env("BUZZ_S3_BUCKET", "fixture")
            .env("BUZZ_S3_ACCESS_KEY", "fixture")
            .env("BUZZ_S3_SECRET_KEY", "fixture-secret")
            .env("BUZZ_S3_ADDRESSING_STYLE", "path");
        command
    };
    let capped_run = Uuid::new_v4();
    let capped = invoke(capped_run, 1).output().await.unwrap();
    assert!(!capped.status.success());
    assert!(String::from_utf8_lossy(&capped.stderr).contains("max-source-rows"));
    assert!(state.methods.lock().unwrap().is_empty());
    let unpublished: bool = sqlx::query_scalar("SELECT NOT (metadata_json ? 'media_discovery') AND NOT EXISTS (SELECT 1 FROM entity_deletion_media_files WHERE stage_id=$1) FROM entity_deletion_stage_history WHERE id=$1")
        .bind(capped_run).fetch_one(&pool).await.unwrap();
    assert!(unpublished);
    let first = invoke(run, 10_000_000).output().await.unwrap();
    assert_eq!(
        first.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let inventory: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(inventory["discovery_complete"], true);
    assert_eq!(inventory["candidate_count"], 2);
    assert_eq!(inventory["execution_authorized"], false);
    assert_eq!(inventory["files"][0]["file_hash"], hash);
    assert_eq!(inventory["files"][0]["status"], "unknown");
    assert_eq!(
        inventory["files"][0]["evidence_json"]["observed_objects"][0]["version_id"],
        "fixture-v1"
    );
    // This file has no database uploader or reference. Its historical upload
    // record alone must add it, and must be fetched at the listed version.
    assert_eq!(inventory["files"][1]["file_hash"], "b".repeat(64));
    assert_eq!(
        inventory["files"][1]["evidence_json"]["sources"][0]["table"],
        "storage_uploads"
    );
    let methods = state.methods.lock().unwrap().clone();
    assert!(!methods.is_empty());
    assert!(methods.iter().all(|m| *m == Method::GET));
    server.abort(); // Recovery must use the saved inventory even with S3 offline.
    let second = invoke(run, 10_000_000).output().await.unwrap();
    assert_eq!(
        second.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let repeated: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(inventory, repeated);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM media_uploaders WHERE community_id=$1")
            .bind(community)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 1);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entity_deletion_media_files WHERE stage_id=$1")
            .bind(run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
    let rejected = Command::new(binary())
        .args([
            "user-deletions",
            "media",
            "--pubkey",
            &human,
            "--mode",
            "print",
        ])
        .env_remove("DATABASE_URL")
        .output()
        .await
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("ProtectedMedia"));
}
