//! Durable output-only NIP-FI verified profiles. Authorization never reads
//! these tables.
//!
//! A profile is a historical record ("verified as Alice at T"). It is never
//! retracted: on a relay that enforces NIP-FI, a key cannot post without a
//! fresh issuer assertion, so a stale name labels only past activity.

use crate::{AdmittedTx, Db, DbError, Result};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use sqlx::Row;

/// Operator-selected publication policy for one issuer.
#[derive(Clone)]
pub struct ProfilePolicy {
    /// Exact private issuer identity.
    pub issuer: String,
    /// Validated public label; `None` disables publication for this issuer.
    pub label: Option<String>,
    /// Unchanged-name refresh interval.
    pub refresh_seconds: i64,
}

impl std::fmt::Debug for ProfilePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProfilePolicy([REDACTED])")
    }
}

/// Verified input from a successful human WebSocket admission.
pub struct ProfileAdmission<'a> {
    /// Community resolved for this connection.
    pub community: CommunityId,
    /// Issuer authenticated by the assertion verifier.
    pub issuer: &'a str,
    /// NIP-42-proven key, equal to the assertion key.
    pub pubkey: &'a str,
    /// Authenticated iat; retains fractional ordering.
    pub issued_at: DateTime<Utc>,
    /// Actual successful admission time.
    pub admitted_at: DateTime<Utc>,
    /// Validated public display name.
    pub display_name: &'a str,
    /// Validated public username.
    pub username: &'a str,
    /// Validated optional public full name.
    pub full_name: Option<&'a str>,
    /// Public issuer label from the enabled policy.
    pub label: &'a str,
    /// Unchanged-name refresh interval from the enabled policy.
    pub refresh_seconds: i64,
    /// Whether the relay requires an explicit `relay_members` row to connect.
    /// Open relays treat enforced NIP-FI admission as eligibility instead.
    pub membership_required: bool,
}

async fn lock_profile(tx: &mut AdmittedTx, pubkey: &str) -> Result<()> {
    let key = format!("profile:{}:{pubkey}", tx.community().as_uuid());
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 30382))")
        .bind(key)
        .execute(tx.conn())
        .await?;
    Ok(())
}

impl Db {
    /// Publish or refresh a verified profile in one admitted transaction.
    ///
    /// Skips banned keys, keys without a member row where the relay requires
    /// one, assertions older than the issuer's last recorded one, and
    /// admissions earlier than the published `verified_at`. Signs a new event
    /// only when the issuer, names or label changed or the refresh interval
    /// passed, so the latest verification from any issuer wins. Returns the
    /// new event for delivery, or `None` when nothing was written.
    pub async fn publish_verified_profile(
        &self,
        input: ProfileAdmission<'_>,
        keys: &Keys,
    ) -> Result<Option<Event>> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            input.community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(tx.conn())
            .await?;
        lock_profile(&mut tx, input.pubkey).await?;

        let access = sqlx::query(
            r#"
            SELECT
                EXISTS (
                    SELECT 1 FROM relay_members
                    WHERE community_id = $1 AND pubkey = $2
                ) AS member,
                EXISTS (
                    SELECT 1 FROM community_bans
                    WHERE community_id = $1 AND pubkey = decode($2, 'hex')
                      AND banned AND (ban_expires_at IS NULL OR ban_expires_at > now())
                ) AS banned
            "#,
        )
        .bind(input.community.as_uuid())
        .bind(input.pubkey)
        .fetch_one(tx.conn())
        .await?;
        if access.try_get::<bool, _>("banned")?
            || (input.membership_required && !access.try_get::<bool, _>("member")?)
        {
            return Ok(None);
        }

        let head = sqlx::query(
            r#"
            SELECT issuer, issued_at::text AS issued_at, event
            FROM nip_fi_profile_heads
            WHERE community_id = $1 AND pubkey = $2
            FOR UPDATE
            "#,
        )
        .bind(input.community.as_uuid())
        .bind(input.pubkey)
        .fetch_optional(tx.conn())
        .await?;
        let (old, same_issuer) = match &head {
            Some(row) => {
                let same_issuer = row.try_get::<String, _>("issuer")? == input.issuer;
                // Assertion times are only comparable within one issuer.
                if same_issuer && nanos(input.issued_at) < parse_nanos(row.try_get("issued_at")?)? {
                    return Ok(None);
                }
                let event =
                    Event::from_json(row.try_get::<serde_json::Value, _>("event")?.to_string())
                        .map_err(|_| DbError::InvalidData("profile event".into()))?;
                // Public verified_at never moves backwards.
                if verified_at(&event).is_some_and(|at| input.admitted_at.timestamp() < at) {
                    return Ok(None);
                }
                (Some(event), same_issuer)
            }
            None => (None, false),
        };

