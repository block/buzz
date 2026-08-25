use crate::{ActionEngine, ActionError, ActionState};
use business_analytics::{ACCEPTANCE_FINANCE_USER, ACCEPTANCE_SALES_USER};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};

#[derive(Clone)]
pub struct PgActionStore {
    pool: PgPool,
}

impl PgActionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        sqlx::migrate!("../business-auth-gateway/migrations")
            .run(&self.pool)
            .await
    }

    pub async fn load(&self) -> Result<Option<ActionState>, ActionError> {
        let value = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT state FROM business_action_state WHERE singleton=true",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ActionError::InvalidRequest)?;
        value
            .map(|value| serde_json::from_value(value).map_err(|_| ActionError::InvalidRequest))
            .transpose()
    }

    pub async fn save(&self, engine: &ActionEngine) -> Result<(), ActionError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("SELECT pg_advisory_xact_lock(651190061)")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        seed_acceptance_users(&mut tx).await?;
        let state = serde_json::to_value(&engine.state).map_err(|_| ActionError::InvalidRequest)?;
        sqlx::query(
            "INSERT INTO business_action_state(singleton,state) VALUES(true,$1)
             ON CONFLICT(singleton) DO UPDATE SET state=EXCLUDED.state,
             version=business_action_state.version+1,updated_at=now()",
        )
        .bind(state)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        save_catalog(&mut tx, engine).await?;
        save_findings(&mut tx, engine).await?;
        save_proposals(&mut tx, engine).await?;
        save_work(&mut tx, engine).await?;
        save_approvals(&mut tx, engine).await?;
        save_audits(&mut tx, engine).await?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}

fn db(_: sqlx::Error) -> ActionError {
    ActionError::PersistenceUnavailable
}

