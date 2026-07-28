//! Channel-scoped Shivai world view binding contracts.

use std::collections::{HashMap, HashSet};

use hmac::{Hmac, KeyInit, Mac};
use nostr::{Event, EventId};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

/// Current serialized scoped world-view binding document version.
pub const WORLD_VIEW_BINDINGS_VERSION: u8 = 4;
/// Maximum number of world views that one channel or thread scope may bind.
pub const MAX_WORLD_VIEW_BINDINGS_PER_SCOPE: usize = 8;
/// Canonical parameterized-replaceable coordinate for channel bindings.
pub const CHANNEL_WORLD_VIEW_BINDINGS_D_TAG: &str = "world-view-bindings:channel";
/// Current private world-authority registry version.
pub const WORLD_AUTHORITY_REGISTRY_VERSION: u8 = 4;
/// Registry file shared by the desktop host and locally running ACP agents.
pub const WORLD_AUTHORITY_REGISTRY_FILE_NAME: &str = "world-authorities.json";
/// Private credential directory stored beside the authority registry.
pub const WORLD_AUTHORITY_SECRET_DIRECTORY: &str = "world-authority-secrets";
/// Version prefix for host-minted, revision-scoped world authority grants.
pub const WORLD_AUTHORITY_GRANT_VERSION: u8 = 1;
/// Short lifetime for prompt-carried grants before a fresh turn must remint.
pub const WORLD_AUTHORITY_GRANT_TTL_SECONDS: i64 = 15 * 60;
/// Shivai's canonical hosted origin, trusted by a fresh Buzz profile.
pub const DEFAULT_SHIVAI_WORLD_ORIGIN: &str = "https://manifest.shivai.space";

/// Private machine-local mappings from public world identities to mutation authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldAuthorityRegistry {
    /// Contract version for explicit forward evolution.
    pub version: u8,
    /// Hosted origins this device may contact while resolving public bindings.
    pub trusted_origins: Vec<String>,
    /// Mutable local packages behind public mirror identities.
    pub local_authorities: Vec<LocalWorldAuthority>,
    /// Private hosted edit-share credentials behind public hosted-world identities.
    pub hosted_authorities: Vec<HostedWorldAuthority>,
    /// Explicit device-local consent for agents to mutate one bound world.
    pub mutation_delegations: Vec<WorldViewMutationDelegation>,
}

/// One private local authority mapping. This shape must never be published to Nostr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalWorldAuthority {
    /// Hosted Shivai origin that owns the mirror identity.
    pub origin: String,
    /// Stable public mirror identity.
    pub mirror_id: String,
    /// Canonical absolute path of the mutable local `.world` package.
    pub source_root: String,
    /// Canonical absolute path of the owner-only local mutation grant secret.
    pub capability_secret_file: String,
}

/// One private hosted authority mapping. This shape must never be published to Nostr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostedWorldAuthority {
    /// Hosted Shivai origin that owns the world.
    pub origin: String,
    /// Stable public hosted-world identity.
    pub hosted_world_id: String,
    /// Canonical absolute path of the owner-only edit-share credential file.
    pub credential_file: String,
}

/// Credential-free identity of one mutable world source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WorldMutationAuthority {
    /// Mutable local package behind a public mirror.
    LocalWorldMirrorLatest {
        /// Hosted Shivai origin that owns the mirror identity.
        origin: String,
        /// Stable public mirror identity.
        #[serde(rename = "mirrorId")]
        mirror_id: String,
    },
    /// Mutable hosted world behind a private edit-share capability.
    HostedWorldLatest {
        /// Hosted Shivai origin that owns the world.
        origin: String,
        /// Stable public hosted-world identity.
        #[serde(rename = "hostedWorldId")]
        hosted_world_id: String,
    },
}

/// Explicit device-local consent for agents in one binding scope to mutate its world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewMutationDelegation {
    /// Channel containing the effective binding.
    pub channel_id: Uuid,
    /// Exact declaration scope that owns the binding.
    pub declared_scope: WorldViewBindingScope,
    /// Stable public binding identity.
    pub binding_id: Uuid,
    /// Exact Nostr event revision that published this binding declaration.
    pub binding_revision_event_id: String,
    /// Mutable world authority agents may exercise through a scoped host grant.
    pub authority: WorldMutationAuthority,
}

impl Default for WorldAuthorityRegistry {
    fn default() -> Self {
        Self {
            version: WORLD_AUTHORITY_REGISTRY_VERSION,
            trusted_origins: vec![DEFAULT_SHIVAI_WORLD_ORIGIN.into()],
            local_authorities: Vec::new(),
            hosted_authorities: Vec::new(),
            mutation_delegations: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorldAuthorityRegistryV3 {
    version: u8,
    local_authorities: Vec<LocalWorldAuthority>,
    hosted_authorities: Vec<HostedWorldAuthority>,
    mutation_delegations: Vec<WorldViewMutationDelegation>,
}

/// Decoded registry plus whether the caller must persist an explicit migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedWorldAuthorityRegistry {
    /// Validated current registry.
    pub registry: WorldAuthorityRegistry,
    /// Whether the decoded predecessor must be written back at the current version.
    pub migrated: bool,
}

/// Decode the current private registry or explicitly migrate its immediate predecessor.
pub fn decode_world_authority_registry(
    value: serde_json::Value,
) -> Result<DecodedWorldAuthorityRegistry, String> {
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "world authority registry is missing a numeric version".to_string())?;
    let (registry, migrated) = match version {
        3 => {
            let legacy: WorldAuthorityRegistryV3 = serde_json::from_value(value)
                .map_err(|error| format!("invalid v3 world authority registry: {error}"))?;
            if legacy.version != 3 {
                return Err(format!(
                    "unsupported world authority registry version: {}",
                    legacy.version
                ));
            }
            let trusted_origins =
                derive_trusted_world_origins(&legacy.local_authorities, &legacy.hosted_authorities);
            (
                WorldAuthorityRegistry {
                    version: WORLD_AUTHORITY_REGISTRY_VERSION,
                    trusted_origins,
                    local_authorities: legacy.local_authorities,
                    hosted_authorities: legacy.hosted_authorities,
                    mutation_delegations: legacy.mutation_delegations,
                },
                true,
            )
        }
        current if current == u64::from(WORLD_AUTHORITY_REGISTRY_VERSION) => (
            serde_json::from_value(value)
                .map_err(|error| format!("invalid world authority registry: {error}"))?,
            false,
        ),
        _ => {
            return Err(format!(
                "unsupported world authority registry version: {version}"
            ));
        }
    };
    registry.validate()?;
    Ok(DecodedWorldAuthorityRegistry { registry, migrated })
}

/// Derive explicit trust while migrating registries whose authorities implied admission.
pub fn derive_trusted_world_origins(
    local_authorities: &[LocalWorldAuthority],
    hosted_authorities: &[HostedWorldAuthority],
) -> Vec<String> {
    let mut origins = vec![DEFAULT_SHIVAI_WORLD_ORIGIN.to_owned()];
    origins.extend(
        local_authorities
            .iter()
            .map(|authority| authority.origin.clone()),
    );
    origins.extend(
        hosted_authorities
            .iter()
            .map(|authority| authority.origin.clone()),
    );
    origins.sort();
    origins.dedup();
    origins
}

impl WorldAuthorityRegistry {
    /// Validate registry identities, paths, and one-to-one mapping invariants.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != WORLD_AUTHORITY_REGISTRY_VERSION {
            return Err(format!(
                "unsupported world authority registry version: {}",
                self.version
            ));
        }
        let mut trusted_origins = HashSet::with_capacity(self.trusted_origins.len());
        for origin in &self.trusted_origins {
            validate_hosted_origin(origin)?;
            if !trusted_origins.insert(origin) {
                return Err(format!("duplicate trusted world origin: {origin}"));
            }
        }

