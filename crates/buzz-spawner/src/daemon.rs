//! The reconcile loop: applies [`crate::reconcile::plan`] actions against the
//! relay, the store, and the container backend.

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use buzz_sdk::spawner::{SpawnPhase, SpawnerAgentSpec, SpawnerAgentStatus, SpawnerAnnouncement};
use nostr::{Keys, PublicKey};
use tracing::{error, info, warn};

use crate::{
    attestation::{self, ResponseOutcome},
    config::{Config, DEFAULT_CPU_MILLIS, DEFAULT_MEMORY_MIB},
    container::{ContainerOps, ContainerSpec},
    env::{build_agent_env, AgentRuntime, ResolvedPrompt},
    reconcile::{plan, Action, DesiredAgent, ReconcileInput},
    relay::{Inbound, SpawnerRelay},
    store::{AgentRecord, Store},
};

/// Long-running spawner daemon.
pub struct Daemon {
    config: Config,
    store: Store,
    relay: SpawnerRelay,
    containers: Arc<dyn ContainerOps>,
    /// Latest spec per `(owner, slug)`, the desired state built from the relay.
    desired: HashMap<(String, String), DesiredAgent>,
    /// Newest `created_at` seen per `(owner, slug)`.
    ///
    /// Kind 30178 is replaceable, but the relay retains superseded revisions and
    /// replays them in an order this client does not control. Without this
    /// guard an older revision silently overwrites a newer one — which loses a
    /// relocation request, and the spawner keeps running the identity it minted
    /// instead of the agent the owner asked it to adopt.
    spec_seen_at: HashMap<(String, String), u64>,
    /// Whether the relay has finished replaying stored specs.
    ///
    /// Until then `desired` is incomplete, and an absent spec must not be read
    /// as a deletion — see [`ReconcileInput::desired_hydrated`].
    desired_hydrated: bool,
}

impl Daemon {
    /// Connect to the relay and open the state store.
    pub async fn start(config: Config, containers: Arc<dyn ContainerOps>) -> Result<Self> {
        let store = Store::open(&config.state_dir)?;
        let relay = SpawnerRelay::connect(&config.relay_url, &config.keys).await?;

        info!(
            spawner = %config.keys.public_key().to_hex(),
            relay = %config.relay_url,
            "buzz-spawner connected"
        );

        Ok(Self {
            config,
            store,
            relay,
            containers,
            desired: HashMap::new(),
            spec_seen_at: HashMap::new(),
            desired_hydrated: false,
        })
    }

