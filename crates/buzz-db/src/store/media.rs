//! Fresh media reads use primary-database event and channel state.
use crate::{Db, DbError, Result};
use buzz_core::{kind, media::reference_hashes, CommunityId};
use buzz_datastore_tracing::datastore_span;
use futures_util::TryStreamExt;
use sqlx::{Postgres, QueryBuilder, Row};

// Unqualified tags lets this exact expression serve indexes and aliased probes.
// Keep DDL/catalog and Rust target parity covered by the PostgreSQL tests.
pub(crate) const MEDIA_EDIT_TARGET_SQL: &str = r#"decode(lower(jsonb_path_query_first(tags, 'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]', '{}'::jsonb, true) #>> '{}'), 'hex')"#;

// Short indexed authorization probes cannot amortize LLVM compilation.
// Partitioned permission plans can compile for seconds before reading any row.
// Ordinary database work retains its configured JIT policy.
pub(super) const MEDIA_LOOKUP_SETTINGS_SQL: &str =
    "SET LOCAL statement_timeout = '2s'; SET LOCAL jit = off";

fn edit_target(alias: &str) -> String {
    MEDIA_EDIT_TARGET_SQL.replacen("(tags,", &format!("({alias}.tags,"), 1)
}

/// Result of a complete live-reference lookup, for bounded denial metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAccess {
    /// A current readable reference.
    Allowed,
    /// Uploaded bytes with no past publication boundary or current reference.
    UploaderPreview,
    /// No published reference or upload proof exists.
    NoReference,
    /// References exist but none is currently readable.
    Restricted,
}

