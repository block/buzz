//! Business authorization policy independent from Buzz transport and storage.
//!
//! Applications resolve roles and direct grants into [`Authority`] values,
//! then call [`evaluate`] for one task. The evaluator is deliberately pure so
//! the gateway can be split into a standalone IAM service without changing
//! authorization semantics.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The durable identity class represented by a business principal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    /// A person authenticated by the enterprise identity provider.
    Human,
    /// A digital employee with its own persistent business entitlements.
    IndependentAgent,
    /// An agent that can only act through a task-scoped human delegation.
    ProxyAgent,
}

/// Whether an IAM principal may participate in authorization decisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalStatus {
    Active,
    Disabled,
}

/// A stable business operation such as `sales_order:read`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capability(pub String);

impl Capability {
    /// Constructs a capability after validating its portable identifier form.
    pub fn parse(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        let valid = (3..=128).contains(&value.len())
            && value.contains(':')
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_:.-".contains(&byte)
            });
        valid.then_some(Self(value)).ok_or("invalid_capability")
    }

    /// Returns the stable string identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Row-level or dimension-level restrictions attached to one capability.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "dimensions", rename_all = "snake_case")]
pub enum DataScope {
    /// No data-dimension restriction beyond the capability itself.
    Unrestricted,
    /// Every named dimension must match one of its allowed values.
    Restricted(BTreeMap<String, BTreeSet<String>>),
}

impl DataScope {
    /// Computes the least-privilege intersection of two scopes.
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::Unrestricted, scope) | (scope, Self::Unrestricted) => Some(scope.clone()),
            (Self::Restricted(left), Self::Restricted(right)) => {
                let mut dimensions = BTreeMap::new();
                for key in left.keys().chain(right.keys()) {
                    if dimensions.contains_key(key) {
                        continue;
                    }
                    let values = match (left.get(key), right.get(key)) {
                        (Some(left), Some(right)) => left.intersection(right).cloned().collect(),
                        (Some(values), None) | (None, Some(values)) => values.clone(),
                        (None, None) => BTreeSet::new(),
                    };
                    if values.is_empty() {
                        return None;
                    }
                    dimensions.insert(key.clone(), values);
                }
                Some(Self::Restricted(dimensions))
            }
        }
    }
}

/// A control the caller must satisfy in addition to a positive decision.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Obligation {
    HumanApproval,
    StepUpAuthentication,
    DualControl,
}

/// One resolved grant after role and direct-assignment expansion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlement {
    pub capability: Capability,
    pub data_scope: DataScope,
    #[serde(default)]
    pub obligations: BTreeSet<Obligation>,
}

/// Resolved authority for a single active or disabled principal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Authority {
    pub principal_id: String,
    pub kind: PrincipalKind,
    pub status: PrincipalStatus,
    /// Persistent grants for humans and independent agents; capability ceilings
    /// for proxy agents.
    pub entitlements: Vec<Entitlement>,
}

/// Capabilities and optional narrower scopes requested for one task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationRequest {
    pub requested: BTreeMap<Capability, DataScope>,
}

/// Effective grant returned by a positive decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveGrant {
    pub capability: Capability,
    pub data_scope: DataScope,
    pub obligations: BTreeSet<Obligation>,
}

/// A deterministic, auditable authorization decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub allowed: bool,
    pub reason: &'static str,
    pub grants: Vec<EffectiveGrant>,
    pub denied_capabilities: Vec<Capability>,
}

/// Evaluates a human, independent-agent, or proxy-agent task.
///
/// Proxy agents never receive persistent business permissions: their
/// `entitlements` are ceilings, and every effective grant is the intersection
/// of the current human authority, that ceiling, and the task request.
pub fn evaluate(
    human: Option<&Authority>,
    agent: &Authority,
    request: &AuthorizationRequest,
) -> Decision {
    if agent.status != PrincipalStatus::Active {
        return denied("agent_inactive", request);
    }
    if request.requested.is_empty() {
        return denied("empty_request", request);
    }

    let source = match agent.kind {
        PrincipalKind::IndependentAgent => &agent.entitlements,
        PrincipalKind::ProxyAgent => {
            let Some(human) = human else {
                return denied("delegating_human_required", request);
            };
            if human.kind != PrincipalKind::Human || human.status != PrincipalStatus::Active {
                return denied("delegating_human_inactive", request);
            }
            return evaluate_proxy(human, agent, request);
        }
        PrincipalKind::Human => return denied("agent_principal_required", request),
    };

    evaluate_against(source, request)
}

fn evaluate_proxy(
    human: &Authority,
    agent: &Authority,
    request: &AuthorizationRequest,
) -> Decision {
    let mut intersected = Vec::new();
    for human_grant in &human.entitlements {
        for ceiling in agent
            .entitlements
            .iter()
            .filter(|grant| grant.capability == human_grant.capability)
        {
            if let Some(data_scope) = human_grant.data_scope.intersection(&ceiling.data_scope) {
                intersected.push(Entitlement {
                    capability: human_grant.capability.clone(),
                    data_scope,
                    obligations: human_grant
                        .obligations
                        .union(&ceiling.obligations)
                        .cloned()
                        .collect(),
                });
            }
        }
    }
    evaluate_against(&intersected, request)
}

