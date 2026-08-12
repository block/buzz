//! Closed typed authority for every protected relay ingress.

use std::collections::BTreeMap;

use buzz_auth::{ProofTransport, RouteCapability, RouteProtection};
use sha2::{Digest, Sha256};

use super::RuntimeAuthorizationError;

/// Complete semantic inventory of protected relay entry points.
///
/// Raw methods, paths, event kinds, and frame strings must be translated to
/// exactly one of these values by their owning adapters. An adapter that cannot
/// translate an input must deny it rather than selecting a capability itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProtectedIngress {
    /// Establish a provider-free WebSocket authorization session.
    WebSocketAuthenticate,
    /// Submit a protected event over WebSocket.
    WebSocketEvent,
    /// Read protected events over WebSocket.
    WebSocketQuery,
    /// Count protected events over WebSocket.
    WebSocketCount,
    /// Submit a protected event through the HTTP bridge.
    BridgeEvent,
    /// Read protected events through the HTTP bridge.
    BridgeQuery,
    /// Count protected events through the HTTP bridge.
    BridgeCount,
    /// Read moderation state.
    ModerationRead,
    /// Mutate moderation state.
    ModerationWrite,
    /// Read deployment-global operator state.
    OperatorRead,
    /// Mutate deployment-global operator state.
    OperatorWrite,
    /// Mint an invitation.
    InviteMint,
    /// Claim an invitation.
    InviteClaim,
    /// Read a media object with GET or HEAD.
    MediaRead,
    /// Upload or replace a media object.
    MediaWrite,
    /// Delete a media object.
    MediaDelete,
    /// Read Git objects or refs.
    GitRead,
    /// Mutate Git objects or refs.
    GitWrite,
    /// Keep a bounded Git stream alive.
    GitStream,
    /// Join a protected audio session.
    AudioJoin,
    /// Send or receive protected audio media.
    AudioMedia,
    /// Read protected runtime discovery.
    Discovery,
    /// Read current local binding status.
    BindingStatus,
}

impl ProtectedIngress {
    /// Closed inventory used to reject partial route installation.
    pub const ALL: [Self; 23] = [
        Self::WebSocketAuthenticate,
        Self::WebSocketEvent,
        Self::WebSocketQuery,
        Self::WebSocketCount,
        Self::BridgeEvent,
        Self::BridgeQuery,
        Self::BridgeCount,
        Self::ModerationRead,
        Self::ModerationWrite,
        Self::OperatorRead,
        Self::OperatorWrite,
        Self::InviteMint,
        Self::InviteClaim,
        Self::MediaRead,
        Self::MediaWrite,
        Self::MediaDelete,
        Self::GitRead,
        Self::GitWrite,
        Self::GitStream,
        Self::AudioJoin,
        Self::AudioMedia,
        Self::Discovery,
        Self::BindingStatus,
    ];

    const fn code(self) -> u8 {
        self as u8 + 1
    }

    /// Exact capability assigned by the closed relay inventory.
    pub const fn required_capability(self) -> RouteCapability {
        match self {
            Self::WebSocketAuthenticate => RouteCapability::BindingStatus,
            Self::WebSocketEvent | Self::BridgeEvent => RouteCapability::MessagesWrite,
            Self::WebSocketQuery | Self::WebSocketCount | Self::BridgeQuery | Self::BridgeCount => {
                RouteCapability::MessagesRead
            }
            Self::ModerationRead | Self::ModerationWrite => RouteCapability::Moderation,
            Self::OperatorRead | Self::OperatorWrite => RouteCapability::AdminChannels,
            Self::InviteMint => RouteCapability::InviteMint,
            Self::InviteClaim => RouteCapability::InviteClaim,
            Self::MediaRead => RouteCapability::MediaRead,
            Self::MediaWrite | Self::MediaDelete => RouteCapability::MediaWrite,
            Self::GitRead => RouteCapability::GitRead,
            Self::GitWrite => RouteCapability::GitWrite,
            Self::GitStream => RouteCapability::GitStream,
            Self::AudioJoin => RouteCapability::AudioJoin,
            Self::AudioMedia => RouteCapability::AudioMedia,
            Self::Discovery => RouteCapability::Discovery,
            Self::BindingStatus => RouteCapability::BindingStatus,
        }
    }

