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
//! 3. Merged user env (`merged_user_env`) — the record's `env_vars` after
//!    reserved-key and malformed-key filtering.  Last-wins on collision.
//!
//! The config-file tier (Goose `~/.config/goose/config.yaml`) is tracked
//! separately because it is not part of the process env — the harness reads
//! it at startup.  We do not evaluate it here; it is exposed for future
//! UI display only.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::managed_agents::{
    agent_env::baked_build_env,
    config_bridge::read_goose_file_config,
    discovery::{known_acp_runtime, resolve_command, KnownAcpRuntime},
    effective_agent_command,
    env_vars::merged_user_env,
    types::{ManagedAgentRecord, PersonaRecord},
};

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
    pub config_file_path: Option<&'static str>,
    /// The resolved harness binary name (e.g. `"buzz-agent"`, `"goose"`).
    pub effective_command: String,
}

/// Assemble the effective agent env from a record, personas, and optional
/// known-runtime metadata — without an `AppHandle` so it is unit-testable.
///
/// # Arguments
/// * `record` — the managed agent record (model/provider/env_vars/…)
/// * `personas` — all current persona records (for persona-backed resolution)
/// * `runtime` — the `KnownAcpRuntime` for the effective command, if any
pub(crate) fn resolve_effective_agent_env(
    record: &ManagedAgentRecord,
    personas: &[PersonaRecord],
    runtime: Option<&KnownAcpRuntime>,
) -> EffectiveAgentEnv {
    let effective_command = effective_agent_command(
        record.persona_id.as_deref(),
        personas,
        record.agent_command_override.as_deref(),
    );

    // Layer 1: baked build defaults (floor — internal builds only; OSS = empty).
    let mut env = baked_build_env();

    // Layer 2: runtime metadata env vars (model / provider keys derived from
    // the record's structured fields).
    let effective_model = record.model.as_deref();
    let effective_provider = record.provider.as_deref();
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

    // Layer 3: merged user env (agent env_vars after reserved/malformed-key
    // filtering; last-wins on collision).
    let user_env = merged_user_env(&BTreeMap::new(), &record.env_vars);
    env.extend(user_env);

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
        /// (e.g. `"run \`codex login --with-api-key\`"`).
        setup_copy: String,
    },
}

impl Requirement {
    /// Short label for logging/nudge copy.
    pub(crate) fn label(&self) -> String {
        match self {
            Requirement::NormalizedField { field } => format!("missing {field}"),
            Requirement::EnvKey { key } => format!("missing env {key}"),
            Requirement::CliLogin { setup_copy, .. } => setup_copy.clone(),
        }
    }
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
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self, AgentReadiness::Ready)
    }

    /// Returns the missing requirements, or an empty slice if ready.
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
        "claude" => cli_login_requirements(
            &["claude", "auth", "status"],
            "complete Claude Code authentication by running the Claude CLI",
        ),
        "codex" => cli_login_requirements(
            &["codex", "login", "status"],
            "run `codex login --with-api-key`",
        ),
        _ => vec![],
    }
}

