//! The community's custom emoji catalog (NIP-CE): one row per shortcode.
//!
//! Each kind:30180 entry a community accepts is stored as an event and applied
//! to the `custom_emoji` row for its shortcode in the same transaction. A
//! shortcode belongs to whoever published it first; only that member, or a
//! community owner or admin, may replace or remove it.
//!
//! Every write takes the community's catalog lock and gives its row the next
//! `change` number, so changes commit in `change` order. A reader that holds
//! everything up to change `n` therefore needs only rows above `n`, and never
//! misses a row that committed late.

use buzz_core::{kind::KIND_EMOJI_ENTRY, CommunityId, StoredEvent};
use sqlx::Row;

use crate::{
    observability,
    replaceable::{
        event_replacement_lock_key, ParameterizedReplacePrecondition, ParameterizedReplaceStatus,
    },
    AdmittedTx, Db, Result,
};

/// Most rows one [`Db::custom_emoji_changes`] page returns.
pub const CUSTOM_EMOJI_PAGE_MAX: u32 = 10_000;
/// Most shortcodes one [`Db::custom_emoji_by_shortcode`] lookup accepts.
pub const CUSTOM_EMOJI_LOOKUP_MAX: usize = 100;

/// One row of the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomEmojiRow {
    /// Normalized shortcode, without colons.
    pub shortcode: String,
    /// Position of this row's latest write in the community's change order.
    pub change: i64,
    /// Image URL, or `None` when the shortcode has been removed.
    pub url: Option<String>,
    /// Owner of the shortcode, or whoever removed it.
    pub pubkey: [u8; 32],
}

/// What the catalog did with a submitted entry.
#[derive(Clone, Debug)]
pub enum CustomEmojiWrite {
    /// The entry was stored and the catalog row now reflects it.
    Applied(StoredEvent),
    /// The exact event is already stored.
    Unchanged(StoredEvent),
    /// The author already has an entry for the shortcode that is newer, or
    /// from the same second with a lower event ID. Nothing changed; the author
    /// must publish again with a later `created_at`.
    Stale,
    /// Another member owns the shortcode and the author may not change it.
    Taken,
    /// A removal named a shortcode that has no live emoji.
    NothingToRemove,
}

/// Advisory lock key that orders every catalog write in one community.
fn catalog_lock_key(community: CommunityId) -> i64 {
    event_replacement_lock_key(
        community,
        KIND_EMOJI_ENTRY as i32,
        &[],
        Some(b"buzz:custom-emoji-catalog"),
    )
}

fn row(row: &sqlx::postgres::PgRow) -> Result<CustomEmojiRow> {
    let pubkey: Vec<u8> = row.try_get("pubkey")?;
    Ok(CustomEmojiRow {
        shortcode: row.try_get("shortcode")?,
        change: row.try_get("change")?,
        url: row.try_get("url")?,
        pubkey: pubkey
            .try_into()
            .map_err(|_| sqlx::Error::Decode("custom_emoji.pubkey is not 32 bytes".into()))?,
    })
}

async fn lock_catalog(tx: &mut AdmittedTx) -> Result<()> {
    let key = catalog_lock_key(tx.community());
    observability::observe_event_lock(
        observability::LockType::Replacement,
        KIND_EMOJI_ENTRY as i32,
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(tx.conn()),
    )
    .await?;
    Ok(())
}

/// Remove from the catalog the emoji whose entry is `event_id`, on `tx`.
///
/// For paths that delete a kind:30180 event without a removal entry, such as
/// a moderation delete: the shortcode leaves the catalog in the same commit
/// and is free to claim. Does nothing when the catalog does not hold that
/// event.
pub(crate) async fn remove_entry_event_in_transaction(
    tx: &mut AdmittedTx,
    event_id: &[u8],
) -> Result<()> {
    lock_catalog(tx).await?;
    let community = tx.community();
    sqlx::query(
        "UPDATE custom_emoji SET url = NULL,              change = (SELECT COALESCE(MAX(change), 0) + 1 FROM custom_emoji WHERE community_id = $1)          WHERE community_id = $1 AND event_id = $2 AND url IS NOT NULL",
    )
    .bind(community.as_uuid())
    .bind(event_id)
    .execute(tx.conn())
    .await?;
    Ok(())
}

