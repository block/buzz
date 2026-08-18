//! Coarse information-flow auditing for audience-scoped agent execution.
//!
//! This module is deliberately isolated from ACP transport and prompt formatting. In
//! `off` mode it is not constructed at all. In `audit` mode it observes the data that
//! the existing harness admits and emits structured decisions without changing runtime
//! behavior. In `route` mode the harness also uses its domain key to route or replace
//! ACP children before state crosses an audience, context, epoch, or capability boundary.
//!
//! Comments prefixed with "Paper" refer to the matching section in the
//! "Practical information-flow for Buzz agents" design paper.

use std::collections::BTreeSet;
use std::fmt;

use buzz_ifc::{
    derive_execution_domain, CapabilityPolicy, CapabilitySet, CompartmentProfile, ConversationKind,
    DerivationError, DomainFacts, DomainKey, ExecutionDomain, MembershipEpoch, Principal,
    ProcessState, RealmId, ResourceLabel, RuleEvaluator,
};
use nostr::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::InformationFlowMode;
use crate::queue::FlushBatch;
use crate::relay::RestClient;

pub(crate) type ProcessAuditState = ProcessState;

/// The complete policy assignment retained beside an ACP child.
///
/// The opaque key prevents cross-domain reuse. The profile tells the runtime
/// whether this is the shared public worker class or a confidential domain that
/// requires an externally enforced compartment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DomainBinding {
    key: DomainKey,
    profile: CompartmentProfile,
}

impl DomainBinding {
    fn from_domain(domain: &ExecutionDomain) -> Self {
        Self {
            key: domain.key(),
            profile: domain.compartment_profile(),
        }
    }

    pub(crate) fn fingerprint(&self) -> String {
        self.key.fingerprint()
    }

    pub(crate) fn profile(&self) -> CompartmentProfile {
        self.profile
    }
}

#[cfg(test)]
pub(crate) fn domain_binding_for_test(value: &str, profile: CompartmentProfile) -> DomainBinding {
    let domain = ExecutionDomain::public(
        RealmId::from_relay_url("wss://ifc.test"),
        MembershipEpoch::new(value),
        CapabilitySet::default(),
    );
    DomainBinding {
        key: domain.key(),
        profile,
    }
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
    InvalidDomain,
    EmptyRestrictedAudience,
    AgentNotMember,
    RequesterNotMember,
    NoOwner,
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
            Self::InvalidDomain => "resolved execution-domain fields are inconsistent",
            Self::EmptyRestrictedAudience => "restricted channel has no human audience",
            Self::AgentNotMember => "executing agent is absent from channel membership",
            Self::RequesterNotMember => "trigger requester is absent from channel membership",
            Self::NoOwner => "owner-private domain requires a resolved agent owner",
            Self::VerificationTask => "signature verification task failed",
        };
        f.write_str(message)
    }
}

/// Stateless policy front-end retained by `PromptContext` in audit or route mode.
pub(crate) struct Auditor {
    realm: RealmId,
    rest_client: RestClient,
    relay_self: Option<PublicKey>,
    agent: Principal,
    owner: Option<Principal>,
    mode: InformationFlowMode,
}

impl Auditor {
    pub(crate) fn new(
        relay_url: &str,
        rest_client: RestClient,
        relay_self: Option<&str>,
        agent: PublicKey,
        owner: Option<PublicKey>,
        mode: InformationFlowMode,
    ) -> Self {
        Self {
            realm: RealmId::from_relay_url(relay_url),
            rest_client,
            relay_self: relay_self.and_then(|value| PublicKey::from_hex(value).ok()),
            agent: Principal::from_public_key(&agent),
            owner: owner.map(|key| Principal::from_public_key(&key)),
            mode,
        }
    }

