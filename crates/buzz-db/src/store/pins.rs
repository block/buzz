//! NIP-PM channel pins.
//!
//! A channel's pins are one relay-signed kind:30623 event. Every writer takes
//! that event's replacement lock before it reads the list, so the pin and
//! unpin commands of a channel, and the deletes that remove a pinned message,
//! apply one at a time.

use buzz_core::channel::MAX_CHANNEL_PINS;
use buzz_core::kind::{KIND_CHANNEL_PINS, KIND_PIN_MESSAGE, KIND_UNPIN_MESSAGE, PINNABLE_KINDS};
use buzz_core::{CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::{AdmittedTx, Db};

pub(crate) const COMMAND_KINDS: [i32; 2] = [KIND_PIN_MESSAGE as i32, KIND_UNPIN_MESSAGE as i32];

/// How long the ID and timestamp of a pin or unpin command outlive account
/// erasure. A resend of the command is still a duplicate for that long; the
/// relay accepts no command with an older timestamp.
const REPLAY_RECORD_SECS: i64 = 60 * 60;

/// Whether account erasure reduces this event to its ID and timestamp rather
/// than removing it: a pin or unpin command young enough to be sent again.
pub(crate) fn keeps_replay_record(
    kind: i32,
    created_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    COMMAND_KINDS.contains(&kind)
        && created_at > now - chrono::Duration::seconds(REPLAY_RECORD_SECS)
}

/// What a NIP-PM command does to its target message.
pub enum PinChange<'a> {
    /// Pin the target. `notice` is the relay-signed kind:40099 system message
    /// that announces it, stored only when the pin changes the list.
    Pin {
        /// The `message_pinned` system message for this pin.
        notice: &'a nostr::Event,
    },
    /// Unpin the target.
    Unpin,
}

/// What storing one NIP-PM command did.
pub struct PinCommandWrite {
    /// The command.
    pub stored: StoredEvent,
    /// `false` when this exact command was stored before. It was not applied
    /// again.
    pub inserted: bool,
    /// The channel's new pin list, when the command changed it.
    pub list: Option<StoredEvent>,
}

/// Take the replacement lock of `channel_id`'s pin list, then read the pinned
/// message IDs behind it, newest pin first.
async fn lock_pins(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
    relay_pubkey: &[u8],
) -> Result<Vec<Vec<u8>>> {
    let kind = KIND_CHANNEL_PINS as i32;
    crate::replaceable::take_replacement_lock(tx, kind, relay_pubkey, Some(channel_id)).await?;
    let tags: Option<Vec<Vec<String>>> = sqlx::query_scalar::<_, sqlx::types::Json<_>>(
        "SELECT tags FROM events WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
         AND channel_id = $4 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
    )
    .bind(tx.community().as_uuid())
    .bind(kind)
    .bind(relay_pubkey)
    .bind(channel_id)
    .fetch_optional(tx.conn())
    .await?
    .map(|tags| tags.0);
    pinned_ids(&tags.unwrap_or_default())
}

/// Take the replacement lock of `channel_id`'s pin list, then read the pinned
/// message IDs behind the list head that [`replace_pins`] recorded.
///
/// Every message delete calls this. Reading the head's
/// exact `created_at` first keeps it to the list's own `events` partition, and
/// to none when the channel has never had a list. Taking the lock before the
/// read means a pin that commits first is seen.
async fn lock_recorded_pins(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
    relay_pubkey: &[u8],
) -> Result<Vec<Vec<u8>>> {
    let kind = KIND_CHANNEL_PINS as i32;
    crate::replaceable::take_replacement_lock(tx, kind, relay_pubkey, Some(channel_id)).await?;
    let head: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT created_at FROM parameterized_event_watermarks \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4",
    )
    .bind(tx.community().as_uuid())
    .bind(kind)
    .bind(relay_pubkey)
    .bind(channel_id.to_string())
    .fetch_optional(tx.conn())
    .await?;
    let Some(head) = head else {
        return Ok(Vec::new());
    };
    // Unnamed, so it is always planned with `head` and prunes to its month
    // (see the known-target statements in `event_deletion`).
    let tags: Option<Vec<Vec<String>>> = sqlx::query_scalar::<_, sqlx::types::Json<_>>(
        "SELECT tags FROM events WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
         AND channel_id = $4 AND created_at = $5 AND deleted_at IS NULL ORDER BY id ASC LIMIT 1",
    )
    .persistent(false)
    .bind(tx.community().as_uuid())
    .bind(kind)
    .bind(relay_pubkey)
    .bind(channel_id)
    .bind(head)
    .fetch_optional(tx.conn())
    .await?
    .map(|tags| tags.0);
    pinned_ids(&tags.unwrap_or_default())
}

