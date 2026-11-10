//! Concrete handler through PostgreSQL and the S3 HTTP protocol, including lost
//! acknowledgements. No fake ProtectedMedia implementation is used here.
use axum::{
    extract::{Query, State},
    http::{Method, StatusCode},
    routing::any,
    Router,
};
use buzz_db::{
    entity_deletion::{NewEntityStage, NextStage, StageTransition},
    Db,
};
use buzz_deletion::user::media::{run_stage, Mode, StageOptions};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone)]
struct Objects {
    versions: Arc<Mutex<BTreeMap<String, String>>>,
    fail_delete_response: Arc<AtomicBool>,
    denied_retention: Arc<AtomicBool>,
    deletes: Arc<Mutex<Vec<String>>>,
}

async fn s3(
    State(state): State<Objects>,
    method: Method,
    Query(query): Query<BTreeMap<String, String>>,
    body: axum::body::Bytes,
) -> (StatusCode, String) {
    if method == Method::GET && query.contains_key("versions") {
        let prefix = query.get("prefix").map(String::as_str).unwrap_or("");
        let mut xml = "<ListVersionsResult><IsTruncated>false</IsTruncated>".to_owned();
        for (key, version) in state
            .versions
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
        {
            xml.push_str(&format!(
                "<Version><Key>{key}</Key><VersionId>{version}</VersionId><Size>4</Size></Version>"
            ));
        }
        xml.push_str("</ListVersionsResult>");
        return (StatusCode::OK, xml);
    }
    if method == Method::GET && query.contains_key("object-lock") {
        if state.denied_retention.load(Ordering::SeqCst) {
            return (
                StatusCode::FORBIDDEN,
                "<Error><Code>AccessDenied</Code></Error>".into(),
            );
        }
        return (
            StatusCode::NOT_FOUND,
            "<Error><Code>ObjectLockConfigurationNotFoundError</Code></Error>".into(),
        );
    }
    if method == Method::POST && query.contains_key("delete") {
        let body = String::from_utf8(body.to_vec()).unwrap();
        let mut versions = state.versions.lock().unwrap();
        let selected = versions
            .iter()
            .find(|(key, version)| {
                body.contains(&format!("<Key>{key}</Key>"))
                    && body.contains(&format!("<VersionId>{version}</VersionId>"))
            })
            .map(|(key, version)| (key.clone(), version.clone()));
        let mut result = "<DeleteResult>".to_owned();
        if let Some((key, version)) = selected {
            versions.remove(&key);
            state.deletes.lock().unwrap().push(version.clone());
            result.push_str(&format!(
                "<Deleted><Key>{key}</Key><VersionId>{version}</VersionId></Deleted>"
            ));
        }
        if state.fail_delete_response.load(Ordering::SeqCst) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "<Error><Code>InternalError</Code></Error>".into(),
            );
        }
        result.push_str("</DeleteResult>");
        return (StatusCode::OK, result);
    }
    (StatusCode::FORBIDDEN, "unexpected storage call".into())
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn actual_handler_checks_retention_blocks_writers_and_recovers_deleted_versions() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let db = Db::from_pool(sqlx::PgPool::connect(&url).await.unwrap());
    let human = nostr::Keys::generate().public_key().to_hex();
    let community = Uuid::new_v4();
    let other = Uuid::new_v4();
    for id in [community, other] {
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(id)
            .bind(format!("{id}.example.org"))
            .execute(db.pool())
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
        .bind(community)
        .bind(&human)
        .execute(db.pool())
        .await
        .unwrap();
    let hash = "a".repeat(64);
    db.record_media_upload(
        buzz_core::CommunityId::from_uuid(community),
        &hash,
        &hex::decode(&human).unwrap(),
    )
    .await
    .unwrap();
    let request = db
        .user_deletion_store()
        .submit(&human, "fixture-operator", None)
        .await
        .unwrap();
    let requests = db.entity_deletion_store();
    requests
        .approve(
            request.id,
            "fixture-approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let lease = requests
        .claim(request.id, "fixture-worker", Duration::from_secs(300))
        .await
        .unwrap()
        .unwrap();
    let NextStage::Stage(stage) = requests
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "purge_media".into(),
                community: None,
                inputs: json!({}),
            }))
        })
        .await
        .unwrap()
    else {
        panic!("stage missing")
    };
    requests.start_stage(&lease, stage.id).await.unwrap();
    let state = Objects {
        versions: Arc::new(Mutex::new(BTreeMap::from([
            (format!("{hash}.jpg"), "original-v1".into()),
            (format!("{hash}.thumb.webp"), "thumbnail-v2".into()),
        ]))),
        fail_delete_response: Arc::new(AtomicBool::new(true)),
        denied_retention: Arc::new(AtomicBool::new(true)),
        deletes: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(any(s3)).with_state(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let media = buzz_media::MediaStorage::new(&buzz_media::MediaConfig {
        s3_endpoint: format!("http://{address}"),
        s3_bucket: "fixture".into(),
        s3_access_key: "fixture-key".into(),
        s3_secret_key: "fixture-secret".into(),
        s3_region: "us-east-1".into(),
        s3_addressing_style: buzz_media::S3AddressingStyle::Path,
        max_image_bytes: 100,
        max_gif_bytes: 100,
        max_video_bytes: 100,
        max_file_bytes: 100,
        public_base_url: "https://example.org/media".into(),
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    })
    .unwrap();
    let options = StageOptions {
        human,
        include_owned_agents: false,
        storage_namespace: "fixture".into(),
        media_bases: vec![],
        scan_budget: Default::default(),
        mode: Mode::NoPrint,
    };
    let first = run_stage(&db, &media, &lease, stage.id, options.clone()).await;
    assert!(
        first.is_err(),
        "denied retention must not authorize deletion"
    );
    assert!(state.deletes.lock().unwrap().is_empty());
    let saved: (String, bool) =
        sqlx::query_as("SELECT status,reserved FROM entity_deletion_media_files WHERE stage_id=$1")
            .bind(stage.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(saved, ("unknown".into(), true));
    // Reservation persists after the first handler has returned. Both an upload
    // claim and an actual signed event in a different community must be refused.
    let outsider = nostr::Keys::generate();
    let other = buzz_core::CommunityId::from_uuid(other);
    assert!(db
        .record_media_upload(other, &hash, outsider.public_key().as_bytes())
        .await
        .is_err());
    let event = nostr::EventBuilder::new(nostr::Kind::TextNote, format!("/media/{hash}.jpg"))
        .sign_with_keys(&outsider)
        .unwrap();
    assert!(db.insert_event(other, &event, None).await.is_err());
    // Embedded JSON may encode each hash character; the writer must use the
    // same recursive interpretation as inventory extraction.
    let embedded = format!(r#"{{"url":"/media/{}.jpg"}}"#, "\\u0061".repeat(64));
    let encoded = nostr::EventBuilder::new(nostr::Kind::TextNote, embedded)
        .sign_with_keys(&outsider)
        .unwrap();
    assert!(db.insert_event(other, &encoded, None).await.is_err());
    state.denied_retention.store(false, Ordering::SeqCst);
    let second = run_stage(&db, &media, &lease, stage.id, options.clone()).await;
    assert!(
        second.is_err(),
        "lost deletion acknowledgement must stay retryable"
    );
    assert_eq!(state.versions.lock().unwrap().len(), 1);
    let saved: (String, serde_json::Value) = sqlx::query_as(
        "SELECT status,delete_targets_json FROM entity_deletion_media_files WHERE stage_id=$1",
    )
    .bind(stage.id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(saved.0, "to_be_deleted");
    assert_eq!(saved.1.as_array().unwrap().len(), 2);
    state.fail_delete_response.store(false, Ordering::SeqCst);
    run_stage(&db, &media, &lease, stage.id, options)
        .await
        .unwrap();
    assert!(state.versions.lock().unwrap().is_empty());
    assert_eq!(
        state.deletes.lock().unwrap().len(),
        2,
        "already removed version is not deleted again"
    );
    let finished: (String,bool,serde_json::Value) = sqlx::query_as("SELECT status,reserved,delete_targets_json FROM entity_deletion_media_files WHERE stage_id=$1")
        .bind(stage.id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(finished, ("deleted".into(), false, saved.1));
    let proofs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM media_uploaders WHERE community_id=$1")
            .bind(community)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(proofs, 0);
    db.record_media_upload(other, &hash, outsider.public_key().as_bytes())
        .await
        .unwrap();
    server.abort();
}
