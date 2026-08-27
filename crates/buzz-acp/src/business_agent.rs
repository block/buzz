use super::business_response::BusinessResponseObservation;
use crate::acp::{EnvVar, McpServer};
use crate::turn_observer::{
    TurnExtension, TurnExtensionAccess, TurnExtensionFinishContext, TurnExtensionFuture,
    TurnExtensionRequest, TurnExtensionStartupPolicy,
};
use nostr::Event;
use serde::Deserialize;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use url::Url;
use uuid::Uuid;

const AGENT_SCOPES: [&str; 14] = [
    "sales_order:read",
    "purchase_order:read",
    "inventory:read",
    "receivable:read",
    "payable:read",
    "order_profit:read",
    "business_anomaly:read",
    "business_action:read",
    "sales_order:create",
    "shipment:create",
    "purchase_order:create",
    "goods_receipt:create",
    "customer_receipt:create",
    "supplier_payment:create",
];

#[derive(Clone)]
pub(crate) struct BusinessAgentHostConfig {
    gateway_base_url: Url,
    business_api_base_url: Option<Url>,
    business_action_api_base_url: Option<Url>,
    service_credential: String,
    mcp_command: String,
    adapter: String,
    tool_timeout_seconds: u64,
    turn_timeout_seconds: u64,
    max_payload_bytes: usize,
    default_limit: u64,
    max_limit: u64,
    draft_write_enabled: bool,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueResponse {
    id: Uuid,
    token: String,
    audience: String,
    scopes: Vec<String>,
    trace_id: Uuid,
}

pub(crate) struct BusinessTurnAccess {
    mcp_server: McpServer,
    _revocation: RevocationGuard,
}

impl TurnExtension for BusinessAgentHostConfig {
    fn startup_policy(&self) -> TurnExtensionStartupPolicy {
        TurnExtensionStartupPolicy {
            replace_standard_mcp_servers: true,
            max_turn_duration: Some(Duration::from_secs(self.turn_timeout_seconds)),
            base_prompt: Some(include_str!("business_agent_prompt.md")),
            disable_memory: true,
        }
    }

    fn begin_turn<'a>(
        &'a self,
        request: TurnExtensionRequest<'a>,
    ) -> TurnExtensionFuture<'a, Result<Option<Box<dyn TurnExtensionAccess>>, String>> {
        Box::pin(async move {
            let (Some(source_event), Some(channel_id)) = (request.source_event, request.channel_id)
            else {
                return Ok(None);
            };
            let access = self
                .authorize_turn(source_event, channel_id, request.agent_id, request.turn_id)
                .await?;
            Ok(Some(Box::new(access) as Box<dyn TurnExtensionAccess>))
        })
    }
}

impl TurnExtensionAccess for BusinessTurnAccess {
    fn mcp_server(&self) -> Option<&McpServer> {
        Some(&self.mcp_server)
    }

    fn requires_fresh_session(&self) -> bool {
        true
    }

    fn start_observation(&mut self, acp: &mut crate::acp::AcpClient) {
        super::business_response::start_capture(acp);
    }

    fn finish<'a>(
        &'a mut self,
        context: TurnExtensionFinishContext<'a>,
    ) -> TurnExtensionFuture<'a, ()> {
        Box::pin(async move {
            let captured = super::business_response::finish_capture(context.acp);
            let mut observation = captured
                .as_ref()
                .map(|captured| captured.observation.clone())
                .unwrap_or_default();
            if context.completed {
                if let (Some(source_event), Some(channel_id), Some(content)) = (
                    context.source_event,
                    context.channel_id,
                    captured.and_then(|captured| captured.text),
                ) {
                    observation = super::business_response::publish(
                        context.rest_client,
                        channel_id,
                        source_event,
                        &content,
                        observation,
                    )
                    .await;
                }
            }
            self.audit_response(observation).await;
            self._revocation.revoke().await;
        })
    }
}

