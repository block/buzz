//! Agent readiness evaluation.
//!
//! # Overview
//!
//! Before spawning a managed agent (or before deciding whether to enter
//! setup-mode nudge), the desktop must know whether the agent has every
//! piece of configuration it will need to start successfully. This module
//! provides:
//!
//! * [`EffectiveAgentEnv`] — the resolved environment a spawn would actually
//!   see: baked build defaults (floor) → runtime metadata env vars → merged
//!   user env_vars (last-wins) → reserved-key filtered.  A separate
//!   `config_file` tier tracks fields the harness reads from its config file
//!   rather than the process env.
//! * [`resolve_effective_agent_env`] — assembles an `EffectiveAgentEnv` from
//!   a record + personas + runtime catalog; no `AppHandle` dependency so it
//!   is fully unit-testable.
//! * [`Requirement`] / [`RequirementSurface`] — structured predicates that
//!   carry enough surface-discrimination for the UI to route each gap to the
//!   right affordance (dropdown field vs env-var row vs CLI login step).
//! * [`AgentReadiness`] / [`agent_readiness`] — evaluates the effective env
//!   against the requirements for the resolved runtime and returns `Ready` or
//!   `NotReady(Vec<Requirement>)`.
//!
//! ## Env-assembly precedence (mirrors `spawn_agent_child`)
//!
//! 1. Baked build defaults (`baked_build_env()`) — injected first so the
//!    layers above can override them.
//! 2. Runtime metadata env vars (`runtime_metadata_env_vars`) — provider /
//!    model env keys derived from the record's `model`/`provider` fields and
//!    the runtime's `model_env_var`/`provider_env_var`.
//! 3. Merged user env (`merged_user_env`) — live persona env under the
//!    record's `env_vars` overrides, after reserved-key and malformed-key
//!    filtering.  Last-wins on collision.
//!
//! The config-file tier (Goose `~/.config/goose/config.yaml`) is tracked
//! separately because it is not part of the process env — the harness reads
//! it at startup.  We do not evaluate it here; it is exposed for future
//! UI display only.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::managed_agents::{
    agent_env::baked_build_env,
    config_bridge::read_goose_file_config,
    discovery::{known_acp_runtime, KnownAcpRuntime},
    env_vars::merged_user_env,
    global_config::GlobalAgentConfig,
    normalize_agent_args,
    types::{AcpAvailabilityStatus, AgentDefinition, ManagedAgentRecord},
};

mod cli_login;
pub(crate) mod cli_probe;

