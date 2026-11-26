//! Community membership and its signed NIP-43 roster commit together.
use super::*;
use buzz_core::CommunityId;

impl UserDeletionStore {
    pub(super) async fn preflight_community_membership(
        &self,
        conn: &mut PgConnection,
        community: Uuid,
        targets: &[String],
    ) -> Result<()> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=ANY($2))",
        )
        .bind(community)
        .bind(targets)
        .fetch_one(&mut *conn)
        .await?;
        if exists {
            let signer = self.account_snapshot_signer.as_ref().ok_or_else(|| {
                invalid("membership cleanup requires the existing relay signing key (BUZZ_RELAY_PRIVATE_KEY)")
            })?;
            check_signer(conn, community, signer).await?;
        }
        Ok(())
    }

    pub(super) async fn delete_community_membership(
        &self,
        options: &Options,
        op: &Operation,
        writer: &mut impl Write,
        stage: Option<(&crate::entity_deletion::EntityLease, Uuid)>,
        summary: &mut AccountBatchSummary,
    ) -> Result<()> {
        let community = options
            .community
            .ok_or_else(|| invalid("membership cleanup requires a community"))?;
        let mut raw = self.pool.begin().await?;
        timeouts(&mut raw).await?;
        if let Some((lease, id)) = stage {
            validate_stage(&mut raw, lease, id, options).await?;
        }
        ownership::recheck(&mut raw, options, true).await?;
        // Admission precedes the member row lock, matching membership writers.
        let mut tx = crate::AdmittedTx::admit(
            raw,
            &crate::deletion::DeletionStore::new(self.pool.clone()),
            CommunityId::from_uuid(community),
        )
        .await?;
        let keys = select(tx.conn(), options, op, None, true, summary).await?;
        if keys.is_empty() {
            tx.commit().await?;
            return Ok(());
        }
        if summary.items == options.max_items {
            summary.supported_scope_exhausted = false;
            return Ok(());
        }
        let signer = self
            .account_snapshot_signer
            .as_ref()
            .ok_or_else(|| invalid("missing relay signing key"))?;
        check_signer(tx.conn(), community, signer).await?;
        batch(writer, options, op, &keys, summary)?;
        // The member row is locked, so no new acceptance can be recorded
        // (`record_join_policy_acceptance` locks it `FOR KEY SHARE`). Delete
        // the ones recorded after the acceptance batch ran, in the same
        // transaction as the member, as `delete_member_with_acceptances` does.
        // A rollback below keeps them with the member.
        sqlx::query("DELETE FROM join_policy_acceptances WHERE community_id=$1 AND pubkey=$2")
            .bind(community)
            .bind(&options.pubkey)
            .execute(tx.conn())
            .await?;
        // The selected row is locked: a concurrent ownership grant cannot turn
        // this target into an owner between this predicate and the delete.
        let removed = sqlx::query(
            "DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2 AND role<>'owner'",
        )
        .bind(community)
        .bind(&options.pubkey)
        .execute(tx.conn())
        .await?
        .rows_affected();
        if removed != 1 {
            return Err(invalid(
                "community owner handoff required; membership removal rolled back",
            ));
        }
        if let Some((lease, id)) = stage {
            validate_stage(tx.conn(), lease, id, options).await?;
        }
        // Roster lock is last, as in the normal membership write protocol.
        crate::relay_members::write_nip43_membership_snapshot(&mut tx, signer).await?;
        tx.commit().await?;
        summary.items += 1;
        summary.batches += 1;
        line(
            writer,
            &json!({"record":"batch_committed", "number":summary.batches,
                "table":op.table,"action":op.action,"items":1,"saved_community_roster":true}),
        )
    }
}

async fn check_signer(
    conn: &mut PgConnection,
    community: Uuid,
    signer: &nostr::Keys,
) -> Result<()> {
    let mismatch: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND kind=13534
         AND channel_id IS NULL AND deleted_at IS NULL AND pubkey<>$2)",
    )
    .bind(community)
    .bind(signer.public_key().to_bytes().as_slice())
    .fetch_one(conn)
    .await?;
    if mismatch {
        return Err(invalid("relay signing key differs from existing community roster; membership removal rolled back"));
    }
    Ok(())
}