    /// Resolve one signed channel batch before it is assigned to an ACP child.
    /// The caller decides whether an unresolved turn is merely audited or denied.
    pub(crate) async fn resolve_turn(
        &self,
        batch: &FlushBatch,
        turn_id: &str,
        agent_index: Option<usize>,
    ) -> ActiveTurn {
        let verified = match verify_trigger_batch(batch).await {
            Ok(verified) => {
                log_rule(RuleLog {
                    mode: self.mode,
                    turn_id,
                    agent_index,
                    domain_id: None,
                    rule: "event_admission",
                    decision: "allow",
                    reason: "all trigger IDs, signatures, and channel bindings verified",
                    enforced: self.mode.routes_workers(),
                });
                verified
            }
            Err(error) => {
                log_rule(RuleLog {
                    mode: self.mode,
                    turn_id,
                    agent_index,
                    domain_id: None,
                    rule: "event_admission",
                    decision: "deny",
                    reason: &error.to_string(),
                    enforced: self.mode.routes_workers(),
                });
                return ActiveTurn::unresolved(
                    turn_id,
                    agent_index,
                    self.owner.clone(),
                    self.mode,
                    TurnOrigin::Channel,
                );
            }
        };

        let domain = match self.resolve_domain(&verified).await {
            Ok(domain) => domain,
            Err(error) => {
                log_rule(RuleLog {
                    mode: self.mode,
                    turn_id,
                    agent_index,
                    domain_id: None,
                    rule: "domain_resolution",
                    decision: "deny",
                    reason: &error.to_string(),
                    enforced: self.mode.routes_workers(),
                });
                return ActiveTurn::unresolved(
                    turn_id,
                    agent_index,
                    self.owner.clone(),
                    self.mode,
                    TurnOrigin::Channel,
                );
            }
        };

        let domain_id = domain.id();
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = %self.mode,
            turn_id,
            agent_index,
            domain_id = %domain_id,
            realm = %self.realm.fingerprint(),
            context = domain.context().kind(),
            compartment = domain.compartment_profile().as_str(),
            audience = if domain.audience().is_public() {
                "public"
            } else {
                "restricted"
            },
            reader_count = domain.audience().reader_count(),
            requester_count = verified.requesters.len(),
            epoch = %domain.epoch_fingerprint(),
            "ifc execution domain resolved"
        );

        ActiveTurn {
            turn_id: turn_id.to_string(),
            agent_index,
            domain: Some(domain),
            owner: self.owner.clone(),
            mode: self.mode,
            origin: TurnOrigin::Channel,
        }
    }

    /// Heartbeats carry no external event audience. They run in an owner-only
    /// domain so they cannot reuse a public or conversation-bound ACP child.
    pub(crate) fn resolve_heartbeat(
        &self,
        turn_id: &str,
        agent_index: Option<usize>,
    ) -> ActiveTurn {
        let Some(owner) = self.owner.clone() else {
            log_rule(RuleLog {
                mode: self.mode,
                turn_id,
                agent_index,
                domain_id: None,
                rule: "domain_resolution",
                decision: "deny",
                reason: &ResolutionError::NoOwner.to_string(),
                enforced: self.mode.routes_workers(),
            });
            return ActiveTurn::unresolved(
                turn_id,
                agent_index,
                None,
                self.mode,
                TurnOrigin::Heartbeat,
            );
        };
        let domain = ExecutionDomain::owner_private(
            self.realm.clone(),
            owner,
            MembershipEpoch::new("owner-heartbeat-v1"),
            bot_capabilities(),
        );
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = %self.mode,
            turn_id,
            agent_index,
            domain_id = %domain.id(),
            realm = %self.realm.fingerprint(),
            context = domain.context().kind(),
            compartment = domain.compartment_profile().as_str(),
            audience = "restricted",
            reader_count = 1,
            epoch = %domain.epoch_fingerprint(),
            "ifc execution domain resolved"
        );
        ActiveTurn {
            turn_id: turn_id.to_string(),
            agent_index,
            domain: Some(domain),
            owner: self.owner.clone(),
            mode: self.mode,
            origin: TurnOrigin::Heartbeat,
        }
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
        let (kind, epoch, members) = match channel_type(&metadata) {
            "stream" => (
                ConversationKind::Public,
                // All public channels in this community intentionally share one
                // execution domain and public memory.
                MembershipEpoch::new(format!("community:{}", relay_self.to_hex())),
                BTreeSet::new(),
            ),
            restricted_kind => {
                let membership = select_authoritative_event(
                    &events,
                    buzz_core::kind::KIND_NIP29_GROUP_MEMBERS,
                    trigger.channel_id,
                    relay_self,
                )
                .await?
                .ok_or(ResolutionError::MissingMembership)?;
                let members = member_principals(&membership)?;
                let kind = if restricted_kind == "dm" {
                    ConversationKind::DirectMessage
                } else {
                    ConversationKind::Restricted
                };
                (
                    kind,
                    // The relay-signed replaceable membership event is the epoch,
                    // not merely a hash of the current roster. Remove-then-readd
                    // rotates state even if the final readers are identical.
                    MembershipEpoch::new(format!("membership:{}", membership.id.to_hex())),
                    members,
                )
            }
        };
        derive_execution_domain(
            DomainFacts {
                realm: self.realm.clone(),
                channel_id: trigger.channel_id,
                kind,
                epoch,
                members,
                executing_agent: self.agent.clone(),
                requesters: trigger.requesters.clone(),
                system_principal: Some(Principal::from_public_key(&relay_self)),
                owner: self.owner.clone(),
            },
            &capability_policy(),
        )
        .map_err(map_derivation_error)
    }
}

