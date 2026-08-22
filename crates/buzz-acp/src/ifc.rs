//! Coarse information-flow auditing for audience-scoped agent execution.
//!
//! This module is deliberately isolated from ACP transport and prompt formatting. In
//! `off` mode it is not constructed at all. In `audit` mode it observes the data that
//! the existing harness admits, evaluates the proposal's rules, and emits structured
//! decisions without changing runtime behavior.
//!
//! Comments prefixed with "Paper" refer to the matching section in the
//! "Practical information-flow for Buzz agents" design paper.

use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;

use nostr::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::queue::FlushBatch;
use crate::relay::RestClient;

/// A Buzz principal represented by a validated, normalized Nostr public key.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Principal(String);

impl Principal {
    fn from_hex(value: &str) -> Result<Self, ResolutionError> {
        PublicKey::from_hex(value)
            .map(|key| Self(key.to_hex().to_ascii_lowercase()))
            .map_err(|_| ResolutionError::InvalidPrincipal)
    }

    fn from_key(value: PublicKey) -> Self {
        Self(value.to_hex().to_ascii_lowercase())
    }
}

/// A confidentiality universe. Public in one Buzz community is not public in
/// another, so every reader-set lattice is scoped to a relay/community realm.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RealmId([u8; 32]);

impl RealmId {
    fn from_relay_url(relay_url: &str) -> Self {
        Self(Sha256::digest(relay_url.as_bytes()).into())
    }

    fn fingerprint(&self) -> String {
        hex::encode(&self.0[..6])
    }
}

/// The authorized readers of a value.
///
/// Paper: "Labels and ordering." `Everyone` is the bottom/public element. An
/// explicit set becomes more restrictive as principals are removed.
#[derive(Clone, Debug, Eq, PartialEq)]
enum ReaderSet {
    Everyone,
    Only(BTreeSet<Principal>),
}

impl ReaderSet {
    /// The paper's square ordering, `source ⊑ destination`, is reverse set
    /// inclusion: every reader at the destination must be able to read source.
    fn can_flow_to(&self, destination: &Self) -> bool {
        match (self, destination) {
            (Self::Everyone, _) => true,
            (Self::Only(_), Self::Everyone) => false,
            (Self::Only(source), Self::Only(destination)) => destination.is_subset(source),
        }
    }

    /// Paper: "Combining information." Join is reader-set intersection, which
    /// gives a result influenced by both inputs the stricter combined label.
    fn join(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Everyone, value) | (value, Self::Everyone) => value.clone(),
            (Self::Only(left), Self::Only(right)) => {
                Self::Only(left.intersection(right).cloned().collect())
            }
        }
    }

    /// The lattice meet is reader-set union.
    #[cfg(test)]
    fn meet(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Everyone, _) | (_, Self::Everyone) => Self::Everyone,
            (Self::Only(left), Self::Only(right)) => {
                Self::Only(left.union(right).cloned().collect())
            }
        }
    }

    fn explicit_count(&self) -> Option<usize> {
        match self {
            Self::Everyone => None,
            Self::Only(readers) => Some(readers.len()),
        }
    }

    fn stable_hash(&self, hasher: &mut Sha256) {
        match self {
            Self::Everyone => hasher.update(b"everyone"),
            Self::Only(readers) => {
                hasher.update(b"only");
                for reader in readers {
                    hash_field(hasher, reader.0.as_bytes());
                }
            }
        }
    }
}

/// A reader-set label within one Buzz community.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfidentialityLabel {
    realm: RealmId,
    readers: ReaderSet,
}

impl ConfidentialityLabel {
    fn public(realm: RealmId) -> Self {
        Self {
            realm,
            readers: ReaderSet::Everyone,
        }
    }

    fn restricted(realm: RealmId, readers: BTreeSet<Principal>) -> Self {
        Self {
            realm,
            readers: ReaderSet::Only(readers),
        }
    }

    fn can_flow_to(&self, destination: &Self) -> bool {
        self.realm == destination.realm && self.readers.can_flow_to(&destination.readers)
    }

