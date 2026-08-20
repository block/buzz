//! Conversions between the persona-command shape [`AgentDefinition`] and the
//! unified store record [`ManagedAgentRecord`], split from `types.rs`
//! (file-size cap). These are the §2.7 compatibility seam: `into_agent_record`
//! projects a fresh persona into a keyless record, `to_definition_view`
//! presents a record back in the legacy command shape, and
//! `apply_definition_view` is the merge-preserving inverse that keeps
//! record-only fields intact across an ordinary persona save.

use super::{
    default_agent_parallelism, AgentDefinition, BackendKind, ManagedAgentRecord, RespondTo,
    DEFAULT_ACP_COMMAND, DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
};

impl AgentDefinition {
    /// Project this persona onto a key-less unified [`ManagedAgentRecord`]
    /// (Phase 1A store fold). Identity fields stay empty — keys are minted on
    /// first start. `AgentDefinition.id` becomes `slug`, preserving the 30175
    /// event coordinate (`d_tag = slug`) across the fold.
    pub fn into_agent_record(self) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: String::new(),
            name: self.display_name.clone(),
            persona_id: None,
            private_key_nsec: String::new(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: self.avatar_url,
            acp_command: DEFAULT_ACP_COMMAND.to_string(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: Vec::new(),
            mcp_command: String::new(),
            turn_timeout_seconds: DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: default_agent_parallelism(),
            system_prompt: (!self.system_prompt.is_empty()).then_some(self.system_prompt),
            model: self.model,
            provider: self.provider,
            persona_source_version: None,
            env_vars: self.env_vars,
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: BackendKind::default(),
            backend_agent_id: None,
            provider_policy_pending: false,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: RespondTo::default(),
            respond_to_allowlist: Vec::new(),
            display_name: Some(self.display_name),
            slug: Some(self.id),
            runtime: self.runtime,
            name_pool: self.name_pool,
            is_builtin: self.is_builtin,
            is_active: self.is_active,
            // Catalog visibility is relay+owner scoped, not definition-global.
            shared: false,
            source_team: self.source_team,
            source_team_persona_slug: self.source_team_persona_slug,
            catalog_source: self.catalog_source,
            // Library linkage is authored only by §3's projection machinery;
            // a freshly projected definition carries none.
            library_ref: None,
            library_applied_revision: None,
            last_completed_deploy_attempt_id: None,
            definition_respond_to: self.respond_to,
            definition_respond_to_allowlist: self.respond_to_allowlist,
            definition_parallelism: self.parallelism,
            relay_mesh: None,
            effort_level: None,
        }
    }
}

impl ManagedAgentRecord {
    /// Present a key-less definition record back in the legacy
    /// [`AgentDefinition`] shape — the compatibility view the persona command
    /// surface serves until Phase 1B unifies the UI. Inverse of
    /// [`AgentDefinition::into_agent_record`] for the fields personas carry.
    pub fn to_definition_view(&self) -> Option<AgentDefinition> {
        let slug = self.slug.clone()?;
        Some(AgentDefinition {
            id: slug,
            display_name: self
                .display_name
                .clone()
                .unwrap_or_else(|| self.name.clone()),
            avatar_url: self.avatar_url.clone(),
            system_prompt: self.system_prompt.clone().unwrap_or_default(),
            runtime: self.runtime.clone(),
            model: self.model.clone(),
            provider: self.provider.clone(),
            name_pool: self.name_pool.clone(),
            is_builtin: self.is_builtin,
            is_active: self.is_active,
            // Projected by `list_personas` from the active retention scope.
            shared: false,
            source_team: self.source_team.clone(),
            source_team_persona_slug: self.source_team_persona_slug.clone(),
            catalog_source: self.catalog_source.clone(),
            env_vars: self.env_vars.clone(),
            respond_to: self.definition_respond_to.clone(),
            respond_to_allowlist: self.definition_respond_to_allowlist.clone(),
            parallelism: self.definition_parallelism,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        })
    }

    /// Inverse of [`to_definition_view`](Self::to_definition_view) for EXACTLY
    /// the fields the persona view carries. Every other field of `self` — the
    /// instance-side slots, and (once §3 lands) `library_ref`,
    /// `library_applied_revision`, `last_completed_deploy_attempt_id`, plus any
    /// future non-view field — is untouched by construction: this writes only
    /// the slots [`to_definition_view`](Self::to_definition_view) reads.
    ///
    /// This is the seam that makes an ordinary persona save merge-preserving.
    /// At head, `save_personas` reconstructed every record wholesale through
    /// [`into_agent_record`](AgentDefinition::into_agent_record), so any field
    /// living only on `ManagedAgentRecord` was erased by an unrelated save.
    /// Applying the view onto the canonical raw record instead keeps those
    /// fields intact. The value mapping mirrors `into_agent_record` so a record
    /// updated this way is byte-identical to one freshly projected.
    pub(crate) fn apply_definition_view(&mut self, view: &AgentDefinition) {
        self.slug = Some(view.id.clone());
        self.display_name = Some(view.display_name.clone());
        self.name = view.display_name.clone();
        self.avatar_url = view.avatar_url.clone();
        self.system_prompt = (!view.system_prompt.is_empty()).then(|| view.system_prompt.clone());
        self.runtime = view.runtime.clone();
        self.model = view.model.clone();
        self.provider = view.provider.clone();
        self.name_pool = view.name_pool.clone();
        self.is_builtin = view.is_builtin;
        self.is_active = view.is_active;
        // Catalog visibility is relay+owner scoped, never definition-global —
        // `view.shared` is a command projection and must not be persisted.
        self.shared = false;
        self.source_team = view.source_team.clone();
        self.source_team_persona_slug = view.source_team_persona_slug.clone();
        self.catalog_source = view.catalog_source.clone();
        self.env_vars = view.env_vars.clone();
        self.definition_respond_to = view.respond_to.clone();
        self.definition_respond_to_allowlist = view.respond_to_allowlist.clone();
        self.definition_parallelism = view.parallelism;
        self.created_at = view.created_at.clone();
        self.updated_at = view.updated_at.clone();
    }
}
