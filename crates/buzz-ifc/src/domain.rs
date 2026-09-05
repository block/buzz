use std::collections::BTreeSet;

use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::hash::{hash_field, short_fingerprint};
use crate::label::{ConfidentialityLabel, LabelError, Principal, ReaderSet, RealmId};

/// Which retained context a worker belongs to.
///
/// Paper: "Execution domains." Context remains separate from audience: two
/// conversations may have identical participants without implicitly sharing
/// memory.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DomainContext {
    /// Shared state for public conversations in one realm.
    RealmPublic(RealmId),
    /// State retained for one specific restricted conversation.
    Conversation {
        /// The Buzz community containing the conversation.
        realm: RealmId,
        /// The channel, DM, or group-DM identifier.
        channel_id: Uuid,
    },
    /// State visible only to the bot owner.
    OwnerPrivate {
        /// The Buzz community containing the owner relationship.
        realm: RealmId,
        /// The bot owner.
        owner: Principal,
    },
}

/// Runtime placement required by an execution domain.
///
/// The IFC rules do not implement an OS sandbox. They tell the harness whether
/// a worker may use the shared public runtime or must be placed in a compartment
/// dedicated to one complete execution domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompartmentProfile {
    /// Realm-public conversations may share a worker, public memory, and public
    /// tools. The worker must still be unable to reach broker secrets or private
    /// compartments.
    SharedPublic,
    /// A restricted conversation or owner-private task requires a worker whose
    /// writable state and output paths are confined to the exact domain.
    DomainConfined,
}

impl CompartmentProfile {
    /// Return the stable wire and log representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SharedPublic => "shared_public",
            Self::DomainConfined => "domain_confined",
        }
    }
}

impl DomainContext {
    /// Return the realm containing this context.
    pub fn realm(&self) -> &RealmId {
        match self {
            Self::RealmPublic(realm)
            | Self::Conversation { realm, .. }
            | Self::OwnerPrivate { realm, .. } => realm,
        }
    }

