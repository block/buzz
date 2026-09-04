use ifc_core::{EgressError, FlowState};

use crate::domain::{DomainContext, DomainKey, ExecutionDomain, MembershipEpoch, OperationEffect};
use crate::label::{CommunityId, ConfidentialityLabel, Principal};

/// The security metadata the broker checks before exposing a resource to an
/// agent session.
///
/// Constructing this from the domain where the resource originated keeps its
/// audience, retained-state context, and membership epoch together. Public
/// community data does not carry an epoch because any context in that
/// community may read it. Restricted data retains the epoch under which its
/// source domain was authorized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceLabel {
    audience: ConfidentialityLabel,
    context: DomainContext,
    epoch: Option<MembershipEpoch>,
}

impl ResourceLabel {
    /// Label a resource with the security metadata of its source domain.
    pub fn from_domain(domain: &ExecutionDomain) -> Self {
        let epoch = (!matches!(domain.context, DomainContext::CommunityPublic(_)))
            .then(|| domain.epoch.clone());
        Self {
            audience: domain.audience.clone(),
            context: domain.context.clone(),
            epoch,
        }
    }
}

/// The audience receiving a publication.
///
/// This type describes only the IFC destination. The trusted broker remains
/// responsible for ordinary product policy, including resolving and checking
/// the channel, message, URL, or other concrete destination named by the
/// operation payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationTarget {
    audience: ConfidentialityLabel,
}

impl PublicationTarget {
    /// Use an execution domain's authorized audience as a publication target.
    pub fn from_domain(domain: &ExecutionDomain) -> Self {
        Self {
            audience: domain.audience.clone(),
        }
    }
}

/// A broker request to execute one publication operation with an exact
/// payload.
pub struct PublicationRequest<T> {
    operation: String,
    target: PublicationTarget,
    payload: T,
}

impl<T> PublicationRequest<T> {
    /// Construct a request after the broker has resolved its IFC target.
    pub fn new(operation: impl Into<String>, target: PublicationTarget, payload: T) -> Self {
        Self {
            operation: operation.into(),
            target,
            payload,
        }
    }
}

/// Proof that an exact publication request passed the session's capability
/// and information-flow checks.
///
/// A broker sink should accept this type rather than [`PublicationRequest`].
/// Its fields are private and it is not cloneable, so only
/// [`IfcSession::publish`] can create the value that authorizes a sink call.
///
/// ```compile_fail
/// let forged = buzz_ifc::AuthorizedPublication {
///     operation: "buzz.reply".to_owned(),
///     target: todo!(),
///     payload: "unchecked",
/// };
/// ```
#[must_use = "the authorization must be consumed by the publication sink"]
pub struct AuthorizedPublication<T> {
    operation: String,
    target: PublicationTarget,
    payload: T,
}

impl<T> AuthorizedPublication<T> {
    /// Return the checked operation name.
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// Return the exact payload covered by the information-flow decision.
    pub fn payload(&self) -> &T {
        &self.payload
    }

    /// Consume the authorization and return the checked sink inputs.
    pub fn into_parts(self) -> (String, PublicationTarget, T) {
        (self.operation, self.target, self.payload)
    }
}

/// Information-flow state for one retained agent execution domain.
///
/// The broker calls [`read`](Self::read) before delivering data to the agent,
/// [`call`](Self::call) before executing operations that cannot publish, and
/// [`publish`](Self::publish) before handing an exact outbound payload to a
/// sink. These are the ordinary-flow checks described in Appendices F and G of
/// the [design paper](../../../docs/practical-information-flow-for-buzz-agents.md#appendix-f-reference-monitor-pseudocode).
///
/// ```
/// # use buzz_ifc::{AuthorizedPublication, ExecutionDomain, IfcError, IfcSession,
/// #     PublicationRequest, PublicationTarget, ResourceLabel};
/// # fn broker_sink(_: AuthorizedPublication<Vec<u8>>) {}
/// # fn run_turn(
/// #     domain: ExecutionDomain,
/// #     resource: &ResourceLabel,
/// #     target: PublicationTarget,
/// # ) -> Result<(), IfcError> {
/// let mut session = IfcSession::enter(domain);
/// session.read(resource)?;
/// session.call("buzz.read.current")?;
///
/// let authorization = session.publish(PublicationRequest::new(
///     "buzz.reply",
///     target,
///     b"hello".to_vec(),
/// ))?;
/// broker_sink(authorization);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct IfcSession {
    domain: ExecutionDomain,
    flow: FlowState<CommunityId, Principal>,
}

