//! Fail-closed compatibility adapter for the direct-final identity foundation.
//!
//! The former auth-time enrollment/lifecycle implementation wrote the legacy
//! `uid`, `pubkey`, `display_name`, `source`, and `revoked_at` schema. Migration
//! 0029 deliberately replaces that authority with immutable, receipt-backed
//! binding generations. Until root callers are moved to the NIP-FI resolver
//! and lifecycle transaction, every legacy mutation entry point fails closed.

use std::fmt;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::error::{DbError, Result};
use buzz_core::CommunityId;

/// Legacy source label retained only for source-compatible root callers.
pub const SOURCE_JWT_NPUB: &str = "jwt_npub";
/// Legacy source label retained only for source-compatible root callers.
pub const SOURCE_DB_BINDING: &str = "db_binding";

const LEGACY_MUTATION_DISABLED: &str =
    "legacy identity mutation authority is disabled; use the NIP-FI lifecycle transaction";

/// Read-only compatibility projection of an active immutable binding.
#[derive(Clone, PartialEq, Eq)]
pub struct IdentityBinding {
    /// Validated identity-provider issuer.
    pub issuer: String,
    /// Opaque issuer-qualified subject (legacy field name).
    pub uid: String,
    /// Bound Nostr event-author key bytes.
    pub pubkey: Vec<u8>,
    /// Direct-final storage never persists display claims.
    pub display_name: Option<String>,
    /// Closed provenance label, never a provider/runtime authority.
    pub source: String,
    /// Immutable generation creation time.
    pub created_at: DateTime<Utc>,
    /// Compatibility timestamp equal to `created_at`.
    pub updated_at: DateTime<Utc>,
    /// Compatibility timestamp equal to `created_at`.
    pub last_seen_at: DateTime<Utc>,
}

impl fmt::Debug for IdentityBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IdentityBinding([REDACTED])")
    }
}

/// Existing active binding that conflicts with a requested binding.
#[derive(Clone, PartialEq, Eq)]
pub struct IdentityBindingConflict {
    /// Existing active issuer.
    pub issuer: String,
    /// Existing active opaque subject.
    pub uid: String,
    /// Existing active event-author key.
    pub pubkey: Vec<u8>,
    /// Existing closed provenance label.
    pub source: String,
}

impl fmt::Debug for IdentityBindingConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IdentityBindingConflict([REDACTED])")
    }
}

/// Legacy mutation result retained for root source compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindIdentityResult {
    /// Unreachable through the disabled compatibility mutation path.
    Created,
    /// Unreachable through the disabled compatibility mutation path.
    Matched,
    /// A conflicting immutable binding was observed.
    Conflict(IdentityBindingConflict),
    /// The operation was rejected by lifecycle state.
    Revoked,
}

/// Legacy staged identity input retained only until root integration changes.
#[derive(Clone, Copy)]
pub struct IdentityBindingInput<'a> {
    /// Canonically verified issuer.
    pub issuer: &'a str,
    /// Canonically verified opaque subject.
    pub uid: &'a str,
    /// Authenticated event-author key.
    pub pubkey: &'a [u8],
    /// Ephemeral display claim; never persisted by the direct-final schema.
    pub display_name: Option<&'a str>,
    /// Legacy source label; never used as authority.
    pub source: &'a str,
}

impl fmt::Debug for IdentityBindingInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IdentityBindingInput([REDACTED])")
    }
}

fn validate_inputs(issuer: &str, subject: &str, pubkey: &[u8], source: &str) -> Result<()> {
    if issuer.trim().is_empty() || subject.trim().is_empty() {
        return Err(DbError::InvalidData(
            "identity binding issuer and subject must not be empty".to_owned(),
        ));
    }
    validate_pubkey(pubkey)?;
    if !matches!(source, SOURCE_JWT_NPUB | SOURCE_DB_BINDING) {
        return Err(DbError::InvalidData(
            "invalid legacy identity binding source".to_owned(),
        ));
    }
    Ok(())
}

