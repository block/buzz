use crate::{
    b2::{
        InventoryCountService, InventoryService, ReturnDispositionService, ReturnService,
        SalesService, SettlementService,
    },
    b3::{
        DeliveryService, PayablesService, PurchasingService, ReceivingService, ReplenishmentService,
    },
    b4::{AdjustmentService, ProfitProjectionService, ProfitReportingService},
    config::Config,
    model::{
        AccessCheckRequest, AccessCheckResponse, ApiErrorBody, BootstrapRequest, CandidateQuery,
        CandidateResponse, MutationResponse, ResourceType, RoleAssignmentRequest,
        ScopeMutationRequest,
    },
    s1::OperationsService,
    security::{valid_key, RequestContext, ServiceAuthenticator},
    store::{PgStore, StoreError},
};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub store: PgStore,
    authenticator: ServiceAuthenticator,
    bootstrap_enabled: bool,
    bootstrap_user_id: Option<Uuid>,
    pub(crate) sales: SalesService,
    pub(crate) inventory: InventoryService,
    pub(crate) inventory_count: InventoryCountService,
    pub(crate) settlement: SettlementService,
    pub(crate) returns: ReturnService,
    pub(crate) return_disposition: ReturnDispositionService,
    pub(crate) purchasing: PurchasingService,
    pub(crate) receiving: ReceivingService,
    pub(crate) payables: PayablesService,
    pub(crate) replenishment: ReplenishmentService,
    pub(crate) delivery: DeliveryService,
    pub(crate) adjustments: AdjustmentService,
    pub(crate) profit_projection: ProfitProjectionService,
    pub(crate) profit_reporting: ProfitReportingService,
    pub(crate) operations: OperationsService,
    pub(crate) master_data: crate::master_data::CoreMasterDataService,
    pub(crate) product_master: crate::product_master::ProductMasterService,
    pub(crate) numbering: crate::numbering::NumberingRuleService,
    pub(crate) business_web_origins: [String; 2],
    pub(crate) business_session_cookie_name: String,
    pub(crate) command_rate_limit_per_minute: u32,
    pub(crate) command_rate: Arc<Mutex<HashMap<(Uuid, i64), u32>>>,
    pub(crate) b2_enabled: [bool; 3],
    pub(crate) b3_enabled: [bool; 3],
    pub(crate) b4_enabled: [bool; 3],
}

