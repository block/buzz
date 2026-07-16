use axum::http::{header, HeaderMap, Method};

use super::error::ApiError;
use crate::state::AppState;

pub(crate) fn is_admin_host(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(config) = state.config.admin.as_ref() else {
        return false;
    };
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| host == config.host)
}

pub fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    _method: &Method,
) -> Result<String, ApiError> {
    let config = state
        .config
        .admin
        .as_ref()
        .ok_or_else(ApiError::not_found)?;
    if !is_admin_host(state, headers) {
        return Err(ApiError::forbidden());
    }
    let reviewer = headers
        .get(&config.reviewer_header)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::forbidden)?;
    if !config.reviewers.contains(reviewer) {
        return Err(ApiError::forbidden());
    }
    Ok(reviewer.to_owned())
}
