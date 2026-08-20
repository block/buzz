//! Twilio SMS allow-list -- the `sms_identities` table.
//!
//! `allowed` gates whether an inbound SMS is admitted at all; `default_project`
//! is the NIP-MP project `d`-tag the sms-operator persona dispatches into when
//! the sender is unambiguous. See migrations/0032_sms_identities.sql.

use sqlx::{PgPool, Row};

use buzz_core::CommunityId;

use crate::error::Result;

/// A phone number's allow-list and project-routing state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsIdentity {
    /// E.164 phone number, the allow-list key.
    pub phone_number: String,
    /// Community this identity belongs to.
    pub community_id: CommunityId,
    /// Whether inbound SMS from this number is admitted at all.
    pub allowed: bool,
    /// Nostr pubkey this number is linked to, if any.
    pub linked_pubkey: Option<Vec<u8>>,
    /// NIP-MP project `d`-tag to dispatch into when unambiguous.
    pub default_project: Option<String>,
}

/// Look up a phone number's allow-list state. Returns `Ok(None)` for an
/// unknown number -- callers must treat "unknown" the same as "not allowed"
/// (fail closed), not as a distinct case worth a different response, so the
/// endpoint doesn't become an oracle for which numbers are registered.
///
/// Deliberately keyed on `phone_number` alone, without a community: the
/// inbound Twilio webhook has no tenant context to pass -- the community is
/// what this lookup *resolves*. Since the table is keyed
/// `(community_id, phone_number)`, two communities on one relay can in
/// principle register the same number, so the ordering below pins which one
/// wins (oldest registration, ties broken by community id) rather than
/// leaving it to the planner. A number claimed by two communities is a
/// provisioning mistake worth noticing, but it must at least route the same
/// way on every message instead of flapping between tenants.
pub async fn get_sms_identity(pool: &PgPool, phone_number: &str) -> Result<Option<SmsIdentity>> {
    let row = sqlx::query(
        r#"
        SELECT phone_number, community_id, allowed, linked_pubkey, default_project
        FROM sms_identities
        WHERE phone_number = $1
        ORDER BY created_at ASC, community_id ASC
        LIMIT 1
        "#,
    )
    .bind(phone_number)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    Ok(Some(SmsIdentity {
        phone_number: row.try_get("phone_number")?,
        community_id: CommunityId::from_uuid(row.try_get("community_id")?),
        allowed: row.try_get("allowed")?,
        linked_pubkey: row.try_get("linked_pubkey")?,
        default_project: row.try_get("default_project")?,
    }))
}

/// Set (or, with `project = None`, clear) a number's default project route.
///
/// Scoped by `community_id` as well as `phone_number` so a command signed in
/// one community can never re-route a number registered in another, even
/// though `phone_number` is the table's primary key.
///
/// Returns `false` when no row matched — an unknown or wrong-community number.
/// Deliberately does **not** insert: this updates the routing of an identity
/// that is already allow-listed, and must never be a way to add one. Admitting
/// a new number is a separate, more privileged decision.
pub async fn set_default_project(
    pool: &PgPool,
    community_id: CommunityId,
    phone_number: &str,
    project: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE sms_identities
        SET default_project = $3
        WHERE phone_number = $1 AND community_id = $2
        "#,
    )
    .bind(phone_number)
    .bind(community_id.as_uuid())
    .bind(project)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}