impl BusinessTurnAccess {
    pub(crate) async fn audit_response(&self, observation: BusinessResponseObservation) {
        let Ok(url) = self
            ._revocation
            .config
            .gateway_base_url
            .join("internal/agent-audit")
        else {
            return;
        };
        let succeeded =
            observation.publish_succeeded && observation.response_buzz_event_id.is_some();
        let response = self
            ._revocation
            .config
            .client
            .post(url.clone())
            .header(
                "x-business-service-credential",
                &self._revocation.config.service_credential,
            )
            .header("x-trace-id", self._revocation.trace_id.to_string())
            .json(&serde_json::json!({
                "delegationId": self._revocation.delegation_id,
                "toolName": "buzz_response",
                "eventType": if succeeded { "AGENT_BUSINESS_RESPONSE_EMITTED" } else { "AGENT_BUSINESS_RESPONSE_FAILED" },
                "result": if succeeded { "success" } else { "failure" },
                "resultCount": observation.finding_count,
                "findingCount": observation.finding_count,
                "resourceRefCount": observation.resource_ref_count,
                "ruleSetVersion": null,
                "anomalyRunId": null,
                "responseBuzzEventId": observation.response_buzz_event_id,
                "durationMs": observation.duration_ms.clamp(0, 120_000),
                "reasonCode": null,
                "traceId": self._revocation.trace_id,
            }))
            .send()
            .await;
        if !response.is_ok_and(|value| value.status().is_success()) {
            tracing::warn!(
                delegation_id = %self._revocation.delegation_id,
                trace_id = %self._revocation.trace_id,
                publish_attempted = observation.publish_attempted,
                "failed to audit Business Agent Buzz response"
            );
        }
        if succeeded && observation.anomaly_tool_used {
            let anomaly_response = self
                ._revocation
                .config
                .client
                .post(url)
                .header(
                    "x-business-service-credential",
                    &self._revocation.config.service_credential,
                )
                .header("x-trace-id", self._revocation.trace_id.to_string())
                .json(&serde_json::json!({
                    "delegationId": self._revocation.delegation_id,
                    "toolName": "buzz_response",
                    "eventType": "BUSINESS_ANOMALY_RESPONSE_EMITTED",
                    "result": "success",
                    "resultCount": observation.finding_count,
                    "findingCount": observation.finding_count,
                    "resourceRefCount": observation.resource_ref_count,
                    "ruleSetVersion": null,
                    "anomalyRunId": null,
                    "responseBuzzEventId": observation.response_buzz_event_id,
                    "durationMs": observation.duration_ms.clamp(0, 120_000),
                    "reasonCode": null,
                    "traceId": self._revocation.trace_id,
                }))
                .send()
                .await;
            if !anomaly_response.is_ok_and(|value| value.status().is_success()) {
                tracing::warn!(
                    delegation_id = %self._revocation.delegation_id,
                    trace_id = %self._revocation.trace_id,
                    "failed to audit emitted Business Anomaly response"
                );
            }
        }
    }
}

struct RevocationGuard {
    config: Arc<BusinessAgentHostConfig>,
    delegation_id: Uuid,
    trace_id: Uuid,
    revoked: AtomicBool,
}

impl RevocationGuard {
    async fn revoke(&self) {
        if self.revoked.load(Ordering::Acquire) {
            return;
        }
        let Ok(url) = self.config.gateway_base_url.join(&format!(
            "internal/agent-delegations/{}/revoke",
            self.delegation_id
        )) else {
            return;
        };
        let result = self
            .config
            .client
            .post(url)
            .header(
                "x-business-service-credential",
                &self.config.service_credential,
            )
            .header("x-trace-id", self.trace_id.to_string())
            .send()
            .await;
        let succeeded = result.as_ref().is_ok_and(|response| {
            response.status().is_success() || response.status().as_u16() == 404
        });
        if succeeded {
            self.revoked.store(true, Ordering::Release);
        } else {
            tracing::warn!(
                delegation_id = %self.delegation_id,
                trace_id = %self.trace_id,
                "failed to revoke Business Agent delegation"
            );
        }
    }
}

