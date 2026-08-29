use std::collections::BTreeMap;

use crate::managed_agents::{
    agent_env::baked_build_env,
    discovery::{known_acp_runtime, known_acp_runtime_exact, KnownAcpRuntime},
    env_vars::merged_user_env,
    global_config::GlobalAgentConfig,
    normalize_agent_args,
    types::{AgentDefinition, ManagedAgentRecord},
};

/// The resolved environment that a spawn of `record` would actually receive.
///
/// Assembled from: baked build defaults (floor) → runtime metadata env vars
/// → merged user env_vars (last-wins) → reserved-key filtered.
///
/// `config_file_path` is the harness config file path (if any) — not part of
/// the process env but relevant for display and future write-back dispatch.
/// `effective_command` is the resolved harness binary name (e.g. `"buzz-agent"`,
/// `"goose"`) after persona and override resolution.
#[derive(Debug, Clone)]
pub(crate) struct EffectiveAgentEnv {
    /// The process-env map the spawned harness would receive.
    pub env: BTreeMap<String, String>,
    /// Effective local catalog identity, when one exists.
    pub runtime_id: Option<String>,
    /// Effective launch arguments used by cached runtime-readiness lookup.
    pub effective_args: Vec<String>,
    /// Harness config file path, if any (e.g. `~/.config/goose/config.yaml`).
    // Not read yet; kept for the unified-agent-record rewrite (chunk A) which
    // replaces this resolution path wholesale.
    #[allow(dead_code)]
    pub config_file_path: Option<&'static str>,
    /// The resolved harness binary name (e.g. `"buzz-agent"`, `"goose"`).
    pub effective_command: String,
}

// ── Typed effective-harness descriptor ───────────────────────────────────────
//
// A single owned type that fully describes what a spawn would run.  Produced
// by `resolve_effective_harness_descriptor` and consumed by spawn_agent_child,
// spawn_snapshot, build_managed_agent_summary, get_agent_models, and
// agent_readiness — so the harness-definition lookup and arg/env resolution
// happen exactly once, in one place.

/// The complete effective description of a harness spawn: resolved command,
/// args, and layered env.  This is the single source of truth for what will
/// actually run — computed once and shared across every consumer that needs
/// the effective values.
#[derive(Debug, Clone)]
pub(crate) struct EffectiveHarnessDescriptor {
    /// Effective local catalog identity. `None` is reserved for legacy/raw
    /// command records that have no catalog-backed launch identity.
    pub runtime_id: Option<String>,
    /// The raw effective command string (e.g. `"buzz-agent"`, `"my-acp-agent"`).
    /// Used for `known_acp_runtime` lookup and hashing.
    pub command: String,
    /// Normalized effective args.  Instance args win when non-empty; otherwise
    /// the harness definition's args apply.
    pub args: Vec<String>,
    /// The full layered process env: baked floor → runtime metadata → definition
    /// env → global → persona → agent.
    pub env: BTreeMap<String, String>,
}

impl EffectiveHarnessDescriptor {
    pub(crate) fn known_runtime(&self) -> Option<&'static KnownAcpRuntime> {
        self.runtime_id
            .as_deref()
            .and_then(known_acp_runtime_exact)
            .or_else(|| known_acp_runtime(&self.command))
    }
}

