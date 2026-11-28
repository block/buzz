//! Current policy inputs, resolved by the same reader as the governed land.
use crate::{
    api::{api_error, git::gate},
    state::AppState,
};
use axum::{http::StatusCode, Json};
use buzz_core::{
    git_perms::{self, UpdateKind},
    governance::{DocumentId, RepoCoordinate},
    TenantContext,
};
use buzz_db::{git_land, DbError};
use serde_json::{json, Value};

pub(super) fn requested(filters: &[Value]) -> bool {
    filters.iter().any(|f| f.get("git_effective").is_some())
}

pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    actor: &nostr::PublicKey,
    grant: Option<&buzz_core::governance::GitGrant>,
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let invalid = || {
        api_error(
            StatusCode::BAD_REQUEST,
            "git_effective requires one exact version-1 repo filter",
        )
    };
    let repo: RepoCoordinate = filters
        .first()
        .and_then(|f| f.get("repo"))
        .and_then(Value::as_str)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    if filters != [json!({"git_effective":1,"repo":repo.to_string()})] {
        return Err(invalid());
    }
    let effective =
        resolve(state, tenant, actor, grant, &repo)
            .await
            .map_err(|error| match error {
                DbError::AccessDenied(_) => {
                    api_error(StatusCode::FORBIDDEN, "restricted: repository read denied")
                }
                error => {
                    tracing::error!(%error, "effective Git policy unavailable");
                    api_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "effective Git policy unavailable; policy cannot authorize a push",
                    )
                }
            })?;
    super::governance::record_read(state, tenant, &repo, actor).await?;
    Ok(Json(effective))
}

async fn resolve(
    state: &AppState,
    tenant: &TenantContext,
    actor: &nostr::PublicKey,
    grant: Option<&buzz_core::governance::GitGrant>,
    repo: &RepoCoordinate,
) -> buzz_db::Result<Value> {
    let mut tx = state
        .db
        .begin_event_write_transaction(tenant.community())
        .await?;
    git_land::hold(&mut tx, repo)
        .await?
        .ok_or_else(|| DbError::AccessDenied("repository read denied".into()))?;
    buzz_db::governance::authorize_read(
        &mut tx,
        &DocumentId::Rules(repo.clone()),
        actor.as_bytes(),
        grant,
        !state.config.require_relay_membership,
    )
    .await?;
    let channel = buzz_db::governance::repo_channel(&mut tx, repo).await?;
    let policy = gate::read_policy(&mut tx, repo).await?;
    let tags = git_land::announcement_tags(&mut tx, repo)
        .await?
        .ok_or_else(|| DbError::InvalidData("missing announcement".into()))?;
    let legacy =
        git_perms::parse_protection_tags(&tags).map_err(|e| DbError::InvalidData(e.to_string()))?;
    if !legacy.unknown_rules.is_empty() {
        return Err(DbError::InvalidData("unknown legacy protection".into()));
    }
    let rules: Vec<_> = legacy.rules.iter().map(|r| json!({"pattern":r.pattern.to_string(),"push_role":r.push_role.map(|role|role.as_str()),"no_force_push":r.no_force_push,"no_delete":r.no_delete,"require_patch":r.require_patch})).collect();
    let defaults: Vec<_> = [("branches","refs/heads/example"),("tags","refs/tags/example"),("other","refs/example")].into_iter().map(|(namespace, reference)| json!({
        "namespace":namespace,
        "create":git_perms::default_min_role(reference, UpdateKind::Create).as_str(),
        "fast_forward":git_perms::default_min_role(reference, UpdateKind::FastForward).as_str(),
        "non_fast_forward":git_perms::default_min_role(reference, UpdateKind::NonFastForward).as_str(),
        "delete":git_perms::default_min_role(reference, UpdateKind::Delete).as_str()
    })).collect();
    let result = json!({
        "git_effective":1,"community_id":tenant.community().as_uuid(),"repo":repo.to_string(),"channel_id":channel,
        "evaluation":"legacy_and_governance","scope":"policy_inputs_not_push_verdict",
        "floor":{"head":policy.floor.head,"rules":policy.floor.value.0,"label_schema":policy.floor.value.1},
        "trusted":{"head":policy.trusted.head,"checks":policy.trusted.value},
        "labels":{"head":policy.labels.head,"source":if policy.labels.head.is_some(){"repo"}else{"floor_defaults"},"values":policy.labels.value},
        "rules":{"head":policy.rules.head,"rules":policy.rules.value},
        "legacy":{"source":repo.to_string(),"rules":rules,"default_min_roles":defaults,"role_combination":"stricter_of_default_and_matching_rules"}
    });
    tx.commit().await?;
    Ok(result)
}
