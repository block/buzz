#![forbid(unsafe_code)]

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;
use uuid::Uuid;

pub const SCHEMA_VERSION: u8 = 1;
pub const EXECUTION_DISABLED_MESSAGE: &str = "Business execution is not available in V6.5.";
pub const EXECUTION_NOT_ENABLED_CODE: &str = "BUSINESS_EXECUTION_NOT_ENABLED";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BusinessEnvironment {
    Production,
    Staging,
    Sandbox,
    Acceptance,
    DesensitizedAcceptance,
    Mock,
}

impl BusinessEnvironment {
    pub fn is_real_structure_non_production(self) -> bool {
        matches!(self, Self::Staging | Self::Sandbox)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAuthMode {
    Mtls,
    WorkloadIdentity,
    ServiceJwt,
}

#[derive(Debug, Clone)]
pub struct V65RuntimeConfig {
    pub environment: BusinessEnvironment,
    pub business_system_origin: Url,
    pub read_api_base_url: Url,
    pub capability_api_base_url: Url,
    pub permission_api_base_url: Url,
    pub directory_api_base_url: Url,
    pub service_auth_mode: ServiceAuthMode,
    pub service_audience: String,
    pub write_readiness_enabled: bool,
    pub staging_reset_supported: bool,
    pub staging_snapshot_supported: bool,
}

impl V65RuntimeConfig {
    pub fn from_values(values: &BTreeMap<String, String>) -> Result<Self, String> {
        let required = |name: &str| {
            values
                .get(name)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{name} is required"))
        };
        ensure_v65_execution_disabled(values.get("BUSINESS_EXECUTION_ENABLED").map(String::as_str))
            .map_err(str::to_owned)?;
        if !parse_bool(values, "BUSINESS_EXECUTION_KILL_SWITCH", true)? {
            return Err("BUSINESS_EXECUTION_KILL_SWITCH=true is required in V6.5".into());
        }
        for name in [
            "BUSINESS_EXECUTION_ALLOWED_ACTIONS",
            "BUSINESS_EXECUTION_ALLOWED_USERS",
        ] {
            if values
                .get(name)
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err(format!("{name} must be empty in V6.5"));
            }
        }
        let environment = match required("BUSINESS_ENVIRONMENT")? {
            "staging" => BusinessEnvironment::Staging,
            "sandbox" => BusinessEnvironment::Sandbox,
            "production" => BusinessEnvironment::Production,
            "acceptance" => BusinessEnvironment::Acceptance,
            "desensitized_acceptance" => BusinessEnvironment::DesensitizedAcceptance,
            "mock" => BusinessEnvironment::Mock,
            _ => return Err("BUSINESS_ENVIRONMENT is not recognized".into()),
        };
        if !environment.is_real_structure_non_production() {
            return Err("V6.5 requires a real-structure Staging or Sandbox environment".into());
        }
        let service_auth_mode =
            match required("BUSINESS_SERVICE_AUTH_MODE")? {
                "mtls" => ServiceAuthMode::Mtls,
                "workload_identity" => ServiceAuthMode::WorkloadIdentity,
                "service_jwt" => ServiceAuthMode::ServiceJwt,
                _ => return Err(
                    "BUSINESS_SERVICE_AUTH_MODE must use mTLS, workload identity, or service JWT"
                        .into(),
                ),
            };
        let service_audience = required("BUSINESS_SERVICE_AUDIENCE")?.to_owned();
        if service_audience.len() > 128 {
            return Err("BUSINESS_SERVICE_AUDIENCE is too long".into());
        }
        Ok(Self {
            environment,
            business_system_origin: exact_https_origin(
                "BUSINESS_SYSTEM_ORIGIN",
                required("BUSINESS_SYSTEM_ORIGIN")?,
            )?,
            read_api_base_url: https_base_url(
                "BUSINESS_READ_API_BASE_URL",
                required("BUSINESS_READ_API_BASE_URL")?,
            )?,
            capability_api_base_url: https_base_url(
                "BUSINESS_CAPABILITY_API_BASE_URL",
                required("BUSINESS_CAPABILITY_API_BASE_URL")?,
            )?,
            permission_api_base_url: https_base_url(
                "BUSINESS_PERMISSION_API_BASE_URL",
                required("BUSINESS_PERMISSION_API_BASE_URL")?,
            )?,
            directory_api_base_url: https_base_url(
                "BUSINESS_DIRECTORY_API_BASE_URL",
                required("BUSINESS_DIRECTORY_API_BASE_URL")?,
            )?,
            service_auth_mode,
            service_audience,
            write_readiness_enabled: parse_bool(values, "BUSINESS_WRITE_READINESS_ENABLED", false)?,
            staging_reset_supported: parse_bool(values, "BUSINESS_STAGING_RESET_SUPPORTED", false)?,
            staging_snapshot_supported: parse_bool(
                values,
                "BUSINESS_STAGING_SNAPSHOT_SUPPORTED",
                false,
            )?,
        })
    }
}

fn parse_bool(
    values: &BTreeMap<String, String>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match values
        .get(name)
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("true" | "1" | "yes" | "on") => Ok(true),
        Some("false" | "0" | "no" | "off") => Ok(false),
        None => Ok(default),
        Some(_) => Err(format!("{name} must be a boolean")),
    }
}

