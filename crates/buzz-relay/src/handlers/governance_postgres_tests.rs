//! Real ingress controls for signed governance and evidence.
use super::group_state_postgres_tests::{community, create_channel, state};
use super::ingest::{ingest_event, HttpAuthMethod, IngestAuth};
use buzz_auth::Scope;
use buzz_core::governance::{Evidence, EvidenceBody, RepoCoordinate};
use nostr::{EventBuilder, Keys, Kind, Tag};

fn auth(keys: &Keys) -> IngestAuth {
    IngestAuth::Http {
        pubkey: keys.public_key(),
        scopes: vec![Scope::ReposWrite],
        auth_method: HttpAuthMethod::Nip98,
    }
}

async fn fixture() -> (
    std::sync::Arc<crate::state::AppState>,
    buzz_core::TenantContext,
    Keys,
    Evidence,
) {
    fixture_on(state().await).await
}
async fn fixture_on(
    state: std::sync::Arc<crate::state::AppState>,
) -> (
    std::sync::Arc<crate::state::AppState>,
    buzz_core::TenantContext,
    Keys,
    Evidence,
) {
    let tenant = community(&state, "git-evidence").await;
    let keys = Keys::generate();
    let channel = create_channel(&state, &tenant, &keys, &[]).await;
    let repo = RepoCoordinate {
        owner: keys.public_key().to_hex(),
        name: format!("evidence-{}", uuid::Uuid::new_v4().simple()),
    };
    buzz_db::git_repo::reserve_repo_name(
        state.db.pool(),
        tenant.community(),
        &repo.name,
        &repo.owner,
    )
    .await
    .unwrap();
    let announcement = EventBuilder::new(Kind::Custom(30617), "")
        .tags([
            Tag::parse(["d", &repo.name]).unwrap(),
            Tag::parse(["buzz-channel", &channel.to_string()]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    state
        .db
        .insert_event(tenant.community(), &announcement, Some(channel))
        .await
        .unwrap();
    let subject = Evidence {
        community: *tenant.community().as_uuid(),
        repo: repo.to_string(),
        git_ref: "refs/heads/main".into(),
        head: "a".repeat(40),
        base: "b".repeat(40),
        body: EvidenceBody::Approval { approved: true },
    };
    (state, tenant, keys, subject)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn evidence_preserves_history_and_replays_without_another_row() {
    let (state, tenant, keys, subject) = fixture().await;
    let approval = subject.builder().unwrap().sign_with_keys(&keys).unwrap();
    for _ in 0..2 {
        ingest_event(&state, &tenant, approval.clone(), auth(&keys))
            .await
            .unwrap();
    }
    let withdrawal = Evidence {
        body: EvidenceBody::Approval { approved: false },
        ..subject.clone()
    }
    .builder()
    .unwrap()
    .sign_with_keys(&keys)
    .unwrap();
    ingest_event(&state, &tenant, withdrawal, auth(&keys))
        .await
        .unwrap();
    let other = Evidence {
        git_ref: "refs/heads/other".into(),
        head: "c".repeat(40),
        body: EvidenceBody::Check {
            name: "ci".into(),
            passed: false,
        },
        ..subject.clone()
    }
    .builder()
    .unwrap()
    .sign_with_keys(&keys)
    .unwrap();
    ingest_event(&state, &tenant, other, auth(&keys))
        .await
        .unwrap();
    let mut tx = state
        .db
        .begin_event_write_transaction(tenant.community())
        .await
        .unwrap();
    let repo = subject.repo.parse().unwrap();
    buzz_db::git_land::hold(&mut tx, &repo)
        .await
        .unwrap()
        .unwrap();
    let rows = buzz_db::git_evidence::read_in_transaction(
        &mut tx,
        &repo,
        &["a".repeat(40), "c".repeat(40)],
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    assert_eq!(rows[0].evidence, subject);
    assert_eq!(
        rows[1].evidence.body,
        EvidenceBody::Approval { approved: false }
    );
    assert_eq!(rows[2].evidence.git_ref, "refs/heads/other");
    assert!(rows.iter().all(|r| r.actor == keys.public_key().to_hex()));
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn evidence_withdrawal_waits_for_in_flight_land() {
    let (state, tenant, keys, mut subject) = fixture().await;
    subject.body = EvidenceBody::Approval { approved: false };
    let event = subject.builder().unwrap().sign_with_keys(&keys).unwrap();
    let mut land = state
        .db
        .begin_event_write_transaction(tenant.community())
        .await
        .unwrap();
    let repo = subject.repo.parse().unwrap();
    let before = buzz_db::git_land::hold(&mut land, &repo)
        .await
        .unwrap()
        .unwrap();
    let mut writer = Box::pin(ingest_event(&state, &tenant, event, auth(&keys)));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), &mut writer)
            .await
            .is_err(),
        "withdrawal passed the held land fence"
    );
    assert!(buzz_db::git_evidence::read_in_transaction(
        &mut land,
        &repo,
        &["a".repeat(40), "c".repeat(40)]
    )
    .await
    .unwrap()
    .is_empty());
    land.commit().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), writer)
        .await
        .unwrap()
        .unwrap();
    let mut after = state
        .db
        .begin_event_write_transaction(tenant.community())
        .await
        .unwrap();
    assert!(
        buzz_db::git_land::hold(&mut after, &repo)
            .await
            .unwrap()
            .unwrap()
            .version
            > before.version
    );
    assert_eq!(
        buzz_db::git_evidence::read_in_transaction(
            &mut after,
            &repo,
            &["a".repeat(40), "c".repeat(40)]
        )
        .await
        .unwrap()
        .len(),
        1
    );
    after.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn evidence_row_cap_errors_instead_of_hiding_newer_failure() {
    let (state, tenant, keys, subject) = fixture().await;
    // Synthetic history isolates the bounded reader. Real append/signature and
    // replay behavior are exercised above; these rows are not a valid chain.
    sqlx::query("INSERT INTO audit_log (community_id,seq,hash,action,actor_pubkey,detail) SELECT $1,100000+n,decode(md5(n::text),'hex'),'git_evidence',$2,$3 FROM generate_series(1,10001) n")
        .bind(tenant.community().as_uuid()).bind(keys.public_key().to_bytes().to_vec())
        .bind(serde_json::json!({"evidence":subject})).execute(state.db.pool()).await.unwrap();
    let mut tx = state
        .db
        .begin_event_write_transaction(tenant.community())
        .await
        .unwrap();
    let repo = subject.repo.parse().unwrap();
    buzz_db::git_land::hold(&mut tx, &repo)
        .await
        .unwrap()
        .unwrap();
    let error = buzz_db::git_evidence::read_in_transaction(
        &mut tx,
        &repo,
        &["a".repeat(40), "c".repeat(40)],
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("exceeds limit"));
    // Historical evidence for other commits never exhausts a new candidate's budget.
    assert!(
        buzz_db::git_evidence::read_in_transaction(&mut tx, &repo, &["d".repeat(40)])
            .await
            .unwrap()
            .is_empty()
    );
    tx.rollback().await.unwrap();
}

const FLOOR: &str =
    r#"{"version":1,"body":{"type":"floor","rules":[],"labels":{},"max_grant_seconds":3600}}"#;
const RULES: &str = r#"{"version":1,"body":{"type":"rules","rules":[]}}"#;

fn command(
    tenant: &buzz_core::TenantContext,
    keys: &Keys,
    document: &str,
    body: &str,
) -> nostr::Event {
    buzz_core::governance::command_builder(
        *tenant.community().as_uuid(),
        &document.parse().unwrap(),
        &buzz_core::governance::ExpectedHead::Absent,
        body,
    )
    .unwrap()
    .sign_with_keys(keys)
    .unwrap()
}
async fn community_role(
    state: &crate::state::AppState,
    tenant: &buzz_core::TenantContext,
    keys: &Keys,
    role: &str,
) {
    sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,$3) ON CONFLICT (community_id,pubkey) DO UPDATE SET role=EXCLUDED.role")
        .bind(tenant.community().as_uuid()).bind(keys.public_key().to_hex()).bind(role).execute(state.db.pool()).await.unwrap();
}
async fn demote(state: &crate::state::AppState, tenant: &buzz_core::TenantContext, keys: &Keys) {
    sqlx::query("UPDATE channel_members SET role='member' WHERE community_id=$1 AND pubkey=$2")
        .bind(tenant.community().as_uuid())
        .bind(keys.public_key().to_bytes().to_vec())
        .execute(state.db.pool())
        .await
        .unwrap();
}
fn authority_denied(result: Result<super::ingest::IngestResult, super::ingest::IngestError>) {
    assert!(
        matches!(result, Err(super::ingest::IngestError::AuthFailed(ref reason)) if reason.contains("governance authority required")),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn repo_admin_floor() {
    let (state, tenant, keys, _) = fixture().await;
    community_role(&state, &tenant, &keys, "member").await;
    let event = command(&tenant, &keys, "floor", FLOOR);
    authority_denied(ingest_event(&state, &tenant, event.clone(), auth(&keys)).await);
    community_role(&state, &tenant, &keys, "admin").await;
    ingest_event(&state, &tenant, event, auth(&keys))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn repo_admin_labels() {
    let (state, tenant, keys, subject) = fixture().await;
    community_role(&state, &tenant, &keys, "admin").await;
    ingest_event(
        &state,
        &tenant,
        command(&tenant, &keys, "floor", FLOOR),
        auth(&keys),
    )
    .await
    .unwrap();
    community_role(&state, &tenant, &keys, "member").await;
    let event = command(
        &tenant,
        &keys,
        &format!("labels:{}", subject.repo),
        r#"{"version":1,"body":{"type":"labels","values":{}}}"#,
    );
    authority_denied(ingest_event(&state, &tenant, event.clone(), auth(&keys)).await);
    community_role(&state, &tenant, &keys, "admin").await;
    ingest_event(&state, &tenant, event, auth(&keys))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn authority_at_sign() {
    let (state, tenant, keys, subject) = fixture().await;
    let event = command(&tenant, &keys, &format!("rules:{}", subject.repo), RULES);
    demote(&state, &tenant, &keys).await;
    authority_denied(ingest_event(&state, &tenant, event, auth(&keys)).await);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn freshness_window_only() {
    let (state, tenant, keys, subject) = fixture().await;
    let document = format!("rules:{}", subject.repo);
    let first = command(&tenant, &keys, &document, RULES);
    let second = command(&tenant, &keys, &document, &RULES.replace("[]", "[ ]"));
    assert_ne!(first.id, second.id);
    ingest_event(&state, &tenant, first, auth(&keys))
        .await
        .unwrap();
    let result = ingest_event(&state, &tenant, second, auth(&keys)).await;
    assert!(
        matches!(result, Err(super::ingest::IngestError::Rejected(ref reason)) if reason.contains("head changed")),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn command_unbound() {
    let (state, tenant, keys, subject) = fixture().await;
    let event = command(&tenant, &keys, &format!("rules:{}", subject.repo), RULES);
    for name in ["community", "d", "prev"] {
        let altered = EventBuilder::new(event.kind, event.content.clone())
            .tags(
                event
                    .tags
                    .iter()
                    .filter(|tag| tag.as_slice()[0] != name)
                    .cloned(),
            )
            .sign_with_keys(&keys)
            .unwrap();
        assert!(ingest_event(&state, &tenant, altered, auth(&keys))
            .await
            .is_err());
    }
    ingest_event(&state, &tenant, event, auth(&keys))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn unrecorded_governance() {
    let (state, tenant, keys, subject) = fixture().await;
    let document = format!("rules:{}", subject.repo);
    let event = command(&tenant, &keys, &document, RULES);
    ingest_event(&state, &tenant, event.clone(), auth(&keys))
        .await
        .unwrap();
    let detail: serde_json::Value = sqlx::query_scalar("SELECT detail FROM audit_log WHERE community_id=$1 AND action='governance_changed' AND object_id=$2")
        .bind(tenant.community().as_uuid()).bind(event.id.to_hex()).fetch_one(state.db.pool()).await.unwrap();
    assert_eq!(detail["document"], document);
    assert_eq!(detail["command_id"], event.id.to_hex());
    assert_eq!(
        detail["document_body"],
        serde_json::from_str::<serde_json::Value>(RULES).unwrap()
    );
    let head = state
        .db
        .governance_head(
            tenant.community(),
            &document.parse().unwrap(),
            keys.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail["state"]["id"], head.id.to_hex());
}

#[test]
fn head_is_not_a_document() {
    // The default branch changes only through the default-branch land, which
    // requires a channel admin and receipts every refusal.
    let repo = format!("30617:{}:r", "a".repeat(64));
    assert!(format!("head:{repo}")
        .parse::<buzz_core::governance::DocumentId>()
        .is_err());
}

fn announce(keys: &Keys, name: &str, tag: Option<&str>, at: u64) -> nostr::Event {
    let mut tags = vec![Tag::parse(["d", name]).unwrap()];
    if let Some(tag) = tag {
        tags.push(Tag::parse(["buzz-channel", tag]).unwrap());
    }
    EventBuilder::new(Kind::Custom(30617), "")
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(at))
        .sign_with_keys(keys)
        .unwrap()
}
async fn pin(
    state: &crate::state::AppState,
    tenant: &buzz_core::TenantContext,
    repo: &RepoCoordinate,
) -> Option<uuid::Uuid> {
    sqlx::query_scalar("SELECT channel_id FROM git_repo_names WHERE community_id=$1 AND repo_id=$2")
        .bind(tenant.community().as_uuid())
        .bind(&repo.name)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn announcer_rebinds() {
    let (state, tenant, keys, subject) = fixture().await;
    let repo: RepoCoordinate = subject.repo.parse().unwrap();
    let bind = |next: uuid::Uuid| {
        let body =
            serde_json::json!({"version":1,"body":{"type":"bind","channel":next}}).to_string();
        command(&tenant, &keys, &format!("bind:{}", subject.repo), &body)
    };
    // An authorized read resolves and commits the pin from the announcement.
    let id: buzz_core::governance::DocumentId = format!("bind:{}", subject.repo).parse().unwrap();
    state
        .db
        .governance_head(tenant.community(), &id, keys.public_key().as_bytes(), None)
        .await
        .unwrap();
    let bound = pin(&state, &tenant, &repo)
        .await
        .expect("read pinned the repo");
    // Admin of the bound channel only: the target channel's admin must agree.
    let foreign = create_channel(&state, &tenant, &Keys::generate(), &[]).await;
    authority_denied(ingest_event(&state, &tenant, bind(foreign), auth(&keys)).await);
    assert_eq!(pin(&state, &tenant, &repo).await, Some(bound));
    // Admin of both: the pin moves in the command's own transaction.
    let next = create_channel(&state, &tenant, &keys, &[]).await;
    ingest_event(&state, &tenant, bind(next), auth(&keys))
        .await
        .unwrap();
    assert_eq!(pin(&state, &tenant, &repo).await, Some(next));
    assert_ne!(bound, next);
    // Demoted to member everywhere: announcing the repo is not authority.
    demote(&state, &tenant, &keys).await;
    let back = command(
        &tenant,
        &keys,
        &format!("bind:{}", subject.repo),
        &serde_json::json!({"version":1,"body":{"type":"bind","channel":bound}}).to_string(),
    );
    let refused = ingest_event(&state, &tenant, back, auth(&keys)).await;
    assert!(refused.is_err(), "a member cannot rebind");
    assert_eq!(pin(&state, &tenant, &repo).await, Some(next));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn announcement_must_name_the_pin() {
    let (state, tenant, keys, subject) = fixture().await;
    let repo: RepoCoordinate = subject.repo.parse().unwrap();
    let pinned = state
        .db
        .git_repo_channel(tenant.community(), &repo, None)
        .await
        .unwrap();
    // The fixture's announcement seeds nothing until something resolves it.
    assert_eq!(pinned, None);
    let channel = create_channel(&state, &tenant, &keys, &[]).await;
    let other = create_channel(&state, &tenant, &keys, &[]).await;
    let now = nostr::Timestamp::now().as_secs();
    // Unpinned: any announcement is accepted, and the first valid tag pins.
    ingest_event(
        &state,
        &tenant,
        announce(&keys, &repo.name, Some(&channel.to_string()), now + 5),
        auth(&keys),
    )
    .await
    .unwrap();
    let pinned = pin(&state, &tenant, &repo).await;
    for (tag, why) in [
        (Some(other.to_string()), "another channel"),
        (None, "no binding"),
        (Some("not-a-uuid".to_string()), "a malformed binding"),
    ] {
        let event = announce(&keys, &repo.name, tag.as_deref(), now + 6);
        let refused = ingest_event(&state, &tenant, event, auth(&keys)).await;
        assert!(
            matches!(refused, Err(super::ingest::IngestError::Rejected(ref m))
                if m.contains(&pinned.unwrap().to_string())),
            "{why}: {:?}",
            refused.map(|r| r.message)
        );
    }
    assert_eq!(pin(&state, &tenant, &repo).await, pinned);
    let same = announce(
        &keys,
        &repo.name,
        Some(&pinned.unwrap().to_string()),
        now + 7,
    );
    ingest_event(&state, &tenant, same, auth(&keys))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn channel_tags_protect() {
    let (state, tenant, keys, subject) = fixture().await;
    community_role(&state, &tenant, &keys, "admin").await;
    let floor = r#"{"version":1,"body":{"type":"floor","rules":[],"labels":{"tier":{"values":["high","low"],"default":"high"}},"max_grant_seconds":3600}}"#;
    ingest_event(
        &state,
        &tenant,
        command(&tenant, &keys, "floor", floor),
        auth(&keys),
    )
    .await
    .unwrap();
    let document = format!("labels:{}", subject.repo);
    let labels = r#"{"version":1,"body":{"type":"labels","values":{"tier":"high"}}}"#;
    ingest_event(
        &state,
        &tenant,
        command(&tenant, &keys, &document, labels),
        auth(&keys),
    )
    .await
    .unwrap();
    let id = document.parse().unwrap();
    let before = state
        .db
        .governance_head(tenant.community(), &id, keys.public_key().as_bytes(), None)
        .await
        .unwrap()
        .unwrap();
    let mut tx = state
        .db
        .begin_event_write_transaction(tenant.community())
        .await
        .unwrap();
    let channel = buzz_db::governance::repo_channel(&mut tx, &subject.repo.parse().unwrap())
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let edit = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["t", "tier:low"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    ingest_event(
        &state,
        &tenant,
        edit,
        super::group_state_postgres_tests::http_auth(&keys),
    )
    .await
    .unwrap();
    let after = state
        .db
        .governance_head(tenant.community(), &id, keys.public_key().as_bytes(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        before, after,
        "channel metadata changed protected governance labels"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn receipts_page_real_actions_and_recheck_current_access() {
    let (state, tenant, keys, subject) = fixture().await;
    let repo = subject.repo.parse().unwrap();
    for action in [
        buzz_audit::AuditAction::GitLanded,
        buzz_audit::AuditAction::GitRefused,
        buzz_audit::AuditAction::GitRefused,
    ] {
        let mut tx = state
            .db
            .begin_event_write_transaction(tenant.community())
            .await
            .unwrap();
        buzz_audit::AuditService::log_in_transaction(
            &mut tx,
            buzz_audit::NewAuditEntry {
                community_id: tenant.community(),
                action,
                actor_pubkey: Some(keys.public_key().to_bytes().to_vec()),
                object_id: Some(subject.repo.clone()),
                detail: serde_json::json!({"stage":"hook","reasons":["fixture"]}),
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let first = state
        .db
        .git_receipts(
            tenant.community(),
            &repo,
            keys.public_key().as_bytes(),
            None,
            None,
            2,
        )
        .await
        .unwrap();
    assert_eq!(first.receipts.len(), 2);
    assert_eq!(first.receipts[0].action, "git_refused");
    let second = state
        .db
        .git_receipts(
            tenant.community(),
            &repo,
            keys.public_key().as_bytes(),
            None,
            first.next_before,
            2,
        )
        .await
        .unwrap();
    assert_eq!(second.receipts.len(), 1);
    assert_eq!(second.receipts[0].action, "git_landed");
    assert!(second.next_before.is_none());
    assert!(second.receipts[0].seq < first.receipts[1].seq);
    let outsider = Keys::generate();
    assert!(matches!(
        state
            .db
            .git_receipts(
                tenant.community(),
                &repo,
                outsider.public_key().as_bytes(),
                None,
                None,
                2
            )
            .await,
        Err(buzz_db::DbError::AccessDenied(_))
    ));
    sqlx::query("UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND pubkey=$2")
        .bind(tenant.community().as_uuid())
        .bind(keys.public_key().to_bytes().to_vec())
        .execute(state.db.pool())
        .await
        .unwrap();
    assert!(matches!(
        state
            .db
            .git_receipts(
                tenant.community(),
                &repo,
                keys.public_key().as_bytes(),
                None,
                None,
                2
            )
            .await,
        Err(buzz_db::DbError::AccessDenied(_))
    ));
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis"]
async fn receipts_signed_http_query_is_private_and_strict() {
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    let (state, tenant, keys, subject) = fixture_on(
        crate::api::bridge::postgres_tests::bridge_handler_test_state()
            .await
            .unwrap(),
    )
    .await;
    community_role(&state, &tenant, &keys, "member").await;
    let outsider = Keys::generate();
    community_role(&state, &tenant, &outsider, "member").await;
    for (actor, filter, status) in [
        (
            &keys,
            serde_json::json!([{"git_receipts":1,"repo":subject.repo}]),
            200,
        ),
        (
            &outsider,
            serde_json::json!([{"git_receipts":1,"repo":subject.repo}]),
            403,
        ),
        (
            &keys,
            serde_json::json!([{"git_receipts":1,"repo":subject.repo,"limit":201}]),
            400,
        ),
    ] {
        let body = serde_json::to_vec(&filter).unwrap();
        let url =
            crate::api::bridge::nip98_expected_url(&state.config.relay_url, &tenant, "/query");
        let proof = EventBuilder::new(Kind::Custom(27235), "")
            .tags([
                Tag::parse(["u", &url]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
                Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
                Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
            ])
            .sign_with_keys(actor)
            .unwrap();
        let authorization = format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&proof).unwrap())
        );
        let response = crate::router::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/query")
                    .header("host", tenant.host())
                    .header("authorization", authorization)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let actual = response.status().as_u16();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(actual, status, "{}", String::from_utf8_lossy(&bytes));
        if status == 200 {
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["git_receipts"], 1);
            assert_eq!(value["repo"], subject.repo);
            assert_eq!(value["receipts"], serde_json::json!([]));
            assert!(value.get("next_before").unwrap().is_null());
        }
    }
    assert_eq!(reads(&state, &tenant, &subject.repo).await, 0, "ungoverned");
}

/// Recorded reads of `repo`, by any key.
async fn reads(
    state: &crate::state::AppState,
    tenant: &buzz_core::TenantContext,
    repo: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE community_id=$1 AND action='git_read' \
         AND object_id=$2 AND detail->>'door'='state'",
    )
    .bind(tenant.community().as_uuid())
    .bind(repo)
    .fetch_one(state.db.pool())
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn receipt_read_persists_pin_for_later_reads_and_evidence() {
    let (state, tenant, keys, subject) = fixture().await;
    let repo: RepoCoordinate = subject.repo.parse().unwrap();
    let pin_sql = "SELECT channel_id FROM git_repo_names WHERE community_id=$1 AND repo_id=$2";
    let before: Option<uuid::Uuid> = sqlx::query_scalar(pin_sql)
        .bind(tenant.community().as_uuid())
        .bind(&repo.name)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(before.is_none(), "legacy fixture starts unpinned");
    state
        .db
        .git_receipts(
            tenant.community(),
            &repo,
            keys.public_key().as_bytes(),
            None,
            None,
            30,
        )
        .await
        .unwrap();
    let pinned: Option<uuid::Uuid> = sqlx::query_scalar(pin_sql)
        .bind(tenant.community().as_uuid())
        .bind(&repo.name)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(
        pinned.is_some(),
        "receipt read must commit the initialized pin"
    );

    let outsider = Keys::generate();
    let other_channel = create_channel(&state, &tenant, &outsider, &[]).await;
    let replacement = EventBuilder::new(Kind::Custom(30617), "")
        .tags([
            Tag::parse(["d", &repo.name]).unwrap(),
            Tag::parse(["buzz-channel", &other_channel.to_string()]).unwrap(),
        ])
        .custom_created_at(nostr::Timestamp::from(
            nostr::Timestamp::now().as_secs() + 1,
        ))
        .sign_with_keys(&keys)
        .unwrap();
    state
        .db
        .insert_event(tenant.community(), &replacement, Some(other_channel))
        .await
        .unwrap();
    state
        .db
        .git_receipts(
            tenant.community(),
            &repo,
            keys.public_key().as_bytes(),
            None,
            None,
            30,
        )
        .await
        .expect("original channel retains receipt access after reannouncement");
    assert!(matches!(
        state
            .db
            .git_receipts(
                tenant.community(),
                &repo,
                outsider.public_key().as_bytes(),
                None,
                None,
                30
            )
            .await,
        Err(buzz_db::DbError::AccessDenied(_))
    ));
    let approval = subject.builder().unwrap().sign_with_keys(&keys).unwrap();
    ingest_event(&state, &tenant, approval, auth(&keys))
        .await
        .expect("original channel retains evidence access after reannouncement");
    let outsider_approval = subject
        .builder()
        .unwrap()
        .sign_with_keys(&outsider)
        .unwrap();
    assert!(
        ingest_event(&state, &tenant, outsider_approval, auth(&outsider))
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis"]
async fn effective_signed_http_query_reports_sources_and_defaults() {
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    let (state, tenant, keys, subject) = fixture_on(
        crate::api::bridge::postgres_tests::bridge_handler_test_state()
            .await
            .unwrap(),
    )
    .await;
    community_role(&state, &tenant, &keys, "member").await;
    community_role(&state, &tenant, &keys, "admin").await;
    let floor = command(
        &tenant,
        &keys,
        "floor",
        r#"{"version":1,"body":{"type":"floor","rules":[],"labels":{"risk":{"values":["high","low"],"default":"high"}},"max_grant_seconds":3600}}"#,
    );
    ingest_event(&state, &tenant, floor, auth(&keys))
        .await
        .unwrap();
    let outsider = Keys::generate();
    community_role(&state, &tenant, &outsider, "member").await;
    for (actor, filter, status) in [
        (
            &keys,
            serde_json::json!([{"git_effective":1,"repo":subject.repo}]),
            200,
        ),
        (
            &outsider,
            serde_json::json!([{"git_effective":1,"repo":subject.repo}]),
            403,
        ),
        (
            &keys,
            serde_json::json!([{"git_effective":1,"repo":subject.repo,"limit":201}]),
            400,
        ),
        (
            &keys,
            serde_json::json!([{"git_receipts":1,"repo":subject.repo}]),
            200,
        ),
        (
            &keys,
            serde_json::json!([{"governance_head":1,"document":format!("rules:{}", subject.repo)}]),
            200,
        ),
        (
            &keys,
            serde_json::json!([{"governance_head":1,"document":"floor"}]),
            200,
        ),
        (
            &keys,
            serde_json::json!([{"git_effective":1,"repo":subject.repo}]),
            503,
        ),
    ] {
        if status == 503 {
            sqlx::query("UPDATE events SET tags=tags || $3::jsonb WHERE community_id=$1 AND kind=30617 AND pubkey=$2")
                .bind(tenant.community().as_uuid()).bind(keys.public_key().to_bytes().to_vec())
                .bind(serde_json::json!([["buzz-protect","refs/heads/*","future-unknown-rule"]]))
                .execute(state.db.pool()).await.unwrap();
        }
        let body = serde_json::to_vec(&filter).unwrap();
        let url =
            crate::api::bridge::nip98_expected_url(&state.config.relay_url, &tenant, "/query");
        let proof = EventBuilder::new(Kind::Custom(27235), "")
            .tags([
                Tag::parse(["u", &url]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
                Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
                Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
            ])
            .sign_with_keys(actor)
            .unwrap();
        let authorization = format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&proof).unwrap())
        );
        let response = crate::router::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/query")
                    .header("host", tenant.host())
                    .header("authorization", authorization)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let actual = response.status().as_u16();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(actual, status, "{}", String::from_utf8_lossy(&bytes));
        if status == 200 && filter[0].get("git_effective").is_some() {
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["git_effective"], 1);
            assert_eq!(value["repo"], subject.repo);
            assert_eq!(value["evaluation"], "legacy_and_governance");
            assert!(value["floor"]["head"].is_string());
            assert!(value["labels"]["head"].is_null());
            assert_eq!(value["labels"]["source"], "floor_defaults");
            assert_eq!(value["labels"]["values"]["risk"], "high");
            assert_eq!(
                value["legacy"]["default_min_roles"][0]["fast_forward"],
                "member"
            );
            assert_eq!(
                value["legacy"]["default_min_roles"][1]["fast_forward"],
                "admin"
            );
        }
    }
    // Governed by the floor: each served read of the repo's state is
    // recorded (effective, receipts, its rules); the refused, the malformed,
    // the unavailable and the community document's are not.
    assert_eq!(reads(&state, &tenant, &subject.repo).await, 3);
}

/// A community document is readable by every key it governs: a relay member
/// or a key with a role in a channel of the community, guest or archived
/// channel included. A banned key, a removed member, an agent with no role of
/// its own, an agent whose owner is banned, and a key whose only role is in a
/// DM, a deleted channel or another community are refused.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn community_documents_are_readable_by_every_key_they_govern() {
    use buzz_core::governance::DocumentId;
    let (state, tenant, keys, _) = fixture().await;
    community_role(&state, &tenant, &keys, "admin").await;
    ingest_event(
        &state,
        &tenant,
        command(&tenant, &keys, "floor", FLOOR),
        auth(&keys),
    )
    .await
    .unwrap();
    let channel = create_channel(&state, &tenant, &keys, &[]).await;
    let archived = create_channel(&state, &tenant, &keys, &[]).await;
    let pool = state.db.pool();
    let cid = *tenant.community().as_uuid();
    let role = |key: &Keys, ch: uuid::Uuid, role: &'static str| {
        let key = key.public_key().to_bytes().to_vec();
        async move {
            sqlx::query("INSERT INTO channel_members (community_id,channel_id,pubkey,role) VALUES ($1,$2,$3,$4::member_role)")
                .bind(cid).bind(ch).bind(key).bind(role).execute(pool).await.unwrap();
        }
    };
    let ban = |key: &Keys| {
        let key = key.public_key().to_bytes().to_vec();
        async move {
            sqlx::query("INSERT INTO community_bans (community_id,pubkey,banned,actor_pubkey) VALUES ($1,$2,true,$2)")
                .bind(cid).bind(key).execute(pool).await.unwrap();
        }
    };
    let agent_of = |agent: &Keys, owner: &Keys| {
        let (agent, owner) = (
            agent.public_key().to_bytes().to_vec(),
            owner.public_key().to_bytes().to_vec(),
        );
        async move {
            sqlx::query("INSERT INTO users (community_id,pubkey,agent_owner_pubkey) VALUES ($1,$2,$3) ON CONFLICT (community_id,pubkey) DO UPDATE SET agent_owner_pubkey=EXCLUDED.agent_owner_pubkey")
                .bind(cid).bind(agent).bind(owner).execute(pool).await.unwrap();
        }
    };
    let guest = Keys::generate();
    role(&guest, channel, "guest").await;
    let in_archived = Keys::generate();
    role(&in_archived, archived, "member").await;
    sqlx::query("UPDATE channels SET archived_at=NOW() WHERE community_id=$1 AND id=$2")
        .bind(cid)
        .bind(archived)
        .execute(pool)
        .await
        .unwrap();
    let stranger = Keys::generate();
    let in_dm = Keys::generate();
    let dm = create_channel(&state, &tenant, &keys, &[]).await;
    role(&in_dm, dm, "member").await;
    sqlx::query("UPDATE channels SET channel_type='dm' WHERE community_id=$1 AND id=$2")
        .bind(cid)
        .bind(dm)
        .execute(pool)
        .await
        .unwrap();
    let in_deleted = Keys::generate();
    let deleted = create_channel(&state, &tenant, &keys, &[]).await;
    role(&in_deleted, deleted, "member").await;
    sqlx::query("UPDATE channels SET deleted_at=NOW() WHERE community_id=$1 AND id=$2")
        .bind(cid)
        .bind(deleted)
        .execute(pool)
        .await
        .unwrap();
    let elsewhere = Keys::generate();
    let other = community(&state, "git-evidence-other").await;
    let there = create_channel(&state, &other, &keys, &[]).await;
    sqlx::query("INSERT INTO channel_members (community_id,channel_id,pubkey,role) VALUES ($1,$2,$3,'member')")
        .bind(other.community().as_uuid())
        .bind(there)
        .bind(elsewhere.public_key().to_bytes().to_vec())
        .execute(pool)
        .await
        .unwrap();
    let banned = Keys::generate();
    role(&banned, channel, "member").await;
    ban(&banned).await;
    let removed = Keys::generate();
    role(&removed, channel, "member").await;
    sqlx::query("UPDATE channel_members SET removed_at=NOW() WHERE community_id=$1 AND pubkey=$2")
        .bind(cid)
        .bind(removed.public_key().to_bytes().to_vec())
        .execute(pool)
        .await
        .unwrap();
    let roleless_agent = Keys::generate();
    agent_of(&roleless_agent, &keys).await;
    let banned_owner = Keys::generate();
    role(&banned_owner, channel, "member").await;
    ban(&banned_owner).await;
    let agent_of_banned = Keys::generate();
    role(&agent_of_banned, channel, "member").await;
    agent_of(&agent_of_banned, &banned_owner).await;
    for (who, key, readable) in [
        ("relay member", &keys, true),
        ("guest", &guest, true),
        ("role in an archived channel", &in_archived, true),
        ("stranger", &stranger, false),
        ("role only in a DM", &in_dm, false),
        ("role only in a deleted channel", &in_deleted, false),
        ("role only in another community", &elsewhere, false),
        ("banned with a role", &banned, false),
        ("removed", &removed, false),
        ("agent with no role of its own", &roleless_agent, false),
        ("agent of a banned owner", &agent_of_banned, false),
    ] {
        for id in [DocumentId::Floor, DocumentId::Trusted] {
            let read = state
                .db
                .governance_head(tenant.community(), &id, key.public_key().as_bytes(), None)
                .await;
            match read {
                Ok(head) => {
                    assert!(readable, "{who} read {id}");
                    assert_eq!(head.is_some(), id == DocumentId::Floor, "{who} {id}");
                }
                Err(buzz_db::DbError::AccessDenied(_)) => assert!(!readable, "{who} refused {id}"),
                Err(error) => panic!("{who} {id}: {error}"),
            }
        }
    }
}

/// NIP-IA archival is visibility, not access (NIP-IA.md:436), so an archived
/// repo admin still governs, as it still pushes.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_archived_repo_admin_still_governs() {
    let (state, tenant, keys, subject) = fixture().await;
    let hex = keys.public_key().to_hex();
    assert!(state
        .db
        .archive(
            tenant.community(),
            &hex,
            "self",
            &hex,
            None,
            None,
            &"c".repeat(64)
        )
        .await
        .unwrap());
    let event = command(&tenant, &keys, &format!("rules:{}", subject.repo), RULES);
    ingest_event(&state, &tenant, event, auth(&keys))
        .await
        .expect("an archived repo admin governs");
}