// ── EffectiveAgentEnv ─────────────────────────────────────────────────────────

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
    personas: &[crate::managed_agents::types::AgentDefinition],
    global: &crate::managed_agents::GlobalAgentConfig,
) -> Result<EffectiveHarnessDescriptor, String> {
    let effective_command = crate::managed_agents::try_record_agent_command(record, personas)?;
    let runtime_meta = known_acp_runtime(&effective_command);

    // Look up the harness definition once — used for both args and env.
    // Resolution order: record.runtime → persona.runtime → "".
    let harness_def = {
        let runtime_id = record
            .runtime
            .as_deref()
            .or_else(|| {
                record.persona_id.as_deref().and_then(|pid| {
                    personas
                        .iter()
                        .find(|p| p.id == pid)
                        .and_then(|p| p.runtime.as_deref())
                })
            })
            .unwrap_or("");
        crate::managed_agents::custom_harnesses::lookup_loaded_harness_by_id(runtime_id)
    };

    // Args: explicit non-empty instance args win; otherwise use definition args.
    let args = {
        let record_args = record.agent_args.clone();
        let instance_has_args = record_args.iter().any(|a| !a.trim().is_empty());
        if instance_has_args {
            normalize_agent_args(&effective_command, record_args)
        } else if let Some(ref def) = harness_def {
            normalize_agent_args(&effective_command, def.args.clone())
        } else {
            normalize_agent_args(&effective_command, record_args)
        }
    };

    // Env: full layered resolution (same as resolve_effective_agent_env).
    // Pass harness_def directly to avoid a second lookup.
    let effective_env =
        resolve_effective_agent_env_with_def(record, personas, runtime_meta, global, harness_def);

    Ok(EffectiveHarnessDescriptor {
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
    // Look up the harness definition for definition-level env (preset/custom).
    // Same resolution logic as spawn_agent_child: record runtime id first, then
    // persona runtime id, then nothing.
    let harness_def = {
        let runtime_id = record
            .runtime
            .as_deref()
            .or_else(|| {
                record.persona_id.as_deref().and_then(|pid| {
                    personas
                        .iter()
                        .find(|p| p.id == pid)
                        .and_then(|p| p.runtime.as_deref())
                })
            })
            .unwrap_or("");
        crate::managed_agents::custom_harnesses::lookup_loaded_harness_by_id(runtime_id)
    };

    resolve_effective_agent_env_with_def(record, personas, runtime, global, harness_def)
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
) -> EffectiveAgentEnv {
    let effective_command = crate::managed_agents::record_agent_command(record, personas);

    // Layer 1: baked build defaults (floor — internal builds only; OSS = empty).
    let mut env = baked_build_env();

    let (effective_model, effective_provider) =
        super::global_config::resolve_effective_model_provider(record, personas, global);

    if let Some(rt) = runtime {
        for (key, value) in super::runtime::runtime_metadata_env_vars(
            rt.model_env_var,
            rt.provider_env_var,
            rt.provider_locked,
            effective_model.as_deref(),
            effective_provider.as_deref(),
        ) {
            env.insert(key.to_string(), value.to_string());
        }
    }

    // Layer 2b: definition env — the harness author's defaults (e.g. CURSOR_ACP=1).
    // Applied as a floor below global so user env always wins on collision.
    // Reserved keys are stripped by the shared `is_reserved_env_key` predicate.
    if let Some(ref def) = harness_def {
        for (key, value) in &def.env {
            if !super::env_vars::is_reserved_env_key(key) {
                env.insert(key.clone(), value.clone());
            }
        }
    }

    // Layer 3a: global env vars — the lowest user-settable layer.
    // Injected before persona/agent so per-agent values win on collision.
    // `merged_user_env` with an empty "lower" map applies reserved/malformed-key
    // filtering to the global map for free.
    let global_env = merged_user_env(&BTreeMap::new(), &global.env_vars);
    env.extend(global_env);

    // Layer 3b: merged user env — live persona env under the record's own
    // overrides (last-wins), after reserved/malformed-key filtering. Reading
    // the persona live is what makes persona credential edits refresh on the
    // next spawn instead of being frozen into the record.
    let user_env = merged_user_env(
        &super::env_vars::live_persona_env(personas, record.persona_id.as_deref()),
        &record.env_vars,
    );
    env.extend(user_env);

    // Buzz shared compute is a native Buzz provider. Translate it to buzz-agent's
    // OpenAI-compatible transport only in the effective runtime environment.
    #[cfg(feature = "mesh-llm")]
    super::apply_relay_mesh_env(
        &mut env,
        effective_provider.as_deref(),
        effective_model.as_deref(),
    );

    // The user's own self-hosted OpenAI-compatible endpoint (llama.cpp, vLLM,
    // a directly-configured mesh-llm instance, etc. — distinct from the
    // built-in `relay-mesh` preset above) shares relay-mesh's failure mode:
    // small local models often finish a turn by writing the answer as prose
    // instead of calling `send_message`, and unlike cloud providers there is
    // no reliable way to make them self-correct. Opt this provider into the
    // same harness delivery fallback, unless a lower layer already set an
    // explicit value. An empty string counts as unset, matching the emptiness
    // convention used throughout this module (e.g. `buzz_agent_requirements`).
    // Scoped to `openai-compat` only: plain `openai` is the real OpenAI cloud
    // API, which — like other cloud providers — reliably calls tools, so
    // enabling the fallback there would risk double-posting.
    if effective_provider.as_deref().map(str::trim) == Some("openai-compat")
        && env
            .get("BUZZ_ACP_DELIVER_PLAIN_REPLIES")
            .is_none_or(|v| v.trim().is_empty())
    {
        env.insert(
            "BUZZ_ACP_DELIVER_PLAIN_REPLIES".to_string(),
            "true".to_string(),
        );
    }

    EffectiveAgentEnv {
        env,
        config_file_path: runtime.and_then(|r| r.config_file_path),
        effective_command,
    }
}

// ── Requirement types ─────────────────────────────────────────────────────────

/// A single missing piece of configuration, tagged with the UI surface that
/// owns it so the UI can route each gap to the right affordance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "surface", rename_all = "snake_case")]
pub enum Requirement {
    /// A normalized dropdown field (provider or model) that is missing.
    /// Routes to the provider/model dropdown in the Edit Agent dialog.
    NormalizedField {
        /// Camel-case field name matching `NormalizedConfig` ("provider", "model").
        field: String,
    },
    /// An env-backed credential that is absent from the effective env.
    /// Routes to the env-var row editor in the Edit Agent dialog.
    EnvKey {
        /// The env var key name (e.g. `"ANTHROPIC_API_KEY"`).
        key: String,
    },
    /// A CLI authentication step that must be completed interactively.
    /// Routes to a setup instruction panel in the Edit Agent dialog.
    CliLogin {
        /// Arguments for the login-status probe (e.g. `["claude", "auth", "status"]`).
        probe_args: Vec<String>,
        /// Human-readable instruction for completing the login
        /// (e.g. `"run \`codex login\`"`).
        setup_copy: String,
        /// Granular install/auth state for this runtime — distinguishes
        /// "not installed" from "logged out" from "adapter missing".
        /// Carried to the FE so the nudge card can show the right message
        /// and route to Doctor with accurate context.
        availability: AcpAvailabilityStatus,
    },
    /// The CLI is installed but its config file could not be parsed.
    /// This is an informational surface only — there is no in-app destination
    /// that can repair an external config file; the user must edit it manually.
    CliConfigInvalid {
        /// Arguments used in the probe (e.g. `["codex", "login", "status"]`);
        /// `probe_args[0]` is the CLI name (e.g. `"codex"`).
        probe_args: Vec<String>,
        /// Human-readable hint shown when no structured copy is available.
        setup_copy: String,
        /// A one-line excerpt from the CLI's stderr (the parse-error line).
        /// Shown verbatim in the nudge so the user can identify the problem.
        diagnostic: String,
    },
    /// Git for Windows is missing, so buzz-agent cannot launch buzz-dev-mcp's
    /// Bash-based shell tool. Doctor owns installation and re-checking.
    GitBash,
    /// A custom harness command that cannot be resolved in the current PATH.
    /// Displayed as a PATH badge in the harness card and a nudge in the agent
    /// message stream.  No in-app action can fix this — the user must install
    /// the binary or update their PATH.
    MissingBinary {
        /// The command name that was not found (e.g. `\"my-acp-agent\"`).
        command: String,
    },
}

// ── AgentReadiness ────────────────────────────────────────────────────────────

/// Whether a managed agent has all required configuration to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AgentReadiness {
    /// All required configuration is present — safe to spawn normally.
    Ready,
    /// One or more requirements are missing.
    NotReady {
        /// Surface-discriminated list of what is missing.
        requirements: Vec<Requirement>,
    },
}

impl AgentReadiness {
    /// Returns `true` if the agent is ready to spawn.
    #[cfg(test)]
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self, AgentReadiness::Ready)
    }

    /// Returns the missing requirements, or an empty slice if ready.
    #[cfg(test)]
    pub(crate) fn requirements(&self) -> &[Requirement] {
        match self {
            AgentReadiness::Ready => &[],
            AgentReadiness::NotReady { requirements } => requirements,
        }
    }
}

