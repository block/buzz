//! Explicit current-authority query accessory; never infer support from event arrays.

use axum::{http::StatusCode, Json};
use buzz_core::TenantContext;
use buzz_db::audience_authority::MAX_SOURCES;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

use crate::{api::api_error, state::AppState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    kinds: Vec<u32>,
    consistency: String,
    audience_authority: Authority,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    version: u8,
    destination: String,
    sources: Vec<String>,
}

pub(super) fn requested(filters: &[Value]) -> bool {
    filters
        .iter()
        .any(|f| f.get("audience_authority").is_some())
}

fn parse(filters: &[Value]) -> Option<(Uuid, Vec<Uuid>)> {
    let [filter] = filters else { return None };
    let filter: Filter = serde_json::from_value(filter.clone()).ok()?;
    let request = filter.audience_authority;
    if filter.kinds != [39000, 39002]
        || filter.consistency != "strong"
        || request.version != 1
        || request.sources.is_empty()
        || request.sources.len() > MAX_SOURCES
    {
        return None;
    }
    let canonical = |s: &str| Uuid::parse_str(s).ok().filter(|id| id.to_string() == s);
    let destination = canonical(&request.destination)?;
    let sources = request
        .sources
        .iter()
        .map(|s| canonical(s))
        .collect::<Option<Vec<_>>>()?;
    if sources.iter().collect::<HashSet<_>>().len() != sources.len() {
        return None;
    }
    Some((destination, sources))
}

pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    viewer: &nostr::PublicKey,
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (destination, sources) = parse(filters).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid audience_authority version-1 request",
        )
    })?;
    let decisions = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state
            .db
            .audience_authority(tenant.community(), viewer, destination, &sources),
    )
    .await
    .map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "audience_authority unavailable",
        )
    })?
    .map_err(|error| {
        tracing::error!(%error, "audience authority query failed");
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "audience_authority unavailable",
        )
    })?;
    Ok(Json(json!({
        "audience_authority": 1,
        "complete": true,
        "community_id": tenant.community().as_uuid(),
        "pubkey": viewer.to_hex(),
        "destination": destination,
        "sources": decisions,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_bounded_request_only() {
        let destination = Uuid::new_v4();
        let source = Uuid::new_v4();
        let valid = json!({"kinds":[39000,39002],"consistency":"strong",
            "audience_authority":{"version":1,"destination":destination,"sources":[source]}});
        assert!(parse(&[valid.clone()]).is_some());
        assert!(parse(&[valid.clone(), valid.clone()]).is_none());
        for path in ["limit", "authors", "read_state_snapshot"] {
            let mut v = valid.clone();
            v[path] = json!(1);
            assert!(parse(&[v]).is_none());
        }
        for sources in [
            json!([]),
            json!([source, source]),
            json!(["not-a-uuid"]),
            json!([source.to_string().to_uppercase()]),
            json!((0..101).map(|_| Uuid::new_v4()).collect::<Vec<_>>()),
        ] {
            let mut v = valid.clone();
            v["audience_authority"]["sources"] = sources;
            assert!(parse(&[v]).is_none());
        }
        let mut v = valid.clone();
        v["audience_authority"]["version"] = json!(2);
        assert!(parse(&[v]).is_none());
        let mut v = valid;
        v["audience_authority"]["viewer"] = json!("spoofed");
        assert!(parse(&[v]).is_none());
    }
}

#[cfg(test)]
mod postgres_tests;
