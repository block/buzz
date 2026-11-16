//! `POST /query` with `[{"access":"channels","ids":[...]}]`: what the signer
//! may do in each named channel, or, without `ids`, one page of the channels
//! it can reach (NOSTR.md, Channel access).

use axum::{http::StatusCode, Json};
use buzz_core::TenantContext;
use serde_json::Value;
use uuid::Uuid;

use crate::api::api_error;
use crate::handlers::channel_access::{answer, reachable, MAX_IDS};
use crate::state::AppState;

/// How long one answer may take before the request fails and is retried.
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(8);

pub(super) fn requested(filters: &[Value]) -> bool {
    filters.iter().any(|f| f.get("access").is_some())
}

/// What a request asks about.
#[derive(Debug, PartialEq, Eq)]
enum Asked {
    /// The named channels, in request order.
    Named(Vec<Uuid>),
    /// One page of the channels the signer can reach.
    Reachable { after: Option<Uuid>, limit: usize },
}

/// A canonical (lowercase, hyphenated) channel id.
fn channel_id(value: &Value) -> Option<Uuid> {
    let s = value.as_str()?;
    Uuid::parse_str(s).ok().filter(|id| id.to_string() == s)
}

/// Exactly one filter with `access` set to `channels` and either `ids`
/// holding 1 to [`MAX_IDS`] distinct canonical channel ids, or no `ids` and
/// optionally `after` (a canonical channel id) and `limit` (1 to
/// [`MAX_IDS`], default [`MAX_IDS`]).
fn asked(filters: &[Value]) -> Result<Asked, &'static str> {
    const SHAPE: &str = "channel access requires one filter with \
                         {\"access\":\"channels\"} and either \
                         \"ids\":[1 to 100 distinct lowercase channel ids] \
                         or optionally \"after\":<channel id> and \"limit\":<1 to 100>";
    let [Value::Object(filter)] = filters else {
        return Err(SHAPE);
    };
    if filter.get("access").and_then(Value::as_str) != Some("channels") {
        return Err(SHAPE);
    }
    let Some(raw) = filter.get("ids") else {
        if filter
            .keys()
            .any(|key| !matches!(key.as_str(), "access" | "after" | "limit"))
        {
            return Err(SHAPE);
        }
        let after = match filter.get("after") {
            None => None,
            Some(value) => Some(channel_id(value).ok_or(SHAPE)?),
        };
        let limit = match filter.get("limit") {
            None => MAX_IDS,
            Some(value) => value
                .as_u64()
                .filter(|limit| (1..=MAX_IDS as u64).contains(limit))
                .ok_or(SHAPE)? as usize,
        };
        return Ok(Asked::Reachable { after, limit });
    };
    let Value::Array(raw) = raw else {
        return Err(SHAPE);
    };
    if filter.len() != 2 || raw.is_empty() || raw.len() > MAX_IDS {
        return Err(SHAPE);
    }
    let mut ids = Vec::with_capacity(raw.len());
    for value in raw {
        let id = channel_id(value).ok_or(SHAPE)?;
        if ids.contains(&id) {
            return Err(SHAPE);
        }
        ids.push(id);
    }
    Ok(Asked::Named(ids))
}

pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    viewer: &nostr::PublicKey,
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let asked = asked(filters).map_err(|e| api_error(StatusCode::BAD_REQUEST, e))?;
    let (community, pubkey) = (tenant.community(), viewer.to_bytes());
    let answered = tokio::time::timeout(DEADLINE, async {
        match asked {
            Asked::Named(ids) => answer(state, community, &pubkey, &ids)
                .await
                .map(serde_json::to_value),
            Asked::Reachable { after, limit } => reachable(state, community, &pubkey, after, limit)
                .await
                .map(serde_json::to_value),
        }
    })
    .await
    .map_err(|_| "timed out".to_string())
    .and_then(|answered| answered.map_err(|e| e.to_string()));
    let encoded = answered.map_err(|e| {
        tracing::warn!(error = %e, "channel access lookup failed");
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "error: channel access lookup failed; retry",
        )
    })?;
    Ok(Json(encoded.map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "error: channel access encoding failed",
        )
    })?))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const A: &str = "8c1f3d2e-4b5a-4c6d-9e7f-0a1b2c3d4e5f";
    const B: &str = "3f3e3cd7-3b56-81d9-8dd2-8d8b5b1f6d57";

    #[test]
    fn accepts_only_the_exact_shape() {
        assert_eq!(
            asked(&[json!({"access":"channels","ids":[A, B]})]).unwrap(),
            Asked::Named(vec![
                Uuid::parse_str(A).unwrap(),
                Uuid::parse_str(B).unwrap()
            ])
        );
        assert_eq!(
            asked(&[json!({"access":"channels"})]).unwrap(),
            Asked::Reachable {
                after: None,
                limit: MAX_IDS
            }
        );
        assert_eq!(
            asked(&[json!({"access":"channels","after":A,"limit":1})]).unwrap(),
            Asked::Reachable {
                after: Some(Uuid::parse_str(A).unwrap()),
                limit: 1
            }
        );
        let too_many: Vec<String> = (0..=MAX_IDS).map(|_| Uuid::new_v4().to_string()).collect();
        for refused in [
            json!([]),
            json!([{"access":"channels","ids":[A]}, {"access":"channels","ids":[B]}]),
            json!([{"access":"channel","ids":[A]}]),
            json!([{"access":"channels","ids":[A],"kinds":[9]}]),
            json!([{"access":"channels","ids":[]}]),
            json!([{"access":"channels","ids":too_many}]),
            json!([{"access":"channels","ids":[A, A]}]),
            json!([{"access":"channels","ids":[A.to_uppercase()]}]),
            json!([{"access":"channels","ids":[A.replace('-', "")]}]),
            json!([{"access":"channels","ids":[7]}]),
            json!([{"access":"channels","ids":A}]),
            json!([{"access":"channels","kinds":[9]}]),
            json!([{"access":"channels","ids":null}]),
            json!([{"access":"channels"}, {"access":"channels"}]),
            json!([{"access":"channels","limit":0}]),
            json!([{"access":"channels","limit":101}]),
            json!([{"access":"channels","limit":"5"}]),
            json!([{"access":"channels","limit":2.5}]),
            json!([{"access":"channels","after":A.to_uppercase()}]),
            json!([{"access":"channels","after":null}]),
            json!([{"access":"channels","ids":[A],"after":B}]),
            json!([{"access":"channels","ids":[A],"limit":1}]),
        ] {
            let filters: Vec<Value> = serde_json::from_value(refused.clone()).unwrap();
            assert!(asked(&filters).is_err(), "{refused}");
        }
    }

    /// A request the relay cannot answer fails whole, so a client never
    /// takes a left-out channel for "cannot read".
    /// Mutation: skip a channel whose lookup fails instead of failing → RED.
    #[tokio::test]
    async fn a_failed_lookup_fails_the_whole_request() {
        let state = crate::state::tests::test_state_with_database_url_and_pool_options(
            "postgres://nobody@127.0.0.1:1/none",
            sqlx::postgres::PgPoolOptions::new()
                .acquire_timeout(std::time::Duration::from_millis(200)),
        )
        .await;
        let tenant = TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(Uuid::new_v4()),
            "fail.test",
        );
        let viewer = nostr::Keys::generate().public_key();
        let filters = [json!({"access":"channels","ids":[A, B]})];
        let (status, Json(body)) = query(&state, &tenant, &viewer, &filters)
            .await
            .expect_err("no answer without the database");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body["error"].as_str().unwrap().contains("retry"), "{body}");
    }
}