        let issued_at = nanos(input.issued_at).to_string();
        let changed = !same_issuer
            || old.as_ref().is_none_or(|event| {
                tag(event, "display_name") != Some(input.display_name)
                    || tag(event, "username") != Some(input.username)
                    || tag(event, "full_name") != input.full_name
                    || tag(event, "issuer_label") != Some(input.label)
                    || verified_at(event).is_none_or(|at| {
                        input.admitted_at.timestamp() - at >= input.refresh_seconds
                    })
            });
        if !changed {
            sqlx::query(
                r#"
                UPDATE nip_fi_profile_heads
                SET issued_at = $3::numeric
                WHERE community_id = $1 AND pubkey = $2 AND issued_at < $3::numeric
                "#,
            )
            .bind(input.community.as_uuid())
            .bind(input.pubkey)
            .bind(&issued_at)
            .execute(tx.conn())
            .await?;
            tx.commit().await?;
            return Ok(None);
        }

        let event = profile_event(keys, &input, old.as_ref())?;
        let stored = self
            .replace_parameterized_event_in_transaction(
                &mut tx,
                &event,
                input.pubkey,
                None,
                crate::replaceable::ParameterizedReplacePrecondition::Unconditional,
            )
            .await?;
        if !matches!(
            stored.status,
            crate::replaceable::ParameterizedReplaceStatus::Inserted
                | crate::replaceable::ParameterizedReplaceStatus::Duplicate
        ) {
            return Err(DbError::InvalidData(
                "profile event did not replace current head".into(),
            ));
        }
        sqlx::query(
            r#"
            INSERT INTO nip_fi_profile_heads (community_id, pubkey, issuer, issued_at, event)
            VALUES ($1, $2, $3, $4::numeric, $5)
            ON CONFLICT (community_id, pubkey) DO UPDATE SET
                issuer = EXCLUDED.issuer,
                issued_at = EXCLUDED.issued_at,
                event = EXCLUDED.event
            "#,
        )
        .bind(input.community.as_uuid())
        .bind(input.pubkey)
        .bind(input.issuer)
        .bind(&issued_at)
        .bind(serde_json::to_value(&event)?)
        .execute(tx.conn())
        .await?;
        tx.commit().await?;
        Ok(Some(event))
    }
}

fn tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| {
        let t = t.as_slice();
        (t.first().map(String::as_str) == Some(name))
            .then(|| t.get(1).map(String::as_str))
            .flatten()
    })
}

fn verified_at(event: &Event) -> Option<i64> {
    tag(event, "verified_at").and_then(|value| value.parse().ok())
}

fn profile_event(
    keys: &Keys,
    input: &ProfileAdmission<'_>,
    previous: Option<&Event>,
) -> Result<Event> {
    let verified_at = input.admitted_at.timestamp().to_string();
    let mut values = vec![
        ("d", input.pubkey),
        ("p", input.pubkey),
        ("active", "true"),
        ("display_name", input.display_name),
        ("username", input.username),
        ("issuer_label", input.label),
    ];
    values.push(("verified_at", &verified_at));
    if let Some(full_name) = input.full_name {
        values.push(("full_name", full_name));
    }
    let tags = values
        .into_iter()
        .map(|(k, v)| Tag::parse([k, v]).map_err(|_| DbError::InvalidData("profile tag".into())))
        .collect::<Result<Vec<_>>>()?;
    let created_at = (Utc::now().timestamp().max(0) as u64)
        .max(previous.map_or(0, |e| e.created_at.as_secs().saturating_add(1)));
    EventBuilder::new(Kind::Custom(30382), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .map_err(|_| DbError::InvalidData("profile signing".into()))
}

// NumericDate supports nanoseconds; PostgreSQL timestamps only retain microseconds.
fn nanos(value: DateTime<Utc>) -> i128 {
    i128::from(value.timestamp()) * 1_000_000_000 + i128::from(value.timestamp_subsec_nanos())
}
fn parse_nanos(value: &str) -> Result<i128> {
    value
        .parse()
        .map_err(|_| DbError::InvalidData("profile assertion timestamp".into()))
}

#[cfg(test)]
mod postgres_tests;
