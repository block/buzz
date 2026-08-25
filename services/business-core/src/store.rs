use crate::{
    model::{
        AuthorizationSnapshot, DataScopes, EligibleUser, GrantOperation, GroupProfile,
        MasterDataRecord, ResourceType, RoleSummary, ScopeDimension,
    },
    security::valid_key,
};
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPool, FromRow, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found or forbidden")]
    NotFoundOrForbidden,
    #[error("authorization revision conflict")]
    Conflict,
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("database migration error")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("serialization error")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

#[derive(Debug, FromRow)]
struct RoleRow {
    id: Uuid,
    role_key: String,
    name: String,
}

#[derive(Debug, FromRow)]
struct CandidateRow {
    enterprise_user_id: Uuid,
    display_name: String,
    role_key: String,
}

#[derive(Debug, FromRow)]
pub struct AssignmentPolicy {
    pub required_permission: String,
    pub eligible_role_keys: Vec<String>,
}

#[derive(Debug, FromRow)]
pub struct ApprovalPolicy {
    pub required_permission: String,
    pub eligible_role_keys: Vec<String>,
    pub min_approvers: i16,
    pub allow_self_approval: bool,
    pub require_distinct_business_unit: bool,
    pub step_up_amount_minor: Option<i64>,
}

impl PgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<(), StoreError> {
        business_auth_gateway::Store::migrate(&self.pool).await?;
        Ok(())
    }

    pub async fn active_user(&self, user_id: Uuid) -> Result<bool, StoreError> {
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM enterprise_users WHERE id=$1 AND status='active')",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn authorization_revision(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT revision FROM business_authorization_revision WHERE singleton",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn snapshot(&self, user_id: Uuid) -> Result<AuthorizationSnapshot, StoreError> {
        if !self.active_user(user_id).await? {
            return Err(StoreError::NotFoundOrForbidden);
        }
        let roles = sqlx::query_as::<_, RoleRow>(
            "SELECT r.id,r.role_key,r.name FROM business_roles r JOIN business_user_roles ur ON ur.role_id=r.id WHERE ur.enterprise_user_id=$1 AND r.status='active' ORDER BY r.role_key",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        let permissions = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT rp.permission_key FROM business_role_permissions rp JOIN business_user_roles ur ON ur.role_id=rp.role_id JOIN business_roles r ON r.id=rp.role_id WHERE ur.enterprise_user_id=$1 AND r.status='active' ORDER BY rp.permission_key",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .collect();
        let scopes = DataScopes {
            legal_entity_ids: legal_entity_ids(&self.pool, user_id).await?,
            warehouse_ids: warehouse_ids(&self.pool, user_id).await?,
            customer_ids: customer_ids(&self.pool, user_id).await?,
            supplier_ids: supplier_ids(&self.pool, user_id).await?,
            brand_ids: brand_ids(&self.pool, user_id).await?,
            business_unit_ids: business_unit_ids(&self.pool, user_id).await?,
        };
        let scope_version = self.authorization_revision().await?;
        let roles = roles
            .into_iter()
            .map(|role| RoleSummary {
                id: role.id,
                role_key: role.role_key,
                name: role.name,
            })
            .collect::<Vec<_>>();
        let effective_scope_hash =
            stable_hash(&(user_id, scope_version, &roles, &permissions, &scopes))?;
        Ok(AuthorizationSnapshot {
            enterprise_user_id: user_id,
            roles,
            permission_keys: permissions,
            scopes,
            scope_version,
            effective_scope_hash,
            evaluated_at: Utc::now(),
        })
    }

    pub async fn resource(
        &self,
        resource_type: ResourceType,
        resource_id: Uuid,
    ) -> Result<MasterDataRecord, StoreError> {
        sqlx::query_as::<_, MasterDataRecord>(
            "SELECT resource_type,id,code,name,status,legal_entity_id,warehouse_id,customer_id,supplier_id,brand_id,business_unit_id,version FROM business_master_data_directory WHERE resource_type=$1 AND id=$2",
        )
        .bind(resource_type.as_str())
        .bind(resource_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFoundOrForbidden)
    }

    pub async fn group_profile(&self) -> Result<GroupProfile, StoreError> {
        sqlx::query_as(
            "SELECT id,code,name,base_currency::text,timezone,status,version FROM business_group_profile WHERE singleton",
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFoundOrForbidden)
    }

    pub async fn list_resources(
        &self,
        resource_type: ResourceType,
        snapshot: &AuthorizationSnapshot,
        limit: i64,
    ) -> Result<Vec<MasterDataRecord>, StoreError> {
        let rows = sqlx::query_as::<_, MasterDataRecord>(
            "SELECT resource_type,id,code,name,status,legal_entity_id,warehouse_id,customer_id,supplier_id,brand_id,business_unit_id,version FROM business_master_data_directory WHERE resource_type=$1 AND status='active' ORDER BY code LIMIT $2",
        )
        .bind(resource_type.as_str())
        .bind(limit.clamp(1, 200))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter(|resource| snapshot.scopes.permits(resource))
            .collect())
    }

    pub async fn can_access(
        &self,
        user_id: Uuid,
        permission_key: &str,
        resource_type: ResourceType,
        resource_id: Uuid,
    ) -> Result<(bool, AuthorizationSnapshot), StoreError> {
        if !valid_key(permission_key, 96) {
            return Err(StoreError::Invalid("invalid permissionKey".into()));
        }
        let snapshot = self.snapshot(user_id).await?;
        let resource = self.resource(resource_type, resource_id).await?;
        let allowed = snapshot.permission_keys.contains(permission_key)
            && resource.status == "active"
            && snapshot.scopes.permits(&resource);
        Ok((allowed, snapshot))
    }

    pub async fn assignment_policy(
        &self,
        action_code: &str,
    ) -> Result<AssignmentPolicy, StoreError> {
        sqlx::query_as(
            "SELECT required_permission,eligible_role_keys FROM business_assignment_policies WHERE action_code=$1 AND status='active'",
        )
        .bind(action_code)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFoundOrForbidden)
    }

    pub async fn approval_policy(&self, action_code: &str) -> Result<ApprovalPolicy, StoreError> {
        sqlx::query_as(
            "SELECT required_permission,eligible_role_keys,min_approvers,allow_self_approval,require_distinct_business_unit,step_up_amount_minor FROM business_approval_policies WHERE action_code=$1 AND status='active'",
        )
        .bind(action_code)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFoundOrForbidden)
    }

    pub async fn eligible_users(
        &self,
        resource: &MasterDataRecord,
        permission: &str,
        eligible_role_keys: &[String],
        excluded_user: Option<Uuid>,
        requester_business_units: Option<&BTreeSet<Uuid>>,
    ) -> Result<Vec<EligibleUser>, StoreError> {
        let rows = sqlx::query_as::<_, CandidateRow>(
            "SELECT u.id enterprise_user_id,u.display_name,r.role_key FROM enterprise_users u JOIN business_user_roles ur ON ur.enterprise_user_id=u.id JOIN business_roles r ON r.id=ur.role_id JOIN business_role_permissions rp ON rp.role_id=r.id WHERE u.status='active' AND r.status='active' AND r.role_key=ANY($1) AND rp.permission_key=$2 ORDER BY u.display_name,u.id,r.role_key",
        )
        .bind(eligible_role_keys)
        .bind(permission)
        .fetch_all(&self.pool)
        .await?;
        let mut grouped: BTreeMap<Uuid, EligibleUser> = BTreeMap::new();
        for row in rows {
            if excluded_user == Some(row.enterprise_user_id) {
                continue;
            }
            let snapshot = self.snapshot(row.enterprise_user_id).await?;
            if !snapshot.scopes.permits(resource) {
                continue;
            }
            if requester_business_units
                .is_some_and(|requester| !requester.is_disjoint(&snapshot.scopes.business_unit_ids))
            {
                continue;
            }
            let item = grouped
                .entry(row.enterprise_user_id)
                .or_insert(EligibleUser {
                    enterprise_user_id: row.enterprise_user_id,
                    display_name: row.display_name,
                    role_keys: Vec::new(),
                });
            item.role_keys.push(row.role_key);
        }
        Ok(grouped.into_values().collect())
    }

    pub async fn require_admin(&self, actor: Uuid) -> Result<(), StoreError> {
        let snapshot = self.snapshot(actor).await?;
        if snapshot.permission_keys.contains("business_core:admin") {
            Ok(())
        } else {
            Err(StoreError::NotFoundOrForbidden)
        }
    }

    pub async fn mutate_role(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        user_id: Uuid,
        role_id: Uuid,
        operation: GrantOperation,
        expected_revision: i64,
    ) -> Result<i64, StoreError> {
        self.require_admin(actor).await?;
        let mut tx = self.pool.begin().await?;
        lock_revision(&mut tx, expected_revision).await?;
        match operation {
            GrantOperation::Grant => {
                sqlx::query("INSERT INTO business_user_roles(enterprise_user_id,role_id,assigned_by) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                    .bind(user_id).bind(role_id).bind(actor).execute(&mut *tx).await?;
            }
            GrantOperation::Revoke => {
                sqlx::query(
                    "DELETE FROM business_user_roles WHERE enterprise_user_id=$1 AND role_id=$2",
                )
                .bind(user_id)
                .bind(role_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        audit(
            &mut tx,
            trace_id,
            actor,
            "role_assignment_mutated",
            "role",
            &role_id.to_string(),
            serde_json::json!({"enterpriseUserId":user_id,"operation":format!("{operation:?}")}),
        )
        .await?;
        outbox(
            &mut tx,
            "business.authorization.changed",
            "enterprise_user",
            &user_id.to_string(),
            serde_json::json!({"reason":"role_assignment"}),
        )
        .await?;
        let revision = current_revision(&mut tx).await?;
        tx.commit().await?;
        Ok(revision)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn mutate_scope(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        user_id: Uuid,
        dimension: ScopeDimension,
        resource_id: Uuid,
        operation: GrantOperation,
        expected_revision: i64,
    ) -> Result<i64, StoreError> {
        self.require_admin(actor).await?;
        let mut tx = self.pool.begin().await?;
        lock_revision(&mut tx, expected_revision).await?;
        mutate_scope_row(&mut tx, actor, user_id, dimension, resource_id, operation).await?;
        audit(
            &mut tx,
            trace_id,
            actor,
            "scope_mutated",
            &format!("{dimension:?}"),
            &resource_id.to_string(),
            serde_json::json!({"enterpriseUserId":user_id,"operation":format!("{operation:?}")}),
        )
        .await?;
        outbox(
            &mut tx,
            "business.authorization.changed",
            "enterprise_user",
            &user_id.to_string(),
            serde_json::json!({"reason":"scope"}),
        )
        .await?;
        let revision = current_revision(&mut tx).await?;
        tx.commit().await?;
        Ok(revision)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

async fn legal_entity_ids(pool: &PgPool, user: Uuid) -> Result<BTreeSet<Uuid>, sqlx::Error> {
    Ok(sqlx::query_scalar("SELECT legal_entity_id FROM business_legal_entity_scopes WHERE enterprise_user_id=$1 ORDER BY legal_entity_id").bind(user).fetch_all(pool).await?.into_iter().collect())
}
async fn warehouse_ids(pool: &PgPool, user: Uuid) -> Result<BTreeSet<Uuid>, sqlx::Error> {
    Ok(sqlx::query_scalar("SELECT warehouse_id FROM business_warehouse_scopes WHERE enterprise_user_id=$1 ORDER BY warehouse_id").bind(user).fetch_all(pool).await?.into_iter().collect())
}
async fn customer_ids(pool: &PgPool, user: Uuid) -> Result<BTreeSet<Uuid>, sqlx::Error> {
    Ok(sqlx::query_scalar("SELECT customer_id FROM business_customer_scopes WHERE enterprise_user_id=$1 ORDER BY customer_id").bind(user).fetch_all(pool).await?.into_iter().collect())
}
async fn supplier_ids(pool: &PgPool, user: Uuid) -> Result<BTreeSet<Uuid>, sqlx::Error> {
    Ok(sqlx::query_scalar("SELECT supplier_id FROM business_supplier_scopes WHERE enterprise_user_id=$1 ORDER BY supplier_id").bind(user).fetch_all(pool).await?.into_iter().collect())
}
async fn brand_ids(pool: &PgPool, user: Uuid) -> Result<BTreeSet<Uuid>, sqlx::Error> {
    Ok(sqlx::query_scalar(
        "SELECT brand_id FROM business_brand_scopes WHERE enterprise_user_id=$1 ORDER BY brand_id",
    )
    .bind(user)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect())
}
async fn business_unit_ids(pool: &PgPool, user: Uuid) -> Result<BTreeSet<Uuid>, sqlx::Error> {
    Ok(sqlx::query_scalar("SELECT business_unit_id FROM business_unit_scopes WHERE enterprise_user_id=$1 ORDER BY business_unit_id").bind(user).fetch_all(pool).await?.into_iter().collect())
}

async fn mutate_scope_row(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    user: Uuid,
    dimension: ScopeDimension,
    resource: Uuid,
    operation: GrantOperation,
) -> Result<(), sqlx::Error> {
    let sql = match (dimension, operation) {
        (ScopeDimension::LegalEntity, GrantOperation::Grant) => "INSERT INTO business_legal_entity_scopes(enterprise_user_id,legal_entity_id,granted_by) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
        (ScopeDimension::Warehouse, GrantOperation::Grant) => "INSERT INTO business_warehouse_scopes(enterprise_user_id,warehouse_id,granted_by) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
        (ScopeDimension::Customer, GrantOperation::Grant) => "INSERT INTO business_customer_scopes(enterprise_user_id,customer_id,granted_by) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
        (ScopeDimension::Supplier, GrantOperation::Grant) => "INSERT INTO business_supplier_scopes(enterprise_user_id,supplier_id,granted_by) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
        (ScopeDimension::Brand, GrantOperation::Grant) => "INSERT INTO business_brand_scopes(enterprise_user_id,brand_id,granted_by) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
        (ScopeDimension::BusinessUnit, GrantOperation::Grant) => "INSERT INTO business_unit_scopes(enterprise_user_id,business_unit_id,granted_by) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
        (ScopeDimension::LegalEntity, GrantOperation::Revoke) => "DELETE FROM business_legal_entity_scopes WHERE enterprise_user_id=$1 AND legal_entity_id=$2",
        (ScopeDimension::Warehouse, GrantOperation::Revoke) => "DELETE FROM business_warehouse_scopes WHERE enterprise_user_id=$1 AND warehouse_id=$2",
        (ScopeDimension::Customer, GrantOperation::Revoke) => "DELETE FROM business_customer_scopes WHERE enterprise_user_id=$1 AND customer_id=$2",
        (ScopeDimension::Supplier, GrantOperation::Revoke) => "DELETE FROM business_supplier_scopes WHERE enterprise_user_id=$1 AND supplier_id=$2",
        (ScopeDimension::Brand, GrantOperation::Revoke) => "DELETE FROM business_brand_scopes WHERE enterprise_user_id=$1 AND brand_id=$2",
        (ScopeDimension::BusinessUnit, GrantOperation::Revoke) => "DELETE FROM business_unit_scopes WHERE enterprise_user_id=$1 AND business_unit_id=$2",
    };
    let mut query = sqlx::query(sql).bind(user).bind(resource);
    if matches!(operation, GrantOperation::Grant) {
        query = query.bind(actor);
    }
    query.execute(&mut **tx).await?;
    Ok(())
}

fn stable_hash<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

async fn lock_revision(
    tx: &mut Transaction<'_, Postgres>,
    expected: i64,
) -> Result<(), StoreError> {
    let actual: i64 = sqlx::query_scalar(
        "SELECT revision FROM business_authorization_revision WHERE singleton FOR UPDATE",
    )
    .fetch_one(&mut **tx)
    .await?;
    if actual != expected {
        return Err(StoreError::Conflict);
    }
    Ok(())
}

async fn current_revision(tx: &mut Transaction<'_, Postgres>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT revision FROM business_authorization_revision WHERE singleton")
        .fetch_one(&mut **tx)
        .await
}

pub(crate) async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    trace_id: Uuid,
    actor: Uuid,
    operation: &str,
    target_type: &str,
    target_id: &str,
    details: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO business_core_audit_events(trace_id,actor_user_id,operation,target_type,target_id,details) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(trace_id).bind(actor).bind(operation).bind(target_type).bind(target_id).bind(details)
        .execute(&mut **tx).await?;
    Ok(())
}

pub(crate) async fn outbox(
    tx: &mut Transaction<'_, Postgres>,
    topic: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO business_core_outbox(id,topic,aggregate_type,aggregate_id,payload) VALUES($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(topic).bind(aggregate_type).bind(aggregate_id).bind(payload)
        .execute(&mut **tx).await?;
    Ok(())
}
