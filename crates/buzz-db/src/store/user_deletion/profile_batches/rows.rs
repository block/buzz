use super::*;
use sqlx::Row;
use std::collections::HashMap;

pub(super) struct Table {
    pub name: &'static str,
    key: &'static str,
    identity: &'static str,
    pub selector: &'static str,
}

pub(super) struct Operation {
    pub table: &'static Table,
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
};
pub(super) const HEADS: Table = Table {
    name: "nip_fi_profile_heads",
    key: "jsonb_build_object('pubkey',t.pubkey)",
    identity: "pubkey=$2->>'pubkey'",
    selector: "t.pubkey=ANY(SELECT encode(k,'hex') FROM unnest($2::bytea[]) k)",
};
pub(super) const ARCHIVES: Table = Table {
    name: "archived_identities",
    key: "jsonb_build_object('pubkey',t.pubkey)",
    identity: "pubkey=$2->>'pubkey'",
    selector: HEADS.selector,
};
pub(super) const USERS: Table = Table {
    name: "users",
    key: "jsonb_build_object('pubkey',t.pubkey)",
    identity: "pubkey=($2->>'pubkey')::bytea",
    selector: "t.pubkey=ANY($2::bytea[])",
};

pub(super) const CHILDREN: &[Table] = &[
    Table { name:"event_mentions", key:"jsonb_build_object('pubkey_hex',t.pubkey_hex,'event_id',t.event_id)",
        identity:"pubkey_hex=$2->>'pubkey_hex' AND event_id=($2->>'event_id')::bytea", selector:"t.event_id=ANY($3::bytea[])" },
    Table { name:"reactions", key:"jsonb_build_object('event_created_at',t.event_created_at,'event_id',t.event_id,'pubkey',t.pubkey,'emoji',t.emoji)",
        identity:"event_created_at=($2->>'event_created_at')::timestamptz AND event_id=($2->>'event_id')::bytea AND pubkey=($2->>'pubkey')::bytea AND emoji=$2->>'emoji'",
        selector:"t.event_id=ANY($3::bytea[]) OR t.reaction_event_id=ANY($3::bytea[])" },
    Table { name:"push_match_queue", key:"jsonb_build_object('event_id',t.event_id)", identity:"event_id=($2->>'event_id')::bytea", selector:"t.event_id=ANY($3::bytea[])" },
    Table { name:"push_wake_outbox", key:"jsonb_build_object('id',t.id)", identity:"id=($2->>'id')::uuid", selector:"t.event_id=ANY($3::bytea[])" },
    Table { name:"operator_listener_outbox", key:"jsonb_build_object('id',t.id)", identity:"id=($2->>'id')::uuid", selector:"t.event_id=ANY($3::bytea[])" },
    Table { name:"delivery_log", key:"jsonb_build_object('id',t.id,'delivered_at',t.delivered_at)", identity:"id=($2->>'id')::bigint AND delivered_at=($2->>'delivered_at')::timestamptz", selector:"t.event_id=ANY($3::bytea[])" },
];

pub(super) async fn collect(
    conn: &mut PgConnection,
    plan: &mut Plan,
    table: &'static Table,
    ids: &[Vec<u8>],
    max: u32,
) -> Result<()> {
    let sql = format!("SELECT {} AS key, encode(sha256(convert_to(to_jsonb(t)::text,'UTF8')),'hex') AS fingerprint FROM {} t \
        WHERE community_id=$1 AND ({}) AND cardinality($2::bytea[])>=0 AND cardinality($3::bytea[])>=0 \
        ORDER BY {} LIMIT $4", table.key, table.name, table.selector, table.key);
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(plan.scope.community)
        .bind(&plan.scope.bytes)
        .bind(ids)
        .bind((i64::from(max).saturating_sub(plan.ops.len() as i64) + 1).min(65_537))
        .fetch_all(conn)
        .await?;
    for row in rows {
        push(
            plan,
            Operation {
                table,
                key: row.try_get("key")?,
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
