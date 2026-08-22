use std::collections::{BTreeMap, HashMap};

use buzz_core_pkg::private_managed_agent::Payload;

use super::{
    validate_respond_to_allowlist, validate_user_env_keys, BackendKind, ManagedAgentRecord,
    RelayMeshConfig, RespondTo, DEFAULT_ACP_COMMAND, DEFAULT_AGENT_PARALLELISM,
    DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
};

#[derive(Clone)]
pub(crate) struct PrivateConfigPatch {
    pubkey: String,
    name: String,
    private_key_nsec: String,
    auth_tag: Option<String>,
    relay_url: String,
    persona_id: Option<String>,
    runtime: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    system_prompt: Option<String>,
    parallelism: u32,
    respond_to: RespondTo,
    respond_to_allowlist: Vec<String>,
    agent_command_override: Option<String>,
    agent_args: Vec<String>,
    idle_timeout_seconds: Option<u64>,
    max_turn_duration_seconds: Option<u64>,
    env_vars: BTreeMap<String, String>,
    backend: BackendKind,
    backend_agent_id: Option<String>,
    team_id: Option<String>,
    persona_name_in_team: Option<String>,
    relay_mesh: Option<RelayMeshConfig>,
    updated_at: String,
}

impl PrivateConfigPatch {
    /// Convert a payload that has already passed the codec's
    /// `validate_and_decrypt` gate. Callers must not feed unvalidated wire data.
    pub(crate) fn from_payload(payload: Payload) -> Result<Self, String> {
        let config = payload.config;
        let backend = serde_json::from_value(config.backend)
            .map_err(|error| format!("invalid private managed-agent backend: {error}"))?;
        let relay_mesh = config
            .relay_mesh
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| format!("invalid private managed-agent relay_mesh: {error}"))?;
        let respond_to = config
            .respond_to
            .as_deref()
            .map(RespondTo::parse_wire)
            .transpose()?
            .unwrap_or_default();
        let respond_to_allowlist = validate_respond_to_allowlist(&config.respond_to_allowlist)?;
        if respond_to == RespondTo::Allowlist && respond_to_allowlist.is_empty() {
            return Err("private managed-agent allowlist mode requires at least one pubkey".into());
        }
        validate_user_env_keys(&config.env_vars)?;
        let parallelism = config.parallelism.unwrap_or(DEFAULT_AGENT_PARALLELISM);
        if !(1..=32).contains(&parallelism) {
            return Err("private managed-agent parallelism must be between 1 and 32".into());
        }

