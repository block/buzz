use buzz_core::CommunityId;
use buzz_db::{
    workflow::{ApprovalStatus, CreateApprovalParams, RunStatus},
    Db, DbConfig,
};
use sqlx::PgPool;
use uuid::Uuid;

async fn claim(pool: &PgPool, tenant: CommunityId, token: &[u8], signer: &[u8]) -> bool {
    buzz_db::workflow::claim_approval_resume(
        &mut pool.acquire().await.unwrap(),
        tenant,
        token,
        signer,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn approval_claim_is_once_only_tenant_bound_and_recoverable() {
    let db = Db::new(&DbConfig {
        database_url: std::env::var("DATABASE_URL").unwrap(),
        ..Default::default()
    })
    .await
    .unwrap();
    buzz_db::migration::run_migrations(db.pool()).await.unwrap();
    let present: bool = sqlx::query_scalar(
        "SELECT to_regprocedure('platform_claim_approval(uuid,bytea,bytea)') IS NOT NULL",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert!(present, "durable approval resume claim is missing");
    let owner = [42u8; 32];
    let tenant = match db
        .create_community_with_owner(&format!("s38-{}.test", Uuid::new_v4()), &hex::encode(owner))
        .await
        .unwrap()
    {
        buzz_db::CreateCommunityWithOwnerResult::Created(row) => row.id,
        other => panic!("{other:?}"),
    };
    db.ensure_user(tenant, &owner).await.unwrap();
    let wf = db
        .create_workflow(tenant, None, &owner, "claim", "{}", &[0; 32])
        .await
        .unwrap();
    let run = db
        .create_workflow_run(tenant, wf, None, None)
        .await
        .unwrap();
    db.update_workflow_run(
        tenant,
        run,
        RunStatus::Running,
        0,
        &serde_json::json!([]),
        None,
    )
    .await
    .unwrap();
    buzz_db::workflow::suspend_for_approval(
        db.pool(),
        CreateApprovalParams {
            community_id: tenant,
            token: "fictitious-s38",
            workflow_id: wf,
            run_id: run,
            step_id: "gate",
            step_index: 0,
            approver_spec: &hex::encode(owner),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        },
        &serde_json::json!([]),
    )
    .await
    .unwrap();
    let approval = db.get_approval(tenant, "fictitious-s38").await.unwrap();
    assert!(
        !claim(
            db.pool(),
            CommunityId::from_uuid(Uuid::new_v4()),
            &approval.token,
            &owner
        )
        .await
    );
    assert!(!claim(db.pool(), tenant, &approval.token, &[43; 32]).await);
    let (a, b) = tokio::join!(
        claim(db.pool(), tenant, &approval.token, &owner),
        claim(db.pool(), tenant, &approval.token, &owner)
    );
    assert_ne!(a, b, "exactly one durable claim wins");
    assert!(!claim(db.pool(), tenant, &approval.token, &owner).await);
    assert_eq!(
        db.get_approval(tenant, "fictitious-s38")
            .await
            .unwrap()
            .status,
        ApprovalStatus::Granted
    );
    // A new connection observes the interrupted continuation, without executing it again.
    let reopened = PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let state: String = sqlx::query_scalar(
        "SELECT state FROM platform_approval_resume_claims WHERE community_id=$1 AND token=$2",
    )
    .bind(tenant.as_uuid())
    .bind(&approval.token)
    .fetch_one(&reopened)
    .await
    .unwrap();
    assert_eq!(state, "recovery_required");
    sqlx::query("DELETE FROM platform_approval_resume_claims WHERE community_id=$1 AND token=$2")
        .bind(tenant.as_uuid())
        .bind(&approval.token)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE workflow_approvals SET status='pending', expires_at=now()-interval '1 second' WHERE community_id=$1 AND token=$2")
        .bind(tenant.as_uuid()).bind(&approval.token).execute(db.pool()).await.unwrap();
    assert!(!claim(db.pool(), tenant, &approval.token, &owner).await);
}