    /// Return a stable context category for logs and protocol responses.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::RealmPublic(_) => "public",
            Self::Conversation { .. } => "conversation",
            Self::OwnerPrivate { .. } => "owner_private",
        }
    }

    /// Whether this is the private aggregation context for `owner`.
    pub fn is_owner_private_for(&self, owner: &Principal) -> bool {
        matches!(self, Self::OwnerPrivate { owner: candidate, .. } if candidate == owner)
    }

    fn resource_context(&self) -> ResourceContext {
        match self {
            Self::RealmPublic(realm) => ResourceContext::RealmPublic(realm.clone()),
            Self::Conversation { realm, channel_id } => ResourceContext::Conversation {
                realm: realm.clone(),
                channel_id: *channel_id,
            },
            Self::OwnerPrivate { realm, owner } => ResourceContext::OwnerPrivate {
                realm: realm.clone(),
                owner: owner.clone(),
            },
        }
    }

    pub(crate) fn permits(&self, resource: &ResourceContext) -> bool {
        match resource {
            ResourceContext::TrustedConfiguration => true,
            ResourceContext::RealmPublic(resource_realm) => self.realm() == resource_realm,
            ResourceContext::Conversation {
                realm: resource_realm,
                channel_id: resource_channel,
            } => match self {
                Self::Conversation { realm, channel_id } => {
                    realm == resource_realm && channel_id == resource_channel
                }
                Self::OwnerPrivate { realm, .. } => realm == resource_realm,
                Self::RealmPublic(_) => false,
            },
            ResourceContext::OwnerPrivate {
                realm: resource_realm,
                owner: resource_owner,
            } => matches!(
                self,
                Self::OwnerPrivate { realm, owner }
                    if realm == resource_realm && owner == resource_owner
            ),
        }
    }

    fn stable_hash(&self, hasher: &mut Sha256) {
        match self {
            Self::RealmPublic(realm) => {
                hasher.update(b"realm-public");
                realm.stable_hash(hasher);
            }
            Self::Conversation { realm, channel_id } => {
                hasher.update(b"conversation");
                realm.stable_hash(hasher);
                hasher.update(channel_id.as_bytes());
            }
            Self::OwnerPrivate { realm, owner } => {
                hasher.update(b"owner-private");
                realm.stable_hash(hasher);
                hash_field(hasher, owner.0.as_bytes());
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ResourceContext {
    TrustedConfiguration,
    RealmPublic(RealmId),
    Conversation { realm: RealmId, channel_id: Uuid },
    OwnerPrivate { realm: RealmId, owner: Principal },
}

/// An operation that a worker may request.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Capability(String);

/// The complete set of operations admitted for one execution domain.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    /// Build a set from stable operation names.
    pub fn from_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(
            names
                .into_iter()
                .map(|name| Capability(name.into()))
                .collect(),
        )
    }

    /// Paper: "Broker behavior — Enforcement rules." Compute
    /// `C(bot) ∩ C(requester) ∩ C(domain)`.
    pub fn effective(bot: &Self, requester: &Self, domain: &Self) -> Self {
        let bot_and_requester: BTreeSet<_> = bot.0.intersection(&requester.0).cloned().collect();
        Self(bot_and_requester.intersection(&domain.0).cloned().collect())
    }

    /// Whether this set admits an operation.
    pub fn contains(&self, operation: &str) -> bool {
        self.0.contains(&Capability(operation.to_string()))
    }

    fn stable_hash(&self, hasher: &mut Sha256) {
        for capability in &self.0 {
            hash_field(hasher, capability.0.as_bytes());
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MembershipEpoch(String);

impl MembershipEpoch {
    /// Construct an epoch from a stable, verifier-controlled identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Return a short identifier suitable for logs.
    pub fn fingerprint(&self) -> String {
        short_fingerprint(&self.0)
    }
}

/// Buzz conversation classification after signed metadata verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationKind {
    /// Realm-wide public channel. All public channels intentionally share one
    /// execution domain.
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
/// A trusted adapter constructs this only after checking trigger signatures,
/// channel binding, and the relay signature on metadata and membership.
pub struct DomainFacts {
    /// Community realm selected by the trusted Buzz connection.
    pub realm: RealmId,
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

/// Domain derivation failed despite the adapter's claim that its facts were
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

/// Paper: "Execution domains." Derive
/// `D = (Agent, Audience, Context, Epoch, Capabilities)` from verified Buzz
/// facts.
/// This is the mapping shared by local ACP and remote agent harnesses.
pub fn derive_execution_domain(
    facts: DomainFacts,
    policy: &CapabilityPolicy,
) -> Result<ExecutionDomain, DerivationError> {
    if facts.requesters.is_empty() {
        return Err(DerivationError::EmptyRequesters);
    }

    if facts.kind == ConversationKind::Public {
        let context = DomainContext::RealmPublic(facts.realm.clone());
        let capabilities = effective_capabilities(&context, &facts, policy);
        return Ok(ExecutionDomain::public(
            facts.executing_agent,
            facts.realm,
            facts.epoch,
            capabilities,
        ));
    }

    if !facts.members.contains(&facts.executing_agent) {
        return Err(DerivationError::AgentNotMember);
    }
    if facts.requesters.iter().any(|requester| {
        facts.system_principal.as_ref() != Some(requester) && !facts.members.contains(requester)
    }) {
        return Err(DerivationError::RequesterNotMember);
    }

    let mut readers = facts.members.clone();
    readers.remove(&facts.executing_agent);
    if readers.is_empty() {
        return Err(DerivationError::EmptyRestrictedAudience);
    }

    let context = match (&facts.owner, facts.kind) {
        (Some(owner), ConversationKind::DirectMessage)
            if readers.len() == 1 && readers.contains(owner) =>
        {
            DomainContext::OwnerPrivate {
                realm: facts.realm.clone(),
                owner: owner.clone(),
            }
        }
        _ => DomainContext::Conversation {
            realm: facts.realm.clone(),
            channel_id: facts.channel_id,
        },
    };
    let capabilities = effective_capabilities(&context, &facts, policy);
    let audience = ConfidentialityLabel::restricted(facts.realm, readers)
        .map_err(|_| DerivationError::EmptyRestrictedAudience)?;
    ExecutionDomain::new(
        facts.executing_agent,
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
    /// Return a short identifier suitable for logs.
    pub fn fingerprint(&self) -> String {
        short_fingerprint(&self.0)
    }

    /// Return the full stable identifier used as a worker-pool routing key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `D = (Agent, Audience, Context, Epoch, Capabilities)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionDomain {
    agent: Principal,
    pub(crate) audience: ConfidentialityLabel,
    pub(crate) context: DomainContext,
    epoch: MembershipEpoch,
    pub(crate) capabilities: CapabilitySet,
}

impl ExecutionDomain {
    /// Construct a domain after the trusted adapter has resolved its inputs.
    pub fn new(
        agent: Principal,
        audience: ConfidentialityLabel,
        context: DomainContext,
        epoch: MembershipEpoch,
        capabilities: CapabilitySet,
    ) -> Result<Self, DomainError> {
        if audience.realm() != context.realm() {
            return Err(DomainError::ContextRealmMismatch);
        }
        match (&audience.readers, &context) {
            (ReaderSet::Everyone, DomainContext::RealmPublic(_)) => {}
            (ReaderSet::Only(readers), DomainContext::Conversation { .. })
                if !readers.is_empty() => {}
            (ReaderSet::Only(readers), DomainContext::OwnerPrivate { owner, .. })
                if readers.len() == 1 && readers.contains(owner) => {}
            _ => return Err(DomainError::AudienceContextMismatch),
        }
        Ok(Self {
            agent,
            audience,
            context,
            epoch,
            capabilities,
        })
    }

    /// Construct the realm-wide public domain.
    pub fn public(
        agent: Principal,
        realm: RealmId,
        epoch: MembershipEpoch,
        capabilities: CapabilitySet,
    ) -> Self {
        Self {
            agent,
            audience: ConfidentialityLabel::public(realm.clone()),
            context: DomainContext::RealmPublic(realm),
            epoch,
            capabilities,
        }
    }

    /// Construct the exact owner-private domain.
    pub fn owner_private(
        agent: Principal,
        realm: RealmId,
        owner: Principal,
        epoch: MembershipEpoch,
        capabilities: CapabilitySet,
    ) -> Self {
        let readers = BTreeSet::from([owner.clone()]);
        Self {
            agent,
            audience: ConfidentialityLabel {
                realm: realm.clone(),
                readers: ReaderSet::Only(readers),
            },
            context: DomainContext::OwnerPrivate { realm, owner },
            epoch,
            capabilities,
        }
    }

    /// Return the managed Buzz identity whose work this domain contains.
    pub fn agent(&self) -> &Principal {
        &self.agent
    }

    /// Return the authorized audience.
    pub fn audience(&self) -> &ConfidentialityLabel {
        &self.audience
    }

    /// Return the retained-state context.
    pub fn context(&self) -> &DomainContext {
        &self.context
    }

    /// Return the runtime placement required to preserve this domain.
    pub fn compartment_profile(&self) -> CompartmentProfile {
        if self.audience.is_public() {
            CompartmentProfile::SharedPublic
        } else {
            CompartmentProfile::DomainConfined
        }
    }

    /// Return the effective capability set.
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Return a short fingerprint of the membership or policy epoch.
    pub fn epoch_fingerprint(&self) -> String {
        self.epoch.fingerprint()
    }

    /// Return the canonical identifier for the complete domain tuple.
    pub fn id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"buzz-ifc-domain-v1");
        hash_field(&mut hasher, self.agent.0.as_bytes());
        self.audience.realm.stable_hash(&mut hasher);
        self.audience.readers.stable_hash(&mut hasher);
        self.context.stable_hash(&mut hasher);
        hash_field(&mut hasher, self.epoch.0.as_bytes());
        self.capabilities.stable_hash(&mut hasher);
        hex::encode(hasher.finalize())
    }

    /// Return the opaque worker-pool routing key.
    pub fn key(&self) -> DomainKey {
        DomainKey(self.id())
    }

    /// Label information whose provenance is this domain itself.
    pub fn resource_label(&self) -> ResourceLabel {
        ResourceLabel {
            confidentiality: self.audience.clone(),
            context: self.context.resource_context(),
        }
    }
}

/// An execution domain contains inconsistent realms.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DomainError {
    /// The audience and retained context belong to different Buzz realms.
    #[error("execution-domain audience and context belong to different realms")]
    ContextRealmMismatch,
    /// Public, conversation, and owner-private contexts require their
    /// corresponding audience shape.
    #[error("execution-domain audience does not match its context")]
    AudienceContextMismatch,
}

/// The confidentiality and context assigned to one input resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceLabel {
    pub(crate) confidentiality: ConfidentialityLabel,
    pub(crate) context: ResourceContext,
}

impl ResourceLabel {
    /// Label immutable configuration that is public in the supplied realm.
    pub fn trusted_configuration(realm: RealmId) -> Self {
        Self {
            confidentiality: ConfidentialityLabel::public(realm),
            context: ResourceContext::TrustedConfiguration,
        }
    }

    /// Label information already scoped to an execution domain.
    pub fn domain(domain: &ExecutionDomain) -> Self {
        domain.resource_label()
    }

    /// Label information public to every member of one realm.
    pub fn realm_public(realm: RealmId) -> Self {
        Self {
            confidentiality: ConfidentialityLabel::public(realm.clone()),
            context: ResourceContext::RealmPublic(realm),
        }
    }

    /// Label information belonging to one restricted conversation.
    pub fn conversation(
        realm: RealmId,
        channel_id: Uuid,
        readers: BTreeSet<Principal>,
    ) -> Result<Self, LabelError> {
        Ok(Self {
            confidentiality: ConfidentialityLabel::restricted(realm.clone(), readers)?,
            context: ResourceContext::Conversation { realm, channel_id },
        })
    }

    /// Label owner-private information such as personal memory.
    pub fn owner_private(realm: RealmId, owner: Principal) -> Self {
        let readers = BTreeSet::from([owner.clone()]);
        Self {
            confidentiality: ConfidentialityLabel {
                realm: realm.clone(),
                readers: ReaderSet::Only(readers),
            },
            context: ResourceContext::OwnerPrivate { realm, owner },
        }
    }
}