impl Db {
    /// Record possession after the upload pipeline verified the actual bytes.
    /// Independent of optional audit and moderation records; admission fences deletion.
    #[datastore_span(name = "record_media_upload", system = "postgresql")]
    pub async fn record_media_upload(
        &self,
        community: CommunityId,
        sha256: &str,
        pubkey: &[u8],
    ) -> Result<()> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::query("INSERT INTO media_uploaders (community_id, sha256, pubkey) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(community.as_uuid()).bind(sha256).bind(pubkey).execute(tx.conn()).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Record a sticky publication boundary before dispatch, even if the later
    /// command fails. This grants no read access and survives source purging.
    pub async fn record_media_publications(
        &self,
        community: CommunityId,
        hashes: Vec<String>,
    ) -> Result<()> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::query("INSERT INTO media_publications (community_id,sha256) SELECT $1, unnest($2::text[]) ON CONFLICT DO NOTHING")
            .bind(community.as_uuid()).bind(hashes).execute(tx.conn()).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Resolve an attachment through live references, never a membership cache
    /// or read replica. A repeatable-read snapshot covers both indexed references
    /// and profile fallback, including the absence check for uploader previews.
    /// `visible` is the ordinary relay REQ visibility predicate.
    #[allow(clippy::too_many_arguments)] // Complete tenant/media/reader authority stays explicit.
    #[datastore_span(name = "media_access", system = "postgresql")]
    pub async fn media_access(
        &self,
        community: CommunityId,
        sha256: &str,
        ext: &str,
        base: &str,
        pubkey: &[u8],
        for_publication: bool,
        visible: fn(&nostr::Event, &[u8]) -> bool,
    ) -> Result<MediaAccess> {
        let _permit = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            self.media_authorization_slots.acquire(),
        )
        .await
        .map_err(|_| DbError::MediaAuthorizationCapacity)?
        .map_err(|_| DbError::MediaAuthorizationCapacity)?;
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        let result = async {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql(MEDIA_LOOKUP_SETTINGS_SQL)
            .execute(&mut *tx)
            .await?;
        let uploaded: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM media_uploaders WHERE community_id=$1 AND sha256=$2 AND pubkey=$3)")
            .bind(community.as_uuid()).bind(sha256).bind(pubkey).fetch_one(&mut *tx).await?;
        if uploaded && for_publication {
            return Ok(MediaAccess::Allowed);
        }

        let needles = media_reference_needles(sha256, ext, base);
        let mut referenced = false;
        let mut scanned_bytes = 0usize;
        // Prefer readable live references before counting any inaccessible rows.
        // The second pass only classifies absence; it can stop at the first real reference.
        for readable_only in [true, false] {
            for probe in 0..4 {
                let mut query = media_reference_query(
                    community,
                    sha256,
                    pubkey,
                    &needles,
                    probe,
                    readable_only,
                );
                // Stream candidates: a frequently shared hash must not allocate
                // 1,025 full message bodies before finding the first readable one.
                let statement = query.build();
                let mut rows = statement.fetch(&mut *tx);
                let mut count = 0;
                while let Some(row) = rows.try_next().await? {
                    count += 1;
                    let content: &str = row.try_get("content")?;
                    let tags: serde_json::Value = row.try_get("tags")?;
                    scanned_bytes = scanned_bytes
                        .saturating_add(content.len())
                        .saturating_add(tags.to_string().len());
                    if count > 1024 || scanned_bytes > 16 * 1024 * 1024 {
                        return Err(DbError::InvalidData(
                            "media reference work budget exceeded".into(),
                        ));
                    }
                    let readable: bool = row.try_get("readable")?;
                    let stored = crate::event::row_to_stored_event(row)?.ok_or_else(|| {
                        DbError::InvalidData("malformed media reference event".into())
                    })?;
                    let event = &stored.event;
                    if !reference_hashes(event, base).contains(sha256) {
                        continue;
                    }
                    referenced = true;
                    if !readable_only {
                        break;
                    }
                    let kind = event.kind.as_u16() as u32;
                    let tagged = event.tags.iter().any(|tag| matches!(tag.as_slice(), [name, value, ..] if name == "p" && value == &hex::encode(pubkey)));
                    let private_ok = (!kind::P_GATED_KINDS.contains(&kind) || tagged)
                        && (kind != kind::KIND_AGENT_ENGRAM
                            || event.pubkey.as_bytes() == pubkey
                            || tagged);
                    if readable && private_ok && visible(event, pubkey) {
                        return Ok(MediaAccess::Allowed);
                    }
                }
                if referenced && !readable_only {
                    break;
                }
            }
            if referenced && !readable_only {
                break;
            }
        }
        // Community icons are current community state; commands validate their
        // structured URLs before updating this row. They grant community access.
        let icon: Option<String> = sqlx::query_scalar("SELECT icon FROM communities WHERE id=$1")
            .bind(community.as_uuid())
            .fetch_one(&mut *tx)
            .await?;
        if icon
            .as_deref()
            .and_then(|value| buzz_core::media::local_hash(value, base))
            .as_deref()
            == Some(sha256)
        {
            return Ok(MediaAccess::Allowed);
        }
        let published: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM media_publications WHERE community_id=$1 AND sha256=$2)",
        )
        .bind(community.as_uuid())
        .bind(sha256)
        .fetch_one(&mut *tx)
        .await?;
        if referenced || published {
            Ok(MediaAccess::Restricted)
        } else if uploaded {
            Ok(MediaAccess::UploaderPreview)
        } else {
            Ok(MediaAccess::NoReference)
        }
        }.await;
        // Dropping a streaming lookup queues rollback asynchronously. Await it
        // while the permit is held so response completion cannot free capacity
        // while the writer is still draining the query/transaction.
        tx.rollback().await?;
        result
    }
}

