//! Stateful, language-neutral access to [`buzz_ifc`] policy decisions.
//!
//! The broker accepts bounded JSON-RPC requests from a trusted Buzz adapter.
//! That adapter remains responsible for authenticating events and resolving
//! authoritative membership. The broker derives the execution domain from
//! those verified facts, retains process-level confinement state, and returns
//! decisions that a local ACP bridge or remote harness can enforce consistently.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use buzz_ifc::{
    derive_execution_domain, CapabilityPolicy, CapabilitySet, ConfidentialityLabel,
    ConversationKind, DomainContext, DomainFacts, ExecutionDomain, MembershipEpoch, Principal,
    ProcessState, RealmId, ResourceLabel, RuleDecision, RuleEvaluator,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

/// Version of the JSON-RPC contract implemented by this broker.
pub const PROTOCOL_VERSION: &str = "buzz-ifc-broker/1";

/// Whether denied decisions are observational or enforced by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnforcementMode {
    /// Return and log decisions while conservatively tracking inputs that
    /// entered the worker despite a denial.
    Audit,
    /// Require the caller to stop denied reads, calls, publications, and
    /// cross-domain worker reuse.
    Enforce,
}

impl EnforcementMode {
    fn enforced(self) -> bool {
        matches!(self, Self::Enforce)
    }
}

impl fmt::Display for EnforcementMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Audit => f.write_str("audit"),
            Self::Enforce => f.write_str("enforce"),
        }
    }
}

/// Stateful policy broker. One instance serves one trusted adapter stream.
pub struct Broker {
    mode: EnforcementMode,
    workers: HashMap<String, WorkerRecord>,
}

impl Broker {
    /// Construct an empty broker in the selected enforcement mode.
    pub fn new(mode: EnforcementMode) -> Self {
        Self {
            mode,
            workers: HashMap::new(),
        }
    }

    /// Parse one JSON-RPC line and return one JSON-RPC response line.
    pub fn handle_line(&mut self, line: &str) -> String {
        let response = match serde_json::from_str::<RpcRequest>(line) {
            Ok(request) => self.handle_request(request),
            Err(error) => RpcResponse::error(
                Value::Null,
                -32700,
                "parse error",
                Some(json!({ "detail": error.to_string() })),
            ),
        };
        match serde_json::to_string(&response) {
            Ok(encoded) => encoded,
            Err(_) => {
                "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"response encoding failed\"}}".to_string()
            }
        }
    }

    /// Return a JSON-RPC error for an input frame rejected before JSON parsing.
    pub fn frame_error(message: &str) -> String {
        let response = RpcResponse::error(Value::Null, -32600, message, None);
        match serde_json::to_string(&response) {
            Ok(encoded) => encoded,
            Err(_) => {
                "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"internal error\"}}".to_string()
            }
        }
    }

    fn handle_request(&mut self, request: RpcRequest) -> RpcResponse {
        let id = match request.id {
            Some(id) => id,
            None => {
                return RpcResponse::error(
                    Value::Null,
                    -32600,
                    "security decisions require a request id",
                    None,
                )
            }
        };
        if request.jsonrpc != "2.0" {
            return RpcResponse::error(id, -32600, "jsonrpc must equal 2.0", None);
        }

        let method = request.method.clone();
        let outcome = self.dispatch(&request.method, request.params);
        match outcome {
            Ok(result) => {
                tracing::info!(
                    target: "buzz_ifc_broker::decision",
                    method,
                    mode = %self.mode,
                    outcome = "ok",
                    "IFC broker request evaluated"
                );
                RpcResponse::success(id, result)
            }
            Err(error) => {
                tracing::info!(
                    target: "buzz_ifc_broker::decision",
                    method,
                    mode = %self.mode,
                    outcome = "error",
                    code = error.code,
                    reason = error.message,
                    "IFC broker request rejected"
                );
                RpcResponse::error(id, error.code, error.message, error.data)
            }
        }
    }

