use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::label::{CommunityId, ConfidentialityLabel, Principal, ReaderSet};

/// Defines where an agent's retained state may be reused.
///
/// After a turn, an agent may retain conversation history, files, or cached
/// data. Changing this context must select different retained state. Matching
/// contexts alone do not permit reuse: the broker must match the complete
/// [`DomainKey`], which also covers the agent, audience, membership epoch, and
/// capabilities.
///
/// Audience answers who may read information; context answers which
/// conversation history and managed memory may carry into later turns. These
/// are independent boundaries. Two conversations with identical participants
/// do not implicitly share state, while public conversations deliberately use
/// one community-wide public context. Owner-private state is likewise distinct
/// from ordinary conversation state, even within the same community.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum DomainContext {
    /// Shared state for public conversations in one community.
    CommunityPublic(CommunityId),
    /// State retained for one specific restricted conversation.
    Conversation {
        /// The Buzz community containing the conversation.
        community: CommunityId,
        /// The channel, DM, or group-DM identifier.
        channel_id: Uuid,
    },
    /// State visible only to the bot owner.
    OwnerPrivate {
        /// The Buzz community containing the owner relationship.
        community: CommunityId,
        /// The bot owner.
        owner: Principal,
    },
}

impl DomainContext {
    /// Return the community containing this context.
    fn community(&self) -> CommunityId {
        match self {
            Self::CommunityPublic(community)
            | Self::Conversation { community, .. }
            | Self::OwnerPrivate { community, .. } => *community,
        }
    }

    /// Whether this retained-state context may admit a resource from `source`.
    ///
    /// Public community data may enter any context in that community. A
    /// conversation admits only its own retained data. Owner-private work may
    /// also narrow conversation data to the owner when the separate audience
    /// check permits that flow; it never admits another owner's private state.
    pub(crate) fn permits(&self, source: &Self) -> bool {
        if self.community() != source.community() {
            return false;
        }

        match source {
            Self::CommunityPublic(_) => true,
            Self::Conversation { .. } => {
                self == source || matches!(self, Self::OwnerPrivate { .. })
            }
            Self::OwnerPrivate { .. } => self == source,
        }
    }

    pub(crate) fn stable_hash(&self, hasher: &mut Sha256) {
        match self {
            Self::CommunityPublic(community) => {
                hash_field(hasher, b"community-public");
                hash_field(hasher, community.as_uuid().as_bytes());
            }
            Self::Conversation {
                community,
                channel_id,
            } => {
                hash_field(hasher, b"conversation");
                hash_field(hasher, community.as_uuid().as_bytes());
                hash_field(hasher, channel_id.as_bytes());
            }
            Self::OwnerPrivate { community, owner } => {
                hash_field(hasher, b"owner-private");
                hash_field(hasher, community.as_uuid().as_bytes());
                hash_field(hasher, &owner.to_bytes());
            }
        }
    }
}

/// Whether invoking an operation can publish information outside the current
/// execution boundary.
///
/// This classification belongs to trusted policy configuration, not to the
/// agent's call request. Publication operations must go through
/// [`crate::IfcSession::publish`], where the destination receives an
/// information-flow check before the broker may execute the operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationEffect {
    /// The operation cannot publish information outside the execution boundary.
    NonEgressing,
    /// The operation publishes information to a broker-resolved destination.
    Publication,
}

impl OperationEffect {
    fn most_restrictive(self, other: Self) -> Self {
        match (self, other) {
            (Self::NonEgressing, Self::NonEgressing) => Self::NonEgressing,
            (Self::Publication, _) | (_, Self::Publication) => Self::Publication,
        }
    }

    fn stable_hash(self, hasher: &mut Sha256) {
        match self {
            Self::NonEgressing => hash_field(hasher, b"non-egressing"),
            Self::Publication => hash_field(hasher, b"publication"),
        }
    }
}

/// The complete set of operations admitted for one execution domain.
///
/// Raw membership is intentionally private because it is not an authorization
/// decision:
///
/// ```compile_fail
/// let capabilities = buzz_ifc::CapabilitySet::default();
/// let _ = capabilities.contains("buzz.post");
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet(BTreeMap<String, OperationEffect>);