impl IfcSession {
    /// Enter an execution domain selected by the trusted broker.
    ///
    /// The domain's audience is observed immediately because retained agent
    /// state and broker-provided instructions may already influence the next
    /// output before the first explicit resource read.
    pub fn enter(domain: ExecutionDomain) -> Self {
        let mut flow = FlowState::default();
        flow.observe(&domain.audience);
        Self { domain, flow }
    }

    /// Return the key the broker uses to route later turns to this session.
    pub fn domain_key(&self) -> DomainKey {
        self.domain.key()
    }

    /// Check and record a labeled resource before exposing it to the agent.
    ///
    /// A failed read does not taint the session because the broker must not
    /// deliver the rejected resource. Successful reads are recorded
    /// monotonically; the domain audience already conservatively bounds output
    /// in this coarse-grained Buzz model.
    pub fn read(&mut self, resource: &ResourceLabel) -> Result<(), IfcError> {
        if !resource.audience.can_flow_to(&self.domain.audience) {
            return Err(IfcError::ReadAudienceDenied);
        }
        if !self.domain.context.permits(&resource.context) {
            return Err(IfcError::ReadContextDenied);
        }
        if resource.context == self.domain.context
            && resource
                .epoch
                .as_ref()
                .is_some_and(|epoch| epoch != &self.domain.epoch)
        {
            return Err(IfcError::StaleResourceEpoch);
        }

        self.flow.observe(&resource.audience);
        Ok(())
    }

    /// Permanently record that unlabeled input reached the agent.
    ///
    /// Ordinary publication remains blocked for the rest of the session. A
    /// future declassification API may authorize a specific exceptional flow,
    /// but this small wrapper deliberately provides no bypass.
    pub fn mark_unknown_input(&mut self) {
        self.flow.mark_unknown();
    }

    /// Authorize an admitted operation that cannot publish information.
    pub fn call(&self, operation: &str) -> Result<(), IfcError> {
        match self.domain.capabilities.effect(operation) {
            Some(OperationEffect::NonEgressing) => Ok(()),
            Some(OperationEffect::Publication) => Err(IfcError::PublicationRequiresPublish),
            None => Err(IfcError::CapabilityDenied),
        }
    }

    /// Authorize an exact outbound payload for a checked broker sink.
    pub fn publish<T>(
        &self,
        request: PublicationRequest<T>,
    ) -> Result<AuthorizedPublication<T>, IfcError> {
        match self.domain.capabilities.effect(&request.operation) {
            Some(OperationEffect::Publication) => {}
            Some(OperationEffect::NonEgressing) => {
                return Err(IfcError::NonEgressingRequiresCall);
            }
            None => return Err(IfcError::CapabilityDenied),
        }

        self.flow.check_egress(&request.target.audience)?;
        Ok(AuthorizedPublication {
            operation: request.operation,
            target: request.target,
            payload: request.payload,
        })
    }
}

/// Why an IFC session refused a broker action.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IfcError {
    /// The resource could be read by principals outside the session audience.
    #[error("resource audience is not safe for this execution domain")]
    ReadAudienceDenied,
    /// The resource belongs to retained state this domain may not reuse.
    #[error("resource belongs to a different retained-state context")]
    ReadContextDenied,
    /// Restricted state was created under a different membership epoch.
    #[error("resource membership epoch does not match the execution domain")]
    StaleResourceEpoch,
    /// The execution domain does not admit the requested operation.
    #[error("operation is not admitted by this execution domain")]
    CapabilityDenied,
    /// An egressing operation was presented to the unchecked call path.
    #[error("publication operation must use IfcSession::publish")]
    PublicationRequiresPublish,
    /// A non-egressing operation was presented to the publication path.
    #[error("non-egressing operation must use IfcSession::call")]
    NonEgressingRequiresCall,
    /// Accumulated information cannot flow to the requested audience.
    #[error("information-flow check failed: {0}")]
    InformationFlow(#[from] EgressError),
}