fn validate_pubkey(pubkey: &[u8]) -> Result<()> {
    if pubkey.len() != 32 {
        return Err(DbError::InvalidData(
            "identity binding pubkey must be 32 bytes".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_membership_identity_key(
    member_pubkey_hex: &str,
    identity: Option<&IdentityBindingInput<'_>>,
) -> Result<()> {
    let Some(identity) = identity else {
        return Ok(());
    };
    let member_pubkey = hex::decode(member_pubkey_hex)
        .map_err(|_| DbError::InvalidData("membership pubkey must be 32-byte hex".to_owned()))?;
    if member_pubkey.len() != 32 || member_pubkey.as_slice() != identity.pubkey {
        return Err(DbError::InvalidData(
            "membership pubkey does not match staged identity key".to_owned(),
        ));
    }
    Ok(())
}

/// Legacy auth-time enrollment is disconnected from the direct-final schema.
#[allow(clippy::too_many_arguments)]
pub async fn bind_or_validate_identity(
    _pool: &PgPool,
    _community_id: CommunityId,
    issuer: &str,
    subject: &str,
    pubkey: &[u8],
    _display_name: Option<&str>,
    source: &str,
) -> Result<BindIdentityResult> {
    validate_inputs(issuer, subject, pubkey, source)?;
    Err(DbError::AccessDenied(LEGACY_MUTATION_DISABLED.to_owned()))
}

/// Caller-owned legacy admission transactions also fail before any SQL write.
pub(crate) async fn bind_or_validate_identity_tx(
    _tx: &mut Transaction<'_, Postgres>,
    _community_id: CommunityId,
    identity: &IdentityBindingInput<'_>,
) -> Result<BindIdentityResult> {
    validate_inputs(
        identity.issuer,
        identity.uid,
        identity.pubkey,
        identity.source,
    )?;
    Err(DbError::AccessDenied(LEGACY_MUTATION_DISABLED.to_owned()))
}

/// Read the exact active direct-final binding for an event-author key.
pub async fn get_active_identity_binding_by_pubkey(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<IdentityBinding>> {
    validate_pubkey(pubkey)?;
    let row = sqlx::query(
        r#"
        SELECT issuer, subject, event_author_pubkey, binding_provenance, created_at
        FROM identity_bindings
        WHERE community_id = $1
          AND event_author_pubkey = $2
          AND binding_state = 1
          AND (expires_at IS NULL OR transaction_timestamp() < expires_at)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let provenance: i16 = row.try_get("binding_provenance")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let source = match provenance {
            1 => "attested",
            2 => "provisioned",
            3 => "risk_labelled_tofu",
            _ => {
                return Err(DbError::InvalidData(
                    "invalid identity binding provenance".to_owned(),
                ));
            }
        };
        Ok(IdentityBinding {
            issuer: row.try_get("issuer")?,
            uid: row.try_get("subject")?,
            pubkey: row.try_get("event_author_pubkey")?,
            display_name: None,
            source: source.to_owned(),
            created_at,
            updated_at: created_at,
            last_seen_at: created_at,
        })
    })
    .transpose()
}

/// Legacy principal revocation is replaced by receipt-backed lifecycle state.
pub async fn revoke_identity_principal(
    _pool: &PgPool,
    _community_id: CommunityId,
    _issuer: &str,
    _subject: &str,
    _revoked_by: Option<&[u8]>,
    _reason: &str,
) -> Result<bool> {
    Err(DbError::AccessDenied(LEGACY_MUTATION_DISABLED.to_owned()))
}

/// Legacy key revocation is replaced by receipt-backed lifecycle state.
pub async fn revoke_identity_key(
    _pool: &PgPool,
    _community_id: CommunityId,
    pubkey: &[u8],
    _revoked_by: Option<&[u8]>,
    _reason: &str,
) -> Result<bool> {
    validate_pubkey(pubkey)?;
    Err(DbError::AccessDenied(LEGACY_MUTATION_DISABLED.to_owned()))
}

/// Legacy rotation is replaced by the direct-final lifecycle transaction.
#[allow(clippy::too_many_arguments)]
pub async fn rotate_identity_binding(
    _pool: &PgPool,
    _community_id: CommunityId,
    issuer: &str,
    subject: &str,
    old_pubkey: &[u8],
    new_pubkey: &[u8],
    _display_name: Option<&str>,
    source: &str,
    _rotated_by: Option<&[u8]>,
    _reason: &str,
) -> Result<()> {
    validate_inputs(issuer, subject, old_pubkey, source)?;
    validate_pubkey(new_pubkey)?;
    Err(DbError::AccessDenied(LEGACY_MUTATION_DISABLED.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_debug_is_redacted() {
        let input = IdentityBindingInput {
            issuer: "secret-issuer",
            uid: "secret-subject",
            pubkey: &[7; 32],
            display_name: Some("secret-name"),
            source: SOURCE_JWT_NPUB,
        };
        assert_eq!(format!("{input:?}"), "IdentityBindingInput([REDACTED])");
    }

    #[test]
    fn malformed_compatibility_inputs_fail_before_io() {
        assert!(validate_inputs("", "subject", &[7; 32], SOURCE_JWT_NPUB).is_err());
        assert!(validate_inputs("issuer", "", &[7; 32], SOURCE_JWT_NPUB).is_err());
        assert!(validate_inputs("issuer", "subject", &[7; 31], SOURCE_JWT_NPUB).is_err());
        assert!(validate_inputs("issuer", "subject", &[7; 32], "provider").is_err());
    }
}
