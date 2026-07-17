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
    let effective_command = crate::managed_agents::record_agent_command(record, personas);

    // Layer 1: baked build defaults (floor — internal builds only; OSS = empty).
    let mut env = baked_build_env();

    // Layer 2: runtime metadata env vars (model / provider keys derived from
    // the record's structured fields, with global as fallback).
    //
    // Uses the shared resolver to guarantee readiness and spawn agree on the
    // effective model/provider: agent → persona → global → None.
    let (effective_model, effective_provider) =
        super::global_config::resolve_effective_model_provider(record, personas, global);

    if let Some(rt) = runtime {
        for (key, value) in super::runtime::runtime_metadata_env_vars(
            rt.model_env_var,
            rt.provider_env_var,
            rt.provider_locked,
            effective_model,
            effective_provider,
        ) {
            env.insert(key.to_string(), value.to_string());
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

    // Buzz shared compute is a native Buzz provider. Translate it to the
    // OpenAI-compatible transport used by the selected agent runtime.
    #[cfg(feature = "mesh-llm")]
    super::apply_relay_mesh_env(&mut env, effective_provider, effective_model);

    if runtime.is_some_and(|candidate| candidate.id == "goose") {
        super::apply_goose_runtime_env(&mut env, effective_provider, effective_model);
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
        // Unknown/custom command — no requirements to check.
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
            if env_key_missing("OPENAI_API_KEY")
                && env_key_missing("OPENAI_COMPAT_API_KEY")
                && !file_key_present("OPENAI_API_KEY")
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
        _ => {}
    }

    missing
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::managed_agents::discovery::known_acp_runtime_exact;

    /// Build a minimal `EffectiveAgentEnv` with the given env map and command.
    fn make_env(command: &str, env: BTreeMap<String, String>) -> EffectiveAgentEnv {
        let runtime = known_acp_runtime_exact(command);
        EffectiveAgentEnv {
            env,
            config_file_path: runtime.and_then(|r| r.config_file_path),
            effective_command: command.to_string(),
        }
    }

    fn env_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // ── buzz-agent tests ──────────────────────────────────────────────────

    #[test]
    fn buzz_agent_missing_provider_returns_not_ready_with_normalized_field() {
        let env = make_env(
            "buzz-agent",
            env_with(&[("BUZZ_AGENT_MODEL", "claude-opus-4-5")]),
        );
        let result = agent_readiness(&env);
        assert!(
            !result.is_ready(),
            "missing BUZZ_AGENT_PROVIDER should be NotReady"
        );
        let reqs = result.requirements();
        assert!(
            reqs.contains(&Requirement::NormalizedField {
                field: "provider".to_string()
            }),
            "requirements should include NormalizedField(provider); got {reqs:?}"
        );
    }

    #[test]
    fn buzz_agent_missing_model_returns_not_ready_with_normalized_field() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
                ("ANTHROPIC_API_KEY", "sk-test"),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(!result.is_ready());
        assert!(result
            .requirements()
            .contains(&Requirement::NormalizedField {
                field: "model".to_string()
            }));
    }

    #[test]
    fn buzz_agent_missing_anthropic_key_returns_not_ready_with_env_key() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
                ("BUZZ_AGENT_MODEL", "claude-opus-4-5"),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(!result.is_ready());
        assert!(result.requirements().contains(&Requirement::EnvKey {
            key: "ANTHROPIC_API_KEY".to_string()
        }));
    }

    #[test]
    fn buzz_agent_missing_openai_key_returns_not_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "openai"),
                ("BUZZ_AGENT_MODEL", "gpt-4o"),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(!result.is_ready());
        assert!(result.requirements().contains(&Requirement::EnvKey {
            key: "OPENAI_COMPAT_API_KEY".to_string()
        }));
    }

    #[test]
    fn buzz_agent_anthropic_with_all_fields_is_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
                ("BUZZ_AGENT_MODEL", "claude-opus-4-5"),
                ("ANTHROPIC_API_KEY", "sk-test"),
            ]),
        );
        assert!(agent_readiness(&env).is_ready());
    }

    #[test]
    fn buzz_agent_databricks_with_host_and_model_is_ready_without_token() {
        // DATABRICKS_TOKEN is NOT required — OAuth PKCE is the normal path.
        // No token present, no OAuth cache present → still Ready because we
        // cannot evaluate OAuth state from the env map alone.
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks"),
                ("BUZZ_AGENT_MODEL", "dbrx-instruct"),
                ("DATABRICKS_HOST", "https://dbc.example.com"),
                // NOTE: no DATABRICKS_TOKEN
            ]),
        );
        assert!(
            agent_readiness(&env).is_ready(),
            "Databricks with HOST+model but no TOKEN should still be Ready (OAuth path)"
        );
    }

    #[test]
    fn buzz_agent_databricks_missing_host_returns_not_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks"),
                ("BUZZ_AGENT_MODEL", "dbrx-instruct"),
                // NOTE: no DATABRICKS_HOST
            ]),
        );
        let result = agent_readiness(&env);
        assert!(!result.is_ready());
        assert!(result.requirements().contains(&Requirement::EnvKey {
            key: "DATABRICKS_HOST".to_string()
        }));
    }

    #[test]
    fn buzz_agent_databricks_v2_missing_host_returns_not_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
                (
                    "BUZZ_AGENT_MODEL",
                    "databricks/meta-llama-4-maverick-17b-instruct",
                ),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(!result.is_ready());
        assert!(result.requirements().contains(&Requirement::EnvKey {
            key: "DATABRICKS_HOST".to_string()
        }));
    }

    // ── goose tests ───────────────────────────────────────────────────────

    #[test]
    fn goose_missing_provider_returns_not_ready() {
        // Call goose_requirements directly with None file config so the test is
        // deterministic — the `agent_readiness` path reads the real
        // ~/.config/goose/config.yaml which may silence requirements on
        // developer machines.
        let env = make_env("goose", env_with(&[("GOOSE_MODEL", "claude-opus-4-5")]));
        let reqs = goose_requirements(&env, None);
        assert!(
            !reqs.is_empty(),
            "missing GOOSE_PROVIDER with no file config must produce requirements"
        );
        assert!(
            reqs.contains(&Requirement::NormalizedField {
                field: "provider".to_string()
            }),
            "requirements must include NormalizedField(provider); got {reqs:?}"
        );
    }

    #[test]
    fn goose_with_provider_and_model_and_key_is_ready() {
        let env = make_env(
            "goose",
            env_with(&[
                ("GOOSE_PROVIDER", "anthropic"),
                ("GOOSE_MODEL", "claude-opus-4-5"),
                ("ANTHROPIC_API_KEY", "sk-test"),
            ]),
        );
        assert!(agent_readiness(&env).is_ready());
    }

    // ── empty-string semantics ────────────────────────────────────────────
    //
    // A key present with an empty value ("") must be treated as MISSING, to
    // match the dialog's (envVars[key] ?? "").length === 0 emptiness check.

    #[test]
    fn buzz_agent_empty_string_provider_is_not_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", ""),
                ("BUZZ_AGENT_MODEL", "claude-opus-4-5"),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(
            !result.is_ready(),
            "empty-string BUZZ_AGENT_PROVIDER must be treated as missing"
        );
        assert!(result
            .requirements()
            .contains(&Requirement::NormalizedField {
                field: "provider".to_string()
            }));
    }

    #[test]
    fn buzz_agent_empty_string_model_is_not_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
                ("BUZZ_AGENT_MODEL", ""),
                ("ANTHROPIC_API_KEY", "sk-test"),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(
            !result.is_ready(),
            "empty-string BUZZ_AGENT_MODEL must be treated as missing"
        );
        assert!(result
            .requirements()
            .contains(&Requirement::NormalizedField {
                field: "model".to_string()
            }));
    }

    #[test]
    fn buzz_agent_empty_string_anthropic_key_is_not_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
                ("BUZZ_AGENT_MODEL", "claude-opus-4-5"),
                ("ANTHROPIC_API_KEY", ""),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(
            !result.is_ready(),
            "empty-string ANTHROPIC_API_KEY must be treated as missing"
        );
        assert!(result.requirements().contains(&Requirement::EnvKey {
            key: "ANTHROPIC_API_KEY".to_string()
        }));
    }

    #[test]
    fn buzz_agent_empty_string_databricks_host_is_not_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks"),
                ("BUZZ_AGENT_MODEL", "dbrx-instruct"),
                ("DATABRICKS_HOST", ""),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(
            !result.is_ready(),
            "empty-string DATABRICKS_HOST must be treated as missing"
        );
        assert!(result.requirements().contains(&Requirement::EnvKey {
            key: "DATABRICKS_HOST".to_string()
        }));
    }

    #[test]
    fn goose_empty_string_provider_is_not_ready() {
        // Call goose_requirements directly with None file config so the test is
        // deterministic — the `agent_readiness` path reads the real
        // ~/.config/goose/config.yaml which may silence requirements on
        // developer machines.
        let env = make_env(
            "goose",
            env_with(&[("GOOSE_PROVIDER", ""), ("GOOSE_MODEL", "claude-opus-4-5")]),
        );
        let reqs = goose_requirements(&env, None);
        assert!(
            !reqs.is_empty(),
            "empty-string GOOSE_PROVIDER must be treated as missing"
        );
        assert!(
            reqs.contains(&Requirement::NormalizedField {
                field: "provider".to_string()
            }),
            "requirements must include NormalizedField(provider); got {reqs:?}"
        );
    }

    #[test]
    fn goose_empty_string_anthropic_key_is_not_ready() {
        // Call goose_requirements directly with None file config so the test is
        // deterministic — the `agent_readiness` path reads the real
        // ~/.config/goose/config.yaml which may silence requirements on
        // developer machines.
        let env = make_env(
            "goose",
            env_with(&[
                ("GOOSE_PROVIDER", "anthropic"),
                ("GOOSE_MODEL", "claude-opus-4-5"),
                ("ANTHROPIC_API_KEY", ""),
            ]),
        );
        let reqs = goose_requirements(&env, None);
        assert!(
            !reqs.is_empty(),
            "empty-string ANTHROPIC_API_KEY must be treated as missing (goose)"
        );
        assert!(
            reqs.contains(&Requirement::EnvKey {
                key: "ANTHROPIC_API_KEY".to_string()
            }),
            "requirements must include ANTHROPIC_API_KEY; got {reqs:?}"
        );
    }

    // ── codex tests ───────────────────────────────────────────────────────

    #[test]
    fn codex_not_ready_copy_does_not_mention_openai_api_key() {
        // codex uses its own credential store via `codex login` (OAuth or API key).
        // The nudge copy must NOT say "set OPENAI_API_KEY".
        // Use a not-installed runtime so the requirement is always emitted
        // regardless of whether codex is on the test machine's PATH.
        let rt = make_cli_runtime(&["__buzz_nonexistent_adapter_xyz789__"], None);
        let reqs = cli_login::requirements(&["codex", "login", "status"], "run `codex login`", &rt);
        // Whether codex is installed or not, the copy (if any) must not mention OPENAI_API_KEY.
        for req in &reqs {
            if let Requirement::CliLogin { setup_copy, .. } = req {
                assert!(
                    !setup_copy.contains("OPENAI_API_KEY"),
                    "codex nudge copy must not mention OPENAI_API_KEY; got: {setup_copy:?}"
                );
                assert!(
                    setup_copy.contains("codex login"),
                    "codex nudge copy should mention `codex login`; got: {setup_copy:?}"
                );
            }
        }
    }

    // ── cli_login_requirements: resolve_command integration ─────────────

    /// Construct a minimal `KnownAcpRuntime` stub for testing cli_login_requirements.
    /// `commands` are the adapter binaries; `underlying_cli` is the CLI name.
    fn make_cli_runtime(
        commands: &'static [&'static str],
        underlying_cli: Option<&'static str>,
    ) -> KnownAcpRuntime {
        KnownAcpRuntime {
            id: "test-cli-runtime",
            label: "Test CLI",
            commands,
            aliases: &[],
            avatar_url: "",
            mcp_command: None,
            mcp_hooks: false,
            underlying_cli,
            cli_install_commands: &[],
            cli_install_commands_windows: &[],
            adapter_install_commands: &[],
            install_instructions_url: "",
            cli_install_hint: "",
            adapter_install_hint: "",
            skill_dir: None,
            supports_acp_model_switching: false,
            config_file_path: None,
            config_file_format: None,
            model_env_var: None,
            provider_env_var: None,
            provider_locked: false,
            default_env: &[],
            supports_acp_native_config: false,
            thinking_env_var: None,
            max_tokens_env_var: None,
            context_limit_env_var: None,
            required_normalized_fields: &[],
            login_hint: None,
            auth_probe_args: None,
        }
    }

    /// Returns the absolute path of the currently-running test binary as a
    /// `&'static str`.  Host-portable stand-in for a "present" binary:
    /// the path is absolute so `find_command` resolves it via `path.exists()`
    /// rather than searching `PATH`, and the file always exists on the host.
    ///
    /// The tiny allocation is intentionally leaked — this runs at most once per
    /// test process and the process exits immediately after tests complete.
    fn present_binary_str() -> &'static str {
        let path = std::env::current_exe().expect("current_exe must be available in tests");
        Box::leak(path.to_string_lossy().into_owned().into_boxed_str())
    }

    /// Leak a runtime slice of `'static` strs for use in `make_cli_runtime`.
    fn static_commands(commands: Vec<&'static str>) -> &'static [&'static str] {
        Box::leak(commands.into_boxed_slice())
    }

    #[test]
    fn cli_login_requirements_missing_binary_is_not_ready() {
        // Both adapter and underlying CLI are nonexistent → NotInstalled state
        // → must return a CliLogin requirement with availability=NotInstalled.
        let rt = make_cli_runtime(
            &["__buzz_nonexistent_adapter_abc123__"],
            Some("__buzz_nonexistent_cli_abc123__"),
        );
        let reqs = cli_login::requirements(
            &["__buzz_nonexistent_binary_abc123__", "status"],
            "install the tool first",
            &rt,
        );
        assert!(
            !reqs.is_empty(),
            "missing binary must produce a CliLogin requirement (NotReady)"
        );
        assert!(
            matches!(reqs[0], Requirement::CliLogin { .. }),
            "requirement must be CliLogin; got {:?}",
            reqs[0]
        );
        if let Requirement::CliLogin {
            ref availability, ..
        } = reqs[0]
        {
            assert_eq!(
                *availability,
                crate::managed_agents::AcpAvailabilityStatus::NotInstalled,
                "both missing → NotInstalled"
            );
        }
    }

    #[test]
    fn cli_login_requirements_adapter_missing_emits_adapter_missing() {
        // Underlying CLI present (use the running test binary as a portable
        // stand-in — it's always present and resolves via absolute path),
        // adapter absent.
        // → AdapterMissing state → no probe run → CliLogin{AdapterMissing}.
        let exe = present_binary_str();
        let rt = make_cli_runtime(&["__buzz_nonexistent_adapter_xyz789__"], Some(exe));
        let reqs = cli_login::requirements(&[exe, "--list"], "install the adapter", &rt);
        assert!(
            !reqs.is_empty(),
            "adapter missing must produce a CliLogin requirement"
        );
        if let Requirement::CliLogin {
            ref availability, ..
        } = reqs[0]
        {
            assert_eq!(
                *availability,
                crate::managed_agents::AcpAvailabilityStatus::AdapterMissing,
                "adapter absent, CLI present → AdapterMissing"
            );
        }
    }

    #[test]
    fn cli_login_requirements_cli_missing_emits_cli_missing() {
        // Adapter present (use the running test binary as a portable stand-in),
        // underlying CLI absent.
        // → CliMissing state → no probe run → CliLogin{CliMissing}.
        let exe = present_binary_str();
        let rt = make_cli_runtime(
            static_commands(vec![exe]),              // adapter found via absolute path
            Some("__buzz_nonexistent_cli_abc123__"), // underlying CLI missing
        );
        let reqs = cli_login::requirements(&[exe, "--list"], "install the CLI", &rt);
        assert!(
            !reqs.is_empty(),
            "CLI missing must produce a CliLogin requirement"
        );
        if let Requirement::CliLogin {
            ref availability, ..
        } = reqs[0]
        {
            assert_eq!(
                *availability,
                crate::managed_agents::AcpAvailabilityStatus::CliMissing,
                "adapter present, CLI absent → CliMissing"
            );
        }
    }

    #[test]
    fn cli_login_requirements_resolvable_binary_runs_probe_at_resolved_path() {
        // Both adapter and CLI present (use the running test binary as a
        // portable stand-in — always present, resolves via absolute path),
        // probe exits 0 (run with `--list` which lists tests and exits 0).
        // → logged_in = true → requirements is empty (Ready).
        let exe = present_binary_str();
        let rt = make_cli_runtime(static_commands(vec![exe]), Some(exe));
        let reqs = cli_login::requirements(
            &[exe, "--list"],
            "this should not show (probe exits 0)",
            &rt,
        );
        assert!(
            reqs.is_empty(),
            "expected Ready (no requirements) when probe binary resolves and exits 0; \
             got {:?}",
            reqs
        );
    }

    #[test]
    fn cli_login_requirements_logged_out_emits_available() {
        // Both adapter and CLI present, but probe exits non-zero (logged out).
        // Use the test binary with an unrecognized argument as the probe —
        // libtest exits non-zero for unknown flags on all platforms.
        // → CliLogin{Available} (tooling installed, needs login).
        let exe = present_binary_str();
        let rt = make_cli_runtime(static_commands(vec![exe]), Some(exe));
        let reqs =
            cli_login::requirements(&[exe, "--buzz-probe-fail-xyz"], "run `tool login`", &rt);
        assert!(
            !reqs.is_empty(),
            "non-zero probe must produce a CliLogin requirement (logged out)"
        );
        if let Requirement::CliLogin {
            ref availability, ..
        } = reqs[0]
        {
            assert_eq!(
                *availability,
                crate::managed_agents::AcpAvailabilityStatus::Available,
                "tooling installed, probe fails → Available (logged-out)"
            );
        }
    }

    // ── codex readiness version gate ───────────────────────────────────────

    /// Build a minimal `KnownAcpRuntime` for testing the codex version gate.
    /// `adapter_commands` are the exact strings passed to `find_command` — use
    /// `&["codex-acp"]` when the binary is on PATH, or `&[<absolute_path>]`
    /// when resolving via absolute path.  `underlying_cli` is a portable
    /// stand-in so the adapter is not misclassified as `CliMissing`.
    fn make_codex_runtime(
        adapter_commands: &'static [&'static str],
        underlying_cli: Option<&'static str>,
    ) -> KnownAcpRuntime {
        KnownAcpRuntime {
            id: "codex",
            label: "Codex",
            commands: adapter_commands,
            aliases: &[],
            avatar_url: "",
            mcp_command: None,
            mcp_hooks: false,
            underlying_cli,
            cli_install_commands: &[],
            cli_install_commands_windows: &[],
            adapter_install_commands: &[],
            install_instructions_url: "",
            cli_install_hint: "",
            adapter_install_hint: "",
            skill_dir: None,
            supports_acp_model_switching: false,
            config_file_path: None,
            config_file_format: None,
            model_env_var: None,
            provider_env_var: None,
            provider_locked: false,
            default_env: &[],
            supports_acp_native_config: false,
            thinking_env_var: None,
            max_tokens_env_var: None,
            context_limit_env_var: None,
            required_normalized_fields: &[],
            login_hint: None,
            auth_probe_args: None,
        }
    }

    /// Build a temp dir containing a `codex-acp` script with the given body,
    /// prepend it to PATH, and clear the resolve cache.  Returns the temp dir
    /// and the original PATH string for restoration.
    #[cfg(unix)]
    fn setup_temp_codex_acp(script_body: &str) -> (tempfile::TempDir, String) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("create temp dir");
        let bin = dir.path().join("codex-acp");
        std::fs::write(&bin, script_body).expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let original_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", dir.path().display(), original_path);
        std::env::set_var("PATH", &new_path);
        crate::managed_agents::clear_resolve_cache();

        (dir, original_path)
    }

    #[cfg(unix)]
    fn leaked_adapter_commands(bin: &std::path::Path) -> &'static [&'static str] {
        let command = Box::leak(bin.display().to_string().into_boxed_str());
        Box::leak(vec![command as &'static str].into_boxed_slice())
    }

    /// Restore PATH and clear the resolve cache after a PATH-mutating test.
    #[cfg(unix)]
    fn restore_path(original: &str) {
        std::env::set_var("PATH", original);
        crate::managed_agents::clear_resolve_cache();
    }

    /// Codex readiness: outdated adapter (exits non-zero) → AdapterOutdated,
    /// login probe skipped.
    #[cfg(unix)]
    #[test]
    fn cli_login_requirements_codex_outdated_adapter_emits_adapter_outdated() {
        let _guard = crate::managed_agents::lock_path_mutex();

        let (dir, orig) = setup_temp_codex_acp("#!/bin/sh\nexit 1\n");
        let exe = present_binary_str();
        // Use the fixture's absolute adapter path here. Bare `codex-acp`
        // intentionally prefers Buzz's managed npm shim when it exists, which
        // would make this version-gate regression test depend on machine state.
        let rt = make_codex_runtime(
            leaked_adapter_commands(&dir.path().join("codex-acp")),
            Some(exe),
        );
        let reqs = cli_login::requirements(
            &[exe, "--buzz-probe-must-not-run-xyz"],
            "run `codex login`",
            &rt,
        );

        restore_path(&orig);
        drop(dir);

        assert!(
            !reqs.is_empty(),
            "outdated codex adapter must produce a requirement; got {reqs:?}"
        );
        if let Requirement::CliLogin {
            ref availability, ..
        } = reqs[0]
        {
            assert_eq!(
                *availability,
                crate::managed_agents::AcpAvailabilityStatus::AdapterOutdated,
                "0.x codex adapter must yield AdapterOutdated; got {availability:?}"
            );
        } else {
            panic!("expected CliLogin requirement; got {:?}", reqs[0]);
        }
    }

    /// Codex readiness: adapter exits 0 but output is not a parseable version
    /// → AdapterOutdated (garbage output treated as outdated, same as non-zero).
    #[cfg(unix)]
    #[test]
    fn cli_login_requirements_codex_garbage_version_output_emits_adapter_outdated() {
        let _guard = crate::managed_agents::lock_path_mutex();

        let (dir, orig) = setup_temp_codex_acp("#!/bin/sh\necho 'not a version string'\nexit 0\n");
        let exe = present_binary_str();
        let rt = make_codex_runtime(
            leaked_adapter_commands(&dir.path().join("codex-acp")),
            Some(exe),
        );
        let reqs = cli_login::requirements(
            &[exe, "--buzz-probe-must-not-run-xyz"],
            "run `codex login`",
            &rt,
        );

        restore_path(&orig);
        drop(dir);

        assert!(
            !reqs.is_empty(),
            "garbage version output must produce a requirement; got {reqs:?}"
        );
        if let Requirement::CliLogin {
            ref availability, ..
        } = reqs[0]
        {
            assert_eq!(
                *availability,
                crate::managed_agents::AcpAvailabilityStatus::AdapterOutdated,
                "unparseable version output must yield AdapterOutdated; got {availability:?}"
            );
        } else {
            panic!("expected CliLogin requirement; got {:?}", reqs[0]);
        }
    }

    // ── custom/unknown command ─────────────────────────────────────────────

    #[test]
    fn unknown_command_is_always_ready() {
        let env = make_env("my-custom-harness", BTreeMap::new());
        assert!(
            agent_readiness(&env).is_ready(),
            "unknown/custom command should always be Ready (no requirements)"
        );
    }

    // ── AgentReadiness helpers ─────────────────────────────────────────────

    #[test]
    fn agent_readiness_ready_has_empty_requirements() {
        assert!(AgentReadiness::Ready.requirements().is_empty());
    }

    #[test]
    fn agent_readiness_not_ready_exposes_requirements() {
        let r = AgentReadiness::NotReady {
            requirements: vec![Requirement::EnvKey {
                key: "FOO".to_string(),
            }],
        };
        assert!(!r.is_ready());
        assert_eq!(r.requirements().len(), 1);
    }

    // ── Requirement serialization ─────────────────────────────────────────

    #[test]
    fn requirement_serializes_with_surface_tag() {
        let r = Requirement::NormalizedField {
            field: "provider".to_string(),
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["surface"], "normalized_field");
        assert_eq!(json["field"], "provider");
    }

    #[test]
    fn git_bash_requirement_serializes_correctly() {
        let json = serde_json::to_value(Requirement::GitBash).unwrap();
        assert_eq!(json, serde_json::json!({ "surface": "git_bash" }));
    }

    #[test]
    fn env_key_requirement_serializes_correctly() {
        let r = Requirement::EnvKey {
            key: "ANTHROPIC_API_KEY".to_string(),
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["surface"], "env_key");
        assert_eq!(json["key"], "ANTHROPIC_API_KEY");
    }

    #[test]
    fn cli_login_requirement_serializes_correctly() {
        let r = Requirement::CliLogin {
            probe_args: vec![
                "codex".to_string(),
                "login".to_string(),
                "status".to_string(),
            ],
            setup_copy: "run `codex login`".to_string(),
            availability: crate::managed_agents::AcpAvailabilityStatus::Available,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["surface"], "cli_login");
        assert!(json["probe_args"].is_array());
        assert!(json["setup_copy"].as_str().unwrap().contains("codex login"));
    }

    // ── resolve_effective_agent_env ─────────────────────────────────────────

    #[test]
    fn resolve_effective_agent_env_user_env_wins_over_structured_fields() {
        // A record whose env_vars explicitly set provider/model must win over
        // any baked defaults. In OSS test builds the baked map is empty, so
        // this test validates the user-env layer is present in the output.
        let mut env_vars = BTreeMap::new();
        env_vars.insert("BUZZ_AGENT_PROVIDER".to_string(), "anthropic".to_string());
        env_vars.insert(
            "BUZZ_AGENT_MODEL".to_string(),
            "claude-opus-4-5".to_string(),
        );

        // Minimal record: only the fields resolve_effective_agent_env reads.
        let record = crate::managed_agents::types::ManagedAgentRecord {
            pubkey: "test-pubkey".to_string(),
            name: "test-agent".to_string(),
            persona_id: None,
            private_key_nsec: String::new(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: None,
            acp_command: "buzz-acp".to_string(),
            agent_command: "buzz-agent".to_string(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 320,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            env_vars,
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: Default::default(),
            backend_agent_id: None,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: Default::default(),
            respond_to_allowlist: vec![],
            display_name: None,
            slug: None,
            runtime: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: Vec::new(),
            definition_parallelism: None,
            relay_mesh: None,
        };

        let runtime = known_acp_runtime_exact("buzz-agent");
        let effective = resolve_effective_agent_env(&record, &[], runtime, &Default::default());

        // User env_vars must be present in the output (last-write-wins).
        assert_eq!(
            effective.env.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
            Some("anthropic")
        );
        assert_eq!(
            effective.env.get("BUZZ_AGENT_MODEL").map(String::as_str),
            Some("claude-opus-4-5")
        );
    }

    // ── provider-specific model fallback tests ────────────────────────────

    #[test]
    fn buzz_agent_databricks_v2_with_databricks_model_but_no_buzz_agent_model_is_ready() {
        // The baked buzz-releases env sets DATABRICKS_MODEL but not BUZZ_AGENT_MODEL.
        // An agent with only DATABRICKS_MODEL must pass the readiness gate.
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
                ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
                ("DATABRICKS_HOST", "https://dbc.example.com"),
            ]),
        );
        assert!(
            agent_readiness(&env).is_ready(),
            "DATABRICKS_MODEL must satisfy the model requirement for databricks_v2"
        );
    }

    #[test]
    fn buzz_agent_databricks_v2_hyphen_alias_with_databricks_model_is_ready() {
        // buzz-agent accepts both "databricks_v2" and "databricks-v2". The
        // readiness gate must recognize the hyphen alias and accept DATABRICKS_MODEL.
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks-v2"),
                ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
                ("DATABRICKS_HOST", "https://dbc.example.com"),
            ]),
        );
        assert!(
            agent_readiness(&env).is_ready(),
            "databricks-v2 alias with DATABRICKS_MODEL must be Ready"
        );
    }

    #[test]
    fn buzz_agent_databricks_hyphen_alias_missing_host_returns_not_ready() {
        // The hyphen alias "databricks-v2" requires DATABRICKS_HOST just like
        // the underscore variants. Without it the agent cannot reach the endpoint.
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks-v2"),
                ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
                // DATABRICKS_HOST intentionally absent
            ]),
        );
        let result = agent_readiness(&env);
        assert!(
            !result.is_ready(),
            "databricks-v2 without DATABRICKS_HOST must be NotReady"
        );
        let reqs = result.requirements();
        assert!(
            reqs.iter()
                .any(|r| matches!(r, Requirement::EnvKey { key } if key == "DATABRICKS_HOST")),
            "missing requirements must include DATABRICKS_HOST; got {reqs:?}"
        );
    }

    #[test]
    fn buzz_agent_databricks_v1_with_databricks_model_but_no_buzz_agent_model_is_ready() {
        // V1 (Model Serving) also resolves DATABRICKS_MODEL — same fallback applies.
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks"),
                ("DATABRICKS_MODEL", "dbrx-instruct"),
                ("DATABRICKS_HOST", "https://dbc.example.com"),
            ]),
        );
        assert!(
            agent_readiness(&env).is_ready(),
            "DATABRICKS_MODEL must satisfy the model requirement for databricks (V1)"
        );
    }

    #[test]
    fn buzz_agent_anthropic_with_anthropic_model_but_no_buzz_agent_model_is_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
                ("ANTHROPIC_MODEL", "claude-opus-4-5"),
                ("ANTHROPIC_API_KEY", "sk-test"),
            ]),
        );
        assert!(
            agent_readiness(&env).is_ready(),
            "ANTHROPIC_MODEL must satisfy the model requirement for anthropic"
        );
    }

    #[test]
    fn buzz_agent_openai_with_openai_compat_model_but_no_buzz_agent_model_is_ready() {
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "openai"),
                ("OPENAI_COMPAT_MODEL", "gpt-4o"),
                ("OPENAI_COMPAT_API_KEY", "sk-test"),
            ]),
        );
        assert!(
            agent_readiness(&env).is_ready(),
            "OPENAI_COMPAT_MODEL must satisfy the model requirement for openai"
        );
    }

    #[test]
    fn buzz_agent_empty_provider_model_fallback_key_is_not_ready() {
        // An empty DATABRICKS_MODEL with no BUZZ_AGENT_MODEL must still be NotReady.
        let env = make_env(
            "buzz-agent",
            env_with(&[
                ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
                ("DATABRICKS_MODEL", ""),
                ("DATABRICKS_HOST", "https://dbc.example.com"),
            ]),
        );
        let result = agent_readiness(&env);
        assert!(
            !result.is_ready(),
            "empty DATABRICKS_MODEL with no BUZZ_AGENT_MODEL must be NotReady"
        );
        assert!(result
            .requirements()
            .contains(&Requirement::NormalizedField {
                field: "model".to_string()
            }));
    }
}