    fn dispatch(&mut self, method: &str, params: Value) -> Result<Value, BrokerError> {
        match method {
            "broker/info" => Ok(json!({
                "protocol_version": PROTOCOL_VERSION,
                "mode": self.mode.to_string(),
                "worker_count": self.workers.len(),
            })),
            "domain/derive" => {
                let params: DeriveParams = parse_params(params)?;
                let domain = params.invocation.resolve()?;
                Ok(domain_description(&domain))
            }
            "worker/enter" => {
                let params: EnterParams = parse_params(params)?;
                validate_worker_id(&params.worker_id)?;
                let domain = params.invocation.resolve()?;
                let domain_id = domain.id();
                let compartment = domain.compartment_profile().as_str();
                let record = self.workers.entry(params.worker_id.clone()).or_default();
                let decision = record.state.enter(&domain);
                let replace_worker = !decision.allowed();
                if replace_worker {
                    record.state.mark_unknown();
                    if self.mode.enforced() {
                        record.poisoned = true;
                    } else {
                        record.domain = Some(domain);
                    }
                } else {
                    record.domain = Some(domain);
                }
                Ok(decision_value(
                    self.mode,
                    "reuse",
                    &decision,
                    Some(&params.worker_id),
                    Some(&domain_id),
                    json!({
                        "replace_worker": replace_worker,
                        "compartment": compartment,
                    }),
                ))
            }
            "worker/observe" => {
                let params: ObserveParams = parse_params(params)?;
                let mode = self.mode;
                let record = active_worker_mut(&mut self.workers, &params.worker_id)?;
                if record.poisoned {
                    return Ok(poisoned_decision(mode, &params.worker_id, record));
                }
                let domain = record.domain.as_ref().ok_or_else(|| {
                    BrokerError::state("worker has not entered an execution domain")
                })?;
                let resource = params.resource.resolve(domain)?;
                let decision = RuleEvaluator::read(domain, &resource);
                if decision.allowed() || !mode.enforced() {
                    record.state.observe(&resource);
                }
                Ok(decision_value(
                    mode,
                    "read",
                    &decision,
                    Some(&params.worker_id),
                    Some(&domain.id()),
                    json!({ "source": params.source }),
                ))
            }
            "worker/call" => {
                let params: CallParams = parse_params(params)?;
                let record = active_worker(&self.workers, &params.worker_id)?;
                if record.poisoned {
                    return Ok(poisoned_decision(self.mode, &params.worker_id, record));
                }
                let domain = record.domain.as_ref().ok_or_else(|| {
                    BrokerError::state("worker has not entered an execution domain")
                })?;
                let decision = RuleEvaluator::call(domain, &params.operation);
                Ok(decision_value(
                    self.mode,
                    "call",
                    &decision,
                    Some(&params.worker_id),
                    Some(&domain.id()),
                    json!({ "operation": params.operation }),
                ))
            }
            "worker/publish" => {
                let params: PublishParams = parse_params(params)?;
                let record = active_worker(&self.workers, &params.worker_id)?;
                if record.poisoned {
                    return Ok(poisoned_decision(self.mode, &params.worker_id, record));
                }
                let domain = record.domain.as_ref().ok_or_else(|| {
                    BrokerError::state("worker has not entered an execution domain")
                })?;
                let (audience, context) = params.destination.resolve(domain)?;
                let digest = parse_digest(&params.content_sha256)?;
                let decision = record
                    .state
                    .publish(domain, &audience, &context, &digest, None);
                Ok(decision_value(
                    self.mode,
                    "publish",
                    &decision,
                    Some(&params.worker_id),
                    Some(&domain.id()),
                    json!({ "destination": context.kind() }),
                ))
            }
            "worker/retire" => {
                let params: WorkerParams = parse_params(params)?;
                validate_worker_id(&params.worker_id)?;
                Ok(json!({
                    "worker_id": params.worker_id,
                    "retired": self.workers.remove(&params.worker_id).is_some(),
                }))
            }
            _ => Err(BrokerError::method_not_found()),
        }
    }
}

#[derive(Default)]
struct WorkerRecord {
    state: ProcessState,
    domain: Option<ExecutionDomain>,
    poisoned: bool,
}

#[derive(Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcErrorBody>,
}

impl RpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcErrorBody {
                code,
                message: message.to_string(),
                data,
            }),
        }
    }
}

#[derive(Serialize)]
struct RpcErrorBody {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

struct BrokerError {
    code: i64,
    message: &'static str,
    data: Option<Value>,
}

impl BrokerError {
    fn invalid_params(detail: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: "invalid params",
            data: Some(json!({ "detail": detail.into() })),
        }
    }

    fn state(detail: impl Into<String>) -> Self {
        Self {
            code: -32001,
            message: "invalid broker state",
            data: Some(json!({ "detail": detail.into() })),
        }
    }

    fn method_not_found() -> Self {
        Self {
            code: -32601,
            message: "method not found",
            data: None,
        }
    }
}