#[derive(Clone, Copy)]
enum TurnOrigin {
    Channel,
    Heartbeat,
}

/// One resolved (or conservatively unresolved) invocation policy.
pub(crate) struct ActiveTurn {
    turn_id: String,
    agent_index: Option<usize>,
    domain: Option<ExecutionDomain>,
    owner: Option<Principal>,
    mode: InformationFlowMode,
    origin: TurnOrigin,
}

impl ActiveTurn {
    fn unresolved(
        turn_id: &str,
        agent_index: Option<usize>,
        owner: Option<Principal>,
        mode: InformationFlowMode,
        origin: TurnOrigin,
    ) -> Self {
        Self {
            turn_id: turn_id.to_string(),
            agent_index,
            domain: None,
            owner,
            mode,
            origin,
        }
    }

    pub(crate) fn domain_binding(&self) -> Option<DomainBinding> {
        self.domain.as_ref().map(DomainBinding::from_domain)
    }

    pub(crate) fn assign_agent(&mut self, agent_index: usize) {
        self.agent_index = Some(agent_index);
    }

    /// Commit the inputs that are about to enter the selected ACP process.
    /// Resolution happens before routing; this step happens only after the
    /// process is either proven reusable or replaced for this domain.
    pub(crate) fn enter_process(&self, process: &mut ProcessAuditState) {
        let Some(domain) = self.domain.as_ref() else {
            process.mark_unknown();
            return;
        };
        let domain_id = domain.id();
        let reuse = process.enter(domain);
        log_rule(RuleLog {
            mode: self.mode,
            turn_id: &self.turn_id,
            agent_index: self.agent_index,
            domain_id: Some(&domain_id),
            rule: "reuse",
            decision: reuse.result(),
            reason: reuse.reason(),
            enforced: self.mode.routes_workers(),
        });
        match self.origin {
            TurnOrigin::Channel => {
                self.observe_domain_input(process, "trigger_events");
                self.observe_domain_input(process, "channel_metadata");
            }
            TurnOrigin::Heartbeat => {
                self.observe_domain_input(process, "heartbeat_trigger");
            }
        }
        self.log_capability_policy();
        self.log_unmediated_coverage();
    }

    /// Whether owner-private material may enter this turn's domain. This is
    /// checked before the harness fetches core memory in route mode.
    pub(crate) fn permits_owner_private_input(&self) -> bool {
        let (Some(domain), Some(owner)) = (self.domain.as_ref(), self.owner.as_ref()) else {
            return false;
        };
        RuleEvaluator::read(
            domain,
            &ResourceLabel::owner_private(domain.audience().realm().clone(), owner.clone()),
        )
        .allowed()
    }