    /// Exact application-effect class assigned by the closed inventory.
    pub const fn required_effect(self) -> ProtectedEffect {
        match self {
            Self::WebSocketAuthenticate
            | Self::WebSocketEvent
            | Self::BridgeEvent
            | Self::ModerationWrite
            | Self::OperatorWrite
            | Self::InviteMint
            | Self::InviteClaim
            | Self::MediaWrite
            | Self::MediaDelete
            | Self::GitWrite => ProtectedEffect::Mutate,
            Self::GitStream | Self::AudioMedia => ProtectedEffect::Stream,
            _ => ProtectedEffect::Read,
        }
    }

    /// Exact server-owned resource namespace assigned by the closed inventory.
    pub const fn required_resource(self) -> ProtectedResourceKind {
        match self {
            Self::BridgeEvent => ProtectedResourceKind::Event,
            Self::ModerationRead | Self::ModerationWrite => ProtectedResourceKind::ModerationTarget,
            Self::MediaRead | Self::MediaWrite | Self::MediaDelete => ProtectedResourceKind::Media,
            Self::GitRead | Self::GitWrite | Self::GitStream => ProtectedResourceKind::Repository,
            Self::AudioJoin | Self::AudioMedia => ProtectedResourceKind::AudioSession,
            Self::InviteMint | Self::InviteClaim => ProtectedResourceKind::Invitation,
            Self::WebSocketAuthenticate | Self::BindingStatus => {
                ProtectedResourceKind::BindingStatus
            }
            Self::WebSocketQuery
            | Self::WebSocketEvent
            | Self::WebSocketCount
            | Self::BridgeQuery
            | Self::BridgeCount
            | Self::OperatorRead
            | Self::OperatorWrite
            | Self::Discovery => ProtectedResourceKind::Domain,
        }
    }

    /// Exact independently verified proof transport for this ingress.
    pub const fn required_transport(self) -> ProofTransport {
        match self {
            Self::WebSocketAuthenticate
            | Self::WebSocketEvent
            | Self::WebSocketQuery
            | Self::WebSocketCount
            | Self::BindingStatus => ProofTransport::Nip42,
            Self::MediaRead | Self::MediaWrite | Self::MediaDelete => ProofTransport::Blossom,
            Self::GitRead | Self::GitWrite | Self::GitStream => ProofTransport::GitSmartHttpSession,
            _ => ProofTransport::Nip98,
        }
    }
}

/// Server-owned protected object namespace resolved by an ingress adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectedResourceKind {
    /// Authorization session scoped to one server-owned domain.
    Domain,
    /// One channel.
    Channel,
    /// One event or closed event set.
    Event,
    /// One repository.
    Repository,
    /// One media object.
    Media,
    /// One moderation target.
    ModerationTarget,
    /// One invitation.
    Invitation,
    /// One actor's connection-local binding-status authority.
    BindingStatus,
    /// One audio session.
    AudioSession,
    /// One delegated-agent relationship.
    DelegatedAgent,
}

impl ProtectedResourceKind {
    const fn code(self) -> u8 {
        self as u8 + 1
    }
}

/// Whether an admitted route observes, mutates, or streams protected state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectedEffect {
    /// Bounded read-only observation.
    Read,
    /// Atomic protected mutation.
    Mutate,
    /// Long-running operation requiring periodic re-fencing.
    Stream,
}

impl ProtectedEffect {
    const fn code(self) -> u8 {
        self as u8 + 1
    }
}

/// One exact rule supplied by the owning protected-ingress layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteRule {
    ingress: ProtectedIngress,
    protection: RouteProtection,
    resource: ProtectedResourceKind,
    effect: ProtectedEffect,
    transport: ProofTransport,
}

impl RouteRule {
    /// Bind one ingress to its exact sealed capability and transport.
    pub const fn protected(
        ingress: ProtectedIngress,
        capability: RouteCapability,
        resource: ProtectedResourceKind,
        effect: ProtectedEffect,
        transport: ProofTransport,
    ) -> Self {
        Self {
            ingress,
            protection: RouteProtection::Protected(capability),
            resource,
            effect,
            transport,
        }
    }

    /// Protected ingress identity.
    pub const fn ingress(self) -> ProtectedIngress {
        self.ingress
    }
}

