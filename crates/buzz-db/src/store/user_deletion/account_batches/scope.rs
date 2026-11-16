//! Community selection for the offline account command, independent of membership.
use super::*;
use serde::Deserialize;
use std::collections::BTreeSet;

const MAX_TARGETS: usize = 10_000;

/// Saved account cleanup inputs. Agent keys are a discovery snapshot only;
/// each handler fetches current ownership. Keep ownership rows until cleanup ends.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountScope {
    /// Current manifest version.
    pub version: u32,
    /// Human key, canonical lowercase hex.
    pub target: String,
    /// Every stored community, including ones with no current membership.
    pub communities: Vec<AccountCommunityScope>,
    /// Explicit retention limits; these are not proof of full erasure.
    pub retained: Vec<String>,
}

/// Frozen human and directly owned agent keys in one community.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCommunityScope {
    /// Database community identity.
    pub community: Uuid,
    /// Human plus agents whose ownership was recorded in this community.
    pub pubkeys: Vec<String>,
}

impl AccountScope {
    /// Reject malformed, oversized or duplicate scope before any mutation.
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || normalize_user_deletion_pubkey(&self.target)? != self.target {
            return Err(invalid(
                "unsupported account scope version or noncanonical target",
            ));
        }
        if self.communities.len() > super::super::MAX_COMMUNITIES {
            return Err(invalid("account scope exceeds community limit"));
        }
        let mut communities = BTreeSet::new();
        let mut count = 0;
        for row in &self.communities {
            let mut keys = BTreeSet::new();
            if !communities.insert(row.community) || !row.pubkeys.contains(&self.target) {
                return Err(invalid(
                    "duplicate community or missing human in account scope",
                ));
            }
            for key in &row.pubkeys {
                count += 1;
                if count > MAX_TARGETS
                    || normalize_user_deletion_pubkey(key)? != *key
                    || !keys.insert(key)
                {
                    return Err(invalid(
                        "invalid, duplicate or excessive account scope keys",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl UserDeletionStore {
    /// Read-only account scope capture. Scan all communities rather than infer
    /// residue from current memberships. Agent authority is community-scoped;
    /// global agent records are retained where exclusive ownership is unproven.
    pub async fn generate_account_scope(&self, pubkey: &str) -> Result<AccountScope> {
        let target = normalize_user_deletion_pubkey(pubkey)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='30s'")
            .execute(&mut *tx)
            .await?;
        let communities: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM communities ORDER BY id LIMIT $1")
                .bind((super::super::MAX_COMMUNITIES + 1) as i64)
                .fetch_all(&mut *tx)
                .await?;
        if communities.len() > super::super::MAX_COMMUNITIES {
            return Err(invalid("account scope exceeds community limit"));
        }
        let agents: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT community_id,encode(pubkey,'hex') FROM users WHERE agent_owner_pubkey=decode($1,'hex') AND pubkey<>decode($1,'hex') ORDER BY community_id,pubkey LIMIT $2")
            .bind(&target).bind((MAX_TARGETS + 1) as i64).fetch_all(&mut *tx).await?;
        if agents.len() > MAX_TARGETS {
            return Err(invalid("account scope exceeds target limit"));
        }
        let scope = AccountScope {
            version: 1,
            target: target.clone(),
            communities: communities.into_iter().map(|community| {
                let mut pubkeys = vec![target.clone()];
                pubkeys.extend(agents.iter().filter(|(id,_)| *id == community).map(|(_,key)| key.clone()));
                AccountCommunityScope { community, pubkeys }
            }).collect(),
            retained: vec!["global agent records: ownership is community-scoped; no exclusive deployment-wide attribution".into(),
                "shared ownership, moderation/audit, device registrations and historical signed membership snapshots: see operator-user-account-retention.md".into()],
        };
        scope.validate()?;
        tx.commit().await?;
        Ok(scope)
    }

    /// Execute the selected communities with one invocation-wide row budget.
    /// Saved agent keys never authorize deletion; each handler fetches them again.
    /// This direct offline command does not bypass or rewrite workflow approval.
    pub async fn run_account_scope(
        &self,
        scope: &AccountScope,
        actor: &str,
        batch_size: u16,
        max_items: u32,
        mode: ExecutionMode,
        writer: &mut impl Write,
    ) -> Result<AccountBatchSummary> {
        scope.validate()?;
        bounds(batch_size, max_items)?;
        let actor = normalize_user_deletion_pubkey(actor)?;
        // Whole-community ownership must go through its existing handoff. Check
        // every scope before global work so a later owner cannot cause partial
        // cleanup of a plan that cannot be executed safely.
        for row in &scope.communities {
            let mut tx = self.pool.begin().await?;
            timeouts(&mut tx).await?;
            let mut keys = vec![scope.target.clone()];
            keys.extend(ownership::agents(&mut tx, row.community, &scope.target, false).await?);
            tx.rollback().await?;
            if mode == ExecutionMode::Delete {
                self.preflight_memberships(row.community, &keys).await?;
            }
            let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=ANY($2) AND role='owner')")
                .bind(row.community).bind(&keys).fetch_one(&self.pool).await?;
            if owned {
                return Err(invalid(
                    "owned-community handoff required; account scope was not executed",
                ));
            }
        }
        let mut total = AccountBatchSummary {
            record: "account_scope_summary",
            read_only: mode == ExecutionMode::ReadOnly,
            items: 0,
            batches: 0,
            supported_scope_exhausted: true,
            account_erasure_complete: false,
        };
        line(
            writer,
            &json!({"record":"account_scope_start","target":scope.target,"communities":scope.communities.len(),"retained":scope.retained,"read_only":total.read_only}),
        )?;
        // Global work runs even with no communities. Never hide it in a first
        // community stage or choose a fabricated community UUID.
        let summary = self
            .run_global_account_batches(
                &GlobalAccountBatchOptions {
                    pubkey: scope.target.clone(),
                    actor,
                    batch_size,
                    max_items,
                },
                mode,
                writer,
            )
            .await?;
        total.items += summary.items;
        total.batches += summary.batches;
        total.supported_scope_exhausted = summary.supported_scope_exhausted;
        for row in &scope.communities {
            if !total.supported_scope_exhausted || total.items == max_items {
                total.supported_scope_exhausted = false;
                break;
            }
            let summary = self
                .run_account_batches(
                    &AccountBatchOptions {
                        community: row.community,
                        pubkey: scope.target.clone(),
                        batch_size,
                        max_items: max_items - total.items,
                    },
                    mode,
                    writer,
                    None,
                )
                .await?;
            total.items += summary.items;
            total.batches += summary.batches;
            total.supported_scope_exhausted = summary.supported_scope_exhausted;
        }
        line(writer, &total)?;
        Ok(total)
    }
}