    pub(crate) fn log_blocked_owner_private_input(&self, source: &'static str) {
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = %self.mode,
            turn_id = %self.turn_id,
            agent_index = self.agent_index,
            domain_id = self.domain.as_ref().map(ExecutionDomain::id),
            rule = "read",
            source,
            decision = "deny",
            reason = "owner-private input cannot flow to this execution domain",
            enforced = true,
            "ifc rule evaluated"
        );
    }

    /// Observe channel-bound material such as message history, a channel
    /// canvas, or huddle instructions.
    pub(crate) fn observe_domain_input(
        &self,
        process: &mut ProcessAuditState,
        source: &'static str,
    ) {
        let Some(domain) = self.domain.as_ref() else {
            process.mark_unknown();
            log_rule(RuleLog {
                mode: self.mode,
                turn_id: &self.turn_id,
                agent_index: self.agent_index,
                domain_id: None,
                rule: "read",
                decision: "deny",
                reason: "execution domain is unresolved",
                enforced: self.mode.routes_workers(),
            });
            return;
        };
        self.observe_resource(process, source, domain.resource_label());
    }

    /// Observe owner-scoped core memory after the caller has admitted it.
    pub(crate) fn observe_owner_private(
        &self,
        process: &mut ProcessAuditState,
        source: &'static str,
    ) {
        let (Some(domain), Some(owner)) = (self.domain.as_ref(), self.owner.as_ref()) else {
            process.mark_unknown();
            let domain_id = self.domain.as_ref().map(ExecutionDomain::id);
            log_rule(RuleLog {
                mode: self.mode,
                turn_id: &self.turn_id,
                agent_index: self.agent_index,
                domain_id: domain_id.as_deref(),
                rule: "read",
                decision: "deny",
                reason: "owner-private input lacks a resolved owner or domain",
                enforced: self.mode.routes_workers(),
            });
            return;
        };
        self.observe_resource(
            process,
            source,
            ResourceLabel::owner_private(domain.audience().realm().clone(), owner.clone()),
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
            process.mark_unknown();
            return;
        };
        self.observe_resource(
            process,
            source,
            ResourceLabel::trusted_configuration(domain.audience().realm().clone()),
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
        process.mark_unknown();
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = %self.mode,
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
            process.mark_unknown();
            return;
        };
        let decision = RuleEvaluator::read(domain, &resource);
        // Audit mode records denied inputs because the model still sees them.
        // Route mode records only admitted inputs; denied owner-private input
        // is stopped before the broker fetches it.
        if decision.allowed() || !self.mode.routes_workers() {
            process.observe(&resource);
        }
        let domain_id = domain.id();
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = %self.mode,
            turn_id = %self.turn_id,
            agent_index = self.agent_index,
            domain_id = %domain_id,
            rule = "read",
            source,
            decision = decision.result(),
            reason = decision.reason(),
            enforced = self.mode.routes_workers(),
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
                ifc_mode = %self.mode,
                turn_id = %self.turn_id,
                agent_index = self.agent_index,
                domain_id = %domain_id,
                rule = "call",
                operation,
                attempted = false,
                stage = "capability_assignment",
                decision = decision.result(),
                reason = decision.reason(),
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
            log_rule(RuleLog {
                mode: self.mode,
                turn_id: &self.turn_id,
                agent_index: self.agent_index,
                domain_id: None,
                rule: "publish",
                decision: "deny",
                reason: "execution domain is unresolved",
                enforced: false,
            });
            return;
        };
        let digest: [u8; 32] = Sha256::digest(b"audit-only-unbound-output").into();
        let decision = process.publish(domain, domain.audience(), domain.context(), &digest, None);
        tracing::info!(
            target: "buzz_acp::ifc",
            ifc_mode = %self.mode,
            turn_id = %self.turn_id,
            agent_index = self.agent_index,
            domain_id = %domain.id(),
            rule = "publish",
            decision = decision.result(),
            reason = decision.reason(),
            output_bound = false,
            enforced = false,
            "ifc rule evaluated"
        );
    }

    fn log_unmediated_coverage(&self) {
        let (compartment, gaps) = match self
            .domain
            .as_ref()
            .map(ExecutionDomain::compartment_profile)
        {
            Some(CompartmentProfile::SharedPublic) => (
                CompartmentProfile::SharedPublic.as_str(),
                "broker_process_protection, private_compartment_protection, ambient_private_sources, direct_buzz_publication, static_capability_inventory",
            ),
            Some(CompartmentProfile::DomainConfined) => (
                CompartmentProfile::DomainConfined.as_str(),
                "dedicated_writable_state, credential_bearing_mcp, controlled_output_paths, direct_buzz_publication, operating_system_isolation",
            ),
            None => (
                "unresolved",
                "shared_agent_process, ambient_workspace, credential_bearing_mcp, direct_buzz_publication, operating_system_isolation",
            ),
        };
        tracing::warn!(
            target: "buzz_acp::ifc",
            ifc_mode = %self.mode,
            turn_id = %self.turn_id,
            agent_index = self.agent_index,
            domain_id = self.domain.as_ref().map(ExecutionDomain::id),
            compartment,
            rule = "confinement_coverage",
            decision = "not_proven",
            enforced = false,
            gaps,
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
            requesters.insert(Principal::from_public_key(&event.pubkey));
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
        .map(|value| Principal::from_hex(value).map_err(|_| ResolutionError::InvalidPrincipal))
        .collect()
}

fn capability_policy() -> CapabilityPolicy {
    let conversation =
        CapabilitySet::from_names(["buzz.read.current", "buzz.publish.current", "memory.domain"]);
    let bot = bot_capabilities();
    CapabilityPolicy::new(bot, conversation)
}

fn bot_capabilities() -> CapabilitySet {
    CapabilitySet::from_names([
        "buzz.read.current",
        "buzz.publish.current",
        "memory.domain",
        "email.read",
        "drive.read",
        "shell.host",
    ])
}

fn map_derivation_error(error: DerivationError) -> ResolutionError {
    match error {
        DerivationError::AgentNotMember => ResolutionError::AgentNotMember,
        DerivationError::RequesterNotMember => ResolutionError::RequesterNotMember,
        DerivationError::EmptyRestrictedAudience => ResolutionError::EmptyRestrictedAudience,
        DerivationError::EmptyRequesters | DerivationError::InvalidDomain => {
            ResolutionError::InvalidDomain
        }
    }
}

struct RuleLog<'a> {
    mode: InformationFlowMode,
    turn_id: &'a str,
    agent_index: Option<usize>,
    domain_id: Option<&'a str>,
    rule: &'static str,
    decision: &'static str,
    reason: &'a str,
    enforced: bool,
}

