//! NIP-AS — Agent Spawner.
//!
//! Typed content bodies and event builders for server-hosted agents: an owner
//! publishes a *spec* describing an agent they want running, a `buzz-spawner`
//! daemon reconciles it into a container and publishes *status* back, and the
//! two exchange an ephemeral *attestation* handshake so the daemon can obtain a
//! NIP-OA auth tag for a key it minted itself.
//!
//! # Why the handshake exists
//!
//! The agent's secret key is generated on the spawner host and never leaves it.
//! But a NIP-OA auth tag is
//! `Schnorr(SHA256("nostr:agent-auth:" || agent_pubkey || ":" || conditions), owner_secret)`
//! (see [`crate::nip_oa`]) — it binds one specific agent pubkey and requires the
//! *owner's* secret key. The spawner cannot self-attest, and the owner cannot
//! pre-authorize a pubkey that does not exist yet. Hence two rounds:
//!
//! ```text
//! owner   ──kind:30178 spec──────────────────────────►  spawner
//! spawner ──kind:24201 AttestationRequest (pubkey+nonce)►  owner
//! owner   ──kind:24201 AttestationResponse (auth tag)──►  spawner
//! spawner ──kind:30179 status: running────────────────►  owner
//! ```
//!
//! # Security: specs are world-readable
//!
//! [`SpawnerAgentSpec`] is an explicit opt-IN projection, exactly like the
//! desktop's kind:30177 managed-agent events. It MUST NEVER carry a secret key,
//! an auth tag, env vars, or provider credentials. The type is the structural
//! guard: it physically cannot represent those fields, so a malicious inbound
//! spec cannot smuggle them onto the spawn path. Add a field here only after
//! asking whether it would be safe printed on a billboard.
//!
//! Only the ephemeral [`KIND_SPAWNER_ATTESTATION`] frames carry sensitive
//! material (the auth tag), and those are NIP-44 encrypted and `#p`-gated.
//!
//! # Status events are self-addressing
//!
//! Status is a NIP-33 parameterized replaceable event addressed by
//! `(pubkey, kind, d_tag)`. Because the author is part of the address, a status
//! event published by an impostor lands at *their* address, not the spawner's —
//! it cannot overwrite the real one. Clients therefore read status at the
//! address of a spawner pubkey they already trust, and the relay needs no
//! special author gate for this kind.

use buzz_core::kind::{
    KIND_SPAWNER_AGENT_SPEC, KIND_SPAWNER_AGENT_STATUS, KIND_SPAWNER_ANNOUNCEMENT,
    KIND_SPAWNER_ATTESTATION,
};
use buzz_core::observer::content_looks_like_nip44;
use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};

use crate::SdkError;

/// Tag name carrying the pubkey of the spawner a spec is addressed to.
pub const SPAWNER_TAG: &str = "spawner";

/// Maximum byte length of a spec slug (`d` tag).
pub const MAX_SPEC_SLUG_LEN: usize = 64;

/// Maximum byte length of a serialized spec or status content body.
pub const MAX_SPAWNER_CONTENT_BYTES: usize = 32 * 1024;

/// Maximum number of allowlisted author pubkeys on a spec.
pub const MAX_RESPOND_TO_ALLOWLIST: usize = 256;

/// Byte length of the attestation handshake nonce.
pub const ATTESTATION_NONCE_BYTES: usize = 32;

// ---------------------------------------------------------------------------
// Announcement (kind 10180)
// ---------------------------------------------------------------------------

/// A spawner advertising itself so owners can find it — content of kind:10180.
///
/// Every field is self-reported and unverifiable. A client may show these to
/// help a user choose, but must not treat any of them as a security property:
/// authorization comes from the per-agent attestation the owner signs, never
/// from an announcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnerAnnouncement {
    /// Human-readable name, e.g. "prod-vps" or "gpu-box".
    pub name: String,
    /// Optional longer description shown in a picker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Agent runtime image this spawner runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_image: Option<String>,
    /// ACP agent binary this spawner runs, e.g. `claude-agent-acp`.
    ///
    /// Display-only. A client shows it so "prod-vps — Claude Code" is legible
    /// in a picker, but it is self-reported and confers nothing: the host alone
    /// decides what executes there, and no spec can influence it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// How many agents it will run at once.
    pub max_agents: u32,
    /// How many it is running now, so a full spawner can be shown as such.
    pub agents_running: u32,
    /// Per-agent CPU ceiling, thousandths of a core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cpu_millis: Option<u32>,
    /// Per-agent memory ceiling, mebibytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mib: Option<u32>,
    /// Providers/models this host can run, so a client scopes its picker to
    /// what the server actually supports. Self-reported, like every field here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<Vec<SpawnerAiProvider>>,
}

