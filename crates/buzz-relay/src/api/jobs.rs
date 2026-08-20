//! Authenticated indexed reads for canonical agent-job projections.

use std::sync::Arc;

use axum::{
    extract::{rejection::QueryRejection, Path, Query, RawQuery, State},
    http::{header, HeaderMap, StatusCode},
    response::Json,
};
use buzz_core::TenantContext;
use nostr::PublicKey;
use serde::Deserialize;
use uuid::Uuid;

use crate::handlers::agent_jobs::{
    list_agent_jobs, lookup_agent_job, AgentJobAdmissionError, AgentJobListFilter, AgentJobLookup,
    AgentJobProjection,
};
use crate::state::AppState;

use super::{api_error, bridge, internal_error};

const DEFAULT_JOB_LIST_LIMIT: u16 = 500;
const MAX_JOB_LIST_LIMIT: u16 = 500;
const JOB_STATES: [&str; 8] = buzz_core::agent_job::AGENT_JOB_STATES;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobListQuery {
    agent: Option<String>,
    channel: Option<String>,
    state: Option<String>,
    limit: Option<String>,
}

#[derive(Debug)]
struct ValidatedJobListQuery {
    target: Option<PublicKey>,
    channel: Option<Uuid>,
    state: Option<String>,
    limit: u16,
}

fn path_with_query(path: &str, raw_query: Option<&str>) -> String {
    raw_query
        .filter(|query| !query.is_empty())
        .map_or_else(|| path.to_string(), |query| format!("{path}?{query}"))
}

fn parse_job_id(value: &str) -> Result<Uuid, (StatusCode, Json<serde_json::Value>)> {
    Uuid::parse_str(value).map_err(|_| api_error(StatusCode::BAD_REQUEST, "job id must be a UUID"))
}

fn validate_list_query(
    query: JobListQuery,
) -> Result<ValidatedJobListQuery, (StatusCode, Json<serde_json::Value>)> {
    let target = query
        .agent
        .map(|value| {
            PublicKey::parse(&value).map_err(|_| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    "agent must be a 64-hex pubkey or npub",
                )
            })
        })
        .transpose()?;
    let channel = query
        .channel
        .map(|value| {
            Uuid::parse_str(&value)
                .map_err(|_| api_error(StatusCode::BAD_REQUEST, "channel must be a UUID"))
        })
        .transpose()?;
    let state = query
        .state
        .map(|value| {
            if JOB_STATES.contains(&value.as_str()) {
                Ok(value)
            } else {
                Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "state must be a canonical agent-job state",
                ))
            }
        })
        .transpose()?;
    let limit = query
        .limit
        .map(|value| {
            value
                .parse::<u16>()
                .ok()
                .filter(|value| (1..=MAX_JOB_LIST_LIMIT).contains(value))
                .ok_or_else(|| {
                    api_error(
                        StatusCode::BAD_REQUEST,
                        "limit must be an integer from 1 through 500",
                    )
                })
        })
        .transpose()?
        .unwrap_or(DEFAULT_JOB_LIST_LIMIT);

    Ok(ValidatedJobListQuery {
        target,
        channel,
        state,
        limit,
    })
}

async fn authenticate(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
) -> Result<(TenantContext, PublicKey), (StatusCode, Json<serde_json::Value>)> {
    let raw_host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let (pubkey, event_id) =
        bridge::verify_bridge_auth(headers, "GET", &url, None, state.config.require_auth_token)?;
    bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    bridge::check_nip98_replay(state, &tenant, event_id).await?;
    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        pubkey.as_bytes(),
        auth_tag,
    )
    .await?;
    Ok((tenant, pubkey))
}

fn projection_error(error: AgentJobAdmissionError) -> (StatusCode, Json<serde_json::Value>) {
    internal_error(&format!("indexed agent-job query failed: {error}"))
}

fn participant_can_read(status: &AgentJobProjection, pubkey: &PublicKey) -> bool {
    let pubkey = pubkey.to_hex();
    status.requester_pubkey == pubkey || status.target_pubkey == pubkey
}

