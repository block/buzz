use crate::{
    acceptance_actor, ActionEngine, ActionError, Actor, ConfirmApprovalDraft, ConfirmWorkItem,
    PgActionStore, PrepareApprovalDraft, PrepareWorkItem, UpdateApprovalDraft, UpdateWorkItem,
    ACCEPTANCE_CLASSIFICATION,
};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use business_action_contracts::{
    ActionReadResult, ApprovalDraftStatus, DismissReasonCode, GetActionProposalInput,
    GetActionRecommendationsInput, GetApprovalDraftInput, GetFindingLifecycleInput,
    GetWorkItemInput, Priority, ResolutionCode, SearchWorkItemsInput, WorkItemStatus,
    BUSINESS_ACTION_READ,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Instant,
};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

pub const AGENT_ACTION_TOOLS: [&str; 6] = [
    "get_finding_lifecycle",
    "get_action_recommendations",
    "get_action_proposal",
    "search_work_items",
    "get_work_item",
    "get_approval_draft",
];

const FINANCE_SESSION: &str = "acceptance-finance-session-32-byte-token";
const SALES_SESSION: &str = "acceptance-sales-session-32-byte-token-01";
const FINANCE_CSRF: &str = "acceptance-finance-csrf-32-byte-token-01";
const SALES_CSRF: &str = "acceptance-sales-csrf-32-byte-token-0001";

#[derive(Clone)]
pub enum AgentVerifier {
    Gateway {
        url: Url,
        credential: String,
        client: reqwest::Client,
    },
    Acceptance,
}

#[derive(Clone)]
pub struct ApiState {
    engine: Arc<Mutex<ActionEngine>>,
    store: Option<PgActionStore>,
    allowed_origin: String,
    service_credential: String,
    verifier: AgentVerifier,
    rate_limit_per_minute: usize,
    write_attempts: Arc<Mutex<BTreeMap<Uuid, VecDeque<Instant>>>>,
}

pub fn acceptance_router(
    engine: ActionEngine,
    allowed_origin: impl Into<String>,
    service_credential: impl Into<String>,
    store: Option<PgActionStore>,
) -> Router {
    router(ApiState {
        engine: Arc::new(Mutex::new(engine)),
        store,
        allowed_origin: allowed_origin.into(),
        service_credential: service_credential.into(),
        verifier: AgentVerifier::Acceptance,
        rate_limit_per_minute: 60,
        write_attempts: Arc::new(Mutex::new(BTreeMap::new())),
    })
}