async fn seed_acceptance_users(tx: &mut Transaction<'_, Postgres>) -> Result<(), ActionError> {
    for (id, subject, name) in [
        (
            ACCEPTANCE_FINANCE_USER,
            "acceptance-finance",
            "脱敏验收财务用户",
        ),
        (
            ACCEPTANCE_SALES_USER,
            "acceptance-sales",
            "脱敏验收销售用户",
        ),
    ] {
        sqlx::query(
            "INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,display_name,status)
             VALUES($1,'https://acceptance.invalid',$2,$3,'active')
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(id)
        .bind(subject)
        .bind(name)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

async fn save_catalog(
    tx: &mut Transaction<'_, Postgres>,
    engine: &ActionEngine,
) -> Result<(), ActionError> {
    let entries =
        serde_json::to_value(engine.catalog()).map_err(|_| ActionError::InvalidRequest)?;
    let hash = hex::encode(Sha256::digest(
        serde_json::to_vec(&entries).map_err(|_| ActionError::InvalidRequest)?,
    ));
    let first = engine
        .catalog()
        .first()
        .ok_or(ActionError::InvalidRequest)?;
    sqlx::query(
        "INSERT INTO business_action_catalog_versions(version,config_hash,effective_from,effective_to,enabled,payload)
         VALUES($1,$2,$3,$4,true,$5)
         ON CONFLICT(version) DO UPDATE SET config_hash=EXCLUDED.config_hash,
         effective_from=EXCLUDED.effective_from,effective_to=EXCLUDED.effective_to,
         enabled=EXCLUDED.enabled,payload=EXCLUDED.payload",
    )
    .bind(&first.version)
    .bind(hash)
    .bind(first.effective_from)
    .bind(first.effective_to)
    .bind(entries)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}

async fn save_findings(
    tx: &mut Transaction<'_, Postgres>,
    engine: &ActionEngine,
) -> Result<(), ActionError> {
    for finding in engine.state.findings.values() {
        sqlx::query(
            "INSERT INTO business_anomaly_findings(
               id,finding_key,legal_entity_id,scope_hash,rule_set_version,condition_status,
               review_status,occurrence_count,first_seen_at,last_seen_at,cleared_at,resolved_at,
               dismissed_at,review_after,finding_snapshot_hash,version,trace_id,payload)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
             ON CONFLICT(id) DO UPDATE SET condition_status=EXCLUDED.condition_status,
               review_status=EXCLUDED.review_status,occurrence_count=EXCLUDED.occurrence_count,
               last_seen_at=EXCLUDED.last_seen_at,cleared_at=EXCLUDED.cleared_at,
               resolved_at=EXCLUDED.resolved_at,dismissed_at=EXCLUDED.dismissed_at,
               review_after=EXCLUDED.review_after,finding_snapshot_hash=EXCLUDED.finding_snapshot_hash,
               version=EXCLUDED.version,trace_id=EXCLUDED.trace_id,payload=EXCLUDED.payload",
        )
        .bind(finding.id)
        .bind(&finding.finding_key)
        .bind(&finding.scope.legal_entity_id)
        .bind(&finding.scope_hash)
        .bind(&finding.rule_set_version)
        .bind(enum_value(finding.condition_status)?)
        .bind(enum_value(finding.review_status)?)
        .bind(i64::try_from(finding.occurrence_count).map_err(|_| ActionError::InvalidRequest)?)
        .bind(finding.first_seen_at)
        .bind(finding.last_seen_at)
        .bind(finding.cleared_at)
        .bind(finding.resolved_at)
        .bind(finding.dismissed_at)
        .bind(finding.review_after)
        .bind(&finding.finding_snapshot_hash)
        .bind(i64::try_from(finding.version).map_err(|_| ActionError::InvalidRequest)?)
        .bind(finding.trace_id)
        .bind(json(finding)?)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

async fn save_proposals(
    tx: &mut Transaction<'_, Postgres>,
    engine: &ActionEngine,
) -> Result<(), ActionError> {
    for value in engine.state.proposals.values() {
        sqlx::query(
            "INSERT INTO business_action_proposals(id,finding_id,action_catalog_version,
             action_code,status,finding_version,finding_snapshot_hash,proposal_hash,created_at,
             expires_at,trace_id,version,payload)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
             ON CONFLICT(id) DO UPDATE SET status=EXCLUDED.status,proposal_hash=EXCLUDED.proposal_hash,
             version=EXCLUDED.version,payload=EXCLUDED.payload",
        )
        .bind(value.id)
        .bind(value.finding_id)
        .bind(&value.action_catalog_version)
        .bind(&value.action_code)
        .bind(enum_value(value.status)?)
        .bind(i64::try_from(value.finding_version).map_err(|_| ActionError::InvalidRequest)?)
        .bind(&value.finding_snapshot_hash)
        .bind(&value.proposal_hash)
        .bind(value.created_at)
        .bind(value.expires_at)
        .bind(value.trace_id)
        .bind(i64::try_from(value.version).map_err(|_| ActionError::InvalidRequest)?)
        .bind(json(value)?)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

async fn save_work(
    tx: &mut Transaction<'_, Postgres>,
    engine: &ActionEngine,
) -> Result<(), ActionError> {
    for value in engine.state.work_item_drafts.values() {
        sqlx::query(
            "INSERT INTO business_work_item_drafts(id,proposal_id,finding_id,status,preview_hash,
             finding_snapshot_hash,created_by_user_id,created_at,expires_at,trace_id,payload)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT(id) DO UPDATE SET status=EXCLUDED.status,payload=EXCLUDED.payload",
        )
        .bind(value.id)
        .bind(value.proposal_id)
        .bind(value.finding_id)
        .bind(enum_value(value.status)?)
        .bind(&value.preview_hash)
        .bind(&value.finding_snapshot_hash)
        .bind(value.created_by_user_id)
        .bind(value.created_at)
        .bind(value.expires_at)
        .bind(value.trace_id)
        .bind(json(value)?)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    for value in engine.state.work_items.values() {
        sqlx::query(
            "INSERT INTO business_work_items(id,work_item_number,finding_id,proposal_id,action_code,
             status,assignee_user_id,assignee_role_key,created_by_user_id,due_at,
             source_condition_status,finding_snapshot_hash,created_at,updated_at,version,trace_id,payload)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
             ON CONFLICT(id) DO UPDATE SET status=EXCLUDED.status,assignee_user_id=EXCLUDED.assignee_user_id,
             assignee_role_key=EXCLUDED.assignee_role_key,source_condition_status=EXCLUDED.source_condition_status,
             updated_at=EXCLUDED.updated_at,version=EXCLUDED.version,payload=EXCLUDED.payload",
        )
        .bind(value.id).bind(&value.work_item_number).bind(value.finding_id)
        .bind(value.proposal_id).bind(&value.action_code).bind(enum_value(value.status)?)
        .bind(value.assignee_user_id).bind(&value.assignee_role_key).bind(value.created_by_user_id)
        .bind(value.due_at).bind(enum_value(value.source_condition_status)?)
        .bind(&value.finding_snapshot_hash).bind(value.created_at).bind(value.updated_at)
        .bind(i64::try_from(value.version).map_err(|_| ActionError::InvalidRequest)?)
        .bind(value.trace_id).bind(json(value)?).execute(&mut **tx).await.map_err(db)?;
    }
    for value in engine.state.work_item_events.values() {
        sqlx::query(
            "INSERT INTO business_work_item_events(id,work_item_id,event_type,actor_user_id,
             occurred_at,trace_id,payload) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(id) DO NOTHING",
        )
        .bind(value.id)
        .bind(value.work_item_id)
        .bind(&value.event_type)
        .bind(value.actor_user_id)
        .bind(value.occurred_at)
        .bind(value.trace_id)
        .bind(json(value)?)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    for (key, value) in &engine.state.idempotency {
        sqlx::query(
            "INSERT INTO business_action_idempotency(enterprise_user_id,idempotency_key,request_hash,entity_id)
             VALUES($1,$2,$3,$4) ON CONFLICT(enterprise_user_id,idempotency_key) DO NOTHING",
        )
        .bind(value.user_id).bind(key).bind(&value.request_hash).bind(value.entity_id)
        .execute(&mut **tx).await.map_err(db)?;
    }
    Ok(())
}

async fn save_approvals(
    tx: &mut Transaction<'_, Postgres>,
    engine: &ActionEngine,
) -> Result<(), ActionError> {
    for value in engine.state.approval_previews.values() {
        sqlx::query(
            "INSERT INTO business_approval_draft_previews(id,work_item_id,finding_id,preview_hash,
             created_by_user_id,created_at,expires_at,consumed,trace_id,payload)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
             ON CONFLICT(id) DO UPDATE SET consumed=EXCLUDED.consumed,payload=EXCLUDED.payload",
        )
        .bind(value.id)
        .bind(value.work_item_id)
        .bind(value.finding_id)
        .bind(&value.preview_hash)
        .bind(value.created_by_user_id)
        .bind(value.created_at)
        .bind(value.expires_at)
        .bind(value.consumed)
        .bind(value.trace_id)
        .bind(json(value)?)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    for value in engine.state.approval_drafts.values() {
        sqlx::query(
            "INSERT INTO business_approval_drafts(id,approval_draft_number,work_item_id,finding_id,
             action_code,draft_type,status,draft_only,source_snapshot_hash,draft_hash,
             created_by_user_id,created_at,updated_at,expires_at,version,trace_id,payload)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
             ON CONFLICT(id) DO UPDATE SET status=EXCLUDED.status,draft_hash=EXCLUDED.draft_hash,
             updated_at=EXCLUDED.updated_at,version=EXCLUDED.version,payload=EXCLUDED.payload",
        )
        .bind(value.id)
        .bind(&value.approval_draft_number)
        .bind(value.work_item_id)
        .bind(value.finding_id)
        .bind(&value.action_code)
        .bind(&value.draft_type)
        .bind(enum_value(value.status)?)
        .bind(value.draft_only)
        .bind(&value.source_snapshot_hash)
        .bind(&value.draft_hash)
        .bind(value.created_by_user_id)
        .bind(value.created_at)
        .bind(value.updated_at)
        .bind(value.expires_at)
        .bind(i64::try_from(value.version).map_err(|_| ActionError::InvalidRequest)?)
        .bind(value.trace_id)
        .bind(json(value)?)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
        let evidence_id = crate::catalog::stable_uuid(format!("evidence:{}", value.id).as_bytes());
        sqlx::query(
            "INSERT INTO business_approval_draft_evidence(id,approval_draft_id,evidence_type,
             source_snapshot_hash,payload) VALUES($1,$2,'finding_snapshot',$3,$4)
             ON CONFLICT(id) DO UPDATE SET source_snapshot_hash=EXCLUDED.source_snapshot_hash,payload=EXCLUDED.payload",
        )
        .bind(evidence_id).bind(value.id).bind(&value.source_snapshot_hash)
        .bind(serde_json::json!({"findingId":value.finding_id,"draftOnly":true}))
        .execute(&mut **tx).await.map_err(db)?;
    }
    Ok(())
}

async fn save_audits(
    tx: &mut Transaction<'_, Postgres>,
    engine: &ActionEngine,
) -> Result<(), ActionError> {
    for value in &engine.state.audits {
        sqlx::query(
            "INSERT INTO business_action_audit_events(id,occurred_at,event_type,result,entity_id,
             action_code,status,entity_hash,enterprise_user_id,reason_code,entity_version,trace_id)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT(id) DO NOTHING",
        )
        .bind(value.id)
        .bind(value.occurred_at)
        .bind(&value.event_type)
        .bind(&value.result)
        .bind(value.entity_id)
        .bind(&value.action_code)
        .bind(&value.status)
        .bind(&value.hash)
        .bind(value.user_id)
        .bind(&value.reason_code)
        .bind(value.version.and_then(|item| i64::try_from(item).ok()))
        .bind(value.trace_id)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

fn json<T: Serialize>(value: &T) -> Result<serde_json::Value, ActionError> {
    serde_json::to_value(value).map_err(|_| ActionError::InvalidRequest)
}

fn enum_value<T: Serialize>(value: T) -> Result<String, ActionError> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(ActionError::InvalidRequest)
}