    fn join(&self, other: &Self) -> Result<Self, LatticeError> {
        if self.realm != other.realm {
            return Err(LatticeError::CrossRealm);
        }
        Ok(Self {
            realm: self.realm.clone(),
            readers: self.readers.join(&other.readers),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LatticeError {
    CrossRealm,
}

/// Which retained context a worker belongs to. Audience and context remain
/// separate: two private conversations can have identical readers without
/// implicitly sharing memory.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum DomainContext {
    RealmPublic(RealmId),
    Conversation { realm: RealmId, channel_id: Uuid },
    OwnerPrivate { realm: RealmId, owner: Principal },
}

impl DomainContext {
    fn realm(&self) -> &RealmId {
        match self {
            Self::RealmPublic(realm)
            | Self::Conversation { realm, .. }
            | Self::OwnerPrivate { realm, .. } => realm,
        }
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

    /// Context policy used by the paper's `ContextPolicy(D, x)` predicate.
    /// Owner-private work may aggregate conversations the owner can read; the
    /// confidentiality check independently proves that the owner is a reader.
    fn permits(&self, resource: &ResourceContext) -> bool {
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
                hasher.update(realm.0);
            }
            Self::Conversation { realm, channel_id } => {
                hasher.update(b"conversation");
                hasher.update(realm.0);
                hasher.update(channel_id.as_bytes());
            }
            Self::OwnerPrivate { realm, owner } => {
                hasher.update(b"owner-private");
                hasher.update(realm.0);
                hash_field(hasher, owner.0.as_bytes());
            }
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::RealmPublic(_) => "public",
            Self::Conversation { .. } => "conversation",
            Self::OwnerPrivate { .. } => "owner_private",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ResourceContext {
    TrustedConfiguration,
    RealmPublic(RealmId),
    Conversation { realm: RealmId, channel_id: Uuid },
    OwnerPrivate { realm: RealmId, owner: Principal },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Capability(String);

impl Capability {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    fn from_names<const N: usize>(names: [&str; N]) -> Self {
        Self(names.into_iter().map(Capability::new).collect())
    }

    /// Paper: `C(turn) = C(bot) ∩ C(requester) ∩ C(domain)`.
    fn effective(bot: &Self, requester: &Self, domain: &Self) -> Self {
        let bot_and_requester: BTreeSet<_> = bot.0.intersection(&requester.0).cloned().collect();
        Self(bot_and_requester.intersection(&domain.0).cloned().collect())
    }

    fn contains(&self, operation: &str) -> bool {
        self.0.contains(&Capability::new(operation))
    }

    fn stable_hash(&self, hasher: &mut Sha256) {
        for capability in &self.0 {
            hash_field(hasher, capability.0.as_bytes());
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MembershipEpoch(String);

/// Paper: `D = (Audience, Context, Epoch, Capabilities)`.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecutionDomain {
    audience: ConfidentialityLabel,
    context: DomainContext,
    epoch: MembershipEpoch,
    capabilities: CapabilitySet,
}

impl ExecutionDomain {
    fn id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"buzz-ifc-domain-v1");
        hasher.update(self.audience.realm.0);
        self.audience.readers.stable_hash(&mut hasher);
        self.context.stable_hash(&mut hasher);
        hash_field(&mut hasher, self.epoch.0.as_bytes());
        self.capabilities.stable_hash(&mut hasher);
        hex::encode(hasher.finalize())
    }

    fn resource_label(&self) -> ResourceLabel {
        ResourceLabel {
            confidentiality: self.audience.clone(),
            context: self.context.resource_context(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourceLabel {
    confidentiality: ConfidentialityLabel,
    context: ResourceContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuleDecision {
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

    fn result(&self) -> &'static str {
        if self.allowed {
            "allow"
        } else {
            "deny"
        }
    }
}

struct RuleEvaluator;

impl RuleEvaluator {
    /// Paper: `read(D, x) ⇔ A(D) ⊆ R(x) ∧ ContextPolicy(D, x)`.
    fn read(domain: &ExecutionDomain, resource: &ResourceLabel) -> RuleDecision {
        if !resource.confidentiality.can_flow_to(&domain.audience) {
            return RuleDecision::deny("destination audience includes an unauthorized reader");
        }
        if !domain.context.permits(&resource.context) {
            return RuleDecision::deny("resource belongs to a different context");
        }
        RuleDecision::allow("audience and context both permit the read")
    }

    /// Paper: `call(D, op) ⇔ op ∈ C(D)`.
    fn call(domain: &ExecutionDomain, operation: &str) -> RuleDecision {
        if domain.capabilities.contains(operation) {
            RuleDecision::allow("operation is in the effective capability set")
        } else {
            RuleDecision::deny("operation is absent from the effective capability set")
        }
    }

    /// Paper: `reuse(D, e)` requires the complete domain, including epoch, to
    /// match. A separate ACP session in the same process is not sufficient.
    fn reuse(existing: &ExecutionDomain, requested: &ExecutionDomain) -> RuleDecision {
        if existing == requested {
            RuleDecision::allow("complete execution domain matches")
        } else {
            RuleDecision::deny("agent process has already entered a different domain")
        }
    }

    /// Paper: "Confinement invariant." Output may flow only to an audience no
    /// broader than the accumulated label, unless an exact verified grant applies.
    fn publish(
        state: &ConfinementState,
        source_domain: &ExecutionDomain,
        destination: &ConfidentialityLabel,
        destination_context: &DomainContext,
        content_digest: &[u8; 32],
        grant: Option<&DeclassificationGrant<VerifiedGrant>>,
    ) -> RuleDecision {
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
        if state.unknown_input || state.cross_realm {
            return RuleDecision::deny("process state contains unresolved input provenance");
        }
        if !source_domain.audience.can_flow_to(destination) {
            return RuleDecision::deny("destination is broader than the execution domain");
        }
        if state
            .label
            .as_ref()
            .is_some_and(|label| label.can_flow_to(destination))
        {
            return RuleDecision::allow("destination is no broader than accumulated state");
        }
        RuleDecision::deny("output would widen the accumulated reader set")
    }
}

/// Conservative label for everything an agent process may remember. This is
/// process-level rather than ACP-session-level because model sessions, tools,
/// files, and caches can influence one another inside a worker.
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
                Err(LatticeError::CrossRealm) => {
                    self.cross_realm = true;
                    Some(existing)
                }
            },
        };
    }

    fn mark_unknown(&mut self) {
        self.unknown_input = true;
    }
}

/// Audit state tied to one actual ACP agent process. It intentionally survives
/// channel-session invalidation: replacing a session does not make the process
/// forget information it has already seen.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProcessAuditState {
    entered_domains: Vec<ExecutionDomain>,
    confinement: ConfinementState,
}

impl ProcessAuditState {
    fn enter(&mut self, requested: &ExecutionDomain) -> RuleDecision {
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
}

/// Typestate marker: the owner's signature has not yet been checked.
#[allow(dead_code)]
struct PendingGrant;

/// Typestate marker: an external verifier has authenticated the owner signature.
struct VerifiedGrant;

/// A content-specific declassification grant. The evaluator only accepts the
/// `VerifiedGrant` state, so an unchecked grant cannot accidentally reach the
/// publish rule.
struct DeclassificationGrant<State> {
    approver: Principal,
    source_domain_id: String,
    destination: ConfidentialityLabel,
    destination_context: DomainContext,
    content_digest: [u8; 32],
    _state: PhantomData<State>,
}

#[allow(dead_code)]
trait GrantSignatureVerifier {
    fn verifies(&self, grant: &DeclassificationGrant<PendingGrant>) -> bool;
}

#[allow(dead_code)]
impl DeclassificationGrant<PendingGrant> {
    fn verify<V: GrantSignatureVerifier>(
        self,
        expected_owner: &Principal,
        verifier: &V,
    ) -> Result<DeclassificationGrant<VerifiedGrant>, GrantError> {
        if &self.approver != expected_owner {
            return Err(GrantError::WrongApprover);
        }
        if !verifier.verifies(&self) {
            return Err(GrantError::InvalidSignature);
        }
        Ok(DeclassificationGrant {
            approver: self.approver,
            source_domain_id: self.source_domain_id,
            destination: self.destination,
            destination_context: self.destination_context,
            content_digest: self.content_digest,
            _state: PhantomData,
        })
    }
}

impl DeclassificationGrant<VerifiedGrant> {
    fn matches(
        &self,
        source_domain_id: &str,
        destination: &ConfidentialityLabel,
        destination_context: &DomainContext,
        content_digest: &[u8; 32],
    ) -> bool {
        self.source_domain_id == source_domain_id
            && &self.destination == destination
            && &self.destination_context == destination_context
            && &self.content_digest == content_digest
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
enum GrantError {
    WrongApprover,
    InvalidSignature,
}

/// Trigger events after ID/signature and channel-binding checks. Domain
/// resolution requires this type, making event admission an explicit phase.
struct VerifiedTrigger {
    channel_id: Uuid,
    requesters: BTreeSet<Principal>,
}

#[derive(Debug)]
enum ResolutionError {
    EmptyBatch,
    InvalidTrigger,
    TriggerChannelMismatch,
    NoRelayIdentity,
    RelayQuery,
    MalformedRelayResponse,
    MissingMetadata,
    MissingMembership,
    InvalidAuthoritativeEvent,
    InvalidPrincipal,
    EmptyRestrictedAudience,
    AgentNotMember,
    RequesterNotMember,
    VerificationTask,
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyBatch => "trigger batch is empty",
            Self::InvalidTrigger => "trigger event signature or ID is invalid",
            Self::TriggerChannelMismatch => "trigger event is not bound to the queued channel",
            Self::NoRelayIdentity => "relay signing identity is unavailable",
            Self::RelayQuery => "authoritative channel query failed",
            Self::MalformedRelayResponse => "authoritative channel response is malformed",
            Self::MissingMetadata => "relay-signed channel metadata is missing",
            Self::MissingMembership => "relay-signed channel membership is missing",
            Self::InvalidAuthoritativeEvent => {
                "channel policy event failed identity, signature, or channel checks"
            }
            Self::InvalidPrincipal => "channel policy contains an invalid principal",
            Self::EmptyRestrictedAudience => "restricted channel has no human audience",
            Self::AgentNotMember => "executing agent is absent from channel membership",
            Self::RequesterNotMember => "trigger requester is absent from channel membership",
            Self::VerificationTask => "signature verification task failed",
        };
        f.write_str(message)
    }
}

/// Stateless policy front-end retained by `PromptContext` only in audit mode.
pub(crate) struct Auditor {
    realm: RealmId,
    rest_client: RestClient,
    relay_self: Option<PublicKey>,
    agent: Principal,
    owner: Option<Principal>,
}

impl Auditor {
    pub(crate) fn new(
        relay_url: &str,
        rest_client: RestClient,
        relay_self: Option<&str>,
        agent: PublicKey,
        owner: Option<PublicKey>,
    ) -> Self {
        Self {
            realm: RealmId::from_relay_url(relay_url),
            rest_client,
            relay_self: relay_self.and_then(|value| PublicKey::from_hex(value).ok()),
            agent: Principal::from_key(agent),
            owner: owner.map(Principal::from_key),
        }
    }

    /// Begin one audit turn. Failure to derive a trustworthy domain does not
    /// block the turn in audit mode, but permanently marks this process state as
    /// having unknown provenance so later publish checks cannot claim safety.
    pub(crate) async fn begin_turn(
        &self,
        batch: &FlushBatch,
        turn_id: &str,
        agent_index: usize,
        process: &mut ProcessAuditState,
    ) -> ActiveTurn {
        let verified = match verify_trigger_batch(batch).await {
            Ok(verified) => {
                log_rule(
                    turn_id,
                    agent_index,
                    None,
                    "event_admission",
                    "allow",
                    "all trigger IDs, signatures, and channel bindings verified",
                    false,
                );
                verified
            }
            Err(error) => {
                process.confinement.mark_unknown();
                log_rule(
                    turn_id,
                    agent_index,
                    None,
                    "event_admission",
                    "deny",
                    &error.to_string(),
                    false,
                );
                return ActiveTurn::unresolved(turn_id, agent_index, self.owner.clone());
            }
        };

        let domain = match self.resolve_domain(&verified).await {
            Ok(domain) => domain,
            Err(error) => {
                process.confinement.mark_unknown();
                log_rule(
                    turn_id,
                    agent_index,
                    None,
                    "domain_resolution",
                    "deny",
                    &error.to_string(),
                    false,
                );
                return ActiveTurn::unresolved(turn_id, agent_index, self.owner.clone());
            }
        };

        let domain_id = domain.id();
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = "audit",
            turn_id,
            agent_index,
            domain_id = %domain_id,
            realm = %self.realm.fingerprint(),
            context = domain.context.kind(),
            audience = if matches!(domain.audience.readers, ReaderSet::Everyone) {
                "public"
            } else {
                "restricted"
            },
            reader_count = domain.audience.readers.explicit_count(),
            requester_count = verified.requesters.len(),
            epoch = %short_fingerprint(&domain.epoch.0),
            "ifc execution domain resolved"
        );

        let reuse = process.enter(&domain);
        log_rule(
            turn_id,
            agent_index,
            Some(&domain_id),
            "reuse",
            reuse.result(),
            reuse.reason,
            false,
        );

        let turn = ActiveTurn {
            turn_id: turn_id.to_string(),
            agent_index,
            domain: Some(domain),
            owner: self.owner.clone(),
        };
        turn.observe_domain_input(process, "trigger_events");
        turn.observe_domain_input(process, "channel_metadata");
        turn.log_capability_policy();
        turn.log_unmediated_coverage();
        turn
    }

    async fn resolve_domain(
        &self,
        trigger: &VerifiedTrigger,
    ) -> Result<ExecutionDomain, ResolutionError> {
        let relay_self = self.relay_self.ok_or(ResolutionError::NoRelayIdentity)?;
        let d_tag = SingleLetterTag::lowercase(Alphabet::D);
        let channel = trigger.channel_id.to_string();
        let metadata_filter = Filter::new()
            .kind(Kind::Custom(
                buzz_core::kind::KIND_NIP29_GROUP_METADATA as u16,
            ))
            .custom_tags(d_tag, [channel.as_str()]);
        let membership_filter = Filter::new()
            .kind(Kind::Custom(
                buzz_core::kind::KIND_NIP29_GROUP_MEMBERS as u16,
            ))
            .custom_tags(d_tag, [channel.as_str()]);
        let response = self
            .rest_client
            .query(&[metadata_filter, membership_filter])
            .await
            .map_err(|_| ResolutionError::RelayQuery)?;
        let values = response
            .as_array()
            .ok_or(ResolutionError::MalformedRelayResponse)?;
        let events: Vec<Event> = values
            .iter()
            .filter_map(|value| serde_json::from_value(value.clone()).ok())
            .collect();

        let metadata = select_authoritative_event(
            &events,
            buzz_core::kind::KIND_NIP29_GROUP_METADATA,
            trigger.channel_id,
            relay_self,
        )
        .await?
        .ok_or(ResolutionError::MissingMetadata)?;
        let channel_type = channel_type(&metadata);
        if channel_type == "stream" {
            let context = DomainContext::RealmPublic(self.realm.clone());
            return Ok(ExecutionDomain {
                audience: ConfidentialityLabel::public(self.realm.clone()),
                capabilities: effective_turn_capabilities(&context, trigger, self.owner.as_ref()),
                context,
                // All public channels in this community intentionally share one
                // execution domain and public memory.
                epoch: MembershipEpoch(format!("community:{}", relay_self.to_hex())),
            });
        }

        let membership = select_authoritative_event(
            &events,
            buzz_core::kind::KIND_NIP29_GROUP_MEMBERS,
            trigger.channel_id,
            relay_self,
        )
        .await?
        .ok_or(ResolutionError::MissingMembership)?;
        let mut readers = member_principals(&membership)?;
        if !readers.contains(&self.agent) {
            return Err(ResolutionError::AgentNotMember);
        }
        let relay_principal = Principal::from_key(relay_self);
        if verified_requester_outside_membership(trigger, &readers, &relay_principal) {
            return Err(ResolutionError::RequesterNotMember);
        }
        // The executing bot is a processor, not an additional human recipient.
        // Other bots remain in the audience because they may relay what they see.
        readers.remove(&self.agent);
        if readers.is_empty() {
            return Err(ResolutionError::EmptyRestrictedAudience);
        }

        let context = match (&self.owner, channel_type) {
            (Some(owner), "dm")
                if readers.len() == 1 && readers.first().is_some_and(|reader| reader == owner) =>
            {
                DomainContext::OwnerPrivate {
                    realm: self.realm.clone(),
                    owner: owner.clone(),
                }
            }
            _ => DomainContext::Conversation {
                realm: self.realm.clone(),
                channel_id: trigger.channel_id,
            },
        };

        Ok(ExecutionDomain {
            audience: ConfidentialityLabel::restricted(self.realm.clone(), readers),
            capabilities: effective_turn_capabilities(&context, trigger, self.owner.as_ref()),
            context,
            // The relay-signed replaceable membership event is the epoch, not
            // merely a hash of the current roster. Remove-then-readd therefore
            // rotates state even if the final reader set happens to be identical.
            epoch: MembershipEpoch(format!("membership:{}", membership.id.to_hex())),
        })
    }
}

/// One resolved (or conservatively unresolved) invocation audit.
pub(crate) struct ActiveTurn {
    turn_id: String,
    agent_index: usize,
    domain: Option<ExecutionDomain>,
    owner: Option<Principal>,
}

impl ActiveTurn {
    fn unresolved(turn_id: &str, agent_index: usize, owner: Option<Principal>) -> Self {
        Self {
            turn_id: turn_id.to_string(),
            agent_index,
            domain: None,
            owner,
        }
    }

    /// Observe channel-bound material such as message history, a channel
    /// canvas, or huddle instructions.
    pub(crate) fn observe_domain_input(
        &self,
        process: &mut ProcessAuditState,
        source: &'static str,
    ) {
        let Some(domain) = self.domain.as_ref() else {
            process.confinement.mark_unknown();
            log_rule(
                &self.turn_id,
                self.agent_index,
                None,
                "read",
                "deny",
                "execution domain is unresolved",
                false,
            );
            return;
        };
        self.observe_resource(process, source, domain.resource_label());
    }

    /// Observe owner-scoped core memory. In the current harness the same NIP-AE
    /// core can be injected into every channel session; audit mode makes the
    /// resulting illegal flow visible without yet changing that behavior.
    pub(crate) fn observe_owner_private(
        &self,
        process: &mut ProcessAuditState,
        source: &'static str,
    ) {
        let (Some(domain), Some(owner)) = (self.domain.as_ref(), self.owner.as_ref()) else {
            process.confinement.mark_unknown();
            log_rule(
                &self.turn_id,
                self.agent_index,
                self.domain.as_ref().map(ExecutionDomain::id).as_deref(),
                "read",
                "deny",
                "owner-private input lacks a resolved owner or domain",
                false,
            );
            return;
        };
        let mut readers = BTreeSet::new();
        readers.insert(owner.clone());
        self.observe_resource(
            process,
            source,
            ResourceLabel {
                confidentiality: ConfidentialityLabel::restricted(
                    domain.audience.realm.clone(),
                    readers,
                ),
                context: ResourceContext::OwnerPrivate {
                    realm: domain.audience.realm.clone(),
                    owner: owner.clone(),
                },
            },
        );
    }

    /// Observe immutable platform configuration whose contents are part of the
    /// open Buzz distribution and therefore public within every realm.
    pub(crate) fn observe_public_configuration(
        &self,
        process: &mut ProcessAuditState,
        source: &'static str,
    ) {
        let Some(domain) = self.domain.as_ref() else {
            process.confinement.mark_unknown();
            return;
        };
        self.observe_resource(
            process,
            source,
            ResourceLabel {
                confidentiality: ConfidentialityLabel::public(domain.audience.realm.clone()),
                context: ResourceContext::TrustedConfiguration,
            },
        );
    }

    /// User and team supplied prompts have no label in today's configuration.
    /// Paper: "Labels and ordering." Unknown provenance must not be silently
    /// treated as public, so audit mode marks the process state unresolved.
    pub(crate) fn observe_unclassified(
        &self,
        process: &mut ProcessAuditState,
        source: &'static str,
    ) {
        process.confinement.mark_unknown();
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = "audit",
            turn_id = %self.turn_id,
            agent_index = self.agent_index,
            domain_id = self.domain.as_ref().map(ExecutionDomain::id),
            rule = "classify",
            source,
            decision = "deny",
            reason = "configured input has no explicit confidentiality label",
            enforced = false,
            "ifc rule evaluated"
        );
    }

    fn observe_resource(
        &self,
        process: &mut ProcessAuditState,
        source: &'static str,
        resource: ResourceLabel,
    ) {
        let Some(domain) = self.domain.as_ref() else {
            process.confinement.mark_unknown();
            return;
        };
        let decision = RuleEvaluator::read(domain, &resource);
        // Audit mode does not suppress a denied input. Since the real model sees
        // it, the confinement label must still include it; otherwise the later
        // publish decision would incorrectly claim the process remained clean.
        process.confinement.observe(&resource);
        let domain_id = domain.id();
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = "audit",
            turn_id = %self.turn_id,
            agent_index = self.agent_index,
            domain_id = %domain_id,
            rule = "read",
            source,
            decision = decision.result(),
            reason = decision.reason,
            enforced = false,
            "ifc rule evaluated"
        );
    }

    fn log_capability_policy(&self) {
        let Some(domain) = self.domain.as_ref() else {
            return;
        };
        let domain_id = domain.id();
        for operation in [
            "buzz.read.current",
            "buzz.publish.current",
            "email.read",
            "drive.read",
            "shell.host",
            "buzz.publish.arbitrary",
        ] {
            let decision = RuleEvaluator::call(domain, operation);
            tracing::info!(
                target: "buzz_acp::ifc",
                ifc_mode = "audit",
                turn_id = %self.turn_id,
                agent_index = self.agent_index,
                domain_id = %domain_id,
                rule = "call",
                operation,
                attempted = false,
                stage = "capability_assignment",
                decision = decision.result(),
                reason = decision.reason,
                enforced = false,
                "ifc rule evaluated"
            );
        }
    }

    /// Evaluate the intended reply sink after all known prompt inputs have been
    /// observed. The digest is intentionally synthetic in audit mode because the
    /// current ACP harness does not receive and bind the agent's final message.
    pub(crate) fn audit_reply(&self, process: &ProcessAuditState) {
        let Some(domain) = self.domain.as_ref() else {
            log_rule(
                &self.turn_id,
                self.agent_index,
                None,
                "publish",
                "deny",
                "execution domain is unresolved",
                false,
            );
            return;
        };
        let digest: [u8; 32] = Sha256::digest(b"audit-only-unbound-output").into();
        let decision = RuleEvaluator::publish(
            &process.confinement,
            domain,
            &domain.audience,
            &domain.context,
            &digest,
            None,
        );
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = "audit",
            turn_id = %self.turn_id,
            agent_index = self.agent_index,
            domain_id = %domain.id(),
            rule = "publish",
            decision = decision.result(),
            reason = decision.reason,
            output_bound = false,
            enforced = false,
            "ifc rule evaluated"
        );
    }

    fn log_unmediated_coverage(&self) {
        tracing::warn!(
            target: "buzz_acp::ifc",
            ifc_mode = "audit",
            turn_id = %self.turn_id,
            agent_index = self.agent_index,
            domain_id = self.domain.as_ref().map(ExecutionDomain::id),
            rule = "confinement_coverage",
            decision = "not_proven",
            enforced = false,
            gaps = "shared_agent_process, ambient_workspace, credential_bearing_mcp, direct_buzz_publication, static_capability_inventory, operating_system_isolation",
            "ifc audit cannot prove confinement while these paths bypass policy"
        );
    }
}

async fn verify_trigger_batch(batch: &FlushBatch) -> Result<VerifiedTrigger, ResolutionError> {
    let channel_id = batch.channel_id;
    let events: Vec<Event> = batch
        .events
        .iter()
        .chain(batch.cancelled_events.iter())
        .map(|item| item.event.clone())
        .collect();
    if events.is_empty() {
        return Err(ResolutionError::EmptyBatch);
    }
    tokio::task::spawn_blocking(move || {
        let mut requesters = BTreeSet::new();
        for event in events {
            buzz_core::verify_event(&event).map_err(|_| ResolutionError::InvalidTrigger)?;
            if !has_channel_tag(&event, "h", &channel_id.to_string()) {
                return Err(ResolutionError::TriggerChannelMismatch);
            }
            requesters.insert(Principal::from_key(event.pubkey));
        }
        Ok(VerifiedTrigger {
            channel_id,
            requesters,
        })
    })
    .await
    .map_err(|_| ResolutionError::VerificationTask)?
}

async fn select_authoritative_event(
    events: &[Event],
    kind: u32,
    channel_id: Uuid,
    relay_self: PublicKey,
) -> Result<Option<Event>, ResolutionError> {
    let candidates: Vec<Event> = events
        .iter()
        .filter(|event| event.kind == Kind::Custom(kind as u16))
        .filter(|event| has_channel_tag(event, "d", &channel_id.to_string()))
        .cloned()
        .collect();
    tokio::task::spawn_blocking(move || {
        let mut verified = Vec::new();
        for event in candidates {
            if event.pubkey != relay_self || buzz_core::verify_event(&event).is_err() {
                return Err(ResolutionError::InvalidAuthoritativeEvent);
            }
            verified.push(event);
        }
        verified.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.id.to_hex().cmp(&right.id.to_hex()))
        });
        Ok(verified.into_iter().next())
    })
    .await
    .map_err(|_| ResolutionError::VerificationTask)?
}