        let mut local_references = HashSet::with_capacity(self.local_authorities.len());
        let mut roots = HashSet::with_capacity(self.local_authorities.len());
        let mut secret_files =
            HashSet::with_capacity(self.local_authorities.len() + self.hosted_authorities.len());
        for authority in &self.local_authorities {
            validate_hosted_origin(&authority.origin)?;
            if !trusted_origins.contains(&authority.origin) {
                return Err(format!(
                    "local world authority origin is not trusted: {}",
                    authority.origin
                ));
            }
            validate_required_text("mirrorId", &authority.mirror_id, 1024)?;
            validate_required_text("sourceRoot", &authority.source_root, 4096)?;
            validate_required_text(
                "capabilitySecretFile",
                &authority.capability_secret_file,
                4096,
            )?;
            if !std::path::Path::new(&authority.source_root).is_absolute() {
                return Err("sourceRoot must be an absolute path".into());
            }
            if !std::path::Path::new(&authority.capability_secret_file).is_absolute() {
                return Err("capabilitySecretFile must be an absolute path".into());
            }
            if !local_references.insert((&authority.origin, &authority.mirror_id)) {
                return Err(format!(
                    "duplicate local world authority: {} {}",
                    authority.origin, authority.mirror_id
                ));
            }
            if !roots.insert(&authority.source_root) {
                return Err(format!(
                    "duplicate local world source root: {}",
                    authority.source_root
                ));
            }
            if !secret_files.insert(&authority.capability_secret_file) {
                return Err(format!(
                    "duplicate world authority secret file: {}",
                    authority.capability_secret_file
                ));
            }
        }

        let mut hosted_references = HashSet::with_capacity(self.hosted_authorities.len());
        for authority in &self.hosted_authorities {
            validate_hosted_origin(&authority.origin)?;
            if !trusted_origins.contains(&authority.origin) {
                return Err(format!(
                    "hosted world authority origin is not trusted: {}",
                    authority.origin
                ));
            }
            validate_required_text("hostedWorldId", &authority.hosted_world_id, 1024)?;
            validate_required_text("credentialFile", &authority.credential_file, 4096)?;
            if !std::path::Path::new(&authority.credential_file).is_absolute() {
                return Err("credentialFile must be an absolute path".into());
            }
            if !hosted_references.insert((&authority.origin, &authority.hosted_world_id)) {
                return Err(format!(
                    "duplicate hosted world authority: {} {}",
                    authority.origin, authority.hosted_world_id
                ));
            }
            if !secret_files.insert(&authority.credential_file) {
                return Err(format!(
                    "duplicate world authority secret file: {}",
                    authority.credential_file
                ));
            }
        }

