//! Validates CEL expressions when a management request writes them, so a typo
//! fails the request instead of every delivery.
use crate::filter::{check_expression, CompileError, Returns};
use crate::http::error;
use crate::store::{DeliveryMode, IntegrationConfig};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};

/// A subscription's filter, checked to return a bool.
pub(super) async fn refuse_filter(source: Option<String>) -> Option<Response> {
    refuse(
        source
            .map(|s| vec![("filter", s, Returns::Bool)])
            .unwrap_or_default(),
    )
    .await
}

/// An integration's expressions. Caller-selected delivery needs a
/// `channel_expr`, since nothing else names the destination channel.
pub(super) async fn refuse_config(config: &IntegrationConfig) -> Option<Response> {
    if config.delivery_mode == DeliveryMode::CallerSelectsChannel && config.channel_expr.is_none() {
        return Some(invalid_expression(
            "channel_expr",
            "required when delivery_mode is caller_selects_channel",
        ));
    }
    let expressions = config
        .expressions()
        .into_iter()
        .filter_map(|(field, source)| {
            let returns = match field {
                "message_data_expr" => Returns::Any,
                _ => Returns::String,
            };
            Some((field, source?.to_owned(), returns))
        })
        .collect();
    refuse(expressions).await
}

/// Check each expression off the async runtime: the parser runs on a scoped
/// thread with a large stack. `Some` is the response refusing the request.
async fn refuse(expressions: Vec<(&'static str, String, Returns)>) -> Option<Response> {
    if expressions.is_empty() {
        return None;
    }
    let checked = tokio::task::spawn_blocking(move || {
        expressions
            .into_iter()
            .find_map(|(field, source, returns)| {
                check_expression(&source, returns).err().map(|e| (field, e))
            })
    })
    .await;
    match checked {
        Ok(None) => None,
        Ok(Some((_, CompileError::Unavailable(_)))) | Err(_) => Some(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
        )),
        Ok(Some((field, e))) => Some(invalid_expression(field, &e.to_string())),
    }
}

fn invalid_expression(field: &str, message: &str) -> Response {
    let body = serde_json::json!({
        "error": "invalid_expression",
        "field": field,
        "message": message,
    });
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}