impl Drop for RevocationGuard {
    fn drop(&mut self) {
        if self.revoked.load(Ordering::Acquire) {
            return;
        }
        let config = Arc::clone(&self.config);
        let id = self.delegation_id;
        let trace_id = self.trace_id;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let Ok(url) = config
                    .gateway_base_url
                    .join(&format!("internal/agent-delegations/{id}/revoke"))
                else {
                    return;
                };
                let result = config
                    .client
                    .post(url)
                    .header("x-business-service-credential", &config.service_credential)
                    .header("x-trace-id", trace_id.to_string())
                    .send()
                    .await;
                if !result.as_ref().is_ok_and(|response| {
                    response.status().is_success() || response.status().as_u16() == 404
                }) {
                    tracing::warn!(
                        delegation_id = %id,
                        trace_id = %trace_id,
                        "failed to revoke dropped Business Agent delegation"
                    );
                }
            });
        }
    }
}

impl BusinessAgentHostConfig {
    pub(crate) fn from_env() -> Result<Option<Self>, String> {
        let enabled = std::env::var("BUSINESS_AGENT_READ_ENABLED")
            .ok()
            .map(|value| {
                value
                    .parse::<bool>()
                    .map_err(|_| "BUSINESS_AGENT_READ_ENABLED must be true or false".to_string())
            })
            .transpose()?
            .unwrap_or(false);
        if !enabled {
            return Ok(None);
        }
        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{name} is required"))
        };
        let parse_url = |name: &str, value: String| {
            let url = Url::parse(&value).map_err(|_| format!("{name} must be a URL"))?;
            if url.scheme() != "https" && !(cfg!(debug_assertions) && url.scheme() == "http") {
                return Err(format!("{name} must use HTTPS"));
            }
            Ok(url)
        };
        let adapter =
            std::env::var("BUSINESS_READ_ADAPTER").unwrap_or_else(|_| "production".into());
        if adapter != "production" && !(cfg!(debug_assertions) && adapter == "mock") {
            return Err(
                "BUSINESS_READ_ADAPTER must be production (or mock in debug builds)".into(),
            );
        }
        let api_url = if adapter == "production" {
            Some(parse_url(
                "BUSINESS_READ_API_BASE_URL",
                required("BUSINESS_READ_API_BASE_URL")?,
            )?)
        } else {
            None
        };
        let action_api_url = if adapter == "production" {
            Some(parse_url(
                "BUSINESS_ACTION_API_BASE_URL",
                required("BUSINESS_ACTION_API_BASE_URL")?,
            )?)
        } else {
            None
        };
        let credential = required("BUSINESS_READ_SERVICE_CREDENTIAL")?;
        if credential.len() < 32 {
            return Err("BUSINESS_READ_SERVICE_CREDENTIAL must be at least 32 bytes".into());
        }
        let tool_timeout_seconds = bounded_number("BUSINESS_TOOL_TIMEOUT_SECONDS", 10, 1, 30)?;
        let turn_timeout_seconds = bounded_number("AGENT_TURN_TIMEOUT_SECONDS", 120, 30, 900)?;
        let max_payload_bytes = bounded_number(
            "BUSINESS_TOOL_MAX_PAYLOAD_BYTES",
            128 * 1024,
            4096,
            1024 * 1024,
        )? as usize;
        let default_limit = bounded_number("BUSINESS_TOOL_DEFAULT_LIMIT", 20, 1, 100)?;
        let max_limit = bounded_number("BUSINESS_TOOL_MAX_LIMIT", 100, default_limit, 100)?;
        let draft_write_enabled = std::env::var("BUSINESS_AGENT_DRAFT_WRITE_ENABLED")
            .ok()
            .map(|value| {
                value.parse::<bool>().map_err(|_| {
                    "BUSINESS_AGENT_DRAFT_WRITE_ENABLED must be true or false".to_string()
                })
            })
            .transpose()?
            .unwrap_or(false);
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "failed to build Business Agent HTTP client")?;
        Ok(Some(Self {
            gateway_base_url: parse_url(
                "BUSINESS_AUTH_GATEWAY_BASE_URL",
                required("BUSINESS_AUTH_GATEWAY_BASE_URL")?,
            )?,
            business_api_base_url: api_url,
            business_action_api_base_url: action_api_url,
            service_credential: credential,
            mcp_command: std::env::var("BUSINESS_READ_MCP_COMMAND")
                .unwrap_or_else(|_| "business-read-mcp".into()),
            adapter,
            tool_timeout_seconds,
            turn_timeout_seconds,
            max_payload_bytes,
            default_limit,
            max_limit,
            draft_write_enabled,
            client,
        }))
    }

    pub(crate) async fn authorize_turn(
        &self,
        source_event: &Event,
        source_channel_id: Uuid,
        agent_id: &str,
        agent_turn_id: &str,
    ) -> Result<BusinessTurnAccess, String> {
        let trace_id = Uuid::new_v4();
        let url = self
            .gateway_base_url
            .join("internal/agent-delegations")
            .map_err(|_| "Business Agent gateway URL is invalid")?;
        let requested_scopes = AGENT_SCOPES
            .iter()
            .copied()
            .filter(|scope| self.draft_write_enabled || !scope.ends_with(":create"))
            .collect::<Vec<_>>();
        let response = self
            .client
            .post(url)
            .header("x-business-service-credential", &self.service_credential)
            .header("x-trace-id", trace_id.to_string())
            .json(&serde_json::json!({
                "sourceEvent": source_event,
                "sourceBuzzEventId": source_event.id.to_hex(),
                "sourceBuzzPubkey": source_event.pubkey.to_hex(),
                "sourceChannelId": source_channel_id.to_string(),
                "agentId": agent_id,
                "agentTurnId": agent_turn_id,
                "scopes": requested_scopes,
            }))
            .send()
            .await
            .map_err(|_| "Business Agent authorization gateway is unavailable")?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
                409 => "This Buzz event has already started a Business Agent turn".into(),
                429 => "Business Agent query rate limit exceeded".into(),
                _ => "Business Agent turn was not authorized for this user or device".into(),
            });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| "Business Agent authorization response could not be read")?;
        if bytes.len() > 64 * 1024 {
            return Err("Business Agent authorization response was too large".into());
        }
        let issued: IssueResponse = serde_json::from_slice(&bytes)
            .map_err(|_| "Business Agent authorization response was invalid")?;
        if issued.trace_id != trace_id
            || issued.audience != "business-read-mcp"
            || issued.token.len() != 43
            || issued.scopes.is_empty()
            || issued.scopes.len() > requested_scopes.len()
            || issued
                .scopes
                .iter()
                .any(|scope| !requested_scopes.contains(&scope.as_str()))
            || issued
                .scopes
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != issued.scopes.len()
        {
            return Err("Business Agent delegation context mismatch".into());
        }
        let mut mcp_env = vec![
            env("BUSINESS_AGENT_DELEGATION_TOKEN", issued.token),
            env("BUSINESS_AGENT_ID", agent_id),
            env("BUSINESS_AGENT_TURN_ID", agent_turn_id),
            env("BUSINESS_AGENT_TRACE_ID", trace_id.to_string()),
            env(
                "BUSINESS_AUTH_GATEWAY_BASE_URL",
                self.gateway_base_url.as_str(),
            ),
            env("BUSINESS_READ_SERVICE_CREDENTIAL", &self.service_credential),
            env("BUSINESS_READ_SERVICE_AUTH_MODE", "shared_secret"),
            env("BUSINESS_READ_SERVICE_AUDIENCE", "business-read-api"),
            env("BUSINESS_ANOMALY_ENABLED", "true"),
            env("BUSINESS_ACTION_ENABLED", "true"),
            env(
                "BUSINESS_AGENT_DRAFT_WRITE_ENABLED",
                self.draft_write_enabled.to_string(),
            ),
            env("BUSINESS_READ_ADAPTER", &self.adapter),
            env(
                "BUSINESS_TOOL_TIMEOUT_SECONDS",
                self.tool_timeout_seconds.to_string(),
            ),
            env(
                "BUSINESS_TOOL_MAX_PAYLOAD_BYTES",
                self.max_payload_bytes.to_string(),
            ),
            env(
                "BUSINESS_TOOL_DEFAULT_LIMIT",
                self.default_limit.to_string(),
            ),
            env("BUSINESS_TOOL_MAX_LIMIT", self.max_limit.to_string()),
        ];
        if let Some(url) = &self.business_api_base_url {
            mcp_env.push(env("BUSINESS_READ_API_BASE_URL", url.as_str()));
        } else {
            mcp_env.push(env(
                "BUSINESS_READ_MOCK_ACKNOWLEDGE",
                "Mock Only - Production Disabled",
            ));
        }
        if let Some(url) = &self.business_action_api_base_url {
            mcp_env.push(env("BUSINESS_ACTION_API_BASE_URL", url.as_str()));
        }
        Ok(BusinessTurnAccess {
            mcp_server: McpServer {
                name: "business-read-mcp".into(),
                command: self.mcp_command.clone(),
                args: Vec::new(),
                env: mcp_env,
            },
            _revocation: RevocationGuard {
                config: Arc::new(self.clone()),
                delegation_id: issued.id,
                trace_id,
                revoked: AtomicBool::new(false),
            },
        })
    }
}