        let mut delegation_coordinates = HashSet::with_capacity(self.mutation_delegations.len());
        for delegation in &self.mutation_delegations {
            delegation.declared_scope.validate()?;
            validate_nostr_event_id(
                "mutationDelegation.bindingRevisionEventId",
                &delegation.binding_revision_event_id,
            )?;
            if !delegation_coordinates.insert((
                delegation.channel_id,
                delegation.declared_scope.d_tag(),
                delegation.binding_id,
            )) {
                return Err(format!(
                    "duplicate world mutation delegation: {} {} {}",
                    delegation.channel_id,
                    delegation.declared_scope.d_tag(),
                    delegation.binding_id,
                ));
            }
            if !self.mutation_authority_is_registered(&delegation.authority) {
                return Err(format!(
                    "world mutation delegation references an unregistered authority: {:?}",
                    delegation.authority
                ));
            }
        }
        Ok(())
    }

    /// Whether this device has explicitly admitted one canonical hosted origin.
    pub fn is_trusted_origin(&self, origin: &str) -> bool {
        self.trusted_origins.iter().any(|trusted| trusted == origin)
    }

    /// Admit one canonical hosted origin for public world-view resolution.
    pub fn trust_origin(&mut self, origin: String) -> Result<bool, String> {
        validate_hosted_origin(&origin)?;
        if self.is_trusted_origin(&origin) {
            return Ok(false);
        }
        self.trusted_origins.push(origin);
        self.trusted_origins.sort();
        self.validate()?;
        Ok(true)
    }

    /// Remove device-local trust when no connected authority still depends on it.
    pub fn revoke_origin_trust(&mut self, origin: &str) -> Result<bool, String> {
        if self
            .local_authorities
            .iter()
            .any(|authority| authority.origin == origin)
            || self
                .hosted_authorities
                .iter()
                .any(|authority| authority.origin == origin)
        {
            return Err(format!(
                "cannot revoke world origin trust while `{origin}` has connected authority"
            ));
        }
        let previous_len = self.trusted_origins.len();
        self.trusted_origins.retain(|trusted| trusted != origin);
        Ok(self.trusted_origins.len() != previous_len)
    }

    /// Resolve mutable local authority for one public mirror reference.
    pub fn resolve_local(&self, origin: &str, mirror_id: &str) -> Option<&LocalWorldAuthority> {
        self.local_authorities
            .iter()
            .find(|authority| authority.origin == origin && authority.mirror_id == mirror_id)
    }

    /// Resolve mutable hosted authority for one public hosted-world reference.
    pub fn resolve_hosted(
        &self,
        origin: &str,
        hosted_world_id: &str,
    ) -> Option<&HostedWorldAuthority> {
        self.hosted_authorities.iter().find(|authority| {
            authority.origin == origin && authority.hosted_world_id == hosted_world_id
        })
    }

    /// Resolve explicit mutation consent for one exact binding coordinate.
    pub fn resolve_mutation_delegation(
        &self,
        channel_id: Uuid,
        declared_scope: &WorldViewBindingScope,
        binding_id: Uuid,
        binding_revision_event_id: &str,
    ) -> Option<&WorldViewMutationDelegation> {
        self.mutation_delegations.iter().find(|delegation| {
            delegation.channel_id == channel_id
                && delegation.declared_scope == *declared_scope
                && delegation.binding_id == binding_id
                && delegation.binding_revision_event_id == binding_revision_event_id
        })
    }

    /// Whether one credential-free mutation identity is connected on this device.
    pub fn mutation_authority_is_registered(&self, authority: &WorldMutationAuthority) -> bool {
        match authority {
            WorldMutationAuthority::LocalWorldMirrorLatest { origin, mirror_id } => {
                self.resolve_local(origin, mirror_id).is_some()
            }
            WorldMutationAuthority::HostedWorldLatest {
                origin,
                hosted_world_id,
            } => self.resolve_hosted(origin, hosted_world_id).is_some(),
        }
    }

    /// Replace consent for one exact binding coordinate, then validate.
    pub fn upsert_mutation_delegation(
        &mut self,
        delegation: WorldViewMutationDelegation,
    ) -> Result<(), String> {
        self.mutation_delegations.retain(|candidate| {
            candidate.channel_id != delegation.channel_id
                || candidate.declared_scope != delegation.declared_scope
                || candidate.binding_id != delegation.binding_id
        });
        self.mutation_delegations.push(delegation);
        self.mutation_delegations.sort_by(|left, right| {
            (
                left.channel_id,
                left.declared_scope.d_tag(),
                left.binding_id,
            )
                .cmp(&(
                    right.channel_id,
                    right.declared_scope.d_tag(),
                    right.binding_id,
                ))
        });
        self.validate()
    }

    /// Revoke mutation consent for one exact binding coordinate.
    pub fn revoke_mutation_delegation(
        &mut self,
        channel_id: Uuid,
        declared_scope: &WorldViewBindingScope,
        binding_id: Uuid,
    ) -> bool {
        let previous_len = self.mutation_delegations.len();
        self.mutation_delegations.retain(|delegation| {
            delegation.channel_id != channel_id
                || delegation.declared_scope != *declared_scope
                || delegation.binding_id != binding_id
        });
        self.mutation_delegations.len() != previous_len
    }

    /// Replace local mappings that share either identity or source, then validate.
    pub fn upsert_local(&mut self, authority: LocalWorldAuthority) -> Result<(), String> {
        self.trust_origin(authority.origin.clone())?;
        self.local_authorities.retain(|candidate| {
            (candidate.origin != authority.origin || candidate.mirror_id != authority.mirror_id)
                && candidate.source_root != authority.source_root
                && candidate.capability_secret_file != authority.capability_secret_file
        });
        self.local_authorities.push(authority);
        self.local_authorities.sort_by(|left, right| {
            (&left.origin, &left.mirror_id).cmp(&(&right.origin, &right.mirror_id))
        });
        self.retain_registered_mutation_delegations();
        self.validate()
    }

    /// Replace hosted mappings that share either identity or credential, then validate.
    pub fn upsert_hosted(&mut self, authority: HostedWorldAuthority) -> Result<(), String> {
        self.trust_origin(authority.origin.clone())?;
        self.hosted_authorities.retain(|candidate| {
            (candidate.origin != authority.origin
                || candidate.hosted_world_id != authority.hosted_world_id)
                && candidate.credential_file != authority.credential_file
        });
        self.hosted_authorities.push(authority);
        self.hosted_authorities.sort_by(|left, right| {
            (&left.origin, &left.hosted_world_id).cmp(&(&right.origin, &right.hosted_world_id))
        });
        self.retain_registered_mutation_delegations();
        self.validate()
    }

    fn retain_registered_mutation_delegations(&mut self) {
        let local_authorities = &self.local_authorities;
        let hosted_authorities = &self.hosted_authorities;
        self.mutation_delegations
            .retain(|delegation| match &delegation.authority {
                WorldMutationAuthority::LocalWorldMirrorLatest { origin, mirror_id } => {
                    local_authorities.iter().any(|authority| {
                        authority.origin == *origin && authority.mirror_id == *mirror_id
                    })
                }
                WorldMutationAuthority::HostedWorldLatest {
                    origin,
                    hosted_world_id,
                } => hosted_authorities.iter().any(|authority| {
                    authority.origin == *origin && authority.hosted_world_id == *hosted_world_id
                }),
            });
    }
}

/// Exact channel or thread-root scope owned by one binding document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WorldViewBindingScope {
    /// Bindings declared directly on the channel.
    Channel,
    /// Bindings declared on one canonical thread root.
    Thread {
        /// Lowercase Nostr event id of the canonical thread root.
        #[serde(rename = "threadRootEventId")]
        thread_root_event_id: String,
    },
}

impl WorldViewBindingScope {
    /// Construct a validated thread scope.
    pub fn thread(thread_root_event_id: impl Into<String>) -> Result<Self, String> {
        let scope = Self::Thread {
            thread_root_event_id: thread_root_event_id.into(),
        };
        scope.validate()?;
        Ok(scope)
    }

    /// Stable `d` tag used by relay replacement and exact-scope reads.
    pub fn d_tag(&self) -> String {
        match self {
            Self::Channel => CHANNEL_WORLD_VIEW_BINDINGS_D_TAG.into(),
            Self::Thread {
                thread_root_event_id,
            } => format!("world-view-bindings:thread:{thread_root_event_id}"),
        }
    }

    /// Canonical thread root when this is a thread scope.
    pub fn thread_root_event_id(&self) -> Option<&str> {
        match self {
            Self::Channel => None,
            Self::Thread {
                thread_root_event_id,
            } => Some(thread_root_event_id),
        }
    }

    /// Validate the serialized scope identity.
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Thread {
            thread_root_event_id,
        } = self
        {
            validate_nostr_event_id("scope.threadRootEventId", thread_root_event_id)?;
        }
        Ok(())
    }
}

impl Default for WorldViewBindingScope {
    fn default() -> Self {
        Self::Channel
    }
}

/// Opaque grant scope signed by the host from private world authority.
///
/// Every field is immutable input to the signature. A grant therefore cannot
/// be retargeted to another agent, conversation scope, binding revision, or
/// world source revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldAuthorityGrantScope {
    /// Managed Buzz agent allowed to exercise the grant.
    pub agent_pubkey: String,
    /// Channel containing the effective binding.
    pub channel_id: Uuid,
    /// Exact channel or thread scope active for this turn.
    pub effective_scope: WorldViewBindingScope,
    /// Opaque public binding id resolved by the host.
    pub binding_id: Uuid,
    /// Nostr event revision that defined the effective binding.
    pub binding_revision_event_id: String,
    /// Hosted package revision accepted by the mutation.
    pub source_revision: String,
}