// ── goose file-config–aware requirement tests ─────────────────────────────
//
// These tests call `goose_requirements` directly, injecting a synthetic
// `RuntimeFileConfig` so there is no disk I/O and tests are deterministic.

#[cfg(test)]
mod goose_file_config_tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::managed_agents::config_bridge::RuntimeFileConfig;

    fn empty_env() -> EffectiveAgentEnv {
        EffectiveAgentEnv {
            env: BTreeMap::new(),
            config_file_path: Some("~/.config/goose/config.yaml"),
            effective_command: "goose".to_string(),
        }
    }

    fn env_with(pairs: &[(&str, &str)]) -> EffectiveAgentEnv {
        EffectiveAgentEnv {
            env: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            config_file_path: Some("~/.config/goose/config.yaml"),
            effective_command: "goose".to_string(),
        }
    }

    fn databricks_file_config() -> RuntimeFileConfig {
        let mut extra = BTreeMap::new();
        extra.insert(
            "DATABRICKS_HOST".to_string(),
            "https://dbc.example.com".to_string(),
        );
        RuntimeFileConfig {
            provider: Some("databricks_v2".to_string()),
            model: Some("goose-claude-4-6-opus".to_string()),
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn goose_file_config_silences_databricks_host_requirement() {
        // File has provider, model, and DATABRICKS_HOST — all requirements silenced.
        let env = empty_env();
        let cfg = databricks_file_config();
        let result = goose_requirements(&env, Some(&cfg));
        assert!(
            result.is_empty(),
            "all requirements should be silenced by goose file config; \
             got: {:?}",
            result
        );
    }

    #[test]
    fn goose_env_empty_file_absent_still_not_ready() {
        // No env, no file config → provider and model both required.
        let env = empty_env();
        let result = goose_requirements(&env, None);
        assert!(
            result.contains(&Requirement::NormalizedField {
                field: "provider".to_string()
            }),
            "provider must be required when absent from both env and file"
        );
        assert!(
            result.contains(&Requirement::NormalizedField {
                field: "model".to_string()
            }),
            "model must be required when absent from both env and file"
        );
    }

    #[test]
    fn goose_file_config_silences_provider_and_model_but_not_anthropic_key() {
        // File has provider=anthropic and model, but ANTHROPIC_API_KEY is not
        // in the file's `extra` map — it must still be required.
        let cfg = RuntimeFileConfig {
            provider: Some("anthropic".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            extra: BTreeMap::new(),
            ..Default::default()
        };
        let env = empty_env();
        let result = goose_requirements(&env, Some(&cfg));
        // Provider and model silenced.
        assert!(
            !result.contains(&Requirement::NormalizedField {
                field: "provider".to_string()
            }),
            "provider silenced by file config"
        );
        assert!(
            !result.contains(&Requirement::NormalizedField {
                field: "model".to_string()
            }),
            "model silenced by file config"
        );
        // ANTHROPIC_API_KEY not in file extra → still required.
        assert!(
            result.contains(&Requirement::EnvKey {
                key: "ANTHROPIC_API_KEY".to_string()
            }),
            "ANTHROPIC_API_KEY must remain required when not in file extra"
        );
    }

    #[test]
    fn goose_file_config_accepts_upstream_openai_key_name() {
        let cfg = RuntimeFileConfig {
            provider: Some("openai".to_string()),
            model: Some("gpt-5".to_string()),
            extra: BTreeMap::from([(
                "OPENAI_API_KEY".to_string(),
                "configured-secret".to_string(),
            )]),
            ..Default::default()
        };

        let result = goose_requirements(&empty_env(), Some(&cfg));

        assert!(
            result.is_empty(),
            "native Goose OpenAI config should be ready"
        );
    }

    #[test]
    fn goose_env_provider_wins_over_file_provider_for_cred_check() {
        // Env has GOOSE_PROVIDER=anthropic (different from file's databricks_v2).
        // The env provider must win for credential checking.
        let env = env_with(&[
            ("GOOSE_PROVIDER", "anthropic"),
            ("GOOSE_MODEL", "claude-opus-4-5"),
        ]);
        let cfg = databricks_file_config(); // has provider=databricks_v2
        let result = goose_requirements(&env, Some(&cfg));
        // anthropic requires ANTHROPIC_API_KEY, not DATABRICKS_HOST.
        assert!(
            result.contains(&Requirement::EnvKey {
                key: "ANTHROPIC_API_KEY".to_string()
            }),
            "env provider=anthropic must require ANTHROPIC_API_KEY"
        );
        assert!(
            !result.contains(&Requirement::EnvKey {
                key: "DATABRICKS_HOST".to_string()
            }),
            "env provider=anthropic must NOT require DATABRICKS_HOST"
        );
    }

    #[test]
    fn goose_flat_databricks_host_in_file_config_silences_requirement() {
        // Will's typical goose config: flat DATABRICKS_HOST at the top level,
        // no active_provider — provider inferred as "databricks".
        // The parser must store extra["DATABRICKS_HOST"] = value (canonical key),
        // and goose_requirements must then silence the DATABRICKS_HOST requirement.
        let mut extra = BTreeMap::new();
        extra.insert(
            "DATABRICKS_HOST".to_string(),
            "https://block.cloud.databricks.com".to_string(),
        );
        let cfg = RuntimeFileConfig {
            provider: Some("databricks".to_string()),
            model: Some("goose-claude-4-5".to_string()),
            extra,
            ..Default::default()
        };
        let env = empty_env();
        let result = goose_requirements(&env, Some(&cfg));
        // All requirements silenced — provider (file), model (file), DATABRICKS_HOST (file).
        assert!(
            result.is_empty(),
            "flat DATABRICKS_HOST in file config must silence all requirements; \
             got: {:?}",
            result
        );
    }

    #[test]
    fn goose_goose_provider_databricks_flat_host_silences_databricks_host() {
        // GOOSE_PROVIDER=databricks (not active_provider) + flat DATABRICKS_HOST.
        // The parser canonicalizes to extra["DATABRICKS_HOST"]; readiness must silence it.
        let mut extra = BTreeMap::new();
        extra.insert(
            "DATABRICKS_HOST".to_string(),
            "https://dbc.example.com".to_string(),
        );
        let cfg = RuntimeFileConfig {
            provider: Some("databricks".to_string()),
            model: Some("some-model".to_string()),
            extra,
            ..Default::default()
        };
        let env = empty_env();
        let result = goose_requirements(&env, Some(&cfg));
        assert!(
            !result.contains(&Requirement::EnvKey {
                key: "DATABRICKS_HOST".to_string()
            }),
            "DATABRICKS_HOST must be silenced when canonical key is in file extra"
        );
    }
}