/// One inference provider a spawner host can run, with its model ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnerAiProvider {
    /// Provider id, e.g. "anthropic".
    pub id: String,
    /// Model ids this host can run for the provider.
    #[serde(default)]
    pub models: Vec<String>,
}

impl SpawnerAnnouncement {
    /// Validate the announcement's own invariants.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.name.trim().is_empty() {
            return Err(SdkError::InvalidInput(
                "spawner name must not be empty".into(),
            ));
        }
        if self.name.len() > 128 {
            return Err(SdkError::InvalidInput(format!(
                "spawner name exceeds 128 bytes (got {})",
                self.name.len()
            )));
        }
        if let Some(description) = &self.description {
            if description.len() > 512 {
                return Err(SdkError::InvalidInput(format!(
                    "spawner description exceeds 512 bytes (got {})",
                    description.len()
                )));
            }
        }
        Ok(())
    }

    /// Whether the spawner reports itself at capacity.
    ///
    /// Advisory only — a client should still let the user try, because these
    /// numbers are a snapshot the spawner chose to publish.
    pub fn is_full(&self) -> bool {
        self.agents_running >= self.max_agents
    }
}

/// Build a spawner announcement event (kind 10180).
pub fn build_spawner_announcement(
    announcement: &SpawnerAnnouncement,
) -> Result<EventBuilder, SdkError> {
    announcement.validate()?;
    let content = serde_json::to_string(announcement)
        .map_err(|e| SdkError::InvalidInput(format!("failed to serialize announcement: {e}")))?;
    check_spawner_content(&content)?;
    Ok(EventBuilder::new(
        Kind::Custom(KIND_SPAWNER_ANNOUNCEMENT as u16),
        content,
    ))
}

/// Parse a kind:10180 event's content into a [`SpawnerAnnouncement`].
pub fn announcement_from_event(event: &nostr::Event) -> Result<SpawnerAnnouncement, SdkError> {
    let announcement: SpawnerAnnouncement = serde_json::from_str(event.content.as_ref())
        .map_err(|e| SdkError::InvalidInput(format!("failed to parse announcement: {e}")))?;
    announcement.validate()?;
    Ok(announcement)
}

// ---------------------------------------------------------------------------
// Spec (kind 30178)
// ---------------------------------------------------------------------------

/// Inbound author gate for a spawned agent — mirrors the desktop's `RespondTo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RespondTo {
    /// Respond to anyone who can address the agent.
    #[default]
    Anyone,
    /// Respond only to the attesting owner.
    OwnerOnly,
    /// Respond only to pubkeys on `respond_to_allowlist`.
    Allowlist,
}

/// Container resource limits for a spawned agent.
///
/// Both fields are optional; the spawner applies its own configured defaults
/// when they are absent, and clamps anything above its configured ceiling. A
/// spec asking for more than the host allows is clamped, not rejected — the
/// owner should get a smaller agent, not a broken one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResourceRequest {
    /// CPU allocation in thousandths of a core (1000 = one full core).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_millis: Option<u32>,
    /// Memory limit in mebibytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<u32>,
}

/// Desired state for a server-hosted agent — the content body of kind:30178.
///
/// See the module docs for the opt-IN projection contract. `system_prompt`,
/// `model`, and `provider` follow the same slimming rule as kind:30177: when
/// `persona_id` is set they are resolved through the referenced kind:30175
/// persona and omitted here, so a prompt lives in exactly one place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnerAgentSpec {
    /// Display name for the agent.
    pub name: String,
    /// Existing agent identity this spec relocates, rather than minting a new one.
    ///
    /// # Why relocation, not creation
    ///
    /// An agent's pubkey *is* its continuity: channel membership, its kind:0
    /// profile, its kind:30177 coordinate (whose `d` tag is this pubkey), DMs,
    /// turn metrics, the relay's `users.agent_owner_pubkey` row, and — most
    /// destructively — its NIP-AE memory, whose d-tags derive from
    /// `conversation_key(agent_seckey, owner_pubkey)`. A new key changes every
    /// d-tag *and* leaves the old ciphertext undecryptable, so the memory is
    /// gone for good.
    ///
    /// So moving an existing agent to a spawner names it here and delivers its
    /// secret key over the encrypted handshake, exactly as a provider deploy
    /// already ships `private_key_nsec` to a remote runner
    /// (`desktop/src-tauri/src/commands/agents_deploy.rs`). Only the public key
    /// appears here; the secret never touches a stored event.
    ///
    /// `None` means "mint a fresh identity", which is right for an agent that
    /// has no local existence to preserve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_pubkey: Option<String>,
    /// Persona (kind:30175) this agent is an instance of, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<String>,
    /// Inline system prompt. Set only for definition-less specs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Model id. Set only for definition-less specs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Inference provider id. Set only for definition-less specs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Number of concurrent ACP sessions the harness should hold.
    #[serde(default = "default_parallelism")]
    pub parallelism: u32,
    /// Inbound author gate mode.
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Allowlisted author pubkeys when `respond_to == Allowlist`. Public keys,
    /// not secrets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub respond_to_allowlist: Vec<String>,
    /// Requested container resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceRequest>,
    /// When false the spawner tears the container down but keeps the spec, so
    /// the agent keeps its identity and can be resumed. Deleting the spec
    /// (NIP-09) is the permanent teardown signal.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_parallelism() -> u32 {
    1
}

