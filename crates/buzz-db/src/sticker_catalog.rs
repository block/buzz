//! Revision-pinned, tenant-scoped workspace sticker catalog.

use nostr::{EventBuilder, Kind, Tag, Timestamp};
use serde_json::Value;
use sqlx::Row;
use std::collections::HashSet;

use buzz_core::kind::{KIND_STICKER_CATALOG, KIND_STICKER_PACK};
use buzz_core::stickers::MAX_STICKER_CATALOG_PACKS;
use buzz_core::{CommunityId, StoredEvent};

use crate::{event_replacement_lock_key, Db, DbError, Result};

/// Requested mutation of a workspace sticker catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickerCatalogAction<'a> {
    /// Pin the current pack head to this exact event revision.
    Approve {
        /// Exact current kind:30031 event ID reviewed by the administrator.
        event_id: &'a [u8],
    },
    /// Remove the coordinate from the workspace catalog.
    Remove,
}

/// Result of a catalog mutation and its relay-authored snapshot publication.
#[derive(Debug)]
pub struct StickerCatalogMutation {
    /// Newly built relay-signed kind:13536 snapshot.
    pub snapshot: StoredEvent,
    /// Whether the snapshot row was inserted.
    pub was_inserted: bool,
    /// Whether the approval table changed.
    pub changed: bool,
    /// Number of approved pack revisions in the resulting catalog.
    pub approval_count: usize,
}

fn invalid(message: impl Into<String>) -> DbError {
    DbError::InvalidData(message.into())
}

/// Validate enough of a stored pack's structure to prevent legacy malformed
/// kind:30031 rows from being promoted into the curated catalog. New writes are
/// subject to the relay's complete Sonar validation before storage.
fn is_structurally_valid_pack(tags: &Value, identifier: &str) -> bool {
    let Some(tags) = tags.as_array() else {
        return false;
    };

    let exact_count = |name: &str, value: &str| {
        tags.iter()
            .filter(|tag| {
                tag.as_array().is_some_and(|parts| {
                    parts.len() == 2
                        && parts[0].as_str() == Some(name)
                        && parts[1].as_str() == Some(value)
                })
            })
            .count()
    };
    let title_count = tags
        .iter()
        .filter(|tag| {
            tag.as_array().is_some_and(|parts| {
                parts.len() == 2
                    && parts[0].as_str() == Some("title")
                    && parts[1].as_str().is_some_and(|title| !title.is_empty())
            })
        })
        .count();
    let sticker_tags: Vec<_> = tags
        .iter()
        .filter(|tag| {
            tag.as_array()
                .and_then(|parts| parts.first())
                .and_then(Value::as_str)
                == Some("sticker")
        })
        .collect();
    let mut shortcodes = HashSet::new();
    let mut hashes = HashSet::new();
    let stickers_valid = sticker_tags.iter().all(|tag| {
        let Some(parts) = tag.as_array() else {
            return false;
        };
        if !(6..=8).contains(&parts.len()) {
            return false;
        }
        let fields: Option<Vec<&str>> = parts.iter().map(Value::as_str).collect();
        let Some(fields) = fields else {
            return false;
        };
        let shortcode = fields[1];
        let url = fields[2];
        let hash = fields[3];
        let mime = fields[4];
        let dim = fields[5];
        let alt = fields.get(6).copied().unwrap_or("");
        let emoji = fields.get(7).copied();
        let shortcode_valid = !shortcode.is_empty()
            && shortcode.len() <= 64
            && shortcode
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && shortcodes.insert(shortcode);
        let hash_valid = hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            && hashes.insert(hash);
        let url_valid = url::Url::parse(url).is_ok_and(|parsed| {
            parsed.scheme() == "https"
                && parsed.host_str().is_some()
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.port().is_none_or(|port| port == 443)
                && parsed.path().to_ascii_lowercase().contains(hash)
        });
        let mime_valid = matches!(
            mime.to_ascii_lowercase().as_str(),
            "image/webp" | "image/png" | "image/apng" | "image/gif"
        );
        let dim_valid = if dim.is_empty() {
            true
        } else {
            dim.split_once('x').is_some_and(|(width, height)| {
                width
                    .parse::<u32>()
                    .is_ok_and(|value| (1..=4096).contains(&value))
                    && height
                        .parse::<u32>()
                        .is_ok_and(|value| (1..=4096).contains(&value))
            })
        };
        shortcode_valid
            && hash_valid
            && url_valid
            && mime_valid
            && dim_valid
            && alt.chars().count() <= 160
            && emoji.is_none_or(|value| value.chars().count() <= 8)
    });
    let compatibility_tags: Vec<_> = tags
        .iter()
        .filter_map(Value::as_array)
        .filter(|parts| parts.first().and_then(Value::as_str) == Some("emoji"))
        .collect();
    let compatibility_valid = compatibility_tags.iter().all(|emoji| {
        emoji.len() == 3
            && sticker_tags
                .iter()
                .filter(|sticker| {
                    sticker.as_array().is_some_and(|sticker| {
                        emoji[1].as_str() == sticker[1].as_str()
                            && emoji[2].as_str() == sticker[2].as_str()
                    })
                })
                .count()
                == 1
            && compatibility_tags
                .iter()
                .filter(|other| other[1] == emoji[1] && other[2] == emoji[2])
                .count()
                == 1
    });
    let category_tags: Vec<_> = tags
        .iter()
        .filter_map(Value::as_array)
        .filter(|parts| parts.first().and_then(Value::as_str) == Some("t"))
        .collect();
    let category_valid = category_tags.is_empty()
        || (category_tags.len() == 1
            && category_tags[0].len() == 2
            && category_tags[0][1].as_str() == Some("sonar-sticker-pack-v1"));

    exact_count("d", identifier) == 1
        && exact_count("pack_format", "sonar-sticker-pack-v1") == 1
        && category_valid
        && title_count == 1
        && (1..=200).contains(&sticker_tags.len())
        && stickers_valid
        && compatibility_valid
}