/// Resolve the complete harness descriptor from a record + context — the single
/// authoritative path for command, args, and env.
///
/// This is the only place where harness-definition lookup and arg/env layering
/// happen; spawn, hash, summary, and both model-probe paths all consume this.
///
/// Returns `Err("DANGLING_HARNESS_ID:<id>")` when the record (or its linked
/// persona) references a runtime id that no longer exists in the registry —
/// the same typed error produced by `try_record_agent_command`.  Callers that
/// cannot meaningfully continue with a dangling id (e.g. `spawn_agent_child`)
/// propagate the error; callers that degrade gracefully may use
/// `.unwrap_or_else(|_| …)`.
///
/// Does NOT require an `AppHandle` so it is fully unit-testable.
///
/// # Arguments
/// * `record` — the managed agent record
/// * `personas` — all current personas (for command/env resolution)
/// * `global` — global agent config defaults
pub(crate) fn resolve_effective_harness_descriptor(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> Result<EffectiveHarnessDescriptor, String> {
    let persona_runtime_id = record.persona_id.as_deref().and_then(|persona_id| {
        personas
            .iter()
            .find(|persona| persona.id == persona_id)
            .and_then(|persona| persona.runtime.as_deref())
    });
    let catalog_runtime_id = record
        .launch_runtime_id
        .clone()
        .or_else(|| {
            // Only old records lack provenance. New raw selections deliberately
            // leave launch_runtime_id unset and must never become catalog-backed
            // merely because their command happens to resemble one.
            (!record.raw_command_explicit)
                .then(|| {
                    record
                        .agent_command_override
                        .as_deref()
                        .and_then(crate::managed_agents::unique_catalog_runtime_id_for_command)
                })
                .flatten()
        })
        .or_else(|| {
            record
                .agent_command_override
                .is_none()
                .then(|| record.runtime.as_deref().or(persona_runtime_id))
                .flatten()
                .map(str::to_string)
        });

    let launch_spec = catalog_runtime_id
        .as_deref()
        .map(crate::managed_agents::resolve_catalog_harness_by_id)
        .transpose()?;
    let runtime_id = launch_spec.as_ref().map(|spec| spec.runtime_id.clone());
    let effective_command = launch_spec
        .as_ref()
        .map(|spec| spec.command.clone())
        .unwrap_or(crate::managed_agents::try_record_agent_command(
            record, personas,
        )?);
    let runtime_meta = runtime_id
        .as_deref()
        .and_then(known_acp_runtime_exact)
        .or_else(|| known_acp_runtime(&effective_command));
    let harness_def = launch_spec
        .as_ref()
        .and_then(|spec| spec.definition.clone());

    let record_args = record.agent_args.clone();
    let instance_has_args = record_args.iter().any(|arg| !arg.trim().is_empty());
    let args = if record.raw_command_explicit {
        record_args
    } else if instance_has_args {
        normalize_agent_args(&effective_command, record_args)
    } else if let Some(spec) = launch_spec.as_ref() {
        normalize_agent_args(&effective_command, spec.default_args.clone())
    } else {
        normalize_agent_args(&effective_command, record_args)
    };

    let effective_env = resolve_effective_agent_env_with_def(
        record,
        personas,
        runtime_meta,
        global,
        harness_def,
        &effective_command,
    );

    Ok(EffectiveHarnessDescriptor {
        runtime_id,
        command: effective_command,
        args,
        env: effective_env.env,
    })
}

/// Assemble the effective agent env from a record, personas, optional
/// known-runtime metadata, and the global agent config defaults — without an
/// `AppHandle` so it is fully unit-testable.
///
/// # Arguments
/// * `record` — the managed agent record (model/provider/env_vars/…)
/// * `personas` — all current persona records (for persona-backed resolution)
/// * `runtime` — the `KnownAcpRuntime` for the effective command, if any
/// * `global` — global agent config defaults (lowest user layer; pass
///   `&GlobalAgentConfig::default()` in tests that don't need global config)
pub(crate) fn resolve_effective_agent_env(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    runtime: Option<&KnownAcpRuntime>,
    global: &GlobalAgentConfig,
) -> EffectiveAgentEnv {
    if let Ok(descriptor) = resolve_effective_harness_descriptor(record, personas, global) {
        let config_file_path = descriptor
            .known_runtime()
            .and_then(|metadata| metadata.config_file_path);
        return EffectiveAgentEnv {
            runtime_id: descriptor.runtime_id,
            effective_args: descriptor.args,
            env: descriptor.env,
            config_file_path,
            effective_command: descriptor.command,
        };
    }

    let effective_command = crate::managed_agents::record_agent_command(record, personas);
    let harness_def = record
        .runtime
        .as_deref()
        .and_then(crate::managed_agents::custom_harnesses::lookup_loaded_harness_by_id);
    resolve_effective_agent_env_with_def(
        record,
        personas,
        runtime,
        global,
        harness_def,
        &effective_command,
    )
}

/// Inner implementation that accepts a pre-fetched `harness_def` to avoid a
/// second registry lookup when the caller (e.g. `resolve_effective_harness_descriptor`)
/// already has the definition in hand.
fn resolve_effective_agent_env_with_def(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    runtime: Option<&KnownAcpRuntime>,
    global: &GlobalAgentConfig,
    harness_def: Option<std::sync::Arc<crate::managed_agents::custom_harnesses::HarnessDefinition>>,
    effective_command: &str,
) -> EffectiveAgentEnv {
    let effective_command = effective_command.to_string();

    // Layer 1: baked build defaults (floor — internal builds only; OSS = empty).
    let mut env = baked_build_env();

    let (effective_model, effective_provider) =
        crate::managed_agents::global_config::resolve_effective_model_provider(
            record, personas, global,
        );

    if let Some(rt) = runtime {
        for (key, value) in crate::managed_agents::runtime::runtime_metadata_env_vars(
            rt.model_env_var,
            rt.provider_env_var,
            rt.provider_locked,
            effective_model.as_deref(),
            effective_provider.as_deref(),
        ) {
            env.insert(key.to_string(), value.to_string());
        }

        for (key, value) in rt.default_env {
            if !crate::managed_agents::env_vars::is_reserved_env_key(key) {
                env.insert((*key).to_string(), (*value).to_string());
            }
        }
    }

    // Layer 2b: definition env — the harness author's defaults (e.g. CURSOR_ACP=1).
    // Applied as a floor below global so user env always wins on collision.
    // Reserved keys are stripped by the shared `is_reserved_env_key` predicate.
    if let Some(definition) = &harness_def {
        for (key, value) in &definition.env {
            if !crate::managed_agents::env_vars::is_reserved_env_key(key) {
                env.insert(key.clone(), value.clone());
            }
        }
    }

    // Layer 3a: global env vars — the lowest user-settable layer.
    // Injected before persona/agent so per-agent values win on collision.
    // `merged_user_env` with an empty "lower" map applies reserved/malformed-key
    // filtering to the global map for free.
    let mut global_env = merged_user_env(&BTreeMap::new(), &global.env_vars);
    // Defense in depth for configs persisted by older builds: remote computer
    // connection values are agent-scoped and must never fan out to every child.
    global_env
        .retain(|key, _| !crate::managed_agents::global_config::is_agent_scoped_setup_key(key));
    env.extend(global_env);

    // Layer 3b: merged user env — live persona env under the record's own
    // overrides (last-wins), after reserved/malformed-key filtering. Reading
    // the persona live is what makes persona credential edits refresh on the
    // next spawn instead of being frozen into the record.
    let user_env = merged_user_env(
        &crate::managed_agents::env_vars::live_persona_env(personas, record.persona_id.as_deref()),
        &record.env_vars,
    );
    // Setup secrets are never an ambient env tier. Remove any baked/custom/
    // legacy copy, then inject a hydrated keyring value only when the
    // effective runtime's own catalog entry declares that secret field. This
    // prevents a stored manual-agent token from crossing into another runtime
    // after a harness switch.
    crate::managed_agents::setup_secrets::strip_setup_secret_env_values(&mut env);
    let allowed_setup_secrets: std::collections::BTreeSet<String> = runtime
        .into_iter()
        .flat_map(|runtime| {
            crate::managed_agents::setup_secrets::runtime_setup_secret_fields(runtime.id)
        })
        .map(|field| field.env_key)
        .collect();
    for (key, value) in user_env {
        if !crate::managed_agents::setup_secrets::is_setup_secret_env_key(&key)
            || allowed_setup_secrets.contains(&key)
        {
            env.insert(key, value);
        }
    }

    // Pkzz shared compute is a native Pkzz provider. Translate it to buzz-agent's
    // OpenAI-compatible transport only in the effective runtime environment.
    #[cfg(feature = "mesh-llm")]
    crate::managed_agents::apply_relay_mesh_env(
        &mut env,
        effective_provider.as_deref(),
        effective_model.as_deref(),
    );

    EffectiveAgentEnv {
        runtime_id: runtime.map(|metadata| metadata.id.to_string()),
        effective_args: normalize_agent_args(&effective_command, record.agent_args.clone()),
        env,
        config_file_path: runtime.and_then(|runtime| runtime.config_file_path),
        effective_command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::{
        apply_harness_update, resolve_create_harness_selection, AgentDefinition,
    };

    fn record() -> ManagedAgentRecord {
        AgentDefinition {
            id: "persona".to_string(),
            display_name: "Persona".to_string(),
            avatar_url: None,
            system_prompt: String::new(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
        .into_agent_record()
    }

    #[test]
    fn legacy_unique_command_recovers_catalog_launch_defaults() {
        let mut record = record();
        record.agent_command = "/opt/tools/ompk".to_string();
        record.agent_command_override = Some("/opt/tools/ompk".to_string());

        let descriptor =
            resolve_effective_harness_descriptor(&record, &[], &GlobalAgentConfig::default())
                .expect("legacy command should recover its unique catalog identity");

        assert_eq!(descriptor.runtime_id.as_deref(), Some("ompk"));
        assert_eq!(descriptor.command, "ompk");
        assert_eq!(descriptor.args, ["acp"]);
    }

    #[test]
    fn setup_secret_reaches_only_the_catalog_runtime_that_declares_it() {
        let mut record = record();
        record.env_vars.insert(
            "MANUAL_AGENT_TOKEN".to_string(),
            "test-app-password-0123456789-abcdef".to_string(),
        );

        let remote =
            crate::managed_agents::known_acp_runtime_exact("remote-agent-computer").unwrap();
        let remote_env = resolve_effective_agent_env_with_def(
            &record,
            &[],
            Some(remote),
            &GlobalAgentConfig::default(),
            None,
            "buzz-manual-agent-acp",
        );
        assert_eq!(
            remote_env.env.get("MANUAL_AGENT_TOKEN").map(String::as_str),
            Some("test-app-password-0123456789-abcdef")
        );

        let ordinary = crate::managed_agents::known_acp_runtime_exact("buzz-agent").unwrap();
        let ordinary_env = resolve_effective_agent_env_with_def(
            &record,
            &[],
            Some(ordinary),
            &GlobalAgentConfig::default(),
            None,
            "buzz-agent",
        );
        assert!(!ordinary_env.env.contains_key("MANUAL_AGENT_TOKEN"));
    }

    #[test]
    fn explicit_raw_command_never_selects_catalog_by_path() {
        let mut record = record();
        apply_harness_update(&mut record, &[], Some(None), Some("/opt/tools/ompk"), true)
            .expect("explicit raw command should persist");

        assert!(record.launch_runtime_id.is_none());
        assert!(record.raw_command_explicit);

        let descriptor =
            resolve_effective_harness_descriptor(&record, &[], &GlobalAgentConfig::default())
                .expect("raw command should resolve without a catalog launch spec");

        assert_eq!(descriptor.runtime_id, None);
        assert_eq!(descriptor.command, "/opt/tools/ompk");
        assert!(descriptor.args.is_empty());
    }

    #[test]
    fn explicit_raw_create_keeps_command_and_empty_args() {
        let selection = resolve_create_harness_selection(
            None,
            &[],
            Some(None),
            Some("/opt/tools/ompk"),
            &[],
            false,
        )
        .expect("explicit raw create should resolve");

        assert!(selection.launch_runtime_id.is_none());
        assert!(selection.raw_command_explicit);
        assert_eq!(selection.command, "/opt/tools/ompk");
        assert_eq!(
            selection.command_override.as_deref(),
            Some("/opt/tools/ompk")
        );
        assert!(selection.args.is_empty());
    }

    #[test]
    fn remote_connection_values_never_fan_out_from_global_env() {
        let record = record();
        let global = GlobalAgentConfig {
            env_vars: BTreeMap::from([
                (
                    "MANUAL_AGENT_BASE_URL".to_string(),
                    "https://device.example-tailnet.ts.net:8787".to_string(),
                ),
                (
                    "MANUAL_AGENT_TOKEN".to_string(),
                    "do-not-inherit-this-app-password".to_string(),
                ),
                ("SAFE_GLOBAL".to_string(), "kept".to_string()),
            ]),
            ..GlobalAgentConfig::default()
        };

        let effective =
            resolve_effective_agent_env_with_def(&record, &[], None, &global, None, "custom-acp");
        assert!(!effective.env.contains_key("MANUAL_AGENT_BASE_URL"));
        assert!(!effective.env.contains_key("MANUAL_AGENT_TOKEN"));
        assert_eq!(
            effective.env.get("SAFE_GLOBAL").map(String::as_str),
            Some("kept")
        );
    }

    #[test]
    fn remote_connection_values_remain_available_at_agent_scope() {
        let mut record = record();
        record.env_vars.insert(
            "MANUAL_AGENT_BASE_URL".to_string(),
            "https://device.example-tailnet.ts.net:8787".to_string(),
        );
        record.env_vars.insert(
            "MANUAL_AGENT_TOKEN".to_string(),
            "agent-scoped-app-password-0123456789".to_string(),
        );

        let remote =
            crate::managed_agents::known_acp_runtime_exact("remote-agent-computer").unwrap();
        let effective = resolve_effective_agent_env_with_def(
            &record,
            &[],
            Some(remote),
            &GlobalAgentConfig::default(),
            None,
            "buzz-manual-agent-acp",
        );
        assert_eq!(
            effective.env.get("MANUAL_AGENT_TOKEN").map(String::as_str),
            Some("agent-scoped-app-password-0123456789")
        );
    }
}