impl WorldAuthorityGrantScope {
    /// Validate every identity included in a host grant.
    pub fn validate(&self) -> Result<(), String> {
        validate_nostr_event_id("grant.agentPubkey", &self.agent_pubkey)?;
        self.effective_scope.validate()?;
        validate_nostr_event_id(
            "grant.bindingRevisionEventId",
            &self.binding_revision_event_id,
        )?;
        validate_nostr_event_id("grant.sourceRevision", &self.source_revision)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorldAuthorityGrantClaims {
    expires_at_unix_seconds: i64,
    scope: WorldAuthorityGrantScope,
}

/// Mint one opaque, expiring grant from private authority material.
///
/// The token contains only signed public scope data. It never contains the
/// private edit-share value used as the HMAC key.
pub fn issue_world_authority_grant(
    scope: &WorldAuthorityGrantScope,
    authority_secret: &[u8],
    expires_at_unix_seconds: i64,
) -> Result<String, String> {
    scope.validate()?;
    if authority_secret.is_empty() {
        return Err("world authority secret must not be empty".into());
    }
    if expires_at_unix_seconds <= 0 {
        return Err("world authority grant expiry must be positive".into());
    }
    let payload = serde_json::to_vec(&WorldAuthorityGrantClaims {
        expires_at_unix_seconds,
        scope: scope.clone(),
    })
    .map_err(|error| format!("encode world authority grant: {error}"))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(authority_secret)
        .map_err(|_| "world authority secret is invalid".to_owned())?;
    mac.update(&payload);
    let signature = mac.finalize().into_bytes();
    Ok(format!(
        "wvg{}.{}.{}",
        WORLD_AUTHORITY_GRANT_VERSION,
        hex::encode(payload),
        hex::encode(signature)
    ))
}

/// Verify that an unexpired opaque host grant authorizes one exact scope.
pub fn verify_world_authority_grant(
    token: &str,
    authority_secret: &[u8],
    expected_scope: &WorldAuthorityGrantScope,
    now_unix_seconds: i64,
) -> Result<(), String> {
    expected_scope.validate()?;
    if authority_secret.is_empty() || token.len() > 16_384 {
        return Err("invalid world authority grant".into());
    }
    let mut parts = token.split('.');
    let expected_prefix = format!("wvg{}", WORLD_AUTHORITY_GRANT_VERSION);
    if parts.next() != Some(expected_prefix.as_str()) {
        return Err("invalid world authority grant".into());
    }
    let payload = parts
        .next()
        .ok_or_else(|| "invalid world authority grant".to_owned())
        .and_then(|value| {
            hex::decode(value).map_err(|_| "invalid world authority grant".to_owned())
        })?;
    let signature = parts
        .next()
        .ok_or_else(|| "invalid world authority grant".to_owned())
        .and_then(|value| {
            hex::decode(value).map_err(|_| "invalid world authority grant".to_owned())
        })?;
    if parts.next().is_some() {
        return Err("invalid world authority grant".into());
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(authority_secret)
        .map_err(|_| "invalid world authority grant".to_owned())?;
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| "invalid world authority grant".to_owned())?;
    let claims: WorldAuthorityGrantClaims =
        serde_json::from_slice(&payload).map_err(|_| "invalid world authority grant".to_owned())?;
    claims.scope.validate()?;
    if &claims.scope != expected_scope {
        return Err("world authority grant does not match this request".into());
    }
    if now_unix_seconds >= claims.expires_at_unix_seconds {
        return Err("world authority grant expired".into());
    }
    Ok(())
}

/// One exact-scope document containing every bound Shivai world view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewBindingsDocument {
    /// Contract version for explicit forward evolution.
    pub version: u8,
    /// Exact channel or thread-root scope represented by this document.
    pub scope: WorldViewBindingScope,
    /// Ordered views rendered by clients and supplied to agents.
    pub bindings: Vec<WorldViewBinding>,
}

/// Exact-scope binding state plus the relay revision needed for the next write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewBindingsSnapshot {
    /// Current document, or an empty document for an absent coordinate.
    pub document: WorldViewBindingsDocument,
    /// Current event id; `None` means the next write must explicitly expect creation.
    pub revision_event_id: Option<String>,
    /// Relay event timestamp for the current revision.
    pub updated_at: Option<u64>,
    /// Public key that authored the current revision.
    pub author: Option<String>,
}

impl WorldViewBindingsSnapshot {
    /// Construct an absent exact-scope snapshot.
    pub fn empty(scope: WorldViewBindingScope) -> Self {
        Self {
            document: WorldViewBindingsDocument {
                version: WORLD_VIEW_BINDINGS_VERSION,
                scope,
                bindings: Vec::new(),
            },
            revision_event_id: None,
            updated_at: None,
            author: None,
        }
    }
}

/// Structurally decoded state from one already signature-verified bindings event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedWorldViewBindingsEvent {
    /// Exact channel coordinate carried by the event's sole `h` tag.
    pub channel_id: Uuid,
    /// Exact-scope state represented by this event.
    pub snapshot: WorldViewBindingsSnapshot,
    /// Revision that this event expected to replace; `None` means creation.
    pub previous_revision_event_id: Option<EventId>,
}

/// Decode the canonical signed-event envelope after signature verification.
///
/// This validates kind, channel/scope coordinates, optimistic revision, thread
/// root tags, and strict JSON content in one source-shaped boundary. Callers at
/// untrusted relay boundaries must verify the event signature before invoking
/// this CPU-cheap structural decoder.
pub fn decode_verified_world_view_bindings_event(
    event: &Event,
) -> Result<DecodedWorldViewBindingsEvent, String> {
    if event.kind.as_u16() as u32 != crate::kind::KIND_WORLD_VIEW_BINDINGS {
        return Err("world-view bindings event has the wrong kind".into());
    }

    let document: WorldViewBindingsDocument = serde_json::from_str(&event.content)
        .map_err(|error| format!("world-view bindings content is invalid: {error}"))?;
    document.validate()?;

    let h_tags = exact_event_tags(event, "h");
    if h_tags.len() != 1 || h_tags[0].len() != 2 {
        return Err("world-view bindings require one exact channel h tag".into());
    }
    let channel_id = Uuid::parse_str(&h_tags[0][1])
        .map_err(|_| "world-view bindings require one exact channel h tag".to_string())?;

    let d_tag = document.scope.d_tag();
    let d_tags = exact_event_tags(event, "d");
    if d_tags.len() != 1 || d_tags[0].len() != 2 || d_tags[0][1] != d_tag {
        return Err("world-view bindings d tag does not match document scope".into());
    }

    let previous_tags = exact_event_tags(event, "prev");
    if previous_tags.len() != 1 || previous_tags[0].len() != 2 {
        return Err("world-view bindings require one exact prev tag".into());
    }
    let previous = &previous_tags[0][1];
    let previous_revision_event_id = if previous.is_empty() {
        None
    } else {
        validate_nostr_event_id("world-view bindings prev tag", previous)?;
        Some(
            EventId::from_hex(previous)
                .map_err(|_| "world-view bindings prev tag is not valid hex".to_string())?,
        )
    };

    let root_event_id = document.scope.thread_root_event_id();
    let e_tags = exact_event_tags(event, "e");
    match root_event_id {
        Some(root)
            if e_tags.len() != 1
                || e_tags[0].len() != 4
                || e_tags[0][1] != root
                || !e_tags[0][2].is_empty()
                || e_tags[0][3] != "root" =>
        {
            return Err("thread world-view bindings require one canonical root e tag".into());
        }
        None if !e_tags.is_empty() => {
            return Err("channel world-view bindings must not carry e tags".into());
        }
        Some(_) | None => {}
    }

    Ok(DecodedWorldViewBindingsEvent {
        channel_id,
        snapshot: WorldViewBindingsSnapshot {
            document,
            revision_event_id: Some(event.id.to_hex()),
            updated_at: Some(event.created_at.as_secs()),
            author: Some(event.pubkey.to_hex()),
        },
        previous_revision_event_id,
    })
}