pub(super) fn media_reference_query(
    community: CommunityId,
    sha256: &str,
    pubkey: &[u8],
    needles: &[serde_json::Value],
    probe: usize,
    readable_only: bool,
) -> QueryBuilder<Postgres> {
    let mut query = QueryBuilder::<Postgres>::new("");
    query.push("SELECT id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id,");
    query.push(if readable_only { "true" } else { "false" });
    query.push(" AS readable FROM (");
    query.push("SELECT e.id,e.pubkey,e.created_at,e.kind,");
    query.push(if probe == 3 {
        "jsonb_path_query_array(e.tags, 'strict $[*] ? (@[0] == \"link-preview\")') AS tags,"
    } else {
        "e.tags,"
    });
    query.push(
                "e.content,e.sig,e.received_at,e.channel_id, \
                 (e.deleted_at IS NULL AND (e.channel_id IS NULL OR EXISTS ( \
                     SELECT 1 FROM channels c WHERE c.community_id=e.community_id AND c.id=e.channel_id \
                     AND c.deleted_at IS NULL AND (c.visibility='open' OR EXISTS ( \
                         SELECT 1 FROM channel_members m WHERE m.community_id=c.community_id AND m.channel_id=c.id \
                         AND m.removed_at IS NULL AND m.pubkey="
            );
    query
        .push_bind(pubkey)
        .push(
            ")))) AND (e.kind <> 40003 OR EXISTS (SELECT 1 FROM events original \
                WHERE original.community_id=e.community_id AND original.channel_id=e.channel_id \
                AND original.deleted_at IS NULL AND original.kind IN (9,40002) \
                AND ",
        )
        .push(edit_target("e"))
        .push(
            "=original.id AND NOT EXISTS (SELECT 1 FROM events newer \
                    WHERE newer.community_id=e.community_id AND newer.channel_id=e.channel_id \
                    AND newer.kind=40003 AND newer.deleted_at IS NULL \
                    AND (newer.created_at,newer.id) > (e.created_at,e.id) AND ",
        )
        .push(edit_target("newer"))
        .push("=original.id))) ");
    if probe == 3 {
        // The renderer retains snapshots unless the original or any live
        // edit suppresses them; later body edits never undo suppression.
        query.push("AND e.kind IN (9,40002) AND NOT EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) marker(value) WHERE marker.value='[\"link-preview\",\"none\"]'::jsonb) AND EXISTS (SELECT 1 FROM events edit WHERE edit.community_id=e.community_id AND edit.channel_id=e.channel_id AND edit.kind=40003 AND edit.deleted_at IS NULL AND ")
            .push(edit_target("edit")).push("=e.id) AND NOT EXISTS (SELECT 1 FROM events suppressed WHERE suppressed.community_id=e.community_id AND suppressed.channel_id=e.channel_id AND suppressed.kind=40003 AND suppressed.deleted_at IS NULL AND ")
            .push(edit_target("suppressed")).push("=e.id AND EXISTS (SELECT 1 FROM jsonb_array_elements(suppressed.tags) marker(value) WHERE marker.value='[\"link-preview\",\"none\"]'::jsonb))");
    } else {
        query.push("AND (e.kind NOT IN (9,40002) OR NOT EXISTS (SELECT 1 FROM events edit WHERE edit.community_id=e.community_id AND edit.channel_id=e.channel_id AND edit.kind=40003 AND edit.deleted_at IS NULL AND ")
            .push(edit_target("edit")).push("=e.id))");
    }
    query
        .push(") AS readable FROM events e WHERE e.community_id=")
        .push_bind(*community.as_uuid())
        .push(" AND e.deleted_at IS NULL");
    if probe == 2 {
        // Never scan message bodies. Only profiles and personas have content references;
        // current profiles are indexed by community/kind/deleted_at.
        query
            .push(" AND e.kind IN (0,30175) AND e.deleted_at IS NULL AND (strpos(e.content, ")
            .push_bind(sha256)
            .push(") > 0) AND CASE WHEN e.content IS JSON OBJECT THEN ((e.kind=0 AND ((jsonb_typeof(COALESCE(e.content::jsonb->'picture', e.content::jsonb->'image'))='string' AND strpos(COALESCE(e.content::jsonb->'picture',e.content::jsonb->'image') #>> '{}', ")
            .push_bind(sha256)
            .push(") > 0) OR (jsonb_typeof(e.content::jsonb->'banner')='string' AND strpos(e.content::jsonb->>'banner', ")
            .push_bind(sha256)
            .push(") > 0))) OR (e.kind=30175 AND jsonb_typeof(e.content::jsonb->'avatar_url')='string' AND strpos(e.content::jsonb->>'avatar_url', ")
            .push_bind(sha256)
            .push(") > 0)) ELSE false END");
    } else {
        query.push(" AND (");
        let selected = if probe == 0 {
            &needles[..1]
        } else {
            &needles[1..]
        };
        for (i, tag) in ["imeta", "picture", "emoji", "link-preview"]
            .into_iter()
            .enumerate()
        {
            let matching: Vec<serde_json::Value> = selected
                .iter()
                .filter(|needle| {
                    needle[0][0].as_str() == Some(tag) && (probe != 3 || tag == "link-preview")
                })
                .cloned()
                .collect();
            if i > 0 {
                query.push(" OR ");
            }
            query.push("(");
            if tag == "picture" {
                query.push("e.kind=39000 AND ");
            }
            if tag == "emoji" {
                query.push("e.kind IN (10030,30030,30180) AND ");
            }
            let preview_urls = (tag == "link-preview").then(|| {
                matching
                    .iter()
                    .filter_map(|needle| needle[0][1].as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            });
            query
                .push("e.tags @> ANY(")
                .push_bind(matching)
                .push("::jsonb[])");
            if tag == "imeta" && probe == 0 {
                // A bare x marker is not a reference. Require the matching URL
                // in that same tag before charging this row to the work budget.
                let urls: Vec<serde_json::Value> = needles
                    .iter()
                    .filter_map(|needle| needle[0][1].as_str())
                    .filter(|value| value.starts_with("url "))
                    .map(|value| serde_json::json!([value]))
                    .collect();
                query.push(" AND EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) AS imeta(value) WHERE imeta.value @> ")
                    .push_bind(serde_json::json!(["imeta", format!("x {sha256}")]))
                    .push(" AND imeta.value @> ANY(").push_bind(urls).push("::jsonb[]))");
            }
            if let Some(urls) = preview_urls {
                // Containment ignores array positions. Recheck the actual
                // snapshot fields before non-counting rows consume the budget.
                query.push(" AND EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) AS preview(value) \
                    WHERE preview.value->>0='link-preview' AND preview.value->>1='snapshot' \
                    AND preview.value->>2='1' AND CASE WHEN jsonb_typeof(preview.value)='array' \
                    THEN jsonb_array_length(preview.value)=11 ELSE false END \
                    AND (preview.value->>7 = ANY(")
                    .push_bind(urls.clone()).push("::text[]) OR preview.value->>9 = ANY(")
                    .push_bind(urls).push("::text[])))");
            }
            query.push(")");
        }
        query.push(")");
    }
    query.push(") candidate");
    if readable_only {
        query.push(" WHERE readable");
        // Match the ordinary visibility classes before budgeting candidates.
        // The relay predicate still performs its authoritative final check.
        let gated: Vec<i32> = kind::P_GATED_KINDS
            .iter()
            .chain(kind::RESULT_GATED_KINDS)
            .map(|kind| *kind as i32)
            .collect();
        let author_only: Vec<i32> = kind::AUTHOR_ONLY_KINDS
            .iter()
            .map(|kind| *kind as i32)
            .collect();
        let shared: Vec<i32> = kind::SHARED_GATED_KINDS
            .iter()
            .map(|kind| *kind as i32)
            .collect();
        query
            .push(" AND (kind <> ALL(")
            .push_bind(gated)
            .push(") OR tags @> ")
            .push_bind(serde_json::json!([["p", hex::encode(pubkey)]]))
            .push(")");
        query
            .push(" AND (kind <> ALL(")
            .push_bind(author_only)
            .push(") OR pubkey=")
            .push_bind(pubkey)
            .push(")");
        query
            .push(" AND (kind <> ")
            .push_bind(kind::KIND_AGENT_ENGRAM as i32)
            .push(" OR pubkey=")
            .push_bind(pubkey)
            .push(" OR tags @> ")
            .push_bind(serde_json::json!([["p", hex::encode(pubkey)]]))
            .push(")");
        query.push(" AND (kind <> ALL(").push_bind(shared).push(") OR pubkey=").push_bind(pubkey)
            .push(" OR jsonb_path_query_array(tags, 'strict $[*] ? (@[0] == \"shared\")') = '[[\"shared\",\"true\"]]'::jsonb)");
    }
    if probe == 2 {
        query
            .push(" ORDER BY (strpos(content, ")
            .push_bind(sha256)
            .push(") > 0) DESC, created_at ASC, id ASC");
    }
    query.push(" LIMIT 1025");
    query
}

pub(super) fn media_reference_needles(
    sha256: &str,
    ext: &str,
    base: &str,
) -> Vec<serde_json::Value> {
    let mut needles = vec![serde_json::json!([["imeta", format!("x {sha256}")]])];
    let mut bases = vec!["/media".to_owned(), base.trim_end_matches('/').to_owned()];
    // Both schemes have historically appeared in tenant descriptor URLs.
    if let Some(rest) = base.strip_prefix("https://") {
        bases.push(format!("http://{rest}"));
    }
    if let Some(rest) = base.strip_prefix("http://") {
        bases.push(format!("https://{rest}"));
    }
    for prefix in bases {
        for suffix in [
            String::new(),
            format!(".{ext}"),
            ".thumb.jpg".into(),
            ".thumb.webp".into(),
        ] {
            let url = format!("{prefix}/{sha256}{suffix}");
            for field in ["url", "image"] {
                needles.push(serde_json::json!([["imeta", format!("{field} {url}")]]));
            }
            needles.push(serde_json::json!([["link-preview", url]]));
            needles.push(serde_json::json!([["picture", url]]));
            needles.push(serde_json::json!([["emoji", url]]));
        }
    }
    needles
}
