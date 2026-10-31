//! Durable output-only NIP-FI projections. Authorization never reads these tables.

use crate::{AdmittedTx, Db, DbError, Result};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use sha2::{Digest, Sha256};
use sqlx::Row;

/// Operator-selected publication policy. None retracts and disables publication.
#[derive(Clone)]
pub struct ProfilePolicy {
    /// Exact private issuer identity.
    pub issuer: String,
    /// Sorted environment-authorized hosts; None selects database bindings.
    pub community_hosts: Option<Vec<String>>,
    /// Validated public label, absent when disabled.
    pub label: Option<String>,
    /// Unchanged-name refresh interval.
    pub refresh_seconds: i64,
}

impl std::fmt::Debug for ProfilePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProfilePolicy([REDACTED])")
    }
}

/// Epochs captured before admission completes, never when deferred work starts.
#[derive(Clone, Copy)]
pub struct ProfileEpoch {
    access: AccessEpoch,
    policy: i64,
}

/// Verified input from a successful human/agent WebSocket admission.
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
    /// Validated public name/username, or None for retraction-only inputs.
    pub profile: Option<(&'a str, &'a str, Option<&'a str>)>,
    /// Epoch captured during this admission.
    pub epoch: ProfileEpoch,
    /// Database community bindings, as opposed to operator environment bindings.
    pub database_binding: bool,
    /// Whether the relay requires an explicit `relay_members` row to connect.
    /// Open relays treat enforced NIP-FI admission as eligibility instead.
    pub membership_required: bool,
}

/// Access-owned incarnations survive removal/restoration without projection locks.
#[derive(Clone, Copy, PartialEq, Eq)]
struct AccessEpoch {
    member: bool,
    membership: Option<uuid::Uuid>,
    ban: Option<uuid::Uuid>,
    binding: bool,
    issuer: Option<uuid::Uuid>,
}

async fn access_epoch(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    pubkey: &str,
    issuer: &str,
) -> Result<(AccessEpoch, bool)> {
    let row = sqlx::query("SELECT m.pubkey IS NOT NULL AS member, m.access_generation AS membership, b.access_generation AS ban, i.issuer IS NOT NULL AS binding, i.access_generation AS issuer, COALESCE(b.banned AND (b.ban_expires_at IS NULL OR b.ban_expires_at > now()), FALSE) AS banned FROM (SELECT 1) seed LEFT JOIN relay_members m ON m.community_id = $1 AND m.pubkey = $2 LEFT JOIN community_bans b ON b.community_id = $1 AND b.pubkey = decode($2, 'hex') LEFT JOIN community_nip_fi_issuers i ON i.community_id = $1 AND i.issuer = $3")
        .bind(community.as_uuid()).bind(pubkey).bind(issuer).fetch_one(conn).await?;
    Ok((
        AccessEpoch {
            member: row.try_get("member")?,
            membership: row.try_get("membership")?,
            ban: row.try_get("ban")?,
            binding: row.try_get("binding")?,
            issuer: row.try_get("issuer")?,
        },
        row.try_get("banned")?,
    ))
}

async fn lock_profile(tx: &mut AdmittedTx, pubkey: &str) -> Result<()> {
    let key = format!("profile:{}:{pubkey}", tx.community().as_uuid());
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 30382))")
        .bind(key)
        .execute(tx.conn())
        .await?;
    Ok(())
}

fn same_access(
    mut left: AccessEpoch,
    mut right: AccessEpoch,
    database_binding: bool,
    membership_required: bool,
) -> bool {
    if !database_binding {
        left.binding = false;
        left.issuer = None;
        right.binding = false;
        right.issuer = None;
    }
    if !membership_required {
        left.member = false;
        left.membership = None;
        right.member = false;
        right.membership = None;
    }
    left == right
}