fn parse_params<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, BrokerError> {
    serde_json::from_value(params).map_err(|error| BrokerError::invalid_params(error.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeriveParams {
    invocation: InvocationSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnterParams {
    worker_id: String,
    invocation: InvocationSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObserveParams {
    worker_id: String,
    source: String,
    resource: ResourceSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallParams {
    worker_id: String,
    operation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishParams {
    worker_id: String,
    content_sha256: String,
    destination: DestinationSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerParams {
    worker_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationSpec {
    realm_url: String,
    channel_id: Uuid,
    conversation_kind: ConversationKindSpec,
    epoch: String,
    #[serde(default)]
    members: Vec<String>,
    executing_agent: String,
    requesters: Vec<String>,
    system_principal: Option<String>,
    owner: Option<String>,
    #[serde(default)]
    bot_capabilities: Vec<String>,
    #[serde(default)]
    conversation_capabilities: Vec<String>,
}

impl InvocationSpec {
    fn resolve(self) -> Result<ExecutionDomain, BrokerError> {
        if self.realm_url.trim().is_empty() {
            return Err(BrokerError::invalid_params("realm_url must not be empty"));
        }
        if self.epoch.trim().is_empty() {
            return Err(BrokerError::invalid_params("epoch must not be empty"));
        }
        let policy = CapabilityPolicy::new(
            CapabilitySet::from_names(self.bot_capabilities),
            CapabilitySet::from_names(self.conversation_capabilities),
        );
        derive_execution_domain(
            DomainFacts {
                realm: RealmId::from_relay_url(&self.realm_url),
                channel_id: self.channel_id,
                kind: self.conversation_kind.into(),
                epoch: MembershipEpoch::new(self.epoch),
                members: resolve_principals(self.members)?,
                executing_agent: parse_principal(&self.executing_agent)?,
                requesters: resolve_principals(self.requesters)?,
                system_principal: self
                    .system_principal
                    .as_deref()
                    .map(parse_principal)
                    .transpose()?,
                owner: self.owner.as_deref().map(parse_principal).transpose()?,
            },
            &policy,
        )
        .map_err(|error| BrokerError::invalid_params(error.to_string()))
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConversationKindSpec {
    Public,
    Restricted,
    DirectMessage,
}

impl From<ConversationKindSpec> for ConversationKind {
    fn from(value: ConversationKindSpec) -> Self {
        match value {
            ConversationKindSpec::Public => Self::Public,
            ConversationKindSpec::Restricted => Self::Restricted,
            ConversationKindSpec::DirectMessage => Self::DirectMessage,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ResourceSpec {
    Domain,
    TrustedConfiguration,
    Public,
    Conversation {
        channel_id: Uuid,
        readers: Vec<String>,
    },
    OwnerPrivate {
        owner: String,
    },
}

impl ResourceSpec {
    fn resolve(self, domain: &ExecutionDomain) -> Result<ResourceLabel, BrokerError> {
        let realm = domain.audience().realm().clone();
        match self {
            Self::Domain => Ok(ResourceLabel::domain(domain)),
            Self::TrustedConfiguration => Ok(ResourceLabel::trusted_configuration(realm)),
            Self::Public => Ok(ResourceLabel::realm_public(realm)),
            Self::Conversation {
                channel_id,
                readers,
            } => ResourceLabel::conversation(realm, channel_id, resolve_principals(readers)?)
                .map_err(|error| BrokerError::invalid_params(error.to_string())),
            Self::OwnerPrivate { owner } => Ok(ResourceLabel::owner_private(
                realm,
                parse_principal(&owner)?,
            )),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DestinationSpec {
    Current,
    Public,
    Conversation {
        channel_id: Uuid,
        readers: Vec<String>,
    },
    OwnerPrivate {
        owner: String,
    },
}

impl DestinationSpec {
    fn resolve(
        self,
        domain: &ExecutionDomain,
    ) -> Result<(ConfidentialityLabel, DomainContext), BrokerError> {
        let realm = domain.audience().realm().clone();
        match self {
            Self::Current => Ok((domain.audience().clone(), domain.context().clone())),
            Self::Public => Ok((
                ConfidentialityLabel::public(realm.clone()),
                DomainContext::RealmPublic(realm),
            )),
            Self::Conversation {
                channel_id,
                readers,
            } => Ok((
                ConfidentialityLabel::restricted(realm.clone(), resolve_principals(readers)?)
                    .map_err(|error| BrokerError::invalid_params(error.to_string()))?,
                DomainContext::Conversation { realm, channel_id },
            )),
            Self::OwnerPrivate { owner } => {
                let owner = parse_principal(&owner)?;
                Ok((
                    ConfidentialityLabel::restricted(
                        realm.clone(),
                        BTreeSet::from([owner.clone()]),
                    )
                    .map_err(|error| BrokerError::invalid_params(error.to_string()))?,
                    DomainContext::OwnerPrivate { realm, owner },
                ))
            }
        }
    }
}

fn resolve_principals(values: Vec<String>) -> Result<BTreeSet<Principal>, BrokerError> {
    values
        .into_iter()
        .map(|value| parse_principal(&value))
        .collect()
}

fn parse_principal(value: &str) -> Result<Principal, BrokerError> {
    Principal::from_hex(value).map_err(|error| BrokerError::invalid_params(error.to_string()))
}

fn parse_digest(value: &str) -> Result<[u8; 32], BrokerError> {
    let bytes = hex::decode(value)
        .map_err(|error| BrokerError::invalid_params(format!("invalid content_sha256: {error}")))?;
    bytes
        .try_into()
        .map_err(|_| BrokerError::invalid_params("content_sha256 must contain 32 bytes"))
}

fn validate_worker_id(value: &str) -> Result<(), BrokerError> {
    let length = value.len();
    if length == 0 || length > 256 {
        return Err(BrokerError::invalid_params(
            "worker_id must contain between 1 and 256 bytes",
        ));
    }
    Ok(())
}

fn active_worker<'a>(
    workers: &'a HashMap<String, WorkerRecord>,
    worker_id: &str,
) -> Result<&'a WorkerRecord, BrokerError> {
    validate_worker_id(worker_id)?;
    workers
        .get(worker_id)
        .ok_or_else(|| BrokerError::state("worker has not entered an execution domain"))
}

fn active_worker_mut<'a>(
    workers: &'a mut HashMap<String, WorkerRecord>,
    worker_id: &str,
) -> Result<&'a mut WorkerRecord, BrokerError> {
    validate_worker_id(worker_id)?;
    workers
        .get_mut(worker_id)
        .ok_or_else(|| BrokerError::state("worker has not entered an execution domain"))
}

fn domain_description(domain: &ExecutionDomain) -> Value {
    json!({
        "domain_id": domain.id(),
        "domain_fingerprint": domain.key().fingerprint(),
        "context": domain.context().kind(),
        "compartment": domain.compartment_profile().as_str(),
        "audience": if domain.audience().is_public() { "public" } else { "restricted" },
        "reader_count": domain.audience().reader_count(),
        "epoch_fingerprint": domain.epoch_fingerprint(),
    })
}

fn decision_value(
    mode: EnforcementMode,
    rule: &'static str,
    decision: &RuleDecision,
    worker_id: Option<&str>,
    domain_id: Option<&str>,
    details: Value,
) -> Value {
    tracing::info!(
        target: "buzz_ifc_broker::decision",
        rule,
        decision = decision.result(),
        reason = decision.reason(),
        enforced = mode.enforced(),
        worker_id,
        domain_id,
        "IFC broker security decision"
    );
    json!({
        "decision": decision.result(),
        "allowed": decision.allowed(),
        "reason": decision.reason(),
        "enforced": mode.enforced(),
        "worker_id": worker_id,
        "domain_id": domain_id,
        "details": details,
    })
}

fn poisoned_decision(mode: EnforcementMode, worker_id: &str, record: &WorkerRecord) -> Value {
    let reason = "worker crossed an execution-domain boundary and must be retired";
    let domain_id = record.domain.as_ref().map(ExecutionDomain::id);
    tracing::info!(
        target: "buzz_ifc_broker::decision",
        rule = "reuse",
        decision = "deny",
        reason,
        enforced = mode.enforced(),
        worker_id,
        domain_id,
        "IFC broker security decision"
    );
    json!({
        "decision": "deny",
        "allowed": false,
        "reason": reason,
        "enforced": mode.enforced(),
        "worker_id": worker_id,
        "domain_id": domain_id,
        "details": { "replace_worker": true },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "0101010101010101010101010101010101010101010101010101010101010101";
    const BOB: &str = "0202020202020202020202020202020202020202020202020202020202020202";
    const AGENT: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    fn request(id: u64, method: &str, params: Value) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
    }

    fn restricted_invocation(epoch: &str) -> Value {
        json!({
            "realm_url": "wss://buzz.example",
            "channel_id": "00000000-0000-0000-0000-000000000001",
            "conversation_kind": "restricted",
            "epoch": epoch,
            "members": [AGENT, ALICE, BOB],
            "executing_agent": AGENT,
            "requesters": [ALICE],
            "owner": ALICE,
            "bot_capabilities": ["buzz.read.current", "email.read"],
            "conversation_capabilities": ["buzz.read.current"]
        })
    }

    fn response(line: &str) -> Value {
        serde_json::from_str(line).expect("broker response")
    }

    #[test]
    fn cross_domain_reuse_requires_worker_replacement() {
        let mut broker = Broker::new(EnforcementMode::Enforce);
        let first = response(&broker.handle_line(&request(
            1,
            "worker/enter",
            json!({ "worker_id": "worker-1", "invocation": restricted_invocation("v1") }),
        )));
        assert_eq!(first["result"]["decision"], "allow");
        assert_eq!(first["result"]["details"]["compartment"], "domain_confined");

        let second = response(&broker.handle_line(&request(
            2,
            "worker/enter",
            json!({ "worker_id": "worker-1", "invocation": restricted_invocation("v2") }),
        )));
        assert_eq!(second["result"]["decision"], "deny");
        assert_eq!(second["result"]["details"]["replace_worker"], true);

        let call = response(&broker.handle_line(&request(
            3,
            "worker/call",
            json!({ "worker_id": "worker-1", "operation": "buzz.read.current" }),
        )));
        assert_eq!(call["result"]["decision"], "deny");
    }

    #[test]
    fn owner_private_read_is_denied_in_a_conversation_domain() {
        let mut broker = Broker::new(EnforcementMode::Enforce);
        let _ = broker.handle_line(&request(
            1,
            "worker/enter",
            json!({ "worker_id": "worker-1", "invocation": restricted_invocation("v1") }),
        ));
        let read = response(&broker.handle_line(&request(
            2,
            "worker/observe",
            json!({
                "worker_id": "worker-1",
                "source": "email",
                "resource": { "kind": "owner_private", "owner": ALICE }
            }),
        )));
        assert_eq!(read["result"]["decision"], "deny");
        assert_eq!(read["result"]["enforced"], true);
    }

    #[test]
    fn conversation_worker_is_bound_to_shared_tools_and_its_source_context() {
        let mut broker = Broker::new(EnforcementMode::Enforce);
        let _ = broker.handle_line(&request(
            1,
            "worker/enter",
            json!({ "worker_id": "worker-1", "invocation": restricted_invocation("v1") }),
        ));

        let call = response(&broker.handle_line(&request(
            2,
            "worker/call",
            json!({ "worker_id": "worker-1", "operation": "email.read" }),
        )));
        assert_eq!(call["result"]["decision"], "deny");

        let observe = response(&broker.handle_line(&request(
            3,
            "worker/observe",
            json!({
                "worker_id": "worker-1",
                "source": "conversation_history",
                "resource": { "kind": "domain" }
            }),
        )));
        assert_eq!(observe["result"]["decision"], "allow");

        let current = response(&broker.handle_line(&request(
            4,
            "worker/publish",
            json!({
                "worker_id": "worker-1",
                "content_sha256": "00".repeat(32),
                "destination": { "kind": "current" }
            }),
        )));
        assert_eq!(current["result"]["decision"], "allow");

        let other = response(&broker.handle_line(&request(
            5,
            "worker/publish",
            json!({
                "worker_id": "worker-1",
                "content_sha256": "00".repeat(32),
                "destination": {
                    "kind": "conversation",
                    "channel_id": "00000000-0000-0000-0000-000000000002",
                    "readers": [ALICE, BOB]
                }
            }),
        )));
        assert_eq!(other["result"]["decision"], "deny");
    }

    #[test]
    fn malformed_verified_fact_fails_derivation_without_panicking() {
        let mut broker = Broker::new(EnforcementMode::Audit);
        let invalid = response(&broker.handle_line(&request(
            1,
            "domain/derive",
            json!({
                "invocation": {
                    "realm_url": "wss://buzz.example",
                    "channel_id": "00000000-0000-0000-0000-000000000001",
                    "conversation_kind": "direct_message",
                    "epoch": "v1",
                    "members": [AGENT, ALICE],
                    "executing_agent": AGENT,
                    "requesters": [ALICE],
                    "owner": "not-a-key"
                }
            }),
        )));
        assert_eq!(invalid["error"]["code"], -32602);
    }

    #[test]
    fn public_domain_selects_the_shared_public_compartment() {
        let mut broker = Broker::new(EnforcementMode::Audit);
        let derived = response(&broker.handle_line(&request(
            1,
            "domain/derive",
            json!({
                "invocation": {
                    "realm_url": "wss://buzz.example",
                    "channel_id": "00000000-0000-0000-0000-000000000001",
                    "conversation_kind": "public",
                    "epoch": "community:v1",
                    "executing_agent": AGENT,
                    "requesters": [ALICE],
                    "owner": ALICE
                }
            }),
        )));

        assert_eq!(derived["result"]["compartment"], "shared_public");
    }
}