/// Decode one already verified event and require an exact requested coordinate.
pub fn world_view_bindings_snapshot_from_verified_event(
    event: &Event,
    expected_channel_id: Uuid,
    expected_scope: &WorldViewBindingScope,
) -> Result<WorldViewBindingsSnapshot, String> {
    let decoded = decode_verified_world_view_bindings_event(event)?;
    if decoded.channel_id != expected_channel_id {
        return Err("world-view bindings event channel did not match its relay coordinate".into());
    }
    if &decoded.snapshot.document.scope != expected_scope {
        return Err("world-view bindings event scope did not match its relay coordinate".into());
    }
    Ok(decoded.snapshot)
}

fn exact_event_tags<'a>(event: &'a Event, name: &str) -> Vec<&'a [String]> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|parts| parts.first().is_some_and(|part| part == name))
        .collect()
}
/// One effective binding with the exact declaration that currently owns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveWorldViewBinding {
    /// Bound view selected after applying thread shadowing.
    pub binding: WorldViewBinding,
    /// Exact channel or thread-root scope that declared this binding.
    pub declared_scope: WorldViewBindingScope,
    /// Relay revision event that supplied this binding.
    pub binding_revision_event_id: String,
}

/// Effective bindings for one channel turn, with optional thread inheritance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveWorldViewBindings {
    /// Scope in which the views are being consumed.
    pub effective_scope: WorldViewBindingScope,
    /// Channel order with same-id thread overrides and new thread views appended.
    pub bindings: Vec<EffectiveWorldViewBinding>,
    /// Current exact channel document revision, when present.
    pub channel_revision_event_id: Option<String>,
    /// Current exact thread document revision, when present.
    pub thread_revision_event_id: Option<String>,
}

/// Merge exact channel and thread-root snapshots into one effective declaration.
///
/// A thread binding with the same stable id replaces its channel declaration
/// in place. New thread bindings append in authored order.
pub fn effective_world_view_bindings(
    channel: &WorldViewBindingsSnapshot,
    thread: Option<&WorldViewBindingsSnapshot>,
) -> Result<EffectiveWorldViewBindings, String> {
    if channel.document.scope != WorldViewBindingScope::Channel {
        return Err("channel inheritance source must declare channel scope".into());
    }

    let effective_scope = thread
        .map(|snapshot| snapshot.document.scope.clone())
        .unwrap_or(WorldViewBindingScope::Channel);
    let mut bindings = Vec::with_capacity(
        channel.document.bindings.len()
            + thread
                .map(|snapshot| snapshot.document.bindings.len())
                .unwrap_or_default(),
    );
    let mut positions = HashMap::with_capacity(channel.document.bindings.len());

    if let Some(revision_event_id) = channel.revision_event_id.as_ref() {
        for binding in &channel.document.bindings {
            positions.insert(binding.id, bindings.len());
            bindings.push(EffectiveWorldViewBinding {
                binding: binding.clone(),
                declared_scope: WorldViewBindingScope::Channel,
                binding_revision_event_id: revision_event_id.clone(),
            });
        }
    } else if !channel.document.bindings.is_empty() {
        return Err("channel bindings require a source revision event id".into());
    }

    if let Some(thread) = thread {
        if !matches!(thread.document.scope, WorldViewBindingScope::Thread { .. }) {
            return Err("thread override source must declare thread scope".into());
        }
        if let Some(revision_event_id) = thread.revision_event_id.as_ref() {
            for binding in &thread.document.bindings {
                let effective = EffectiveWorldViewBinding {
                    binding: binding.clone(),
                    declared_scope: thread.document.scope.clone(),
                    binding_revision_event_id: revision_event_id.clone(),
                };
                if let Some(position) = positions.get(&binding.id).copied() {
                    bindings[position] = effective;
                } else {
                    positions.insert(binding.id, bindings.len());
                    bindings.push(effective);
                }
            }
        } else if !thread.document.bindings.is_empty() {
            return Err("thread bindings require a source revision event id".into());
        }
    }

    Ok(EffectiveWorldViewBindings {
        effective_scope,
        bindings,
        channel_revision_event_id: channel.revision_event_id.clone(),
        thread_revision_event_id: thread.and_then(|snapshot| snapshot.revision_event_id.clone()),
    })
}

/// A single channel-bound Shivai world view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewBinding {
    /// Stable binding identity used by clients when views are reordered or replaced.
    pub id: Uuid,
    /// Optional channel-authored label shown above the rendered view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Source authority for resolving the world snapshot.
    pub reference: WorldViewReference,
    /// Qualified realm name selected inside the world.
    pub realm_qualified_name: String,
    /// Qualified view name selected inside the realm.
    pub view_qualified_name: String,
    /// Initial presentation selected by the channel author.
    pub display_mode: WorldViewDisplayMode,
}

/// A supported authority for resolving a bound world view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WorldViewReference {
    /// The latest read-only projection of a stable published local world mirror.
    LocalWorldMirrorLatest {
        /// Hosted Shivai origin serving the public mirror projection.
        origin: String,
        /// Stable mirror identity; revisions advance without replacing this binding.
        #[serde(rename = "mirrorId")]
        mirror_id: String,
    },
    /// A read-only hosted world view export shared by bearer token.
    HostedWorldViewExport {
        /// Hosted Shivai origin serving the public export.
        origin: String,
        /// Public read-only export token.
        #[serde(rename = "shareToken")]
        share_token: String,
    },
    /// A stable public view capability that follows the hosted world's latest revision.
    HostedWorldLiveViewShare {
        /// Hosted Shivai origin serving the public live view.
        origin: String,
        /// Stable public read-only live-view share token.
        #[serde(rename = "shareToken")]
        share_token: String,
    },
    /// The latest projection of a hosted world, authorized privately on each client.
    HostedWorldLatest {
        /// Hosted Shivai origin serving and mutating the world.
        origin: String,
        /// Stable public hosted-world identity.
        #[serde(rename = "hostedWorldId")]
        hosted_world_id: String,
    },
}

impl WorldViewReference {
    /// Canonical hosted origin named by this public reference.
    pub fn origin(&self) -> &str {
        match self {
            Self::LocalWorldMirrorLatest { origin, .. }
            | Self::HostedWorldViewExport { origin, .. }
            | Self::HostedWorldLiveViewShare { origin, .. }
            | Self::HostedWorldLatest { origin, .. } => origin,
        }
    }
    /// Validate one public source identity without requiring a bound realm/view.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::LocalWorldMirrorLatest { origin, mirror_id } => {
                validate_hosted_origin(origin)?;
                validate_required_text("reference.mirrorId", mirror_id, 1024)
            }
            Self::HostedWorldViewExport {
                origin,
                share_token,
            } => {
                validate_hosted_origin(origin)?;
                validate_required_text("reference.shareToken", share_token, 1024)
            }
            Self::HostedWorldLiveViewShare {
                origin,
                share_token,
            } => {
                validate_hosted_origin(origin)?;
                validate_required_text("reference.shareToken", share_token, 1024)
            }
            Self::HostedWorldLatest {
                origin,
                hosted_world_id,
            } => {
                validate_hosted_origin(origin)?;
                validate_required_text("reference.hostedWorldId", hosted_world_id, 1024)
            }
        }
    }
}