/// Return one canonical job projection and its ordered signed event chain.
pub(crate) async fn get_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Path(job_id): Path<String>,
) -> Result<Json<AgentJobLookup>, (StatusCode, Json<serde_json::Value>)> {
    let route_path = format!("/jobs/{job_id}");
    let signed_path = path_with_query(&route_path, raw_query.as_deref());
    let (tenant, pubkey) = authenticate(&state, &headers, &signed_path).await?;
    if raw_query.as_deref().is_some_and(|query| !query.is_empty()) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "job status does not accept query parameters",
        ));
    }
    let job_id = parse_job_id(&job_id)?;
    let lookup = lookup_agent_job(&state.db, tenant.community(), job_id)
        .await
        .map_err(projection_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "job not found"))?;
    let accessible_channels = state
        .get_accessible_channel_ids_cached(tenant.community(), pubkey.as_bytes())
        .await
        .map_err(|error| internal_error(&format!("channel access lookup: {error}")))?;
    if !participant_can_read(&lookup.status, &pubkey)
        || !accessible_channels.contains(&lookup.status.channel_id)
    {
        return Err(api_error(StatusCode::NOT_FOUND, "job not found"));
    }
    Ok(Json(lookup))
}

/// List canonical jobs involving the authenticated participant.
pub(crate) async fn list_jobs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    query: Result<Query<JobListQuery>, QueryRejection>,
) -> Result<Json<Vec<AgentJobProjection>>, (StatusCode, Json<serde_json::Value>)> {
    let path = path_with_query("/jobs", raw_query.as_deref());
    let (tenant, pubkey) = authenticate(&state, &headers, &path).await?;
    let Query(query) = query.map_err(|error| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid job list filters: {error}"),
        )
    })?;
    let query = validate_list_query(query)?;
    let accessible_channels = state
        .get_accessible_channel_ids_cached(tenant.community(), pubkey.as_bytes())
        .await
        .map_err(|error| internal_error(&format!("channel access lookup: {error}")))?;
    if query
        .channel
        .is_some_and(|channel| !accessible_channels.contains(&channel))
    {
        return Ok(Json(Vec::new()));
    }
    let target_bytes = query.target.as_ref().map(|target| target.to_bytes());
    let target = target_bytes.as_ref().map(|bytes| bytes.as_slice());
    let jobs = list_agent_jobs(
        &state.db,
        tenant.community(),
        pubkey.as_bytes(),
        &accessible_channels,
        AgentJobListFilter {
            target_pubkey: target,
            channel_id: query.channel,
            state: query.state.as_deref(),
            limit: query.limit,
        },
    )
    .await
    .map_err(projection_error)?;
    Ok(Json(jobs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_query_rejects_malformed_filters_and_limits() {
        for query in [
            JobListQuery {
                channel: Some("not-a-uuid".into()),
                ..JobListQuery::default()
            },
            JobListQuery {
                state: Some("done".into()),
                ..JobListQuery::default()
            },
            JobListQuery {
                limit: Some("0".into()),
                ..JobListQuery::default()
            },
            JobListQuery {
                limit: Some("501".into()),
                ..JobListQuery::default()
            },
            JobListQuery {
                limit: Some("not-a-number".into()),
                ..JobListQuery::default()
            },
        ] {
            assert!(validate_list_query(query).is_err());
        }
    }

    #[test]
    fn status_route_rejects_malformed_job_uuid() {
        let (status, _) = parse_job_id("not-a-job").unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn participant_scope_accepts_only_requester_or_target() {
        let requester = nostr::Keys::generate().public_key();
        let target = nostr::Keys::generate().public_key();
        let outsider = nostr::Keys::generate().public_key();
        let status = AgentJobProjection {
            job_id: Uuid::nil(),
            request_event_id: "44".repeat(32),
            channel_id: Uuid::nil(),
            requester_pubkey: requester.to_hex(),
            target_pubkey: target.to_hex(),
            state: "requested".into(),
            attempt: 0,
            progress_seq: None,
            summary: "queued".into(),
            cancel_requested: false,
            terminal_event_id: None,
            updated_at: chrono::Utc::now(),
        };
        assert!(participant_can_read(&status, &requester));
        assert!(participant_can_read(&status, &target));
        assert!(!participant_can_read(&status, &outsider));
    }

    #[test]
    fn signed_list_path_includes_exact_query_string() {
        assert_eq!(
            path_with_query("/jobs", Some("state=running&limit=20")),
            "/jobs?state=running&limit=20"
        );
        assert_eq!(path_with_query("/jobs", None), "/jobs");
    }
}
