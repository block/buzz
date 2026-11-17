use super::*;
use std::collections::BTreeSet;

pub(super) async fn targets(
    conn: &mut PgConnection,
    community: Uuid,
    human: &str,
    lock: bool,
) -> Result<Vec<String>> {
    let sql = if lock {
        "SELECT pubkey FROM users WHERE community_id=$1 AND agent_owner_pubkey=decode($2,'hex') ORDER BY pubkey LIMIT 10001 FOR SHARE"
    } else {
        "SELECT pubkey FROM users WHERE community_id=$1 AND agent_owner_pubkey=decode($2,'hex') ORDER BY pubkey LIMIT 10001"
    };
    let agents: Vec<Vec<u8>> = sqlx::query_scalar(sql)
        .bind(community)
        .bind(human)
        .fetch_all(conn)
        .await?;
    let mut targets = BTreeSet::from([human.to_owned()]);
    targets.extend(agents.iter().map(hex::encode));
    if targets.len() > 10_000 {
        return Err(invalid("profile target limit exceeded (10000)"));
    }
    Ok(targets.into_iter().collect())
}

pub(super) async fn recheck(
    conn: &mut PgConnection,
    scope: &Scope,
    remaining: &[String],
) -> Result<()> {
    // Lock original identities too, so transfers away cannot race validation.
    sqlx::query("SELECT pubkey FROM users WHERE community_id=$1 AND pubkey=ANY($2::bytea[]) ORDER BY pubkey FOR SHARE")
        .bind(scope.community).bind(&scope.bytes).fetch_all(&mut *conn).await?;
    if targets(conn, scope.community, &scope.human, true).await? != remaining {
        return Err(invalid(
            "community agent ownership changed; retry to discover current agents",
        ));
    }
    plan::validate_ownership(conn, scope).await
}