fn log_rule(entry: RuleLog<'_>) {
    let RuleLog {
        mode,
        turn_id,
        agent_index,
        domain_id,
        rule,
        decision,
        reason,
        enforced,
    } = entry;
    tracing::info!(
        target: "buzz_acp::ifc",
        ifc_mode = %mode,
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
    use buzz_ifc::DomainContext;
    use nostr::{EventBuilder, Keys, Tag};
    use std::time::Instant;

    fn principal(name: &str) -> Principal {
        let digest = Sha256::digest(name.as_bytes());
        Principal::from_hex(&hex::encode(digest)).expect("valid deterministic test principal")
    }

    fn realm() -> RealmId {
        RealmId::from_relay_url("wss://buzz.example")
    }

    #[test]
    fn owner_private_input_is_admitted_only_to_the_owner_domain() {
        let owner = principal("alice");
        let owner_turn = ActiveTurn {
            turn_id: "owner".into(),
            agent_index: Some(0),
            domain: Some(ExecutionDomain::owner_private(
                realm(),
                owner.clone(),
                MembershipEpoch::new("owner-v1"),
                bot_capabilities(),
            )),
            owner: Some(owner.clone()),
            mode: InformationFlowMode::Route,
            origin: TurnOrigin::Channel,
        };
        let public_turn = ActiveTurn {
            turn_id: "public".into(),
            agent_index: Some(1),
            domain: Some(ExecutionDomain::public(
                realm(),
                MembershipEpoch::new("public-v1"),
                CapabilitySet::default(),
            )),
            owner: Some(owner),
            mode: InformationFlowMode::Route,
            origin: TurnOrigin::Channel,
        };

        assert!(owner_turn.permits_owner_private_input());
        assert!(!public_turn.permits_owner_private_input());
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
            BTreeSet::from([Principal::from_public_key(&member.public_key())])
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
            InformationFlowMode::Audit,
        );
        let mut process = ProcessAuditState::default();
        let turn = auditor.resolve_turn(&batch, "test-turn", Some(0)).await;
        turn.enter_process(&mut process);
        let domain = turn.domain.as_ref().expect("resolved domain");

        assert!(matches!(
            domain.context(),
            DomainContext::OwnerPrivate { .. }
        ));
        assert_eq!(domain.audience().reader_count(), Some(1));
        assert!(domain.capabilities().contains("email.read"));
        assert_eq!(process.entered_domain_count(), 1);
        server.await.expect("policy server");
    }
}