fn https_base_url(name: &str, value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| format!("{name} must be a URL"))?;
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(format!(
            "{name} must be an HTTPS URL without credentials or query data"
        ));
    }
    Ok(url)
}

fn exact_https_origin(name: &str, value: &str) -> Result<Url, String> {
    let url = https_base_url(name, value)?;
    if url.path() != "/" {
        return Err(format!("{name} must contain only an origin"));
    }
    Ok(url)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    Supported,
    Partial,
    Unsupported,
}

/// A wire marker that can only be `false`. Deserializing `true` is rejected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ExecutionUnavailable(bool);

impl ExecutionUnavailable {
    pub const fn new() -> Self {
        Self(false)
    }

    pub const fn get(self) -> bool {
        false
    }
}

impl<'de> Deserialize<'de> for ExecutionUnavailable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = bool::deserialize(deserializer)?;
        if value {
            return Err(serde::de::Error::custom(
                "executionAvailable must be false in V6.5",
            ));
        }
        Ok(Self::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessWriteCapability {
    pub capability_id: String,
    pub action_code: String,
    pub title: String,
    pub resource_type: String,
    pub risk_level: RiskLevel,
    pub reversible: bool,
    pub compensating_action_code: Option<String>,
    pub supports_dry_run: bool,
    pub supports_expected_version: bool,
    pub supports_idempotency: bool,
    pub supports_postcondition_readback: bool,
    pub required_permissions: Vec<String>,
    pub required_approver_roles: Vec<String>,
    pub minimum_approver_count: u8,
    pub step_up_required: bool,
    pub staging_supported: bool,
    pub production_supported: bool,
    pub api_contract_version: String,
    pub enabled: bool,
}

impl BusinessWriteCapability {
    pub fn v7_pilot_eligible(&self) -> bool {
        self.enabled
            && matches!(self.risk_level, RiskLevel::Low | RiskLevel::Medium)
            && self.reversible
            && self.compensating_action_code.is_some()
            && self.supports_dry_run
            && self.supports_expected_version
            && self.supports_idempotency
            && self.supports_postcondition_readback
            && self.staging_supported
            && self.minimum_approver_count > 0
            && !self.required_permissions.is_empty()
            && !self.required_approver_roles.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessActionPreflight {
    pub schema_version: u8,
    pub action_code: String,
    pub capability_id: String,
    pub capability_version: String,
    pub resource_type: String,
    pub resource_id: String,
    pub current_version: String,
    pub current_state_hash: String,
    pub proposal_hash: String,
    pub approval_draft_hash: String,
    pub permission_version: String,
    pub approval_policy_version: String,
    pub proposed_change_summary: BTreeMap<String, Value>,
    pub risk_level: RiskLevel,
    pub reversible: bool,
    pub compensating_action_code: Option<String>,
    pub required_permission_keys: Vec<String>,
    pub required_approver_role_keys: Vec<String>,
    pub minimum_approver_count: u8,
    pub separation_of_duties_required: bool,
    pub step_up_required: bool,
    pub idempotency_supported: bool,
    pub expected_version_supported: bool,
    pub postcondition_readback_supported: bool,
    pub staging_supported: bool,
    pub execution_available: ExecutionUnavailable,
    pub warnings: Vec<String>,
    pub data_as_of: DateTime<Utc>,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessApprovalPolicy {
    pub policy_id: String,
    pub version: String,
    pub action_code: String,
    pub risk_level: RiskLevel,
    pub requester_role_keys: Vec<String>,
    pub approver_role_keys: Vec<String>,
    pub minimum_approver_count: u8,
    pub self_approval_allowed: bool,
    pub separation_of_duties: bool,
    pub step_up_level: String,
    pub approval_expiry_minutes: u32,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub config_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproverResolution {
    pub eligible_approver_role_keys: Vec<String>,
    pub eligible_approver_user_ids: Vec<Uuid>,
    pub separation_of_duties: bool,
    pub step_up_required: bool,
    pub minimum_approver_count: u8,
    pub directory_version: String,
    pub permission_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NonExecutableTestGrant {
    pub id: Uuid,
    pub grant_token_hash: String,
    pub approval_request_id: Uuid,
    pub action_code: String,
    pub capability_version: String,
    pub resource_type: String,
    pub resource_id: String,
    pub expected_resource_version: String,
    pub expected_state_hash: String,
    pub approved_payload_hash: String,
    pub approval_policy_version: String,
    pub enterprise_user_id: Uuid,
    pub approver_decision_ids: Vec<Uuid>,
    pub audience: String,
    pub status: TestGrantStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TestGrantStatus {
    #[serde(rename = "NON_EXECUTABLE_TEST_GRANT")]
    NonExecutableTestGrant,
}

pub trait BusinessActionAdapter: Send + Sync {
    fn capabilities(&self) -> &[BusinessWriteCapability];
    fn preflight(&self, request: &PreflightRequest) -> Result<BusinessActionPreflight, String>;
    fn verify_current_state(&self, preflight: &BusinessActionPreflight) -> Result<bool, String>;
    fn verify_postcondition(&self, contract: &PostconditionContract) -> Result<bool, String>;
    fn describe_compensation(&self, action_code: &str) -> Result<String, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreflightRequest {
    pub action_code: String,
    pub resource_type: String,
    pub resource_id: String,
    pub expected_resource_version: String,
    pub proposal_hash: String,
    pub approval_draft_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostconditionContract {
    pub resource_type: String,
    pub resource_id: String,
    pub expected_version: String,
    pub expected_state_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyMethod {
    Get,
    Post,
}

#[derive(Debug, Clone)]
pub struct ReadOnlyEndpointPolicy {
    origins: BTreeSet<String>,
    allowed_get_prefixes: BTreeSet<String>,
    allowed_post_paths: BTreeSet<String>,
}

impl ReadOnlyEndpointPolicy {
    pub fn new(origins: impl IntoIterator<Item = Url>) -> Result<Self, String> {
        let mut normalized = BTreeSet::new();
        for origin in origins {
            if origin.scheme() != "https"
                || origin.username() != ""
                || origin.password().is_some()
                || origin.path() != "/"
                || origin.query().is_some()
                || origin.fragment().is_some()
            {
                return Err("readiness origins must be exact HTTPS origins".into());
            }
            normalized.insert(origin.origin().ascii_serialization());
        }
        if normalized.is_empty() {
            return Err("at least one readiness origin is required".into());
        }
        Ok(Self {
            origins: normalized,
            allowed_get_prefixes: [
                "/read",
                "/search",
                "/capabilities",
                "/preflight",
                "/authorize",
                "/versions",
                "/postconditions",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            allowed_post_paths: ["/read", "/search", "/preflight", "/authorize"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        })
    }

    pub fn allows(&self, method: ReadOnlyMethod, url: &Url) -> bool {
        if url.username() != ""
            || url.password().is_some()
            || url.fragment().is_some()
            || !self.origins.contains(&url.origin().ascii_serialization())
        {
            return false;
        }
        match method {
            ReadOnlyMethod::Get => self.allowed_get_prefixes.iter().any(|prefix| {
                url.path() == prefix || url.path().starts_with(&format!("{prefix}/"))
            }),
            ReadOnlyMethod::Post => {
                url.query().is_none() && self.allowed_post_paths.contains(url.path())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum V7Decision {
    #[serde(rename = "V7_READY")]
    Ready,
    #[serde(rename = "V7_BLOCKED")]
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadinessConditions {
    pub real_business_system_connected: bool,
    pub production_read_adapter_ready: bool,
    pub production_permission_service_ready: bool,
    pub enterprise_directory_ready: bool,
    pub assignee_resolver_ready: bool,
    pub approver_resolver_ready: bool,
    pub real_business_dock_pages_ready: bool,
    pub candidate_action_selected: bool,
    pub candidate_action_low_or_medium_risk: bool,
    pub candidate_action_reversible: bool,
    pub candidate_action_single_object: bool,
    pub candidate_action_staging_supported: bool,
    pub candidate_action_idempotency_supported: bool,
    pub candidate_action_expected_version_supported: bool,
    pub candidate_action_postcondition_supported: bool,
    pub approval_policy_ready: bool,
    pub separation_of_duties_ready: bool,
    pub step_up_auth_ready: bool,
    pub service_identity_ready: bool,
    pub staging_reset_or_recovery_ready: bool,
    pub audit_ready: bool,
    pub kill_switch_ready: bool,
    pub business_execution_disabled: bool,
}

impl ReadinessConditions {
    pub fn named(&self) -> [(&'static str, bool); 23] {
        [
            (
                "real_business_system_connected",
                self.real_business_system_connected,
            ),
            (
                "production_read_adapter_ready",
                self.production_read_adapter_ready,
            ),
            (
                "production_permission_service_ready",
                self.production_permission_service_ready,
            ),
            (
                "enterprise_directory_ready",
                self.enterprise_directory_ready,
            ),
            ("assignee_resolver_ready", self.assignee_resolver_ready),
            ("approver_resolver_ready", self.approver_resolver_ready),
            (
                "real_business_dock_pages_ready",
                self.real_business_dock_pages_ready,
            ),
            ("candidate_action_selected", self.candidate_action_selected),
            (
                "candidate_action_low_or_medium_risk",
                self.candidate_action_low_or_medium_risk,
            ),
            (
                "candidate_action_reversible",
                self.candidate_action_reversible,
            ),
            (
                "candidate_action_single_object",
                self.candidate_action_single_object,
            ),
            (
                "candidate_action_staging_supported",
                self.candidate_action_staging_supported,
            ),
            (
                "candidate_action_idempotency_supported",
                self.candidate_action_idempotency_supported,
            ),
            (
                "candidate_action_expected_version_supported",
                self.candidate_action_expected_version_supported,
            ),
            (
                "candidate_action_postcondition_supported",
                self.candidate_action_postcondition_supported,
            ),
            ("approval_policy_ready", self.approval_policy_ready),
            (
                "separation_of_duties_ready",
                self.separation_of_duties_ready,
            ),
            ("step_up_auth_ready", self.step_up_auth_ready),
            ("service_identity_ready", self.service_identity_ready),
            (
                "staging_reset_or_recovery_ready",
                self.staging_reset_or_recovery_ready,
            ),
            ("audit_ready", self.audit_ready),
            ("kill_switch_ready", self.kill_switch_ready),
            (
                "business_execution_disabled",
                self.business_execution_disabled,
            ),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V7ReadinessEvidence {
    pub schema_version: u8,
    pub environment: BusinessEnvironment,
    pub evaluated_at: DateTime<Utc>,
    pub candidate_action_codes: Vec<String>,
    pub conditions: ReadinessConditions,
    pub evidence_refs: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V7ReadinessReport {
    pub schema_version: u8,
    pub decision: V7Decision,
    pub blockers: Vec<String>,
    pub evaluated_at: DateTime<Utc>,
}

pub fn evaluate_v7_readiness(evidence: &V7ReadinessEvidence) -> V7ReadinessReport {
    let mut blockers = Vec::new();
    if evidence.schema_version != SCHEMA_VERSION {
        blockers.push("unsupported_schema_version".into());
    }
    if !evidence.environment.is_real_structure_non_production() {
        blockers.push("environment_is_not_real_structure_staging_or_sandbox".into());
    }
    if evidence.candidate_action_codes.len() != 1 {
        blockers.push("candidate_action_count_must_equal_one".into());
    }
    for (name, satisfied) in evidence.conditions.named() {
        if !satisfied {
            blockers.push(name.to_owned());
        } else if evidence.evidence_refs.get(name).is_none_or(Vec::is_empty) {
            blockers.push(format!("{name}:missing_evidence"));
        }
    }
    V7ReadinessReport {
        schema_version: SCHEMA_VERSION,
        decision: if blockers.is_empty() {
            V7Decision::Ready
        } else {
            V7Decision::Blocked
        },
        blockers,
        evaluated_at: evidence.evaluated_at,
    }
}

pub fn ensure_v65_execution_disabled(value: Option<&str>) -> Result<(), &'static str> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("true" | "1" | "yes" | "on") => Err(EXECUTION_DISABLED_MESSAGE),
        Some("false" | "0" | "no" | "off") | None => Ok(()),
        Some(_) => Err("BUSINESS_EXECUTION_ENABLED must be a boolean"),
    }
}

pub const AUDIT_EVENT_TYPES: &[&str] = &[
    "REAL_BUSINESS_SYSTEM_CONNECTED",
    "REAL_BUSINESS_SYSTEM_CONNECTION_FAILED",
    "PRODUCTION_PERMISSION_EVALUATED",
    "PRODUCTION_PERMISSION_DENIED",
    "ENTERPRISE_DIRECTORY_RESOLVED",
    "ENTERPRISE_DIRECTORY_RESOLUTION_FAILED",
    "BUSINESS_CAPABILITY_DISCOVERED",
    "BUSINESS_CAPABILITY_REJECTED",
    "V7_CANDIDATE_ACTION_SELECTED",
    "V7_CANDIDATE_ACTION_REJECTED",
    "BUSINESS_ACTION_PREFLIGHT_REQUESTED",
    "BUSINESS_ACTION_PREFLIGHT_SUCCEEDED",
    "BUSINESS_ACTION_PREFLIGHT_FAILED",
    "BUSINESS_EXECUTION_ATTEMPT_BLOCKED",
    "V7_READINESS_GATE_PASSED",
    "V7_READINESS_GATE_FAILED",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn all_conditions(value: bool) -> ReadinessConditions {
        ReadinessConditions {
            real_business_system_connected: value,
            production_read_adapter_ready: value,
            production_permission_service_ready: value,
            enterprise_directory_ready: value,
            assignee_resolver_ready: value,
            approver_resolver_ready: value,
            real_business_dock_pages_ready: value,
            candidate_action_selected: value,
            candidate_action_low_or_medium_risk: value,
            candidate_action_reversible: value,
            candidate_action_single_object: value,
            candidate_action_staging_supported: value,
            candidate_action_idempotency_supported: value,
            candidate_action_expected_version_supported: value,
            candidate_action_postcondition_supported: value,
            approval_policy_ready: value,
            separation_of_duties_ready: value,
            step_up_auth_ready: value,
            service_identity_ready: value,
            staging_reset_or_recovery_ready: value,
            audit_ready: value,
            kill_switch_ready: value,
            business_execution_disabled: value,
        }
    }

    #[test]
    fn execution_true_is_rejected_with_required_message() {
        assert_eq!(
            ensure_v65_execution_disabled(Some("true")),
            Err(EXECUTION_DISABLED_MESSAGE)
        );
        assert!(ensure_v65_execution_disabled(Some("false")).is_ok());
    }

    #[test]
    fn execution_available_can_never_deserialize_true() {
        assert!(serde_json::from_str::<ExecutionUnavailable>("false").is_ok());
        assert!(serde_json::from_str::<ExecutionUnavailable>("true").is_err());
    }

    #[test]
    fn readiness_is_computed_and_requires_evidence_for_every_true_condition() {
        let conditions = all_conditions(true);
        let evidence_refs = conditions
            .named()
            .into_iter()
            .map(|(name, _)| (name.to_owned(), vec![format!("evidence://{name}")]))
            .collect();
        let evidence = V7ReadinessEvidence {
            schema_version: SCHEMA_VERSION,
            environment: BusinessEnvironment::Staging,
            evaluated_at: Utc::now(),
            candidate_action_codes: vec!["fixed_action".into()],
            conditions,
            evidence_refs,
        };
        assert_eq!(evaluate_v7_readiness(&evidence).decision, V7Decision::Ready);

        let mut missing = evidence.clone();
        missing.evidence_refs.remove("step_up_auth_ready");
        let report = evaluate_v7_readiness(&missing);
        assert_eq!(report.decision, V7Decision::Blocked);
        assert!(report
            .blockers
            .contains(&"step_up_auth_ready:missing_evidence".to_owned()));
    }

    #[test]
    fn acceptance_environment_can_never_pass_the_gate() {
        let evidence = V7ReadinessEvidence {
            schema_version: SCHEMA_VERSION,
            environment: BusinessEnvironment::DesensitizedAcceptance,
            evaluated_at: Utc::now(),
            candidate_action_codes: Vec::new(),
            conditions: all_conditions(false),
            evidence_refs: BTreeMap::new(),
        };
        let report = evaluate_v7_readiness(&evidence);
        assert_eq!(report.decision, V7Decision::Blocked);
        assert!(report
            .blockers
            .contains(&"environment_is_not_real_structure_staging_or_sandbox".to_owned()));
    }

    #[test]
    fn dynamic_and_write_endpoints_are_not_expressible() {
        let policy =
            ReadOnlyEndpointPolicy::new([
                Url::parse("https://business-staging.example.com/").expect("origin")
            ])
            .expect("policy");
        assert!(policy.allows(
            ReadOnlyMethod::Post,
            &Url::parse("https://business-staging.example.com/preflight").expect("url")
        ));
        for denied in [
            "https://business-staging.example.com/orders/SO-1/hold",
            "https://business-staging.example.com/preflight?method=PATCH",
            "https://evil.example/preflight",
        ] {
            assert!(!policy.allows(
                ReadOnlyMethod::Post,
                &Url::parse(denied).expect("denied url")
            ));
        }
    }

    #[test]
    fn only_safe_fixed_capabilities_are_pilot_eligible() {
        let mut capability = BusinessWriteCapability {
            capability_id: "cap-1".into(),
            action_code: "fixed_action".into(),
            title: "Fixed action".into(),
            resource_type: "sales_order".into(),
            risk_level: RiskLevel::Low,
            reversible: true,
            compensating_action_code: Some("fixed_compensation".into()),
            supports_dry_run: true,
            supports_expected_version: true,
            supports_idempotency: true,
            supports_postcondition_readback: true,
            required_permissions: vec!["order:review".into()],
            required_approver_roles: vec!["sales_manager".into()],
            minimum_approver_count: 1,
            step_up_required: true,
            staging_supported: true,
            production_supported: false,
            api_contract_version: "1".into(),
            enabled: true,
        };
        assert!(capability.v7_pilot_eligible());
        capability.risk_level = RiskLevel::High;
        assert!(!capability.v7_pilot_eligible());
    }

    fn valid_runtime_values() -> BTreeMap<String, String> {
        [
            ("BUSINESS_ENVIRONMENT", "staging"),
            (
                "BUSINESS_SYSTEM_ORIGIN",
                "https://business-staging.example.com",
            ),
            (
                "BUSINESS_READ_API_BASE_URL",
                "https://business-staging-api.example.com/read/",
            ),
            (
                "BUSINESS_CAPABILITY_API_BASE_URL",
                "https://business-staging-api.example.com/capabilities/",
            ),
            (
                "BUSINESS_PERMISSION_API_BASE_URL",
                "https://permissions-staging.example.com/",
            ),
            (
                "BUSINESS_DIRECTORY_API_BASE_URL",
                "https://directory-staging.example.com/",
            ),
            ("BUSINESS_SERVICE_AUTH_MODE", "workload_identity"),
            ("BUSINESS_SERVICE_AUDIENCE", "business-execution-preflight"),
            ("BUSINESS_EXECUTION_ENABLED", "false"),
            ("BUSINESS_EXECUTION_KILL_SWITCH", "true"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    #[test]
    fn runtime_config_rejects_fake_environment_shared_secret_and_execution_allowlists() {
        let values = valid_runtime_values();
        assert!(V65RuntimeConfig::from_values(&values).is_ok());

        for (key, value) in [
            ("BUSINESS_ENVIRONMENT", "desensitized_acceptance"),
            ("BUSINESS_SERVICE_AUTH_MODE", "shared_secret"),
            ("BUSINESS_EXECUTION_ENABLED", "true"),
            ("BUSINESS_EXECUTION_KILL_SWITCH", "false"),
            ("BUSINESS_EXECUTION_ALLOWED_ACTIONS", "some_action"),
        ] {
            let mut invalid = values.clone();
            invalid.insert(key.into(), value.into());
            assert!(V65RuntimeConfig::from_values(&invalid).is_err(), "{key}");
        }
    }

    #[test]
    fn adapter_contract_has_no_execute_or_compensate_method() {
        let source = include_str!("lib.rs");
        let contract = source
            .split("pub trait BusinessActionAdapter")
            .nth(1)
            .expect("trait")
            .split('}')
            .next()
            .expect("trait body");
        assert!(!contract.contains("fn execute"));
        assert!(!contract.contains("fn compensate"));
    }

    #[test]
    fn evidence_cannot_supply_a_manual_decision_and_requires_exactly_one_candidate() {
        let mut json = serde_json::json!({
            "schemaVersion": 1,
            "environment": "staging",
            "evaluatedAt": "2026-08-20T00:00:00Z",
            "candidateActionCodes": [],
            "conditions": all_conditions(false),
            "evidenceRefs": {},
            "decision": "V7_READY"
        });
        assert!(serde_json::from_value::<V7ReadinessEvidence>(json.clone()).is_err());
        json.as_object_mut().expect("object").remove("decision");
        let evidence = serde_json::from_value::<V7ReadinessEvidence>(json).expect("evidence");
        assert!(evaluate_v7_readiness(&evidence)
            .blockers
            .contains(&"candidate_action_count_must_equal_one".to_owned()));
    }
}