        Ok(Self {
            pubkey: payload.agent_pubkey,
            name: config.name,
            private_key_nsec: payload.identity.private_key_nsec,
            auth_tag: payload.identity.auth_tag,
            relay_url: config.relay_url,
            persona_id: config.persona_id,
            runtime: config.runtime,
            model: config.model,
            provider: config.provider,
            system_prompt: config.system_prompt,
            parallelism,
            respond_to,
            respond_to_allowlist,
            agent_command_override: config.agent_command_override,
            agent_args: config.agent_args,
            idle_timeout_seconds: config.idle_timeout_seconds,
            max_turn_duration_seconds: config.max_turn_duration_seconds,
            env_vars: config.env_vars,
            backend,
            backend_agent_id: config.backend_agent_id,
            team_id: config.team_id,
            persona_name_in_team: config.persona_name_in_team,
            relay_mesh,
            updated_at: payload.updated_at,
        })
    }

    fn apply(&self, record: &mut ManagedAgentRecord) {
        record.pubkey.clone_from(&self.pubkey);
        record.name.clone_from(&self.name);
        record.private_key_nsec.clone_from(&self.private_key_nsec);
        record.auth_tag.clone_from(&self.auth_tag);
        record.relay_url.clone_from(&self.relay_url);
        record.persona_id.clone_from(&self.persona_id);
        record.runtime.clone_from(&self.runtime);
        record.model.clone_from(&self.model);
        record.provider.clone_from(&self.provider);
        record.system_prompt.clone_from(&self.system_prompt);
        record.parallelism = self.parallelism;
        record.respond_to = self.respond_to;
        record
            .respond_to_allowlist
            .clone_from(&self.respond_to_allowlist);
        record
            .agent_command_override
            .clone_from(&self.agent_command_override);
        record.agent_args.clone_from(&self.agent_args);
        record.idle_timeout_seconds = self.idle_timeout_seconds;
        record.max_turn_duration_seconds = self.max_turn_duration_seconds;
        record.env_vars.clone_from(&self.env_vars);
        record.backend.clone_from(&self.backend);
        record.backend_agent_id.clone_from(&self.backend_agent_id);
        record.team_id.clone_from(&self.team_id);
        record
            .persona_name_in_team
            .clone_from(&self.persona_name_in_team);
        record.relay_mesh.clone_from(&self.relay_mesh);
        record.updated_at.clone_from(&self.updated_at);
    }

    fn fresh_record(&self) -> ManagedAgentRecord {
        let mut record = ManagedAgentRecord {
            pubkey: String::new(),
            name: String::new(),
            persona_id: None,
            team_id: None,
            private_key_nsec: String::new(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: None,
            acp_command: DEFAULT_ACP_COMMAND.into(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: DEFAULT_AGENT_PARALLELISM,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            env_vars: BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: BackendKind::Local,
            backend_agent_id: None,
            provider_policy_pending: false,
            provider_binary_path: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: self.updated_at.clone(),
            updated_at: self.updated_at.clone(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: RespondTo::default(),
            respond_to_allowlist: vec![],
            display_name: None,
            slug: None,
            runtime: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: vec![],
            definition_parallelism: None,
            relay_mesh: None,
            effort_level: None,
        };
        self.apply(&mut record);
        record
    }
}

#[derive(Default)]
pub(crate) struct PrivateConfigOverlay(HashMap<String, PrivateConfigPatch>);

impl PrivateConfigOverlay {
    #[cfg(test)]
    pub(crate) fn insert(&mut self, payload: Payload) -> Result<(), String> {
        let patch = PrivateConfigPatch::from_payload(payload)?;
        self.0.insert(patch.pubkey.clone(), patch);
        Ok(())
    }

    pub(crate) fn insert_patch(&mut self, patch: PrivateConfigPatch) {
        self.0.insert(patch.pubkey.clone(), patch);
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn remove(&mut self, pubkey: &str) {
        self.0.remove(pubkey);
    }

    /// Write-through for a SELF-AUTHORED retain: adopt the kind:30179 head this
    /// device just wrote as the config it is following.
    ///
    /// Both existing fill paths are inbound-only — `insert_patch` on an
    /// `Applied` inbound event (`personas/inbound.rs`) and boot hydration
    /// below — and neither fires for an event this device authored: the
    /// relay's echo of our own event dedupes to `Skipped` against the row we
    /// already retained. So without this the overlay stays pinned at the last
    /// *received* generation, and the NEXT edit resolves that stale patch on
    /// top of the fresher disk record, silently reverting the previous edit
    /// and publishing the reversion as a valid successor.
    ///
    /// Absorbing unconditionally (not only when the retain reported a change)
    /// is safe and strictly convergent: the overlay is never ahead of
    /// retention — every insert either comes from a row written in the same
    /// step or is read back out of retention. A missing/undecodable head
    /// leaves the current entry alone rather than clearing it.
    pub(crate) fn absorb_retained_head(
        &mut self,
        conn: &rusqlite::Connection,
        owner_keys: &nostr::Keys,
        agent_pubkey: &str,
    ) -> Result<(), String> {
        let row = crate::managed_agents::retention::get_retained_event(
            conn,
            buzz_core_pkg::kind::KIND_PRIVATE_MANAGED_AGENT,
            &owner_keys.public_key().to_hex(),
            agent_pubkey,
        )?;
        if let Some(patch) = row
            .as_ref()
            .and_then(|row| patch_from_retained_row(&row.raw_event, owner_keys))
        {
            self.insert_patch(patch);
        }
        Ok(())
    }

    pub(crate) fn resolve_local_record(&self, record: &ManagedAgentRecord) -> ManagedAgentRecord {
        let mut resolved = record.clone();
        if let Some(patch) = self.0.get(&record.pubkey) {
            patch.apply(&mut resolved);
        }
        resolved
    }

    pub(crate) fn materialize_relay_only_record(
        &self,
        pubkey: &str,
        local: &[ManagedAgentRecord],
    ) -> Option<ManagedAgentRecord> {
        if local.iter().any(|record| record.pubkey == pubkey) {
            return None;
        }
        let mut record = self.0.get(pubkey)?.fresh_record();
        // Persona definitions are device-local. A fresh device can still run the
        // complete relay snapshot, but must not bind it to an absent local persona.
        record.persona_id = None;
        Some(record)
    }

    pub(crate) fn resolved_records(&self, local: &[ManagedAgentRecord]) -> Vec<ManagedAgentRecord> {
        let mut resolved = local.to_vec();
        for record in &mut resolved {
            if let Some(patch) = self.0.get(&record.pubkey) {
                patch.apply(record);
            }
        }
        let mut relay_only: Vec<_> = self
            .0
            .values()
            .filter(|patch| !local.iter().any(|record| record.pubkey == patch.pubkey))
            .map(PrivateConfigPatch::fresh_record)
            .collect();
        relay_only.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
        resolved.extend(relay_only);
        resolved
    }
}

pub(crate) fn resolved_local_record(
    state: &crate::app_state::AppState,
    record: &ManagedAgentRecord,
) -> Result<ManagedAgentRecord, String> {
    state
        .private_managed_agent_overlay
        .lock()
        .map_err(|error| error.to_string())
        .map(|overlay| overlay.resolve_local_record(record))
}

pub(crate) fn copy_lifecycle_state(
    destination: &mut ManagedAgentRecord,
    source: &ManagedAgentRecord,
) {
    destination.runtime_pid = source.runtime_pid;
    destination
        .last_started_at
        .clone_from(&source.last_started_at);
    destination
        .last_stopped_at
        .clone_from(&source.last_stopped_at);
    destination.last_exit_code = source.last_exit_code;
    destination.last_error.clone_from(&source.last_error);
    destination.last_error_code = source.last_error_code;
}

pub(crate) fn materialize_relay_only_agent(
    app: &tauri::AppHandle,
    state: &crate::app_state::AppState,
    pubkey: &str,
) -> Result<(), String> {
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if state
        .shutdown_started
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("desktop shutdown has started".into());
    }
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = super::load_managed_agents(app)?;
    let relay_only = state
        .private_managed_agent_overlay
        .lock()
        .map_err(|error| error.to_string())?
        .materialize_relay_only_record(pubkey, &records);
    if let Some(record) = relay_only {
        if record.backend != BackendKind::Local {
            return Err("relay-only provider agents cannot be started on this device".into());
        }
        records.push(record);
        super::save_managed_agents(app, &records)?;
    }
    Ok(())
}

/// Minimal valid decrypted kind:30179 payload for cross-module overlay tests
/// (the pair-spawn, provider-deploy, and provider-access final-use-boundary
/// regressions). Callers mutate `config` fields for scenario-specific shapes.
#[cfg(test)]
pub(crate) fn test_relay_payload(pubkey: &str) -> Payload {
    use buzz_core_pkg::private_managed_agent::{PrivateConfig, PrivateIdentity, FORMAT, VERSION};
    Payload {
        format: FORMAT.into(),
        version: VERSION,
        agent_pubkey: pubkey.into(),
        owner_pubkey: "11".repeat(32),
        generation: 2,
        previous_event_id: None,
        updated_at: "2026-08-20T00:00:00Z".into(),
        identity: PrivateIdentity {
            private_key_nsec: "nsec-relay".into(),
            auth_tag: None,
        },
        config: PrivateConfig {
            relay_url: "wss://relay.example".into(),
            name: "relay name".into(),
            persona_id: None,
            runtime: Some("goose".into()),
            model: Some("relay-model".into()),
            provider: None,
            system_prompt: Some("relay prompt".into()),
            parallelism: Some(4),
            respond_to: None,
            respond_to_allowlist: vec![],
            agent_command_override: None,
            agent_args: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            env_vars: BTreeMap::new(),
            backend: serde_json::json!({"type":"local"}),
            backend_agent_id: None,
            team_id: None,
            persona_name_in_team: None,
            relay_mesh: None,
            extra: serde_json::Map::new(),
        },
        extensions: BTreeMap::new(),
        extra: serde_json::Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core_pkg::private_managed_agent::{
        Payload, PrivateConfig, PrivateIdentity, FORMAT, VERSION,
    };
    use serde_json::{json, Map};

    fn payload(pubkey: &str, name: &str) -> Payload {
        Payload {
            format: FORMAT.into(),
            version: VERSION,
            agent_pubkey: pubkey.into(),
            owner_pubkey: "11".repeat(32),
            generation: 1,
            previous_event_id: None,
            updated_at: "2026-08-06T00:00:00Z".into(),
            identity: PrivateIdentity {
                private_key_nsec: "nsec-test".into(),
                auth_tag: None,
            },
            config: PrivateConfig {
                relay_url: "wss://relay.example".into(),
                name: name.into(),
                persona_id: None,
                runtime: Some("goose".into()),
                model: Some("m".into()),
                provider: None,
                system_prompt: Some("relay prompt".into()),
                parallelism: None,
                respond_to: None,
                respond_to_allowlist: vec![],
                agent_command_override: None,
                agent_args: vec![],
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
                env_vars: BTreeMap::new(),
                backend: json!({"type":"local"}),
                backend_agent_id: None,
                team_id: None,
                persona_name_in_team: None,
                relay_mesh: None,
                extra: Map::new(),
            },
            extensions: BTreeMap::new(),
            extra: Map::new(),
        }
    }

    #[test]
    fn resolves_overlay_and_relay_only_without_mutating_local() {
        let mut overlay = PrivateConfigOverlay::default();
        overlay.insert(payload("aa", "relay local")).unwrap();
        overlay.insert(payload("bb", "relay only")).unwrap();
        let mut local = overlay.0["aa"].fresh_record();
        local.name = "disk".into();
        local.system_prompt = Some("disk prompt".into());
        let original = local.clone();

        let resolved = overlay.resolved_records(std::slice::from_ref(&local));
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].name, "relay local");
        assert_eq!(resolved[0].system_prompt.as_deref(), Some("relay prompt"));
        assert_eq!(resolved[1].pubkey, "bb");
        assert!(!resolved[1].start_on_app_launch);
        assert_eq!(local, original);
    }

    #[test]
    fn materializes_only_relay_only_record_and_preserves_disk_overlay() {
        let mut overlay = PrivateConfigOverlay::default();
        overlay.insert(payload("aa", "relay local")).unwrap();
        overlay.insert(payload("bb", "relay only")).unwrap();
        let mut local = overlay.0["aa"].fresh_record();
        local.name = "disk".into();
        local.private_key_nsec = "device-local-key".into();

        let resolved = overlay.resolve_local_record(&local);
        assert_eq!(resolved.name, "relay local");
        assert_eq!(resolved.private_key_nsec, "nsec-test");
        assert_eq!(local.name, "disk");
        assert_eq!(local.private_key_nsec, "device-local-key");
        assert!(overlay
            .materialize_relay_only_record("aa", std::slice::from_ref(&local))
            .is_none());

        let relay_only = overlay
            .materialize_relay_only_record("bb", &[local])
            .unwrap();
        assert_eq!(relay_only.name, "relay only");
        assert_eq!(relay_only.private_key_nsec, "nsec-test");
        assert_eq!(relay_only.backend, BackendKind::Local);
        assert!(relay_only.persona_id.is_none());
    }

    #[test]
    fn rejected_patch_preserves_cached_value_and_clear_drops_scope() {
        let mut overlay = PrivateConfigOverlay::default();
        overlay.insert(payload("aa", "valid")).unwrap();
        let mut invalid = payload("aa", "invalid");
        invalid.config.backend = json!({"type":"provider"});
        assert!(overlay.insert(invalid).is_err());
        assert_eq!(overlay.resolved_records(&[])[0].name, "valid");
        overlay.clear();
        assert!(overlay.resolved_records(&[]).is_empty());
    }

    #[test]
    fn unresolved_harness_has_named_refusal() {
        let mut patch = PrivateConfigPatch::from_payload(payload("aa", "agent")).unwrap();
        patch.runtime = Some("missing-custom-harness".into());
        let record = patch.fresh_record();
        let error = crate::managed_agents::try_record_agent_command(&record, &[]).unwrap_err();
        assert_eq!(
            crate::managed_agents::dangling_harness_id(&error),
            Some("missing-custom-harness")
        );
    }
}

/// Decode one retained kind:30179 row into a patch. Shared by boot hydration
/// and the self-authored write-through so both learn config through exactly
/// one decode path. Best-effort: a row that fails to parse, decrypt, or
/// validate yields `None` rather than an error, matching the inbound path's
/// per-record reject.
fn patch_from_retained_row(
    raw_event: &str,
    owner_keys: &nostr::Keys,
) -> Option<PrivateConfigPatch> {
    use buzz_core_pkg::private_managed_agent;
    use nostr::JsonUtil;

    let event = nostr::Event::from_json(raw_event).ok()?;
    let (_, payload) = private_managed_agent::validate_and_decrypt(&event, owner_keys).ok()?;
    PrivateConfigPatch::from_payload(payload).ok()
}

/// Rebuild the in-memory overlay from the retained kind:30179 rows.
///
/// The inbound path only calls `insert_patch` when `retain_inbound_event`
/// returns `Applied`, i.e. when the event is STRICTLY newer than the retained
/// row. After a restart the backfill re-delivers the same events, retention
/// dedupes them to `Skipped`, and the overlay would stay empty for the whole
/// session — every resolve site silently falling back to stale disk config.
/// Hydrating from the durable rows at boot makes relay-primary config survive
/// a restart.
///
/// Best-effort per row: a row that fails to parse, decrypt, or validate is
/// skipped rather than failing the boot, matching the inbound path's
/// per-record reject.
pub(crate) fn hydrate_from_retention(
    conn: &rusqlite::Connection,
    owner_keys: &nostr::Keys,
) -> Result<PrivateConfigOverlay, String> {
    use buzz_core_pkg::kind::KIND_PRIVATE_MANAGED_AGENT;

    let rows = crate::managed_agents::retention::get_retained_events_of_kind(
        conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
    )?;

    let mut overlay = PrivateConfigOverlay::default();
    for row in rows {
        if let Some(patch) = patch_from_retained_row(&row.raw_event, owner_keys) {
            overlay.insert_patch(patch);
        }
    }
    Ok(overlay)
}

/// Guards that each known stale-disk-republish write site actually calls the
/// overlay resolve. The behavioural tests for these sites (in
/// `reconcile/tests.rs` and `personas/update/name_propagation_tests.rs`) can
/// only *model* the ordering: every site is inside a `#[tauri::command]` that
/// needs a live `AppHandle`, so they call `retain_agent_record` directly and
/// stay green even when the production call is deleted. Measured, not assumed:
/// removing the resolve from `agent_models.rs` left the full lib suite at
/// 2261 passed / 0 failed. This module is the only thing that fails when a
/// site loses its resolve — or when a NEW site is added without one.
///
/// A source assertion is a weak instrument (it cannot see ordering, only
/// presence), so it is deliberately paired with the behavioural ordering tests
/// rather than replacing them. It exists because the alternative here is no
/// coverage at all.
#[cfg(test)]
mod write_site_resolve_guard {
    /// `(file, source, expected_resolve_calls)` — every write site that
    /// retains a managed-agent record derived from disk.
    fn sites() -> Vec<(&'static str, &'static str, usize)> {
        vec![
            (
                "commands/agent_models_update.rs",
                include_str!("../commands/agent_models_update.rs"),
                1,
            ),
            // 4 = the 3 sites Carl already resolved correctly (start/stop/
            // delete) plus the pair-start snapshot re-apply. The count is
            // deliberately exact rather than `>= 1`: a lower bound would not
            // notice a site losing its resolve while another gained one.
            (
                "commands/agents.rs",
                include_str!("../commands/agents.rs"),
                4,
            ),
            // 2 = the preflight snapshot resolve plus the locked spawn-record
            // resolve in `start_local_agent_with_preflight` (extracted from
            // agents.rs by the upstream file-size split).
            (
                "commands/agents_lifecycle.rs",
                include_str!("../commands/agents_lifecycle.rs"),
                2,
            ),
            (
                "commands/personas/update.rs",
                include_str!("../commands/personas/update.rs"),
                1,
            ),
            // The launch-restore fold: every Phase-A spawn candidate is
            // resolved through the overlay before Phase B spawns it. Without
            // this, a follower device hydrates relay config B and then
            // auto-starts stale disk config A (obsolete prompt/model/ACL/
            // identity) — the boot-time variant of the stale-republish bug.
            ("managed_agents/restore.rs", include_str!("restore.rs"), 1),
            // 2 = the pair-start spawn record (`start_pair`) plus the
            // multi-community reconcile fan-out candidates — the final-use
            // boundaries Pair Start/Restart and boot reconcile execute.
            // Without these, a follower device showing relay config B
            // pair-starts stale disk config A.
            (
                "managed_agents/runtime_commands.rs",
                include_str!("runtime_commands.rs"),
                2,
            ),
            // The post-deploy-lock payload rebuild: the exact bytes the
            // provider invocation executes after waiting behind another
            // deployment.
            (
                "commands/agents/provider_deploy.rs",
                include_str!("../commands/agents/provider_deploy.rs"),
                1,
            ),
            // Workspace provider-access reconciliation: both the target
            // selection predicate and the redeploy payload read resolved
            // records, not raw disk rows.
            (
                "commands/agents/provider_access.rs",
                include_str!("../commands/agents/provider_access.rs"),
                1,
            ),
        ]
    }

    #[test]
    fn every_stale_republish_write_site_resolves_the_overlay() {
        for (file, source, expected) in sites() {
            let found = source.matches("resolved_local_record(").count();
            assert_eq!(
                found, expected,
                "{file}: expected {expected} `resolved_local_record(` call(s), found {found}. \
                 A write site that retains a disk-derived record without resolving the \
                 relay overlay republishes stale config over a newer relay head as a \
                 validly-chained successor event (see \
                 `sami_probe_2b_stale_disk_republish_over_newer_relay_head`)."
            );
        }
    }

    /// The guard above is a substring count, so prove it can FAIL: a source
    /// with the call removed must not satisfy it. Without this, a typo in the
    /// searched string would make every row vacuously pass.
    #[test]
    fn guard_detects_a_missing_resolve_call() {
        for (file, source, _) in sites() {
            let stripped = source.replace("resolved_local_record(", "REMOVED(");
            assert_eq!(
                stripped.matches("resolved_local_record(").count(),
                0,
                "{file}: negative control — the guard's search string must actually \
                 match the production call, or the guard is vacuous"
            );
            assert_ne!(
                source.matches("resolved_local_record(").count(),
                0,
                "{file}: positive control — the search string must be present at HEAD"
            );
        }
    }
}
