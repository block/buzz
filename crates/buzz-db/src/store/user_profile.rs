//! Kind-0 profile projection into the `users` table.
//!
//! A kind-0 head is the authoritative profile; `users` carries a per-pubkey
//! projection of it (`name`, `display_name`, `avatar_url`, `about`,
//! `nip05_handle`, `metadata_event_id`) so people search can run over a small
//! table instead of the events partitions. The projection is written inside
//! the kind-0 replace transaction ([`Db::replace_profile_event`]) so an
//! accepted profile and its `users` row commit or roll back together, and
//! un-projected inside the soft-delete transaction when the head itself is
//! deleted ([`clear_profile_projection_for_deleted_head`]); the batched
//! backfill here repopulates rows that predate that seam.

use buzz_core::{CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use sqlx::{Acquire, PgConnection, Row, Transaction};

use crate::error::{DbError, Result};
use crate::store::replaceable::event_replacement_lock_key;
use crate::Db;

/// The `users` columns one kind-0 head projects to.
///
/// Built once per write from the event content and the bound tenant host
/// (NIP-05 handles are only kept when their domain is that host), then applied
/// as absolute state: a field absent from the event clears its column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileProjection {
    /// Raw kind-0 `name`.
    pub name: Option<String>,
    /// Kind-0 `display_name`, falling back to `displayName`.
    pub display_name: Option<String>,
    /// Kind-0 `picture`, falling back to `image`.
    pub avatar_url: Option<String>,
    /// Kind-0 `about`.
    pub about: Option<String>,
    /// Canonical `nip05` when its domain matches the tenant host; `None` otherwise.
    pub nip05_handle: Option<String>,
    /// Id of the kind-0 event this projection was derived from.
    pub metadata_event_id: Vec<u8>,
}

impl ProfileProjection {
    /// Project a kind-0 event for the community served at `host`.
    pub fn from_kind0_event(event: &nostr::Event, host: &str) -> Self {
        Self::from_kind0_content(&event.content, event.id.as_bytes(), host)
    }

    /// Project raw kind-0 content. Content that is not a JSON object clears
    /// every field; ingest rejects such events before storage, so this arm is
    /// only reachable for historical rows the backfill revisits.
    pub fn from_kind0_content(content: &str, event_id: &[u8], host: &str) -> Self {
        let content: serde_json::Value =
            serde_json::from_str(content).unwrap_or(serde_json::Value::Null);
        // First non-empty string among the candidate keys. Empty strings count
        // as absent so `nip05_handle`'s UNIQUE index never collides on "".
        let field = |keys: &[&str]| -> Option<String> {
            keys.iter()
                .filter_map(|key| content.get(*key).and_then(|v| v.as_str()))
                .find(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let nip05_handle = field(&["nip05"])
            .and_then(|raw| buzz_core::nip05::canonicalize_nip05(&raw, host).ok())
            .map(fit_varchar);
        Self {
            name: field(&["name"]).map(fit_varchar),
            display_name: field(&["display_name", "displayName"]).map(fit_varchar),
            avatar_url: field(&["picture", "image"]),
            about: field(&["about"]),
            nip05_handle,
            metadata_event_id: event_id.to_vec(),
        }
    }
}

/// Width of the `VARCHAR(255)` label columns on `users`.
const VARCHAR_LABEL_CHARS: usize = 255;

/// Clip a label to the column width. The projection is a search index over
/// the kind-0 head, not the head itself, so an oversized `name` must shorten
/// rather than fail the write and reject the profile event with it.
/// PostgreSQL measures `VARCHAR(n)` in characters, so clip on chars.
fn fit_varchar(value: String) -> String {
    match value.char_indices().nth(VARCHAR_LABEL_CHARS) {
        Some((byte_index, _)) => value[..byte_index].to_owned(),
        None => value,
    }
}

/// Outcome of a kind-0 replace that also projected into `users`.
#[derive(Debug, Clone)]
pub struct ProfileReplaceOutcome {
    /// The stored event (the incoming event echoed back on stale/duplicate writes).
    pub event: StoredEvent,
    /// `true` when the event became the live head; `false` for stale and duplicate writes.
    pub was_inserted: bool,
    /// `true` when the projection created the `users` row rather than updating it.
    pub user_created: bool,
}

/// One community's backfill progress as seen by the precheck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileBackfillPrecheck {
    /// The community the counts belong to.
    pub community_id: CommunityId,
    /// Its bound host.
    pub host: String,
    /// Distinct pubkeys with a live, channel-less kind-0 head.
    pub kind0_publishers: i64,
    /// Rows in `users` for the community.
    pub users_rows: i64,
    /// Publishers whose `users.metadata_event_id` is not their current head
    /// (missing row included). Zero means the projection is complete.
    pub unprojected_publishers: i64,
}

/// Result of one [`Db::backfill_profiles_batch`] call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileBackfillBatch {
    /// Publishers visited in this batch; zero means the scan is complete.
    pub scanned: usize,
    /// Visited publishers that still had a live head and were projected.
    pub projected: usize,
    /// `users` rows created (rather than updated) by this batch.
    pub created: usize,
    /// Cursor for the next batch: the last pubkey visited.
    pub last_pubkey: Option<Vec<u8>>,
}

fn is_unique_violation(error: &DbError) -> bool {
    matches!(error, DbError::Sqlx(sqlx::Error::Database(db)) if db.is_unique_violation())
}