/// Initial channel presentation for a bound world view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewDisplayMode {
    /// Interactive Shivai dependency graph.
    Graph,
    /// Plain ordered task list.
    Tasks,
}

impl WorldViewBindingsDocument {
    /// Validate scope, limits, and identity invariants before publication.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != WORLD_VIEW_BINDINGS_VERSION {
            return Err(format!(
                "unsupported world view bindings version: {}",
                self.version
            ));
        }
        self.scope.validate()?;
        if self.bindings.len() > MAX_WORLD_VIEW_BINDINGS_PER_SCOPE {
            return Err(format!(
                "a scope may bind at most {MAX_WORLD_VIEW_BINDINGS_PER_SCOPE} world views"
            ));
        }

        let mut ids = HashSet::with_capacity(self.bindings.len());
        for binding in &self.bindings {
            if !ids.insert(binding.id) {
                return Err(format!("duplicate world view binding id: {}", binding.id));
            }
            validate_required_text("realmQualifiedName", &binding.realm_qualified_name, 512)?;
            validate_required_text("viewQualifiedName", &binding.view_qualified_name, 512)?;
            if let Some(label) = &binding.label {
                validate_required_text("label", label, 160)?;
            }
            binding.reference.validate()?;
        }
        Ok(())
    }
}

fn validate_hosted_origin(value: &str) -> Result<(), String> {
    validate_required_text("reference.origin", value, 2048)?;
    let parsed = url::Url::parse(value)
        .map_err(|error| format!("reference.origin must be an absolute URL: {error}"))?;
    let is_loopback_http = parsed.scheme() == "http"
        && parsed.host().is_some_and(|host| match host {
            url::Host::Domain(domain) => domain == "localhost",
            url::Host::Ipv4(address) => address.is_loopback(),
            url::Host::Ipv6(address) => address.is_loopback(),
        });
    if parsed.scheme() != "https" && !is_loopback_http {
        return Err(
            "reference.origin must use https (http is allowed only for loopback development)"
                .into(),
        );
    }
    if parsed.origin().ascii_serialization() != value {
        return Err("reference.origin must contain only scheme, host, and optional port".into());
    }
    Ok(())
}

fn validate_nostr_event_id(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{field} must be 64 lowercase hex characters"));
    }
    Ok(())
}