/// Access owners call this only after commit. Capturing the trigger requires
/// no storage, signing or projection lock on the access-control path.
pub(crate) fn schedule_access_cleanup(
    pool: &sqlx::PgPool,
    community: CommunityId,
    pubkey: Option<&str>,
    issuer: Option<&str>,
) {
    schedule_cleanup(
        pool,
        Some(community),
        pubkey,
        issuer,
        None,
        Utc::now().timestamp(),
    );
}

fn schedule_cleanup(
    pool: &sqlx::PgPool,
    community: Option<CommunityId>,
    pubkey: Option<&str>,
    issuer: Option<&str>,
    hosts: Option<&[String]>,
    triggered_at: i64,
) {
    let pool = pool.clone();
    let pubkey = pubkey.map(str::to_owned);
    let issuer = issuer.map(str::to_owned);
    let hosts = hosts.map(<[String]>::to_vec);
    tokio::spawn(async move {
        for delay in [0, 1, 4] {
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
            let result = async {
                let mut after: Option<(uuid::Uuid, String)> = None;
                loop {
                    let heads: Vec<(uuid::Uuid, String, String)> = sqlx::query_as("SELECT h.community_id, h.pubkey, h.issuer FROM nip_fi_profile_heads h JOIN communities c ON c.id = h.community_id WHERE h.active AND ($1::uuid IS NULL OR h.community_id = $1) AND ($2::text IS NULL OR h.pubkey = $2) AND ($3::text IS NULL OR h.issuer = $3) AND ($4::uuid IS NULL OR (h.community_id, h.pubkey) > ($4, $5)) AND ($6::text[] IS NULL OR lower(c.host) = ANY($6)) ORDER BY h.community_id, h.pubkey LIMIT 100")
                        .bind(community.as_ref().map(|id| id.as_uuid())).bind(&pubkey).bind(&issuer).bind(after.as_ref().map(|(id, _)| id)).bind(after.as_ref().map(|(_, key)| key)).bind(&hosts).fetch_all(&pool).await?;
                    let Some((id, key, _)) = heads.last() else { break; };
                    after = Some((*id, key.clone()));
                    for (id, key, producer) in heads {
                        mark_cleanup(&pool, CommunityId::from_uuid(id), &producer, &key, triggered_at, None).await?;
                    }
                }
                Ok::<(), DbError>(())
            }.await;
            metrics::counter!("buzz_verified_profile_total", "outcome" => if result.is_ok() { "cleanup_scheduled" } else { "cleanup_failed" }).increment(1);
            if result.is_ok() {
                break;
            }
        }
    });
}

async fn mark_cleanup(
    pool: &sqlx::PgPool,
    community: CommunityId,
    issuer: &str,
    pubkey: &str,
    triggered_at: i64,
    event_id: Option<&str>,
) -> Result<()> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(tx.conn())
        .await?;
    lock_profile(&mut tx, pubkey).await?;
    sqlx::query("UPDATE nip_fi_profile_heads SET retract_before = GREATEST(retract_before, $4) WHERE community_id = $1 AND pubkey = $2 AND issuer = $3 AND active AND ($5::text IS NULL OR event->>'id' = $5) AND EXISTS (SELECT 1 FROM jsonb_array_elements(event->'tags') t WHERE CASE WHEN t->>0 = 'verified_at' THEN (t->>1)::bigint END <= $4)")
        .bind(community.as_uuid()).bind(pubkey).bind(issuer).bind(triggered_at).bind(event_id).execute(tx.conn()).await?;
    tx.commit().await?;
    Ok(())
}

