use super::*;
use chrono::{DateTime, Utc};
use nostr::{Event, EventBuilder, Kind, Tag, Timestamp};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) struct Snapshot {
    signer: String,
    time: u64,
    survivors: Arc<Vec<String>>,
}
pub(super) struct Selected {
    pub id: Vec<u8>,
    pub key: Value,
    pub fingerprint: String,
    pub snapshot: Option<Snapshot>,
}

fn subject(tags: &Value, name: &str) -> Result<String> {
    let all = tags
        .as_array()
        .ok_or_else(|| invalid("profile tags are not an array"))?;
    let tagged: Vec<_> = all
        .iter()
        .filter(|t| t.get(0).and_then(Value::as_str) == Some(name))
        .collect();
    if tagged.len() != 1 {
        return Err(invalid(format!(
            "profile requires exactly one {name} subject"
        )));
    }
    let value = tagged[0]
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing profile subject"))?;
    if value.len() != 64 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid("malformed profile subject"));
    }
    normalize_user_deletion_pubkey(value)
}

pub(super) async fn select(
    conn: &mut PgConnection,
    scope: &Scope,
    max: u32,
) -> Result<Vec<Selected>> {
    // Scope before payload inspection. Case-insensitive subject comparison covers historical hex.
    // Historical replacement references survive unarchive removing the canonical row.
    // The statement budget bounds the fallback tag scan; no new schema is required.
    let candidate_sql="SELECT id,created_at,kind,encode(pubkey,'hex') AS author,channel_id,
        CASE WHEN pg_column_size(tags)<=65536 THEN tags END AS tags,
        jsonb_build_object('id',id,'created_at',created_at) AS key,encode(sha256(convert_to(to_jsonb(e)::text,'UTF8')),'hex') AS fingerprint
        FROM events e WHERE community_id=$1 AND kind IN (0,10100,30177,30179,30382,9035,9036,8002,8003,13535)
        AND (pubkey=ANY($2::bytea[]) OR EXISTS(SELECT 1 FROM jsonb_array_elements(CASE WHEN jsonb_typeof(tags)='array' THEN tags ELSE '[]'::jsonb END) tag
          WHERE (tag->>0 IN ('p','d') AND lower(tag->>1)=ANY($3::text[])) OR (tag->>0='consent' AND lower(tag->>2)=ANY($3::text[]))
          OR (kind IN (9035,9036,8002,8003) AND tag->>0='replaced-by' AND lower(tag->>1)=ANY($3::text[]))))
        ORDER BY created_at,id LIMIT $4";
    // Bound payload memory before materializing candidate tags. The query remains read-only.
    let budget_sql = format!("SELECT count(*) AS count, COALESCE(sum(pg_column_size(tags)),0)::bigint AS bytes FROM ({candidate_sql}) candidates");
    let budget = sqlx::query(sqlx::AssertSqlSafe(budget_sql))
        .bind(scope.community)
        .bind(&scope.bytes)
        .bind(&scope.targets)
        .bind((i64::from(max) + 1).min(65_537))
        .fetch_one(&mut *conn)
        .await?;
    if budget.try_get::<i64, _>("count")? > i64::from(max).min(65_536)
        || budget.try_get::<i64, _>("bytes")? > 32 * 1024 * 1024
    {
        return Err(invalid(
            "profile candidates exceed item or 32 MiB payload budget",
        ));
    }
    let candidates = sqlx::query(candidate_sql)
        .bind(scope.community)
        .bind(&scope.bytes)
        .bind(&scope.targets)
        .bind((i64::from(max) + 1).min(65_537))
        .fetch_all(&mut *conn)
        .await?;
    if candidates.len() > max as usize {
        return Err(invalid(
            "profile candidates exceed max-items; no deletion performed",
        ));
    }
    let survivors:Vec<String>=sqlx::query_scalar("SELECT pubkey FROM archived_identities WHERE community_id=$1 AND NOT(pubkey=ANY($2::text[])) ORDER BY pubkey LIMIT 10001")
        .bind(scope.community).bind(&scope.targets).fetch_all(&mut *conn).await?;
    let survivors = Arc::new(survivors);
    let mut out: Vec<Selected> = Vec::new();
    // Every replacement is strictly newer than every current snapshot from that signer.
    let mut next_time: HashMap<String, u64> = HashMap::new();
    let mut snapshot_index: HashMap<String, usize> = HashMap::new();
    for row in candidates {
        let kind: i32 = row.try_get("kind")?;
        let author: String = row.try_get("author")?;
        let tags: Value = row
            .try_get::<Option<Value>, _>("tags")?
            .ok_or_else(|| invalid("profile tags exceed 64 KiB"))?;
        let authored = scope.targets.contains(&author);
        let mut snapshot = None;
        let selected = match kind {
            0 | 10100 => authored,
            30177 | 30179 => {
                let agent = subject(&tags, "d")?;
                if authored && !scope.targets.contains(&agent) {
                    return Err(invalid(
                        "managed definition names an agent not currently owned in this community",
                    ));
                }
                if scope.targets.contains(&agent) && author != scope.human {
                    return Err(invalid("managed agent definition belongs to another owner"));
                }
                authored && scope.targets.contains(&agent)
            }
            30382 => {
                let target = subject(&tags, "p")?;
                if target != subject(&tags, "d")? {
                    return Err(invalid("verified profile d/p subjects disagree"));
                }
                if scope.targets.contains(&target) {
                    trusted(scope, &author)?;
                    true
                } else {
                    false
                }
            }
            9035 | 9036 => {
                let target = subject(&tags, "p")?;
                if !scope.targets.contains(&target) {
                    return Err(invalid("archive request concerns a surviving identity; shared-history disposition required"));
                }
                scope.targets.contains(&target)
            }
            8002 | 8003 => {
                let target = subject(&tags, "p")?;
                if scope.targets.contains(&target) {
                    trusted(scope, &author)?;
                    true
                } else {
                    return Err(invalid("archive delta references a target actor or replacement on a surviving identity; shared-history disposition required"));
                }
            }
            13535 => {
                trusted(scope, &author)?;
                if survivors.len() > 10_000 {
                    return Err(invalid(
                        "archive snapshot exceeds 10000 retained identities",
                    ));
                }
                let previous = if let Some(previous) = next_time.get(&author) {
                    *previous
                } else {
                    let max_time:Option<DateTime<Utc>>=sqlx::query_scalar("SELECT max(created_at) FROM events WHERE community_id=$1 AND kind=13535 AND pubkey=decode($2,'hex')")
                        .bind(scope.community).bind(&author).fetch_one(&mut *conn).await?;
                    max_time
                        .map_or(0, |t| t.timestamp().max(0) as u64)
                        .max(Timestamp::now().as_secs())
                };
                let time = previous
                    .checked_add(1)
                    .ok_or_else(|| invalid("archive snapshot timestamp overflow"))?;
                next_time.insert(author.clone(), time);
                snapshot = Some(Snapshot {
                    signer: author.clone(),
                    time,
                    survivors: survivors.clone(),
                });
                true
            }
            _ => false,
        };
        if selected {
            if row.try_get::<Option<Uuid>, _>("channel_id")?.is_some() {
                return Err(invalid("channel-bearing profile requires content cleanup"));
            }
            if let Some(new_snapshot) = &snapshot {
                if let Some(previous) =
                    snapshot_index.insert(new_snapshot.signer.clone(), out.len())
                {
                    out[previous].snapshot = None;
                }
            }
            out.push(Selected {
                id: row.try_get("id")?,
                key: row.try_get("key")?,
                fingerprint: row.try_get("fingerprint")?,
                snapshot,
            });
        }
    }
    Ok(out)
}
fn trusted(scope: &Scope, author: &str) -> Result<()> {
    if !scope.relays.iter().any(|r| r == author) {
        return Err(invalid("subject profile/archive event has an untrusted relay signer; supply verified --relay-pubkey"));
    }
    Ok(())
}

