use crate::{
    iam::TurnDecision,
    model::{Audit, RequestFacts},
    security,
    store::{Rejection, Store},
};
use business_iam::EffectiveGrant;
use chrono::{Duration, Utc};
use nostr::Event;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

const READ_SCOPES: [&str; 8] = [
    "sales_order:read",
    "purchase_order:read",
    "inventory:read",
    "receivable:read",
    "payable:read",
    "order_profit:read",
    "business_anomaly:read",
    "business_action:read",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueAgentDelegationRequest {
    pub source_event: Event,
    pub source_buzz_event_id: String,
    pub source_buzz_pubkey: String,
    pub source_channel_id: String,
    pub agent_id: String,
    pub agent_turn_id: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueAgentDelegationResponse {
    pub id: Uuid,
    pub token: String,
    pub audience: String,
    pub scopes: Vec<String>,
    pub expires_at: chrono::DateTime<Utc>,
    pub max_calls: i32,
    pub trace_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsumeAgentDelegationRequest {
    pub tool_name: String,
    pub required_scope: String,
    pub agent_id: String,
    pub agent_turn_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyAgentDelegationRequest {
    pub delegation_id: Uuid,
    pub enterprise_user_id: Uuid,
    pub identity_binding_id: Uuid,
    pub agent_id: String,
    pub agent_turn_id: String,
    pub trace_id: Uuid,
    pub used_calls: i32,
    pub required_scope: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDelegationContext {
    pub delegation_id: Uuid,
    pub enterprise_user_id: Uuid,
    pub identity_binding_id: Uuid,
    pub source_buzz_event_id: String,
    pub source_buzz_pubkey: String,
    pub source_channel_id: String,
    pub agent_id: String,
    pub agent_turn_id: String,
    pub trace_id: Uuid,
    pub used_calls: i32,
    pub max_calls: i32,
    pub required_scope: String,
    pub effective_grant: EffectiveGrant,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentToolAuditRequest {
    pub delegation_id: Uuid,
    pub tool_name: String,
    pub event_type: AgentToolAuditEvent,
    pub result: AgentToolAuditResult,
    pub result_count: Option<i32>,
    pub finding_count: Option<i32>,
    pub resource_ref_count: Option<i32>,
    pub rule_set_version: Option<String>,
    pub anomaly_run_id: Option<Uuid>,
    pub response_buzz_event_id: Option<String>,
    pub duration_ms: i64,
    pub reason_code: Option<String>,
    pub trace_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentToolAuditEvent {
    BusinessMcpToolSucceeded,
    BusinessMcpToolFailed,
    BusinessReadAuthorizationDenied,
    BusinessReadPartialResult,
    AgentBusinessResponseEmitted,
    AgentBusinessResponseFailed,
    BusinessAnomalyRunCompleted,
    BusinessAnomalyRunPartial,
    BusinessAnomalyRunFailed,
    BusinessAnomalyRunStarted,
    BusinessAnomalyFindingCreated,
    BusinessAnomalyAuthorizationDenied,
    BusinessAnomalyDataQualityBlocked,
    BusinessAnomalyResponseEmitted,
    AgentActionRecommendationEmitted,
    BusinessActionAuthorizationDenied,
}

impl AgentToolAuditEvent {
    fn as_str(&self) -> &'static str {
        match self {
            Self::BusinessMcpToolSucceeded => "BUSINESS_MCP_TOOL_SUCCEEDED",
            Self::BusinessMcpToolFailed => "BUSINESS_MCP_TOOL_FAILED",
            Self::BusinessReadAuthorizationDenied => "BUSINESS_READ_AUTHORIZATION_DENIED",
            Self::BusinessReadPartialResult => "BUSINESS_READ_PARTIAL_RESULT",
            Self::AgentBusinessResponseEmitted => "AGENT_BUSINESS_RESPONSE_EMITTED",
            Self::AgentBusinessResponseFailed => "AGENT_BUSINESS_RESPONSE_FAILED",
            Self::BusinessAnomalyRunCompleted => "BUSINESS_ANOMALY_RUN_COMPLETED",
            Self::BusinessAnomalyRunPartial => "BUSINESS_ANOMALY_RUN_PARTIAL",
            Self::BusinessAnomalyRunFailed => "BUSINESS_ANOMALY_RUN_FAILED",
            Self::BusinessAnomalyRunStarted => "BUSINESS_ANOMALY_RUN_STARTED",
            Self::BusinessAnomalyFindingCreated => "BUSINESS_ANOMALY_FINDING_CREATED",
            Self::BusinessAnomalyAuthorizationDenied => "BUSINESS_ANOMALY_AUTHORIZATION_DENIED",
            Self::BusinessAnomalyDataQualityBlocked => "BUSINESS_ANOMALY_DATA_QUALITY_BLOCKED",
            Self::BusinessAnomalyResponseEmitted => "BUSINESS_ANOMALY_RESPONSE_EMITTED",
            Self::AgentActionRecommendationEmitted => "AGENT_ACTION_RECOMMENDATION_EMITTED",
            Self::BusinessActionAuthorizationDenied => "BUSINESS_ACTION_AUTHORIZATION_DENIED",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentToolAuditResult {
    Success,
    Failure,
}

impl AgentToolAuditResult {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

fn safe_id(value: &str) -> bool {
    let len = value.chars().count();
    (1..=128).contains(&len)
        && !value.chars().any(char::is_control)
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

fn source_event_has_channel(event: &Event, channel: &str) -> bool {
    event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().is_some_and(|name| name == "h")
            && values.get(1).is_some_and(|value| value == channel)
    })
}

fn data_scope_hash(
    user_id: Uuid,
    scopes: &[String],
    effective_grants: &serde_json::Value,
) -> Vec<u8> {
    let mut scopes = scopes.to_vec();
    scopes.sort();
    Sha256::digest(format!("{user_id}:{}:{effective_grants}", scopes.join(",")).as_bytes()).to_vec()
}

impl Store {
    pub async fn issue_agent_delegation(
        &self,
        request: IssueAgentDelegationRequest,
        facts: RequestFacts,
    ) -> Result<IssueAgentDelegationResponse, Rejection> {
        let valid_source = request.source_event.id.to_hex() == request.source_buzz_event_id
            && request.source_event.pubkey.to_hex() == request.source_buzz_pubkey
            && request.source_event.verify_id()
            && request.source_event.verify_signature()
            && source_event_has_channel(&request.source_event, &request.source_channel_id);
        let valid_ids = security::valid_pubkey(&request.source_buzz_pubkey)
            && security::valid_pubkey(&request.source_buzz_event_id)
            && safe_id(&request.agent_id)
            && safe_id(&request.agent_turn_id)
            && security::safe_text(&request.source_channel_id, 1, 200);
        let valid_scopes = !request.scopes.is_empty()
            && request.scopes.len() <= READ_SCOPES.len()
            && request
                .scopes
                .iter()
                .all(|scope| READ_SCOPES.contains(&scope.as_str()));
        if !valid_source || !valid_ids || !valid_scopes {
            let mut audit = Audit::event("AGENT_TURN_REJECTED", "failure", facts);
            audit.reason = Some(if !valid_source {
                "source_event_invalid"
            } else if !valid_scopes {
                "scope_rejected"
            } else {
                "turn_context_invalid"
            });
            audit.agent_id = Some(request.agent_id);
            audit.agent_turn_id = Some(request.agent_turn_id);
            audit.source_buzz_event_id = Some(request.source_buzz_event_id);
            audit.source_channel_id = Some(request.source_channel_id);
            audit.pubkey_short = Some(security::short_pubkey(&request.source_buzz_pubkey));
            self.audit(audit).await?;
            return Err(Rejection::Forbidden("agent_turn_rejected"));
        }

        let mut tx = self.pool().begin().await.map_err(|_| Rejection::Database)?;
        let binding = sqlx::query(
            "SELECT b.id,b.enterprise_user_id FROM buzz_identity_bindings b
             JOIN enterprise_users u ON u.id=b.enterprise_user_id
             WHERE b.buzz_pubkey=$1 AND b.status='active' AND u.status='active' FOR UPDATE",
        )
        .bind(&request.source_buzz_pubkey)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| Rejection::Database)?;
        let Some(binding) = binding else {
            let mut audit = Audit::event("AGENT_TURN_REJECTED", "failure", facts);
            audit.reason = Some("binding_or_user_inactive");
            audit.agent_id = Some(request.agent_id);
            audit.agent_turn_id = Some(request.agent_turn_id);
            audit.source_buzz_event_id = Some(request.source_buzz_event_id);
            audit.source_channel_id = Some(request.source_channel_id);
            audit.pubkey_short = Some(security::short_pubkey(&request.source_buzz_pubkey));
            Store::audit_tx(&mut tx, audit).await?;
            tx.commit().await.map_err(|_| Rejection::Database)?;
            return Err(Rejection::Forbidden("binding_or_user_inactive"));
        };
        let binding_id: Uuid = binding.get("id");
        let user_id: Uuid = binding.get("enterprise_user_id");
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM agent_read_delegations WHERE enterprise_user_id=$1 AND created_at>now()-interval '1 minute'",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Rejection::Database)?;
        if recent >= self.config().business_agent_rate_limit_per_minute {
            let mut audit = Audit::event("AGENT_TURN_REJECTED", "failure", facts);
            audit.reason = Some("rate_limited");
            audit.user_id = Some(user_id);
            audit.binding_id = Some(binding_id);
            audit.agent_id = Some(request.agent_id);
            audit.agent_turn_id = Some(request.agent_turn_id);
            Store::audit_tx(&mut tx, audit).await?;
            tx.commit().await.map_err(|_| Rejection::Database)?;
            return Err(Rejection::RateLimited);
        }
        let authorization = self
            .authorize_agent_turn_tx(
                &mut tx,
                user_id,
                &request.agent_id,
                &request.agent_turn_id,
                &request.scopes,
                facts.trace_id,
            )
            .await?;
        let grant = match authorization {
            TurnDecision::Granted(grant) => grant,
            TurnDecision::Denied(reason) => {
                let mut audit = Audit::event("AGENT_TURN_REJECTED", "failure", facts);
                audit.reason = Some(reason);
                audit.user_id = Some(user_id);
                audit.binding_id = Some(binding_id);
                audit.agent_id = Some(request.agent_id);
                audit.agent_turn_id = Some(request.agent_turn_id);
                audit.source_buzz_event_id = Some(request.source_buzz_event_id);
                audit.source_channel_id = Some(request.source_channel_id);
                audit.pubkey_short = Some(security::short_pubkey(&request.source_buzz_pubkey));
                Store::audit_tx(&mut tx, audit).await?;
                tx.commit().await.map_err(|_| Rejection::Database)?;
                return Err(Rejection::Forbidden("business_iam_denied"));
            }
        };
        let token = security::random_token();
        let id = Uuid::new_v4();
        let expires_at = Utc::now()
            + Duration::from_std(self.config().agent_delegation_ttl)
                .map_err(|_| Rejection::Database)?;
        let inserted = sqlx::query(
            "INSERT INTO agent_read_delegations(id,token_hash,enterprise_user_id,identity_binding_id,agent_id,agent_turn_id,source_buzz_event_id,source_channel_id,audience,scopes,data_scope_hash,max_calls,expires_at,trace_id,iam_decision_id,agent_principal_id,effective_grants)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
        )
        .bind(id)
        .bind(security::hash(&token))
        .bind(user_id)
        .bind(binding_id)
        .bind(&request.agent_id)
        .bind(&request.agent_turn_id)
        .bind(&request.source_buzz_event_id)
        .bind(&request.source_channel_id)
        .bind(&self.config().business_read_mcp_audience)
        .bind(&grant.scopes)
        .bind(data_scope_hash(
            user_id,
            &grant.scopes,
            &grant.effective_grants,
        ))
        .bind(self.config().agent_delegation_max_calls)
        .bind(expires_at)
        .bind(facts.trace_id)
        .bind(grant.decision_id)
        .bind(grant.agent_principal_id)
        .bind(&grant.effective_grants)
        .execute(&mut *tx)
        .await;
        if let Err(error) = inserted {
            tx.rollback().await.map_err(|_| Rejection::Database)?;
            if error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
            {
                let mut audit = Audit::event("AGENT_TURN_REJECTED", "failure", facts);
                audit.reason = Some("source_event_already_authorized");
                audit.user_id = Some(user_id);
                audit.binding_id = Some(binding_id);
                audit.agent_id = Some(request.agent_id);
                audit.agent_turn_id = Some(request.agent_turn_id);
                audit.source_buzz_event_id = Some(request.source_buzz_event_id);
                audit.source_channel_id = Some(request.source_channel_id);
                audit.pubkey_short = Some(security::short_pubkey(&request.source_buzz_pubkey));
                self.audit(audit).await?;
                return Err(Rejection::Conflict("source_event_already_authorized"));
            }
            return Err(Rejection::Database);
        }
        for event_type in ["AGENT_TURN_AUTHORIZED", "AGENT_DELEGATION_ISSUED"] {
            let mut audit = Audit::event(event_type, "success", facts.clone());
            audit.user_id = Some(user_id);
            audit.binding_id = Some(binding_id);
            audit.delegation_id = Some(id);
            audit.pubkey_short = Some(security::short_pubkey(&request.source_buzz_pubkey));
            audit.agent_id = Some(request.agent_id.clone());
            audit.agent_turn_id = Some(request.agent_turn_id.clone());
            audit.source_buzz_event_id = Some(request.source_buzz_event_id.clone());
            audit.source_channel_id = Some(request.source_channel_id.clone());
            Store::audit_tx(&mut tx, audit).await?;
        }
        tx.commit().await.map_err(|_| Rejection::Database)?;
        Ok(IssueAgentDelegationResponse {
            id,
            token,
            audience: self.config().business_read_mcp_audience.clone(),
            scopes: grant.scopes,
            expires_at,
            max_calls: self.config().agent_delegation_max_calls,
            trace_id: facts.trace_id,
        })
    }

    pub async fn consume_agent_delegation(
        &self,
        token: &str,
        request: ConsumeAgentDelegationRequest,
        facts: RequestFacts,
    ) -> Result<AgentDelegationContext, Rejection> {
        if token.len() != 43
            || !safe_id(&request.tool_name)
            || !safe_id(&request.agent_id)
            || !safe_id(&request.agent_turn_id)
            || !READ_SCOPES.contains(&request.required_scope.as_str())
        {
            return Err(Rejection::Unauthorized("delegation_invalid"));
        }
        let mut tx = self.pool().begin().await.map_err(|_| Rejection::Database)?;
        let row = sqlx::query(
            "UPDATE agent_read_delegations d SET
               used_calls=d.used_calls+1,
               last_used_at=now(),
               status=CASE WHEN d.used_calls+1>=d.max_calls THEN 'exhausted' ELSE d.status END,
               version=d.version+1
             FROM buzz_identity_bindings b, enterprise_users u
             WHERE d.token_hash=$1 AND d.identity_binding_id=b.id AND d.enterprise_user_id=u.id
               AND d.status='active' AND d.expires_at>now()
               AND d.audience=$2 AND d.agent_id=$3 AND d.agent_turn_id=$4
               AND $5=ANY(d.scopes) AND d.used_calls<d.max_calls
               AND b.status='active' AND u.status='active'
             RETURNING d.id,d.enterprise_user_id,d.identity_binding_id,d.source_buzz_event_id,
               d.source_channel_id,d.agent_id,d.agent_turn_id,d.trace_id,d.used_calls,d.max_calls,
               b.buzz_pubkey,d.status,d.effective_grants",
        )
        .bind(security::hash(token))
        .bind(&self.config().business_read_mcp_audience)
        .bind(&request.agent_id)
        .bind(&request.agent_turn_id)
        .bind(&request.required_scope)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| Rejection::Database)?;
        let Some(row) = row else {
            sqlx::query(
                "UPDATE agent_read_delegations SET status=CASE
                   WHEN status='active' AND expires_at<=now() THEN 'expired'
                   WHEN status='active' AND used_calls>=max_calls THEN 'exhausted'
                   ELSE status END,
                 version=CASE WHEN status='active' AND (expires_at<=now() OR used_calls>=max_calls) THEN version+1 ELSE version END
                 WHERE token_hash=$1",
            )
            .bind(security::hash(token))
            .execute(&mut *tx)
            .await
            .map_err(|_| Rejection::Database)?;
            let denied_event = match request.required_scope.as_str() {
                "business_anomaly:read" => "BUSINESS_ANOMALY_AUTHORIZATION_DENIED",
                "business_action:read" => "BUSINESS_ACTION_AUTHORIZATION_DENIED",
                _ => "BUSINESS_READ_AUTHORIZATION_DENIED",
            };
            let mut audit = Audit::event(denied_event, "failure", facts);
            audit.reason = Some("delegation_expired_revoked_exhausted_or_scope_mismatch");
            audit.agent_id = Some(request.agent_id);
            audit.agent_turn_id = Some(request.agent_turn_id);
            audit.tool_name = Some(request.tool_name);
            Store::audit_tx(&mut tx, audit).await?;
            tx.commit().await.map_err(|_| Rejection::Database)?;
            return Err(Rejection::Unauthorized("delegation_rejected"));
        };
        let status: String = row.get("status");
        let effective_grant = effective_grant(row.get("effective_grants"), &request.required_scope)
            .ok_or(Rejection::Database)?;
        let mut audit = Audit::event("BUSINESS_MCP_TOOL_CALLED", "success", facts);
        audit.user_id = Some(row.get("enterprise_user_id"));
        audit.binding_id = Some(row.get("identity_binding_id"));
        audit.delegation_id = Some(row.get("id"));
        audit.agent_id = Some(row.get("agent_id"));
        audit.agent_turn_id = Some(row.get("agent_turn_id"));
        audit.source_buzz_event_id = Some(row.get("source_buzz_event_id"));
        audit.source_channel_id = Some(row.get("source_channel_id"));
        audit.tool_name = Some(request.tool_name);
        audit.pubkey_short = Some(security::short_pubkey(row.get("buzz_pubkey")));
        Store::audit_tx(&mut tx, audit).await?;
        if status == "exhausted" {
            let mut exhausted = Audit::event(
                "AGENT_DELEGATION_EXHAUSTED",
                "success",
                RequestFacts {
                    trace_id: row.get("trace_id"),
                    ..RequestFacts::default()
                },
            );
            exhausted.delegation_id = Some(row.get("id"));
            exhausted.user_id = Some(row.get("enterprise_user_id"));
            exhausted.binding_id = Some(row.get("identity_binding_id"));
            exhausted.agent_id = Some(row.get("agent_id"));
            exhausted.agent_turn_id = Some(row.get("agent_turn_id"));
            Store::audit_tx(&mut tx, exhausted).await?;
        }
        let context = AgentDelegationContext {
            delegation_id: row.get("id"),
            enterprise_user_id: row.get("enterprise_user_id"),
            identity_binding_id: row.get("identity_binding_id"),
            source_buzz_event_id: row.get("source_buzz_event_id"),
            source_buzz_pubkey: row.get("buzz_pubkey"),
            source_channel_id: row.get("source_channel_id"),
            agent_id: row.get("agent_id"),
            agent_turn_id: row.get("agent_turn_id"),
            trace_id: row.get("trace_id"),
            used_calls: row.get("used_calls"),
            max_calls: row.get("max_calls"),
            required_scope: request.required_scope,
            effective_grant,
        };
        tx.commit().await.map_err(|_| Rejection::Database)?;
        Ok(context)
    }

    pub async fn verify_agent_delegation(
        &self,
        request: VerifyAgentDelegationRequest,
        facts: RequestFacts,
    ) -> Result<EffectiveGrant, Rejection> {
        if request.delegation_id.is_nil()
            || request.enterprise_user_id.is_nil()
            || request.identity_binding_id.is_nil()
            || !safe_id(&request.agent_id)
            || !safe_id(&request.agent_turn_id)
            || request.trace_id != facts.trace_id
            || request.used_calls <= 0
            || !READ_SCOPES.contains(&request.required_scope.as_str())
        {
            return Err(Rejection::Forbidden("delegation_context_rejected"));
        }
        let row = sqlx::query(
            "SELECT d.effective_grants FROM agent_read_delegations d
               JOIN buzz_identity_bindings b ON b.id=d.identity_binding_id
               JOIN enterprise_users u ON u.id=d.enterprise_user_id
               WHERE d.id=$1 AND d.enterprise_user_id=$2 AND d.identity_binding_id=$3
                 AND d.agent_id=$4 AND d.agent_turn_id=$5 AND d.trace_id=$6
                 AND d.used_calls=$7 AND d.used_calls<=d.max_calls
                 AND $8=ANY(d.scopes)
                 AND d.status IN ('active','exhausted') AND d.expires_at>now()
                 AND b.status='active' AND u.status='active'
             ",
        )
        .bind(request.delegation_id)
        .bind(request.enterprise_user_id)
        .bind(request.identity_binding_id)
        .bind(&request.agent_id)
        .bind(&request.agent_turn_id)
        .bind(request.trace_id)
        .bind(request.used_calls)
        .bind(&request.required_scope)
        .fetch_optional(self.pool())
        .await
        .map_err(|_| Rejection::Database)?;
        if let Some(row) = row {
            effective_grant(row.get("effective_grants"), &request.required_scope)
                .ok_or(Rejection::Database)
        } else {
            let mut audit = Audit::event("BUSINESS_READ_AUTHORIZATION_DENIED", "failure", facts);
            audit.reason = Some("delegation_changed_or_revoked_before_response");
            audit.delegation_id = Some(request.delegation_id);
            audit.agent_id = Some(request.agent_id);
            audit.agent_turn_id = Some(request.agent_turn_id);
            self.audit(audit).await?;
            Err(Rejection::Forbidden("delegation_rejected"))
        }
    }

    pub async fn revoke_agent_delegation(
        &self,
        id: Uuid,
        facts: RequestFacts,
    ) -> Result<(), Rejection> {
        let mut tx = self.pool().begin().await.map_err(|_| Rejection::Database)?;
        let row = sqlx::query(
            "UPDATE agent_read_delegations SET status='revoked',revoked_at=now(),version=version+1
             WHERE id=$1 AND status IN ('active','exhausted')
             RETURNING enterprise_user_id,identity_binding_id,agent_id,agent_turn_id,source_buzz_event_id,source_channel_id,trace_id",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| Rejection::Database)?
        .ok_or(Rejection::NotFound)?;
        let mut audit = Audit::event("AGENT_DELEGATION_REVOKED", "success", facts);
        audit.facts.trace_id = row.get("trace_id");
        audit.user_id = Some(row.get("enterprise_user_id"));
        audit.binding_id = Some(row.get("identity_binding_id"));
        audit.delegation_id = Some(id);
        audit.agent_id = Some(row.get("agent_id"));
        audit.agent_turn_id = Some(row.get("agent_turn_id"));
        audit.source_buzz_event_id = Some(row.get("source_buzz_event_id"));
        audit.source_channel_id = Some(row.get("source_channel_id"));
        Store::audit_tx(&mut tx, audit).await?;
        tx.commit().await.map_err(|_| Rejection::Database)
    }

    pub async fn audit_agent_tool(
        &self,
        request: AgentToolAuditRequest,
        facts: RequestFacts,
    ) -> Result<(), Rejection> {
        if !safe_id(&request.tool_name)
            || request.duration_ms < 0
            || request.duration_ms > 120_000
            || request
                .result_count
                .is_some_and(|count| !(0..=100).contains(&count))
            || request
                .finding_count
                .is_some_and(|count| !(0..=100).contains(&count))
            || request
                .resource_ref_count
                .is_some_and(|count| !(0..=1000).contains(&count))
            || request
                .rule_set_version
                .as_deref()
                .is_some_and(|value| !safe_id(value))
            || request
                .response_buzz_event_id
                .as_deref()
                .is_some_and(|value| {
                    value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            || request.trace_id != facts.trace_id
        {
            return Err(Rejection::Invalid("invalid_audit_event"));
        }
        let row = sqlx::query(
            "SELECT enterprise_user_id,identity_binding_id,agent_id,agent_turn_id,source_buzz_event_id,source_channel_id,trace_id
             FROM agent_read_delegations WHERE id=$1",
        )
        .bind(request.delegation_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|_| Rejection::Database)?
        .ok_or(Rejection::NotFound)?;
        if row.get::<Uuid, _>("trace_id") != request.trace_id {
            return Err(Rejection::Forbidden("trace_mismatch"));
        }
        let mut audit = Audit::event(request.event_type.as_str(), request.result.as_str(), facts);
        audit.reason = request.reason_code.as_deref().and_then(known_reason_code);
        audit.user_id = Some(row.get("enterprise_user_id"));
        audit.binding_id = Some(row.get("identity_binding_id"));
        audit.delegation_id = Some(request.delegation_id);
        audit.agent_id = Some(row.get("agent_id"));
        audit.agent_turn_id = Some(row.get("agent_turn_id"));
        audit.source_buzz_event_id = Some(row.get("source_buzz_event_id"));
        audit.response_buzz_event_id = request.response_buzz_event_id;
        audit.source_channel_id = Some(row.get("source_channel_id"));
        audit.tool_name = Some(request.tool_name);
        audit.result_count = request.result_count;
        audit.finding_count = request.finding_count;
        audit.resource_ref_count = request.resource_ref_count;
        audit.rule_set_version = request.rule_set_version;
        audit.anomaly_run_id = request.anomaly_run_id;
        audit.duration_ms = Some(request.duration_ms);
        self.audit(audit).await
    }
}

fn effective_grant(value: serde_json::Value, required_scope: &str) -> Option<EffectiveGrant> {
    serde_json::from_value::<Vec<EffectiveGrant>>(value)
        .ok()?
        .into_iter()
        .find(|grant| grant.capability.as_str() == required_scope)
}

fn known_reason_code(value: &str) -> Option<&'static str> {
    match value {
        "partial_data" => Some("partial_data"),
        "upstream_unavailable" => Some("upstream_unavailable"),
        "invalid_filter" => Some("invalid_filter"),
        "not_found_or_forbidden" => Some("not_found_or_forbidden"),
        "rate_limited" => Some("rate_limited"),
        "payload_too_large" => Some("payload_too_large"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    #[test]
    fn source_event_must_carry_matching_channel() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "query")
            .tags([nostr::Tag::custom(
                nostr::TagKind::Custom("h".into()),
                ["channel-a"],
            )])
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(source_event_has_channel(&event, "channel-a"));
        assert!(!source_event_has_channel(&event, "channel-b"));
    }

    #[test]
    fn only_fixed_read_scopes_are_accepted() {
        assert!(READ_SCOPES.contains(&"inventory:read"));
        assert!(!READ_SCOPES.contains(&"sales_order:write"));
    }
}