pub fn acceptance_router_with_gateway(
    engine: ActionEngine,
    allowed_origin: impl Into<String>,
    service_credential: impl Into<String>,
    gateway_base_url: Url,
    rate_limit_per_minute: usize,
    store: PgActionStore,
) -> Result<Router, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "failed to build gateway client")?;
    let credential = service_credential.into();
    Ok(router(ApiState {
        engine: Arc::new(Mutex::new(engine)),
        store: Some(store),
        allowed_origin: allowed_origin.into(),
        service_credential: credential.clone(),
        verifier: AgentVerifier::Gateway {
            url: gateway_base_url,
            credential,
            client,
        },
        rate_limit_per_minute,
        write_attempts: Arc::new(Mutex::new(BTreeMap::new())),
    }))
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/findings", get(list_findings))
        .route("/v1/findings/{id}", get(get_finding))
        .route("/v1/findings/{id}/acknowledge", post(acknowledge))
        .route("/v1/findings/{id}/resolve", post(resolve))
        .route("/v1/findings/{id}/dismiss", post(dismiss))
        .route("/v1/findings/{id}/action-proposals", get(list_proposals))
        .route("/v1/action-proposals/{id}", get(get_proposal))
        .route("/v1/action-proposals/{id}/dismiss", post(dismiss_proposal))
        .route("/v1/work-item-drafts", post(prepare_work_item))
        .route(
            "/v1/work-items",
            post(confirm_work_item).get(list_work_items),
        )
        .route(
            "/v1/work-items/{id}",
            get(get_work_item).patch(update_work_item),
        )
        .route("/v1/approval-draft-previews", post(prepare_approval_draft))
        .route(
            "/v1/approval-drafts",
            post(confirm_approval_draft).get(list_approval_drafts),
        )
        .route(
            "/v1/approval-drafts/{id}",
            get(get_approval_draft).patch(update_approval_draft),
        )
        .route("/v1/agent-read/{tool}", post(agent_read))
        .fallback(blocked_write_fallback)
        .with_state(state)
        .layer(DefaultBodyLimit::max(128 * 1024))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewRequest {
    reason_code: Option<DismissReasonCode>,
    comment: Option<String>,
    review_after: Option<DateTime<Utc>>,
    resolution_code: Option<ResolutionCode>,
    resolution_note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkItemDraftRequest {
    proposal_id: Uuid,
    assignee_user_id: Option<Uuid>,
    assignee_role_key: Option<String>,
    priority: Priority,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfirmWorkItemRequest {
    draft_id: Uuid,
    preview_hash: String,
    expected_finding_version: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateWorkItemRequest {
    expected_version: u64,
    status: WorkItemStatus,
    assignee_user_id: Option<Uuid>,
    assignee_role_key: Option<String>,
    reason_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApprovalPreviewRequest {
    work_item_id: Uuid,
    business_reason: String,
    requested_change_summary: String,
    impact_summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfirmApprovalRequest {
    preview_id: Uuid,
    preview_hash: String,
    expected_work_item_version: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateApprovalRequest {
    expected_version: u64,
    status: ApprovalDraftStatus,
    business_reason: Option<String>,
    requested_change_summary: Option<String>,
    impact_summary: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ListQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn list_findings(
    State(state): State<ApiState>,
    Query(query): Query<ListQuery>,
    request: Request,
) -> Response {
    let Some(actor) = business_actor(request.headers(), &state, false) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let engine = state.engine.lock().await;
    let mut values = engine
        .state
        .findings
        .values()
        .filter(|value| actor.can_read(value))
        .cloned()
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.last_seen_at));
    paged(values, query).into_response()
}

async fn get_finding(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    read_entity(&state, request.headers(), |engine, actor| {
        engine.finding(actor, id).cloned()
    })
    .await
}

async fn acknowledge(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    mutate(&state, &headers, |engine, actor, key| {
        engine.acknowledge(actor, id, key, Utc::now())
    })
    .await
}

async fn resolve(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    if business_actor(&headers, &state, true).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let body = read_json::<ReviewRequest>(request).await;
    let Ok(body) = body else {
        return error_response(ActionError::InvalidRequest);
    };
    let Some(code) = body.resolution_code else {
        return error_response(ActionError::InvalidRequest);
    };
    let Some(note) = body.resolution_note else {
        return error_response(ActionError::InvalidRequest);
    };
    mutate(&state, &headers, |engine, actor, key| {
        engine.resolve(actor, id, key, code, &note, Utc::now())
    })
    .await
}

async fn dismiss(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    if business_actor(&headers, &state, true).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(body) = read_json::<ReviewRequest>(request).await else {
        return error_response(ActionError::InvalidRequest);
    };
    let (Some(code), Some(comment), Some(review_after)) =
        (body.reason_code, body.comment, body.review_after)
    else {
        return error_response(ActionError::InvalidRequest);
    };
    mutate(&state, &headers, |engine, actor, key| {
        engine.dismiss(actor, id, key, code, &comment, review_after, Utc::now())
    })
    .await
}

async fn list_proposals(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    read_entity(&state, request.headers(), |engine, actor| {
        engine
            .proposals(actor, id)
            .map(|values| values.into_iter().cloned().collect::<Vec<_>>())
    })
    .await
}

async fn get_proposal(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    read_entity(&state, request.headers(), |engine, actor| {
        engine.proposal(actor, id).cloned()
    })
    .await
}

async fn dismiss_proposal(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    mutate(&state, &headers, |engine, actor, key| {
        engine.dismiss_proposal(actor, id, key, Utc::now())
    })
    .await
}

async fn prepare_work_item(State(state): State<ApiState>, request: Request) -> Response {
    let headers = request.headers().clone();
    if business_actor(&headers, &state, true).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(body) = read_json::<WorkItemDraftRequest>(request).await else {
        return error_response(ActionError::InvalidRequest);
    };
    mutate_without_key(&state, &headers, |engine, actor| {
        engine.prepare_work_item(
            actor,
            PrepareWorkItem {
                proposal_id: body.proposal_id,
                assignee_user_id: body.assignee_user_id,
                assignee_role_key: body.assignee_role_key,
                priority: body.priority,
                now: Utc::now(),
            },
        )
    })
    .await
}

async fn confirm_work_item(State(state): State<ApiState>, request: Request) -> Response {
    let headers = request.headers().clone();
    if business_actor(&headers, &state, true).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(body) = read_json::<ConfirmWorkItemRequest>(request).await else {
        return error_response(ActionError::InvalidRequest);
    };
    mutate(&state, &headers, |engine, actor, key| {
        engine.confirm_work_item(
            actor,
            ConfirmWorkItem {
                draft_id: body.draft_id,
                preview_hash: body.preview_hash.clone(),
                idempotency_key: key.into(),
                expected_finding_version: body.expected_finding_version,
                now: Utc::now(),
            },
        )
    })
    .await
}

async fn list_work_items(
    State(state): State<ApiState>,
    Query(query): Query<ListQuery>,
    request: Request,
) -> Response {
    let Some(actor) = business_actor(request.headers(), &state, false) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let engine = state.engine.lock().await;
    let values = engine
        .state
        .work_items
        .values()
        .filter(|item| engine.finding(&actor, item.finding_id).is_ok())
        .cloned()
        .collect();
    paged(values, query).into_response()
}

async fn get_work_item(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    read_entity(&state, request.headers(), |engine, actor| {
        let item = engine
            .state
            .work_items
            .get(&id)
            .ok_or(ActionError::NotFoundOrForbidden)?;
        engine.finding(actor, item.finding_id)?;
        Ok(item.clone())
    })
    .await
}

async fn update_work_item(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    if business_actor(&headers, &state, true).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(body) = read_json::<UpdateWorkItemRequest>(request).await else {
        return error_response(ActionError::InvalidRequest);
    };
    mutate_without_key(&state, &headers, |engine, actor| {
        engine.update_work_item(
            actor,
            UpdateWorkItem {
                work_item_id: id,
                expected_version: body.expected_version,
                status: body.status,
                assignee_user_id: body.assignee_user_id,
                assignee_role_key: body.assignee_role_key.clone(),
                reason_code: body.reason_code.clone(),
                now: Utc::now(),
            },
        )
    })
    .await
}

async fn prepare_approval_draft(State(state): State<ApiState>, request: Request) -> Response {
    let headers = request.headers().clone();
    if business_actor(&headers, &state, true).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(body) = read_json::<ApprovalPreviewRequest>(request).await else {
        return error_response(ActionError::InvalidRequest);
    };
    mutate_without_key(&state, &headers, |engine, actor| {
        engine.prepare_approval_draft(
            actor,
            PrepareApprovalDraft {
                work_item_id: body.work_item_id,
                business_reason: body.business_reason.clone(),
                requested_change_summary: body.requested_change_summary.clone(),
                impact_summary: body.impact_summary.clone(),
                now: Utc::now(),
            },
        )
    })
    .await
}

async fn confirm_approval_draft(State(state): State<ApiState>, request: Request) -> Response {
    let headers = request.headers().clone();
    if business_actor(&headers, &state, true).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(body) = read_json::<ConfirmApprovalRequest>(request).await else {
        return error_response(ActionError::InvalidRequest);
    };
    mutate(&state, &headers, |engine, actor, key| {
        engine.confirm_approval_draft(
            actor,
            ConfirmApprovalDraft {
                preview_id: body.preview_id,
                preview_hash: body.preview_hash.clone(),
                idempotency_key: key.into(),
                expected_work_item_version: body.expected_work_item_version,
                now: Utc::now(),
            },
        )
    })
    .await
}

async fn list_approval_drafts(
    State(state): State<ApiState>,
    Query(query): Query<ListQuery>,
    request: Request,
) -> Response {
    let Some(actor) = business_actor(request.headers(), &state, false) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let engine = state.engine.lock().await;
    let values = engine
        .state
        .approval_drafts
        .values()
        .filter(|item| engine.finding(&actor, item.finding_id).is_ok())
        .cloned()
        .collect();
    paged(values, query).into_response()
}

async fn get_approval_draft(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    read_entity(&state, request.headers(), |engine, actor| {
        let item = engine
            .state
            .approval_drafts
            .get(&id)
            .ok_or(ActionError::NotFoundOrForbidden)?;
        engine.finding(actor, item.finding_id)?;
        Ok(item.clone())
    })
    .await
}

async fn update_approval_draft(
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    if business_actor(&headers, &state, true).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(body) = read_json::<UpdateApprovalRequest>(request).await else {
        return error_response(ActionError::InvalidRequest);
    };
    mutate_without_key(&state, &headers, |engine, actor| {
        engine.update_approval_draft(
            actor,
            UpdateApprovalDraft {
                approval_draft_id: id,
                expected_version: body.expected_version,
                status: body.status,
                business_reason: body.business_reason.clone(),
                requested_change_summary: body.requested_change_summary.clone(),
                impact_summary: body.impact_summary.clone(),
                now: Utc::now(),
            },
        )
    })
    .await
}

async fn agent_read(
    State(state): State<ApiState>,
    Path(tool): Path<String>,
    request: Request,
) -> Response {
    if !AGENT_ACTION_TOOLS.contains(&tool.as_str())
        || !service_authorized(request.headers(), &state.service_credential)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(actor) = agent_actor(request.headers(), &state).await else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !actor.can(BUSINESS_ACTION_READ) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), 128 * 1024).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let input: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let engine = state.engine.lock().await;
    let result = agent_tool_result(&engine, &actor, &tool, input);
    match result {
        Ok(items) => Json(ActionReadResult {
            schema_version: 1,
            status: "ok".into(),
            items,
            pagination: None,
            data_classification: ACCEPTANCE_CLASSIFICATION.into(),
            trace_id: actor.trace_id,
        })
        .into_response(),
        Err(error) => error_response(error),
    }
}

fn agent_tool_result(
    engine: &ActionEngine,
    actor: &Actor,
    tool: &str,
    input: Value,
) -> Result<Vec<Value>, ActionError> {
    let one = |value: Value| Ok(vec![value]);
    match tool {
        "get_finding_lifecycle" => {
            let input: GetFindingLifecycleInput = parse(input)?;
            one(json(engine.finding(actor, input.finding_id)?))
        }
        "get_action_recommendations" => {
            let input: GetActionRecommendationsInput = parse(input)?;
            engine
                .proposals(actor, input.finding_id)?
                .into_iter()
                .map(|value| Ok(json(value)))
                .collect()
        }
        "get_action_proposal" => {
            let input: GetActionProposalInput = parse(input)?;
            one(json(engine.proposal(actor, input.proposal_id)?))
        }
        "search_work_items" => {
            let input: SearchWorkItemsInput = parse(input)?;
            let limit = input.limit.unwrap_or(20).clamp(1, 100) as usize;
            engine
                .state
                .work_items
                .values()
                .filter(|item| input.finding_id.is_none_or(|id| item.finding_id == id))
                .filter(|item| {
                    input
                        .statuses
                        .as_ref()
                        .is_none_or(|statuses| statuses.contains(&item.status))
                })
                .filter(|item| {
                    input
                        .action_codes
                        .as_ref()
                        .is_none_or(|codes| codes.contains(&item.action_code))
                })
                .filter(|item| engine.finding(actor, item.finding_id).is_ok())
                .take(limit)
                .map(|value| Ok(json(value)))
                .collect()
        }
        "get_work_item" => {
            let input: GetWorkItemInput = parse(input)?;
            let item = engine
                .state
                .work_items
                .get(&input.work_item_id)
                .ok_or(ActionError::NotFoundOrForbidden)?;
            engine.finding(actor, item.finding_id)?;
            one(json(item))
        }
        "get_approval_draft" => {
            let input: GetApprovalDraftInput = parse(input)?;
            let item = engine
                .state
                .approval_drafts
                .get(&input.approval_draft_id)
                .ok_or(ActionError::NotFoundOrForbidden)?;
            engine.finding(actor, item.finding_id)?;
            one(json(item))
        }
        _ => Err(ActionError::NotFoundOrForbidden),
    }
}

async fn blocked_write_fallback(State(state): State<ApiState>, request: Request<Body>) -> Response {
    const BLOCKED_PATHS: [&str; 7] = [
        "/v1/approve",
        "/v1/reject",
        "/v1/execute",
        "/v1/apply",
        "/v1/commit",
        "/v1/post",
        "/v1/sync-to-erp",
    ];
    if request.method() != Method::POST || !BLOCKED_PATHS.contains(&request.uri().path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let trace_id = trace_id(request.headers());
    let actor = business_actor(request.headers(), &state, true);
    let operation = request.uri().path().to_owned();
    let mut engine = state.engine.lock().await;
    let before = engine.state.clone();
    let error = engine.block_business_write(actor.as_ref(), &operation, Utc::now(), trace_id);
    if let Err(persist_error) = persist(&state, &engine).await {
        engine.state = before;
        return error_response(persist_error);
    }
    error_response(error)
}

async fn mutate<T: serde::Serialize>(
    state: &ApiState,
    headers: &HeaderMap,
    operation: impl FnOnce(&mut ActionEngine, &Actor, &str) -> Result<T, ActionError>,
) -> Response {
    let Some(actor) = business_actor(headers, state, true) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !allow_write_attempt(state, actor.user_id).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"RATE_LIMITED"})),
        )
            .into_response();
    }
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
    else {
        return error_response(ActionError::InvalidRequest);
    };
    let mut engine = state.engine.lock().await;
    let before = engine.state.clone();
    let result = operation(&mut engine, &actor, key);
    if result.is_ok() {
        if let Err(error) = persist(state, &engine).await {
            engine.state = before;
            return error_response(error);
        }
    }
    result
        .map(Json)
        .map(IntoResponse::into_response)
        .unwrap_or_else(error_response)
}