/// Upsert the projection for `pubkey` inside the caller's transaction.
///
/// Returns `true` when the `users` row was created. A NIP-05 handle another
/// row already holds (partial unique index `idx_users_nip05`) is written
/// inside a savepoint so the violation cannot abort the surrounding event
/// transaction. The conflict is then judged against the holder's current
/// kind-0 head, not its row: a holder whose head still carries the handle
/// keeps it and this row projects with `nip05_handle = NULL` (NIP-05
/// resolution is identity-bearing, so a live claim is never displaced, and
/// the row never keeps a handle its head no longer claims); a holder whose
/// head no longer carries it — a row that predates the projection seam, or
/// was left behind when its owner moved on — is stale and releases it, so the
/// claim lands in this pass instead of waiting for the holder to be
/// re-projected and the claimant to be written again.
pub(crate) async fn upsert_profile_in_transaction(
    conn: &mut PgConnection,
    community_id: CommunityId,
    pubkey: &[u8],
    profile: &ProfileProjection,
) -> Result<bool> {
    if let Some(created) =
        try_upsert_profile_with_handle(conn, community_id, pubkey, profile).await?
    {
        return Ok(created);
    }
    if let Some(handle) = profile.nip05_handle.as_deref() {
        if release_stale_nip05_holder(conn, community_id, pubkey, handle).await? {
            if let Some(created) =
                try_upsert_profile_with_handle(conn, community_id, pubkey, profile).await?
            {
                return Ok(created);
            }
        }
    }
    tracing::warn!(
        pubkey = %hex::encode(pubkey),
        "kind:0 NIP-05 handle contested, projecting profile without it"
    );
    upsert_profile_on(conn, community_id, pubkey, profile, false).await
}

