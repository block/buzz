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
    effort_level: Option<String>,
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
            effort_level: config.effort_level,
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
        record.effort_level.clone_from(&self.effort_level);
        record.updated_at.clone_from(&self.updated_at);
    }

    fn fresh_record(&self) -> ManagedAgentRecord {
        let mut record = ManagedAgentRecord {
            description: None,
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
            team_catalog_source: None,
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
pub(crate) struct PrivateConfigOverlay(
    HashMap<String, PrivateConfigPatch>,
    std::collections::HashSet<String>,
);

impl PrivateConfigOverlay {
    #[cfg(test)]
    pub(crate) fn insert(&mut self, payload: Payload) -> Result<(), String> {
        let patch = PrivateConfigPatch::from_payload(payload)?;
        self.1.remove(&patch.pubkey);
        self.0.insert(patch.pubkey.clone(), patch);
        Ok(())
    }

    pub(crate) fn insert_patch(&mut self, patch: PrivateConfigPatch) {
        self.1.remove(&patch.pubkey);
        self.0.insert(patch.pubkey.clone(), patch);
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
        self.1.clear();
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
    /// step or is read back out of retention. A missing head leaves the
    /// current entry alone; an undecodable retained head is an error.
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
        if let Some(row) = row {
            if super::retention::managed_agent_head_is_deleted(conn, &row)? {
                self.deny_deleted_config(agent_pubkey);
            } else {
                self.insert_patch(patch_from_retained_row(&row, owner_keys)?);
            }
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
        // The definition link travels with the config. Clearing it here would
        // only hold until the next overlay resolve re-applies the head, so
        // the saved row and its resolve would disagree; whether the linked
        // definition is present on this device is checked at the save seam
        // (`materialize_relay_only_agent`), not erased.
        Some(self.0.get(pubkey)?.fresh_record())
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

/// Caller holds the store lock, pairing readiness with the active scope.
pub(crate) fn require_authority_ready(state: &crate::app_state::AppState) -> Result<(), String> {
    if !state
        .managed_agent_authority_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(
            "managed-agent authority is unavailable; retry workspace initialization".into(),
        );
    }
    Ok(())
}

pub(crate) fn resolved_local_record(
    state: &crate::app_state::AppState,
    record: &ManagedAgentRecord,
) -> Result<ManagedAgentRecord, String> {
    require_authority_ready(state)?;
    state
        .private_managed_agent_overlay
        .lock()
        .map_err(|error| error.to_string())
        .and_then(|overlay| {
            overlay.require_config_authority(&record.pubkey)?;
            Ok(overlay.resolve_local_record(record))
        })
}

/// Read one relay-primary record without creating local lifecycle membership.
/// Caller holds store lock. Read surfaces must not use a disk-only lookup or
/// materialize just to display configuration.
pub(crate) fn resolved_record_for_read(
    state: &crate::app_state::AppState,
    local: &[ManagedAgentRecord],
    pubkey: &str,
) -> Result<ManagedAgentRecord, String> {
    require_authority_ready(state)?;
    let overlay = state
        .private_managed_agent_overlay
        .lock()
        .map_err(|e| e.to_string())?;
    overlay.require_config_authority(pubkey)?;
    local
        .iter()
        .find(|record| record.pubkey == pubkey)
        .map(|record| overlay.resolve_local_record(record))
        .or_else(|| overlay.materialize_relay_only_record(pubkey, local))
        .ok_or_else(|| format!("agent {pubkey} not found"))
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

pub(crate) fn materialize_relay_only_agent<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
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
    require_authority_ready(state)?;
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
        // A linked instance whose definition has not reached this device is
        // an orphan; refuse it here, before a lifecycle row exists, with the
        // same message every other orphan boundary uses. The link itself is
        // kept: once the definition syncs, the next attempt materializes.
        if let Some(persona_id) = record.persona_id.as_deref() {
            let personas = super::load_personas(app)?;
            if !personas.iter().any(|persona| persona.id == persona_id) {
                return Err(super::effective_config::ORPHANED_INSTANCE_ERROR.to_string());
            }
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
            effort_level: None,
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
                effort_level: None,
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

    /// A follower runs the leader's effort: the relay head's `effort_level`
    /// overrides the disk column, and a head that cleared it clears the column
    /// (the local picker's "inherit" round-trips as `None`).
    #[test]
    fn follower_adopts_relay_effort_level() {
        let mut overlay = PrivateConfigOverlay::default();
        let mut head = payload("aa", "agent");
        head.config.effort_level = Some("high".into());
        overlay.insert(head).unwrap();
        let mut local = overlay.0["aa"].fresh_record();
        local.effort_level = Some("low".into());

        assert_eq!(
            overlay.resolve_local_record(&local).effort_level.as_deref(),
            Some("high")
        );
        assert_eq!(
            overlay
                .materialize_relay_only_record("aa", &[])
                .unwrap()
                .effort_level
                .as_deref(),
            Some("high")
        );

        overlay.insert(payload("aa", "agent")).unwrap();
        assert_eq!(overlay.resolve_local_record(&local).effort_level, None);
    }

    /// F2: the definition link is portable state, not a device-local detail.
    /// Materialization must carry it verbatim so the record a device saves
    /// and the record the next overlay resolve produces agree — otherwise
    /// materialize clears it, the next START resolve restores it, and the
    /// start path refuses the agent as an orphan it just created.
    #[test]
    fn materialization_preserves_persona_link_through_next_resolve() {
        let mut overlay = PrivateConfigOverlay::default();
        let mut head = payload("aa", "linked");
        head.config.persona_id = Some("def-1".into());
        overlay.insert(head).unwrap();

        let materialized = overlay.materialize_relay_only_record("aa", &[]).unwrap();
        assert_eq!(materialized.persona_id.as_deref(), Some("def-1"));
        assert_eq!(
            overlay
                .resolve_local_record(&materialized)
                .persona_id
                .as_deref(),
            Some("def-1"),
            "the saved row and its next resolve must agree on the definition link"
        );
    }

    /// F2 on the real seam: a persona-linked relay-only head whose definition
    /// has not reached this device is refused by name BEFORE any lifecycle
    /// row lands in the store; once the definition arrives the same call
    /// materializes the record with its link intact, and the production
    /// second resolve keeps it. A definition-less head is the control: it
    /// materializes with no definition present.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn materialize_agent_refuses_until_linked_definition_is_present() {
        use crate::app_state::{build_app_state, AppState};
        use crate::managed_agents::{load_managed_agents, save_personas, AgentDefinition};
        use tauri::Manager;

        // Tauri resolves `app_data_dir` from `$HOME` (macOS) / `$XDG_DATA_HOME`
        // (Linux); hold the crate-wide env lock and point both at a tempdir.
        let _env_lock = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        struct EnvVarGuard(&'static str, Option<std::ffi::OsString>);
        impl EnvVarGuard {
            fn set(key: &'static str, value: &std::path::Path) -> Self {
                let prior = std::env::var_os(key);
                std::env::set_var(key, value);
                Self(key, prior)
            }
        }
        impl Drop for EnvVarGuard {
            fn drop(&mut self) {
                match self.1.take() {
                    Some(prior) => std::env::set_var(self.0, prior),
                    None => std::env::remove_var(self.0),
                }
            }
        }
        let _home = EnvVarGuard::set("HOME", &home);
        let _xdg = EnvVarGuard::set("XDG_DATA_HOME", &home);

        let app = tauri::test::mock_builder()
            .manage(build_app_state())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds headless");
        let state = app.state::<AppState>();

        let linked = "ab".repeat(32);
        let unlinked = "cd".repeat(32);
        {
            let mut overlay = state.private_managed_agent_overlay.lock().unwrap();
            let mut head = test_relay_payload(&linked);
            head.config.persona_id = Some("def-1".into());
            overlay.insert(head).unwrap();
            overlay.insert(test_relay_payload(&unlinked)).unwrap();
        }

        // Even a cached, definition-less head cannot create lifecycle state
        // before the active scope's authority has hydrated.
        let error = materialize_relay_only_agent(app.handle(), &state, &unlinked)
            .expect_err("unhydrated authority must refuse materialization");
        assert!(error.contains("authority is unavailable"));
        assert!(load_managed_agents(app.handle()).unwrap().is_empty());
        state
            .managed_agent_authority_ready
            .store(true, std::sync::atomic::Ordering::Release);

        let error = materialize_relay_only_agent(app.handle(), &state, &linked)
            .expect_err("a linked head must not materialize without its definition");
        assert_eq!(
            error,
            crate::managed_agents::effective_config::ORPHANED_INSTANCE_ERROR
        );
        assert!(
            load_managed_agents(app.handle()).unwrap().is_empty(),
            "refusal must happen before any lifecycle row is saved"
        );

        // Control: no definition link, no definition needed.
        materialize_relay_only_agent(app.handle(), &state, &unlinked).unwrap();
        assert_eq!(load_managed_agents(app.handle()).unwrap().len(), 1);

        // The definition arrives (persona sync, import, ...) and the retry
        // succeeds with the link intact on disk and after the next resolve.
        let definition = AgentDefinition {
            id: "def-1".into(),
            display_name: "Definition".into(),
            avatar_url: None,
            description: None,
            system_prompt: "definition prompt".into(),
            runtime: Some("goose".into()),
            model: None,
            provider: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        };
        save_personas(app.handle(), &[definition]).unwrap();
        materialize_relay_only_agent(app.handle(), &state, &linked).unwrap();

        let records = load_managed_agents(app.handle()).unwrap();
        let saved = records
            .iter()
            .find(|record| record.pubkey == linked)
            .expect("linked record materializes once its definition is present");
        assert_eq!(saved.persona_id.as_deref(), Some("def-1"));
        let resolved = resolved_local_record(&state, saved).unwrap();
        assert_eq!(resolved.persona_id.as_deref(), Some("def-1"));
        assert_eq!(resolved.name, "relay name");
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
/// one decode path. Unlike an untrusted inbound candidate, an already-retained
/// row is known authority: corruption must fail closed, not revive stale disk.
fn patch_from_retained_row(
    row: &super::retention::RetainedEvent,
    owner_keys: &nostr::Keys,
) -> Result<PrivateConfigPatch, String> {
    use buzz_core_pkg::private_managed_agent;
    use nostr::JsonUtil;

    let event = nostr::Event::from_json(&row.raw_event)
        .map_err(|error| format!("invalid retained private authority: {error}"))?;
    let (_, payload) = private_managed_agent::validate_and_decrypt(&event, owner_keys)
        .map_err(|error| format!("unreadable retained private authority: {error}"))?;
    if row.d_tag != payload.agent_pubkey
        || row.pubkey != payload.owner_pubkey
        || row.created_at != event.created_at.as_secs() as i64
        || row.content != event.content
    {
        return Err("retained private authority metadata does not match signed event".into());
    }
    PrivateConfigPatch::from_payload(payload)
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
/// All-or-nothing: an unreadable retained row fails hydration. Its absence
/// from an apparently healthy overlay would silently restore stale disk config.
pub(crate) fn hydrate_from_retention(
    conn: &rusqlite::Connection,
    owner_keys: &nostr::Keys,
) -> Result<PrivateConfigOverlay, String> {
    use buzz_core_pkg::kind::KIND_PRIVATE_MANAGED_AGENT;

    let rows = crate::managed_agents::retention::get_retained_events_by_kind(
        conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
    )?;

    let mut overlay = PrivateConfigOverlay::default();
    overlay.load_deletion_fences(conn, owner_keys)?;
    for row in rows {
        if !super::retention::managed_agent_head_is_deleted(conn, &row)? {
            overlay.insert_patch(patch_from_retained_row(&row, owner_keys)?);
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
            // Exact count: Stop now lives in agents_stop.rs and is tested
            // through its native blocking seam with tracked child processes.
            (
                "commands/agents.rs",
                include_str!("../commands/agents.rs"),
                3,
            ),
            (
                "commands/agents_stop.rs",
                include_str!("../commands/agents_stop.rs"),
                1,
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
            // 2 = the post-deploy-lock payload rebuild (the exact bytes the
            // provider invocation executes after waiting behind another
            // deployment) plus the post-deploy settlement, which writes the
            // returned `backend_agent_id` onto the relay-resolved record and
            // retains it as the next head instead of leaving it disk-only
            // where every resolve site would hide it.
            (
                "commands/agents/provider_deploy.rs",
                include_str!("../commands/agents/provider_deploy.rs"),
                2,
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

#[cfg(test)]
#[path = "private_config_read_tests.rs"]
mod read_tests;

#[path = "private_config_deletion.rs"]
mod deletion;
