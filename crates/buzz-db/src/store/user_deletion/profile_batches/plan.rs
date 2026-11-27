use super::*;
use sqlx::Row;

pub(super) async fn build(conn: &mut PgConnection, options: &ProfileBatchOptions) -> Result<Plan> {
    let human = normalize_user_deletion_pubkey(&options.pubkey)?;
    let targets = ownership::targets(conn, options.community, &human, false).await?;
    let relays: Vec<String> = options
        .relay_pubkeys
        .iter()
        .map(|p| normalize_user_deletion_pubkey(p))
        .collect::<Result<_>>()?;
    if relays.iter().any(|p| targets.contains(p)) {
        return Err(invalid("target keys and trusted relay keys must differ"));
    }
    let bytes = targets
        .iter()
        .map(|p| hex::decode(p).map_err(|_| invalid("invalid target key")))
        .collect::<Result<_>>()?;
    let scope = Scope {
        community: options.community,
        human,
        targets,
        bytes,
        relays,
    };
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM communities WHERE id=$1 AND deletion_state='active' AND deleted_at IS NULL)")
        .bind(scope.community).fetch_one(&mut *conn).await?;
    if !active {
        return Err(invalid(
            "community is absent or undergoing whole-community deletion",
        ));
    }
    validate_ownership(conn, &scope).await?;
    inspect(conn, scope, options.max_items).await
}

// Only callers with a scope freshly discovered by this invocation may inspect it.
// This never executes mutations; final inspection may include identities we removed.
pub(super) async fn inspect(conn: &mut PgConnection, scope: Scope, max_items: u32) -> Result<Plan> {
    let blockers = dependencies(conn, &scope).await?;
    let mut plan = Plan {
        scope,
        ops: Vec::new(),
        blockers,
    };
    let selected = events::select(conn, &plan.scope, max_items).await?;
    let ids: Vec<Vec<u8>> = selected.iter().map(|e| e.id.clone()).collect();
    // Threaded/channel-bearing data belongs to content cleanup; never break a surviving discussion.
    let threaded: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM thread_metadata WHERE community_id=$1 AND (event_id=ANY($2::bytea[]) OR parent_event_id=ANY($2::bytea[]) OR root_event_id=ANY($2::bytea[])))")
        .bind(plan.scope.community).bind(&ids).fetch_one(&mut *conn).await?;
    if threaded {
        plan.blockers
            .push("selected profiles have thread dependencies; content cleanup required".into());
    }
    for table in rows::CHILDREN {
        rows::collect(conn, &mut plan, table, &ids, max_items).await?;
    }
    let mut snapshots = Vec::new();
    for event in selected {
        let op = rows::Operation {
            table: &rows::EVENTS,
            key: event.key,
            fingerprint: event.fingerprint,
            snapshot: event.snapshot,
        };
        if op.snapshot.is_some() {
            snapshots.push(op);
        } else {
            rows::push(&mut plan, op, max_items)?;
        }
    }
    rows::collect(conn, &mut plan, &rows::HEADS, &[], max_items).await?;
    rows::collect(conn, &mut plan, &rows::ARCHIVES, &[], max_items).await?;
    // Archive rows go first, then snapshots; retries can find stale snapshots while identities remain.
    for op in snapshots {
        rows::push(&mut plan, op, max_items)?;
    }
    let before_users = plan.ops.len();
    rows::collect(conn, &mut plan, &rows::USERS, &[], max_items).await?;
    let human_bytea = format!("\\x{}", plan.scope.human);
    plan.ops[before_users..].sort_by_key(|op| op.key["pubkey"].as_str() == Some(&human_bytea));
    Ok(plan)
}

pub(super) async fn validate_ownership(conn: &mut PgConnection, scope: &Scope) -> Result<()> {
    let conflict: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE community_id=$1 AND \
        ((pubkey=ANY($2::bytea[]) AND pubkey<>decode($3,'hex') AND agent_owner_pubkey IS DISTINCT FROM decode($3,'hex')) \
        OR (agent_owner_pubkey=ANY($2::bytea[]) AND NOT(pubkey=ANY($2::bytea[])))))")
        .bind(scope.community).bind(&scope.bytes).bind(&scope.human).fetch_one(conn).await?;
    if conflict {
        return Err(invalid(
            "agent ownership changed, is missing, or has unselected dependents",
        ));
    }
    Ok(())
}

pub(super) async fn dependencies(conn: &mut PgConnection, scope: &Scope) -> Result<Vec<String>> {
    let mut blockers = Vec::new();
    // Static fragments only. These checks also run immediately before final user removal.
    // Repository announcements/state (30617/30618) survive content and profile cleanup.
    // Keep this exemption narrow: other authored content still requires disposition.
    for (table, selector) in [
        ("api_tokens","owner_pubkey=ANY($2::bytea[])"),
        ("workflows","owner_pubkey=ANY($2::bytea[])"),
        ("channel_members","pubkey=ANY($2::bytea[])"),
        ("relay_members","pubkey=ANY(SELECT encode(k,'hex') FROM unnest($2::bytea[]) k)"),
        ("events","pubkey=ANY($2::bytea[]) AND kind NOT IN (0,10100,30177,30179,9035,9036,30617,30618)"),
        ("archived_identities","NOT(pubkey=ANY(SELECT encode(k,'hex') FROM unnest($2::bytea[]) k)) AND (actor=ANY(SELECT encode(k,'hex') FROM unnest($2::bytea[]) k) OR replaced_by=ANY(SELECT encode(k,'hex') FROM unnest($2::bytea[]) k))"),
    ] {
        let sql=format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE community_id=$1 AND ({selector}))");
        let exists:bool=sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).bind(scope.community).bind(&scope.bytes).fetch_one(&mut *conn).await?;
        if exists {blockers.push(format!("{table}: content/account cleanup or shared-reference disposition required"));}
    }
    // Avoid silently erasing an ownership link through the users self-FK.
    let rows=sqlx::query("SELECT encode(pubkey,'hex') AS pubkey FROM users WHERE community_id=$1 AND agent_owner_pubkey=ANY($2::bytea[]) AND NOT(pubkey=ANY($2::bytea[])) LIMIT 1")
        .bind(scope.community).bind(&scope.bytes).fetch_all(conn).await?;
    if let Some(row) = rows.first() {
        let _: String = row.try_get("pubkey")?;
        blockers.push("unselected agent depends on a target identity".into());
    }
    Ok(blockers)
}