    /// Run until cancelled.
    pub async fn run(&mut self, shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let mut ticker = tokio::time::interval(self.config.reconcile_interval);
        let mut shutdown = shutdown;

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("shutdown requested; leaving agent containers running");
                        return Ok(());
                    }
                }
                _ = ticker.tick() => {
                    if let Err(e) = self.reconcile().await {
                        error!("reconcile pass failed: {e:#}");
                    }
                }
                inbound = self.relay.next() => {
                    match inbound {
                        Ok(inbound) => {
                            if let Err(e) = self.handle_inbound(inbound).await {
                                warn!("failed to handle relay frame: {e:#}");
                            }
                        }
                        Err(e) => {
                            // A dropped socket is expected over a long run.
                            // Reconnect rather than exiting: agent containers
                            // keep serving while the control plane recovers.
                            warn!("relay stream error, reconnecting: {e:#}");
                            if let Err(e) = self.relay.reconnect().await {
                                error!("reconnect failed: {e:#}");
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                }
            }
        }
    }

    async fn handle_inbound(&mut self, inbound: Inbound) -> Result<()> {
        match inbound {
            Inbound::Idle => Ok(()),
            Inbound::SpecsHydrated => {
                if !self.desired_hydrated {
                    info!(
                        specs = self.desired.len(),
                        "desired state hydrated; deletions now enabled"
                    );
                    self.desired_hydrated = true;
                }
                self.reconcile().await
            }
            Inbound::Spec {
                owner_pubkey,
                desired,
                created_at,
            } => {
                let key = (owner_pubkey, desired.slug.clone());
                if !self.accept_revision(&key, created_at) {
                    return Ok(());
                }
                self.desired.insert(key, desired);
                self.reconcile().await
            }
            Inbound::SpecDeleted {
                owner_pubkey,
                slug,
                created_at,
            } => {
                let key = (owner_pubkey, slug);
                if !self.accept_revision(&key, created_at) {
                    return Ok(());
                }
                self.desired.remove(&key);
                self.reconcile().await
            }
            Inbound::Attestation { sender, frame } => {
                if let buzz_sdk::spawner::AttestationFrame::PromptUpdate {
                    agent_pubkey,
                    prompt,
                    ..
                } = &frame
                {
                    return self
                        .apply_prompt_update(&sender, agent_pubkey, prompt)
                        .await;
                }
                self.apply_attestation(&sender, &frame).await
            }
        }
    }

    async fn apply_attestation(
        &mut self,
        sender: &PublicKey,
        frame: &buzz_sdk::spawner::AttestationFrame,
    ) -> Result<()> {
        let Some(record) = self
            .store
            .find_by_agent_pubkey(frame.agent_pubkey())
            .cloned()
        else {
            // Not ours, or already torn down. Silently ignoring is correct:
            // anyone can address a frame to this spawner.
            return Ok(());
        };

        match attestation::evaluate_response(&record, sender, frame) {
            Ok(ResponseOutcome::Accept {
                auth_tag,
                prompt,
                private_key_nsec,
            }) => {
                info!(
                    slug = %record.slug,
                    agent = %record.agent_pubkey,
                    "attestation accepted"
                );
                self.store.update(&record.owner_pubkey, &record.slug, |r| {
                    r.auth_tag = Some(auth_tag);
                    r.pending_nonce = None;
                    r.attestation_sent_at = None;
                    // Only overwrite when the owner actually sent something —
                    // a shared-persona deployment sends no prompt, and must not
                    // clear one delivered earlier.
                    if prompt.as_ref().is_some_and(|p| !p.is_empty()) {
                        r.prompt = prompt;
                    }
                    // A relocated agent arrives with its own key. Verified
                    // against the attested pubkey in `evaluate_response`, so by
                    // here it is known to be the right identity.
                    if let Some(nsec) = private_key_nsec {
                        r.private_key_nsec = nsec;
                    }
                })?;
                self.reconcile().await
            }
            Ok(ResponseOutcome::Rejected { reason }) => {
                warn!(slug = %record.slug, "owner declined attestation: {reason}");
                self.store.update(&record.owner_pubkey, &record.slug, |r| {
                    r.pending_nonce = None;
                    r.attestation_sent_at = None;
                })?;
                self.publish_status(
                    &record.slug,
                    &record.owner_pubkey,
                    SpawnPhase::Failed,
                    Some(&record.agent_pubkey),
                    None,
                    Some(format!("owner declined attestation: {reason}")),
                    0,
                )
                .await
            }
            Err(e) => {
                // Rejected frames are not an error condition for the daemon —
                // anyone can send one. Log and carry on.
                warn!(
                    slug = %record.slug,
                    sender = %sender.to_hex(),
                    "rejected attestation frame: {e:#}"
                );
                Ok(())
            }
        }
    }

    /// Whether a spec revision is newer than what is already held.
    ///
    /// Ties are accepted: two revisions can share a second, and refusing both
    /// would strand whichever arrived first.
    fn accept_revision(&mut self, key: &(String, String), created_at: u64) -> bool {
        if self
            .spec_seen_at
            .get(key)
            .is_some_and(|seen| created_at < *seen)
        {
            return false;
        }
        self.spec_seen_at.insert(key.clone(), created_at);
        true
    }

    /// Apply replacement prompt material for an agent after a persona edit.
    ///
    /// Accepted only from the agent's own owner. There is no nonce to check —
    /// this opens no handshake round — so ownership is the entire gate: without
    /// it anyone could rewrite a running agent's instructions.
    async fn apply_prompt_update(
        &mut self,
        sender: &PublicKey,
        agent_pubkey: &str,
        prompt: &buzz_sdk::spawner::PromptMaterial,
    ) -> Result<()> {
        let Some(record) = self.store.find_by_agent_pubkey(agent_pubkey).cloned() else {
            return Ok(());
        };
        if sender.to_hex() != record.owner_pubkey {
            warn!(
                agent = %agent_pubkey,
                sender = %sender.to_hex(),
                "ignoring prompt update from a non-owner"
            );
            return Ok(());
        }
        if prompt.is_empty() {
            return Ok(());
        }

        info!(slug = %record.slug, "prompt updated by owner");
        let carried = carried_team_instructions(&record, prompt);
        self.store.update(&record.owner_pubkey, &record.slug, |r| {
            // `r.prompt` is stored exactly as received so `prompt_hash_for`
            // reproduces the hash the owner computed over the frame — that echo
            // is the client's only ack. Team instructions, which the desktop's
            // prompt update never carries, ride alongside instead of being
            // merged in, so a model tweak no longer wipes them (see
            // `AgentRecord::carried_team_instructions`).
            r.carried_team_instructions = carried.clone();
            r.prompt = Some(prompt.clone());
            // Force a restart: the container bakes the prompt into its env, so
            // a new prompt only takes effect on a fresh container.
            r.spec_hash = None;
        })?;
        self.reconcile().await
    }

    /// One reconciliation pass.
    pub async fn reconcile(&mut self) -> Result<()> {
        let spawner_pubkey = self.config.keys.public_key().to_hex();
        let containers = self.containers.list(&spawner_pubkey).await?;
        let desired: Vec<DesiredAgent> = self.desired.values().cloned().collect();
        let records: Vec<AgentRecord> = self.store.agents().cloned().collect();

        let actions = plan(ReconcileInput {
            desired: &desired,
            records: &records,
            containers: &containers,
            now: chrono::Utc::now().timestamp(),
            attestation_timeout_secs: self.config.attestation_timeout.as_secs() as i64,
            max_agents: self.config.max_agents,
            desired_hydrated: self.desired_hydrated,
        });

        for action in actions {
            // One failing agent must not abort the pass for every other agent.
            if let Err(e) = self.apply(action).await {
                error!("action failed: {e:#}");
            }
        }

        // Advertise after acting, so the published count reflects this pass.
        // A failure here is not fatal: discovery is a convenience, and an
        // un-advertised spawner still works for anyone who knows its pubkey.
        if let Err(e) = self.announce(containers.len()).await {
            warn!("failed to publish spawner announcement: {e:#}");
        }
        Ok(())
    }

    /// Publish this spawner's self-description and current capacity.
    async fn announce(&mut self, agents_running: usize) -> Result<()> {
        let announcement = SpawnerAnnouncement {
            name: self.config.name.clone(),
            description: self.config.description.clone(),
            agent_image: Some(self.config.agent_image.clone()),
            // What this host actually runs, so a picker can say "Claude Code"
            // rather than leaving the user guessing.
            runtime: self.config.agent_command.clone(),
            max_agents: self.config.max_agents as u32,
            agents_running: agents_running as u32,
            max_cpu_millis: Some(self.config.max_cpu_millis),
            max_memory_mib: Some(self.config.max_memory_mib),
            ai: self.config.ai_catalog.clone(),
        };
        self.relay.publish_announcement(&announcement).await
    }

    async fn apply(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Provision { desired } => self.provision(&desired).await,
            Action::ReRequestAttestation { owner_pubkey, slug } => {
                self.request_attestation(&owner_pubkey, &slug).await
            }
            Action::Start { desired } => self.start_agent(&desired, None, true).await,
            Action::Restart {
                desired,
                container_id,
                crashed,
            } => {
                if crashed {
                    // Count the crash before restarting, so backoff grows even
                    // though the *creation* that follows will succeed. The
                    // container's own logs are the only place the reason
                    // exists — surface a tail of them rather than making the
                    // owner SSH into the host to find out why.
                    let reason = self.crash_reason(&container_id).await;
                    if let Some(record) = self
                        .store
                        .get(&desired.owner_pubkey, &desired.slug)
                        .cloned()
                    {
                        self.record_start_failure(&desired, &record, reason).await?;
                    }
                }
                self.start_agent(&desired, Some(&container_id), !crashed)
                    .await
            }
            Action::Stop {
                owner_pubkey,
                slug,
                container_id,
            } => {
                // Volume preserved: a disabled agent keeps its workspace so
                // re-enabling resumes rather than starting from nothing.
                self.containers.remove(&container_id, None).await?;
                self.store.update(&owner_pubkey, &slug, |r| {
                    r.spec_hash = None;
                })?;
                self.publish_status(
                    &slug,
                    &owner_pubkey,
                    SpawnPhase::Stopped,
                    None,
                    None,
                    None,
                    0,
                )
                .await
            }
            Action::Delete {
                owner_pubkey,
                slug,
                container_id,
            } => {
                let volume = volume_name(&owner_pubkey, &slug);
                if let Some(id) = container_id {
                    self.containers.remove(&id, Some(&volume)).await?;
                }
                self.store.remove(&owner_pubkey, &slug)?;
                // Clear the status too, or clients keep a row for an agent that
                // is gone — stuck in whatever phase it died in.
                if let Err(e) = self.relay.tombstone_status(&slug, &owner_pubkey).await {
                    warn!(slug = %slug, "failed to tombstone status: {e:#}");
                }
                info!(slug = %slug, "agent deleted");
                Ok(())
            }
            Action::RemoveOrphan { container_id } => {
                warn!(container = %container_id, "removing container with no spawner record");
                self.containers.remove(&container_id, None).await
            }
        }
    }

    async fn provision(&mut self, desired: &DesiredAgent) -> Result<()> {
        // A spec that names an identity is relocating an existing agent, not
        // creating one. Its pubkey carries the agent's channels, profile, and
        // NIP-AE memory — none of which survive a new key — so the record is
        // created around that pubkey and the secret arrives over the encrypted
        // handshake. Until it does the record has no key and is not startable
        // (`AgentRecord::is_attested`).
        if let Some(agent_pubkey) = desired.spec.agent_pubkey.as_deref() {
            let record = AgentRecord {
                slug: desired.slug.clone(),
                owner_pubkey: desired.owner_pubkey.clone(),
                agent_pubkey: agent_pubkey.to_ascii_lowercase(),
                private_key_nsec: String::new(),
                auth_tag: None,
                pending_nonce: None,
                attestation_sent_at: None,
                spec_hash: None,
                prompt: None,
                restart_count: 0,
                last_failure_at: None,
                carried_team_instructions: None,
            };
            info!(
                slug = %record.slug,
                agent = %record.agent_pubkey,
                "adopting an existing agent; requesting its key from the owner"
            );
            self.store.put(record)?;
            return self
                .request_attestation(&desired.owner_pubkey, &desired.slug)
                .await;
        }

        let keys = Keys::generate();
        let record = AgentRecord {
            slug: desired.slug.clone(),
            owner_pubkey: desired.owner_pubkey.clone(),
            agent_pubkey: keys.public_key().to_hex(),
            private_key_nsec: {
                use nostr::nips::nip19::ToBech32;
                keys.secret_key()
                    .to_bech32()
                    .context("failed to encode agent secret key")?
            },
            auth_tag: None,
            pending_nonce: None,
            attestation_sent_at: None,
            spec_hash: None,
            prompt: None,
            restart_count: 0,
            last_failure_at: None,
            carried_team_instructions: None,
        };

        info!(
            slug = %record.slug,
            agent = %record.agent_pubkey,
            "minted agent key; requesting owner attestation"
        );
        // Persist before sending: a crash between the two must not lose the key
        // for a pubkey the owner is about to attest.
        self.store.put(record)?;
        self.request_attestation(&desired.owner_pubkey, &desired.slug)
            .await
    }

    async fn request_attestation(&mut self, owner_pubkey: &str, slug: &str) -> Result<()> {
        let Some(record) = self.store.get(owner_pubkey, slug).cloned() else {
            return Ok(());
        };
        let owner = PublicKey::parse(owner_pubkey).context("invalid owner pubkey")?;

        let nonce = attestation::new_nonce();
        let frame = buzz_sdk::spawner::AttestationFrame::Request {
            spec_slug: record.slug.clone(),
            agent_pubkey: record.agent_pubkey.clone(),
            conditions: String::new(),
            nonce: nonce.clone(),
        };

        self.relay.send_attestation(&owner, &frame).await?;
        self.store.update(owner_pubkey, slug, |r| {
            r.pending_nonce = Some(nonce);
            r.attestation_sent_at = Some(chrono::Utc::now().timestamp());
        })?;

        self.publish_status(
            slug,
            owner_pubkey,
            SpawnPhase::PendingAttestation,
            Some(&record.agent_pubkey),
            None,
            None,
            record.restart_count,
        )
        .await
    }

    /// Create (or replace) an agent's container.
    ///
    /// `reset_failures` is false when restarting after a crash. Creating a
    /// container always succeeds even when the process inside it exits
    /// immediately, so clearing the counters on creation would erase the crash
    /// that just happened and the backoff would never grow — the container
    /// would be recreated on every pass forever.
    async fn start_agent(
        &mut self,
        desired: &DesiredAgent,
        replace: Option<&str>,
        reset_failures: bool,
    ) -> Result<()> {
        let Some(record) = self
            .store
            .get(&desired.owner_pubkey, &desired.slug)
            .cloned()
        else {
            return Ok(());
        };

        if let Some(container_id) = replace {
            // Volume preserved across a restart: a config change should not
            // wipe the agent's workspace.
            self.containers.remove(container_id, None).await?;
        }

        let hash = desired.spec_hash();
        // A prompt that will not resolve is a persistent failure, not a
        // transient one: propagating it here would skip the backoff bookkeeping
        // below and the reconcile loop would retry on every inbound event,
        // hammering the relay until it rate-limits us.
        let prompt = match self.resolve_prompt(desired).await {
            Ok(prompt) => prompt,
            Err(e) => {
                return self
                    .record_start_failure(desired, &record, format!("{e:#}"))
                    .await;
            }
        };
        let (cpu_millis, memory_mib) = self.resources_for(&desired.spec);

        let spec = ContainerSpec {
            name: record.container_name(),
            image: self.config.agent_image.clone(),
            agent_pubkey: record.agent_pubkey.clone(),
            slug: record.slug.clone(),
            spawner_pubkey: self.config.keys.public_key().to_hex(),
            env: build_agent_env(
                &record,
                &desired.spec,
                &prompt,
                &self.config.agent_relay_url,
                &self.config.agent_env,
                &AgentRuntime {
                    command: self.config.agent_command.as_deref(),
                    args: self.config.agent_args.as_deref(),
                },
            ),
            cpu_millis,
            memory_mib,
            volume_name: volume_name(&desired.owner_pubkey, &desired.slug),
        };

        match self.containers.create(&spec).await {
            Ok(id) => {
                info!(slug = %desired.slug, container = %id, "agent started");
                self.store
                    .update(&desired.owner_pubkey, &desired.slug, |r| {
                        r.spec_hash = Some(hash.clone());
                        if reset_failures {
                            r.restart_count = 0;
                            r.last_failure_at = None;
                        }
                    })?;
                if !reset_failures {
                    // The Failed status published moments ago is the truthful
                    // one; overwriting it with Running would hide a crash loop
                    // behind a green badge that flickers.
                    return Ok(());
                }
                self.publish_status(
                    &desired.slug,
                    &desired.owner_pubkey,
                    SpawnPhase::Running,
                    Some(&record.agent_pubkey),
                    Some(&hash),
                    None,
                    0,
                )
                .await
            }
            Err(e) => {
                self.record_start_failure(desired, &record, format!("{e:#}"))
                    .await
            }
        }
    }

    /// Best-effort explanation for why a container died, from its own logs.
    async fn crash_reason(&self, container_id: &str) -> String {
        match self.containers.logs(container_id, 20).await {
            Ok(logs) => {
                let tail: String = logs
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .rev()
                    .take(3)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join(" | ");
                if tail.is_empty() {
                    "container exited without output".to_string()
                } else {
                    format!("container exited: {}", truncate(&tail, 900))
                }
            }
            Err(e) => format!("container exited; logs unavailable: {e}"),
        }
    }

    /// Record a failed start: bump the backoff counters and report it.
    ///
    /// Every path that fails to bring an agent up must go through here. A
    /// failure that skips it is retried on the next inbound event with no delay,
    /// which turns one broken agent into a request storm against the relay.
    async fn record_start_failure(
        &mut self,
        desired: &DesiredAgent,
        record: &AgentRecord,
        message: String,
    ) -> Result<()> {
        error!(slug = %desired.slug, "failed to start agent: {message}");
        let restart_count = record.restart_count.saturating_add(1);
        self.store
            .update(&desired.owner_pubkey, &desired.slug, |r| {
                r.restart_count = restart_count;
                r.last_failure_at = Some(chrono::Utc::now().timestamp());
            })?;
        self.publish_status(
            &desired.slug,
            &desired.owner_pubkey,
            SpawnPhase::Failed,
            Some(&record.agent_pubkey),
            None,
            Some(message),
            restart_count,
        )
        .await
    }

    /// Clamp a spec's requested resources to the host's configured ceiling.
    ///
    /// Clamped rather than rejected: an owner asking for more than the host
    /// allows should get a smaller agent, not a broken one.
    fn resources_for(&self, spec: &SpawnerAgentSpec) -> (u32, u32) {
        let requested = spec.resources.unwrap_or_default();
        (
            requested
                .cpu_millis
                .unwrap_or(DEFAULT_CPU_MILLIS)
                .min(self.config.max_cpu_millis),
            requested
                .memory_mib
                .unwrap_or(DEFAULT_MEMORY_MIB)
                .min(self.config.max_memory_mib),
        )
    }

    /// Resolve prompt material for a spec.
    ///
    /// A spec that names a `persona_id` resolves through the owner's kind:30175
    /// persona, matching how the desktop slims kind:30177 — the prompt lives in
    /// exactly one place. A definition-less spec carries its own prompt inline.
    async fn resolve_prompt(&mut self, desired: &DesiredAgent) -> Result<ResolvedPrompt> {
        let mut prompt = self.resolve_prompt_uncached(desired).await?;
        // Host defaults fill only what the spec and persona left empty, so an
        // explicit choice always wins over the operator's fallback.
        if prompt.provider.is_none() {
            prompt.provider = self.config.default_provider.clone();
        }
        if prompt.model.is_none() {
            prompt.model = self.config.default_model.clone();
        }
        Ok(prompt)
    }

    async fn resolve_prompt_uncached(&mut self, desired: &DesiredAgent) -> Result<ResolvedPrompt> {
        // Prompt delivered over the encrypted handshake wins: it is the only
        // source that works for an unshared persona, and it is what the owner
        // most recently intended.
        if let Some(record) = self.store.get(&desired.owner_pubkey, &desired.slug) {
            if let Some(prompt) = record.prompt.as_ref().filter(|p| !p.is_empty()) {
                return Ok(resolved_from_record(record, prompt));
            }
        }

        let Some(persona_id) = desired.spec.persona_id.as_deref() else {
            return Ok(ResolvedPrompt {
                system_prompt: desired.spec.system_prompt.clone(),
                team_instructions: None,
                model: desired.spec.model.clone(),
                provider: desired.spec.provider.clone(),
            });
        };

        let owner = PublicKey::parse(&desired.owner_pubkey).context("invalid owner pubkey")?;
        let personas = self.relay.fetch_personas(&owner).await?;

        let Some(event) = personas.get(persona_id) else {
            // Falling back to the inline fields keeps a definition-less spec
            // working, and surfaces a clear failure for one that genuinely
            // depends on a persona the spawner cannot read.
            if desired.spec.system_prompt.is_none() {
                // Kind 30175 is author-only unless tagged ["shared","true"], and
                // the spawner is not the author. It authenticates as itself with
                // no NIP-OA attestation, so no owner delegation applies to it —
                // the two ways out are both the owner's to take.
                anyhow::bail!(
                    "no prompt for persona {persona_id}: it is not readable by \
                     this spawner and none was delivered over the attestation \
                     handshake. Re-approve the agent from a client that sends \
                     prompt material, or publish the persona with \
                     [\"shared\",\"true\"]."
                );
            }
            warn!(
                persona = %persona_id,
                "persona not found; falling back to the spec's inline prompt"
            );
            return Ok(ResolvedPrompt {
                system_prompt: desired.spec.system_prompt.clone(),
                team_instructions: None,
                model: desired.spec.model.clone(),
                provider: desired.spec.provider.clone(),
            });
        };

        let body: PersonaContent = serde_json::from_str(event.content.as_ref())
            .with_context(|| format!("failed to parse persona {persona_id}"))?;

        Ok(ResolvedPrompt {
            system_prompt: body
                .system_prompt
                .or_else(|| desired.spec.system_prompt.clone()),
            team_instructions: body.team_instructions,
            model: body.model.or_else(|| desired.spec.model.clone()),
            provider: body.provider.or_else(|| desired.spec.provider.clone()),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish_status(
        &mut self,
        slug: &str,
        owner_pubkey: &str,
        phase: SpawnPhase,
        agent_pubkey: Option<&str>,
        spec_hash: Option<&str>,
        error: Option<String>,
        restart_count: u32,
    ) -> Result<()> {
        let prompt_hash = prompt_hash_for(self.store.get(owner_pubkey, slug));
        let status = SpawnerAgentStatus {
            phase,
            agent_pubkey: agent_pubkey.map(str::to_string),
            spec_hash: spec_hash.map(str::to_string),
            error,
            restart_count,
            prompt_hash,
        };
        self.relay.publish_status(slug, owner_pubkey, &status).await
    }
}

/// Hash of a record's cached prompt material, if any, for `prompt_hash` on the
/// published status. Split out as a pure function so it is testable without a
/// running daemon.
fn prompt_hash_for(record: Option<&AgentRecord>) -> Option<String> {
    record.and_then(|r| r.prompt.as_ref()).map(|p| p.hash())
}

/// Team instructions to keep after applying `incoming` to `record`.
///
/// "Previous wins when incoming is None": a desktop prompt update carries only
/// system prompt / model / provider, so without this a model tweak would drop
/// the agent's team instructions at its next restart.
fn carried_team_instructions(
    record: &AgentRecord,
    incoming: &buzz_sdk::spawner::PromptMaterial,
) -> Option<String> {
    incoming
        .team_instructions
        .clone()
        .or_else(|| {
            record
                .prompt
                .as_ref()
                .and_then(|p| p.team_instructions.clone())
        })
        .or_else(|| record.carried_team_instructions.clone())
}

/// Container-env view of a record's stored prompt, with team instructions
/// filled in from [`AgentRecord::carried_team_instructions`] when the stored
/// material itself has none.
fn resolved_from_record(
    record: &AgentRecord,
    prompt: &buzz_sdk::spawner::PromptMaterial,
) -> ResolvedPrompt {
    ResolvedPrompt {
        system_prompt: prompt.system_prompt.clone(),
        team_instructions: prompt
            .team_instructions
            .clone()
            .or_else(|| record.carried_team_instructions.clone()),
        model: prompt.model.clone(),
        provider: prompt.provider.clone(),
    }
}

/// Truncate on a char boundary so a multi-byte log tail cannot panic.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// The subset of a kind:30175 persona body the spawner needs.
///
/// Deliberately narrow: the spawner reads a prompt, not a whole persona record,
/// and unknown fields are dropped rather than carried into the container env.
#[derive(serde::Deserialize)]
struct PersonaContent {
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    team_instructions: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    provider: Option<String>,
}

/// Per-agent volume name, scoped by owner for the same reason container names
/// are: slugs are only unique per owner.
fn volume_name(owner_pubkey: &str, slug: &str) -> String {
    format!(
        "buzz-agent-{}-{}",
        &owner_pubkey[..12.min(owner_pubkey.len())],
        slug
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_names_are_scoped_per_owner() {
        assert_ne!(
            volume_name(&"a".repeat(64), "fizz"),
            volume_name(&"b".repeat(64), "fizz")
        );
    }

    fn test_record(prompt: Option<buzz_sdk::spawner::PromptMaterial>) -> AgentRecord {
        AgentRecord {
            slug: "fizz".into(),
            owner_pubkey: "a".repeat(64),
            agent_pubkey: "b".repeat(64),
            private_key_nsec: String::new(),
            auth_tag: None,
            pending_nonce: None,
            attestation_sent_at: None,
            spec_hash: None,
            prompt,
            restart_count: 0,
            last_failure_at: None,
            carried_team_instructions: None,
        }
    }

    #[test]
    fn prompt_hash_for_present_prompt_matches_material_hash() {
        let material = buzz_sdk::spawner::PromptMaterial {
            system_prompt: Some("be Fizz".into()),
            team_instructions: None,
            model: None,
            provider: None,
        };
        let record = test_record(Some(material.clone()));
        assert_eq!(prompt_hash_for(Some(&record)), Some(material.hash()));
    }

    #[test]
    fn prompt_hash_for_absent_prompt_is_none() {
        let record = test_record(None);
        assert_eq!(prompt_hash_for(Some(&record)), None);
        assert_eq!(prompt_hash_for(None), None);
    }

    #[test]
    fn prompt_update_without_team_instructions_keeps_them_for_the_container() {
        // The desktop's prompt update only carries system prompt / model /
        // provider. Applying one must not wipe team instructions delivered
        // earlier over the attestation handshake, and the status hash must
        // still equal the hash of the frame material the owner sent.
        let mut record = test_record(Some(buzz_sdk::spawner::PromptMaterial {
            system_prompt: Some("be Fizz".into()),
            team_instructions: Some("ship small PRs".into()),
            model: Some("old-model".into()),
            provider: None,
        }));

        let incoming = buzz_sdk::spawner::PromptMaterial {
            system_prompt: Some("be Fizz".into()),
            team_instructions: None,
            model: Some("new-model".into()),
            provider: None,
        };

        // Mirrors `apply_prompt_update`'s store mutation.
        record.carried_team_instructions = carried_team_instructions(&record, &incoming);
        record.prompt = Some(incoming.clone());

        let resolved = resolved_from_record(&record, record.prompt.as_ref().unwrap());
        assert_eq!(
            resolved.team_instructions.as_deref(),
            Some("ship small PRs")
        );
        assert_eq!(resolved.model.as_deref(), Some("new-model"));
        assert_eq!(prompt_hash_for(Some(&record)), Some(incoming.hash()));

        // And a later update still carries them, now via the carry field.
        let second = buzz_sdk::spawner::PromptMaterial {
            system_prompt: Some("be Fizzier".into()),
            ..Default::default()
        };
        assert_eq!(
            carried_team_instructions(&record, &second).as_deref(),
            Some("ship small PRs")
        );
    }

    #[test]
    fn an_explicit_team_instruction_in_the_update_wins() {
        let record = test_record(Some(buzz_sdk::spawner::PromptMaterial {
            team_instructions: Some("old".into()),
            ..Default::default()
        }));
        let incoming = buzz_sdk::spawner::PromptMaterial {
            team_instructions: Some("new".into()),
            ..Default::default()
        };
        assert_eq!(
            carried_team_instructions(&record, &incoming).as_deref(),
            Some("new")
        );
    }

    #[test]
    fn persona_content_ignores_unknown_fields() {
        // A persona event carrying extra keys must not leak them anywhere.
        let body: PersonaContent = serde_json::from_str(
            r#"{"system_prompt":"be Fizz","env_vars":{"K":"v"},"private_key_nsec":"nsec1x"}"#,
        )
        .unwrap();
        assert_eq!(body.system_prompt.as_deref(), Some("be Fizz"));
        assert!(body.model.is_none());
    }
}