async fn mutate_without_key<T: serde::Serialize>(
    state: &ApiState,
    headers: &HeaderMap,
    operation: impl FnOnce(&mut ActionEngine, &Actor) -> Result<T, ActionError>,
) -> Response {
    let Some(actor) = business_actor(headers, state, true) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !allow_write_attempt(state, actor.user_id).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"RATE_LIMITED"})),
        )
            .into_response();
    }
    let mut engine = state.engine.lock().await;
    let before = engine.state.clone();
    let result = operation(&mut engine, &actor);
    if result.is_ok() {
        if let Err(error) = persist(state, &engine).await {
            engine.state = before;
            return error_response(error);
        }
    }
    result
        .map(Json)
        .map(IntoResponse::into_response)
        .unwrap_or_else(error_response)
}

async fn read_entity<T: serde::Serialize>(
    state: &ApiState,
    headers: &HeaderMap,
    operation: impl FnOnce(&ActionEngine, &Actor) -> Result<T, ActionError>,
) -> Response {
    let Some(actor) = business_actor(headers, state, false) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let engine = state.engine.lock().await;
    operation(&engine, &actor)
        .map(Json)
        .map(IntoResponse::into_response)
        .unwrap_or_else(error_response)
}

async fn persist(state: &ApiState, engine: &ActionEngine) -> Result<(), ActionError> {
    if let Some(store) = &state.store {
        store.save(engine).await?;
    }
    Ok(())
}

