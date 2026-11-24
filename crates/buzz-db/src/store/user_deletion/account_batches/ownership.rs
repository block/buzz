//! Direct ownership is rediscovered per handler and checked in each batch.
use super::*;

impl UserDeletionStore {
    /// Fetch the target's current directly owned agents in this community, then
    /// print or apply bounded account batches. Retries repeat discovery.
    /// Workflow callers supply the human's current lease and stage.
    pub async fn run_account_batches(
        &self,
        options: &AccountBatchOptions,
        mode: ExecutionMode,
        writer: &mut impl Write,
        stage: Option<(&crate::entity_deletion::EntityLease, Uuid)>,
    ) -> Result<AccountBatchSummary> {
        let mut current = validated(options)?;
        let mut tx = self.pool.begin().await?;
        timeouts(&mut tx).await?;
        current.agents = agents(&mut tx, options.community, &current.target, false).await?;
        tx.rollback().await?;
        let keys: Vec<String> = std::iter::once(current.target.clone())
            .chain(current.agents.iter().cloned())
            .collect();
        if mode == ExecutionMode::Delete {
            self.preflight_memberships(options.community, &keys).await?;
        }
        let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=ANY($2) AND role='owner')")
            .bind(options.community).bind(&keys).fetch_one(&self.pool).await?;
        if owned {
            return Err(invalid(
                "owned-community handoff required; account handler was not executed",
            ));
        }
        line(
            writer,
            &json!({"record":"account_targets", "community":options.community,
            "target":current.target, "agent_count":current.agents.len(), "read_only":mode == ExecutionMode::ReadOnly}),
        )?;
        let mut total = AccountBatchSummary {
            record: "account_handler_summary",
            read_only: mode == ExecutionMode::ReadOnly,
            items: 0,
            batches: 0,
            supported_scope_exhausted: true,
            account_erasure_complete: false,
        };
        for key in keys {
            if total.items == options.max_items || !total.supported_scope_exhausted {
                total.supported_scope_exhausted = false;
                break;
            }
            current.pubkey = key;
            current.max_items = options.max_items - total.items;
            let result = self
                .run_batches(&current, OPERATIONS, mode, writer, stage)
                .await?;
            total.items += result.items;
            total.batches += result.batches;
            total.supported_scope_exhausted = result.supported_scope_exhausted;
        }
        // Include additions as well as transfers away: a changed set requires a
        // retry, which discovers it afresh. Never claim those agents were cleaned.
        let mut tx = self.pool.begin().await?;
        timeouts(&mut tx).await?;
        recheck(&mut tx, &current, false).await?;
        tx.rollback().await?;
        line(writer, &total)?;
        Ok(total)
    }
}

pub(super) async fn agents(
    conn: &mut PgConnection,
    community: Uuid,
    target: &str,
    lock: bool,
) -> Result<Vec<String>> {
    Ok(super::super::agent_ownership::discover(
        conn,
        target,
        Some(community),
        lock,
        super::super::agent_ownership::DiscoveryLimit::AgentRows,
    )
    .await?
    .agents
    .into_iter()
    .map(|(_, key)| key)
    .collect())
}

pub(super) async fn recheck(conn: &mut PgConnection, options: &Options, lock: bool) -> Result<()> {
    if let Some(community) = options.community {
        if agents(conn, community, &options.target, lock).await? != options.agents {
            return Err(invalid(
                "agent ownership changed; rerun the handler to fetch current agents",
            ));
        }
    }
    Ok(())
}