impl AppState {
    pub fn new(store: PgStore, config: &Config) -> Self {
        let sales = SalesService::new(
            store.clone(),
            config.sales_order_number_prefix.clone(),
            config.shipment_number_prefix.clone(),
            config.default_payment_terms_days,
        );
        let inventory = InventoryService::new(
            store.clone(),
            config.inventory_opening_number_prefix.clone(),
            config.receivable_number_prefix.clone(),
        );
        let inventory_count =
            InventoryCountService::new(store.clone(), config.inventory_count_number_prefix.clone());
        let settlement =
            SettlementService::new(store.clone(), config.customer_receipt_number_prefix.clone());
        let returns = ReturnService::new(
            store.clone(),
            config.sales_return_number_prefix.clone(),
            config.purchase_return_number_prefix.clone(),
        );
        let return_disposition = ReturnDispositionService::new(store.clone());
        let purchasing = PurchasingService::new(
            store.clone(),
            config.purchase_order_number_prefix.clone(),
            config.default_supplier_payment_terms_days,
        );
        let receiving = ReceivingService::new(
            store.clone(),
            purchasing.clone(),
            config.goods_receipt_number_prefix.clone(),
            config.trade_payable_number_prefix.clone(),
        );
        let payables =
            PayablesService::new(store.clone(), config.supplier_payment_number_prefix.clone());
        let replenishment = ReplenishmentService::new(
            store.clone(),
            config.purchase_requisition_number_prefix.clone(),
        );
        let delivery = DeliveryService::new(store.clone());
        let adjustments = AdjustmentService::new(
            store.clone(),
            config.profit_adjustment_number_prefix.clone(),
            config.profit_allocation_max_targets,
        );
        let profit_projection = ProfitProjectionService::with_retry_limit(
            store.clone(),
            config.profit_projection_retry_limit,
        );
        let profit_reporting = ProfitReportingService::new(
            store.clone(),
            config.management_report_snapshot_number_prefix.clone(),
            config.profit_report_max_rows,
            config.profit_projection_worker_enabled,
            config.profit_data_stale_after_minutes,
        );
        let operations = OperationsService::new(
            store.clone(),
            config.profit_projection_worker_enabled,
            config.profit_data_stale_after_minutes,
        );
        let master_data = crate::master_data::CoreMasterDataService::new(store.clone());
        let product_master = crate::product_master::ProductMasterService::new(store.clone());
        let numbering = crate::numbering::NumberingRuleService::new(store.clone());
        Self {
            store,
            authenticator: ServiceAuthenticator::new(
                &config.service_credential,
                config.service_audience.clone(),
            ),
            bootstrap_enabled: config.bootstrap_enabled,
            bootstrap_user_id: config.bootstrap_user_id,
            sales,
            inventory,
            inventory_count,
            settlement,
            returns,
            return_disposition,
            purchasing,
            receiving,
            payables,
            replenishment,
            delivery,
            adjustments,
            profit_projection,
            profit_reporting,
            operations,
            master_data,
            product_master,
            numbering,
            business_web_origins: [
                config.business_web_origin.clone(),
                config.business_web_embed_origin.clone(),
            ],
            business_session_cookie_name: config.business_session_cookie_name.clone(),
            command_rate_limit_per_minute: config.command_rate_limit_per_minute,
            command_rate: Arc::new(Mutex::new(HashMap::new())),
            b2_enabled: [
                config.sales_enabled,
                config.inventory_enabled,
                config.receivables_enabled,
            ],
            b3_enabled: [
                config.purchasing_enabled,
                config.receiving_enabled,
                config.payables_enabled,
            ],
            b4_enabled: [
                config.profitability_enabled,
                config.management_reporting_enabled,
                config.operational_adjustments_enabled,
            ],
        }
    }
}

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    trace_id: Uuid,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorBody {
                code: self.code,
                message: self.message,
                trace_id: self.trace_id,
            }),
        )
            .into_response()
    }
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "service authentication failed",
            trace_id: Uuid::new_v4(),
        }
    }

    fn from_store(error: StoreError, trace_id: Uuid) -> Self {
        match error {
            StoreError::NotFoundOrForbidden => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found_or_forbidden",
                message: "resource was not found or is not accessible",
                trace_id,
            },
            StoreError::Conflict => Self {
                status: StatusCode::CONFLICT,
                code: "authorization_revision_conflict",
                message: "authorization state changed; refresh and retry",
                trace_id,
            },
            StoreError::Invalid(_) => Self {
                status: StatusCode::BAD_REQUEST,
                code: "invalid_request",
                message: "request validation failed",
                trace_id,
            },
            StoreError::Database(_) | StoreError::Migration(_) | StoreError::Serialization(_) => {
                tracing::error!(%trace_id, error = %error, "Business Core request failed");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: "internal_error",
                    message: "request could not be completed",
                    trace_id,
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

pub fn router(state: AppState) -> Router {
    let state = Arc::new(state);
    let protected = Router::new()
        .route("/v1/group-profile", get(group_profile))
        .route("/v1/master-data/{resource_type}", get(list_master_data))
        .route("/v1/master-data/{resource_type}/{id}", get(get_master_data))
        .route("/v1/authorization/users/{id}/roles", get(user_roles))
        .route("/v1/authorization/users/{id}/scopes", get(user_scopes))
        .route("/v1/authorization/access-check", post(access_check))
        .route("/v1/authorization/assignees/query", post(query_assignees))
        .route("/v1/authorization/approvers/query", post(query_approvers))
        .route("/v1/admin/bootstrap", post(bootstrap))
        .route("/v1/admin/role-assignments", post(mutate_role))
        .route("/v1/admin/scopes", post(mutate_scope))
        .merge(crate::b2::api::service_routes())
        .merge(crate::b3::api::service_routes())
        .merge(crate::b4::api::service_routes())
        .merge(crate::s1::api::service_routes())
        .merge(crate::master_data_api::service_routes())
        .merge(crate::product_master_api::service_routes())
        .merge(crate::numbering_api::service_routes())
        .layer(DefaultBodyLimit::max(256 * 1024))
        .route_layer(middleware::from_fn_with_state(state.clone(), service_auth))
        .with_state(state.clone());
    Router::new()
        .route("/health", get(health))
        .merge(protected)
        .merge(crate::b2::api::browser_routes(state))
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"status":"ok","service":"business-core","stage":"S1"}))
}

