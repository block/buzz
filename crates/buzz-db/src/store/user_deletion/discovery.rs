//! Read-only discovery of a public key's current community memberships.

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

// Identity resolution belongs to intake, never approval or workflow recovery.
pub(super) fn normalize_target(input: &str) -> Result<String> {
    if let Some(slug) = input.strip_prefix("demo:") {
        if slug.is_empty()
            || slug.len() > 64
            || !slug.as_bytes()[0].is_ascii_alphanumeric()
            || !slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(DbError::DeletionSafety("invalid demo target label".into()));
        }
        return Ok(input.to_owned());
    }
    normalize_user_deletion_pubkey(input)
}

fn demo_inventory() -> serde_json::Value {
    serde_json::json!({
        "communities": [
            {
                "community_id": "00000000-0000-0000-0000-00000000000b",
                "label": "demo:community-b",
                "relationship": "participant"
            },
            {
                "community_id": "00000000-0000-0000-0000-00000000000c",
                "label": "demo:community-c",
                "relationship": "former_participant"
            }
        ]
    })
}

#[derive(sqlx::FromRow)]
struct Membership {
    community_id: Uuid,
    host: String,
    role: Option<String>,
}

impl UserDeletionStore {
    /// Generate only community identities and relationships for demo or real input.
    /// `demo` selects the existing fixture; other inputs accept hex or npub.
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

// Submission uses its existing transaction, including when the pool has one connection.
pub(super) async fn generate_inventory_on(
    conn: &mut sqlx::PgConnection,
    pubkey: &str,
) -> Result<Value> {
    if pubkey == "demo" {
        return Ok(demo_inventory());
    }
    let pubkey = normalize_target(pubkey)?;
    if pubkey.starts_with("demo:") {
        return Ok(demo_inventory());
    }
    sqlx::query("SET LOCAL statement_timeout = '30s'")
        .execute(&mut *conn)
        .await?;
    // Operator-global discovery: deliberately do not apply the product's
    // active-community filters. One statement gives one membership snapshot.
    let rows = sqlx::query_as::<_, Membership>(
        "WITH current_channels AS ( \
                 SELECT DISTINCT community_id FROM channel_members \
                 WHERE pubkey = decode($1, 'hex') AND removed_at IS NULL \
             ), matched_communities AS ( \
                 SELECT community_id FROM relay_members WHERE pubkey = $1 \
                 UNION SELECT community_id FROM current_channels \
             ) \
             SELECT c.id AS community_id, c.host, m.role \
             FROM matched_communities matched \
             JOIN communities c ON c.id = matched.community_id \
             LEFT JOIN relay_members m ON m.community_id = c.id AND m.pubkey = $1 \
             ORDER BY c.id LIMIT $2",
    )
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
            let relationship = match row.role.as_deref() {
                Some("owner") => "owner",
                Some("admin" | "member") | None => "participant",
                _ => {
                    return Err(DbError::DeletionSafety(
                        "unsupported membership role".into(),
                    ))
                }
            };
            Ok(json!({
                "community_id": row.community_id,
                "label": row.host,
                "relationship": relationship,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "communities": communities,
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