fn has_channel_tag(event: &Event, tag_name: &str, value: &str) -> bool {
    event.tags.iter().any(|tag| {
        let fields = tag.as_slice();
        fields.first().is_some_and(|field| field == tag_name)
            && fields.get(1).is_some_and(|field| field == value)
    })
}

fn channel_type(metadata: &Event) -> &'static str {
    let mut hidden = false;
    let mut private = false;
    let mut declared = None;
    for tag in metadata.tags.iter() {
        let fields = tag.as_slice();
        match fields.first().map(String::as_str) {
            Some("hidden") => hidden = true,
            Some("private") => private = true,
            Some("t") => declared = fields.get(1).map(String::as_str),
            _ => {}
        }
    }
    if hidden || declared == Some("dm") {
        "dm"
    } else if private || declared == Some("private") {
        "private"
    } else {
        "stream"
    }
}

fn member_principals(event: &Event) -> Result<BTreeSet<Principal>, ResolutionError> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let fields = tag.as_slice();
            (fields.first().map(String::as_str) == Some("p"))
                .then(|| fields.get(1).map(String::as_str))
                .flatten()
        })
        .map(Principal::from_hex)
        .collect()
}

fn verified_requester_outside_membership(
    trigger: &VerifiedTrigger,
    members: &BTreeSet<Principal>,
    relay: &Principal,
) -> bool {
    // Relay-signed workflow events are admitted by Buzz's existing attributed-
    // author gate. They remain trusted system events here; requester-specific
    // capability delegation still needs the attributed human identity.
    trigger
        .requesters
        .iter()
        .any(|requester| requester != relay && !members.contains(requester))
}