impl CapabilitySet {
    /// Build a set from stable operation names and trusted effect classes.
    ///
    /// If an operation is repeated with different effects, publication wins so
    /// conflicting configuration cannot downgrade an egressing operation.
    pub fn from_operations<I, S>(operations: I) -> Self
    where
        I: IntoIterator<Item = (S, OperationEffect)>,
        S: Into<String>,
    {
        let mut capabilities: BTreeMap<String, OperationEffect> = BTreeMap::new();
        for (name, effect) in operations {
            capabilities
                .entry(name.into())
                .and_modify(|existing| *existing = existing.most_restrictive(effect))
                .or_insert(effect);
        }
        Self(capabilities)
    }

    /// Compute the operations admitted by every independent capability
    /// ceiling: the bot, the authenticated requester, and the execution
    /// domain.
    ///
    /// An operation missing from any ceiling is denied. When the same
    /// operation has conflicting effect classifications, the result preserves
    /// the classification that requires more checking: publication wins over
    /// non-egressing. No policy layer can accidentally downgrade an egressing
    /// operation into an unchecked call.
    pub(crate) fn effective(bot: &Self, requester: &Self, domain: &Self) -> Self {
        let mut effective = BTreeMap::new();
        for (name, bot_effect) in &bot.0 {
            let (Some(requester_effect), Some(domain_effect)) =
                (requester.0.get(name), domain.0.get(name))
            else {
                continue;
            };
            effective.insert(
                name.clone(),
                bot_effect
                    .most_restrictive(*requester_effect)
                    .most_restrictive(*domain_effect),
            );
        }
        Self(effective)
    }

    pub(crate) fn effect(&self, operation: &str) -> Option<OperationEffect> {
        self.0.get(operation).copied()
    }

    fn stable_hash(&self, hasher: &mut Sha256) {
        hash_field(hasher, &(self.0.len() as u64).to_be_bytes());
        for (name, effect) in &self.0 {
            hash_field(hasher, name.as_bytes());
            effect.stable_hash(hasher);
        }
    }
}

/// Capability ceilings used while deriving an invocation's effective set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityPolicy {
    bot: CapabilitySet,
    conversation: CapabilitySet,
}

impl CapabilityPolicy {
    /// Construct policy from the bot's full ceiling and the ceiling permitted
    /// in shared Buzz conversations.
    pub fn new(bot: CapabilitySet, conversation: CapabilitySet) -> Self {
        Self { bot, conversation }
    }
}

/// The membership or policy version under which state was created.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct MembershipEpoch(String);

impl MembershipEpoch {
    /// Construct an epoch from a stable, verifier-controlled identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

/// Buzz conversation classification after signed metadata verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationKind {
    /// Community-wide public channel. All public channels intentionally share
    /// one execution domain.
    Public,
    /// Invite-only channel or group DM with conversation-specific state.
    Restricted,
    /// A DM. A two-party owner/bot DM becomes owner-private; group DMs remain
    /// conversation-specific.
    DirectMessage,
}

/// Verified Buzz facts from which the shared policy derives an execution
/// domain.
///
/// The trusted broker constructs this only after checking trigger signatures,
/// channel binding, and the relay signature on metadata and membership.
pub struct DomainFacts {
    /// Community selected by the trusted broker's resolved tenant context.
    pub community: CommunityId,
    /// Channel, DM, or group-DM identifier that triggered the invocation.
    pub channel_id: Uuid,
    /// Verified conversation classification.
    pub kind: ConversationKind,
    /// Relay-controlled membership or community policy version.
    pub epoch: MembershipEpoch,
    /// Complete verified roster. Public derivation does not consume this set.
    pub members: BTreeSet<Principal>,
    /// Managed Buzz identity whose work the execution domain contains.
    pub executing_agent: Principal,
    /// Authors whose events are included in this invocation.
    pub requesters: BTreeSet<Principal>,
    /// Optional relay principal allowed to author trusted workflow events.
    pub system_principal: Option<Principal>,
    /// Optional human owner of the executing agent.
    pub owner: Option<Principal>,
}

