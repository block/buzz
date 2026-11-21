use std::collections::BTreeSet;

use serde::Serialize;

use crate::declassification::{DeclassificationGrant, VerifiedGrant};
use crate::domain::{DomainContext, ExecutionDomain, ResourceContext, ResourceLabel};
use crate::label::{ConfidentialityLabel, LabelError};

/// The result of evaluating one IFC rule.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuleDecision {
    allowed: bool,
    reason: &'static str,
}

impl RuleDecision {
    fn allow(reason: &'static str) -> Self {
        Self {
            allowed: true,
            reason,
        }
    }

    fn deny(reason: &'static str) -> Self {
        Self {
            allowed: false,
            reason,
        }
    }

    /// Whether the operation is admitted by policy.
    pub fn allowed(&self) -> bool {
        self.allowed
    }

    /// Stable explanation intended for logs and operator diagnostics.
    pub fn reason(&self) -> &'static str {
        self.reason
    }

    /// Return `allow` or `deny` for structured logs.
    pub fn result(&self) -> &'static str {
        if self.allowed {
            "allow"
        } else {
            "deny"
        }
    }
}

/// Pure IFC rule evaluation.
pub struct RuleEvaluator;

impl RuleEvaluator {
    /// Evaluate `read(D, x) ⇔ A(D) ⊆ R(x) ∧ ContextPolicy(D, x)`.
    pub fn read(domain: &ExecutionDomain, resource: &ResourceLabel) -> RuleDecision {
        if !resource.confidentiality.can_flow_to(&domain.audience) {
            return RuleDecision::deny("destination audience includes an unauthorized reader");
        }
        if !domain.context.permits(&resource.context) {
            return RuleDecision::deny("resource belongs to a different context");
        }
        RuleDecision::allow("audience and context both permit the read")
    }

    /// Evaluate `call(D, op) ⇔ op ∈ C(D)`.
    pub fn call(domain: &ExecutionDomain, operation: &str) -> RuleDecision {
        if domain.capabilities.contains(operation) {
            RuleDecision::allow("operation is in the effective capability set")
        } else {
            RuleDecision::deny("operation is absent from the effective capability set")
        }
    }

    /// Require every component of the execution domain to match for reuse.
    pub fn reuse(existing: &ExecutionDomain, requested: &ExecutionDomain) -> RuleDecision {
        if existing == requested {
            RuleDecision::allow("complete execution domain matches")
        } else {
            RuleDecision::deny("agent process has already entered a different domain")
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ConfinementState {
    label: Option<ConfidentialityLabel>,
    contexts: BTreeSet<ResourceContext>,
    unknown_input: bool,
    cross_realm: bool,
}

impl ConfinementState {
    fn observe(&mut self, resource: &ResourceLabel) {
        self.contexts.insert(resource.context.clone());
        self.label = match self.label.take() {
            None => Some(resource.confidentiality.clone()),
            Some(existing) => match existing.join(&resource.confidentiality) {
                Ok(combined) => Some(combined),
                Err(LabelError::CrossRealm) => {
                    self.cross_realm = true;
                    Some(existing)
                }
                Err(LabelError::EmptyReaderSet) => Some(existing),
            },
        };
    }
}

/// Conservative policy state for one actual agent process.
///
/// Paper: "Confinement invariant." The accumulated label and observed contexts
/// cover every input that actually entered the process.
///
/// This state intentionally survives model-session invalidation. A new session
/// does not make the surrounding process forget information it has observed.
#[derive(Clone, Debug, Default)]
pub struct ProcessState {
    entered_domains: Vec<ExecutionDomain>,
    confinement: ConfinementState,
}

impl ProcessState {
    /// Record entry into a domain and decide whether this process is reusable.
    pub fn enter(&mut self, requested: &ExecutionDomain) -> RuleDecision {
        let decision = match self.entered_domains.as_slice() {
            [] => RuleDecision::allow("fresh process has no prior execution domain"),
            [existing] => RuleEvaluator::reuse(existing, requested),
            _ => RuleDecision::deny("agent process has entered multiple execution domains"),
        };
        if !self
            .entered_domains
            .iter()
            .any(|domain| domain == requested)
        {
            self.entered_domains.push(requested.clone());
        }
        decision
    }

    /// Record an input that actually entered the process.
    pub fn observe(&mut self, resource: &ResourceLabel) {
        self.confinement.observe(resource);
    }

    /// Record input whose provenance could not be established.
    pub fn mark_unknown(&mut self) {
        self.confinement.unknown_input = true;
    }

    /// Return how many distinct domains have entered this process.
    pub fn entered_domain_count(&self) -> usize {
        self.entered_domains.len()
    }

    /// Paper: "Confinement invariant" and "Declassification." Evaluate
    /// publication under the process's accumulated label.
    pub fn publish(
        &self,
        source_domain: &ExecutionDomain,
        destination: &ConfidentialityLabel,
        destination_context: &DomainContext,
        content_digest: &[u8; 32],
        grant: Option<&mut DeclassificationGrant<VerifiedGrant>>,
    ) -> RuleDecision {
        if self.entered_domains.len() != 1 || self.entered_domains.first() != Some(source_domain) {
            return RuleDecision::deny(
                "process state is not confined to the claimed source domain",
            );
        }
        if self.confinement.unknown_input || self.confinement.cross_realm {
            return RuleDecision::deny("process state contains unresolved input provenance");
        }
        if self
            .confinement
            .contexts
            .iter()
            .any(|context| !source_domain.context.permits(context))
        {
            return RuleDecision::deny("process state contains input from another context");
        }
        if grant.is_some_and(|grant| {
            grant.matches(
                &source_domain.id(),
                destination,
                destination_context,
                content_digest,
            )
        }) {
            return RuleDecision::allow("exact owner-authorized declassification grant matches");
        }
        if &source_domain.context != destination_context {
            return RuleDecision::deny(
                "publishing to a different context requires declassification",
            );
        }
        if !source_domain.audience.can_flow_to(destination) {
            return RuleDecision::deny("destination is broader than the execution domain");
        }
        if self
            .confinement
            .label
            .as_ref()
            .is_some_and(|label| label.can_flow_to(destination))
        {
            return RuleDecision::allow("destination is no broader than accumulated state");
        }
        RuleDecision::deny("output would widen the accumulated reader set")
    }
}