// ── agent_readiness ───────────────────────────────────────────────────────────

/// Evaluate whether a managed agent has all required configuration to start.
///
/// Checks the `effective` env surface against the requirements for the
/// resolved runtime:
///
/// * **buzz-agent / goose**: provider + model are required (both must be
///   present in the effective env or as structured fields). Additionally,
///   provider-specific credentials are required:
///   - `anthropic` → `ANTHROPIC_API_KEY`
///   - `openai` → `OPENAI_COMPAT_API_KEY`
///   - `databricks` / `databricks_v2` → `DATABRICKS_HOST` (token optional —
///     OAuth PKCE is the fallback)
/// * **claude**: a successful `claude auth status` probe.
/// * **codex**: a successful `codex login status` probe (checks the codex
///   credential store — NOT `OPENAI_API_KEY`).
/// * **unknown / custom command**: always `Ready` (no requirements known).
///
/// Databricks note: `DATABRICKS_TOKEN` is `.unwrap_or_default()` in
/// `buzz-agent/src/config.rs:143` — it is an escape hatch for static tokens
/// but the normal path is OAuth PKCE.  We intentionally do NOT mark the
/// token as required to avoid a false NotReady for users on OAuth.
pub(crate) fn agent_readiness(effective: &EffectiveAgentEnv) -> AgentReadiness {
    let runtime = known_acp_runtime(&effective.effective_command);
    let missing = collect_missing_requirements(effective, runtime);
    if missing.is_empty() {
        AgentReadiness::Ready
    } else {
        AgentReadiness::NotReady {
            requirements: missing,
        }
    }
}