/// Domain derivation failed despite the broker's claim that its facts were
/// already verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DerivationError {
    /// Every invocation must contain at least one authenticated requester.
    #[error("invocation has no authenticated requester")]
    EmptyRequesters,
    /// Restricted conversations must include the executing agent in their
    /// verified roster.
    #[error("executing agent is absent from channel membership")]
    AgentNotMember,
    /// A non-system requester is absent from restricted membership.
    #[error("requester is absent from channel membership")]
    RequesterNotMember,
    /// Removing the executing processor left no authorized recipient.
    #[error("restricted conversation has no recipient audience")]
    EmptyRestrictedAudience,
    /// The derived audience and context violated a domain invariant.
    #[error("derived execution domain is inconsistent")]
    InvalidDomain,
}

/// Derive a complete execution domain from verified Buzz facts.
///
/// The trusted broker supplies authenticated requesters, verified conversation
/// membership, and a relay-controlled membership epoch; the agent cannot
/// assert any of these values. Public conversations receive the community-wide
/// audience and shared public context. Restricted conversations receive the
/// verified member audience, excluding the executing agent, and retain state
/// only for their channel. A two-party owner/agent DM instead receives the
/// owner's private context. Effective capabilities are the intersection of
/// the bot, requester, and context ceilings.
///
/// Agent identity, owner, audience, context, epoch, and capabilities all feed
/// the domain identifier used to select retained agent state. A change to any
/// component therefore selects a different domain rather than silently reusing
/// state that may contain information admitted under older authority. A broker
/// can use the resulting [`DomainKey`] when it selects an ACP or remote-agent
/// session.
pub fn derive_execution_domain(
    facts: DomainFacts,
    policy: &CapabilityPolicy,
) -> Result<ExecutionDomain, DerivationError> {
    if facts.requesters.is_empty() {
        return Err(DerivationError::EmptyRequesters);
    }

    let (audience, context) = match facts.kind {
        ConversationKind::Public => (
            ConfidentialityLabel::public(facts.community),
            DomainContext::CommunityPublic(facts.community),
        ),
        ConversationKind::Restricted | ConversationKind::DirectMessage => {
            if !facts.members.contains(&facts.executing_agent) {
                return Err(DerivationError::AgentNotMember);
            }
            if facts.requesters.iter().any(|requester| {
                facts.system_principal.as_ref() != Some(requester)
                    && !facts.members.contains(requester)
            }) {
                return Err(DerivationError::RequesterNotMember);
            }

            let mut readers = facts.members.clone();
            readers.remove(&facts.executing_agent);
            let context = match &facts.owner {
                Some(owner)
                    if facts.kind == ConversationKind::DirectMessage
                        && readers.len() == 1
                        && readers.contains(owner) =>
                {
                    DomainContext::OwnerPrivate {
                        community: facts.community,
                        owner: *owner,
                    }
                }
                _ => DomainContext::Conversation {
                    community: facts.community,
                    channel_id: facts.channel_id,
                },
            };
            let audience = ConfidentialityLabel::restricted(facts.community, readers)
                .map_err(|_| DerivationError::EmptyRestrictedAudience)?;
            (audience, context)
        }
    };
    let capabilities = effective_capabilities(&context, &facts, policy);
    ExecutionDomain::new(
        facts.executing_agent,
        facts.owner,
        audience,
        context,
        facts.epoch,
        capabilities,
    )
    .map_err(|_| DerivationError::InvalidDomain)
}

fn effective_capabilities(
    context: &DomainContext,
    facts: &DomainFacts,
    policy: &CapabilityPolicy,
) -> CapabilitySet {
    let requester_is_owner = facts
        .owner
        .as_ref()
        .is_some_and(|owner| facts.requesters.iter().all(|requester| requester == owner));
    let requester = if requester_is_owner {
        &policy.bot
    } else {
        &policy.conversation
    };
    let domain = if matches!(context, DomainContext::OwnerPrivate { .. }) {
        &policy.bot
    } else {
        &policy.conversation
    };
    CapabilitySet::effective(&policy.bot, requester, domain)
}