async fn allow_write_attempt(state: &ApiState, user_id: Uuid) -> bool {
    let now = Instant::now();
    let mut attempts = state.write_attempts.lock().await;
    let user_attempts = attempts.entry(user_id).or_default();
    while user_attempts
        .front()
        .is_some_and(|attempt| now.duration_since(*attempt).as_secs() >= 60)
    {
        user_attempts.pop_front();
    }
    if user_attempts.len() >= state.rate_limit_per_minute {
        return false;
    }
    user_attempts.push_back(now);
    true
}

fn business_actor(headers: &HeaderMap, state: &ApiState, write: bool) -> Option<Actor> {
    if write {
        let origin = headers.get(header::ORIGIN)?.to_str().ok()?;
        if !constant_eq(origin, &state.allowed_origin) {
            return None;
        }
    }
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    let session = cookie
        .split(';')
        .map(str::trim)
        .find_map(|item| item.strip_prefix("business_session="))?;
    let user_id = if constant_eq(session, FINANCE_SESSION) {
        business_analytics::ACCEPTANCE_FINANCE_USER
    } else if constant_eq(session, SALES_SESSION) {
        business_analytics::ACCEPTANCE_SALES_USER
    } else {
        return None;
    };
    if write {
        let csrf = headers.get("x-csrf-token")?.to_str().ok()?;
        let expected = if user_id == business_analytics::ACCEPTANCE_FINANCE_USER {
            FINANCE_CSRF
        } else {
            SALES_CSRF
        };
        if !constant_eq(csrf, expected) {
            return None;
        }
    }
    let claimed: Uuid = headers
        .get("x-enterprise-user-id")?
        .to_str()
        .ok()?
        .parse()
        .ok()?;
    if claimed != user_id {
        return None;
    }
    acceptance_actor(user_id, trace_id(headers))
}