fn effective_turn_capabilities(
    context: &DomainContext,
    trigger: &VerifiedTrigger,
    owner: Option<&Principal>,
) -> CapabilitySet {
    let conversation =
        CapabilitySet::from_names(["buzz.read.current", "buzz.publish.current", "memory.domain"]);
    let bot = CapabilitySet::from_names([
        "buzz.read.current",
        "buzz.publish.current",
        "memory.domain",
        "email.read",
        "drive.read",
        "shell.host",
    ]);
    let requester_is_owner = owner.is_some_and(|owner| {
        !trigger.requesters.is_empty()
            && trigger
                .requesters
                .iter()
                .all(|requester| requester == owner)
    });
    let requester = if requester_is_owner {
        bot.clone()
    } else {
        conversation.clone()
    };
    let domain = if matches!(context, DomainContext::OwnerPrivate { .. }) {
        bot.clone()
    } else {
        conversation
    };

    // Paper: "Integrity and capabilities." Personal tools require both an
    // owner-authored request and the owner-private execution domain. Either
    // condition alone is insufficient.
    CapabilitySet::effective(&bot, &requester, &domain)
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(value.len().to_be_bytes());
    hasher.update(value);
}

fn short_fingerprint(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..6])
}

fn log_rule(
    turn_id: &str,
    agent_index: usize,
    domain_id: Option<&str>,
    rule: &'static str,
    decision: &'static str,
    reason: &str,
    enforced: bool,
) {
    tracing::info!(
        target: "buzz_acp::ifc",
        ifc_mode = "audit",
        turn_id,
        agent_index,
        domain_id,
        rule,
        decision,
        reason,
        enforced,
        "ifc rule evaluated"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::BatchEvent;
    use nostr::{EventBuilder, Keys, Tag};
    use std::time::Instant;

    fn principal(name: &str) -> Principal {
        Principal(name.to_string())
    }

    fn set(names: &[&str]) -> BTreeSet<Principal> {
        names.iter().map(|name| principal(name)).collect()
    }

    fn realm() -> RealmId {
        RealmId::from_relay_url("wss://buzz.example")
    }

    fn label(readers: &[&str]) -> ConfidentialityLabel {
        ConfidentialityLabel::restricted(realm(), set(readers))
    }

    fn public_label() -> ConfidentialityLabel {
        ConfidentialityLabel::public(realm())
    }

    fn domain(
        readers: &[&str],
        channel_id: Uuid,
        epoch: &str,
        capabilities: CapabilitySet,
    ) -> ExecutionDomain {
        ExecutionDomain {
            audience: label(readers),
            context: DomainContext::Conversation {
                realm: realm(),
                channel_id,
            },
            epoch: MembershipEpoch(epoch.to_string()),
            capabilities,
        }
    }

    #[test]
    fn reader_order_is_reverse_set_inclusion() {
        let public = ReaderSet::Everyone;
        let alice_bob = ReaderSet::Only(set(&["alice", "bob"]));
        let alice = ReaderSet::Only(set(&["alice"]));

        assert!(public.can_flow_to(&alice_bob));
        assert!(alice_bob.can_flow_to(&alice));
        assert!(!alice.can_flow_to(&alice_bob));
        assert!(!alice.can_flow_to(&public));
    }

    #[test]
    fn reader_sets_obey_lattice_laws() {
        let values = [
            ReaderSet::Everyone,
            ReaderSet::Only(set(&[])),
            ReaderSet::Only(set(&["alice"])),
            ReaderSet::Only(set(&["bob"])),
            ReaderSet::Only(set(&["alice", "bob"])),
        ];

        for a in &values {
            assert_eq!(a.join(a), *a);
            assert_eq!(a.meet(a), *a);
            for b in &values {
                assert_eq!(a.join(b), b.join(a));
                assert_eq!(a.meet(b), b.meet(a));
                assert_eq!(a.join(&a.meet(b)), *a);
                assert_eq!(a.meet(&a.join(b)), *a);
                for c in &values {
                    assert_eq!(a.join(&b.join(c)), a.join(b).join(c));
                    assert_eq!(a.meet(&b.meet(c)), a.meet(b).meet(c));
                }
            }
        }
    }

    #[test]
    fn combining_inputs_intersects_authorized_readers() {
        let left = label(&["alice", "bob"]);
        let right = label(&["alice", "carol"]);
        assert_eq!(left.join(&right).expect("same realm"), label(&["alice"]));
    }

    #[test]
    fn labels_never_flow_across_communities() {
        let first = ConfidentialityLabel::public(RealmId::from_relay_url("wss://one"));
        let second = ConfidentialityLabel::public(RealmId::from_relay_url("wss://two"));
        assert!(!first.can_flow_to(&second));
        assert_eq!(first.join(&second), Err(LatticeError::CrossRealm));
    }

    #[test]
    fn read_requires_both_audience_and_context() {
        let channel = Uuid::new_v4();
        let other_channel = Uuid::new_v4();
        let turn = domain(&["alice", "bob"], channel, "v1", CapabilitySet::default());
        let wrong_audience = ResourceLabel {
            confidentiality: label(&["alice"]),
            context: ResourceContext::Conversation {
                realm: realm(),
                channel_id: channel,
            },
        };
        let wrong_context = ResourceLabel {
            confidentiality: label(&["alice", "bob"]),
            context: ResourceContext::Conversation {
                realm: realm(),
                channel_id: other_channel,
            },
        };

        assert!(!RuleEvaluator::read(&turn, &wrong_audience).allowed);
        assert!(!RuleEvaluator::read(&turn, &wrong_context).allowed);
        assert!(RuleEvaluator::read(&turn, &turn.resource_label()).allowed);
    }

    #[test]
    fn owner_private_context_can_aggregate_only_data_owner_may_read() {
        let owner = principal("alice");
        let owner_domain = ExecutionDomain {
            audience: label(&["alice"]),
            context: DomainContext::OwnerPrivate {
                realm: realm(),
                owner,
            },
            epoch: MembershipEpoch("owner-v1".into()),
            capabilities: CapabilitySet::default(),
        };
        let readable_channel = ResourceLabel {
            confidentiality: label(&["alice", "bob"]),
            context: ResourceContext::Conversation {
                realm: realm(),
                channel_id: Uuid::new_v4(),
            },
        };
        let unreadable_channel = ResourceLabel {
            confidentiality: label(&["bob", "carol"]),
            context: ResourceContext::Conversation {
                realm: realm(),
                channel_id: Uuid::new_v4(),
            },
        };

        assert!(RuleEvaluator::read(&owner_domain, &readable_channel).allowed);
        assert!(!RuleEvaluator::read(&owner_domain, &unreadable_channel).allowed);
    }

    #[test]
    fn effective_capabilities_are_the_three_way_intersection() {
        let bot = CapabilitySet::from_names(["buzz.read.current", "email.read", "drive.read"]);
        let requester = CapabilitySet::from_names(["buzz.read.current", "email.read"]);
        let domain = CapabilitySet::from_names(["buzz.read.current", "drive.read"]);
        assert_eq!(
            CapabilitySet::effective(&bot, &requester, &domain),
            CapabilitySet::from_names(["buzz.read.current"])
        );
    }

    #[test]
    fn personal_capabilities_require_owner_request_and_owner_context() {
        let owner = principal("alice");
        let owner_trigger = VerifiedTrigger {
            channel_id: Uuid::new_v4(),
            requesters: BTreeSet::from([owner.clone()]),
        };
        let other_trigger = VerifiedTrigger {
            channel_id: Uuid::new_v4(),
            requesters: BTreeSet::from([principal("bob")]),
        };
        let owner_context = DomainContext::OwnerPrivate {
            realm: realm(),
            owner: owner.clone(),
        };
        let public_context = DomainContext::RealmPublic(realm());

        assert!(
            effective_turn_capabilities(&owner_context, &owner_trigger, Some(&owner))
                .contains("email.read")
        );
        assert!(
            !effective_turn_capabilities(&public_context, &owner_trigger, Some(&owner))
                .contains("email.read")
        );
        assert!(
            !effective_turn_capabilities(&owner_context, &other_trigger, Some(&owner))
                .contains("email.read")
        );
    }

    #[test]
    fn reuse_requires_context_epoch_and_capabilities_to_match() {
        let channel = Uuid::new_v4();
        let capabilities = CapabilitySet::from_names(["buzz.read.current"]);
        let first = domain(&["alice", "bob"], channel, "v1", capabilities.clone());
        let same = first.clone();
        let new_epoch = domain(&["alice", "bob"], channel, "v2", capabilities.clone());
        let new_context = domain(&["alice", "bob"], Uuid::new_v4(), "v1", capabilities);

        assert!(RuleEvaluator::reuse(&first, &same).allowed);
        assert!(!RuleEvaluator::reuse(&first, &new_epoch).allowed);
        assert!(!RuleEvaluator::reuse(&first, &new_context).allowed);
    }

    #[test]
    fn process_reuse_stays_denied_after_crossing_a_domain_boundary() {
        let capabilities = CapabilitySet::from_names(["buzz.read.current"]);
        let first = domain(
            &["alice", "bob"],
            Uuid::new_v4(),
            "v1",
            capabilities.clone(),
        );
        let second = domain(&["alice", "carol"], Uuid::new_v4(), "v1", capabilities);
        let mut process = ProcessAuditState::default();

        assert!(process.enter(&first).allowed);
        assert!(!process.enter(&second).allowed);
        assert!(
            !process.enter(&first).allowed,
            "returning to the first domain does not erase the second domain's influence"
        );
    }

    #[test]
    fn denied_input_still_taints_a_log_only_process() {
        let channel = Uuid::new_v4();
        let turn = domain(&["alice", "bob"], channel, "v1", CapabilitySet::default());
        let private = ResourceLabel {
            confidentiality: label(&["alice"]),
            context: ResourceContext::OwnerPrivate {
                realm: realm(),
                owner: principal("alice"),
            },
        };
        assert!(!RuleEvaluator::read(&turn, &private).allowed);

        let mut state = ConfinementState::default();
        state.observe(&turn.resource_label());
        state.observe(&private);
        let digest = [7; 32];
        assert!(
            !RuleEvaluator::publish(&state, &turn, &turn.audience, &turn.context, &digest, None,)
                .allowed,
            "audit mode must not claim safety after allowing a denied read through"
        );
    }

    struct AlwaysValid;

    impl GrantSignatureVerifier for AlwaysValid {
        fn verifies(&self, _grant: &DeclassificationGrant<PendingGrant>) -> bool {
            true
        }
    }

    #[test]
    fn declassification_is_owner_verified_and_exact() {
        let channel = Uuid::new_v4();
        let turn = domain(&["alice"], channel, "v1", CapabilitySet::default());
        let destination = public_label();
        let content = [9; 32];
        let pending = DeclassificationGrant::<PendingGrant> {
            approver: principal("alice"),
            source_domain_id: turn.id(),
            destination: destination.clone(),
            destination_context: DomainContext::RealmPublic(realm()),
            content_digest: content,
            _state: PhantomData,
        };
        let verified = pending
            .verify(&principal("alice"), &AlwaysValid)
            .expect("owner signature");
        let mut state = ConfinementState::default();
        state.observe(&turn.resource_label());

        assert!(
            RuleEvaluator::publish(
                &state,
                &turn,
                &destination,
                &DomainContext::RealmPublic(realm()),
                &content,
                Some(&verified),
            )
            .allowed
        );
        assert!(
            !RuleEvaluator::publish(
                &state,
                &turn,
                &destination,
                &DomainContext::RealmPublic(realm()),
                &[8; 32],
                Some(&verified),
            )
            .allowed
        );
        assert!(
            !RuleEvaluator::publish(
                &state,
                &turn,
                &destination,
                &DomainContext::Conversation {
                    realm: realm(),
                    channel_id: Uuid::new_v4(),
                },
                &content,
                Some(&verified),
            )
            .allowed,
            "a grant for one destination context cannot authorize another"
        );
    }

    #[test]
    fn public_domain_ids_match_across_public_channels() {
        let capabilities = CapabilitySet::from_names(["buzz.read.current"]);
        let first = ExecutionDomain {
            audience: public_label(),
            context: DomainContext::RealmPublic(realm()),
            epoch: MembershipEpoch("community".into()),
            capabilities: capabilities.clone(),
        };
        let second = ExecutionDomain {
            audience: public_label(),
            context: DomainContext::RealmPublic(realm()),
            epoch: MembershipEpoch("community".into()),
            capabilities,
        };
        assert_eq!(first.id(), second.id());
    }

    #[tokio::test]
    async fn trigger_typestate_requires_signature_and_channel_binding() {
        let keys = Keys::generate();
        let channel_id = Uuid::new_v4();
        let tag = Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag");
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .tags([tag])
            .sign_with_keys(&keys)
            .expect("signed event");
        let batch = FlushBatch {
            channel_id,
            events: vec![BatchEvent {
                event: event.clone(),
                prompt_tag: "test".into(),
                received_at: Instant::now(),
            }],
            cancelled_events: Vec::new(),
            cancel_reason: None,
        };
        assert!(verify_trigger_batch(&batch).await.is_ok());

        let mut tampered = serde_json::to_value(event).expect("event JSON");
        tampered["content"] = serde_json::Value::String("changed after signing".into());
        let mut invalid = batch.clone();
        invalid.events[0].event = serde_json::from_value(tampered).expect("tampered event");
        assert!(matches!(
            verify_trigger_batch(&invalid).await,
            Err(ResolutionError::InvalidTrigger)
        ));

        let mut wrong_channel = batch;
        wrong_channel.channel_id = Uuid::new_v4();
        assert!(matches!(
            verify_trigger_batch(&wrong_channel).await,
            Err(ResolutionError::TriggerChannelMismatch)
        ));
    }

    #[tokio::test]
    async fn channel_policy_requires_the_advertised_relay_signer() {
        let relay = Keys::generate();
        let attacker = Keys::generate();
        let channel_id = Uuid::new_v4();
        let make_metadata = |keys: &Keys| {
            let d = channel_id.to_string();
            EventBuilder::new(
                Kind::Custom(buzz_core::kind::KIND_NIP29_GROUP_METADATA as u16),
                "",
            )
            .tags([Tag::parse(["d", d.as_str()]).expect("d tag")])
            .sign_with_keys(keys)
            .expect("signed metadata")
        };

        let valid = make_metadata(&relay);
        assert!(select_authoritative_event(
            std::slice::from_ref(&valid),
            buzz_core::kind::KIND_NIP29_GROUP_METADATA,
            channel_id,
            relay.public_key(),
        )
        .await
        .expect("valid policy")
        .is_some());

        let forged = make_metadata(&attacker);
        assert!(matches!(
            select_authoritative_event(
                &[forged],
                buzz_core::kind::KIND_NIP29_GROUP_METADATA,
                channel_id,
                relay.public_key(),
            )
            .await,
            Err(ResolutionError::InvalidAuthoritativeEvent)
        ));
    }

    #[test]
    fn membership_snapshot_accepts_only_valid_nostr_principals() {
        let relay = Keys::generate();
        let member = Keys::generate();
        let channel_id = Uuid::new_v4().to_string();
        let valid = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_NIP29_GROUP_MEMBERS as u16),
            "",
        )
        .tags([
            Tag::parse(["d", channel_id.as_str()]).expect("d tag"),
            Tag::parse(["p", member.public_key().to_hex().as_str()]).expect("p tag"),
        ])
        .sign_with_keys(&relay)
        .expect("signed membership");
        assert_eq!(
            member_principals(&valid).expect("valid members"),
            BTreeSet::from([Principal::from_key(member.public_key())])
        );

        let invalid = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_NIP29_GROUP_MEMBERS as u16),
            "",
        )
        .tags([
            Tag::parse(["d", channel_id.as_str()]).expect("d tag"),
            Tag::parse(["p", "not-a-pubkey"]).expect("syntactic p tag"),
        ])
        .sign_with_keys(&relay)
        .expect("signed membership");
        assert!(matches!(
            member_principals(&invalid),
            Err(ResolutionError::InvalidPrincipal)
        ));
    }

    #[test]
    fn restricted_event_admission_rejects_nonmember_requesters() {
        let alice = principal("alice");
        let bob = principal("bob");
        let relay = principal("relay");
        let members = BTreeSet::from([alice.clone()]);

        let member = VerifiedTrigger {
            channel_id: Uuid::new_v4(),
            requesters: BTreeSet::from([alice]),
        };
        assert!(!verified_requester_outside_membership(
            &member, &members, &relay
        ));

        let nonmember = VerifiedTrigger {
            channel_id: Uuid::new_v4(),
            requesters: BTreeSet::from([bob]),
        };
        assert!(verified_requester_outside_membership(
            &nonmember, &members, &relay
        ));

        let relay_workflow = VerifiedTrigger {
            channel_id: Uuid::new_v4(),
            requesters: BTreeSet::from([relay.clone()]),
        };
        assert!(!verified_requester_outside_membership(
            &relay_workflow,
            &members,
            &relay,
        ));
    }

    #[tokio::test]
    async fn auditor_resolves_owner_dm_from_relay_signed_membership() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let relay = Keys::generate();
        let agent = Keys::generate();
        let owner = Keys::generate();
        let channel_id = Uuid::new_v4();
        let channel = channel_id.to_string();
        let metadata = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_NIP29_GROUP_METADATA as u16),
            "",
        )
        .tags([
            Tag::parse(["d", channel.as_str()]).expect("d tag"),
            Tag::parse(["hidden"]).expect("hidden tag"),
            Tag::parse(["t", "dm"]).expect("type tag"),
        ])
        .sign_with_keys(&relay)
        .expect("signed metadata");
        let membership = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_NIP29_GROUP_MEMBERS as u16),
            "",
        )
        .tags([
            Tag::parse(["d", channel.as_str()]).expect("d tag"),
            Tag::parse(["p", agent.public_key().to_hex().as_str()]).expect("agent tag"),
            Tag::parse(["p", owner.public_key().to_hex().as_str()]).expect("owner tag"),
        ])
        .sign_with_keys(&relay)
        .expect("signed membership");
        let response_body = serde_json::to_string(&[metadata, membership]).expect("response");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind policy server");
        let base_url = format!("http://{}", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept query");
            let mut request = vec![0; 16 * 1024];
            let _ = socket.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body,
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        let trigger = EventBuilder::new(Kind::TextNote, "hello")
            .tags([Tag::parse(["h", channel.as_str()]).expect("h tag")])
            .sign_with_keys(&owner)
            .expect("signed trigger");
        let batch = FlushBatch {
            channel_id,
            events: vec![BatchEvent {
                event: trigger,
                prompt_tag: "test".into(),
                received_at: Instant::now(),
            }],
            cancelled_events: Vec::new(),
            cancel_reason: None,
        };
        let rest = RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: agent.clone(),
            auth_tag_json: None,
        };
        let auditor = Auditor::new(
            "ws://example.test",
            rest,
            Some(&relay.public_key().to_hex()),
            agent.public_key(),
            Some(owner.public_key()),
        );
        let mut process = ProcessAuditState::default();
        let turn = auditor
            .begin_turn(&batch, "test-turn", 0, &mut process)
            .await;
        let domain = turn.domain.expect("resolved domain");

        assert!(matches!(domain.context, DomainContext::OwnerPrivate { .. }));
        assert_eq!(domain.audience.readers.explicit_count(), Some(1));
        assert!(domain.capabilities.contains("email.read"));
        assert_eq!(process.entered_domains.len(), 1);
        server.await.expect("policy server");
    }
}