/// Immutable complete route inventory installed into production state once.
#[derive(Clone)]
pub struct RouteAuthority {
    rules: BTreeMap<ProtectedIngress, RouteRule>,
    inventory_fingerprint: [u8; 32],
}

impl RouteAuthority {
    /// Install the repository's one closed production inventory.
    pub fn closed() -> Result<Self, RuntimeAuthorizationError> {
        Self::new(ProtectedIngress::ALL.into_iter().map(|ingress| {
            RouteRule::protected(
                ingress,
                ingress.required_capability(),
                ingress.required_resource(),
                ingress.required_effect(),
                ingress.required_transport(),
            )
        }))
    }

    /// Validate a unique rule for every protected ingress.
    pub fn new(
        rules: impl IntoIterator<Item = RouteRule>,
    ) -> Result<Self, RuntimeAuthorizationError> {
        let mut by_ingress = BTreeMap::new();
        for rule in rules {
            let RouteProtection::Protected(capability) = rule.protection else {
                return Err(RuntimeAuthorizationError::IncompleteRouteInventory);
            };
            if capability != rule.ingress.required_capability()
                || rule.resource != rule.ingress.required_resource()
                || rule.effect != rule.ingress.required_effect()
                || rule.transport != rule.ingress.required_transport()
            {
                return Err(RuntimeAuthorizationError::IncompleteRouteInventory);
            }
            if by_ingress.insert(rule.ingress, rule).is_some() {
                return Err(RuntimeAuthorizationError::IncompleteRouteInventory);
            }
        }
        if by_ingress.len() != ProtectedIngress::ALL.len()
            || ProtectedIngress::ALL
                .iter()
                .any(|ingress| !by_ingress.contains_key(ingress))
        {
            return Err(RuntimeAuthorizationError::IncompleteRouteInventory);
        }

        let mut digest = Sha256::new();
        digest.update(b"buzz:nip-fi:protected-route-inventory:v1\0");
        for ingress in ProtectedIngress::ALL {
            let rule = by_ingress
                .get(&ingress)
                .ok_or(RuntimeAuthorizationError::IncompleteRouteInventory)?;
            let RouteProtection::Protected(capability) = rule.protection else {
                return Err(RuntimeAuthorizationError::IncompleteRouteInventory);
            };
            digest.update([
                ingress.code(),
                capability_code(capability),
                rule.resource.code(),
                rule.effect.code(),
                proof_transport_code(rule.transport),
            ]);
        }

        Ok(Self {
            rules: by_ingress,
            inventory_fingerprint: digest.finalize().into(),
        })
    }

    /// Resolve an exact ingress and independently verified proof transport.
    pub fn resolve(
        &self,
        ingress: ProtectedIngress,
        transport: ProofTransport,
    ) -> Result<ResolvedProtectedRoute, RuntimeAuthorizationError> {
        let rule = self
            .rules
            .get(&ingress)
            .copied()
            .ok_or(RuntimeAuthorizationError::UnknownProtectedRoute)?;
        if rule.transport != transport {
            return Err(RuntimeAuthorizationError::TransportMismatch);
        }
        let RouteProtection::Protected(capability) = rule.protection else {
            return Err(RuntimeAuthorizationError::UnknownProtectedRoute);
        };
        Ok(ResolvedProtectedRoute {
            ingress,
            capability,
            resource: rule.resource,
            effect: rule.effect,
            transport,
        })
    }

    /// Stable fingerprint proving that one complete inventory was installed.
    pub const fn inventory_fingerprint(&self) -> &[u8; 32] {
        &self.inventory_fingerprint
    }
}

impl std::fmt::Debug for RouteAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RouteAuthority([REDACTED])")
    }
}

/// Exact route classification consumed by preparation and finalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedProtectedRoute {
    ingress: ProtectedIngress,
    capability: RouteCapability,
    resource: ProtectedResourceKind,
    effect: ProtectedEffect,
    transport: ProofTransport,
}

impl ResolvedProtectedRoute {
    /// Typed ingress identity.
    pub const fn ingress(self) -> ProtectedIngress {
        self.ingress
    }

    /// Exact sealed capability.
    pub const fn capability(self) -> RouteCapability {
        self.capability
    }