/// Opaque routing key for one complete execution domain.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DomainKey(String);

impl DomainKey {
    /// Return the full stable identifier used to route turns to retained state.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `D = (Agent, Owner, Audience, Context, Epoch, Capabilities)`.
///
/// This is the executable form of the domain model in [Appendix B of the
/// design paper](../../../docs/practical-information-flow-for-buzz-agents.md#appendix-b-formal-execution-domains).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionDomain {
    agent: Principal,
    owner: Option<Principal>,
    pub(crate) audience: ConfidentialityLabel,
    pub(crate) context: DomainContext,
    pub(crate) epoch: MembershipEpoch,
    pub(crate) capabilities: CapabilitySet,
}

impl ExecutionDomain {
    /// Construct a domain after the trusted broker has resolved its inputs.
    pub(crate) fn new(
        agent: Principal,
        owner: Option<Principal>,
        audience: ConfidentialityLabel,
        context: DomainContext,
        epoch: MembershipEpoch,
        capabilities: CapabilitySet,
    ) -> Result<Self, DomainError> {
        if *audience.universe() != context.community() {
            return Err(DomainError::ContextCommunityMismatch);
        }
        if !audience_context_shape_matches(&audience, &context) {
            return Err(DomainError::AudienceContextMismatch);
        }
        Ok(Self {
            agent,
            owner,
            audience,
            context,
            epoch,
            capabilities,
        })
    }

    /// Return the opaque key used to route turns to retained state.
    pub fn key(&self) -> DomainKey {
        let mut hasher = Sha256::new();
        hasher.update(b"buzz-ifc-domain-v6");
        hash_field(&mut hasher, &self.agent.to_bytes());
        match &self.owner {
            Some(owner) => {
                hash_field(&mut hasher, b"owner");
                hash_field(&mut hasher, &owner.to_bytes());
            }
            None => hash_field(&mut hasher, b"no-owner"),
        }
        hash_field(&mut hasher, self.audience.universe().as_uuid().as_bytes());
        hash_reader_set(self.audience.reader_set(), &mut hasher);
        self.context.stable_hash(&mut hasher);
        hash_field(&mut hasher, self.epoch.0.as_bytes());
        self.capabilities.stable_hash(&mut hasher);
        DomainKey(hex::encode(hasher.finalize()))
    }
}

fn hash_reader_set(readers: &ReaderSet, hasher: &mut Sha256) {
    match readers {
        ReaderSet::Everyone => hash_field(hasher, b"everyone"),
        ReaderSet::Only(readers) => {
            hash_field(hasher, b"only");
            hash_field(hasher, &(readers.len() as u64).to_be_bytes());
            for reader in readers {
                hash_field(hasher, &reader.to_bytes());
            }
        }
    }
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).unwrap_or(u64::MAX);
    hasher.update(length.to_be_bytes());
    hasher.update(value);
}

fn audience_context_shape_matches(
    audience: &ConfidentialityLabel,
    context: &DomainContext,
) -> bool {
    match (audience.reader_set(), context) {
        (ReaderSet::Everyone, DomainContext::CommunityPublic(_)) => true,
        (ReaderSet::Only(readers), DomainContext::Conversation { .. }) => !readers.is_empty(),
        (ReaderSet::Only(readers), DomainContext::OwnerPrivate { owner, .. }) => {
            readers.len() == 1 && readers.contains(owner)
        }
        _ => false,
    }
}

/// An execution domain contains inconsistent communities.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DomainError {
    /// The audience and retained context belong to different Buzz communities.
    #[error("execution-domain audience and context belong to different communities")]
    ContextCommunityMismatch,
    /// Public, conversation, and owner-private contexts require their
    /// corresponding audience shape.
    #[error("execution-domain audience does not match its context")]
    AudienceContextMismatch,
}