fn default_enabled() -> bool {
    true
}

impl SpawnerAgentSpec {
    /// Validate the spec's own invariants.
    ///
    /// Called by [`build_spawner_agent_spec`] on the write path and by the
    /// spawner on the read path — an inbound spec is untrusted input even
    /// though it is signed, because a signature proves authorship, not sanity.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.name.trim().is_empty() {
            return Err(SdkError::InvalidInput("spec name must not be empty".into()));
        }
        if self.name.len() > 128 {
            return Err(SdkError::InvalidInput(format!(
                "spec name exceeds 128 bytes (got {})",
                self.name.len()
            )));
        }
        // Deliberately no "must carry a prompt" rule. Prompt material normally
        // arrives over the encrypted kind:24201 handshake precisely so it does
        // NOT appear here — a spec is world-readable, and requiring a prompt on
        // it would force every server agent's instructions to be public.
        if self.parallelism == 0 || self.parallelism > 16 {
            return Err(SdkError::InvalidInput(format!(
                "parallelism must be in 1..=16 (got {})",
                self.parallelism
            )));
        }
        if self.respond_to_allowlist.len() > MAX_RESPOND_TO_ALLOWLIST {
            return Err(SdkError::InvalidInput(format!(
                "respond_to_allowlist exceeds {MAX_RESPOND_TO_ALLOWLIST} entries (got {})",
                self.respond_to_allowlist.len()
            )));
        }
        for pk in &self.respond_to_allowlist {
            check_pubkey_hex(pk, "respond_to_allowlist entry")?;
        }
        if let Some(agent_pubkey) = &self.agent_pubkey {
            check_pubkey_hex(agent_pubkey, "agent_pubkey")?;
        }
        if self.respond_to == RespondTo::Allowlist && self.respond_to_allowlist.is_empty() {
            return Err(SdkError::InvalidInput(
                "respond_to=allowlist requires a non-empty respond_to_allowlist".into(),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Status (kind 30179)
// ---------------------------------------------------------------------------

/// Reconciliation phase reported by the spawner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnPhase {
    /// Keys minted; waiting for the owner to return a signed auth tag.
    PendingAttestation,
    /// Attested; container being created.
    Starting,
    /// Container running.
    Running,
    /// Reconciliation failed. `error` carries a human-readable reason.
    Failed,
    /// Intentionally not running — spec has `enabled: false`.
    Stopped,
}

/// Actual state of a spawned agent — the content body of kind:30179.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnerAgentStatus {
    /// Current reconciliation phase.
    pub phase: SpawnPhase,
    /// The minted agent pubkey, present from `PendingAttestation` onward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_pubkey: Option<String>,
    /// Hash of the spec content this status reflects, so a client can tell
    /// whether the spawner has caught up with an edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_hash: Option<String>,
    /// Human-readable failure reason. Set iff `phase == Failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Consecutive failed start attempts, for surfacing backoff in a UI.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub restart_count: u32,
    /// Hash of the prompt material this agent is running with (see
    /// [`PromptMaterial::hash`]), so a client can tell whether a pushed
    /// prompt update has been applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl SpawnerAgentStatus {
    /// Validate the status's own invariants.
    pub fn validate(&self) -> Result<(), SdkError> {
        if let Some(pk) = &self.agent_pubkey {
            check_pubkey_hex(pk, "agent_pubkey")?;
        }
        if self.phase == SpawnPhase::Failed && self.error.is_none() {
            return Err(SdkError::InvalidInput(
                "phase=failed requires an error message".into(),
            ));
        }
        if let Some(err) = &self.error {
            if err.len() > 2048 {
                return Err(SdkError::InvalidInput(format!(
                    "status error exceeds 2048 bytes (got {})",
                    err.len()
                )));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Attestation handshake (kind 24201)
// ---------------------------------------------------------------------------

/// Prompt material delivered to a spawner over the encrypted handshake.
///
/// # Why this travels here rather than on the spec
///
/// A kind:30178 spec is world-readable, so inlining a system prompt there would
/// publish it to the whole community — as would marking the persona
/// `["shared","true"]`. The attestation channel is already NIP-44 encrypted
/// owner-to-spawner, and the owner can obviously read their own persona, so
/// delivering the prompt here keeps it private with no relay involvement at all.
///
/// The cost is that a prompt edit does not reach a running spawner by itself;
/// the owner sends an [`AttestationFrame::PromptUpdate`] to push a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PromptMaterial {
    /// The agent's system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Team-level instructions appended after the system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_instructions: Option<String>,
    /// Model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Inference provider id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl PromptMaterial {
    /// Whether this carries anything worth storing.
    pub fn is_empty(&self) -> bool {
        self.system_prompt.is_none()
            && self.team_instructions.is_none()
            && self.model.is_none()
            && self.provider.is_none()
    }

    /// Lowercase sha256 hex of this material's JSON serialization.
    ///
    /// Serialization skips `None` fields, so two materials with the same set
    /// values hash identically regardless of construction order.
    pub fn hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let json = serde_json::to_string(self).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(json.as_bytes());
        h.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    }
}

/// The plaintext payload of a [`KIND_SPAWNER_ATTESTATION`] frame, before NIP-44
/// encryption.
///
/// `nonce` binds the two rounds together: the spawner will only accept a
/// response whose nonce matches the request it is still waiting on, so a
/// replayed or crossed response for a different agent is rejected rather than
/// silently applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AttestationFrame {
    /// Spawner → owner: "I minted this pubkey for your spec; please attest it."
    Request {
        /// The `d` tag of the spec this agent belongs to.
        spec_slug: String,
        /// The freshly minted agent pubkey, hex.
        agent_pubkey: String,
        /// NIP-OA conditions string the owner should sign over. Usually empty.
        #[serde(default)]
        conditions: String,
        /// Hex-encoded random nonce, [`ATTESTATION_NONCE_BYTES`] bytes.
        nonce: String,
    },
    /// Owner → spawner: the signed auth tag.
    Response {
        /// Echoed spec slug.
        spec_slug: String,
        /// Echoed agent pubkey — must match the pending request.
        agent_pubkey: String,
        /// Echoed nonce — must match the pending request.
        nonce: String,
        /// The NIP-OA `auth` tag as a JSON array string, in the form
        /// `["auth", "<owner-pubkey>", "<conditions>", "<sig>"]`. Verify with
        /// [`crate::nip_oa::verify_auth_tag`] before use.
        auth_tag: String,
        /// Prompt material for this agent, when the spawner cannot read the
        /// referenced persona itself. Optional so a shared-persona deployment
        /// need not duplicate it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<PromptMaterial>,
        /// Secret key of an existing agent being relocated to this spawner.
        ///
        /// Present only when the spec named an `agent_pubkey`. The enclosing
        /// frame is NIP-44 encrypted to the spawner and the kind is ephemeral,
        /// so the key is never stored by the relay — but it does transit it, and
        /// that is the deliberate cost of keeping one identity across hosts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        private_key_nsec: Option<String>,
    },
    /// Owner → spawner: replacement prompt material for an existing agent.
    ///
    /// Sent after a persona edit. Carries no nonce because it opens no round —
    /// the spawner accepts it only for an agent it already holds an attestation
    /// for, from that agent's own owner, so there is nothing to bind it to.
    PromptUpdate {
        /// The `d` tag of the spec this agent belongs to.
        spec_slug: String,
        /// The agent being updated.
        agent_pubkey: String,
        /// The new prompt material.
        prompt: PromptMaterial,
    },
    /// Owner → spawner: refusal. Lets a client decline explicitly instead of
    /// leaving the spawner waiting on a timeout.
    Reject {
        /// Echoed spec slug.
        spec_slug: String,
        /// Echoed agent pubkey.
        agent_pubkey: String,
        /// Echoed nonce.
        nonce: String,
        /// Human-readable reason, surfaced in the spawner's status event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

impl AttestationFrame {
    /// The agent pubkey this frame concerns, regardless of variant.
    pub fn agent_pubkey(&self) -> &str {
        match self {
            Self::Request { agent_pubkey, .. }
            | Self::Response { agent_pubkey, .. }
            | Self::Reject { agent_pubkey, .. }
            | Self::PromptUpdate { agent_pubkey, .. } => agent_pubkey,
        }
    }

    /// The nonce binding this frame to a handshake round.
    ///
    /// Empty for [`Self::PromptUpdate`], which opens no round.
    pub fn nonce(&self) -> &str {
        match self {
            Self::Request { nonce, .. }
            | Self::Response { nonce, .. }
            | Self::Reject { nonce, .. } => nonce,
            Self::PromptUpdate { .. } => "",
        }
    }

    /// The spec slug this frame concerns.
    pub fn spec_slug(&self) -> &str {
        match self {
            Self::Request { spec_slug, .. }
            | Self::Response { spec_slug, .. }
            | Self::Reject { spec_slug, .. }
            | Self::PromptUpdate { spec_slug, .. } => spec_slug,
        }
    }

    /// Validate structural invariants shared by every variant.
    pub fn validate(&self) -> Result<(), SdkError> {
        check_spec_slug(self.spec_slug())?;
        check_pubkey_hex(self.agent_pubkey(), "agent_pubkey")?;
        // A prompt update opens no handshake round, so it carries no nonce.
        if matches!(self, Self::PromptUpdate { .. }) {
            return Ok(());
        }
        let nonce = self.nonce();
        if nonce.len() != ATTESTATION_NONCE_BYTES * 2
            || !nonce.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(SdkError::InvalidInput(format!(
                "nonce must be {} hex characters",
                ATTESTATION_NONCE_BYTES * 2
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Build a spawner agent spec event (kind 30178).
///
/// `slug` becomes the `d` tag and is the stable handle for this agent across
/// edits — it is chosen by the client, not derived from the agent pubkey, which
/// does not exist until the spawner mints it.
///
/// `spawner_pubkey` becomes a `spawner` tag so a host running more than one
/// daemon, or a client watching several, can filter without parsing content.
pub fn build_spawner_agent_spec(
    slug: &str,
    spawner_pubkey: &str,
    spec: &SpawnerAgentSpec,
) -> Result<EventBuilder, SdkError> {
    let slug = check_spec_slug(slug)?;
    let spawner_pubkey = check_pubkey_hex(spawner_pubkey, "spawner_pubkey")?;
    spec.validate()?;

    let content = serde_json::to_string(spec)
        .map_err(|e| SdkError::InvalidInput(format!("failed to serialize spec: {e}")))?;
    check_spawner_content(&content)?;

    let tags = vec![
        parse_tag(&["d", &slug])?,
        parse_tag(&[SPAWNER_TAG, &spawner_pubkey])?,
        parse_tag(&["p", &spawner_pubkey])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(KIND_SPAWNER_AGENT_SPEC as u16), content).tags(tags))
}

/// Build a spawner agent status event (kind 30179).
///
/// `owner_pubkey` becomes a `p` tag so the owner's client can subscribe to
/// status for its own agents without knowing every slug in advance.
pub fn build_spawner_agent_status(
    slug: &str,
    owner_pubkey: &str,
    status: &SpawnerAgentStatus,
) -> Result<EventBuilder, SdkError> {
    let slug = check_spec_slug(slug)?;
    let owner_pubkey = check_pubkey_hex(owner_pubkey, "owner_pubkey")?;
    status.validate()?;

    let content = serde_json::to_string(status)
        .map_err(|e| SdkError::InvalidInput(format!("failed to serialize status: {e}")))?;
    check_spawner_content(&content)?;

    let mut tags = vec![parse_tag(&["d", &slug])?, parse_tag(&["p", &owner_pubkey])?];
    // Surface the agent pubkey as a tag too, so a client can resolve
    // slug → agent identity from the tag index without reading content.
    if let Some(agent_pubkey) = &status.agent_pubkey {
        tags.push(parse_tag(&["agent", agent_pubkey])?);
    }
    Ok(EventBuilder::new(Kind::Custom(KIND_SPAWNER_AGENT_STATUS as u16), content).tags(tags))
}

/// Build an ephemeral attestation handshake frame (kind 24201).
///
/// `recipient_pubkey` is the cleartext `p` tag the relay uses to route the
/// frame to exactly one counterparty. `encrypted_content` must be NIP-44 v2
/// ciphertext of a serialized [`AttestationFrame`] — this builder deliberately
/// takes ciphertext rather than the frame itself, so no code path can construct
/// a plaintext auth tag on the wire by mistake.
pub fn build_spawner_attestation(
    recipient_pubkey: &str,
    encrypted_content: &str,
) -> Result<EventBuilder, SdkError> {
    let recipient_pubkey = check_pubkey_hex(recipient_pubkey, "recipient_pubkey")?;
    if !content_looks_like_nip44(encrypted_content) {
        return Err(SdkError::InvalidInput(
            "attestation frame content must be NIP-44 v2 ciphertext".into(),
        ));
    }
    check_spawner_content(encrypted_content)?;

    Ok(EventBuilder::new(
        Kind::Custom(KIND_SPAWNER_ATTESTATION as u16),
        encrypted_content,
    )
    .tags(vec![parse_tag(&["p", &recipient_pubkey])?]))
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// Parse a kind:30178 event's content into a [`SpawnerAgentSpec`].
///
/// # Security
///
/// Returns the projection type, which physically cannot represent a secret key,
/// auth tag, env vars, or provider config — unknown keys in a malicious event
/// are dropped at deserialization rather than filtered afterward. The spec is
/// validated here as well; a signature proves authorship, not sanity.
pub fn spec_from_event(event: &nostr::Event) -> Result<SpawnerAgentSpec, SdkError> {
    let spec: SpawnerAgentSpec = serde_json::from_str(event.content.as_ref())
        .map_err(|e| SdkError::InvalidInput(format!("failed to parse spec content: {e}")))?;
    spec.validate()?;
    Ok(spec)
}

/// Parse a kind:30179 event's content into a [`SpawnerAgentStatus`].
pub fn status_from_event(event: &nostr::Event) -> Result<SpawnerAgentStatus, SdkError> {
    let status: SpawnerAgentStatus = serde_json::from_str(event.content.as_ref())
        .map_err(|e| SdkError::InvalidInput(format!("failed to parse status content: {e}")))?;
    status.validate()?;
    Ok(status)
}

/// Read the `d` tag (spec slug) from a spawner event.
pub fn spec_slug_from_event(event: &nostr::Event) -> Option<String> {
    single_tag_value(event, "d")
}

/// Read the `spawner` tag (target spawner pubkey) from a spec event.
pub fn spawner_pubkey_from_event(event: &nostr::Event) -> Option<String> {
    single_tag_value(event, SPAWNER_TAG)
}

fn single_tag_value(event: &nostr::Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|t| {
        let parts = t.as_slice();
        (parts.len() >= 2 && parts[0].as_str() == name).then(|| parts[1].to_string())
    })
}

// ---------------------------------------------------------------------------
// Local validation helpers
// ---------------------------------------------------------------------------

fn parse_tag(parts: &[&str]) -> Result<Tag, SdkError> {
    Tag::parse(parts.iter().copied()).map_err(|e| SdkError::InvalidTag(e.to_string()))
}

fn check_pubkey_hex(s: &str, field: &str) -> Result<String, SdkError> {
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(format!(
            "{field} must be a 64-character hex pubkey"
        )));
    }
    Ok(s.to_ascii_lowercase())
}

/// Validate a spec slug.
///
/// Slugs appear in container names, volume names, and log paths on the spawner
/// host, so the character set is deliberately narrow: lowercase alphanumerics,
/// hyphens, and underscores. That rules out path traversal, shell
/// metacharacters, and Docker name-rule violations in one pass, at the point
/// where the value enters the system rather than at each use site.
pub fn check_spec_slug(slug: &str) -> Result<String, SdkError> {
    if slug.is_empty() {
        return Err(SdkError::InvalidInput("spec slug must not be empty".into()));
    }
    if slug.len() > MAX_SPEC_SLUG_LEN {
        return Err(SdkError::InvalidInput(format!(
            "spec slug exceeds {MAX_SPEC_SLUG_LEN} bytes (got {})",
            slug.len()
        )));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(SdkError::InvalidInput(
            "spec slug may only contain lowercase ASCII letters, digits, hyphens, and underscores"
                .into(),
        ));
    }
    if slug.starts_with('-') || slug.starts_with('_') {
        return Err(SdkError::InvalidInput(
            "spec slug must start with a letter or digit".into(),
        ));
    }
    Ok(slug.to_string())
}

fn check_spawner_content(content: &str) -> Result<(), SdkError> {
    let got = content.len();
    if got > MAX_SPAWNER_CONTENT_BYTES {
        return Err(SdkError::ContentTooLarge {
            max: MAX_SPAWNER_CONTENT_BYTES,
            got,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn sample_spec() -> SpawnerAgentSpec {
        SpawnerAgentSpec {
            name: "Fizz".into(),
            agent_pubkey: None,
            persona_id: Some("builtin:fizz".into()),
            system_prompt: None,
            model: None,
            provider: None,
            parallelism: 1,
            respond_to: RespondTo::Anyone,
            respond_to_allowlist: Vec::new(),
            resources: Some(ResourceRequest {
                cpu_millis: Some(1000),
                memory_mib: Some(2048),
            }),
            enabled: true,
        }
    }

    #[test]
    fn spec_round_trips_through_an_event() {
        let keys = Keys::generate();
        let spawner = Keys::generate();
        let spec = sample_spec();
        let event = build_spawner_agent_spec("fizz-prod", &spawner.public_key().to_hex(), &spec)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();

        assert_eq!(spec_slug_from_event(&event).as_deref(), Some("fizz-prod"));
        assert_eq!(
            spawner_pubkey_from_event(&event),
            Some(spawner.public_key().to_hex())
        );
        assert_eq!(spec_from_event(&event).unwrap(), spec);
    }

    #[test]
    fn spec_rejects_secret_bearing_keys_by_dropping_them() {
        // A malicious spec that tries to smuggle an nsec and an auth tag. The
        // projection type has no such fields, so they are dropped at parse.
        let json = r#"{
            "name": "Evil",
            "persona_id": "builtin:fizz",
            "private_key_nsec": "nsec1deadbeef",
            "auth_tag": "[\"auth\",\"..\"]",
            "env_vars": {"ANTHROPIC_API_KEY": "sk-leak"}
        }"#;
        let spec: SpawnerAgentSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.name, "Evil");
        // Re-serializing cannot reproduce the smuggled fields.
        let round_tripped = serde_json::to_string(&spec).unwrap();
        assert!(!round_tripped.contains("nsec"));
        assert!(!round_tripped.contains("auth_tag"));
        assert!(!round_tripped.contains("sk-leak"));
    }

    #[test]
    fn spec_may_name_an_existing_identity_to_relocate() {
        let agent = Keys::generate();
        let mut spec = sample_spec();
        spec.agent_pubkey = Some(agent.public_key().to_hex());
        assert!(spec.validate().is_ok());

        // Only the public key ever appears on a spec — it is world-readable.
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains(&agent.public_key().to_hex()));
        assert!(!json.contains("nsec"));

        spec.agent_pubkey = Some("not-a-pubkey".into());
        assert!(spec.validate().is_err());
    }

    #[test]
    fn spec_need_not_carry_a_prompt() {
        // Prompt material rides the encrypted handshake so it never becomes
        // public. A spec with neither persona_id nor system_prompt is the
        // normal, private case — not an error.
        let mut spec = sample_spec();
        spec.persona_id = None;
        spec.system_prompt = None;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn spec_rejects_empty_allowlist_when_gated_on_it() {
        let mut spec = sample_spec();
        spec.respond_to = RespondTo::Allowlist;
        assert!(spec.validate().is_err());
        spec.respond_to_allowlist = vec![Keys::generate().public_key().to_hex()];
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn slug_rejects_traversal_and_metacharacters() {
        for bad in [
            "../etc/passwd",
            "fizz prod",
            "fizz;rm -rf /",
            "Fizz",
            "-leading",
            "",
        ] {
            assert!(
                check_spec_slug(bad).is_err(),
                "slug {bad:?} should be rejected"
            );
        }
        assert!(check_spec_slug("fizz-prod_2").is_ok());
    }

    #[test]
    fn status_failed_requires_an_error() {
        let mut status = SpawnerAgentStatus {
            phase: SpawnPhase::Failed,
            agent_pubkey: None,
            spec_hash: None,
            error: None,
            restart_count: 3,
            prompt_hash: None,
        };
        assert!(status.validate().is_err());
        status.error = Some("image pull failed".into());
        assert!(status.validate().is_ok());
    }

    #[test]
    fn status_round_trips_and_tags_the_agent() {
        let keys = Keys::generate();
        let owner = Keys::generate();
        let agent = Keys::generate();
        let status = SpawnerAgentStatus {
            phase: SpawnPhase::Running,
            agent_pubkey: Some(agent.public_key().to_hex()),
            spec_hash: Some("abc123".into()),
            error: None,
            restart_count: 0,
            prompt_hash: None,
        };
        let event = build_spawner_agent_status("fizz-prod", &owner.public_key().to_hex(), &status)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();

        assert_eq!(status_from_event(&event).unwrap(), status);
        assert_eq!(
            single_tag_value(&event, "agent"),
            Some(agent.public_key().to_hex())
        );
        assert_eq!(
            single_tag_value(&event, "p"),
            Some(owner.public_key().to_hex())
        );
    }

    fn sample_announcement() -> SpawnerAnnouncement {
        SpawnerAnnouncement {
            name: "prod-vps".into(),
            description: Some("Hetzner CX42, Frankfurt".into()),
            agent_image: Some("ghcr.io/block/buzz-acp:main".into()),
            runtime: Some("claude-agent-acp".into()),
            max_agents: 16,
            agents_running: 3,
            max_cpu_millis: Some(4000),
            max_memory_mib: Some(8192),
            ai: None,
        }
    }

    #[test]
    fn announcement_round_trips_through_an_event() {
        let keys = Keys::generate();
        let announcement = sample_announcement();
        let event = build_spawner_announcement(&announcement)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(announcement_from_event(&event).unwrap(), announcement);
    }

    #[test]
    fn announcement_ai_catalog_round_trips() {
        let mut a = sample_announcement();
        a.ai = Some(vec![SpawnerAiProvider {
            id: "anthropic".into(),
            models: vec!["claude-opus-5".into(), "claude-sonnet-5".into()],
        }]);
        let json = serde_json::to_string(&a).unwrap();
        let back: SpawnerAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ai, a.ai);
        // Old announcements without the field still parse.
        let legacy: SpawnerAnnouncement =
            serde_json::from_str(r#"{"name":"x","max_agents":1,"agents_running":0}"#).unwrap();
        assert!(legacy.ai.is_none());
    }

    #[test]
    fn prompt_material_hash_is_stable_and_content_sensitive() {
        let a = PromptMaterial {
            model: Some("m1".into()),
            ..Default::default()
        };
        let b = PromptMaterial {
            model: Some("m1".into()),
            ..Default::default()
        };
        let c = PromptMaterial {
            model: Some("m2".into()),
            ..Default::default()
        };
        assert_eq!(a.hash(), b.hash());
        assert_ne!(a.hash(), c.hash());
        assert_eq!(a.hash().len(), 64);
    }

    #[test]
    fn status_prompt_hash_round_trips() {
        let s = SpawnerAgentStatus {
            phase: SpawnPhase::Running,
            agent_pubkey: None,
            spec_hash: None,
            error: None,
            restart_count: 0,
            prompt_hash: Some("ab".repeat(32)),
        };
        let back: SpawnerAgentStatus =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.prompt_hash, s.prompt_hash);
    }

    #[test]
    fn announcement_reports_capacity() {
        let mut a = SpawnerAnnouncement {
            name: "vps".into(),
            description: None,
            agent_image: None,
            runtime: None,
            max_agents: 2,
            agents_running: 1,
            max_cpu_millis: None,
            max_memory_mib: None,
            ai: None,
        };
        assert!(!a.is_full());
        a.agents_running = 2;
        assert!(a.is_full());
        // Over-capacity can happen after a limit is lowered; still full.
        a.agents_running = 5;
        assert!(a.is_full());
    }

    #[test]
    fn announcement_rejects_an_empty_name() {
        // A nameless spawner would render as a blank row in a picker, which is
        // worse than not listing it.
        let a = SpawnerAnnouncement {
            name: "   ".into(),
            description: None,
            agent_image: None,
            runtime: None,
            max_agents: 1,
            agents_running: 0,
            max_cpu_millis: None,
            max_memory_mib: None,
            ai: None,
        };
        assert!(a.validate().is_err());
    }

    #[test]
    fn announcement_cannot_carry_secrets() {
        // Announcements are world-readable by design. The projection type must
        // drop anything a malicious or careless spawner adds.
        let json = r#"{"name":"evil","max_agents":1,"agents_running":0,
            "private_key_nsec":"nsec1leak","env_vars":{"K":"sk-leak"}}"#;
        let a: SpawnerAnnouncement = serde_json::from_str(json).unwrap();
        let round_tripped = serde_json::to_string(&a).unwrap();
        assert!(!round_tripped.contains("nsec"));
        assert!(!round_tripped.contains("sk-leak"));
    }

    #[test]
    fn attestation_requires_nip44_ciphertext() {
        let recipient = Keys::generate().public_key().to_hex();
        assert!(build_spawner_attestation(&recipient, "plaintext auth tag").is_err());
    }

    #[test]
    fn attestation_frame_validates_nonce_width() {
        let agent = Keys::generate().public_key().to_hex();
        let mut frame = AttestationFrame::Request {
            spec_slug: "fizz-prod".into(),
            agent_pubkey: agent.clone(),
            conditions: String::new(),
            nonce: "ab".repeat(ATTESTATION_NONCE_BYTES),
        };
        assert!(frame.validate().is_ok());

        frame = AttestationFrame::Request {
            spec_slug: "fizz-prod".into(),
            agent_pubkey: agent,
            conditions: String::new(),
            nonce: "tooshort".into(),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn attestation_frame_json_is_tagged_by_type() {
        let frame = AttestationFrame::Response {
            spec_slug: "fizz-prod".into(),
            agent_pubkey: Keys::generate().public_key().to_hex(),
            nonce: "ab".repeat(ATTESTATION_NONCE_BYTES),
            auth_tag: r#"["auth","owner","","sig"]"#.into(),
            prompt: None,
            private_key_nsec: None,
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"response""#));
        assert_eq!(
            serde_json::from_str::<AttestationFrame>(&json).unwrap(),
            frame
        );
    }
}
