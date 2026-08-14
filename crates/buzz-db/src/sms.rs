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
pub async fn get_sms_identity(pool: &PgPool, phone_number: &str) -> Result<Option<SmsIdentity>> {
    let row = sqlx::query(
        r#"
        SELECT phone_number, community_id, allowed, linked_pubkey, default_project
        FROM sms_identities
        WHERE phone_number = $1
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
