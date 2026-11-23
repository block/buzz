//! Shared read-only discovery of communities with user cleanup candidates.

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
        self.generate_inventory_with_relay(pubkey, None).await
    }

    /// Generate the same inventory used by both submission modes, including
    /// historical content attributed by an optional trusted relay signer.
    pub async fn generate_inventory_with_relay(
        &self,
        pubkey: &str,
        relay_pubkey: Option<&str>,
    ) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        let inventory = generate_inventory_on(&mut tx, pubkey, relay_pubkey).await?;
        tx.commit().await?;
        Ok(inventory)
    }
}

// Select communities with cleanup candidates, including residue after leaving.
// Agent ownership is community-local: an owned users row is itself a candidate,
// so there is no need to expand those keys into other communities here.
// Handlers still validate attribution and discover their exact current rows.
pub(super) async fn generate_inventory_on(
    conn: &mut sqlx::PgConnection,
    subject: &str,
    relay_pubkey: Option<&str>,
) -> Result<Value> {
    let subject = if subject == "demo" {
        "demo:default"
    } else {
        subject
    };
    let subject = normalize_target(subject)?;
    let relay_pubkey = normalize_relay(&subject, relay_pubkey)?;
    if subject.starts_with("demo:") {
        return Ok(demo_inventory());
    }
    sqlx::query("SET LOCAL statement_timeout='30s'")
        .execute(&mut *conn)
        .await?;
    // Closed SQL fragments only; all operator input stays bound. Reusing the
    // planner predicate keeps legacy actor/p and private-state ownership aligned.
    let attributed = include_str!("content_batches/authored.sql")
        .replace("$1", "c.id")
        .replace("$2", "ARRAY[decode($1, 'hex')]");
    let query =
        include_str!("inventory_discovery.sql").replace("/* ATTRIBUTED_CONTENT */", &attributed);
    let rows = sqlx::query_as::<_, Membership>(sqlx::AssertSqlSafe(query))
        .bind(&subject)
        .bind((MAX_COMMUNITIES + 1) as i64)
        .bind(relay_pubkey.as_deref())
        .fetch_all(conn)
        .await?;
    if rows.len() > MAX_COMMUNITIES {
        return Err(DbError::DeletionSafety(format!(
            "inventory exceeds {MAX_COMMUNITIES} communities; no partial inventory returned"
        )));
    }
    let rows = rows.into_iter().map(|row| {
        let relationship = match row.role.as_deref() {
            Some("owner") => "owner",
            Some("admin" | "member") => "participant",
            None => "past_participant",
            _ => return Err(DbError::DeletionSafety("unsupported membership role".into())),
        };
        Ok(json!({"community_id":row.community_id,"label":row.host,"relationship":relationship}))
    }).collect::<Result<Vec<_>>>()?;
    let mut inventory = json!({"communities":rows,"scope":"affected_communities"});
    if let Some(key) = relay_pubkey {
        inventory["discovery_relay_pubkey"] = key.into();
    }
    Ok(inventory)
}

// Normalize attribution once for preview, submission and idempotency checks.
pub(super) fn normalize_relay(subject: &str, relay: Option<&str>) -> Result<Option<String>> {
    let relay = relay.map(normalize_user_deletion_pubkey).transpose()?;
    if relay.is_some() && subject.starts_with("demo:") {
        return Err(DbError::DeletionSafety(
            "trusted relay is only valid for real public keys".into(),
        ));
    }
    if relay.as_deref() == Some(subject) {
        return Err(DbError::DeletionSafety(
            "the human and trusted relay public keys must differ".into(),
        ));
    }
    Ok(relay)
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