fn validate_required_text(field: &str, value: &str, max_len: usize) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be blank"));
    }
    if trimmed.len() > max_len {
        return Err(format!("{field} exceeds {max_len} bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(id: Uuid) -> WorldViewBinding {
        WorldViewBinding {
            id,
            label: Some("Launch board".into()),
            reference: WorldViewReference::HostedWorldViewExport {
                origin: "https://manifest.shivai.space".into(),
                share_token: "view-token".into(),
            },
            realm_qualified_name: "world::main".into(),
            view_qualified_name: "world::main::@Board".into(),
            display_mode: WorldViewDisplayMode::Graph,
        }
    }

    fn signed_bindings_event(
        channel_id: Uuid,
        document: &WorldViewBindingsDocument,
        previous_revision_event_id: &str,
        root_override: Option<&str>,
    ) -> Event {
        use nostr::{EventBuilder, Kind, Tag};

        let mut tags = vec![
            Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag"),
            Tag::parse(["d", document.scope.d_tag().as_str()]).expect("d tag"),
            Tag::parse(["prev", previous_revision_event_id]).expect("prev tag"),
        ];
        if let Some(root) = root_override.or(document.scope.thread_root_event_id()) {
            tags.push(Tag::parse(["e", root, "", "root"]).expect("e tag"));
        }
        EventBuilder::new(
            Kind::Custom(crate::kind::KIND_WORLD_VIEW_BINDINGS as u16),
            serde_json::to_string(document).expect("serialize"),
        )
        .tags(tags)
        .sign_with_keys(&nostr::Keys::generate())
        .expect("sign")
    }

    #[test]
    fn round_trips_the_versioned_binding_document() {
        let document = WorldViewBindingsDocument {
            version: WORLD_VIEW_BINDINGS_VERSION,
            scope: WorldViewBindingScope::Channel,
            bindings: vec![binding(Uuid::nil())],
        };

        let encoded = serde_json::to_value(&document).expect("serialize");
        assert_eq!(
            encoded["bindings"][0]["reference"]["kind"],
            "hosted-world-view-export"
        );
        assert_eq!(
            encoded["bindings"][0]["reference"]["origin"],
            "https://manifest.shivai.space"
        );
        assert_eq!(encoded["scope"]["kind"], "channel");
        assert_eq!(encoded["bindings"][0]["displayMode"], "graph");
        let decoded: WorldViewBindingsDocument =
            serde_json::from_value(encoded).expect("deserialize");
        assert_eq!(decoded, document);
        assert_eq!(decoded.validate(), Ok(()));
    }

    #[test]
    fn decodes_verified_binding_event_into_typed_revision_state() {
        let channel_id = Uuid::new_v4();
        let document = WorldViewBindingsDocument {
            version: WORLD_VIEW_BINDINGS_VERSION,
            scope: WorldViewBindingScope::Channel,
            bindings: vec![binding(Uuid::nil())],
        };
        let event = signed_bindings_event(channel_id, &document, &"a".repeat(64), None);

        let decoded =
            decode_verified_world_view_bindings_event(&event).expect("decode canonical event");

        assert_eq!(decoded.channel_id, channel_id);
        assert_eq!(decoded.snapshot.document, document);
        assert_eq!(
            decoded.previous_revision_event_id.map(|id| id.to_hex()),
            Some("a".repeat(64))
        );
        assert_eq!(decoded.snapshot.revision_event_id, Some(event.id.to_hex()));
    }

    #[test]
    fn rejects_mismatched_thread_root_at_the_shared_event_boundary() {
        let channel_id = Uuid::new_v4();
        let document = WorldViewBindingsDocument {
            version: WORLD_VIEW_BINDINGS_VERSION,
            scope: WorldViewBindingScope::thread("a".repeat(64)).expect("thread scope"),
            bindings: Vec::new(),
        };
        let event = signed_bindings_event(
            channel_id,
            &document,
            &"b".repeat(64),
            Some(&"c".repeat(64)),
        );

        assert_eq!(
            decode_verified_world_view_bindings_event(&event).map(|_| ()),
            Err("thread world-view bindings require one canonical root e tag".into())
        );
    }

    #[test]
    fn rejects_unknown_nested_binding_and_reference_fields() {
        let document = WorldViewBindingsDocument {
            version: WORLD_VIEW_BINDINGS_VERSION,
            scope: WorldViewBindingScope::Channel,
            bindings: vec![binding(Uuid::nil())],
        };
        let mut binding_value = serde_json::to_value(&document).expect("serialize");
        binding_value["bindings"][0]["unexpected"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<WorldViewBindingsDocument>(binding_value)
                .expect_err("binding extras must fail")
                .to_string()
                .contains("unknown field")
        );

        let mut reference_value = serde_json::to_value(&document).expect("serialize");
        reference_value["bindings"][0]["reference"]["accessToken"] =
            serde_json::json!("must-not-cross-boundary");
        assert!(
            serde_json::from_value::<WorldViewBindingsDocument>(reference_value)
                .expect_err("reference extras must fail")
                .to_string()
                .contains("unknown field")
        );
    }

    #[test]
    fn thread_bindings_shadow_channel_ids_without_reordering_inherited_views() {
        let inherited_id = Uuid::nil();
        let shadowed_id = Uuid::from_u128(1);
        let appended_id = Uuid::from_u128(2);
        let channel = WorldViewBindingsSnapshot {
            document: WorldViewBindingsDocument {
                version: WORLD_VIEW_BINDINGS_VERSION,
                scope: WorldViewBindingScope::Channel,
                bindings: vec![binding(inherited_id), binding(shadowed_id)],
            },
            revision_event_id: Some("a".repeat(64)),
            updated_at: Some(1),
            author: Some("channel-author".into()),
        };
        let thread_scope = WorldViewBindingScope::Thread {
            thread_root_event_id: "b".repeat(64),
        };
        let mut shadow = binding(shadowed_id);
        shadow.label = Some("Thread override".into());
        let mut appended = binding(appended_id);
        appended.label = Some("Thread only".into());
        let thread = WorldViewBindingsSnapshot {
            document: WorldViewBindingsDocument {
                version: WORLD_VIEW_BINDINGS_VERSION,
                scope: thread_scope.clone(),
                bindings: vec![shadow, appended],
            },
            revision_event_id: Some("c".repeat(64)),
            updated_at: Some(2),
            author: Some("thread-author".into()),
        };

        let effective =
            effective_world_view_bindings(&channel, Some(&thread)).expect("merge effective views");

        assert_eq!(effective.effective_scope, thread_scope);
        assert_eq!(
            effective
                .bindings
                .iter()
                .map(|entry| entry.binding.id)
                .collect::<Vec<_>>(),
            vec![inherited_id, shadowed_id, appended_id]
        );
        assert_eq!(
            effective.bindings[0].declared_scope,
            WorldViewBindingScope::Channel
        );
        assert_eq!(
            effective.bindings[0].binding_revision_event_id,
            "a".repeat(64)
        );
        assert_eq!(
            effective.bindings[1].declared_scope,
            effective.effective_scope
        );
        assert_eq!(
            effective.bindings[1].binding.label.as_deref(),
            Some("Thread override")
        );
        assert_eq!(
            effective.bindings[2].binding.label.as_deref(),
            Some("Thread only")
        );
        assert_eq!(effective.thread_revision_event_id, Some("c".repeat(64)));
    }

    #[test]
    fn rejects_non_origin_and_insecure_remote_urls() {
        let mut document = WorldViewBindingsDocument {
            version: WORLD_VIEW_BINDINGS_VERSION,
            scope: WorldViewBindingScope::Channel,
            bindings: vec![binding(Uuid::nil())],
        };
        if let WorldViewReference::HostedWorldViewExport { origin, .. } =
            &mut document.bindings[0].reference
        {
            *origin = "https://manifest.shivai.space/world/export".into();
        }
        assert_eq!(
            document.validate(),
            Err("reference.origin must contain only scheme, host, and optional port".into())
        );

        if let WorldViewReference::HostedWorldViewExport { origin, .. } =
            &mut document.bindings[0].reference
        {
            *origin = "https://manifest.shivai.space/".into();
        }
        assert_eq!(
            document.validate(),
            Err("reference.origin must contain only scheme, host, and optional port".into())
        );

        if let WorldViewReference::HostedWorldViewExport { origin, .. } =
            &mut document.bindings[0].reference
        {
            *origin = "http://manifest.shivai.space".into();
        }
        assert_eq!(
            document.validate(),
            Err(
                "reference.origin must use https (http is allowed only for loopback development)"
                    .into()
            )
        );
    }

    #[test]
    fn world_authority_registry_preserves_one_to_one_mappings() {
        let mut registry = WorldAuthorityRegistry::default();
        registry
            .upsert_local(LocalWorldAuthority {
                origin: "https://manifest.shivai.space".into(),
                mirror_id: "mirror-1".into(),
                source_root: "/worlds/one.world".into(),
                capability_secret_file: "/credentials/local-one.txt".into(),
            })
            .unwrap();
        registry
            .upsert_local(LocalWorldAuthority {
                origin: "https://manifest.shivai.space".into(),
                mirror_id: "mirror-2".into(),
                source_root: "/worlds/one.world".into(),
                capability_secret_file: "/credentials/local-two.txt".into(),
            })
            .unwrap();
        registry
            .upsert_hosted(HostedWorldAuthority {
                origin: "https://manifest.shivai.space".into(),
                hosted_world_id: "hosted-1".into(),
                credential_file: "/credentials/one.txt".into(),
            })
            .unwrap();
        registry
            .upsert_hosted(HostedWorldAuthority {
                origin: "https://manifest.shivai.space".into(),
                hosted_world_id: "hosted-2".into(),
                credential_file: "/credentials/one.txt".into(),
            })
            .unwrap();

        assert_eq!(registry.local_authorities.len(), 1);
        assert!(registry
            .resolve_local("https://manifest.shivai.space", "mirror-1")
            .is_none());
        assert_eq!(
            registry
                .resolve_local("https://manifest.shivai.space", "mirror-2")
                .map(|authority| authority.source_root.as_str()),
            Some("/worlds/one.world")
        );
        assert_eq!(registry.hosted_authorities.len(), 1);
        assert!(registry
            .resolve_hosted("https://manifest.shivai.space", "hosted-1")
            .is_none());
        assert_eq!(
            registry
                .resolve_hosted("https://manifest.shivai.space", "hosted-2")
                .map(|authority| authority.credential_file.as_str()),
            Some("/credentials/one.txt")
        );
    }

    #[test]
    fn world_origin_trust_is_explicit_canonical_and_authority_aware() {
        let mut registry = WorldAuthorityRegistry::default();

        assert!(registry.is_trusted_origin(DEFAULT_SHIVAI_WORLD_ORIGIN));
        assert!(!registry.is_trusted_origin("https://untrusted.example"));
        assert_eq!(
            registry
                .trust_origin("https://trusted.example".into())
                .unwrap(),
            true
        );
        assert_eq!(
            registry
                .trust_origin("https://trusted.example".into())
                .unwrap(),
            false
        );

        registry
            .upsert_local(LocalWorldAuthority {
                origin: "https://connected.example".into(),
                mirror_id: "mirror-1".into(),
                source_root: "/worlds/connected.world".into(),
                capability_secret_file: "/credentials/connected.txt".into(),
            })
            .unwrap();
        assert!(registry.is_trusted_origin("https://connected.example"));
        assert_eq!(
            registry
                .revoke_origin_trust("https://connected.example")
                .unwrap_err(),
            "cannot revoke world origin trust while `https://connected.example` has connected authority"
        );
        assert!(registry
            .revoke_origin_trust("https://trusted.example")
            .unwrap());
        assert!(!registry.is_trusted_origin("https://trusted.example"));
        assert_eq!(registry.validate(), Ok(()));
    }

    #[test]
    fn world_mutation_delegation_is_exact_replaceable_and_revocable() {
        let channel_id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
        let binding_id = Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
        let mut registry = WorldAuthorityRegistry::default();
        let binding_revision_event_id = "d".repeat(64);
        registry
            .upsert_local(LocalWorldAuthority {
                origin: "https://manifest.shivai.space".into(),
                mirror_id: "mirror-1".into(),
                source_root: "/worlds/one.world".into(),
                capability_secret_file: "/credentials/local-one.txt".into(),
            })
            .unwrap();
        registry
            .upsert_hosted(HostedWorldAuthority {
                origin: "https://manifest.shivai.space".into(),
                hosted_world_id: "hosted-1".into(),
                credential_file: "/credentials/hosted-one.txt".into(),
            })
            .unwrap();
        registry
            .upsert_mutation_delegation(WorldViewMutationDelegation {
                channel_id,
                declared_scope: WorldViewBindingScope::Channel,
                binding_id,
                binding_revision_event_id: binding_revision_event_id.clone(),
                authority: WorldMutationAuthority::HostedWorldLatest {
                    origin: "https://manifest.shivai.space".into(),
                    hosted_world_id: "hosted-1".into(),
                },
            })
            .unwrap();

        assert_eq!(
            registry
                .resolve_mutation_delegation(
                    channel_id,
                    &WorldViewBindingScope::Channel,
                    binding_id,
                    &binding_revision_event_id,
                )
                .map(|delegation| &delegation.authority),
            Some(&WorldMutationAuthority::HostedWorldLatest {
                origin: "https://manifest.shivai.space".into(),
                hosted_world_id: "hosted-1".into(),
            })
        );
        assert!(registry
            .resolve_mutation_delegation(
                channel_id,
                &WorldViewBindingScope::thread("c".repeat(64)).unwrap(),
                binding_id,
                &binding_revision_event_id,
            )
            .is_none());

        registry
            .upsert_mutation_delegation(WorldViewMutationDelegation {
                channel_id,
                declared_scope: WorldViewBindingScope::Channel,
                binding_id,
                binding_revision_event_id: "e".repeat(64),
                authority: WorldMutationAuthority::LocalWorldMirrorLatest {
                    origin: "https://manifest.shivai.space".into(),
                    mirror_id: "mirror-1".into(),
                },
            })
            .unwrap();
        assert_eq!(registry.mutation_delegations.len(), 1);
        assert!(registry
            .resolve_mutation_delegation(
                channel_id,
                &WorldViewBindingScope::Channel,
                binding_id,
                &binding_revision_event_id,
            )
            .is_none());
        assert!(registry
            .resolve_mutation_delegation(
                channel_id,
                &WorldViewBindingScope::Channel,
                binding_id,
                &"e".repeat(64),
            )
            .is_some());
        assert!(registry.revoke_mutation_delegation(
            channel_id,
            &WorldViewBindingScope::Channel,
            binding_id,
        ));
        assert!(registry.mutation_delegations.is_empty());
        assert!(!registry.revoke_mutation_delegation(
            channel_id,
            &WorldViewBindingScope::Channel,
            binding_id,
        ));
    }

    #[test]
    fn derives_stable_thread_scope_coordinate() {
        let root = "a".repeat(64);
        let scope = WorldViewBindingScope::thread(root.clone()).expect("valid thread scope");

        assert_eq!(scope.d_tag(), format!("world-view-bindings:thread:{root}"));
        assert_eq!(scope.thread_root_event_id(), Some(root.as_str()));
    }

    #[test]
    fn rejects_noncanonical_thread_event_ids() {
        assert_eq!(
            WorldViewBindingScope::thread("A".repeat(64)),
            Err("scope.threadRootEventId must be 64 lowercase hex characters".into())
        );
    }

    #[test]
    fn rejects_duplicate_binding_ids() {
        let id = Uuid::nil();
        let document = WorldViewBindingsDocument {
            version: WORLD_VIEW_BINDINGS_VERSION,
            scope: WorldViewBindingScope::Channel,
            bindings: vec![binding(id), binding(id)],
        };

        assert_eq!(
            document.validate(),
            Err(format!("duplicate world view binding id: {id}"))
        );
    }

    #[test]
    fn world_authority_grant_is_exactly_agent_scope_binding_and_revision_bound() {
        let scope = WorldAuthorityGrantScope {
            agent_pubkey: "a".repeat(64),
            channel_id: Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap(),
            effective_scope: WorldViewBindingScope::thread("b".repeat(64)).unwrap(),
            binding_id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
            binding_revision_event_id: "c".repeat(64),
            source_revision: "d".repeat(64),
        };
        let token = issue_world_authority_grant(&scope, b"private-edit-share", 200).unwrap();

        assert!(verify_world_authority_grant(&token, b"private-edit-share", &scope, 100,).is_ok());
        assert!(!token.contains("private-edit-share"));

        let mut another_agent = scope.clone();
        another_agent.agent_pubkey = "e".repeat(64);
        assert_eq!(
            verify_world_authority_grant(&token, b"private-edit-share", &another_agent, 100,),
            Err("world authority grant does not match this request".into())
        );

        let mut another_scope = scope.clone();
        another_scope.effective_scope = WorldViewBindingScope::Channel;
        assert!(
            verify_world_authority_grant(&token, b"private-edit-share", &another_scope, 100,)
                .is_err()
        );

        let mut another_revision = scope.clone();
        another_revision.source_revision = "f".repeat(64);
        assert!(verify_world_authority_grant(
            &token,
            b"private-edit-share",
            &another_revision,
            100,
        )
        .is_err());
        assert_eq!(
            verify_world_authority_grant(&token, b"private-edit-share", &scope, 200,),
            Err("world authority grant expired".into())
        );
    }

    #[test]
    fn world_authority_grant_rejects_tampering() {
        let scope = WorldAuthorityGrantScope {
            agent_pubkey: "a".repeat(64),
            channel_id: Uuid::nil(),
            effective_scope: WorldViewBindingScope::Channel,
            binding_id: Uuid::nil(),
            binding_revision_event_id: "b".repeat(64),
            source_revision: "c".repeat(64),
        };
        let token = issue_world_authority_grant(&scope, b"private-edit-share", 200).unwrap();
        let mut tampered = token.into_bytes();
        let last = tampered.last_mut().unwrap();
        *last = if *last == b'0' { b'1' } else { b'0' };
        let tampered = String::from_utf8(tampered).unwrap();

        assert_eq!(
            verify_world_authority_grant(&tampered, b"private-edit-share", &scope, 100,),
            Err("invalid world authority grant".into())
        );
    }
}