async fn group_profile(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = require_permission(&state, &context, "business_master_data:read").await?;
    let profile = state
        .store
        .group_profile()
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    Ok(Json(json!({
        "item": profile,
        "scopeVersion": snapshot.scope_version,
        "effectiveScopeHash": snapshot.effective_scope_hash
    })))
}

async fn service_auth(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let context = state
        .authenticator
        .authenticate(request.headers())
        .ok_or_else(ApiError::unauthorized)?;
    request.extensions_mut().insert(context);
    Ok(next.run(request).await)
}

async fn require_permission(
    state: &AppState,
    context: &RequestContext,
    permission: &str,
) -> Result<crate::model::AuthorizationSnapshot, ApiError> {
    let snapshot = state
        .store
        .snapshot(context.actor_user_id)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    if !snapshot.permission_keys.contains(permission) {
        return Err(ApiError::from_store(
            StoreError::NotFoundOrForbidden,
            context.trace_id,
        ));
    }
    Ok(snapshot)
}

async fn authorize_user_read(
    state: &AppState,
    context: &RequestContext,
    target: Uuid,
) -> Result<(), ApiError> {
    if context.actor_user_id == target {
        return Ok(());
    }
    require_permission(state, context, "business_authorization:read_all")
        .await
        .map(|_| ())
}

pub(crate) async fn list_master_data(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(resource_type): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = require_permission(&state, &context, "business_master_data:read").await?;
    let resource_type = ResourceType::from_str(&resource_type).map_err(|_| {
        ApiError::from_store(StoreError::Invalid("resourceType".into()), context.trace_id)
    })?;
    let records = state
        .store
        .list_resources(resource_type, &snapshot, query.limit)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    Ok(Json(json!({
        "items": records,
        "scopeVersion": snapshot.scope_version,
        "effectiveScopeHash": snapshot.effective_scope_hash
    })))
}