impl Db {
    /// Reconcile an operator configuration once at startup. Tombstones retain
    /// policy epochs across removal/recreation. Old processes cannot commit work
    /// captured before this transition. All serving replicas use one config.
    pub async fn configure_profile_policies(
        &self,
        policies: &[ProfilePolicy],
        revision: i64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(30382)")
            .execute(&mut *tx)
            .await?;
        let mut canonical: Vec<_> = policies
            .iter()
            .map(|p| (&p.issuer, &p.label, p.refresh_seconds, &p.community_hosts))
            .collect();
        canonical.sort();
        let digest = hex::encode(Sha256::digest(serde_json::to_vec(&canonical)?));
        if let Some(row) =
            sqlx::query("SELECT revision, digest FROM nip_fi_profile_config WHERE singleton")
                .fetch_optional(&mut *tx)
                .await?
        {
            let previous: i64 = row.try_get("revision")?;
            if revision < previous
                || (revision == previous && row.get::<String, _>("digest") != digest)
            {
                return Err(DbError::InvalidData(
                    "profile configuration revision must advance".into(),
                ));
            }
            if revision == previous {
                return Ok(());
            }
        }
        sqlx::query("INSERT INTO nip_fi_profile_config (singleton, revision, digest) VALUES (TRUE, $1, $2) ON CONFLICT (singleton) DO UPDATE SET revision = EXCLUDED.revision, digest = EXCLUDED.digest")
            .bind(revision).bind(digest).execute(&mut *tx).await?;
        let issuers: Vec<&str> = policies.iter().map(|p| p.issuer.as_str()).collect();
        let mut disabled: Vec<String> = sqlx::query_scalar("UPDATE nip_fi_profile_policies SET label = NULL, generation = generation + 1 WHERE label IS NOT NULL AND NOT (issuer = ANY($1)) RETURNING issuer")
            .bind(&issuers).fetch_all(&mut *tx).await?;
        let mut removed_bindings = Vec::new();
        for policy in policies {
            let previous: Option<Option<Vec<String>>> = sqlx::query_scalar(
                "SELECT community_hosts FROM nip_fi_profile_policies WHERE issuer = $1",
            )
            .bind(&policy.issuer)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(Some(hosts)) = previous {
                let removed: Vec<_> = hosts
                    .into_iter()
                    .filter(|host| {
                        !policy
                            .community_hosts
                            .as_ref()
                            .is_some_and(|current| current.contains(host))
                    })
                    .collect();
                if !removed.is_empty() {
                    removed_bindings.push((policy.issuer.clone(), removed));
                }
            }
            if policy.label.is_none() {
                disabled.push(policy.issuer.clone());
            }
            sqlx::query("INSERT INTO nip_fi_profile_policies (issuer, label, refresh_seconds, community_hosts) VALUES ($1, $2, $3, $4) ON CONFLICT (issuer) DO UPDATE SET label = EXCLUDED.label, refresh_seconds = EXCLUDED.refresh_seconds, community_hosts = EXCLUDED.community_hosts, generation = nip_fi_profile_policies.generation + 1 WHERE (nip_fi_profile_policies.label, nip_fi_profile_policies.refresh_seconds, nip_fi_profile_policies.community_hosts) IS DISTINCT FROM (EXCLUDED.label, EXCLUDED.refresh_seconds, EXCLUDED.community_hosts)")
                .bind(&policy.issuer).bind(&policy.label).bind(policy.refresh_seconds).bind(&policy.community_hosts).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        let triggered_at = Utc::now().timestamp();
        for issuer in disabled {
            schedule_cleanup(&self.pool, None, None, Some(&issuer), None, triggered_at);
        }
        for (issuer, hosts) in removed_bindings {
            schedule_cleanup(
                &self.pool,
                None,
                None,
                Some(&issuer),
                Some(&hosts),
                triggered_at,
            );
        }
        Ok(())
    }

    /// Capture current durable epochs only when this process's configured
    /// publication policy still matches the deployment's current policy.
    pub async fn profile_epoch(
        &self,
        community: CommunityId,
        pubkey: &str,
        policy: &ProfilePolicy,
    ) -> Result<Option<ProfileEpoch>> {
        if policy.label.is_none() {
            return Ok(None);
        }
        let mut conn = self.pool.acquire().await?;
        let generation: Option<i64> = sqlx::query_scalar("SELECT generation FROM nip_fi_profile_policies WHERE issuer = $1 AND label = $2 AND refresh_seconds = $3 AND community_hosts IS NOT DISTINCT FROM $4")
            .bind(&policy.issuer).bind(&policy.label).bind(policy.refresh_seconds).bind(&policy.community_hosts).fetch_optional(&mut *conn).await?;
        let Some(policy_generation) = generation else {
            return Ok(None);
        };
        let (access, _) = access_epoch(&mut conn, community, pubkey, &policy.issuer).await?;
        Ok(Some(ProfileEpoch {
            access,
            policy: policy_generation,
        }))
    }

    /// Publish/refresh/retract under the current policy, access (membership
    /// only where the relay requires it), and recorded assertion ordering in
    /// one admitted transaction.
    pub async fn apply_profile_admission(
        &self,
        input: ProfileAdmission<'_>,
        keys: &Keys,
    ) -> Result<Option<String>> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            input.community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(tx.conn())
            .await?;
        let policy = sqlx::query("SELECT label, refresh_seconds, generation FROM nip_fi_profile_policies WHERE issuer = $1 FOR SHARE")
            .bind(input.issuer).fetch_optional(tx.conn()).await?;
        let Some(policy) = policy else {
            return Ok(None);
        };
        let label: Option<String> = policy.try_get("label")?;
        let Some(label) = label else {
            return Ok(None);
        };
        if policy.try_get::<i64, _>("generation")? != input.epoch.policy {
            return Ok(None);
        }
        lock_profile(&mut tx, input.pubkey).await?;
        let (access, banned) =
            access_epoch(tx.conn(), input.community, input.pubkey, input.issuer).await?;
        if !same_access(
            access,
            input.epoch.access,
            input.database_binding,
            input.membership_required,
        ) || (input.database_binding && !access.binding)
            || (input.profile.is_some()
                && ((input.membership_required && !access.member) || banned))
        {
            return Ok(None);
        }
        let head = sqlx::query("SELECT issuer, policy_generation, issued_at::text AS issued_at, event, active, retract_before FROM nip_fi_profile_heads WHERE community_id = $1 AND pubkey = $2 FOR UPDATE")
            .bind(input.community.as_uuid()).bind(input.pubkey).fetch_optional(tx.conn()).await?;
        if head.as_ref().is_some_and(|row| {
            let event: serde_json::Value = row.get("event");
            event
                .get("tags")
                .and_then(|tags| tags.as_array())
                .into_iter()
                .flatten()
                .find(|tag| tag[0] == "verified_at")
                .and_then(|tag| tag[1].as_str())
                .and_then(|value| value.parse::<i64>().ok())
                .is_some_and(|at| input.admitted_at.timestamp() < at)
        }) {
            return Ok(None);
        }
        let active = head.as_ref().is_some_and(|r| r.get::<bool, _>("active"));
        if active
            && head
                .as_ref()
                .is_some_and(|r| r.get::<String, _>("issuer") != input.issuer)
        {
            return Ok(None);
        }
        let previous_iat: Option<String> = sqlx::query_scalar("SELECT issued_at::text FROM nip_fi_profile_assertions WHERE community_id = $1 AND issuer = $2 AND pubkey = $3")
            .bind(input.community.as_uuid()).bind(input.issuer).bind(input.pubkey).fetch_optional(tx.conn()).await?;
        if previous_iat
            .as_deref()
            .map(parse_nanos)
            .transpose()?
            .is_some_and(|old| nanos(input.issued_at) < old)
        {
            return Ok(None);
        }
        sqlx::query("INSERT INTO nip_fi_profile_assertions (community_id, issuer, pubkey, issued_at) VALUES ($1, $2, $3, $4::numeric) ON CONFLICT (community_id, issuer, pubkey) DO UPDATE SET issued_at = GREATEST(nip_fi_profile_assertions.issued_at, EXCLUDED.issued_at)")
            .bind(input.community.as_uuid()).bind(input.issuer).bind(input.pubkey).bind(nanos(input.issued_at).to_string()).execute(tx.conn()).await?;
        if input.profile.is_none() {
            if active {
                sqlx::query("UPDATE nip_fi_profile_heads SET retract_before = GREATEST(retract_before, $4), issued_at = $3::numeric WHERE community_id = $1 AND pubkey = $2")
                    .bind(input.community.as_uuid()).bind(input.pubkey).bind(nanos(input.issued_at).to_string()).bind(input.admitted_at.timestamp()).execute(tx.conn()).await?;
            }
            tx.commit().await?;
            return Ok(None);
        }
        let old = head
            .as_ref()
            .map(|r| {
                Event::from_json(r.get::<serde_json::Value, _>("event").to_string())
                    .map_err(|_| DbError::InvalidData("profile event".into()))
            })
            .transpose()?;
        let should_write = match input.profile {
            None => active,
            Some((name, username, full_name)) => {
                !active
                    || old.as_ref().is_none_or(|event| {
                        tag(event, "display_name") != Some(name)
                            || tag(event, "username") != Some(username)
                            || tag(event, "full_name") != full_name
                            || tag(event, "issuer_label") != Some(label.as_str())
                            || tag(event, "verified_at")
                                .and_then(|v| v.parse::<i64>().ok())
                                .is_none_or(|at| {
                                    input.admitted_at.timestamp() - at
                                        >= policy.get::<i64, _>("refresh_seconds")
                                })
                    })
            }
        };
        let mut affected_event = old.as_ref().map(|event| event.id.to_hex());
        if should_write {
            let event = profile_event(
                keys,
                input.pubkey,
                input
                    .profile
                    .map(|(name, username, full_name)| (name, username, label.as_str(), full_name)),
                input.admitted_at,
                old.as_ref(),
            )?;
            affected_event = Some(event.id.to_hex());
            self.store_profile_head(
                &mut tx,
                input.pubkey,
                input.issuer,
                input.epoch.policy,
                &nanos(input.issued_at).to_string(),
                &event,
            )
            .await?;
        }
        sqlx::query("UPDATE nip_fi_profile_heads SET issued_at = $3::numeric, database_binding = $4, policy_generation = $5, retract_before = CASE WHEN EXISTS (SELECT 1 FROM jsonb_array_elements(event->'tags') t WHERE CASE WHEN t->>0 = 'verified_at' THEN (t->>1)::bigint END > retract_before) THEN NULL ELSE retract_before END WHERE community_id = $1 AND pubkey = $2")
            .bind(input.community.as_uuid()).bind(input.pubkey).bind(nanos(input.issued_at).to_string()).bind(input.database_binding).bind(input.epoch.policy).execute(tx.conn()).await?;
        // Access mutations do not take the projection lock. Check again after
        // signing/writing, then let version-scoped housekeeping catch later races.
        if !same_access(
            access_epoch(tx.conn(), input.community, input.pubkey, input.issuer)
                .await?
                .0,
            access,
            input.database_binding,
            input.membership_required,
        ) {
            return Ok(None);
        }
        tx.commit().await?;
        Ok(affected_event)
    }

