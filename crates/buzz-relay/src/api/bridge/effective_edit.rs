//! Explicit, versioned effective-edit lookup; generic #e matching stays exact.
use crate::{
    api::{api_error, db_read_error},
    state::AppState,
};
use axum::{http::StatusCode, Json};
use buzz_core::TenantContext;
use nostr::{EventId, PublicKey};
use serde_json::{json, Value};
use uuid::Uuid;

pub(super) fn requested(filters: &[Value]) -> bool {
    filters.iter().any(|f| f.get("effective_edit").is_some())
}

fn request(filters: &[Value]) -> Result<(Uuid, EventId), &'static str> {
    let filter = filters.first().ok_or("missing effective_edit filter")?;
    let channel = filter["#h"][0]
        .as_str()
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or("invalid channel")?;
    let target = filter["ids"][0]
        .as_str()
        .and_then(|v| EventId::from_hex(v).ok())
        .ok_or("invalid target")?;
    if filters
        != [
            json!({"kinds":buzz_core::media::EDIT_GOVERNED_KINDS, "ids":[target.to_hex()], "#h":[channel.to_string()], "effective_edit":1, "consistency":"strong"}),
        ]
    {
        return Err("effective_edit requires one exact version-1 original-message filter with strong consistency");
    }
    Ok((channel, target))
}

pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    reader: &PublicKey,
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (channel, target) =
        request(filters).map_err(|message| api_error(StatusCode::BAD_REQUEST, message))?;
    let snapshot = state
        .db
        .message_edit_state(
            tenant.community(),
            channel,
            target.as_bytes(),
            reader.as_bytes(),
        )
        .await
        .map_err(|error| db_read_error("effective edit lookup", &error))?
        .filter(|snapshot| {
            crate::handlers::req::event_visible_to_reader(&snapshot.original, reader.as_bytes())
                && snapshot.edit.as_ref().is_none_or(|edit| {
                    crate::handlers::req::event_visible_to_reader(edit, reader.as_bytes())
                })
        })
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "message not found"))?;
    Ok(Json(
        json!({"effective_edit":1, "original":snapshot.original, "edit":snapshot.edit}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_versioned_shape_is_required() {
        let channel = Uuid::new_v4();
        let valid = json!({"kinds":[9,40002,40008,45001,45003],"ids":["a".repeat(64)],"#h":[channel.to_string()],"effective_edit":1,"consistency":"strong"});
        assert!(request(std::slice::from_ref(&valid)).is_ok());
        for (key, value) in [
            ("effective_edit", json!(2)),
            ("consistency", json!("eventual")),
            ("#h", json!([])),
            ("ids", json!([])),
            ("kinds", json!([40003])),
            ("limit", json!(1)),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(request(&[invalid]).is_err(), "{key}");
        }
        assert!(request(&[valid.clone(), valid]).is_err());
    }
}