/// The message IDs a pin list's tags pin, in order.
fn pinned_ids(tags: &[Vec<String>]) -> Result<Vec<Vec<u8>>> {
    tags.iter()
        .filter(|tag| tag.first().is_some_and(|name| name == "e"))
        .map(|tag| {
            tag.get(1)
                .and_then(|id| hex::decode(id).ok())
                .ok_or_else(|| DbError::InvalidData("malformed pin list entry".to_string()))
        })
        .collect()
}

/// The entries of `ids` that are live messages of `channel_id`, in order.
async fn live_messages(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
    mut ids: Vec<Vec<u8>>,
) -> Result<Vec<Vec<u8>>> {
    // Unnamed, so by default it is planned with the channel and the IDs and
    // probes the ID index. A cached generic plan assumes an average channel
    // and reads this channel's whole index instead.
    let live: std::collections::HashSet<Vec<u8>> = sqlx::query_scalar(
        "SELECT id FROM events WHERE community_id = $1 AND channel_id = $2 AND id = ANY($3) \
         AND kind = ANY($4) AND deleted_at IS NULL",
    )
    .persistent(false)
    .bind(tx.community().as_uuid())
    .bind(channel_id)
    .bind(&ids)
    .bind(PINNABLE_KINDS.map(|kind| kind as i32).as_slice())
    .fetch_all(tx.conn())
    .await?
    .into_iter()
    .collect();
    ids.retain(|id| live.contains(id));
    Ok(ids)
}

/// Sign and store `ids` as `channel_id`'s pin list, and record it as the
/// list head [`lock_recorded_pins`] reads. The caller holds the list's
/// replacement lock ([`lock_pins`]) and commits.
async fn replace_pins(
    tx: &mut AdmittedTx,
    channel_id: Uuid,
    ids: &[Vec<u8>],
    relay_signer: &nostr::Keys,
) -> Result<StoredEvent> {
    let tag = |name: &str, value: &str| {
        nostr::Tag::parse([name, value]).map_err(|error| DbError::InvalidData(error.to_string()))
    };
    let channel = channel_id.to_string();
    // `d` addresses the list; `h` lets a `#h` filter, which names the channel
    // (NIP-CL), match it.
    let mut tags = vec![tag("d", &channel)?, tag("h", &channel)?];
    for id in ids {
        tags.push(tag("e", &hex::encode(id))?);
    }
    let (stored, inserted) = crate::channel::replace_signed_group_state_under_lock(
        tx,
        channel_id,
        KIND_CHANNEL_PINS as i32,
        tags,
        relay_signer,
    )
    .await?;
    if !inserted {
        return Err(DbError::InvalidData(
            "pin list replacement failed".to_string(),
        ));
    }
    let created_at_secs = stored.event.created_at.as_secs() as i64;
    let created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
    sqlx::query(
        "INSERT INTO parameterized_event_watermarks \
             (community_id, kind, pubkey, d_tag, created_at, event_id) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (community_id, kind, pubkey, d_tag) DO UPDATE SET \
             created_at = EXCLUDED.created_at, event_id = EXCLUDED.event_id",
    )
    .bind(tx.community().as_uuid())
    .bind(KIND_CHANNEL_PINS as i32)
    .bind(stored.event.pubkey.to_bytes().as_slice())
    .bind(&channel)
    .bind(created_at)
    .bind(stored.event.id.as_bytes().as_slice())
    .execute(tx.conn())
    .await?;
    Ok(stored)
}

/// Remove deleted messages from their channels' pin lists, on the transaction
/// that deleted them. `deleted` holds each message's channel and ID. Returns
/// each list that held one of them, rewritten once.
///
/// The caller deletes every message first. A delete then never waits for a
/// message while it holds a list lock, and takes the list locks in one order,
/// so two deletes cannot wait on each other.
pub(crate) async fn unpin_deleted_messages(
    tx: &mut AdmittedTx,
    deleted: &[(Uuid, &[u8])],
    relay_signer: &nostr::Keys,
) -> Result<Vec<StoredEvent>> {
    let mut channels: std::collections::BTreeMap<Uuid, Vec<&[u8]>> = Default::default();
    for (channel_id, event_id) in deleted {
        channels.entry(*channel_id).or_default().push(event_id);
    }
    let mut lists = Vec::new();
    for (channel_id, event_ids) in channels {
        let pins =
            lock_recorded_pins(tx, channel_id, &relay_signer.public_key().to_bytes()).await?;
        if !pins.iter().any(|id| event_ids.contains(&id.as_slice())) {
            continue;
        }
        let pins = live_messages(tx, channel_id, pins).await?;
        lists.push(replace_pins(tx, channel_id, &pins, relay_signer).await?);
    }
    Ok(lists)
}

/// The author column of a command record that account erasure scrubbed.
const ERASED: &[u8] = &[];

