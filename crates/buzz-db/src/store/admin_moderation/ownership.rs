//! Read-only, tenant-scoped inventory for an operator reviewing identity retirement.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

use crate::{Db, Result};

const LIMIT: usize = 200;

/// A bounded point-in-time preview; never an authorization to mutate this set.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInventory {
    /// Community in which every resource was read.
    pub community_id: Uuid,
    /// Canonical host of the community read in this snapshot.
    pub community_host: String,
    /// Exact root public key, independent of its current name or account binding.
    pub pubkey: String,
    /// Root followed by agents with a recorded, verified link to this exact key.
    pub identities: Vec<InventoryIdentity>,
    /// Workflows owned by the returned keys, including disabled/archived workflows.
    pub workflows: Vec<InventoryWorkflow>,
    /// More identities exist than this preview can return.
    pub identities_truncated: bool,
    /// More workflows exist for the returned identities than this preview can return.
    pub workflows_truncated: bool,
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    async fn community(pool: &sqlx::PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("inventory-{}.example", id.simple()))
            .execute(pool)
            .await
            .unwrap();
        CommunityId::from_uuid(id)
    }

    async fn user(pool: &sqlx::PgPool, c: CommunityId, key: &[u8], owner: Option<&[u8]>) {
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)",
        )
        .bind(c.as_uuid())
        .bind(key)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn workflow(pool: &sqlx::PgPool, c: CommunityId, owner: &[u8]) -> Uuid {
        sqlx::query_scalar("INSERT INTO workflows (community_id, owner_pubkey, name, definition, definition_hash) VALUES ($1, $2, 'example', '{}', $2) RETURNING id")
            .bind(c.as_uuid()).bind(owner).fetch_one(pool).await.unwrap()
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn inventory_follows_exact_owner_and_community_and_preserves_archive_and_run_facts() {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let a = community(&pool).await;
        let b = community(&pool).await;
        let root = [1; 32];
        let replacement = [2; 32];
        let agent = [3; 32];
        let other_agent = [4; 32];
        for c in [a, b] {
            user(&pool, c, &root, None).await;
            user(&pool, c, &replacement, None).await;
            user(&pool, c, &agent, Some(&root)).await;
            user(&pool, c, &other_agent, Some(&replacement)).await;
        }
        let direct = workflow(&pool, a, &root).await;
        let owned = workflow(&pool, a, &agent).await;
        workflow(&pool, a, &other_agent).await;
        workflow(&pool, b, &root).await;
        db.archive(
            a,
            &hex::encode(agent),
            "admin",
            &hex::encode(root),
            None,
            None,
            &"a".repeat(64),
        )
        .await
        .unwrap();
        for status in [
            "pending",
            "pending",
            "running",
            "waiting_approval",
            "completed",
        ] {
            sqlx::query("INSERT INTO workflow_runs (community_id, workflow_id, status) VALUES ($1, $2, $3::run_status)")
                .bind(a.as_uuid()).bind(owned).bind(status).execute(&pool).await.unwrap();
        }
        sqlx::query("UPDATE workflows SET status = 'archived', enabled = false WHERE community_id = $1 AND id = $2")
            .bind(a.as_uuid()).bind(owned).execute(&pool).await.unwrap();
        let inventory = db.admin_identity_inventory(a, &root).await.unwrap();
        assert_eq!(inventory.community_id, *a.as_uuid());
        assert_eq!(inventory.pubkey, hex::encode(root));
        assert_eq!(inventory.identities.len(), 2);
        assert_eq!(inventory.identities[0].pubkey, hex::encode(root));
        assert!(inventory.identities[1].archived);
        assert_eq!(
            inventory.identities[1].owner_pubkey,
            Some(hex::encode(root))
        );
        assert_eq!(inventory.workflows.len(), 2);
        assert!(inventory.workflows.iter().any(|row| row.id == direct));
        let row = inventory
            .workflows
            .iter()
            .find(|row| row.id == owned)
            .unwrap();
        assert_eq!(
            (
                row.pending_runs,
                row.running_runs,
                row.waiting_approval_runs
            ),
            (2, 1, 1)
        );
        assert_eq!(row.status, "archived");
        assert!(!row.enabled);
        assert!(!inventory.identities_truncated && !inventory.workflows_truncated);
        assert!(
            !db.admin_identity_inventory(b, &root)
                .await
                .unwrap()
                .identities[1]
                .archived
        );
        let missing = db.admin_identity_inventory(a, &[9; 32]).await.unwrap();
        assert_eq!(missing.identities.len(), 1);
        assert!(missing.workflows.is_empty());
        // A preview preserves all of the stored run/archive facts.
        assert!(db.is_archived(a, &hex::encode(agent)).await.unwrap());
        assert_eq!(
            db.get_workflow(a, owned).await.unwrap().status.to_string(),
            "archived"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn inventory_discloses_identity_and_workflow_truncation() {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let c = community(&pool).await;
        let root = [255; 32];
        user(&pool, c, &root, None).await;
        sqlx::query("INSERT INTO users (community_id, pubkey, agent_owner_pubkey) SELECT $1, decode(lpad(to_hex(i),64,'0'),'hex'), $2 FROM generate_series(1,201) i")
            .bind(c.as_uuid()).bind(root.as_slice()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workflows (community_id, owner_pubkey, name, definition, definition_hash) SELECT $1, $2, 'example', '{}', $2 FROM generate_series(1,201)")
            .bind(c.as_uuid()).bind(root.as_slice()).execute(&pool).await.unwrap();
        let inventory = db.admin_identity_inventory(c, &root).await.unwrap();
        assert_eq!(inventory.identities.len(), LIMIT);
        assert_eq!(inventory.workflows.len(), LIMIT);
        assert!(inventory.identities_truncated && inventory.workflows_truncated);
        assert_eq!(inventory.identities[0].pubkey, hex::encode(root));
    }
}

/// One identity, including archived and profile-less keys.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryIdentity {
    /// Lowercase hex public key.
    pub pubkey: String,
    /// Profile label, never ownership evidence.
    pub display_name: Option<String>,
    /// Recorded owner, if this key itself is an agent.
    pub owner_pubkey: Option<String>,
    /// Current community-local NIP-IA state.
    pub archived: bool,
}

/// Workflow metadata and run counts, excluding definitions and private approval data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryWorkflow {
    /// Community-local workflow identifier.
    pub id: Uuid,
    /// Saved workflow label.
    pub name: String,
    /// Exact human or agent owner.
    pub owner_pubkey: String,
    /// Channel scope, when present; a channel is not the workflow's owner.
    pub channel_id: Option<Uuid>,
    /// Saved lifecycle state, not evidence of a running process.
    pub status: String,
    /// Saved trigger eligibility flag.
    pub enabled: bool,
    /// Queued runs.
    pub pending_runs: i64,
    /// Executing runs; this read does not stop them.
    pub running_runs: i64,
    /// Runs awaiting approval.
    pub waiting_approval_runs: i64,
}

impl Db {
    /// Read a bounded ownership preview in one consistent, read-only snapshot.
    ///
    /// Ownership follows the exact root key in this community only. The preview
    /// covers materialized relay ownership links, not issuer-only identities or
    /// external execution hosts. Every action must discover and revalidate its
    /// own inventory; callers must not treat this response as a mutation token.
    #[datastore_span(name = "admin_identity_inventory", system = "postgresql")]
    pub async fn admin_identity_inventory(
        &self,
        community: CommunityId,
        root: &[u8],
    ) -> Result<IdentityInventory> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        // The operator may repeat a preview after a client timeout. Keep a
        // slow metadata/count query from occupying the serving pool indefinitely.
        sqlx::query(
            "SELECT set_config('statement_timeout', \
             LEAST(NULLIF(setting::bigint, 0), 5000)::text, true) \
             FROM pg_settings WHERE name = 'statement_timeout'",
        )
        .execute(&mut *tx)
        .await?;
        let community_host: String =
            sqlx::query_scalar("SELECT host FROM communities WHERE id = $1")
                .bind(community.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
        let mut rows = sqlx::query(
            "WITH keys AS (SELECT $2::bytea AS pubkey UNION \
             SELECT pubkey FROM users WHERE community_id = $1 AND agent_owner_pubkey = $2) \
             SELECT k.pubkey, u.display_name, u.agent_owner_pubkey, \
                    (a.pubkey IS NOT NULL) AS archived \
             FROM keys k \
             LEFT JOIN users u ON u.community_id = $1 AND u.pubkey = k.pubkey \
             LEFT JOIN archived_identities a \
               ON a.community_id = $1 AND a.pubkey = encode(k.pubkey, 'hex') \
             ORDER BY (k.pubkey = $2) DESC, k.pubkey LIMIT $3",
        )
        .bind(community.as_uuid())
        .bind(root)
        .bind((LIMIT + 1) as i64)
        .fetch_all(&mut *tx)
        .await?;
        let identities_truncated = rows.len() > LIMIT;
        rows.truncate(LIMIT);
        let mut owners = Vec::with_capacity(rows.len());
        let mut identities = Vec::with_capacity(rows.len());
        for row in rows {
            let pubkey: Vec<u8> = row.try_get("pubkey")?;
            let owner: Option<Vec<u8>> = row.try_get("agent_owner_pubkey")?;
            identities.push(InventoryIdentity {
                pubkey: hex::encode(&pubkey),
                display_name: row.try_get("display_name")?,
                owner_pubkey: owner.map(hex::encode),
                archived: row.try_get("archived")?,
            });
            owners.push(pubkey);
        }
        let mut rows = sqlx::query(
            "WITH owned AS MATERIALIZED ( \
                 SELECT id, name, owner_pubkey, channel_id, status, enabled FROM workflows \
                 WHERE community_id = $1 AND owner_pubkey = ANY($2) ORDER BY id LIMIT $3) \
             SELECT w.id, w.name, w.owner_pubkey, w.channel_id, w.status::text AS status, w.enabled, \
                    r.pending_runs, r.running_runs, r.waiting_approval_runs \
             FROM owned w \
             CROSS JOIN LATERAL (SELECT \
                 count(*) FILTER (WHERE status = 'pending') AS pending_runs, \
                 count(*) FILTER (WHERE status = 'running') AS running_runs, \
                 count(*) FILTER (WHERE status = 'waiting_approval') AS waiting_approval_runs \
                 FROM workflow_runs WHERE community_id = $1 AND workflow_id = w.id \
                   AND status IN ('pending', 'running', 'waiting_approval')) r \
             ORDER BY w.id",
        )
        .bind(community.as_uuid())
        .bind(&owners)
        .bind((LIMIT + 1) as i64)
        .fetch_all(&mut *tx)
        .await?;
        let workflows_truncated = rows.len() > LIMIT;
        rows.truncate(LIMIT);
        let workflows = rows
            .into_iter()
            .map(|row| {
                let owner: Vec<u8> = row.try_get("owner_pubkey")?;
                Ok(InventoryWorkflow {
                    id: row.try_get("id")?,
                    name: row.try_get("name")?,
                    owner_pubkey: hex::encode(owner),
                    channel_id: row.try_get("channel_id")?,
                    status: row.try_get("status")?,
                    enabled: row.try_get("enabled")?,
                    pending_runs: row.try_get("pending_runs")?,
                    running_runs: row.try_get("running_runs")?,
                    waiting_approval_runs: row.try_get("waiting_approval_runs")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(IdentityInventory {
            community_id: *community.as_uuid(),
            community_host,
            pubkey: hex::encode(root),
            identities,
            workflows,
            identities_truncated,
            workflows_truncated,
        })
    }
}
