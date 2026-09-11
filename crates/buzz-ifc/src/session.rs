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

/// An outbound request that passed the session's capability and audience checks.
///
/// The request owns its serialized bytes and destination label. Neither can
/// change through a shared reference after authorization. A broker sink should
/// accept this type and execute those bytes, without substituting a new payload
/// or resolving destination fields from mutable agent state.
///
/// Only [`IfcSession::publish`] can construct this value. It cannot be cloned.
///
/// ```compile_fail
/// # fn forge(destination: buzz_ifc::ConfidentialityLabel) {
/// let forged = buzz_ifc::AuthorizedPublication {
///     operation: "buzz.reply".to_owned(),
///     destination,
///     payload: b"unchecked".to_vec(),
/// };
/// # }
/// ```
///
/// Payload access is read-only until the sink consumes the authorization:
///
/// ```compile_fail
/// # fn change(mut authorization: buzz_ifc::AuthorizedPublication) {
/// authorization.payload()[0] = b'!';
/// # }
/// ```
#[must_use = "the authorization must be consumed by the publication sink"]
pub struct AuthorizedPublication {
    operation: String,
    destination: ConfidentialityLabel,
    payload: Vec<u8>,
}

impl AuthorizedPublication {
    /// Return the checked operation name.
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// Return the exact payload covered by the information-flow decision.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Consume the authorization and return the checked sink inputs.
    pub fn into_parts(self) -> (String, ConfidentialityLabel, Vec<u8>) {
        (self.operation, self.destination, self.payload)
    }
}

/// Information-flow state for one retained agent execution domain.
///
/// The broker calls [`read`](Self::read) before delivering data to the agent,
/// [`call`](Self::call) before executing operations that cannot publish, and
/// [`publish`](Self::publish) before handing an exact outbound payload to a
/// sink. The audience checks follow the flow ordering in
/// [Appendix G of the design paper](../../../docs/practical-information-flow-for-buzz-agents.md#appendix-g-security-labels-as-a-lattice).
///
/// The broker keeps this value for as long as it keeps the agent's history,
/// files, or other state. The same domain key selects both. This example uses a
/// broker-owned pool so a later turn cannot reset the session's restrictions:
///
/// ```
/// # use std::collections::HashMap;
/// # use buzz_ifc::{AuthorizedPublication, ConfidentialityLabel, ExecutionDomain,
/// #     DomainKey, IfcError, IfcSession, ResourceLabel};
/// # fn broker_sink(_: AuthorizedPublication) {}
/// # fn run_turn(
/// #     sessions: &mut HashMap<DomainKey, IfcSession>,
/// #     domain: ExecutionDomain,
/// #     resource: &ResourceLabel,
/// #     destination: &ConfidentialityLabel,
/// #     request_bytes: Vec<u8>,
/// # ) -> Result<(), IfcError> {
/// let session = sessions
///     .entry(domain.key())
///     .or_insert_with(|| IfcSession::enter(domain));
/// session.call("buzz.read.current")?;
/// session.read(resource)?;
///
/// let authorization = session.publish("buzz.reply", destination, request_bytes)?;
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
    ///
    /// Do not replace an existing session while retaining the agent's state.
    /// A fresh session would forget any earlier [`Self::mark_unknown_input`].
    pub fn enter(domain: ExecutionDomain) -> Self {
        let mut flow = FlowState::default();
        flow.observe(&domain.audience);
        Self { domain, flow }
    }

    /// Return the key the broker uses to route later turns to this session.
    pub fn domain_key(&self) -> DomainKey {
        self.domain.key()
    }

    /// Check a labeled resource before exposing it to the agent.
    ///
    /// The broker must not deliver a rejected resource. Every admitted resource
    /// is readable by the domain's entire audience, so reading it does not
    /// further restrict output: `enter` already applied that audience.
    /// The broker must check current resource permissions separately; matching
    /// stored membership epochs does not prove that membership is still current.
    pub fn read(&self, resource: &ResourceLabel) -> Result<(), IfcError> {
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

        Ok(())
    }

    /// Permanently record that unlabeled input reached the agent.
    ///
    /// No new publication can be authorized for the rest of the session.
    /// Already authorized requests still contain only their earlier, frozen
    /// bytes. They cannot be updated to include the unknown input.
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
    ///
    /// The broker must serialize the complete request, including its concrete
    /// destination, and resolve `destination` from that request before calling
    /// this method. The sink must execute the returned bytes as checked. This
    /// method does not parse the request or check current destination policy.
    ///
    /// Owned bytes prevent a caller from changing the payload through a shared
    /// mutable value after it passes the checks:
    ///
    /// ```compile_fail
    /// # use buzz_ifc::{ConfidentialityLabel, IfcSession};
    /// # use std::{cell::RefCell, rc::Rc};
    /// # fn publish_shared(session: &IfcSession, destination: &ConfidentialityLabel) {
    /// let payload = Rc::new(RefCell::new(b"hello".to_vec()));
    /// session.publish("buzz.reply", destination, payload);
    /// # }
    /// ```
    pub fn publish(
        &self,
        operation: &str,
        destination: &ConfidentialityLabel,
        payload: Vec<u8>,
    ) -> Result<AuthorizedPublication, IfcError> {
        match self.domain.capabilities.effect(operation) {
            Some(OperationEffect::Publication) => {}
            Some(OperationEffect::NonEgressing) => {
                return Err(IfcError::NonEgressingRequiresCall);
            }
            None => return Err(IfcError::CapabilityDenied),
        }

        self.flow.check_egress(destination)?;
        Ok(AuthorizedPublication {
            operation: operation.to_owned(),
            destination: destination.clone(),
            payload,
        })
    }
}

/// Why an IFC session refused a broker action.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IfcError {
    /// Some session readers are not allowed to read the resource.
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