/// Remove the community's erased command records from `cutoff` and before.
async fn delete_erased_command_records(tx: &mut AdmittedTx, cutoff: DateTime<Utc>) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM events WHERE community_id = $1 AND kind = ANY($2) \
         AND pubkey = $3 AND created_at <= $4",
    )
    .bind(tx.community().as_uuid())
    .bind(COMMAND_KINDS.as_slice())
    .bind(ERASED)
    .bind(cutoff)
    .execute(tx.conn())
    .await?;
    Ok(result.rows_affected())
}

impl Db {
    /// Store a NIP-PM pin (kind:40010) or unpin (kind:40011) command and
    /// apply it to `channel_id`'s pin list, in one transaction.
    ///
    /// Pinning a message that is already pinned, or unpinning one that is
    /// not, stores the command and leaves the list alone. A list that
    /// changes also drops every entry whose message is gone.
    ///
    /// Account erasure keeps the ID and timestamp of a command under an hour
    /// old, so a resend of it is still a duplicate here.
    ///
    /// Returns [`DbError::AccessDenied`], with nothing stored, when the
    /// target of a pin is not a live message of the channel or the channel
    /// already holds [`MAX_CHANNEL_PINS`] pins.
    #[datastore_span(name = "apply_pin_command", system = "postgresql")]
    pub async fn apply_pin_command(
        &self,
        community: CommunityId,
        channel_id: Uuid,
        event: &nostr::Event,
        target: &[u8],
        change: PinChange<'_>,
        relay_signer: &nostr::Keys,
    ) -> Result<PinCommandWrite> {
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        // The lock also orders two copies of the same command: the second
        // waits, then finds the first stored.
        let mut pins =
            lock_pins(&mut tx, channel_id, &relay_signer.public_key().to_bytes()).await?;
        let (stored, inserted) = crate::event::insert_event_with_thread_metadata_tx(
            &mut tx,
            event,
            Some(channel_id),
            None,
        )
        .await?;
        if !inserted {
            tx.rollback().await?;
            return Ok(PinCommandWrite {
                stored,
                inserted,
                list: None,
            });
        }

        let pinned = pins.iter().position(|id| id == target);
        let list = match (change, pinned) {
            (PinChange::Pin { notice }, pinned) => {
                // A listed message can be gone: bulk erasure leaves the list.
                if live_messages(&mut tx, channel_id, vec![target.to_vec()])
                    .await?
                    .is_empty()
                {
                    return Err(DbError::AccessDenied(
                        "target is not a message in this channel".to_string(),
                    ));
                }
                if pinned.is_some() {
                    None
                } else {
                    pins = live_messages(&mut tx, channel_id, pins).await?;
                    if pins.len() >= MAX_CHANNEL_PINS {
                        return Err(DbError::AccessDenied("pin limit reached".to_string()));
                    }
                    pins.insert(0, target.to_vec());
                    let list = replace_pins(&mut tx, channel_id, &pins, relay_signer).await?;
                    crate::event::insert_event_in_transaction(&mut tx, notice, Some(channel_id))
                        .await?;
                    Some(list)
                }
            }
            (PinChange::Unpin, Some(index)) => {
                pins.remove(index);
                pins = live_messages(&mut tx, channel_id, pins).await?;
                Some(replace_pins(&mut tx, channel_id, &pins, relay_signer).await?)
            }
            (PinChange::Unpin, None) => None,
        };
        tx.commit().await?;
        Ok(PinCommandWrite {
            stored,
            inserted,
            list,
        })
    }

    /// Remove, in every community, the records account erasure left of pin
    /// and unpin commands that are now an hour old. Returns how many.
    ///
    /// The relay calls this from its leader-only periodic tick, so a record
    /// goes whether or not its channel sees another command.
    #[datastore_span(name = "reap_expired_pin_command_records", system = "postgresql")]
    pub async fn reap_expired_pin_command_records(&self) -> Result<u64> {
        let cutoff = Utc::now() - chrono::Duration::seconds(REPLAY_RECORD_SECS);
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        // Every index on `events` leads with the community, so probe once for
        // each community rather than scan the table for the kinds.
        let communities: Vec<Uuid> = sqlx::query_scalar(
            "SELECT community.id FROM communities community \
             WHERE community.deletion_state = 'active' AND community.deleted_at IS NULL \
               AND EXISTS (SELECT 1 FROM events record \
                 WHERE record.community_id = community.id AND record.kind = ANY($1) \
                   AND record.pubkey = $2 AND record.created_at <= $3)",
        )
        .bind(COMMAND_KINDS.as_slice())
        .bind(ERASED)
        .bind(cutoff)
        .fetch_all(&mut *connection)
        .await?;
        drop(connection);

        let removed = crate::sweep_admitted_communities(
            &self.pool,
            communities,
            crate::observability::WriterOperation::Maintenance,
            "pin_command_record_reap",
            |_, mut tx| async move {
                let removed = delete_erased_command_records(&mut tx, cutoff).await?;
                tx.commit().await?;
                Ok(removed)
            },
        )
        .await;
        Ok(removed.into_iter().sum())
    }
}

#[cfg(test)]
mod postgres_tests;