/// Collect all missing requirements for the given effective env + runtime.
fn collect_missing_requirements(
    effective: &EffectiveAgentEnv,
    runtime: Option<&KnownAcpRuntime>,
) -> Vec<Requirement> {
    let Some(rt) = runtime else {
        // Unknown/custom command — check that the binary is actually resolvable.
        // No known requirement set beyond the PATH check.
        if crate::managed_agents::resolve_command(&effective.effective_command).is_none() {
            return vec![Requirement::MissingBinary {
                command: effective.effective_command.clone(),
            }];
        }
        return vec![];
    };

    match rt.id {
        "buzz-agent" => buzz_agent_requirements(effective),
        "goose" => {
            // Read the file config once at the call site so the inner fn is
            // pure and unit-testable by injection.
            let file_cfg = read_goose_file_config();
            goose_requirements(effective, file_cfg.as_ref())
        }
        "claude" => cli_login::requirements(
            &["claude", "auth", "status"],
            "complete Claude Code authentication by running the Claude CLI",
            rt,
        ),
        "codex" => cli_login::requirements(&["codex", "login", "status"], "run `codex login`", rt),
        _ => vec![],
    }
}

/// Requirements for buzz-agent (provider + model + provider-specific creds).
fn buzz_agent_requirements(effective: &EffectiveAgentEnv) -> Vec<Requirement> {
    let mut missing = Vec::new();

    #[cfg(windows)]
    if !crate::managed_agents::git_bash_available(&effective.env) {
        missing.push(Requirement::GitBash);
    }

    // Provider is required — maps to BUZZ_AGENT_PROVIDER in the effective env.
    // An empty string is treated as absent: a key set to "" is not a valid
    // provider and must not pass the readiness gate.
    let provider = effective
        .env
        .get("BUZZ_AGENT_PROVIDER")
        .filter(|v| !v.is_empty())
        .map(String::as_str);
    if provider.is_none() {
        missing.push(Requirement::NormalizedField {
            field: "provider".to_string(),
        });
    }

    // Model is required — maps to BUZZ_AGENT_MODEL in the effective env.
    // Same empty-string treatment as provider.
    // Also accept provider-specific model fallback keys, matching buzz-agent's
    // own config.rs `from_env()` resolution order (e.g. DATABRICKS_MODEL for
    // databricks/databricks_v2, ANTHROPIC_MODEL for anthropic, etc.). The
    // baked buzz-releases env sets DATABRICKS_MODEL but not BUZZ_AGENT_MODEL,
    // so without this fallback agents baked from releases appear "not ready".
    let provider_model_key = match provider {
        Some("databricks") | Some("databricks_v2") | Some("databricks-v2") => {
            Some("DATABRICKS_MODEL")
        }
        Some("anthropic") => Some("ANTHROPIC_MODEL"),
        Some("openai") | Some("openai-compat") => Some("OPENAI_COMPAT_MODEL"),
        Some("openrouter") => Some("OPENROUTER_MODEL"),
        _ => None,
    };
    let model_present = effective
        .env
        .get("BUZZ_AGENT_MODEL")
        .filter(|v| !v.is_empty())
        .is_some()
        || provider_model_key
            .and_then(|k| effective.env.get(k))
            .filter(|v| !v.is_empty())
            .is_some();
    if !model_present {
        missing.push(Requirement::NormalizedField {
            field: "model".to_string(),
        });
    }

    // Provider-specific credential requirements.
    // A key present with an empty value is treated as absent — matching the
    // dialog's (envVars[key] ?? "").length === 0 emptiness check.
    let env_key_missing = |key: &str| effective.env.get(key).is_none_or(|v| v.is_empty());
    match provider {
        Some("anthropic")
            if env_key_missing("ANTHROPIC_API_KEY") => {
                missing.push(Requirement::EnvKey {
                    key: "ANTHROPIC_API_KEY".to_string(),
                });
            }
        Some("openai")
            if env_key_missing("OPENAI_COMPAT_API_KEY") => {
                missing.push(Requirement::EnvKey {
                    key: "OPENAI_COMPAT_API_KEY".to_string(),
                });
            }
        Some("databricks") | Some("databricks_v2") | Some("databricks-v2")
            // DATABRICKS_HOST is hard-required; DATABRICKS_TOKEN is optional
            // (OAuth PKCE is the normal path — see buzz-agent/src/config.rs:143).
            if env_key_missing("DATABRICKS_HOST") => {
                missing.push(Requirement::EnvKey {
                    key: "DATABRICKS_HOST".to_string(),
                });
            }
        Some("openrouter")
            if env_key_missing("OPENROUTER_API_KEY") => {
                missing.push(Requirement::EnvKey {
                    key: "OPENROUTER_API_KEY".to_string(),
                });
            }
        _ => {
            // Unknown provider or no provider yet — only the NormalizedField
            // requirement above captures this gap.
        }
    }

    missing
}