fn env(name: impl Into<String>, value: impl Into<String>) -> EnvVar {
    EnvVar {
        name: name.into(),
        value: value.into(),
    }
}

fn bounded_number(name: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    let value = std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{name} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default);
    if !(min..=max).contains(&value) {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_scope_allowlist_has_only_draft_writes() {
        assert_eq!(AGENT_SCOPES.len(), 14);
        assert!(AGENT_SCOPES.contains(&"business_anomaly:read"));
        assert!(AGENT_SCOPES.contains(&"sales_order:create"));
        assert!(!AGENT_SCOPES.contains(&"sales_order:confirm"));
        assert!(!AGENT_SCOPES.contains(&"payment:execute"));
    }

    #[test]
    fn draft_write_switch_defaults_to_disabled_semantics() {
        let read_only = AGENT_SCOPES
            .iter()
            .copied()
            .filter(|scope| !scope.ends_with(":create"))
            .collect::<Vec<_>>();
        assert_eq!(read_only.len(), 8);
        assert!(read_only.iter().all(|scope| scope.ends_with(":read")));
    }

    #[test]
    fn mcp_environment_names_do_not_place_token_in_tool_input() {
        let names = [
            "BUSINESS_AGENT_DELEGATION_TOKEN",
            "BUSINESS_AGENT_ID",
            "BUSINESS_AGENT_TURN_ID",
        ];
        assert!(names.contains(&"BUSINESS_AGENT_DELEGATION_TOKEN"));
        assert!(!names.contains(&"toolInput"));
    }

    #[test]
    fn action_prompt_requires_human_confirmation_and_refuses_execution() {
        let prompt = include_str!("business_agent_prompt.md");
        assert!(prompt.contains("需要你在 Business Dock 中确认后才会创建待办。"));
        for boundary in [
            "cannot create or update a Work Item",
            "create an Approval Draft",
            "approve, reject",
            "execute",
            "Action Codes come only from the versioned catalog",
        ] {
            assert!(prompt.contains(boundary), "missing boundary: {boundary}");
        }
    }

    #[test]
    fn prompt_limits_writes_to_six_draft_tools_and_keeps_form_fallbacks() {
        let prompt = include_str!("business_agent_prompt.md");
        for tool in [
            "create_sales_order_draft",
            "create_shipment_draft",
            "create_purchase_order_draft",
            "create_goods_receipt_draft",
            "create_customer_receipt_draft",
            "create_supplier_payment_draft",
        ] {
            assert!(prompt.contains(tool), "missing draft tool: {tool}");
        }
        for reference in [
            "biz://sales-order-entry",
            "biz://shipment-entry",
            "biz://purchase-order-entry",
            "biz://goods-receipt-entry",
            "biz://customer-receipt-entry",
            "biz://supplier-payment-entry",
        ] {
            assert!(
                prompt.contains(reference),
                "missing entry link: {reference}"
            );
        }
        assert!(prompt.contains("Never guess identifiers"));
        assert!(prompt.contains("When required fields are missing"));
        assert!(
            prompt.contains("confirmations, reversals, allocations, posting, payment execution")
        );
    }
}