pub(crate) async fn get_master_data(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path((resource_type, id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = require_permission(&state, &context, "business_master_data:read").await?;
    let resource_type = ResourceType::from_str(&resource_type).map_err(|_| {
        ApiError::from_store(StoreError::Invalid("resourceType".into()), context.trace_id)
    })?;
    let record = state
        .store
        .resource(resource_type, id)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    if !snapshot.scopes.permits(&record) {
        return Err(ApiError::from_store(
            StoreError::NotFoundOrForbidden,
            context.trace_id,
        ));
    }
    Ok(Json(json!({
        "item": record,
        "scopeVersion": snapshot.scope_version,
        "effectiveScopeHash": snapshot.effective_scope_hash
    })))
}

async fn user_roles(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize_user_read(&state, &context, id).await?;
    let snapshot = state
        .store
        .snapshot(id)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    Ok(Json(json!({
        "enterpriseUserId": id,
        "roles": snapshot.roles,
        "permissionKeys": snapshot.permission_keys,
        "scopeVersion": snapshot.scope_version,
        "effectiveScopeHash": snapshot.effective_scope_hash
    })))
}

async fn user_scopes(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize_user_read(&state, &context, id).await?;
    let snapshot = state
        .store
        .snapshot(id)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    Ok(Json(json!({
        "enterpriseUserId": id,
        "scopes": snapshot.scopes,
        "scopeVersion": snapshot.scope_version,
        "effectiveScopeHash": snapshot.effective_scope_hash
    })))
}

async fn access_check(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Json(input): Json<AccessCheckRequest>,
) -> Result<Json<AccessCheckResponse>, ApiError> {
    authorize_user_read(&state, &context, input.enterprise_user_id).await?;
    let (allowed, snapshot) = state
        .store
        .can_access(
            input.enterprise_user_id,
            &input.permission_key,
            input.resource_type,
            input.resource_id,
        )
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    Ok(Json(AccessCheckResponse {
        allowed,
        scope_version: snapshot.scope_version,
        effective_scope_hash: snapshot.effective_scope_hash,
    }))
}

async fn query_assignees(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Json(input): Json<CandidateQuery>,
) -> Result<Json<CandidateResponse>, ApiError> {
    let actor_snapshot = require_permission(&state, &context, "business_directory:resolve").await?;
    if !valid_key(&input.action_code, 96) {
        return Err(ApiError::from_store(
            StoreError::Invalid("actionCode".into()),
            context.trace_id,
        ));
    }
    let resource = state
        .store
        .resource(input.resource_type, input.resource_id)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    if !actor_snapshot.scopes.permits(&resource) {
        return Err(ApiError::from_store(
            StoreError::NotFoundOrForbidden,
            context.trace_id,
        ));
    }
    let policy = state
        .store
        .assignment_policy(&input.action_code)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    let candidates = state
        .store
        .eligible_users(
            &resource,
            &policy.required_permission,
            &policy.eligible_role_keys,
            None,
            None,
        )
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    Ok(Json(CandidateResponse {
        action_code: input.action_code,
        candidates,
        minimum_approvers: None,
        scope_version: actor_snapshot.scope_version,
        effective_scope_hash: actor_snapshot.effective_scope_hash,
    }))
}

async fn query_approvers(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Json(input): Json<CandidateQuery>,
) -> Result<Json<CandidateResponse>, ApiError> {
    let actor_snapshot = require_permission(&state, &context, "business_directory:resolve").await?;
    if !valid_key(&input.action_code, 96) || input.amount_minor.is_some_and(|value| value < 0) {
        return Err(ApiError::from_store(
            StoreError::Invalid("candidate query".into()),
            context.trace_id,
        ));
    }
    let resource = state
        .store
        .resource(input.resource_type, input.resource_id)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    if !actor_snapshot.scopes.permits(&resource) {
        return Err(ApiError::from_store(
            StoreError::NotFoundOrForbidden,
            context.trace_id,
        ));
    }
    let policy = state
        .store
        .approval_policy(&input.action_code)
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    let requester = if policy.require_distinct_business_unit {
        let requester_id = input.requester_user_id.ok_or_else(|| {
            ApiError::from_store(
                StoreError::Invalid("requesterUserId is required".into()),
                context.trace_id,
            )
        })?;
        Some(
            state
                .store
                .snapshot(requester_id)
                .await
                .map_err(|error| ApiError::from_store(error, context.trace_id))?,
        )
    } else {
        None
    };
    let excluded = if policy.allow_self_approval {
        None
    } else {
        input.requester_user_id
    };
    let candidates = state
        .store
        .eligible_users(
            &resource,
            &policy.required_permission,
            &policy.eligible_role_keys,
            excluded,
            requester
                .as_ref()
                .map(|snapshot| &snapshot.scopes.business_unit_ids),
        )
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    let minimum_approvers = policy.min_approvers
        + i16::from(
            policy
                .step_up_amount_minor
                .zip(input.amount_minor)
                .is_some_and(|(threshold, amount)| amount >= threshold),
        );
    Ok(Json(CandidateResponse {
        action_code: input.action_code,
        candidates,
        minimum_approvers: Some(minimum_approvers),
        scope_version: actor_snapshot.scope_version,
        effective_scope_hash: actor_snapshot.effective_scope_hash,
    }))
}

async fn bootstrap(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Json(input): Json<BootstrapRequest>,
) -> Result<Json<crate::model::BootstrapResponse>, ApiError> {
    if !state.bootstrap_enabled || state.bootstrap_user_id != Some(context.actor_user_id) {
        return Err(ApiError::from_store(
            StoreError::NotFoundOrForbidden,
            context.trace_id,
        ));
    }
    state
        .store
        .bootstrap(context.actor_user_id, context.trace_id, &input)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_store(error, context.trace_id))
}

async fn mutate_role(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Json(input): Json<RoleAssignmentRequest>,
) -> Result<Json<MutationResponse>, ApiError> {
    let authorization_revision = state
        .store
        .mutate_role(
            context.actor_user_id,
            context.trace_id,
            input.enterprise_user_id,
            input.role_id,
            input.operation,
            input.expected_authorization_revision,
        )
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    Ok(Json(MutationResponse {
        authorization_revision,
    }))
}

async fn mutate_scope(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Json(input): Json<ScopeMutationRequest>,
) -> Result<Json<MutationResponse>, ApiError> {
    let authorization_revision = state
        .store
        .mutate_scope(
            context.actor_user_id,
            context.trace_id,
            input.enterprise_user_id,
            input.dimension,
            input.resource_id,
            input.operation,
            input.expected_authorization_revision,
        )
        .await
        .map_err(|error| ApiError::from_store(error, context.trace_id))?;
    Ok(Json(MutationResponse {
        authorization_revision,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_limit_default_is_bounded() {
        assert_eq!(default_limit(), 100);
    }
}
