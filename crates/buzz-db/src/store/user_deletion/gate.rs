//! Request-wide offline gate. Only this stage installs identity restrictions.
//! The uploader fence is not a general serving fence: the relay and external
//! workers must remain stopped for the entire destructive workflow.
use super::{normalize_user_deletion_pubkey, UserDeletionInventory, UserDeletionStore};
use crate::entity_deletion::{EntityLease, EntityRequest};
use crate::media_deletion::SubjectScope;
use crate::store::entity_deletion::{lock_owned, validate_approved_inventory};
use crate::user_deletion_fences::{self, FenceIdentity};
use crate::{DbError, Result};
use buzz_core::CommunityId;
use nostr::PublicKey;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

fn invalid(message: &str) -> DbError {
    DbError::DeletionSafety(message.into())
}

fn receipt(scope: &SubjectScope) -> Value {
    json!({"version":1,"record":"user_gate_complete","mode":"delete",
        "scope":scope,"offline_required":true,"drain_complete":true,
        "online_write_fencing_complete":false})
}

impl UserDeletionStore {
    /// Install the approved identities' fences in the first request-wide stage,
    /// then verify all admitted external work has drained. Failed drains retain
    /// the fences and retry the same stage. Requires operator-enforced downtime.
    pub async fn gate_user(
        &self,
        lease: &EntityLease,
        stage_id: Uuid,
        scope: &SubjectScope,
    ) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        budgets(&mut tx).await?;
        let request = validate_request(&mut tx, lease, scope).await?;
        let inventory = UserDeletionInventory::read(&request)?;
        let metadata = running_gate(&mut tx, lease, stage_id, &request).await?;
        let binding = serde_json::to_value(scope)?;
        if metadata
            .get("gate_scope")
            .is_some_and(|saved| saved != &binding)
        {
            return Err(invalid(
                "user gate scope changed; resume with the saved scope",
            ));
        }
        let human =
            PublicKey::from_hex(&scope.human).map_err(|_| invalid("invalid gate subject"))?;
        let mut identities = vec![FenceIdentity::user(human)];
        for (community, agent) in &scope.agents {
            if !inventory.communities.iter().any(|(id, _)| id == community) {
                return Err(invalid("agent gate is outside the approved inventory"));
            }
            let key = PublicKey::from_hex(agent).map_err(|_| invalid("invalid agent key"))?;
            identities.push(FenceIdentity::agent(
                CommunityId::from_uuid(*community),
                key,
            ));
        }
        user_deletion_fences::lock_exclusive(&mut tx, &identities).await?;
        user_deletion_fences::insert(&mut tx, identities[0], lease, None).await?;
        for ((community, agent), identity) in
            scope.agents.iter().zip(identities.into_iter().skip(1))
        {
            let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE community_id=$1 AND pubkey=decode($2,'hex') AND agent_owner_pubkey=decode($3,'hex'))")
                .bind(community).bind(agent).bind(&scope.human).fetch_one(&mut *tx).await?;
            if !owned {
                return Err(invalid("selected agent ownership changed before gating"));
            }
            user_deletion_fences::insert(&mut tx, identity, lease, Some(human)).await?;
        }
        sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{gate_scope}',$2),updated_at=clock_timestamp() WHERE id=$1")
            .bind(stage_id).bind(binding).execute(&mut *tx).await?;
        tx.commit().await?;

        // The restriction survives a retry while already-admitted work drains.
        // Check all operations, rather than only media uploads.
        let mut tx = self.pool.begin().await?;
        budgets(&mut tx).await?;
        let request = validate_request(&mut tx, lease, scope).await?;
        let metadata = running_gate(&mut tx, lease, stage_id, &request).await?;
        if metadata["gate_scope"] != serde_json::to_value(scope)? {
            return Err(invalid("user gate scope changed"));
        }
        verify_fences(&mut tx, lease, scope).await?;
        verify_drained(&mut tx).await?;
        let result = receipt(scope);
        sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{gate_ready}',$2),updated_at=clock_timestamp() WHERE id=$1")
            .bind(stage_id).bind(&result).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Verify the same request's completed destructive gate without installing
    /// fences or changing gate metadata. Read-only handlers do not call this.
    /// The offline assertion remains necessary; a receipt does not stop traffic.
    pub async fn verify_user_gate(&self, lease: &EntityLease, scope: &SubjectScope) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        budgets(&mut tx).await?;
        let request = validate_request(&mut tx, lease, scope).await?;
        let gates: Vec<Value> = sqlx::query_scalar("SELECT metadata_json FROM entity_deletion_stage_history WHERE request_id=$1 AND stage='gate_user' AND target_community_id IS NULL AND status='completed' AND NOT is_current LIMIT 2")
            .bind(lease.request_id).fetch_all(&mut *tx).await?;
        let [metadata] = gates.as_slice() else {
            return Err(invalid(
                "deletion requires one completed request-wide user gate",
            ));
        };
        let expected_inputs =
            UserDeletionInventory::read(&request)?.inputs(&request.target, None)?;
        let expected = receipt(scope);
        if metadata["inputs"] != expected_inputs
            || metadata["gate_scope"] != serde_json::to_value(scope)?
            || metadata["gate_ready"] != expected
            || metadata["result"] != expected
        {
            return Err(invalid(
                "completed user gate does not match this request and scope",
            ));
        }
        verify_fences(&mut tx, lease, scope).await?;
        verify_drained(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn budgets(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SET LOCAL lock_timeout='5s'")
        .execute(&mut **tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='30s'")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn validate_request(
    tx: &mut Transaction<'_, Postgres>,
    lease: &EntityLease,
    scope: &SubjectScope,
) -> Result<EntityRequest> {
    let request = lock_owned(tx, lease).await?;
    validate_approved_inventory(tx, &request).await?;
    if request.entity_type != "user"
        || request.target != scope.human
        || normalize_user_deletion_pubkey(&scope.human)? != scope.human
        || scope.agents.len() > 4095
    {
        return Err(invalid(
            "user gate scope does not match the approved deletion request",
        ));
    }
    Ok(request)
}

async fn running_gate(
    tx: &mut Transaction<'_, Postgres>,
    lease: &EntityLease,
    stage_id: Uuid,
    request: &EntityRequest,
) -> Result<Value> {
    let metadata: Option<Value> = sqlx::query_scalar("SELECT metadata_json FROM entity_deletion_stage_history WHERE id=$1 AND request_id=$2 AND stage='gate_user' AND target_community_id IS NULL AND is_current AND status='running' FOR UPDATE")
        .bind(stage_id).bind(lease.request_id).fetch_optional(&mut **tx).await?;
    let metadata = metadata.ok_or_else(|| invalid("user gate stage is not running"))?;
    if metadata["inputs"] != UserDeletionInventory::read(request)?.inputs(&request.target, None)? {
        return Err(invalid(
            "user gate inputs differ from the approved inventory",
        ));
    }
    Ok(metadata)
}

async fn verify_fences(
    tx: &mut Transaction<'_, Postgres>,
    lease: &EntityLease,
    scope: &SubjectScope,
) -> Result<()> {
    for (community, key) in std::iter::once((None, scope.human.as_str()))
        .chain(scope.agents.iter().map(|(c, k)| (Some(*c), k.as_str())))
    {
        let fenced: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_deletion_fences WHERE pubkey=$1 AND scope_community_id IS NOT DISTINCT FROM $2 AND request_id=$3 AND source_owner_pubkey IS NOT DISTINCT FROM $4)")
            .bind(key).bind(community).bind(lease.request_id)
            .bind(community.map(|_| scope.human.as_str())).fetch_one(&mut **tx).await?;
        if !fenced {
            return Err(invalid(
                "user gate fence missing or belongs to a different request",
            ));
        }
    }
    Ok(())
}

async fn verify_drained(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM community_serving_write_leases WHERE lease_until>=clock_timestamp())")
        .fetch_one(&mut **tx).await?;
    if active {
        return Err(invalid("serving work is still active; keep relay and external workers stopped and retry the gate"));
    }
    Ok(())
}