    /// Server-owned resource namespace.
    pub const fn resource(self) -> ProtectedResourceKind {
        self.resource
    }

    /// Protected effect class.
    pub const fn effect(self) -> ProtectedEffect {
        self.effect
    }

    /// Independently verified proof transport.
    pub const fn transport(self) -> ProofTransport {
        self.transport
    }
}

const fn proof_transport_code(transport: ProofTransport) -> u8 {
    match transport {
        ProofTransport::Nip42 => 1,
        ProofTransport::Nip98 => 2,
        ProofTransport::GitSmartHttpSession => 3,
        ProofTransport::Blossom => 4,
    }
}

const fn capability_code(capability: RouteCapability) -> u8 {
    match capability {
        RouteCapability::MessagesRead => 1,
        RouteCapability::MessagesWrite => 2,
        RouteCapability::ChannelsRead => 3,
        RouteCapability::ChannelsWrite => 4,
        RouteCapability::AdminChannels => 5,
        RouteCapability::UsersRead => 6,
        RouteCapability::UsersWrite => 7,
        RouteCapability::AdminUsers => 8,
        RouteCapability::JobsRead => 9,
        RouteCapability::JobsWrite => 10,
        RouteCapability::SubscriptionsRead => 11,
        RouteCapability::SubscriptionsWrite => 12,
        RouteCapability::FilesRead => 13,
        RouteCapability::FilesWrite => 14,
        RouteCapability::ReposRead => 15,
        RouteCapability::ReposWrite => 16,
        RouteCapability::GitRead => 17,
        RouteCapability::GitWrite => 18,
        RouteCapability::GitStream => 19,
        RouteCapability::MediaRead => 20,
        RouteCapability::MediaWrite => 21,
        RouteCapability::Moderation => 22,
        RouteCapability::AudioJoin => 23,
        RouteCapability::AudioMedia => 24,
        RouteCapability::Discovery => 25,
        RouteCapability::BindingStatus => 26,
        RouteCapability::Enrollment => 27,
        RouteCapability::InviteMint => 28,
        RouteCapability::InviteClaim => 29,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_rules() -> Vec<RouteRule> {
        RouteAuthority::closed()
            .unwrap()
            .rules
            .into_values()
            .collect()
    }

    #[test]
    fn inventory_must_be_complete_and_unique() {
        let mut missing = complete_rules();
        missing.pop();
        assert_eq!(
            RouteAuthority::new(missing).unwrap_err(),
            RuntimeAuthorizationError::IncompleteRouteInventory
        );

        let mut duplicate = complete_rules();
        duplicate.push(duplicate[0]);
        assert_eq!(
            RouteAuthority::new(duplicate).unwrap_err(),
            RuntimeAuthorizationError::IncompleteRouteInventory
        );

        let mut wrong_resource = complete_rules();
        let ingress = ProtectedIngress::GitRead;
        wrong_resource[ingress as usize] = RouteRule::protected(
            ingress,
            ingress.required_capability(),
            ProtectedResourceKind::Domain,
            ingress.required_effect(),
            ingress.required_transport(),
        );
        assert_eq!(
            RouteAuthority::new(wrong_resource).unwrap_err(),
            RuntimeAuthorizationError::IncompleteRouteInventory
        );
    }

    #[test]
    fn transport_mismatch_fails_before_capability_use() {
        let authority = RouteAuthority::new(complete_rules()).unwrap();
        assert_eq!(
            authority
                .resolve(ProtectedIngress::GitRead, ProofTransport::Nip42)
                .unwrap_err(),
            RuntimeAuthorizationError::TransportMismatch
        );
        let resolved = authority
            .resolve(
                ProtectedIngress::GitRead,
                ProofTransport::GitSmartHttpSession,
            )
            .unwrap();
        assert_eq!(resolved.capability(), RouteCapability::GitRead);
    }

    #[test]
    fn complete_inventory_fingerprint_is_order_independent() {
        let first = RouteAuthority::new(complete_rules()).unwrap();
        let mut reversed = complete_rules();
        reversed.reverse();
        let second = RouteAuthority::new(reversed).unwrap();
        assert_eq!(
            first.inventory_fingerprint(),
            second.inventory_fingerprint()
        );
    }
}