fn evaluate_against(entitlements: &[Entitlement], request: &AuthorizationRequest) -> Decision {
    let mut grants = Vec::new();
    let mut denied_capabilities = Vec::new();

    for (capability, requested_scope) in &request.requested {
        let mut effective: Option<EffectiveGrant> = None;
        for entitlement in entitlements
            .iter()
            .filter(|grant| &grant.capability == capability)
        {
            let Some(data_scope) = entitlement.data_scope.intersection(requested_scope) else {
                continue;
            };
            effective = Some(EffectiveGrant {
                capability: capability.clone(),
                data_scope,
                obligations: entitlement.obligations.clone(),
            });
            break;
        }
        if let Some(grant) = effective {
            grants.push(grant);
        } else {
            denied_capabilities.push(capability.clone());
        }
    }

    Decision {
        allowed: !grants.is_empty(),
        reason: if grants.is_empty() {
            "no_effective_permission"
        } else if denied_capabilities.is_empty() {
            "allowed"
        } else {
            "partially_allowed"
        },
        grants,
        denied_capabilities,
    }
}

fn denied(reason: &'static str, request: &AuthorizationRequest) -> Decision {
    Decision {
        allowed: false,
        reason,
        grants: Vec::new(),
        denied_capabilities: request.requested.keys().cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(value: &str) -> Capability {
        Capability::parse(value).unwrap_or_else(|error| panic!("{error}: {value}"))
    }

    fn scope(dimension: &str, values: &[&str]) -> DataScope {
        DataScope::Restricted(BTreeMap::from([(
            dimension.to_string(),
            values.iter().map(|value| (*value).to_string()).collect(),
        )]))
    }

    fn entitlement(name: &str, data_scope: DataScope) -> Entitlement {
        Entitlement {
            capability: capability(name),
            data_scope,
            obligations: BTreeSet::new(),
        }
    }

    fn request(name: &str, data_scope: DataScope) -> AuthorizationRequest {
        AuthorizationRequest {
            requested: BTreeMap::from([(capability(name), data_scope)]),
        }
    }

    #[test]
    fn independent_agent_uses_only_its_own_persistent_permissions() {
        let human = Authority {
            principal_id: "human-1".into(),
            kind: PrincipalKind::Human,
            status: PrincipalStatus::Active,
            entitlements: vec![entitlement("sales_order:read", DataScope::Unrestricted)],
        };
        let agent = Authority {
            principal_id: "agent-1".into(),
            kind: PrincipalKind::IndependentAgent,
            status: PrincipalStatus::Active,
            entitlements: vec![entitlement("inventory:read", DataScope::Unrestricted)],
        };

        assert!(
            !evaluate(
                Some(&human),
                &agent,
                &request("sales_order:read", DataScope::Unrestricted)
            )
            .allowed
        );
        assert!(
            evaluate(
                None,
                &agent,
                &request("inventory:read", DataScope::Unrestricted)
            )
            .allowed
        );
    }

    #[test]
    fn proxy_agent_gets_human_ceiling_request_intersection() {
        let human = Authority {
            principal_id: "human-1".into(),
            kind: PrincipalKind::Human,
            status: PrincipalStatus::Active,
            entitlements: vec![entitlement(
                "sales_order:read",
                scope("legal_entity", &["cn", "sg"]),
            )],
        };
        let agent = Authority {
            principal_id: "agent-1".into(),
            kind: PrincipalKind::ProxyAgent,
            status: PrincipalStatus::Active,
            entitlements: vec![entitlement(
                "sales_order:read",
                scope("legal_entity", &["cn", "us"]),
            )],
        };
        let decision = evaluate(
            Some(&human),
            &agent,
            &request("sales_order:read", scope("legal_entity", &["cn", "jp"])),
        );

        assert!(decision.allowed);
        assert_eq!(
            decision.grants[0].data_scope,
            scope("legal_entity", &["cn"])
        );
    }

    #[test]
    fn proxy_agent_requires_an_active_human() {
        let agent = Authority {
            principal_id: "agent-1".into(),
            kind: PrincipalKind::ProxyAgent,
            status: PrincipalStatus::Active,
            entitlements: vec![entitlement("sales_order:read", DataScope::Unrestricted)],
        };
        let requested = request("sales_order:read", DataScope::Unrestricted);

        assert_eq!(
            evaluate(None, &agent, &requested).reason,
            "delegating_human_required"
        );
    }

    #[test]
    fn disjoint_data_scope_is_denied() {
        let agent = Authority {
            principal_id: "agent-1".into(),
            kind: PrincipalKind::IndependentAgent,
            status: PrincipalStatus::Active,
            entitlements: vec![entitlement(
                "inventory:read",
                scope("warehouse", &["shanghai"]),
            )],
        };

        assert!(
            !evaluate(
                None,
                &agent,
                &request("inventory:read", scope("warehouse", &["shenzhen"]))
            )
            .allowed
        );
    }

    #[test]
    fn partial_request_returns_only_effective_capabilities() {
        let agent = Authority {
            principal_id: "agent-1".into(),
            kind: PrincipalKind::IndependentAgent,
            status: PrincipalStatus::Active,
            entitlements: vec![entitlement("inventory:read", DataScope::Unrestricted)],
        };
        let requested = AuthorizationRequest {
            requested: BTreeMap::from([
                (capability("inventory:read"), DataScope::Unrestricted),
                (capability("payable:read"), DataScope::Unrestricted),
            ]),
        };
        let decision = evaluate(None, &agent, &requested);

        assert!(decision.allowed);
        assert_eq!(decision.reason, "partially_allowed");
        assert_eq!(decision.grants.len(), 1);
        assert_eq!(
            decision.denied_capabilities,
            vec![capability("payable:read")]
        );
    }
}