impl Db {
    /// Atomically mutate the revision-pinned approval table and replace its
    /// relay-signed kind:13536 snapshot.
    ///
    /// Approval acquires the same per-coordinate advisory lock used by
    /// parameterized replacement before checking the live kind:30031 head.
    /// It then acquires the catalog snapshot lock, mutates the approval row,
    /// and publishes the snapshot in the same transaction. A successful row
    /// therefore always names the exact event revision that was current when
    /// approved.
    #[allow(clippy::too_many_arguments)]
    pub async fn mutate_sticker_catalog_locked(
        &self,
        community_id: CommunityId,
        coordinate: &str,
        pack_author: &[u8],
        identifier: &str,
        action: StickerCatalogAction<'_>,
        actor: &[u8],
        relay_keypair: &nostr::Keys,
    ) -> Result<StickerCatalogMutation> {
        if pack_author.len() != 32 {
            return Err(invalid("sticker pack author must be 32 bytes"));
        }
        if actor.len() != 32 {
            return Err(invalid("catalog actor must be 32 bytes"));
        }

        let mut tx = self.pool.begin().await?;

        // Approval must serialize with replacement of this exact pack head.
        if matches!(action, StickerCatalogAction::Approve { .. }) {
            let pack_lock = event_replacement_lock_key(
                community_id,
                KIND_STICKER_PACK as i32,
                pack_author,
                Some(identifier.as_bytes()),
            );
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(pack_lock)
                .execute(&mut *tx)
                .await?;
        }

        let relay_pubkey = relay_keypair.public_key().to_bytes();
        let snapshot_lock = event_replacement_lock_key(
            community_id,
            KIND_STICKER_CATALOG as i32,
            relay_pubkey.as_slice(),
            None,
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(snapshot_lock)
            .execute(&mut *tx)
            .await?;

        let changed = match action {
            StickerCatalogAction::Approve { event_id } => {
                if event_id.len() != 32 {
                    return Err(invalid("approved sticker pack event id must be 32 bytes"));
                }

                let current = sqlx::query(
                    "SELECT id, created_at, tags, content, sig FROM events \
                     WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
                     AND d_tag = $4 AND channel_id IS NULL AND deleted_at IS NULL \
                     ORDER BY created_at DESC, id ASC LIMIT 1",
                )
                .bind(community_id.as_uuid())
                .bind(KIND_STICKER_PACK as i32)
                .bind(pack_author)
                .bind(identifier)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| invalid("sticker pack head not found in this workspace"))?;

                let current_id: Vec<u8> = current.try_get("id")?;
                if current_id.as_slice() != event_id {
                    return Err(invalid(
                        "approved event id is not the current sticker pack head",
                    ));
                }
                let tags: Value = current.try_get("tags")?;
                if !is_structurally_valid_pack(&tags, identifier) {
                    return Err(invalid("current sticker pack head is malformed"));
                }
                let event_created_at: chrono::DateTime<chrono::Utc> =
                    current.try_get("created_at")?;
                let content: String = current.try_get("content")?;
                let signature: Vec<u8> = current.try_get("sig")?;
                let candidate: nostr::Event = serde_json::from_value(serde_json::json!({
                    "id": hex::encode(&current_id),
                    "pubkey": hex::encode(pack_author),
                    "created_at": event_created_at.timestamp(),
                    "kind": KIND_STICKER_PACK,
                    "tags": tags,
                    "content": content,
                    "sig": hex::encode(signature),
                }))?;
                buzz_core::stickers::validate_sticker_pack_event(&candidate).map_err(|error| {
                    invalid(format!("current sticker pack is invalid: {error}"))
                })?;

                sqlx::query(
                    "INSERT INTO sticker_catalog_approvals \
                         (community_id, coordinate, approved_event_id, approved_by) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (community_id, coordinate) DO UPDATE SET \
                         approved_event_id = EXCLUDED.approved_event_id, \
                         approved_by = EXCLUDED.approved_by, \
                         approved_at = now(), updated_at = now() \
                     WHERE sticker_catalog_approvals.approved_event_id \
                         IS DISTINCT FROM EXCLUDED.approved_event_id",
                )
                .bind(community_id.as_uuid())
                .bind(coordinate)
                .bind(event_id)
                .bind(actor)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    > 0
            }
            StickerCatalogAction::Remove => {
                sqlx::query(
                    "DELETE FROM sticker_catalog_approvals \
                 WHERE community_id = $1 AND coordinate = $2",
                )
                .bind(community_id.as_uuid())
                .bind(coordinate)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    > 0
            }
        };

        let approvals = sqlx::query(
            "SELECT coordinate, approved_event_id FROM sticker_catalog_approvals \
             WHERE community_id = $1 ORDER BY coordinate ASC",
        )
        .bind(community_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        let approval_count = approvals.len();
        if approval_count > MAX_STICKER_CATALOG_PACKS {
            return Err(invalid(format!(
                "workspace sticker catalog exceeds {MAX_STICKER_CATALOG_PACKS} packs"
            )));
        }

        let mut tags = Vec::with_capacity(approval_count + 1);
        tags.push(Tag::parse(["-"]).map_err(|error| invalid(format!("build '-' tag: {error}")))?);
        for row in approvals {
            let approved_coordinate: String = row.try_get("coordinate")?;
            let approved_event_id: Vec<u8> = row.try_get("approved_event_id")?;
            let approved_event_hex = hex::encode(approved_event_id);
            tags.push(
                Tag::parse([
                    "a",
                    approved_coordinate.as_str(),
                    approved_event_hex.as_str(),
                ])
                .map_err(|error| invalid(format!("build catalog a tag: {error}")))?,
            );
        }

        // Always advance the relay snapshot timestamp. This avoids an event-ID
        // collision when a coordinate is removed and re-approved within one
        // wall-clock second, which would otherwise roll back the table change.
        let prior_created_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM events \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
             AND channel_id IS NULL AND deleted_at IS NULL \
             ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(KIND_STICKER_CATALOG as i32)
        .bind(relay_pubkey.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        let now = Timestamp::now().as_secs();
        let created_at = prior_created_at
            .map(|value| (value.timestamp() as u64).saturating_add(1))
            .map_or(now, |next| next.max(now));

        let event = EventBuilder::new(Kind::Custom(KIND_STICKER_CATALOG as u16), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(relay_keypair)
            .map_err(|error| invalid(format!("sign kind:{KIND_STICKER_CATALOG}: {error}")))?;
        let received_at = chrono::Utc::now();
        let tags_json = serde_json::to_value(&event.tags)?;
        let event_created_at =
            chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
                .ok_or(DbError::InvalidTimestamp(event.created_at.as_secs() as i64))?;
        let signature = event.sig.serialize();

        sqlx::query(
            "UPDATE events SET deleted_at = NOW() \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
             AND channel_id IS NULL AND deleted_at IS NULL",
        )
        .bind(community_id.as_uuid())
        .bind(KIND_STICKER_CATALOG as i32)
        .bind(relay_pubkey.as_slice())
        .execute(&mut *tx)
        .await?;

        let inserted = sqlx::query(
            "INSERT INTO events \
                 (community_id, id, pubkey, created_at, kind, tags, content, sig, \
                  received_at, channel_id, d_tag) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, NULL) \
             ON CONFLICT DO NOTHING",
        )
        .bind(community_id.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(relay_pubkey.as_slice())
        .bind(event_created_at)
        .bind(KIND_STICKER_CATALOG as i32)
        .bind(&tags_json)
        .bind(&event.content)
        .bind(signature.as_slice())
        .bind(received_at)
        .execute(&mut *tx)
        .await?;
        let was_inserted = inserted.rows_affected() > 0;
        if !was_inserted {
            return Err(invalid("failed to insert unique sticker catalog snapshot"));
        }

        tx.commit().await?;

        Ok(StickerCatalogMutation {
            snapshot: StoredEvent::with_received_at(event, received_at, None, true),
            was_inserted,
            changed,
            approval_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_pack_requires_revision_fields_and_stickers() {
        let tags = serde_json::json!([
            ["d", "animals"],
            ["title", "Animals"],
            ["pack_format", "sonar-sticker-pack-v1"],
            ["t", "sonar-sticker-pack-v1"],
            [
                "sticker",
                "wave",
                "https://cdn.example/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.webp",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "image/webp",
                "256x256",
                "Wave",
                "👋"
            ],
            ["emoji", "wave", "https://cdn.example/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.webp"]
        ]);
        assert!(is_structurally_valid_pack(&tags, "animals"));
        assert!(!is_structurally_valid_pack(&tags, "different"));
    }

    #[test]
    fn structural_pack_rejects_uppercase_hash_and_insecure_url() {
        let tags = serde_json::json!([
            ["d", "animals"],
            ["title", "Animals"],
            ["pack_format", "sonar-sticker-pack-v1"],
            ["t", "sonar-sticker-pack-v1"],
            [
                "sticker",
                "wave",
                "http://cdn.example/file.webp",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "image/webp",
                "256x256",
                "Wave",
                "👋"
            ],
            ["emoji", "wave", "http://cdn.example/file.webp"]
        ]);
        assert!(!is_structurally_valid_pack(&tags, "animals"));
    }

    #[test]
    fn structural_pack_accepts_sticker_without_representative_emoji_field() {
        let hash = "a".repeat(64);
        let url = format!("https://cdn.example/{hash}.webp");
        let tags = serde_json::json!([
            ["d", "animals"],
            ["title", "Animals"],
            ["pack_format", "sonar-sticker-pack-v1"],
            ["t", "sonar-sticker-pack-v1"],
            [
                "sticker",
                "wave",
                url,
                hash,
                "image/webp",
                "256x256",
                "Wave"
            ],
            [
                "emoji",
                "wave",
                format!("https://cdn.example/{}.webp", "a".repeat(64))
            ]
        ]);
        assert!(is_structurally_valid_pack(&tags, "animals"));
    }

    #[test]
    fn structural_pack_accepts_minimal_sticker_without_recommended_tags() {
        let hash = "a".repeat(64);
        let url = format!("https://cdn.example/{hash}.webp");
        let tags = serde_json::json!([
            ["d", "animals"],
            ["title", "Animals"],
            ["pack_format", "sonar-sticker-pack-v1"],
            ["sticker", "wave", url, hash, "image/webp", "256x256"]
        ]);
        assert!(is_structurally_valid_pack(&tags, "animals"));
    }
}
