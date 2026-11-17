//! Ownership is a user/agent/community relationship, independent of membership.
use super::*;

pub(super) async fn targets(
    conn: &mut PgConnection,
    community: Uuid,
    subject: &str,
    lock: bool,
) -> Result<Vec<Vec<u8>>> {
    let query = if lock {
        "SELECT pubkey FROM users WHERE community_id=$1 AND agent_owner_pubkey=decode($2,'hex') ORDER BY pubkey LIMIT $3 FOR SHARE"
    } else {
        "SELECT pubkey FROM users WHERE community_id=$1 AND agent_owner_pubkey=decode($2,'hex') ORDER BY pubkey LIMIT $3"
    };
    let mut keys: Vec<Vec<u8>> = sqlx::query_scalar(query)
        .bind(community)
        .bind(subject)
        .bind((MAX_TARGETS + 1) as i64)
        .fetch_all(conn)
        .await?;
    keys.push(hex::decode(subject).map_err(|_| invalid("invalid target key"))?);
    keys.sort();
    keys.dedup();
    if keys.len() > MAX_TARGETS {
        return Err(invalid("content target key limit exceeded (10,000)"));
    }
    Ok(keys)
}

pub(super) async fn excluded(
    conn: &mut PgConnection,
    community: Uuid,
    subject: &str,
) -> Result<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('pubkey',encode(other.pubkey,'hex'),'reason',
          CASE WHEN scoped.agent_owner_pubkey IS NULL THEN 'no ownership evidence in target community'
               ELSE 'different owner in target community' END)
         FROM (SELECT DISTINCT pubkey FROM users WHERE agent_owner_pubkey=decode($2,'hex')) other
         LEFT JOIN users scoped ON scoped.community_id=$1 AND scoped.pubkey=other.pubkey
         WHERE other.pubkey<>decode($2,'hex')
           AND scoped.agent_owner_pubkey IS DISTINCT FROM decode($2,'hex')
         ORDER BY other.pubkey LIMIT $3",
    ).bind(community).bind(subject).bind((MAX_TARGETS + 1) as i64).fetch_all(conn).await?;
    if rows.len() > MAX_TARGETS {
        return Err(invalid("excluded agent key limit exceeded (10,000)"));
    }
    Ok(rows)
}

pub(super) async fn recheck(
    conn: &mut PgConnection,
    options: &ContentBatchOptions,
    start: &Value,
) -> Result<()> {
    let subject = normalize_user_deletion_pubkey(&options.pubkey)?;
    let current: Vec<String> = targets(conn, options.community, &subject, true)
        .await?
        .iter()
        .map(hex::encode)
        .collect();
    if json!(current) != start["target_pubkeys"] {
        return Err(invalid(
            "agent ownership changed after planning; current batch rolled back; rerun",
        ));
    }
    Ok(())
}
