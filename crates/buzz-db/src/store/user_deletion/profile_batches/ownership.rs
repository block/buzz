use super::*;

pub(super) async fn targets(
    conn: &mut PgConnection,
    community: Uuid,
    human: &str,
    lock: bool,
) -> Result<Vec<String>> {
    Ok(super::super::agent_ownership::discover(
        conn,
        human,
        Some(community),
        lock,
        super::super::agent_ownership::DiscoveryLimit::TargetKeys,
    )
    .await?
    .pubkeys())
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