    async fn store_profile_head(
        &self,
        tx: &mut AdmittedTx,
        pubkey: &str,
        issuer: &str,
        generation: i64,
        issued_at: &str,
        event: &Event,
    ) -> Result<()> {
        let stored = self
            .replace_parameterized_event_in_transaction(
                tx,
                event,
                pubkey,
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
        let community = tx.community();
        sqlx::query("INSERT INTO nip_fi_profile_heads (community_id, pubkey, issuer, policy_generation, issued_at, event, active) VALUES ($1, $2, $3, $4, $5::numeric, $6, $7) ON CONFLICT (community_id, pubkey) DO UPDATE SET issuer = EXCLUDED.issuer, policy_generation = EXCLUDED.policy_generation, issued_at = EXCLUDED.issued_at, event = EXCLUDED.event, active = EXCLUDED.active, retract_before = CASE WHEN nip_fi_profile_heads.issuer = EXCLUDED.issuer AND EXCLUDED.active THEN nip_fi_profile_heads.retract_before ELSE NULL END, broadcast_pending = TRUE")
            .bind(community.as_uuid()).bind(pubkey).bind(issuer).bind(generation).bind(issued_at).bind(serde_json::to_value(event)?).bind(tag(event, "active") == Some("true")).execute(tx.conn()).await?;
        Ok(())
    }

    /// Bounded inventory for delivery, time-scoped cleanup, and optional
    /// current-eligibility housekeeping. None of this gates access.
    pub async fn pending_profile_work(
        &self,
        after: Option<(CommunityId, String)>,
    ) -> Result<Vec<(CommunityId, String)>> {
        let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as("SELECT h.community_id, h.pubkey FROM nip_fi_profile_heads h JOIN communities c ON c.id = h.community_id WHERE ($1::uuid IS NULL OR (h.community_id, h.pubkey) > ($1, $2)) AND c.deletion_state = 'active' AND c.archived_at IS NULL AND (h.broadcast_pending OR h.active) ORDER BY h.community_id, h.pubkey LIMIT 100")
            .bind(after.as_ref().map(|(id, _)| id.as_uuid())).bind(after.as_ref().map(|(_, key)| key)).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|(id, key)| (CommunityId::from_uuid(id), key))
            .collect())
    }

    /// Materialize a pending retraction, retaining ordering records, then return
    /// the durable event for retryable local/cross-process broadcast. Only
    /// relays that require membership retract a key that has no member row.
    pub async fn process_profile_work(
        &self,
        community: CommunityId,
        pubkey: &str,
        keys: &Keys,
        membership_required: bool,
    ) -> Result<Option<Event>> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(tx.conn())
            .await?;
        lock_profile(&mut tx, pubkey).await?;
        let row = sqlx::query("SELECT h.issuer, h.policy_generation, h.issued_at::text AS issued_at, h.event, h.broadcast_pending, h.active, h.database_binding, h.retract_before, h.active AND (p.label IS NULL OR (p.community_hosts IS NOT NULL AND NOT EXISTS (SELECT 1 FROM communities c WHERE c.id = h.community_id AND lower(c.host) = ANY(p.community_hosts)))) AS retract FROM nip_fi_profile_heads h LEFT JOIN nip_fi_profile_policies p ON p.issuer = h.issuer WHERE h.community_id = $1 AND h.pubkey = $2 FOR UPDATE OF h")
            .bind(community.as_uuid()).bind(pubkey).fetch_optional(tx.conn()).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut event = Event::from_json(row.get::<serde_json::Value, _>("event").to_string())
            .map_err(|_| DbError::InvalidData("profile event".into()))?;
        let (current, banned) =
            access_epoch(tx.conn(), community, pubkey, row.get("issuer")).await?;
        let before: Option<i64> = row.try_get("retract_before")?;
        let pending = before.is_some_and(|triggered_at| {
            tag(&event, "verified_at")
                .and_then(|at| at.parse::<i64>().ok())
                .is_some_and(|at| at <= triggered_at)
        });
        let retract = row.get::<bool, _>("active")
            && (pending
                || row.try_get::<bool, _>("retract")?
                || ((membership_required && !current.member)
                    || banned
                    || (row.get::<bool, _>("database_binding") && !current.binding)));
        if retract {
            event = profile_event(keys, pubkey, None, Utc::now(), Some(&event))?;
            self.store_profile_head(
                &mut tx,
                pubkey,
                row.get("issuer"),
                row.get("policy_generation"),
                row.get("issued_at"),
                &event,
            )
            .await?;
        }
        if !retract && before.is_some() && !pending {
            sqlx::query("UPDATE nip_fi_profile_heads SET retract_before = NULL WHERE community_id = $1 AND pubkey = $2")
                .bind(community.as_uuid()).bind(pubkey).execute(tx.conn()).await?;
        }
        let broadcast: bool = row.try_get("broadcast_pending")?;
        tx.commit().await?;
        Ok((retract || broadcast).then_some(event))
    }

    /// Capture a disconnect trigger and schedule every matching community's
    /// projection separately from the command response, including past-until.
    pub fn schedule_profile_cleanup(&self, issuer: &str, pubkey: &str, triggered_at: i64) {
        schedule_cleanup(
            &self.pool,
            None,
            Some(pubkey),
            Some(issuer),
            None,
            triggered_at,
        );
    }

    /// Record a time-scoped post-publication cleanup; later verified_at survives.
    /// Without a command timestamp, callers must constrain cancellation to the
    /// event their admission observed, using that admission's time boundary.
    pub async fn retract_profile_admission(
        &self,
        community: CommunityId,
        issuer: &str,
        pubkey: &str,
        triggered_at: i64,
        event_id: Option<&str>,
    ) -> Result<()> {
        mark_cleanup(
            &self.pool,
            community,
            issuer,
            pubkey,
            triggered_at,
            event_id,
        )
        .await
    }

    /// Clear only the exact delivered version; concurrent replacement stays queued.
    pub async fn acknowledge_profile_broadcast(
        &self,
        community: CommunityId,
        pubkey: &str,
        event_id: &str,
    ) -> Result<()> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        sqlx::query("UPDATE nip_fi_profile_heads SET broadcast_pending = FALSE WHERE community_id = $1 AND pubkey = $2 AND event->>'id' = $3")
            .bind(community.as_uuid()).bind(pubkey).bind(event_id).execute(tx.conn()).await?;
        tx.commit().await?;
        Ok(())
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

