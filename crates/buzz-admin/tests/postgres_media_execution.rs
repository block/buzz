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
    let hash = Uuid::new_v4().simple().to_string().repeat(2);
    db.record_media_upload(
        buzz_core::CommunityId::from_uuid(community),
        &hash,
        &hex::decode(&human).unwrap(),
    )
    .await
    .unwrap();
    // A second person copying the file URL must not keep its bytes alive.
    let copier = nostr::Keys::generate();
    let copied =
        nostr::EventBuilder::new(nostr::Kind::TextNote, format!("copied /media/{hash}.jpg"))
            .sign_with_keys(&copier)
            .unwrap();
    db.insert_event(buzz_core::CommunityId::from_uuid(other), &copied, None)
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
    let inventory = buzz_db::user_deletion::UserDeletionInventory::read(&request).unwrap();
    let NextStage::Stage(gate) = requests
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "gate_user".into(),
                community: None,
                inputs: inventory.inputs(&human, None)?,
            }))
        })
        .await
        .unwrap()
    else {
        panic!("gate missing")
    };
    requests.start_stage(&lease, gate.id).await.unwrap();
    let scope = buzz_db::media_deletion::SubjectScope {
        human: human.clone(),
        agents: Default::default(),
    };
    let result = db
        .user_deletion_store()
        .gate_user(&lease, gate.id, &scope)
        .await
        .unwrap();
    requests
        .save_stage_success(&lease, gate.id, result)
        .await
        .unwrap();
    let NextStage::Stage(stage) = requests
        .transition_stage(&lease, Some(gate.id), |_, _| {
            Ok(StageTransition::Advance {
                previous: gate.id,
                decision: json!({"kind":"stage", "stage":"purge_media"}),
                successor: Some(NewEntityStage {
                    stage: "purge_media".into(),
                    community: None,
                    inputs: inventory.inputs(&human, None)?,
                }),
            })
        })
        .await
        .unwrap()
    else {
        panic!("media stage missing")
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
    // Workflow preview must not bind or advance this destructive stage, create
    // journal rows, gate uploads, or change source/storage state.
    let preview_media = buzz_media::MediaStorage::new(
        &serde_json::from_value::<buzz_media::MediaConfig>(json!({
            "s3_endpoint": format!("http://{address}"), "s3_bucket": "fixture",
            "s3_access_key": "fixture", "s3_secret_key": Uuid::new_v4().to_string(),
            "max_image_bytes": 1, "max_gif_bytes": 1,
            "public_base_url": "https://example.org/media"
        }))
        .unwrap(),
    )
    .unwrap();
    let preview_options = buzz_deletion::user::media::StageOptions {
        human: human.clone(),
        include_owned_agents: false,
        storage_namespace: "fixture".into(),
        scan_budget: Default::default(),
        mode: buzz_deletion::user::media::Mode::PrintOnly,
    };
    let snapshot = || async {
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT jsonb_build_object(
             'requests',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM entity_deletion_requests r),
             'stages',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM entity_deletion_stage_history s),
             'files',(SELECT jsonb_agg(to_jsonb(f)) FROM entity_deletion_media_files f),
             'fences',(SELECT jsonb_agg(to_jsonb(f)) FROM user_deletion_fences f),
             'uploaders',(SELECT jsonb_agg(to_jsonb(u)) FROM media_uploaders u))"
        ).fetch_one(db.pool()).await.unwrap()
    };
    let before = snapshot().await;
    let mut wrong_options = preview_options.clone();
    wrong_options.human = nostr::Keys::generate().public_key().to_hex();
    wrong_options.mode = buzz_deletion::user::media::Mode::Delete;
    let wrong =
        buzz_deletion::user::media::run_stage(&db, &preview_media, &lease, stage.id, wrong_options)
            .await
            .unwrap_err();
    assert!(wrong
        .to_string()
        .contains("must match the approved user request"));
    assert_eq!(snapshot().await, before);
    let unresolved = buzz_deletion::user::media::preview(&db, &preview_media, &preview_options)
        .await
        .unwrap();
    assert_eq!(unresolved["decision_complete"], false);
    assert_eq!(unresolved["unknown_count"], 1);
    assert_eq!(unresolved["execution_authorized"], false);
    let incomplete = buzz_deletion::user::media::run_stage(
        &db,
        &preview_media,
        &lease,
        stage.id,
        preview_options.clone(),
    )
    .await
    .unwrap_err();
    assert!(incomplete.to_string().contains("unresolved decisions"));
    assert_eq!(snapshot().await, before);
    assert_eq!(state.versions.lock().unwrap().len(), 2);
    assert!(state.deletes.lock().unwrap().is_empty());
    state.denied_retention.store(false, Ordering::SeqCst);
    let planned = buzz_deletion::user::media::preview(&db, &preview_media, &preview_options)
        .await
        .unwrap();
    assert_eq!(planned["decision_complete"], true);
    assert_eq!(
        planned["files"][0]["delete_targets_json"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let summary = buzz_deletion::user::media::run_stage(
        &db,
        &preview_media,
        &lease,
        stage.id,
        preview_options,
    )
    .await
    .unwrap();
    assert_eq!(summary["candidate_count"], 1);
    assert_eq!(summary["unknown_count"], 0);
    assert_eq!(summary["supported_scope_exhausted"], true);
    assert_eq!(snapshot().await, before);
    assert_eq!(state.versions.lock().unwrap().len(), 2);
    assert!(state.deletes.lock().unwrap().is_empty());
    state.denied_retention.store(true, Ordering::SeqCst);
    requests.release(&lease).await.unwrap();
    let invoke = |subject: &str| {
        let binary = std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into());
        let mut command = tokio::process::Command::new(binary);
        command
            .args([
                "user-deletions",
                "media",
                "--pubkey",
                subject,
                "--delete",
                "--offline",
                "--request-id",
                &request.id.to_string(),
                "--max-source-rows",
                "10000000",
                "--scan-timeout-secs",
                "900",
            ])
            .env("DATABASE_URL", &url)
            .env_remove("BUZZ_MEDIA_SOURCE_DATABASE_URL")
            .env("BUZZ_S3_ENDPOINT", format!("http://{address}"))
            .env("BUZZ_S3_BUCKET", "fixture")
            .env("BUZZ_S3_ACCESS_KEY", "fixture-key")
            .env("BUZZ_S3_SECRET_KEY", "fixture-secret")
            .env("BUZZ_S3_ADDRESSING_STYLE", "path");
        command
    };
    let wrong_subject = invoke(&nostr::Keys::generate().public_key().to_hex())
        .output()
        .await
        .unwrap();
    assert!(!wrong_subject.status.success());
    assert!(String::from_utf8_lossy(&wrong_subject.stderr).contains("account request for --pubkey"));
    let separate_source = invoke(&human)
        .env(
            "BUZZ_MEDIA_SOURCE_DATABASE_URL",
            "postgres://reader@example.org/other",
        )
        .output()
        .await
        .unwrap();
    assert!(!separate_source.status.success());
    assert!(String::from_utf8_lossy(&separate_source.stderr).contains("identical to DATABASE_URL"));
    assert!(state.deletes.lock().unwrap().is_empty());
    let first = invoke(&human).output().await.unwrap();
    assert!(
        !first.status.success(),
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
    // Existing serving reservations still reject conflicting writes even when
    // a URL encodes each hash character. Inventory does not scan these links.
    let embedded = format!(
        r#"{{"url":"/media/{}.jpg"}}"#,
        hash.chars()
            .map(|c| format!("\\u{:04x}", c as u32))
            .collect::<String>()
    );
    let encoded = nostr::EventBuilder::new(nostr::Kind::TextNote, embedded)
        .sign_with_keys(&outsider)
        .unwrap();
    assert!(db.insert_event(other, &encoded, None).await.is_err());
    state.denied_retention.store(false, Ordering::SeqCst);
    let second = invoke(&human).output().await.unwrap();
    assert!(
        !second.status.success(),
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
    let last = invoke(&human).output().await.unwrap();
    assert!(
        last.status.success(),
        "{}",
        String::from_utf8_lossy(&last.stderr)
    );
    let again = invoke(&human).output().await.unwrap();
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&again.stdout).unwrap()["complete"],
        true
    );
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
    let copied_still_present: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(other.as_uuid())
            .bind(copied.id.as_bytes().as_slice())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(
        copied_still_present,
        "media deletion preserves the other person's now-broken link"
    );
    server.abort();
}