async fn agent_actor(headers: &HeaderMap, state: &ApiState) -> Option<Actor> {
    let user_id: Uuid = headers
        .get("x-enterprise-user-id")?
        .to_str()
        .ok()?
        .parse()
        .ok()?;
    let actor = acceptance_actor(user_id, trace_id(headers))?;
    match &state.verifier {
        AgentVerifier::Acceptance => {
            let verified = headers
                .get("x-acceptance-delegation-verified")?
                .to_str()
                .ok()?;
            constant_eq(verified, "desensitized-only").then_some(actor)
        }
        AgentVerifier::Gateway {
            url,
            credential,
            client,
        } => {
            let response = client
                .post(url.join("internal/agent-delegations/verify").ok()?)
                .header("x-business-service-credential", credential)
                .header("x-trace-id", actor.trace_id.to_string())
                .json(&json!({
                    "delegationId": headers.get("x-delegation-id")?.to_str().ok()?,
                    "enterpriseUserId": user_id,
                    "identityBindingId": headers.get("x-identity-binding-id")?.to_str().ok()?,
                    "agentId": headers.get("x-agent-id")?.to_str().ok()?,
                    "agentTurnId": headers.get("x-agent-turn-id")?.to_str().ok()?,
                    "traceId": actor.trace_id,
                    "usedCalls": headers.get("x-used-calls")?.to_str().ok()?.parse::<i32>().ok()?,
                    "requiredScope": "business_action:read",
                }))
                .send()
                .await
                .ok()?;
            response.status().is_success().then_some(actor)
        }
    }
}