/// Apply one validated kind:30180 entry on `tx`. `url` is the entry's image,
/// or `None` for a removal. The caller commits only on
/// [`CustomEmojiWrite::Applied`] and rolls back otherwise.
async fn apply_entry(
    db: &Db,
    tx: &mut AdmittedTx,
    event: &nostr::Event,
    shortcode: &str,
    url: Option<&str>,
) -> Result<CustomEmojiWrite> {
    let community = tx.community();
    let author = event.pubkey.to_bytes();

    // Before the event's own replacement lock, on every catalog write.
    lock_catalog(tx).await?;

    let current = sqlx::query(
        "SELECT pubkey, url IS NOT NULL AS live FROM custom_emoji \
         WHERE community_id = $1 AND shortcode = $2",
    )
    .bind(community.as_uuid())
    .bind(shortcode)
    .fetch_optional(tx.conn())
    .await?;
    let (owner, live) = match &current {
        Some(current) => (
            Some(current.try_get::<Vec<u8>, _>("pubkey")?),
            current.try_get::<bool, _>("live")?,
        ),
        None => (None, false),
    };
    if url.is_none() && !live {
        return Ok(CustomEmojiWrite::NothingToRemove);
    }
    // A removed shortcode is free for anyone to claim.
    let owned_by_other = live && owner.as_deref() != Some(author.as_slice());
    if owned_by_other {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM relay_members WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(event.pubkey.to_hex())
        .fetch_optional(tx.conn())
        .await?;
        if !matches!(role.as_deref(), Some("owner" | "admin")) {
            return Ok(CustomEmojiWrite::Taken);
        }
    }

    let stored = db
        .replace_parameterized_event_in_transaction(
            tx,
            event,
            shortcode,
            None,
            ParameterizedReplacePrecondition::Unconditional,
        )
        .await?;
    match stored.status {
        ParameterizedReplaceStatus::Inserted => {}
        ParameterizedReplaceStatus::Duplicate => {
            return Ok(CustomEmojiWrite::Unchanged(stored.event))
        }
        _ => return Ok(CustomEmojiWrite::Stale),
    }

    // Keep one live entry event per shortcode: retire the previous holder's.
    if let Some(previous) = owner.filter(|owner| owner.as_slice() != author.as_slice()) {
        sqlx::query(
            "UPDATE events SET deleted_at = NOW() \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 \
               AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(KIND_EMOJI_ENTRY as i32)
        .bind(previous)
        .bind(shortcode)
        .execute(tx.conn())
        .await?;
    }

    sqlx::query(
        "INSERT INTO custom_emoji (community_id, shortcode, change, url, pubkey, event_id) \
         VALUES ($1, $2, \
                 (SELECT COALESCE(MAX(change), 0) + 1 FROM custom_emoji WHERE community_id = $1), \
                 $3, $4, $5) \
         ON CONFLICT (community_id, shortcode) DO UPDATE \
         SET change = EXCLUDED.change, url = EXCLUDED.url, \
             pubkey = EXCLUDED.pubkey, event_id = EXCLUDED.event_id",
    )
    .bind(community.as_uuid())
    .bind(shortcode)
    .bind(url)
    .bind(author.as_slice())
    .bind(event.id.as_bytes().as_slice())
    .execute(tx.conn())
    .await?;

    Ok(CustomEmojiWrite::Applied(stored.event))
}

impl Db {
    /// Store one validated kind:30180 entry and apply it to the catalog in one
    /// commit. `shortcode` is the entry's normalized `d` value and `url` its
    /// image, or `None` when the entry removes the shortcode.
    pub async fn put_custom_emoji_entry(
        &self,
        community: CommunityId,
        event: &nostr::Event,
        shortcode: &str,
        url: Option<&str>,
    ) -> Result<CustomEmojiWrite> {
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            observability::WriterOperation::EventWrite,
        )
        .await?;
        tx.set_push_enqueue(self.push_enqueue.clone());
        let write = apply_entry(self, &mut tx, event, shortcode, url).await?;
        if matches!(write, CustomEmojiWrite::Applied(_)) {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(write)
    }

    /// Catalog rows written after change `since`, oldest change first, and
    /// whether more remain past this page. `since` 0 reads the catalog from
    /// the start and leaves out removed shortcodes, which a reader with
    /// nothing has no use for.
    pub async fn custom_emoji_changes(
        &self,
        community: CommunityId,
        since: i64,
        limit: u32,
    ) -> Result<(Vec<CustomEmojiRow>, bool)> {
        let limit = i64::from(limit.clamp(1, CUSTOM_EMOJI_PAGE_MAX));
        let mut connection = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut rows = sqlx::query(
            "SELECT shortcode, change, url, pubkey FROM custom_emoji \
             WHERE community_id = $1 AND change > $2 AND ($2 > 0 OR url IS NOT NULL) \
             ORDER BY change LIMIT $3",
        )
        .bind(community.as_uuid())
        .bind(since.max(0))
        .bind(limit + 1)
        .fetch_all(&mut *connection)
        .await?
        .iter()
        .map(row)
        .collect::<Result<Vec<_>>>()?;
        let more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        Ok((rows, more))
    }

    /// The live catalog rows for `shortcodes`, at most
    /// [`CUSTOM_EMOJI_LOOKUP_MAX`] of them. Unknown and removed shortcodes are
    /// left out.
    pub async fn custom_emoji_by_shortcode(
        &self,
        community: CommunityId,
        shortcodes: &[String],
    ) -> Result<Vec<CustomEmojiRow>> {
        let shortcodes = &shortcodes[..shortcodes.len().min(CUSTOM_EMOJI_LOOKUP_MAX)];
        let mut connection = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        sqlx::query(
            "SELECT shortcode, change, url, pubkey FROM custom_emoji \
             WHERE community_id = $1 AND shortcode = ANY($2) AND url IS NOT NULL",
        )
        .bind(community.as_uuid())
        .bind(shortcodes)
        .fetch_all(&mut *connection)
        .await?
        .iter()
        .map(row)
        .collect()
    }
}
