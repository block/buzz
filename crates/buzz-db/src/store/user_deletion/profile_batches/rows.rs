use super::*;
use sqlx::Row;
use std::collections::HashMap;

pub(super) struct Table {
    pub name: &'static str,
    key: &'static str,
    identity: &'static str,
    pub selector: &'static str,
    pub source_events: &'static str,
}

pub(super) struct Operation {
    pub table: &'static Table,
    pub event_kind: Option<i32>,
    pub subject: Option<usize>,
    pub key: Value,
    pub fingerprint: String,
    pub snapshot: Option<events::Snapshot>,
}
impl Operation {
    pub fn cost(&self) -> usize {
        if self.snapshot.is_some() {
            2
        } else {
            1
        }
    }
    pub fn description(&self) -> Value {
        json!({"table":self.table.name,"key":self.key,
            "action":if self.snapshot.is_some() {"replace_archive_snapshot"} else {"delete"}})
    }
}

pub(super) const EVENTS: Table = Table {
    name: "events",
    key: "jsonb_build_object('id',t.id,'created_at',t.created_at)",
    identity: "id=($2->>'id')::bytea AND created_at=($2->>'created_at')::timestamptz",
    selector: "t.id=ANY($3::bytea[])",
    source_events: "NULL::bytea[]",
};
pub(super) const HEADS: Table = Table {
    name: "nip_fi_profile_heads",
    key: "jsonb_build_object('pubkey',t.pubkey)",
    identity: "pubkey=$2->>'pubkey'",
    selector: "t.pubkey=ANY(SELECT encode(k,'hex') FROM unnest($2::bytea[]) k)",
    source_events: "NULL::bytea[]",
};
pub(super) const ARCHIVES: Table = Table {
    name: "archived_identities",
    key: "jsonb_build_object('pubkey',t.pubkey)",
    identity: "pubkey=$2->>'pubkey'",
    selector: HEADS.selector,
    source_events: "NULL::bytea[]",
};
pub(super) const USERS: Table = Table {
    name: "users",
    // Preserve the bytea key format used by the plan across database output settings.
    key: "jsonb_build_object('pubkey',chr(92)||'x'||encode(t.pubkey,'hex'))",
    identity: "pubkey=($2->>'pubkey')::bytea",
    selector: "t.pubkey=ANY($2::bytea[])",
    source_events: "NULL::bytea[]",
};

pub(super) const CHILDREN: &[Table] = &[
    Table { name:"event_mentions", key:"jsonb_build_object('pubkey_hex',t.pubkey_hex,'event_id',t.event_id)",
        identity:"pubkey_hex=$2->>'pubkey_hex' AND event_id=($2->>'event_id')::bytea", selector:"t.event_id=ANY($3::bytea[])", source_events:"ARRAY[t.event_id]" },
    Table { name:"reactions", key:"jsonb_build_object('event_created_at',t.event_created_at,'event_id',t.event_id,'pubkey',t.pubkey,'emoji',t.emoji)",
        identity:"event_created_at=($2->>'event_created_at')::timestamptz AND event_id=($2->>'event_id')::bytea AND pubkey=($2->>'pubkey')::bytea AND emoji=$2->>'emoji'",
        selector:"t.event_id=ANY($3::bytea[]) OR t.reaction_event_id=ANY($3::bytea[])", source_events:"ARRAY[t.event_id,t.reaction_event_id]" },
    Table { name:"push_match_queue", key:"jsonb_build_object('event_id',t.event_id)", identity:"event_id=($2->>'event_id')::bytea", selector:"t.event_id=ANY($3::bytea[])", source_events:"ARRAY[t.event_id]" },
    Table { name:"push_wake_outbox", key:"jsonb_build_object('id',t.id)", identity:"id=($2->>'id')::uuid", selector:"t.event_id=ANY($3::bytea[])", source_events:"ARRAY[t.event_id]" },
    Table { name:"operator_listener_outbox", key:"jsonb_build_object('id',t.id)", identity:"id=($2->>'id')::uuid", selector:"t.event_id=ANY($3::bytea[])", source_events:"ARRAY[t.event_id]" },
];

