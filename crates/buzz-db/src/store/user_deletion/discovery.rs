//! Shared read-only discovery of current community memberships.

use super::{UserDeletionStore, MAX_COMMUNITIES};
use crate::{DbError, Result};
use serde_json::{json, Value};
use uuid::Uuid;

/// Normalize a hex public key or NIP-19 npub to Buzz's lowercase hex form.
/// This does not resolve an email, account, or other keys owned by the person.
pub fn normalize_user_deletion_pubkey(input: &str) -> Result<String> {
    let is_hex = input.len() == 64 && input.bytes().all(|b| b.is_ascii_hexdigit());
    if !is_hex && !input.starts_with("npub1") {
        return Err(invalid_pubkey());
    }
    nostr::PublicKey::parse(input)
        .map(|key| key.to_hex())
        .map_err(|_| invalid_pubkey())
}

fn invalid_pubkey() -> DbError {
    DbError::DeletionSafety("expected a 64-character hex public key or an npub".into())
}

#[derive(sqlx::FromRow)]
struct Membership {
    community_id: Uuid,
    host: String,
    owned: bool,
    via_agent: bool,
}

impl UserDeletionStore {
    /// List current memberships and communities owned by the subject or a direct agent.
    pub async fn generate_inventory(&self, pubkey: &str) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        let inventory = generate_inventory_on(&mut tx, pubkey).await?;
        tx.commit().await?;
        Ok(inventory)
    }
}

/// Recheck ownership inside the caller's community write transaction.
pub(crate) async fn check_community_ownership(
    conn: &mut sqlx::PgConnection,
    target: &str,
    community: Uuid,
    expected_owner: bool,
) -> Result<()> {
    // Serving ownership changes take the shared form of this lock. A fresh
    // statement snapshot after acquiring it must see any writer we waited for.
    crate::deletion::lock_community_deletion(conn, crate::CommunityId::from_uuid(community))
        .await?;
    let (owned, isolation): (bool, String) = sqlx::query_as(concat!(
        "SELECT EXISTS(SELECT 1 FROM (",
        include_str!("community_owners.sql"),
        ") owners WHERE community_id = $2), current_setting('transaction_isolation')"
    ))
    .bind(target)
    .bind(community)
    .fetch_one(conn)
    .await?;
    if isolation != "read committed" {
        return Err(DbError::DeletionSafety(
            "ownership validation requires read committed isolation".into(),
        ));
    }
    if owned != expected_owner {
        return Err(ownership_changed(community));
    }
    Ok(())
}

fn ownership_changed(community: Uuid) -> DbError {
    DbError::DeletionSafety(format!(
        "ownership of community {community} differs from the approved inventory; review the ownership change before continuing"
    ))
}

impl UserDeletionStore {
    /// Verify the set of owned communities before the global media stage starts.
    pub async fn check_owned_inventory(
        &self,
        target: &str,
        inventory: &super::UserDeletionInventory,
    ) -> Result<()> {
        let approved: Vec<_> = inventory
            .communities
            .iter()
            .filter(|(_, entry)| entry["relationship"] == "owner")
            .map(|(id, _)| *id)
            .collect();
        // An independently submitted child may already have removed its owner
        // rows. The parent will observe that child without submitting another.
        let changed: Option<Uuid> = sqlx::query_scalar(concat!(
            "WITH owners AS (", include_str!("community_owners.sql"), ")
             SELECT community_id FROM owners WHERE NOT (community_id = ANY($2))
             UNION ALL
             SELECT id FROM unnest($2::uuid[]) AS approved(id)
             WHERE NOT EXISTS (SELECT 1 FROM owners WHERE community_id = approved.id)
               AND NOT EXISTS (SELECT 1 FROM community_deletion_requests WHERE community_id = approved.id)
             ORDER BY 1 LIMIT 1"
        ))
        .bind(target)
        .bind(approved)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(id) = changed {
            return Err(ownership_changed(id));
        }
        Ok(())
    }
}

// Submission uses its existing transaction, including when the pool has one connection.
pub(super) async fn generate_inventory_on(
    conn: &mut sqlx::PgConnection,
    pubkey: &str,
) -> Result<Value> {
    let pubkey = normalize_user_deletion_pubkey(pubkey)?;
    sqlx::query("SET LOCAL statement_timeout = '30s'")
        .execute(&mut *conn)
        .await?;
    // Operator-global discovery: deliberately do not apply the product's
    // active-community filters. One statement gives one membership snapshot.
    let rows = sqlx::query_as::<_, Membership>(concat!(
        "WITH owners AS (",
        include_str!("community_owners.sql"),
        "),
             matches AS (
                 SELECT community_id, true AS owned, via_agent FROM owners
                 UNION ALL SELECT community_id, false, false FROM relay_members WHERE pubkey = $1
                 UNION ALL SELECT DISTINCT community_id, false, false FROM channel_members
                      WHERE pubkey = decode($1, 'hex') AND removed_at IS NULL
             )
             SELECT c.id AS community_id, c.host, bool_or(m.owned) AS owned,
                    bool_or(m.via_agent) AS via_agent
             FROM matches m JOIN communities c ON c.id = m.community_id
             GROUP BY c.id ORDER BY c.id LIMIT $2"
    ))
    .bind(&pubkey)
    .bind((MAX_COMMUNITIES + 1) as i64)
    .fetch_all(&mut *conn)
    .await?;
    if rows.len() > MAX_COMMUNITIES {
        return Err(DbError::DeletionSafety(format!(
            "inventory exceeds {MAX_COMMUNITIES} communities; no partial inventory returned"
        )));
    }
    let communities = rows
        .into_iter()
        .map(|row| {
            let mut entry = json!({
                "community_id": row.community_id,
                "label": row.host,
                "relationship": if row.owned { "owner" } else { "participant" },
            });
            if row.via_agent {
                entry["owned_by_agent"] = true.into();
            }
            entry
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "communities": communities,
        "workflow_version": 2,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::ToBech32;

    #[test]
    fn public_key_formats_identify_the_same_subject() {
        let hex = "5e4083a7a793d02575876eaf47932b4835999f5af2653d26250a909b450da7c9";
        let npub = nostr::PublicKey::parse(hex).unwrap().to_bech32().unwrap();
        for input in [hex.to_owned(), hex.to_uppercase(), npub] {
            assert_eq!(normalize_user_deletion_pubkey(&input).unwrap(), hex);
        }
        for input in [
            "",
            "alice@example.com",
            "demo:alice",
            "npub1invalid",
            "abcdef",
        ] {
            assert!(normalize_user_deletion_pubkey(input).is_err());
        }
    }
}
