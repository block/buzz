//! `{"membership":"removed","since":T}`: the reader's memberships that ended since T.
//!
//! Parsed from the raw filter before `nostr::Filter` discards the extension keys.
use super::{api_error, db_read_error};
use crate::state::AppState;
use axum::{http::StatusCode, Json};
use buzz_core::TenantContext;
use buzz_db::membership_changes::{MembershipChanges, MAX_ROWS};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::sync::Arc;

/// Capability advertised in NIP-11 as `membership`.
pub(crate) fn capability() -> Value {
    json!({"removedSince": true, "maxRows": MAX_ROWS})
}

/// Serve a membership-changes read; `None` leaves the request to the generic
/// path. Any filter naming `membership` claims the whole request.
pub(super) async fn query(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &nostr::PublicKey,
    filters: &[Value],
    count: bool,
) -> Option<Result<Json<Value>, (StatusCode, Json<Value>)>> {
    if !filters.iter().any(|f| f.get("membership").is_some()) {
        return None;
    }
    Some(execute(state, tenant, reader, filters, count).await)
}

fn parse_since(filters: &[Value]) -> Result<DateTime<Utc>, (StatusCode, Json<Value>)> {
    let bad = |msg: &str| api_error(StatusCode::BAD_REQUEST, msg);
    let [filter] = filters else {
        return Err(bad("membership queries require exactly one filter"));
    };
    let object = filter
        .as_object()
        .ok_or_else(|| bad("membership filter must be an object"))?;
    if object.len() != 2 || object.get("membership").and_then(Value::as_str) != Some("removed") {
        return Err(bad(
            "membership filter must be exactly {\"membership\":\"removed\",\"since\":<unix seconds>}",
        ));
    }
    object
        .get("since")
        .and_then(Value::as_i64)
        .filter(|since| *since >= 0)
        .and_then(|since| DateTime::from_timestamp(since, 0))
        .ok_or_else(|| bad("since must be unix seconds"))
}

async fn execute(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &nostr::PublicKey,
    filters: &[Value],
    count: bool,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if count {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "membership requires /query",
        ));
    }
    let since = parse_since(filters)?;
    let result = state
        .db
        .membership_changes_since(tenant.community(), reader.as_bytes(), since)
        .await
        .map_err(|e| db_read_error("membership changes", &e))?;
    let MembershipChanges::Changes { as_of, changes } = result else {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too many membership changes; read memberships in full",
        ));
    };
    let removed: Vec<Value> = changes
        .iter()
        .map(|c| {
            json!({"channelId": c.channel_id, "at": c.at.timestamp(), "reason": c.reason.as_str()})
        })
        .collect();
    Ok(Json(json!({"asOf": as_of.timestamp(), "removed": removed})))
}

#[cfg(test)]
mod tests {
    use super::parse_since;
    use serde_json::json;

    #[test]
    fn accepts_only_the_exact_filter() {
        let ok = parse_since(&[json!({"membership":"removed","since":1_700_000_000})]).unwrap();
        assert_eq!(ok.timestamp(), 1_700_000_000);
        for bad in [
            json!({"membership":"removed"}),
            json!({"membership":"removed","since":"1"}),
            json!({"membership":"removed","since":1.5}),
            json!({"membership":"removed","since":-1}),
            json!({"membership":"removed","since":99_999_999_999_999_999_i64}),
            json!({"membership":"added","since":1}),
            json!({"membership":"removed","since":1,"kinds":[1]}),
            json!({"membership":["removed"],"since":1}),
        ] {
            assert!(parse_since(std::slice::from_ref(&bad)).is_err(), "{bad}");
        }
        assert!(parse_since(&[]).is_err());
        let one = json!({"membership":"removed","since":1});
        assert!(parse_since(&[one.clone(), one]).is_err());
    }
}