fn service_authorized(headers: &HeaderMap, credential: &str) -> bool {
    let Some(value) = headers
        .get("x-business-service-credential")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let audience = headers
        .get("x-business-service-audience")
        .and_then(|value| value.to_str().ok());
    constant_eq(value, credential) && audience == Some("business-action-service")
}

fn constant_eq(left: &str, right: &str) -> bool {
    left.len() == right.len() && left.as_bytes().ct_eq(right.as_bytes()).into()
}

fn trace_id(headers: &HeaderMap) -> Uuid {
    headers
        .get("x-trace-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(Uuid::new_v4)
}

async fn read_json<T: for<'de> Deserialize<'de>>(request: Request) -> Result<T, ()> {
    let bytes = axum::body::to_bytes(request.into_body(), 128 * 1024)
        .await
        .map_err(|_| ())?;
    serde_json::from_slice(&bytes).map_err(|_| ())
}

fn parse<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, ActionError> {
    serde_json::from_value(value).map_err(|_| ActionError::InvalidRequest)
}

fn json<T: serde::Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn paged<T: serde::Serialize>(values: Vec<T>, query: ListQuery) -> Json<Value> {
    let total = values.len();
    let offset = query.offset.unwrap_or(0).min(total);
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let items = values
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    Json(json!({
        "items": items,
        "totalCount": total,
        "hasMore": offset + items.len() < total,
        "nextOffset": (offset + items.len() < total).then_some(offset + items.len()),
        "dataClassification": ACCEPTANCE_CLASSIFICATION,
    }))
}

fn error_response(error: ActionError) -> Response {
    let (status, code) = match error {
        ActionError::NotFoundOrForbidden => (StatusCode::NOT_FOUND, "not_found_or_forbidden"),
        ActionError::PermissionDenied => (StatusCode::FORBIDDEN, "permission_denied"),
        ActionError::StalePreview => (StatusCode::CONFLICT, "STALE_PREVIEW"),
        ActionError::PreviewExpired => (StatusCode::GONE, "PREVIEW_EXPIRED"),
        ActionError::PreviewConsumed => (StatusCode::CONFLICT, "PREVIEW_CONSUMED"),
        ActionError::IdempotencyConflict => (StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT"),
        ActionError::VersionConflict => (StatusCode::CONFLICT, "VERSION_CONFLICT"),
        ActionError::BusinessWriteNotAvailable => {
            (StatusCode::NOT_IMPLEMENTED, "BUSINESS_WRITE_NOT_AVAILABLE")
        }
        ActionError::PersistenceUnavailable => {
            (StatusCode::SERVICE_UNAVAILABLE, "PERSISTENCE_UNAVAILABLE")
        }
        ActionError::InvalidTransition => (StatusCode::CONFLICT, "INVALID_TRANSITION"),
        ActionError::AssigneeNotAllowed => (StatusCode::BAD_REQUEST, "ASSIGNEE_NOT_ALLOWED"),
        ActionError::ApprovalDraftNotSupported => {
            (StatusCode::BAD_REQUEST, "APPROVAL_DRAFT_NOT_SUPPORTED")
        }
        ActionError::InvalidRequest | ActionError::InvalidActionCode => {
            (StatusCode::BAD_REQUEST, "INVALID_REQUEST")
        }
    };
    (status, Json(json!({"error":code}))).into_response()
}
