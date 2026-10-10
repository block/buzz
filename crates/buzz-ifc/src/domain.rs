use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use crate::label::{CommunityId, ConfidentialityLabel, Principal, ReaderSet};
use crate::session::ResourceLabel;

/// Which conversations may share an agent's history, files, and caches.
///
/// Two private channels keep separate state even if they have the same members.
/// Public channels have a community-wide policy context. The broker keeps
/// conversation histories separate even when their domains match. A DM between
/// an agent and its owner has a separate owner-private policy context.
///
/// A matching context is not enough to reuse state. The broker must compare the
/// full [`ExecutionDomain`], so a change in agent, owner, audience, or
/// capabilities also prevents reuse.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum DomainContext {
    /// Shared state for public conversations in one community.
    CommunityPublic(CommunityId),
    /// State kept for one private channel or DM.
    Conversation {
        /// The community the conversation belongs to.
        community: CommunityId,
        /// The channel, DM, or group-DM identifier.
        channel_id: Uuid,
    },
    /// State kept for conversations between the agent and its owner.
    OwnerPrivate {
        /// The community where this agent has this owner.
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
}

/// Whether an operation can send information out of the agent's environment.
///
/// The broker configures this; the agent does not get to classify its own calls.
/// For a publication, permission to call the operation is not enough: the broker
/// must use [`crate::IfcSession::publish`] to check whether the information may
/// flow to the destination's readers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OperationEffect {
    /// Does not expose information outside the agent's environment.
    NonEgressing,
    /// Sends information to a destination the broker must resolve and check.
    Publication,
}

impl OperationEffect {
    fn most_restrictive(self, other: Self) -> Self {
        match (self, other) {
            (Self::NonEgressing, Self::NonEgressing) => Self::NonEgressing,
            (Self::Publication, _) | (_, Self::Publication) => Self::Publication,
        }
    }
}

/// Operations a policy allows, with each operation's publication behavior.
///
/// An operation's presence in this set does not authorize a call on its own.
/// Publications still need an information-flow check, so there is no public
/// membership check that a caller could mistake for permission to execute:
///
/// ```compile_fail
/// let capabilities = buzz_ifc::CapabilitySet::default();
/// let _ = capabilities.contains("buzz.post");
/// ```
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct CapabilitySet(BTreeMap<String, OperationEffect>);

impl CapabilitySet {
    /// Build a set from stable operation names and broker-configured effects.
    ///
    /// If a name appears more than once, keep `Publication` if any entry uses it.
    /// A duplicate must not remove the requirement to check the destination.
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

    /// Keep only operations allowed by the bot, requester, and domain policies.
    ///
    /// All three must list an operation for it to survive. If any marks it as a
    /// publication, the result does too, even if the others mark it non-egressing.
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
}

/// Limits on the operations available to an agent.
///
/// `bot` lists everything the agent may use. An owner-private DM uses this set
/// when every request comes from the owner. All other work is limited to
/// operations that also appear in `conversation`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityPolicy {
    bot: CapabilitySet,
    conversation: CapabilitySet,
}

impl CapabilityPolicy {
    /// Set the bot's allowed operations and the narrower set for shared
    /// conversations. Entries in `conversation` cannot grant anything absent
    /// from `bot`.
    pub fn new(bot: CapabilitySet, conversation: CapabilitySet) -> Self {
        Self { bot, conversation }
    }
}

/// Conversation type read from metadata the broker has verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationKind {
    /// A public channel, using the community's shared public context.
    Public,
    /// A private channel or group DM that keeps its own state.
    Restricted,
    /// A DM. If its only members are the agent and its owner, use owner-private
    /// context. Otherwise, keep state specific to this DM.
    DirectMessage,
}

/// Inputs the broker must authenticate before deriving an execution domain.
///
/// Before constructing this value, the broker must verify the triggering
/// events' signatures and channel IDs, and the relay signatures on conversation
/// metadata and membership. The fields are public; constructing this struct
/// does not perform those checks.
pub struct DomainFacts {
    /// Community resolved by the broker, not chosen by the agent.
    pub community: CommunityId,
    /// Channel, DM, or group-DM identifier that triggered the invocation.
    pub channel_id: Uuid,
    /// Conversation type from verified metadata.
    pub kind: ConversationKind,
    /// Complete verified member list, including the agent. Ignored for public
    /// channels, whose audience is the whole community.
    pub members: BTreeSet<Principal>,
    /// Agent processing the invocation. Removed from the member list when
    /// deriving the audience: processing data does not make the agent a recipient.
    pub executing_agent: Principal,
    /// Authors of the events being processed in this invocation.
    pub requesters: BTreeSet<Principal>,
    /// Relay identity that may trigger workflows without being a channel member.
    /// This exception does not add it to the audience or grant owner rights.
    pub system_principal: Option<Principal>,
    /// Owner of the executing agent, if one is configured.
    pub owner: Option<Principal>,
}

/// The supplied facts cannot form a valid execution domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DerivationError {
    /// No authenticated requester was supplied.
    #[error("invocation has no authenticated requester")]
    EmptyRequesters,
    /// The executing agent is missing from the private conversation's member list.
    #[error("executing agent is absent from channel membership")]
    AgentNotMember,
    /// A requester other than the trusted relay identity is not a member.
    #[error("requester is absent from channel membership")]
    RequesterNotMember,
    /// No recipients remain after removing the executing agent from the members.
    #[error("restricted conversation has no recipient audience")]
    EmptyRestrictedAudience,
    /// The audience and context disagree on the community or permitted readers.
    #[error("derived execution domain is inconsistent")]
    InvalidDomain,
}