fn profile_event(
    keys: &Keys,
    subject: &str,
    profile: Option<(&str, &str, &str, Option<&str>)>,
    now: DateTime<Utc>,
    previous: Option<&Event>,
) -> Result<Event> {
    if profile.is_none() && previous.is_some_and(|event| event.pubkey != keys.public_key()) {
        return Err(DbError::InvalidData("profile signing key changed".into()));
    }
    let mut values = vec![
        ("d", subject.to_owned()),
        ("p", subject.to_owned()),
        ("active", profile.is_some().to_string()),
    ];
    if let Some((name, username, issuer, full_name)) = profile {
        values.extend([
            ("display_name", name.to_owned()),
            ("username", username.to_owned()),
            ("issuer_label", issuer.to_owned()),
            ("verified_at", now.timestamp().to_string()),
        ]);
        if let Some(full_name) = full_name {
            values.push(("full_name", full_name.to_owned()));
        }
    }
    let tags = values
        .into_iter()
        .map(|(k, v)| {
            Tag::parse([k.to_owned(), v]).map_err(|_| DbError::InvalidData("profile tag".into()))
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retraction_requires_the_original_relay_signing_key() {
        let keys = Keys::generate();
        let subject = Keys::generate().public_key().to_hex();
        let event = profile_event(
            &keys,
            &subject,
            Some(("alice", "alice", "Example", Some("Alice Example"))),
            Utc::now(),
            None,
        )
        .unwrap();
        assert_eq!(tag(&event, "display_name"), Some("alice"));
        assert_eq!(tag(&event, "full_name"), Some("Alice Example"));
        assert!(event.verify().is_ok());
        let retracted = profile_event(&keys, &subject, None, Utc::now(), Some(&event)).unwrap();
        assert_eq!(tag(&retracted, "full_name"), None);
        assert_eq!(retracted.tags.len(), 3);
        assert!(
            profile_event(&Keys::generate(), &subject, None, Utc::now(), Some(&event),).is_err()
        );
    }
}