/// Requirements for goose (provider + model + provider-specific creds).
///
/// Mirrors buzz-agent requirements but uses GOOSE_PROVIDER / GOOSE_MODEL.
///
/// File-config tier: goose reads `~/.config/goose/config.yaml` at startup.
/// Requirements already satisfied there are silenced — we don't need to
/// require them from Buzz's env layer.  The file layer only *silences*
/// requirements; it never injects values into the spawn env.
///
/// `file_cfg` is injected by the caller (read once at `collect_missing_requirements`)
/// so this function is pure and unit-testable without touching disk.
fn goose_requirements(
    effective: &EffectiveAgentEnv,
    file_cfg: Option<&crate::managed_agents::config_bridge::RuntimeFileConfig>,
) -> Vec<Requirement> {
    let mut missing = Vec::new();

    // Empty string treated as absent — same as buzz_agent_requirements.
    let provider = effective
        .env
        .get("GOOSE_PROVIDER")
        .filter(|v| !v.is_empty())
        .map(String::as_str);

    // Effective provider for credential checking: prefer env layer, then file.
    let effective_provider = provider.or_else(|| {
        file_cfg
            .as_ref()
            .and_then(|c| c.provider.as_deref())
            .filter(|v| !v.is_empty())
    });

    if provider.is_none() {
        // Silenced if the file config provides a provider.
        let file_provides_provider = file_cfg
            .as_ref()
            .and_then(|c| c.provider.as_deref())
            .filter(|v| !v.is_empty())
            .is_some();
        if !file_provides_provider {
            missing.push(Requirement::NormalizedField {
                field: "provider".to_string(),
            });
        }
    }

    let model = effective
        .env
        .get("GOOSE_MODEL")
        .filter(|v| !v.is_empty())
        .map(String::as_str);
    if model.is_none() {
        // Silenced if the file config provides a model.
        let file_provides_model = file_cfg
            .as_ref()
            .and_then(|c| c.model.as_deref())
            .filter(|v| !v.is_empty())
            .is_some();
        if !file_provides_model {
            missing.push(Requirement::NormalizedField {
                field: "model".to_string(),
            });
        }
    }

    // Provider-specific credentials — same empty-string semantics as buzz-agent.
    let env_key_missing = |key: &str| effective.env.get(key).is_none_or(|v| v.is_empty());
    // A credential key is also satisfied when the file config's `extra` map
    // contains it (e.g. DATABRICKS_HOST set in the goose config file).
    let file_key_present = |key: &str| -> bool {
        file_cfg
            .as_ref()
            .map(|c| c.extra.get(key).is_some_and(|v| !v.is_empty()))
            .unwrap_or(false)
    };
    match effective_provider {
        Some("anthropic")
            if env_key_missing("ANTHROPIC_API_KEY") && !file_key_present("ANTHROPIC_API_KEY") =>
        {
            missing.push(Requirement::EnvKey {
                key: "ANTHROPIC_API_KEY".to_string(),
            });
        }
        Some("openai")
            if env_key_missing("OPENAI_COMPAT_API_KEY")
                && !file_key_present("OPENAI_COMPAT_API_KEY") =>
        {
            missing.push(Requirement::EnvKey {
                key: "OPENAI_COMPAT_API_KEY".to_string(),
            });
        }
        Some("databricks") | Some("databricks_v2") | Some("databricks-v2")
            if env_key_missing("DATABRICKS_HOST") && !file_key_present("DATABRICKS_HOST") =>
        {
            missing.push(Requirement::EnvKey {
                key: "DATABRICKS_HOST".to_string(),
            });
        }
        Some("openrouter")
            if env_key_missing("OPENROUTER_API_KEY") && !file_key_present("OPENROUTER_API_KEY") =>
        {
            missing.push(Requirement::EnvKey {
                key: "OPENROUTER_API_KEY".to_string(),
            });
        }
        _ => {}
    }

    missing
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// The bulk of readiness's test coverage lives in a sibling file so this
// module stays under the desktop file-size ratchet.
#[cfg(test)]
#[path = "readiness_tests.rs"]
mod tests;

// Provider/model-fallback and CLI-adjacent requirement tests continue in a
// second sibling file — the split above still landed over the new-file cap.
#[cfg(test)]
#[path = "readiness_provider_tests.rs"]
mod provider_tests;

// Goose file-config-aware requirement tests live in a sibling file so this
// module stays under the desktop file-size ratchet.
#[cfg(test)]
#[path = "readiness_goose_file_config_tests.rs"]
mod goose_file_config_tests;
