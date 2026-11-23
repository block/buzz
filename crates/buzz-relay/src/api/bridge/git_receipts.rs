//! Exact repository receipt query; ordinary event filters cannot read audit rows.
use crate::{api::api_error, state::AppState};
use axum::{http::StatusCode, Json};
use buzz_core::{governance::RepoCoordinate, TenantContext};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    git_receipts: u32,
    repo: String,
    #[serde(default)]
    before: Option<i64>,
    #[serde(default = "default_limit")]
    limit: u32,
}
fn default_limit() -> u32 {
    30
}

pub(super) fn requested(filters: &[Value]) -> bool {
    filters
        .iter()
        .any(|filter| filter.get("git_receipts").is_some())
}

pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    actor: &nostr::PublicKey,
    grant: Option<&buzz_core::governance::GitGrant>,
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let invalid = || {
        api_error(StatusCode::BAD_REQUEST, "git_receipts requires one version-1 repo filter, limit 1..200 and a positive before cursor")
    };
    if filters.len() != 1 {
        return Err(invalid());
    }
    let request: Request = serde_json::from_value(filters[0].clone()).map_err(|_| invalid())?;
    if request.git_receipts != 1
        || !(1..=200).contains(&request.limit)
        || request.before.is_some_and(|n| n <= 0)
    {
        return Err(invalid());
    }
    let repo: RepoCoordinate = request.repo.parse().map_err(|_| invalid())?;
    let page = state
        .db
        .git_receipts(
            tenant.community(),
            &repo,
            actor.as_bytes(),
            grant,
            request.before,
            request.limit,
        )
        .await
        .map_err(|error| match error {
            buzz_db::DbError::AccessDenied(_) => {
                api_error(StatusCode::FORBIDDEN, "restricted: repository read denied")
            }
            error => {
                tracing::error!(%error,"Git receipt query failed");
                api_error(StatusCode::SERVICE_UNAVAILABLE, "Git receipts unavailable")
            }
        })?;
    super::governance::record_read(state, tenant, &repo, actor).await?;
    Ok(Json(
        json!({"git_receipts":1,"community_id":tenant.community().as_uuid(),"repo":repo.to_string(),"receipts":page.receipts,"next_before":page.next_before}),
    ))
}