/// One attempt to write the full projection under a savepoint. `Ok(None)`
/// means the NIP-05 unique index rejected it and the savepoint was rolled
/// back, leaving the surrounding transaction usable.
async fn try_upsert_profile_with_handle(
    conn: &mut PgConnection,
    community_id: CommunityId,
    pubkey: &[u8],
    profile: &ProfileProjection,
) -> Result<Option<bool>> {
    let mut savepoint = conn.begin().await?;
    match upsert_profile_on(&mut savepoint, community_id, pubkey, profile, true).await {
        Ok(created) => {
            savepoint.commit().await?;
            Ok(Some(created))
        }
        Err(error) if is_unique_violation(&error) => {
            savepoint.rollback().await?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Clear `handle` from the row that holds it when its owner's current kind-0
/// head no longer claims it. Returns `true` when the handle was released.
///
/// The holder's head is read under the holder's own replace lock so the
/// comparison cannot race a profile the holder is publishing right now. The
/// lock is taken with `pg_try_advisory_xact_lock`: the caller already holds
/// its own replace lock (and the backfill holds one per batch pubkey), so a
/// blocking acquire could cycle with a holder doing the same thing in the
/// other direction. A holder mid-replace is simply treated as contested.
async fn release_stale_nip05_holder(
    conn: &mut PgConnection,
    community_id: CommunityId,
    claimant: &[u8],
    handle: &str,
) -> Result<bool> {
    let holder: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT pubkey FROM users \
         WHERE community_id = $1 AND LOWER(nip05_handle) = LOWER($2) AND pubkey <> $3",
    )
    .bind(community_id.as_uuid())
    .bind(handle)
    .bind(claimant)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(holder) = holder else {
        return Ok(false);
    };
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(event_replacement_lock_key(community_id, 0, &holder, None))
        .fetch_one(&mut *conn)
        .await?;
    if !locked {
        return Ok(false);
    }
    // The projection only keeps handles on the tenant host, so the handle's
    // own domain is that host; project the holder's head against it.
    let host = handle.rsplit_once('@').map_or(handle, |(_, domain)| domain);
    let heads =
        current_profile_events_on(conn, community_id, std::slice::from_ref(&holder)).await?;
    let still_claimed = heads.first().is_some_and(|head| {
        ProfileProjection::from_kind0_event(&head.event, host)
            .nip05_handle
            .is_some_and(|current| current.eq_ignore_ascii_case(handle))
    });
    if still_claimed {
        return Ok(false);
    }
    let released = sqlx::query(
        "UPDATE users SET nip05_handle = NULL, updated_at = NOW() \
         WHERE community_id = $1 AND pubkey = $2 AND LOWER(nip05_handle) = LOWER($3)",
    )
    .bind(community_id.as_uuid())
    .bind(&holder)
    .bind(handle)
    .execute(conn)
    .await?
    .rows_affected();
    if released == 1 {
        tracing::info!(
            holder = %hex::encode(&holder),
            claimant = %hex::encode(claimant),
            "released a NIP-05 handle from a stale users row"
        );
    }
    Ok(released == 1)
}

async fn upsert_profile_on(
    conn: &mut PgConnection,
    community_id: CommunityId,
    pubkey: &[u8],
    profile: &ProfileProjection,
    write_nip05: bool,
) -> Result<bool> {
    // The row is the projection of the head and nothing else. A handle the
    // head claims but this write may not take (`write_nip05 == false`, the
    // contested case) is written as NULL, not left at whatever the row held
    // before: that previous handle came from a head that no longer claims it,
    // and keeping it would let `/.well-known/nostr.json` resolve it to a
    // pubkey whose profile says otherwise.
    let nip05_handle = profile.nip05_handle.as_deref().filter(|_| write_nip05);
    // `xmax = 0` on the returned row distinguishes INSERT from the UPDATE arm.
    let row = sqlx::query(
        "INSERT INTO users (community_id, pubkey, name, display_name, avatar_url, about, \
                            nip05_handle, metadata_event_id, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW()) \
         ON CONFLICT (community_id, pubkey) DO UPDATE SET \
             name = EXCLUDED.name, display_name = EXCLUDED.display_name, \
             avatar_url = EXCLUDED.avatar_url, about = EXCLUDED.about, \
             nip05_handle = EXCLUDED.nip05_handle, \
             metadata_event_id = EXCLUDED.metadata_event_id, updated_at = NOW() \
         RETURNING (xmax = 0) AS created",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .bind(profile.name.as_deref())
    .bind(profile.display_name.as_deref())
    .bind(profile.avatar_url.as_deref())
    .bind(profile.about.as_deref())
    .bind(nip05_handle)
    .bind(profile.metadata_event_id.as_slice())
    .fetch_one(conn)
    .await?;
    Ok(row.try_get::<bool, _>("created")?)
}

/// Un-project a pubkey's row when `deleted_head` — the kind-0 event its
/// `metadata_event_id` points at — has just been soft-deleted (a NIP-09 kind-5
/// on the author's own profile). Runs inside the caller's delete transaction,
/// so the event and its projection leave together.
///
/// Only the head-bound columns clear: `metadata_event_id`, so the row leaves
/// the `users`-backed search set and the precheck instead of taking a page
/// slot that hydrates to nothing (`search_users` requires a projected head,
/// and no live kind-0 exists to hydrate to), and `nip05_handle`, so the
/// handle is free for its next claimant. `name`, `display_name`,
/// `avatar_url`, and `about` are deliberately kept as the last-known profile:
/// readers that label a pubkey (`channel_members`, `dm`, media attribution)
/// keep showing a name rather than a bare hex key for messages that remain.
///
/// The `metadata_event_id = deleted_head` guard makes this a no-op when the
/// deleted row is not the projected head — a stale duplicate, or a head a
/// concurrent replace has already moved on from — so a later projection is
/// never undone by an older row's deletion. Returns `true` when the row was
/// cleared.
pub(crate) async fn clear_profile_projection_for_deleted_head(
    conn: &mut PgConnection,
    community_id: CommunityId,
    pubkey: &[u8],
    deleted_head: &[u8],
) -> Result<bool> {
    let cleared = sqlx::query(
        "UPDATE users SET metadata_event_id = NULL, nip05_handle = NULL, updated_at = NOW() \
         WHERE community_id = $1 AND pubkey = $2 AND metadata_event_id = $3",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .bind(deleted_head)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(cleared == 1)
}

/// Each pubkey's current channel-less kind-0 head, resolved like the replace
/// path does: newest `created_at`, lowest `id` on a tie, live rows only.
async fn current_profile_events_on(
    conn: &mut PgConnection,
    community_id: CommunityId,
    pubkeys: &[Vec<u8>],
) -> Result<Vec<StoredEvent>> {
    if pubkeys.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT DISTINCT ON (pubkey) \
                id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events \
         WHERE community_id = $1 AND kind = 0 AND channel_id IS NULL AND deleted_at IS NULL \
           AND pubkey = ANY($2) \
         ORDER BY pubkey, created_at DESC, id ASC",
    )
    .bind(community_id.as_uuid())
    .bind(pubkeys)
    .fetch_all(conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(event) = crate::event::row_to_stored_event(row)? {
            out.push(event);
        }
    }
    Ok(out)
}

impl Db {
    /// Current kind-0 head for each of `pubkeys` (at most one event per pubkey,
    /// in no particular order). Pubkeys without a live head are absent.
    #[datastore_span(name = "get_current_profile_events", system = "postgresql")]
    pub async fn get_current_profile_events(
        &self,
        community_id: CommunityId,
        pubkeys: &[Vec<u8>],
    ) -> Result<Vec<StoredEvent>> {
        if pubkeys.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        current_profile_events_on(&mut conn, community_id, pubkeys).await
    }

    /// Per-community projection progress for every live community: how many
    /// kind-0 publishers exist, how many `users` rows, and how many publishers
    /// still lack a row projected from their current head.
    #[datastore_span(name = "profile_backfill_precheck", system = "postgresql")]
    pub async fn profile_backfill_precheck(&self) -> Result<Vec<ProfileBackfillPrecheck>> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        let rows = sqlx::query(
            "SELECT c.id, c.host, \
                    (SELECT COUNT(DISTINCT e.pubkey) FROM events e \
                      WHERE e.community_id = c.id AND e.kind = 0 \
                        AND e.channel_id IS NULL AND e.deleted_at IS NULL) AS kind0_publishers, \
                    (SELECT COUNT(*) FROM users u WHERE u.community_id = c.id) AS users_rows, \
                    (SELECT COUNT(*) FROM ( \
                        SELECT DISTINCT ON (e.pubkey) e.pubkey, e.id FROM events e \
                         WHERE e.community_id = c.id AND e.kind = 0 \
                           AND e.channel_id IS NULL AND e.deleted_at IS NULL \
                         ORDER BY e.pubkey, e.created_at DESC, e.id ASC) h \
                      LEFT JOIN users u ON u.community_id = c.id AND u.pubkey = h.pubkey \
                     WHERE u.metadata_event_id IS DISTINCT FROM h.id) AS unprojected_publishers \
             FROM communities c \
             WHERE c.deleted_at IS NULL \
             ORDER BY c.host",
        )
        .fetch_all(&mut *conn)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ProfileBackfillPrecheck {
                    community_id: CommunityId::from_uuid(row.try_get("id")?),
                    host: row.try_get("host")?,
                    kind0_publishers: row.try_get("kind0_publishers")?,
                    users_rows: row.try_get("users_rows")?,
                    unprojected_publishers: row.try_get("unprojected_publishers")?,
                })
            })
            .collect()
    }

    /// Project one batch of kind-0 publishers (pubkey order, after
    /// `after_pubkey`) into `users` for the community served at `host`.
    ///
    /// Each batch is one transaction that takes the same per-pubkey advisory
    /// lock as [`Db::replace_addressable_event`] before reading the head, so a
    /// concurrent profile replace can never be overwritten with an older
    /// projection. One pass converges: a NIP-05 handle still held by a stale
    /// row is released when its claimant is projected, whichever batch each
    /// of them lands in (see [`upsert_profile_in_transaction`]), and
    /// re-running is a no-op over current heads.
    #[datastore_span(name = "backfill_profiles_batch", system = "postgresql")]
    pub async fn backfill_profiles_batch(
        &self,
        community_id: CommunityId,
        host: &str,
        after_pubkey: Option<&[u8]>,
        batch_size: u32,
    ) -> Result<ProfileBackfillBatch> {
        let batch_size = i64::from(batch_size.clamp(1, 1_000));
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        let mut tx = Transaction::begin(connection, None).await?;

        let pubkeys: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT DISTINCT pubkey FROM events \
             WHERE community_id = $1 AND kind = 0 AND channel_id IS NULL AND deleted_at IS NULL \
               AND pubkey > $2 \
             ORDER BY pubkey LIMIT $3",
        )
        .bind(community_id.as_uuid())
        .bind(after_pubkey.unwrap_or(&[]))
        .bind(batch_size)
        .fetch_all(&mut *tx)
        .await?;
        if pubkeys.is_empty() {
            tx.rollback().await?;
            return Ok(ProfileBackfillBatch::default());
        }

        // Pubkey order matches the page order, and the replace path holds one
        // lock per transaction, so the two can wait on each other but never
        // form a cycle.
        for pubkey in &pubkeys {
            let key = event_replacement_lock_key(community_id, 0, pubkey, None);
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(key)
                .execute(&mut *tx)
                .await?;
        }

        let heads = current_profile_events_on(&mut tx, community_id, &pubkeys).await?;
        let mut created = 0usize;
        for head in &heads {
            let projection = ProfileProjection::from_kind0_event(&head.event, host);
            let pubkey = head.event.pubkey.to_bytes();
            if upsert_profile_in_transaction(&mut tx, community_id, &pubkey, &projection).await? {
                created += 1;
            }
        }
        tx.commit().await?;

        Ok(ProfileBackfillBatch {
            scanned: pubkeys.len(),
            projected: heads.len(),
            created,
            last_pubkey: pubkeys.last().cloned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection(content: &str) -> ProfileProjection {
        ProfileProjection::from_kind0_content(content, &[7u8; 32], "buzz.example")
    }

    #[test]
    fn projection_keeps_name_and_display_name_separate() {
        let p = projection(r#"{"name":"wes","display_name":"Wes Anderson"}"#);
        assert_eq!(p.name.as_deref(), Some("wes"));
        assert_eq!(p.display_name.as_deref(), Some("Wes Anderson"));
        assert_eq!(p.metadata_event_id, vec![7u8; 32]);
    }

    #[test]
    fn projection_falls_back_to_camel_case_display_name_only() {
        let p = projection(r#"{"name":"wes","displayName":"Wesley"}"#);
        assert_eq!(p.display_name.as_deref(), Some("Wesley"));
        let name_only = projection(r#"{"name":"wes"}"#);
        assert_eq!(name_only.display_name, None, "`name` is not a display_name");
        assert_eq!(name_only.name.as_deref(), Some("wes"));
    }

    /// `users.name`/`display_name` are `VARCHAR(255)`; an oversized label is
    /// clipped on a char boundary instead of failing the kind-0 write.
    #[test]
    fn projection_clips_labels_to_the_column_width_on_char_boundaries() {
        let long_name = "é".repeat(300);
        let p = projection(&format!(
            r#"{{"name":"{long_name}","about":"{long_name}"}}"#
        ));
        let name = p.name.expect("name kept");
        assert_eq!(name.chars().count(), 255);
        assert!(name.chars().all(|c| c == 'é'), "clip must not split a char");
        assert_eq!(
            p.about.as_deref().map(|s| s.chars().count()),
            Some(300),
            "TEXT columns are not clipped"
        );
        let short = projection(r#"{"name":"wes"}"#);
        assert_eq!(short.name.as_deref(), Some("wes"));
    }

    #[test]
    fn projection_treats_empty_strings_as_absent() {
        let p = projection(r#"{"name":"","display_name":"","picture":"","about":"","nip05":""}"#);
        assert_eq!(
            p,
            ProfileProjection {
                metadata_event_id: vec![7u8; 32],
                ..empty()
            }
        );
    }

    #[test]
    fn projection_keeps_only_on_host_nip05_handles() {
        let on_host = projection(r#"{"nip05":"Wes@Buzz.example"}"#);
        assert_eq!(on_host.nip05_handle.as_deref(), Some("wes@buzz.example"));
        let off_host = projection(r#"{"nip05":"wes@elsewhere.example"}"#);
        assert_eq!(off_host.nip05_handle, None);
        let malformed = projection(r#"{"nip05":"not-a-handle"}"#);
        assert_eq!(malformed.nip05_handle, None);
    }

    #[test]
    fn projection_prefers_picture_over_image() {
        let p = projection(r#"{"picture":"https://a/p.png","image":"https://a/i.png"}"#);
        assert_eq!(p.avatar_url.as_deref(), Some("https://a/p.png"));
        let image = projection(r#"{"image":"https://a/i.png"}"#);
        assert_eq!(image.avatar_url.as_deref(), Some("https://a/i.png"));
    }

    #[test]
    fn projection_of_non_object_content_clears_every_field() {
        assert_eq!(
            projection("not json"),
            ProfileProjection {
                metadata_event_id: vec![7u8; 32],
                ..empty()
            }
        );
    }

    fn empty() -> ProfileProjection {
        ProfileProjection {
            name: None,
            display_name: None,
            avatar_url: None,
            about: None,
            nip05_handle: None,
            metadata_event_id: Vec::new(),
        }
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Timestamp};
    use sqlx::PgPool;

    async fn setup_db() -> Db {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        Db::from_pool(pool)
    }

    async fn make_community(pool: &PgPool) -> (CommunityId, String) {
        let id = uuid::Uuid::new_v4();
        let host = format!("profile-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert test community");
        (CommunityId::from_uuid(id), host)
    }

    fn kind0(keys: &Keys, content: &str, created_at: u64) -> nostr::Event {
        EventBuilder::new(Kind::Metadata, content)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("sign kind:0")
    }

    async fn users_row(
        pool: &PgPool,
        community: CommunityId,
        pubkey: &[u8],
    ) -> Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<Vec<u8>>,
    )> {
        sqlx::query_as(
            "SELECT name, display_name, avatar_url, about, nip05_handle, metadata_event_id \
             FROM users WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_optional(pool)
        .await
        .expect("read users row")
    }

    async fn replace_profile(
        db: &Db,
        community: CommunityId,
        host: &str,
        event: &nostr::Event,
    ) -> ProfileReplaceOutcome {
        db.replace_profile_event(
            community,
            event,
            &ProfileProjection::from_kind0_event(event, host),
        )
        .await
        .expect("replace profile event")
    }

    /// The projection is written by the replace transaction itself: name,
    /// display_name, and metadata_event_id land as separate columns, a newer
    /// head rewrites them as absolute state, and a stale head changes nothing.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replace_profile_event_projects_name_display_name_and_metadata_event_id() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_bytes();

        let first = kind0(
            &keys,
            &format!(
                r#"{{"name":"wes","display_name":"Wes Anderson","picture":"https://a/p.png","about":"hi","nip05":"Wes@{host}"}}"#
            ),
            1_700_000_010,
        );
        let outcome = replace_profile(&db, community, &host, &first).await;
        assert!(outcome.was_inserted, "first head must be accepted");
        assert!(outcome.user_created, "no users row existed yet");
        assert_eq!(
            users_row(&db.pool, community, &pubkey).await,
            Some((
                Some("wes".into()),
                Some("Wes Anderson".into()),
                Some("https://a/p.png".into()),
                Some("hi".into()),
                Some(format!("wes@{host}")),
                Some(first.id.as_bytes().to_vec()),
            ))
        );

        let newer = kind0(&keys, r#"{"displayName":"Wesley"}"#, 1_700_000_020);
        let outcome = replace_profile(&db, community, &host, &newer).await;
        assert!(outcome.was_inserted);
        assert!(!outcome.user_created, "row already existed");
        assert_eq!(
            users_row(&db.pool, community, &pubkey).await,
            Some((
                None,
                Some("Wesley".into()),
                None,
                None,
                None,
                Some(newer.id.as_bytes().to_vec()),
            )),
            "kind:0 is absolute state: absent fields clear, displayName fills display_name"
        );

        let stale = kind0(&keys, r#"{"name":"stale"}"#, 1_700_000_015);
        let outcome = replace_profile(&db, community, &host, &stale).await;
        assert!(!outcome.was_inserted, "older head must be rejected");
        assert!(!outcome.user_created);
        assert_eq!(
            users_row(&db.pool, community, &pubkey)
                .await
                .and_then(|row| row.5),
            Some(newer.id.as_bytes().to_vec()),
            "a stale write must not touch the projection"
        );
    }

    /// A handle another user's live head holds is not taken: the claimant
    /// projects with `nip05_handle = NULL` — not the handle its row held from
    /// a previous head, which this head no longer claims — and the event is
    /// still accepted.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replace_profile_event_projects_a_null_handle_when_nip05_is_contested() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let holder = Keys::generate();
        let claimant = Keys::generate();
        let claimant_pubkey = claimant.public_key().to_bytes();

        replace_profile(
            &db,
            community,
            &host,
            &kind0(
                &holder,
                &format!(r#"{{"nip05":"shared@{host}"}}"#),
                1_700_000_010,
            ),
        )
        .await;
        let own = kind0(
            &claimant,
            &format!(r#"{{"name":"claimant","nip05":"claimant@{host}"}}"#),
            1_700_000_011,
        );
        replace_profile(&db, community, &host, &own).await;

        let contested = kind0(
            &claimant,
            &format!(r#"{{"name":"renamed","nip05":"shared@{host}"}}"#),
            1_700_000_012,
        );
        let outcome = replace_profile(&db, community, &host, &contested).await;
        assert!(outcome.was_inserted, "the event itself is still accepted");

        let row = users_row(&db.pool, community, &claimant_pubkey)
            .await
            .expect("claimant row");
        assert_eq!(
            row.0.as_deref(),
            Some("renamed"),
            "other fields still project"
        );
        assert_eq!(
            row.4, None,
            "the contested handle is not taken, and the previous one is not kept: \
             the current head claims neither"
        );
        assert_eq!(row.5, Some(contested.id.as_bytes().to_vec()));
        assert_eq!(
            users_row(&db.pool, community, &holder.public_key().to_bytes())
                .await
                .and_then(|row| row.4),
            Some(format!("shared@{host}")),
            "the holder keeps the handle"
        );
    }

    /// A kind-5 on the author's own head reaches
    /// `soft_delete_event_and_update_thread`, which un-projects the row in the
    /// delete transaction: `metadata_event_id` and `nip05_handle` clear, so
    /// the pubkey leaves `search_users` and the precheck and the handle is
    /// free for the next claimant, while the labels stay as last-known.
    /// Deleting a live kind-0 that is *not* the projected head changes
    /// nothing, and the author's next profile re-projects the row.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn soft_deleting_the_head_clears_the_projection_and_releases_the_handle() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_bytes();

        let head = kind0(
            &keys,
            &format!(
                r#"{{"name":"wes","display_name":"Wes","picture":"https://a/p.png","about":"hi","nip05":"wes@{host}"}}"#
            ),
            1_700_000_010,
        );
        replace_profile(&db, community, &host, &head).await;

        // A second live kind-0 for the same pubkey only exists as a historical
        // anomaly (the replace path soft-deletes superseded rows), so store
        // one directly to exercise the `metadata_event_id = deleted` guard.
        let stray = kind0(&keys, r#"{"name":"stray"}"#, 1_700_000_005);
        db.insert_event(community, &stray, None)
            .await
            .expect("insert the stray live kind-0");
        assert!(db
            .soft_delete_event_and_update_thread(community, stray.id.as_bytes(), None, None)
            .await
            .expect("delete the stray row"));
        assert_eq!(
            users_row(&db.pool, community, &pubkey)
                .await
                .and_then(|row| row.5),
            Some(head.id.as_bytes().to_vec()),
            "deleting a row that is not the projected head leaves the projection alone"
        );

        assert!(db
            .soft_delete_event_and_update_thread(community, head.id.as_bytes(), None, None)
            .await
            .expect("delete the head"));
        assert_eq!(
            users_row(&db.pool, community, &pubkey).await,
            Some((
                Some("wes".into()),
                Some("Wes".into()),
                Some("https://a/p.png".into()),
                Some("hi".into()),
                None,
                None,
            )),
            "head-bound columns clear; labels and avatar stay as last-known"
        );
        let hits = db
            .search_users(
                community,
                &crate::SearchUsersParams {
                    query: "wes",
                    authors: None,
                    limit: 10,
                    offset: 0,
                    include_unprojected: false,
                },
            )
            .await
            .expect("search users");
        assert!(
            hits.is_empty(),
            "a row without a live head is not a search candidate: {hits:?}"
        );
        let after = precheck_for(&db, community).await;
        assert_eq!(
            (
                after.kind0_publishers,
                after.users_rows,
                after.unprojected_publishers
            ),
            (0, 1, 0),
            "no live head means nothing to project: {after:?}"
        );

        let claimant = Keys::generate();
        replace_profile(
            &db,
            community,
            &host,
            &kind0(
                &claimant,
                &format!(r#"{{"name":"claimant","nip05":"wes@{host}"}}"#),
                1_700_000_020,
            ),
        )
        .await;
        assert_eq!(
            users_row(&db.pool, community, &claimant.public_key().to_bytes())
                .await
                .and_then(|row| row.4),
            Some(format!("wes@{host}")),
            "the released handle is free for the next claimant"
        );
        let again = kind0(&keys, r#"{"name":"wes again"}"#, 1_700_000_030);
        replace_profile(&db, community, &host, &again).await;
        assert_eq!(
            users_row(&db.pool, community, &pubkey)
                .await
                .and_then(|row| row.5),
            Some(again.id.as_bytes().to_vec()),
            "the author's next profile re-projects the row"
        );
    }

    /// Granted `pg_advisory_xact_lock(int8)` rows backend `pid` holds on
    /// `key`. Postgres stores an int8 advisory key as `classid = key >> 32`,
    /// `objid = key & 0xffffffff`, `objsubid = 1`.
    async fn granted_replace_locks(pool: &PgPool, pid: i32, key: i64) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks \
             WHERE locktype = 'advisory' AND pid = $1 AND granted \
               AND classid = $2::oid AND objid = $3::oid AND objsubid = 1",
        )
        .bind(pid)
        .bind(((key as u64) >> 32) as i64)
        .bind(((key as u64) & 0xffff_ffff) as i64)
        .fetch_one(pool)
        .await
        .expect("inspect pg_locks")
    }

    /// Run the `_in_tx` delete body on a transaction the test keeps open, so
    /// the locks it holds can be read before they are released. Returns the
    /// open transaction and its backend pid.
    async fn delete_in_open_tx(
        db: &Db,
        community: CommunityId,
        event_id: &[u8],
    ) -> (crate::AdmittedTx, i32) {
        let mut tx = crate::begin_community_event_write_transaction(
            &db.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
        .expect("begin delete tx");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(tx.conn())
            .await
            .expect("backend pid");
        let deleted =
            crate::event::soft_delete_event_and_update_thread_in_tx(&mut tx, event_id, None, None)
                .await
                .expect("soft delete in tx");
        assert!(deleted, "the row must transition to deleted in this call");
        (tx, pid)
    }

    /// The delete hook's `metadata_event_id = <deleted id>` guard is enough
    /// against a live replace, which writes a new id, but not against a
    /// backfill batch that read this row as the head and will write the same
    /// id back. So the hook takes the pubkey's kind-0 replacement lock before
    /// the UPDATE — for every channel-less kind-0, head or stray, because
    /// whether the row is the head cannot be known until the lock is held —
    /// and a non-kind-0 deletion by the same author holds no exclusive
    /// advisory lock (the community write fence holds a *shared* one on every
    /// write). Read from `pg_locks` while each delete transaction is open,
    /// and again after commit to pin the lock as transaction-scoped.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deleting_a_channel_less_kind0_holds_the_replace_lock_until_commit() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_bytes();
        let key = event_replacement_lock_key(community, 0, &pubkey, None);

        let head = kind0(&keys, r#"{"name":"wes"}"#, 1_700_000_010);
        replace_profile(&db, community, &host, &head).await;
        let stray = kind0(&keys, r#"{"name":"stray"}"#, 1_700_000_005);
        db.insert_event(community, &stray, None)
            .await
            .expect("insert the stray live kind-0");
        let note = EventBuilder::new(Kind::TextNote, "not a profile")
            .custom_created_at(Timestamp::from(1_700_000_011))
            .sign_with_keys(&keys)
            .expect("sign note");
        db.insert_event(community, &note, None)
            .await
            .expect("insert the note");

        let (tx, pid) = delete_in_open_tx(&db, community, note.id.as_bytes()).await;
        let exclusive_advisory: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks \
             WHERE locktype = 'advisory' AND pid = $1 AND mode = 'ExclusiveLock'",
        )
        .bind(pid)
        .fetch_one(&db.pool)
        .await
        .expect("inspect pg_locks");
        assert_eq!(
            exclusive_advisory, 0,
            "a non-kind-0 deletion takes no exclusive advisory lock"
        );
        assert_eq!(
            granted_replace_locks(&db.pool, pid, key).await,
            0,
            "a non-kind-0 deletion does not take the author's kind-0 replace lock"
        );
        tx.commit().await.expect("commit note delete");

        let (tx, pid) = delete_in_open_tx(&db, community, stray.id.as_bytes()).await;
        assert_eq!(
            granted_replace_locks(&db.pool, pid, key).await,
            1,
            "a stray kind-0 takes the lock too: head-ness is only known once it is held"
        );
        tx.commit().await.expect("commit stray delete");
        assert_eq!(
            users_row(&db.pool, community, &pubkey)
                .await
                .and_then(|row| row.5),
            Some(head.id.as_bytes().to_vec()),
            "the stray's deletion leaves the projection alone"
        );

        let (tx, pid) = delete_in_open_tx(&db, community, head.id.as_bytes()).await;
        assert_eq!(
            granted_replace_locks(&db.pool, pid, key).await,
            1,
            "deleting the head holds the pubkey's kind-0 replace lock until commit"
        );
        tx.commit().await.expect("commit head delete");
        assert_eq!(
            granted_replace_locks(&db.pool, pid, key).await,
            0,
            "the lock is transaction-scoped and gone once the delete commits"
        );
        assert_eq!(
            users_row(&db.pool, community, &pubkey)
                .await
                .and_then(|row| row.5),
            None,
            "the head's deletion un-projects the row"
        );
    }

    /// The race the lock closes: a backfill batch reads H1 as A's head, a
    /// kind-5 on H1 soft-deletes it and clears A's row, and the batch then
    /// writes `metadata_event_id = H1` back — a projected row pointing at a
    /// deleted head, invisible to the precheck (A has no live head, so it is
    /// not a publisher). The batch holds A's replacement lock from before its
    /// head read until commit; the delete now takes the same lock, so it
    /// queues behind the batch and its compare-and-clear sees the batch's
    /// write. The batch is driven statement by statement on an open
    /// transaction because `backfill_profiles_batch` cannot be paused between
    /// its read and its write.
    ///
    /// Mutation oracle: without the lock in the delete hook the delete never
    /// queues (it finishes before a waiter appears) and A's row ends with
    /// `metadata_event_id = H1`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deleting_the_head_under_an_in_flight_backfill_batch_leaves_no_stale_projection() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_bytes();
        let key = event_replacement_lock_key(community, 0, &pubkey, None);

        let head = kind0(
            &keys,
            &format!(r#"{{"name":"wes","nip05":"wes@{host}"}}"#),
            1_700_000_010,
        );
        replace_profile(&db, community, &host, &head).await;

        // The batch: lock, then read the head, in `backfill_profiles_batch` order.
        let mut batch = db.pool.begin().await.expect("begin batch tx");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut *batch)
            .await
            .expect("batch takes the lock");
        let heads = current_profile_events_on(&mut batch, community, &[pubkey.to_vec()])
            .await
            .expect("batch reads heads");
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].event.id, head.id, "the batch read H1 as the head");

        // The kind-5 lands while the batch is between its read and its write.
        let pool = db.pool.clone();
        let head_id = head.id.as_bytes().to_vec();
        let delete = tokio::spawn(async move {
            crate::event::soft_delete_event_and_update_thread(
                &pool, community, &head_id, None, None,
            )
            .await
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let waiters: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks \
                 WHERE locktype = 'advisory' AND NOT granted \
                   AND classid = $1::oid AND objid = $2::oid AND objsubid = 1",
            )
            .bind(((key as u64) >> 32) as i64)
            .bind(((key as u64) & 0xffff_ffff) as i64)
            .fetch_one(&db.pool)
            .await
            .expect("inspect pg_locks");
            if waiters == 1 {
                break;
            }
            assert!(
                !delete.is_finished(),
                "the delete finished without queueing behind the batch: the hook took no lock"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for the delete to queue on the replace lock"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        // The batch writes the head it read, as the real batch would, and commits.
        let projection = ProfileProjection::from_kind0_event(&heads[0].event, &host);
        upsert_profile_in_transaction(&mut batch, community, &pubkey, &projection)
            .await
            .expect("batch upsert");
        batch.commit().await.expect("commit batch");

        assert!(
            delete.await.expect("join delete").expect("soft delete"),
            "the head is deleted once the batch commits"
        );
        let row = users_row(&db.pool, community, &pubkey)
            .await
            .expect("users row");
        assert_eq!(
            row.5, None,
            "the batch's write of H1 cannot outlive H1's deletion"
        );
        assert_eq!(row.4, None, "the handle is released with it");
        let after = precheck_for(&db, community).await;
        assert_eq!(
            (
                after.kind0_publishers,
                after.users_rows,
                after.unprojected_publishers
            ),
            (0, 1, 0),
            "no live head, nothing unprojected: {after:?}"
        );
    }

    /// Heads written without a projection (as every kind-0 did before this
    /// seam) are picked up by the batched backfill, which pages by pubkey,
    /// uses the current head rather than a superseded row, and converges the
    /// precheck's `unprojected_publishers` to zero.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn backfill_projects_current_heads_and_precheck_converges() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let mut keys: Vec<Keys> = (0..3).map(|_| Keys::generate()).collect();
        keys.sort_by_key(|k| k.public_key().to_bytes());

        for (i, k) in keys.iter().enumerate() {
            db.replace_addressable_event(
                community,
                &kind0(k, &format!(r#"{{"name":"stale-{i}"}}"#), 1_700_000_000),
                None,
            )
            .await
            .expect("stale head");
            db.replace_addressable_event(
                community,
                &kind0(k, &format!(r#"{{"name":"current-{i}"}}"#), 1_700_000_100),
                None,
            )
            .await
            .expect("current head");
        }
        // One publisher already has a bare row (as AUTH creates them); the
        // others have none.
        crate::user::ensure_user(&db.pool, community, &keys[1].public_key().to_bytes())
            .await
            .expect("ensure user");

        let before = precheck_for(&db, community).await;
        assert_eq!(
            (
                before.kind0_publishers,
                before.users_rows,
                before.unprojected_publishers
            ),
            (3, 1, 3),
            "before the backfill every publisher is unprojected: {before:?}"
        );

        let first = db
            .backfill_profiles_batch(community, &host, None, 2)
            .await
            .expect("first batch");
        assert_eq!(first.scanned, 2);
        assert_eq!(first.projected, 2);
        assert_eq!(first.created, 1, "keys[1] already had a row");
        assert_eq!(
            first.last_pubkey.as_deref(),
            Some(keys[1].public_key().to_bytes().as_slice())
        );

        let second = db
            .backfill_profiles_batch(community, &host, first.last_pubkey.as_deref(), 2)
            .await
            .expect("second batch");
        assert_eq!(
            (second.scanned, second.projected, second.created),
            (1, 1, 1)
        );

        let done = db
            .backfill_profiles_batch(community, &host, second.last_pubkey.as_deref(), 2)
            .await
            .expect("terminal batch");
        assert_eq!(done, ProfileBackfillBatch::default(), "scan is complete");

        for (i, k) in keys.iter().enumerate() {
            let row = users_row(&db.pool, community, &k.public_key().to_bytes())
                .await
                .expect("projected row");
            assert_eq!(row.0.as_deref(), Some(format!("current-{i}").as_str()));
            let head = db
                .get_current_profile_events(community, &[k.public_key().to_bytes().to_vec()])
                .await
                .expect("head");
            assert_eq!(
                row.5.as_deref(),
                Some(head[0].event.id.as_bytes().as_slice())
            );
        }
        let after = precheck_for(&db, community).await;
        assert_eq!(
            (
                after.kind0_publishers,
                after.users_rows,
                after.unprojected_publishers
            ),
            (3, 3, 0),
            "after the backfill nothing is left to project: {after:?}"
        );
    }

    /// Review scenario: `a`'s row still holds `x@host` from a head that has
    /// since moved to `y@host` without being re-projected, and `b`'s current
    /// head claims `x@host`. With `b` visited first (batch size 1, `b` sorts
    /// below `a`), the old fallback projected `b` without the handle and left
    /// it for a second pass; now the stale holder releases it and one pass
    /// converges, across batches.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn backfill_releases_a_stale_nip05_holder_in_one_pass() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let mut keys: Vec<Keys> = (0..2).map(|_| Keys::generate()).collect();
        keys.sort_by_key(|k| k.public_key().to_bytes());
        let (b, a) = (&keys[0], &keys[1]);
        let a_pubkey = a.public_key().to_bytes();
        let b_pubkey = b.public_key().to_bytes();

        let a_stale = kind0(a, &format!(r#"{{"nip05":"x@{host}"}}"#), 1_700_000_010);
        replace_profile(&db, community, &host, &a_stale).await;
        // `a` moves on, but the way every kind-0 landed before the seam: the
        // head changes and the row does not follow.
        let a_head = kind0(a, &format!(r#"{{"nip05":"y@{host}"}}"#), 1_700_000_020);
        db.replace_addressable_event(community, &a_head, None)
            .await
            .expect("a's current head");
        let b_head = kind0(
            b,
            &format!(r#"{{"name":"b","nip05":"x@{host}"}}"#),
            1_700_000_030,
        );
        db.replace_addressable_event(community, &b_head, None)
            .await
            .expect("b's head");

        let before = precheck_for(&db, community).await;
        assert_eq!(
            (before.users_rows, before.unprojected_publishers),
            (1, 2),
            "{before:?}"
        );

        let first = db
            .backfill_profiles_batch(community, &host, None, 1)
            .await
            .expect("batch with b");
        assert_eq!((first.scanned, first.projected, first.created), (1, 1, 1));
        assert_eq!(first.last_pubkey.as_deref(), Some(b_pubkey.as_slice()));
        let b_row = users_row(&db.pool, community, &b_pubkey)
            .await
            .expect("b row");
        assert_eq!(
            (b_row.4, b_row.5),
            (
                Some(format!("x@{host}")),
                Some(b_head.id.as_bytes().to_vec())
            ),
            "b takes the handle in the batch that projects it"
        );
        let a_row = users_row(&db.pool, community, &a_pubkey)
            .await
            .expect("a row");
        assert_eq!(
            (a_row.4, a_row.5),
            (None, Some(a_stale.id.as_bytes().to_vec())),
            "only the handle is released from a's row; its own batch re-projects the rest"
        );

        let second = db
            .backfill_profiles_batch(community, &host, first.last_pubkey.as_deref(), 1)
            .await
            .expect("batch with a");
        assert_eq!(
            (second.scanned, second.projected, second.created),
            (1, 1, 0)
        );
        let a_row = users_row(&db.pool, community, &a_pubkey)
            .await
            .expect("a row");
        assert_eq!(
            (a_row.4, a_row.5),
            (
                Some(format!("y@{host}")),
                Some(a_head.id.as_bytes().to_vec())
            )
        );

        let done = db
            .backfill_profiles_batch(community, &host, second.last_pubkey.as_deref(), 1)
            .await
            .expect("terminal batch");
        assert_eq!(done, ProfileBackfillBatch::default());
        let after = precheck_for(&db, community).await;
        assert_eq!(
            (after.users_rows, after.unprojected_publishers),
            (2, 0),
            "one pass converged: {after:?}"
        );
    }

    /// The same release applies to a live publish: a profile claiming a handle
    /// whose row-holder's head has moved on takes it immediately, and only
    /// the handle leaves the holder's row.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replace_profile_event_takes_a_handle_from_a_stale_holder() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let holder = Keys::generate();
        let claimant = Keys::generate();
        let holder_pubkey = holder.public_key().to_bytes();

        let stale = kind0(
            &holder,
            &format!(r#"{{"name":"holder","nip05":"x@{host}"}}"#),
            1_700_000_010,
        );
        replace_profile(&db, community, &host, &stale).await;
        db.replace_addressable_event(
            community,
            &kind0(&holder, r#"{"name":"holder"}"#, 1_700_000_020),
            None,
        )
        .await
        .expect("holder's current head drops the handle without re-projecting");

        let claim = kind0(
            &claimant,
            &format!(r#"{{"name":"claimant","nip05":"x@{host}"}}"#),
            1_700_000_030,
        );
        let outcome = replace_profile(&db, community, &host, &claim).await;
        assert!(outcome.was_inserted);
        assert_eq!(
            users_row(&db.pool, community, &claimant.public_key().to_bytes())
                .await
                .and_then(|row| row.4),
            Some(format!("x@{host}")),
            "the claimant takes the handle on its first write"
        );
        assert_eq!(
            users_row(&db.pool, community, &holder_pubkey).await,
            Some((
                Some("holder".into()),
                None,
                None,
                None,
                None,
                Some(stale.id.as_bytes().to_vec()),
            )),
            "the stale holder loses only the handle"
        );
    }

    /// Hydration returns exactly one live head per pubkey — the newest — and
    /// nothing for pubkeys without a live kind-0.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_current_profile_events_returns_one_live_head_per_pubkey() {
        let db = setup_db().await;
        let (community, host) = make_community(&db.pool).await;
        let a = Keys::generate();
        let b = Keys::generate();
        let none = Keys::generate();

        replace_profile(
            &db,
            community,
            &host,
            &kind0(&a, r#"{"name":"a1"}"#, 1_700_000_010),
        )
        .await;
        let a_head = kind0(&a, r#"{"name":"a2"}"#, 1_700_000_020);
        replace_profile(&db, community, &host, &a_head).await;
        let b_head = kind0(&b, r#"{"name":"b"}"#, 1_700_000_010);
        replace_profile(&db, community, &host, &b_head).await;

        let mut heads = db
            .get_current_profile_events(
                community,
                &[
                    a.public_key().to_bytes().to_vec(),
                    b.public_key().to_bytes().to_vec(),
                    none.public_key().to_bytes().to_vec(),
                ],
            )
            .await
            .expect("heads");
        heads.sort_by_key(|e| e.event.pubkey.to_bytes());
        let mut expected = vec![a_head.id, b_head.id];
        expected.sort_by_key(|id| {
            if *id == a_head.id {
                a.public_key().to_bytes()
            } else {
                b.public_key().to_bytes()
            }
        });
        assert_eq!(
            heads.iter().map(|e| e.event.id).collect::<Vec<_>>(),
            expected,
            "one newest live head per pubkey, none for an unknown pubkey"
        );
    }

    async fn precheck_for(db: &Db, community: CommunityId) -> ProfileBackfillPrecheck {
        db.profile_backfill_precheck()
            .await
            .expect("precheck")
            .into_iter()
            .find(|row| row.community_id == community)
            .expect("community in precheck")
    }
}