/// Choose the audience, saved-state context, and allowed operations for a turn.
///
/// Public channels use the whole community as their audience and share a public
/// context. Private channels and DMs use their members, minus the executing
/// agent, as readers and keep state per conversation. A DM containing only the
/// agent and its owner uses owner-private context instead.
///
/// When the context is owner-private and every requester is the owner, use the
/// bot's capability set. Otherwise, keep only operations allowed by both the bot
/// and conversation policies, even if the owner made the request.
///
/// The broker must supply verified [`DomainFacts`] and use the resulting
/// [`ExecutionDomain`] to select saved state. This function does not load or
/// clear sessions itself.
pub fn derive_execution_domain(
    facts: DomainFacts,
    policy: &CapabilityPolicy,
) -> Result<ExecutionDomain, DerivationError> {
    if facts.requesters.is_empty() {
        return Err(DerivationError::EmptyRequesters);
    }

    match facts.kind {
        ConversationKind::Public => {}
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
        }
    }
    // Use the same conversation classification as raw resource labels, with
    // output recipients excluding the executing agent.
    let ResourceLabel {
        audience,
        mut context,
    } = ResourceLabel::from_conversation(
        facts.community,
        facts.channel_id,
        facts.kind,
        facts
            .members
            .iter()
            .copied()
            .filter(|member| *member != facts.executing_agent),
    )
    .map_err(|_| DerivationError::EmptyRestrictedAudience)?;
    if let (Some(owner), ReaderSet::Only(readers)) = (&facts.owner, audience.reader_set()) {
        if facts.kind == ConversationKind::DirectMessage
            && readers.len() == 1
            && readers.contains(owner)
        {
            context = DomainContext::OwnerPrivate {
                community: facts.community,
                owner: *owner,
            };
        }
    }
    let capabilities = effective_capabilities(&context, &facts, policy);
    ExecutionDomain::new(
        facts.executing_agent,
        facts.owner,
        audience,
        context,
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

/// The identity, readers, context, and policy for an agent invocation.
///
/// Conceptually, `D = (community, agent, owner, audience, context, capabilities)`.
/// The community is encoded in both the audience and context; the constructor
/// requires them to agree. The audience says who may receive information, the
/// context says which conversations may share saved state, and capabilities
/// list the allowed operations. Agent and owner identities keep different
/// agents and ownership relationships separate.
///
/// Restricted audiences contain the verified member set minus the executing
/// agent. Public audiences represent everyone in the community without listing
/// members. Event IDs, timestamps, names, and topics do not participate in
/// equality. Compare the complete structure before reusing retained state.
///
/// This represents current policy state. Unlike the temporal epochs in
/// [Appendix B of the design paper](../../../docs/practical-information-flow-for-buzz-agents.md#appendix-b-formal-execution-domains)
/// it does not distinguish two occurrences of the same policy separated by a
/// membership change. The broker must verify current access separately and
/// preserve session restrictions when reusing retained state.
/// Conversation identity and lifecycle generation are additional broker
/// boundaries: equal public domains do not merge model histories, and equality
/// does not revoke a session when membership leaves and returns to the same set.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ExecutionDomain {
    agent: Principal,
    owner: Option<Principal>,
    pub(crate) audience: ConfidentialityLabel,
    pub(crate) context: DomainContext,
    pub(crate) capabilities: CapabilitySet,
}

impl ExecutionDomain {
    /// Check that the audience and context agree on community and readers.
    pub(crate) fn new(
        agent: Principal,
        owner: Option<Principal>,
        audience: ConfidentialityLabel,
        context: DomainContext,
        capabilities: CapabilitySet,
    ) -> Result<Self, DomainError> {
        if *audience.universe() != context.community() {
            return Err(DomainError::ContextCommunityMismatch);
        }
        if !audience_matches_context(&audience, &context) {
            return Err(DomainError::AudienceContextMismatch);
        }
        Ok(Self {
            agent,
            owner,
            audience,
            context,
            capabilities,
        })
    }

    /// Return the label describing who may receive this domain's output.
    pub fn audience(&self) -> &ConfidentialityLabel {
        &self.audience
    }
}

fn audience_matches_context(audience: &ConfidentialityLabel, context: &DomainContext) -> bool {
    match (audience.reader_set(), context) {
        (ReaderSet::Everyone, DomainContext::CommunityPublic(_)) => true,
        (ReaderSet::Only(readers), DomainContext::Conversation { .. }) => !readers.is_empty(),
        (ReaderSet::Only(readers), DomainContext::OwnerPrivate { owner, .. }) => {
            readers.len() == 1 && readers.contains(owner)
        }
        _ => false,
    }
}

/// The audience and context cannot be used together.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DomainError {
    /// The audience and context belong to different communities.
    #[error("execution-domain audience and context belong to different communities")]
    ContextCommunityMismatch,
    /// Public context needs a public audience; conversation context needs a
    /// nonempty reader set; owner-private context needs exactly the owner.
    #[error("execution-domain audience does not match its context")]
    AudienceContextMismatch,
}