pub(super) async fn collect(
    conn: &mut PgConnection,
    plan: &mut Plan,
    table: &'static Table,
    ids: &[Vec<u8>],
    event_subjects: &HashMap<Vec<u8>, Option<usize>>,
    max: u32,
) -> Result<()> {
    let sql = format!("SELECT {} AS key, {} AS source_events, encode(sha256(convert_to(to_jsonb(t)::text,'UTF8')),'hex') AS fingerprint FROM {} t \
        WHERE community_id=$1 AND ({}) AND cardinality($2::bytea[])>=0 AND cardinality($3::bytea[])>=0 \
        ORDER BY {} LIMIT $4", table.key, table.source_events, table.name, table.selector, table.key);
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(plan.scope.community)
        .bind(&plan.scope.bytes)
        .bind(ids)
        .bind((i64::from(max).saturating_sub(plan.ops.len() as i64) + 1).min(65_537))
        .fetch_all(conn)
        .await?;
    for row in rows {
        let key: Value = row.try_get("key")?;
        let source_events: Option<Vec<Option<Vec<u8>>>> = row.try_get("source_events")?;
        let subject = if let Some(source_events) = source_events {
            let mut subjects = source_events
                .iter()
                .flatten()
                .filter_map(|id| event_subjects.get(id));
            let first = subjects.next().copied().flatten();
            first.filter(|first| subjects.all(|subject| *subject == Some(*first)))
        } else {
            key["pubkey"].as_str().and_then(|pubkey| {
                plan.scope
                    .target_index(pubkey.strip_prefix("\\x").unwrap_or(pubkey))
            })
        };
        push(
            plan,
            Operation {
                table,
                event_kind: None,
                subject,
                key,
                fingerprint: row.try_get("fingerprint")?,
                snapshot: None,
            },
            max,
        )?;
    }
    Ok(())
}

pub(super) fn push(plan: &mut Plan, op: Operation, max: u32) -> Result<()> {
    if plan.ops.len() >= 65_536 {
        return Err(invalid(
            "profile plan exceeds 65536-operation memory budget; no deletion performed",
        ));
    }
    if plan.ops.len() >= max as usize {
        return Err(invalid(
            "profile plan exceeds max-items; no deletion performed",
        ));
    }
    // Keys are bounded; the full source payload never enters the plan or output.
    if serde_json::to_vec(&op.key)?.len() > 1024 {
        return Err(invalid("oversized profile row key"));
    }
    plan.ops.push(op);
    Ok(())
}

pub(super) async fn apply(
    tx: &mut crate::AdmittedTx,
    scope: &Scope,
    op: &Operation,
    replacements: &HashMap<String, nostr::Event>,
) -> Result<()> {
    if op.table.name == "users" {
        let blockers = plan::dependencies(tx.conn(), scope).await?;
        if !blockers.is_empty() {
            return Err(invalid(format!(
                "identity removal blocked: {}",
                blockers.join(", ")
            )));
        }
    }
    let changed = if op.snapshot.is_some() {
        events::replace_snapshot(tx, scope, op, replacements).await?
    } else {
        let sql = format!(
            "DELETE FROM {} t WHERE community_id=$1 AND ({}) AND encode(sha256(convert_to(to_jsonb(t)::text,'UTF8')),'hex')=$3",
            op.table.name, op.table.identity
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(scope.community)
            .bind(&op.key)
            .bind(&op.fingerprint)
            .execute(tx.conn())
            .await?
            .rows_affected()
    };
    if changed != 1 {
        return Err(invalid(
            "selected profile row changed; current batch rolled back; re-preview",
        ));
    }
    Ok(())
}