pub(super) fn sign_snapshots(
    plan: &Plan,
    signer: Option<&nostr::Keys>,
) -> Result<HashMap<String, Event>> {
    let mut out = HashMap::new();
    let Some(signer) = signer else {
        return Ok(out);
    };
    for op in &plan.ops {
        let Some(snapshot) = &op.snapshot else {
            continue;
        };
        if signer.public_key().to_hex() != snapshot.signer {
            return Err(invalid(
                "relay signing key does not match archive snapshot signer",
            ));
        }
        let mut tags = vec![Tag::parse(["-"]).map_err(|_| invalid("invalid archive marker"))?];
        for p in snapshot.survivors.iter() {
            tags.push(
                Tag::parse(["p", p.as_str()]).map_err(|_| invalid("invalid archive subject"))?,
            );
        }
        let event = EventBuilder::new(Kind::Custom(13535), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(snapshot.time))
            .sign_with_keys(signer)
            .map_err(|_| invalid("could not sign archive snapshot"))?;
        out.insert(op.key.to_string(), event);
        if out.len() > 64 {
            return Err(invalid("too many archive signers"));
        }
    }
    Ok(out)
}

pub(super) async fn replace_snapshot(
    tx: &mut crate::AdmittedTx,
    scope: &Scope,
    op: &rows::Operation,
    replacements: &HashMap<String, Event>,
) -> Result<u64> {
    let event = replacements
        .get(&op.key.to_string())
        .ok_or_else(|| invalid("missing signed archive replacement"))?;
    let current: Vec<String> = sqlx::query_scalar(
        "SELECT pubkey FROM archived_identities WHERE community_id=$1 ORDER BY pubkey LIMIT 10001",
    )
    .bind(scope.community)
    .fetch_all(tx.conn())
    .await?;
    let expected: Vec<String> = event
        .tags
        .iter()
        .filter_map(|t| {
            let t = t.as_slice();
            (t.first().map(String::as_str) == Some("p"))
                .then(|| t.get(1).cloned())
                .flatten()
        })
        .collect();
    if current != expected {
        return Err(invalid(
            "archive membership changed; re-preview before snapshot replacement",
        ));
    }
    // A shared event writer preserves the normal insert hooks and replica rules.
    let removed=sqlx::query("DELETE FROM events e WHERE community_id=$1 AND id=($2->>'id')::bytea AND created_at=($2->>'created_at')::timestamptz AND encode(sha256(convert_to(to_jsonb(e)::text,'UTF8')),'hex')=$3")
        .bind(scope.community).bind(&op.key).bind(&op.fingerprint).execute(tx.conn()).await?.rows_affected();
    if removed != 1 {
        return Ok(removed);
    }
    let (_, inserted) = crate::event::insert_event_in_transaction(tx, event, None).await?;
    if !inserted {
        return Err(invalid("archive replacement already exists; re-preview"));
    }
    Ok(removed)
}