/// Requirements for buzz-agent (provider + model + provider-specific creds).
fn buzz_agent_requirements(effective: &EffectiveAgentEnv) -> Vec<Requirement> {
    let mut missing = Vec::new();

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
    let model = effective
        .env
        .get("BUZZ_AGENT_MODEL")
        .filter(|v| !v.is_empty())
        .map(String::as_str);
    if model.is_none() {
        missing.push(Requirement::NormalizedField {
            field: "model".to_string(),
        });
    }

    // Provider-specific credential requirements.
    // A key present with an empty value is treated as absent — matching the
    // dialog's (envVars[key] ?? "").length === 0 emptiness check.
    let env_key_missing = |key: &str| effective.env.get(key).map_or(true, |v| v.is_empty());
    match provider {
        Some("anthropic") => {
            if env_key_missing("ANTHROPIC_API_KEY") {
                missing.push(Requirement::EnvKey {
                    key: "ANTHROPIC_API_KEY".to_string(),
                });
            }
        }
        Some("openai") => {
            if env_key_missing("OPENAI_COMPAT_API_KEY") {
                missing.push(Requirement::EnvKey {
                    key: "OPENAI_COMPAT_API_KEY".to_string(),
                });
            }
        }
        Some("databricks") | Some("databricks_v2") => {
            // DATABRICKS_HOST is hard-required; DATABRICKS_TOKEN is optional
            // (OAuth PKCE is the normal path — see buzz-agent/src/config.rs:143).
            if env_key_missing("DATABRICKS_HOST") {
                missing.push(Requirement::EnvKey {
                    key: "DATABRICKS_HOST".to_string(),
                });
            }
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
    let env_key_missing = |key: &str| effective.env.get(key).map_or(true, |v| v.is_empty());
    // A credential key is also satisfied when the file config's `extra` map
    // contains it (e.g. DATABRICKS_HOST set in the goose config file).
    let file_key_present = |key: &str| -> bool {
        file_cfg
            .as_ref()
            .map(|c| c.extra.get(key).map_or(false, |v| !v.is_empty()))
            .unwrap_or(false)
    };
    match effective_provider {
        Some("anthropic") => {
            if env_key_missing("ANTHROPIC_API_KEY") && !file_key_present("ANTHROPIC_API_KEY") {
                missing.push(Requirement::EnvKey {
                    key: "ANTHROPIC_API_KEY".to_string(),
                });
            }
        }
        Some("openai") => {
            if env_key_missing("OPENAI_COMPAT_API_KEY")
                && !file_key_present("OPENAI_COMPAT_API_KEY")
            {
                missing.push(Requirement::EnvKey {
                    key: "OPENAI_COMPAT_API_KEY".to_string(),
                });
            }
        }
        Some("databricks") | Some("databricks_v2") => {
            if env_key_missing("DATABRICKS_HOST") && !file_key_present("DATABRICKS_HOST") {
                missing.push(Requirement::EnvKey {
                    key: "DATABRICKS_HOST".to_string(),
                });
            }
        }
        _ => {}
    }

    missing
}

/// Requirements for CLI-login runtimes (claude, codex).
///
/// We probe the CLI's login-status command synchronously. These probes are
/// fast (<300ms) and the results are memoized by the caller for the session
/// lifetime if desired.
fn cli_login_requirements(probe_args: &[&str], setup_copy: &str) -> Vec<Requirement> {
    // Resolve the binary through the full PATH-search + login-shell path so
    // the probe works in a packaged macOS DMG where the GUI PATH lacks
    // npm/homebrew directories (where `claude` / `codex` typically live).
    //
    // If the binary genuinely does not exist, stay NotReady — the user needs
    // to install it. If it exists but is not on the GUI PATH, resolve_command
    // finds it via the login-shell fallback.
    let Some(binary_path) = resolve_command(probe_args[0]) else {
        // Binary not found → not installed → NotReady.
        return vec![Requirement::CliLogin {
            probe_args: probe_args.iter().map(|s| s.to_string()).collect(),
            setup_copy: setup_copy.to_string(),
        }];
    };

    // Run the probe at the resolved absolute path so the GUI-PATH gap is bypassed.
    let logged_in = std::process::Command::new(&binary_path)
        .args(&probe_args[1..])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if logged_in {
        vec![]
    } else {
        vec![Requirement::CliLogin {
            probe_args: probe_args.iter().map(|s| s.to_string()).collect(),
            setup_copy: setup_copy.to_string(),
        }]
    }
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
        // codex uses its own credential store via `codex login --with-api-key`.
        // The nudge copy must NOT say "set OPENAI_API_KEY".
        // We test the static not-logged-in path by constructing the requirement
        // directly (the binary may or may not be installed in CI).
        let reqs = cli_login_requirements(
            &["codex", "login", "status"],
            "run `codex login --with-api-key`",
        );
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

    #[test]
    fn cli_login_requirements_missing_binary_is_not_ready() {
        // A binary that cannot possibly exist on any system → binary not found
        // → resolve_command returns None → function must return CliLogin
        // requirement (NotReady), not panic or return Ready.
        let reqs = cli_login_requirements(
            &["__buzz_nonexistent_binary_abc123__", "status"],
            "install the tool first",
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
    }

    #[test]
    fn cli_login_requirements_resolvable_binary_runs_probe_at_resolved_path() {
        // `true` is a POSIX built-in available on every CI runner and always
        // exits 0. Use it as a probe: probe_args = ["true", "--probe-arg"]
        // → resolved path probed → exit 0 → logged_in = true → requirements
        // is empty (Ready). This exercises the resolve_command fast path
        // (binary found on PATH) + the probe-at-resolved-path branch.
        //
        // `true` is universally resolvable on POSIX/macOS/Linux CI, so we
        // assert the ready (empty) outcome directly rather than hedging.
        let reqs = cli_login_requirements(
            &["true", "--probe-arg"],
            "this should not show (true always succeeds)",
        );
        assert!(
            reqs.is_empty(),
            "expected Ready (no requirements) when probe binary resolves and exits 0; \
             got {:?} — `true` is always on PATH on POSIX/macOS/Linux CI so this \
             must be empty",
            reqs
        );
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
            setup_copy: "run `codex login --with-api-key`".to_string(),
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
            mcp_toolsets: None,
            env_vars,
            start_on_app_launch: false,
            runtime_pid: None,
            backend: Default::default(),
            backend_agent_id: None,
            provider_binary_path: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            respond_to: Default::default(),
            respond_to_allowlist: vec![],
            relay_mesh: None,
        };

        let runtime = known_acp_runtime_exact("buzz-agent");
        let effective = resolve_effective_agent_env(&record, &[], runtime);

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
