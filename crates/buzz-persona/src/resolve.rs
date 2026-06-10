//! Pack resolution: produces fully resolved, ACP-ready output.
//!
//! `resolve_pack()` is the main entry point. It loads a pack directory,
//! applies merge policy, composes prompts, merges MCP servers, and projects
//! env vars. The output (`ResolvedPack`) is designed backward from ACP's
//! `Config` — every field maps directly to what the runtime consumes.
//!
//! Design principles:
//! - **Pure**: no env access, no network, no side effects.
//! - **Complete**: all merge/compose/project logic lives here.
//! - **ACP-shaped**: `ResolvedPersona` maps 1:1 to ACP's needs.

use std::collections::HashMap;
use std::path::Path;

use crate::merge::TriggersData;
use crate::pack::{self, LoadedPack, LoadedPersona, PackError};
use crate::persona::split_model;

// ── Public types ──────────────────────────────────────────────────────────────

/// A fully resolved persona — ready for ACP consumption.
/// All merge, composition, and projection is done.
#[derive(Debug, Clone)]
pub struct ResolvedPersona {
    // Identity
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub avatar: Option<String>,
    pub version: String,

    // → Config.system_prompt (persona body + pack_instructions)
    pub system_prompt: String,

    // → Config.model (plain model ID, post-split)
    pub model: Option<String>,
    /// LLM inference provider extracted from the model string colon prefix (e.g., 'databricks'
    /// from 'databricks:model-id'). Flows into harness-specific env vars (GOOSE_PROVIDER) only.
    pub llm_provider: Option<String>,
    /// Preferred ACP runtime ID from the persona config (e.g., 'goose', 'claude'). Maps to
    /// PersonaRecord.runtime during pack import.
    pub runtime: Option<String>,
    pub temperature: Option<f64>,
    pub max_context_tokens: Option<u64>,

    // → Config.subscribe_mode + channels_override
    pub subscribe: Vec<String>,
    // → mapped to ACP filter rules at startup
    pub triggers: ResolvedTriggers,
    pub thread_replies: bool,
    pub broadcast_replies: bool,

    // Effective MCP (pack shared + persona merged, literals preserved)
    pub mcp_servers: Vec<ResolvedMcpServer>,

    // Hooks (parsed, not executed — reserved for future use, not yet wired)
    pub hooks: Option<ResolvedHooks>,

    // Skills (bare names — reserved for future use, not yet wired)
    pub skills: Vec<String>,

    // Env var projection for agent subprocess
    pub runtime_env_vars: Vec<(String, String)>,
}

/// An MCP server with env values as literals (no interpolation in this PR).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMcpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Lifecycle hooks (pack-relative paths).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedHooks {
    pub on_start: Option<String>,
    pub on_stop: Option<String>,
    pub on_message: Option<String>,
}

/// What triggers a response (renamed from respond_to per spec discussion).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTriggers {
    pub mentions: bool,
    pub keywords: Vec<String>,
    pub all_messages: bool,
}

/// A fully resolved pack.
#[derive(Debug)]
pub struct ResolvedPack {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub personas: Vec<ResolvedPersona>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Load, validate, merge, and resolve a pack directory.
///
/// Returns a `ResolvedPack` with fully typed, ACP-ready output for each
/// persona. All merge policy (levels 3-5) is applied. MCP servers are
/// merged with literal env passthrough (no `${VAR}` interpolation).
/// Env vars are projected from model/temperature/context config.
pub fn resolve_pack(pack_dir: &Path) -> Result<ResolvedPack, PackError> {
    let loaded = pack::load_pack(pack_dir)?;
    resolve_loaded_pack(&loaded)
}

/// Resolve from an already-loaded pack. Useful when you've already called
/// `load_pack()` and want to avoid re-reading the filesystem.
///
/// Runs semantic validation (zero personas, duplicate names, invalid slugs)
/// before resolution. Returns `PackError` on failure.
pub fn resolve_loaded_pack(loaded: &LoadedPack) -> Result<ResolvedPack, PackError> {
    // Semantic validation — catch issues that load_pack() doesn't check.
    if loaded.personas.is_empty() {
        return Err(PackError::ManifestParse(
            "pack contains zero personas".into(),
        ));
    }
    let mut seen_names = std::collections::HashSet::new();
    for p in &loaded.personas {
        if !seen_names.insert(&p.name) {
            return Err(PackError::FileParse {
                path: p.source_path.clone(),
                reason: format!("duplicate persona name \"{}\"", p.name),
            });
        }
        if !p
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(PackError::FileParse {
                path: p.source_path.clone(),
                reason: format!(
                    "persona name \"{}\" contains invalid characters (allowed: [a-zA-Z0-9_-])",
                    p.name
                ),
            });
        }
        if p.name.len() > 64 {
            return Err(PackError::FileParse {
                path: p.source_path.clone(),
                reason: format!(
                    "persona name \"{}\" exceeds 64 characters (got {})",
                    p.name,
                    p.name.len()
                ),
            });
        }
    }

    let pack_version = &loaded.manifest.version;
    let pack_instructions = loaded.pack_instructions.as_deref();
    let shared_mcp = loaded.shared_mcp_config.as_ref();

    let mut personas = Vec::with_capacity(loaded.personas.len());
    for lp in &loaded.personas {
        personas.push(resolve_one_persona(
            lp,
            pack_version,
            pack_instructions,
            shared_mcp,
        ));
    }

    Ok(ResolvedPack {
        id: loaded.manifest.id.clone(),
        name: loaded.manifest.name.clone(),
        version: loaded.manifest.version.clone(),
        // Pack-level description not yet wired through PackManifestData.
        description: loaded.manifest.description.clone().unwrap_or_default(),
        personas,
    })
}

/// Resolve a single persona by name from a pack directory.
///
/// Convenience wrapper: loads the pack, finds the named persona, resolves it.
/// Returns `PackError::PersonaNotFound` if no persona with that name exists.
pub fn resolve_persona_by_name(pack_dir: &Path, name: &str) -> Result<ResolvedPersona, PackError> {
    let pack = resolve_pack(pack_dir)?;
    pack.personas
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| PackError::PersonaNotFound(pack_dir.join(name)))
}

// ── Per-persona resolution ────────────────────────────────────────────────────

fn resolve_one_persona(
    lp: &LoadedPersona,
    pack_version: &str,
    pack_instructions: Option<&str>,
    shared_mcp: Option<&serde_json::Value>,
) -> ResolvedPersona {
    let system_prompt = compose_prompt(&lp.prompt, pack_instructions);

    // Split "provider:model-id" into separate fields (V3 contract).
    let (llm_provider, model) = match lp.model.as_deref() {
        Some(s) if !s.trim().is_empty() => {
            let (prov, id) = split_model(s);
            (
                prov.filter(|p| !p.is_empty()).map(str::to_owned),
                Some(id.to_owned()),
            )
        }
        _ => (None, None),
    };

    let triggers = resolve_triggers(lp.triggers.as_ref());
    let mcp_servers = merge_mcp_servers(shared_mcp, &lp.mcp_servers);
    let hooks = resolve_hooks(lp.hooks.as_ref());
    let runtime_env_vars = runtime_env_vars(lp);

    // Version: LoadedPersona has no per-persona version field — persona files
    // don't declare a version in frontmatter. The pack version is used as-is.
    // If per-persona versioning is added in the future, LoadedPersona should
    // gain `version: Option<String>` and this line should become:
    //   lp.version.clone().unwrap_or_else(|| pack_version.to_owned())
    let version = pack_version.to_owned();

    ResolvedPersona {
        name: lp.name.clone(),
        display_name: lp.display_name.clone(),
        description: lp.description.clone(),
        avatar: lp.avatar.clone(),
        version,
        system_prompt,
        model,
        llm_provider,
        runtime: lp.runtime.clone(),
        temperature: lp.temperature,
        max_context_tokens: lp.max_context_tokens,
        subscribe: lp.subscribe.clone(),
        triggers,
        thread_replies: lp.thread_replies,
        broadcast_replies: lp.broadcast_replies,
        mcp_servers,
        hooks,
        skills: lp.skills.clone(),
        runtime_env_vars,
    }
}

// ── Compose prompt ────────────────────────────────────────────────────────────

/// Compose the effective system prompt: persona body + pack instructions.
fn compose_prompt(persona_prompt: &str, pack_instructions: Option<&str>) -> String {
    match pack_instructions {
        Some(instructions) if !instructions.trim().is_empty() => {
            format!("{persona_prompt}\n\n---\n# Team Instructions\n{instructions}")
        }
        _ => persona_prompt.to_owned(),
    }
}

// ── Triggers resolution ───────────────────────────────────────────────────────

/// Convert `TriggersData` to `ResolvedTriggers`.
fn resolve_triggers(rt: Option<&TriggersData>) -> ResolvedTriggers {
    match rt {
        Some(data) => ResolvedTriggers {
            mentions: data.mentions,
            keywords: data.keywords.clone(),
            all_messages: data.all_messages,
        },
        None => ResolvedTriggers {
            mentions: true,
            keywords: Vec::new(),
            all_messages: false,
        },
    }
}

// ── MCP server merge ──────────────────────────────────────────────────────────

/// Merge pack-level shared MCP servers with per-persona servers.
///
/// Pack shared servers come from `.mcp.json` (a map of `name → config`).
/// Per-persona servers come from frontmatter `mcp_servers:` (a list).
/// Name collision: persona wins (replaces pack server with same name).
///
/// Env values are passed through as literals — no `${VAR}` interpolation.
fn merge_mcp_servers(
    shared_mcp: Option<&serde_json::Value>,
    persona_servers: &[serde_json::Value],
) -> Vec<ResolvedMcpServer> {
    let mut by_name: HashMap<String, ResolvedMcpServer> = HashMap::new();

    // 1. Pack-level shared servers from .mcp.json
    if let Some(shared) = shared_mcp {
        // .mcp.json format: { "mcpServers": { "name": { "command": ..., "args": [...], "env": {...} } } }
        if let Some(servers_obj) = shared.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, config) in servers_obj {
                if let Some(server) = parse_mcp_server_config(name, config) {
                    by_name.insert(name.clone(), server);
                }
            }
        }
    }

    // 2. Per-persona servers (persona wins on name collision)
    for server_val in persona_servers {
        if let Some(name) = server_val.get("name").and_then(|v| v.as_str()) {
            if let Some(server) = parse_mcp_server_config(name, server_val) {
                by_name.insert(name.to_owned(), server);
            }
        }
    }

    // Return in deterministic order (sorted by name)
    let mut servers: Vec<_> = by_name.into_values().collect();
    servers.sort_by_key(|s| s.name.clone());
    servers
}

/// Parse a single MCP server config from JSON.
fn parse_mcp_server_config(name: &str, config: &serde_json::Value) -> Option<ResolvedMcpServer> {
    let command = config.get("command").and_then(|v| v.as_str())?.to_owned();
    let args = config
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let env = config
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default();

    Some(ResolvedMcpServer {
        name: name.to_owned(),
        command,
        args,
        env,
    })
}

// ── Hooks resolution ──────────────────────────────────────────────────────────

/// Store hook paths as raw relative strings (no path resolution).
///
/// Security: we intentionally do NOT resolve these to absolute paths.
/// Hook paths come from untrusted persona frontmatter and could contain
/// `../` traversal. Since hooks are not executed in this PR, we store
/// them as-is. The PR that wires execution MUST validate through
/// `safe_resolve()` before use.
fn resolve_hooks(hooks: Option<&crate::merge::HooksData>) -> Option<ResolvedHooks> {
    let h = hooks?;
    if h.on_start.is_none() && h.on_stop.is_none() && h.on_message.is_none() {
        return None;
    }
    Some(ResolvedHooks {
        on_start: h.on_start.clone(),
        on_stop: h.on_stop.clone(),
        on_message: h.on_message.clone(),
    })
}

// ── Env var projection ────────────────────────────────────────────────────────

/// Project persona config into agent subprocess env vars.
///
/// Pure function — does NOT read the current process env.
/// ACP is responsible for filtering based on operator precedence (level 1):
/// if the operator already set an env var, ACP skips injection so the
/// operator's value wins.
fn runtime_env_vars(persona: &LoadedPersona) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    let runtime = persona.runtime.as_deref();

    if let Some(model_str) = &persona.model {
        let (provider, model_id) = split_model(model_str);

        match runtime {
            Some("buzz-agent") => {
                vars.push(("BUZZ_AGENT_MODEL".to_owned(), model_id.to_owned()));
                if let Some(p) = provider {
                    vars.push(("BUZZ_AGENT_PROVIDER".to_owned(), p.to_owned()));
                }
            }
            _ => {
                if let Some(p) = provider {
                    vars.push(("GOOSE_PROVIDER".to_owned(), p.to_owned()));
                }
                vars.push(("GOOSE_MODEL".to_owned(), model_id.to_owned()));
            }
        }
    }

    // temperature and context_limit stay as GOOSE_* (only goose reads them)
    if let Some(temp) = persona.temperature {
        vars.push(("GOOSE_TEMPERATURE".to_owned(), temp.to_string()));
    }

    if let Some(ctx) = persona.max_context_tokens {
        vars.push(("GOOSE_CONTEXT_LIMIT".to_owned(), ctx.to_string()));
    }

    vars
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::merge::{HooksData, TriggersData};

    // ── compose_prompt ────────────────────────────────────────────────────

    #[test]
    fn compose_prompt_body_only() {
        let result = compose_prompt("You are a bot.", None);
        assert_eq!(result, "You are a bot.");
    }

    #[test]
    fn compose_prompt_with_instructions() {
        let result = compose_prompt("You are a bot.", Some("Follow the rules."));
        assert!(result.starts_with("You are a bot."));
        assert!(result.contains("# Team Instructions"));
        assert!(result.contains("Follow the rules."));
    }

    #[test]
    fn compose_prompt_empty_instructions_ignored() {
        let result = compose_prompt("You are a bot.", Some("   "));
        assert_eq!(result, "You are a bot.");
    }

    // ── resolve_triggers ──────────────────────────────────────────────────

    #[test]
    fn triggers_from_triggers_data() {
        let data = TriggersData {
            mentions: false,
            keywords: vec!["security".into(), "CVE".into()],
            all_messages: false,
        };
        let t = resolve_triggers(Some(&data));
        assert!(!t.mentions);
        assert_eq!(t.keywords, vec!["security", "CVE"]);
        assert!(!t.all_messages);
    }

    #[test]
    fn triggers_default_when_none() {
        let t = resolve_triggers(None);
        assert!(t.mentions);
        assert!(t.keywords.is_empty());
        assert!(!t.all_messages);
    }

    // ── merge_mcp_servers ─────────────────────────────────────────────────

    #[test]
    fn mcp_merge_shared_only() {
        let shared = serde_json::json!({
            "mcpServers": {
                "example-mcp": {
                    "command": "npx",
                    "args": ["-y", "example-mcp"],
                    "env": { "TOKEN": "abc" }
                }
            }
        });
        let result = merge_mcp_servers(Some(&shared), &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "example-mcp");
        assert_eq!(result[0].command, "npx");
        assert_eq!(result[0].env, vec![("TOKEN".into(), "abc".into())]);
    }

    #[test]
    fn mcp_merge_persona_wins_on_collision() {
        let shared = serde_json::json!({
            "mcpServers": {
                "my-server": {
                    "command": "old-cmd",
                    "args": [],
                    "env": {}
                }
            }
        });
        let persona = vec![serde_json::json!({
            "name": "my-server",
            "command": "new-cmd",
            "args": ["--flag"],
            "env": { "KEY": "val" }
        })];
        let result = merge_mcp_servers(Some(&shared), &persona);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].command, "new-cmd");
        assert_eq!(result[0].args, vec!["--flag"]);
    }

    #[test]
    fn mcp_merge_both_sources_combined() {
        let shared = serde_json::json!({
            "mcpServers": {
                "alpha": { "command": "alpha-cmd" }
            }
        });
        let persona = vec![serde_json::json!({
            "name": "beta",
            "command": "beta-cmd"
        })];
        let result = merge_mcp_servers(Some(&shared), &persona);
        assert_eq!(result.len(), 2);
        // Sorted by name
        assert_eq!(result[0].name, "alpha");
        assert_eq!(result[1].name, "beta");
    }

    #[test]
    fn mcp_merge_no_shared_no_persona() {
        let result = merge_mcp_servers(None, &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn mcp_env_literals_preserved() {
        // ${VAR_NAME} should pass through as-is (no interpolation in this PR)
        let persona = vec![serde_json::json!({
            "name": "test",
            "command": "cmd",
            "env": { "PATH": "${HOME}/bin", "SECRET": "${MY_SECRET}" }
        })];
        let result = merge_mcp_servers(None, &persona);
        assert_eq!(result.len(), 1);
        let env: HashMap<String, String> = result[0].env.iter().cloned().collect();
        assert_eq!(env["PATH"], "${HOME}/bin");
        assert_eq!(env["SECRET"], "${MY_SECRET}");
    }

    // ── resolve_hooks ─────────────────────────────────────────────────────

    #[test]
    fn hooks_stored_as_raw_relative_paths() {
        // Security: hooks are stored as raw strings, NOT resolved to absolute.
        // Path traversal validation deferred to the PR that wires execution.
        let data = HooksData {
            on_start: Some("hooks/start.sh".into()),
            on_stop: Some("hooks/stop.sh".into()),
            on_message: None,
        };
        let h = resolve_hooks(Some(&data)).unwrap();
        assert_eq!(h.on_start.as_deref(), Some("hooks/start.sh"));
        assert_eq!(h.on_stop.as_deref(), Some("hooks/stop.sh"));
    }

    #[test]
    fn hooks_none_when_empty() {
        let data = HooksData {
            on_start: None,
            on_stop: None,
            on_message: None,
        };
        assert!(resolve_hooks(Some(&data)).is_none());
    }

    #[test]
    fn hooks_none_when_absent() {
        assert!(resolve_hooks(None).is_none());
    }

    // ── runtime_env_vars ──────────────────────────────────────────────────

    #[test]
    fn env_vars_projected_from_model() {
        let lp = stub_persona(Some("anthropic:claude-sonnet-4-20250514"), None, None);
        let vars = runtime_env_vars(&lp);
        let map: HashMap<&str, &str> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(map["GOOSE_PROVIDER"], "anthropic");
        assert_eq!(map["GOOSE_MODEL"], "claude-sonnet-4-20250514");
    }

    #[test]
    fn env_vars_model_without_provider() {
        let lp = stub_persona(Some("gpt-4o"), None, None);
        let vars = runtime_env_vars(&lp);
        let map: HashMap<&str, &str> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert!(!map.contains_key("GOOSE_PROVIDER"));
        assert_eq!(map["GOOSE_MODEL"], "gpt-4o");
    }

    #[test]
    fn env_vars_temperature_and_context() {
        let lp = stub_persona(None, Some(0.7), Some(8192));
        let vars = runtime_env_vars(&lp);
        let map: HashMap<&str, &str> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(map["GOOSE_TEMPERATURE"], "0.7");
        assert_eq!(map["GOOSE_CONTEXT_LIMIT"], "8192");
    }

    #[test]
    fn env_vars_empty_when_no_config() {
        let lp = stub_persona(None, None, None);
        let vars = runtime_env_vars(&lp);
        assert!(vars.is_empty());
    }

    #[test]
    fn env_vars_full_projection() {
        let lp = stub_persona(Some("openai:gpt-4o"), Some(0.5), Some(16384));
        let vars = runtime_env_vars(&lp);
        assert_eq!(vars.len(), 4); // PROVIDER, MODEL, TEMPERATURE, CONTEXT_LIMIT
    }

    #[test]
    fn runtime_env_vars_buzz_agent_emits_buzz_agent_vars() {
        let mut lp = stub_persona(Some("databricks:goose-claude-4-6-opus"), None, None);
        lp.runtime = Some("buzz-agent".to_owned());
        let vars = runtime_env_vars(&lp);
        let map: HashMap<&str, &str> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(map["BUZZ_AGENT_MODEL"], "goose-claude-4-6-opus");
        assert_eq!(map["BUZZ_AGENT_PROVIDER"], "databricks");
        assert!(!map.contains_key("GOOSE_MODEL"));
        assert!(!map.contains_key("GOOSE_PROVIDER"));
    }

    #[test]
    fn runtime_env_vars_goose_emits_goose_vars() {
        let mut lp = stub_persona(Some("databricks:goose-claude-4-6-opus"), None, None);
        lp.runtime = Some("goose".to_owned());
        let vars = runtime_env_vars(&lp);
        let map: HashMap<&str, &str> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(map["GOOSE_MODEL"], "goose-claude-4-6-opus");
        assert_eq!(map["GOOSE_PROVIDER"], "databricks");
        assert!(!map.contains_key("BUZZ_AGENT_MODEL"));
    }

    #[test]
    fn runtime_env_vars_no_runtime_defaults_to_goose() {
        let lp = stub_persona(Some("anthropic:claude-sonnet-4-20250514"), None, None);
        let vars = runtime_env_vars(&lp);
        let map: HashMap<&str, &str> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(map["GOOSE_PROVIDER"], "anthropic");
        assert_eq!(map["GOOSE_MODEL"], "claude-sonnet-4-20250514");
    }

    #[test]
    fn runtime_env_vars_buzz_agent_model_without_provider() {
        let mut lp = stub_persona(Some("gpt-4o"), None, None);
        lp.runtime = Some("buzz-agent".to_owned());
        let vars = runtime_env_vars(&lp);
        let map: HashMap<&str, &str> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(map["BUZZ_AGENT_MODEL"], "gpt-4o");
        assert!(!map.contains_key("BUZZ_AGENT_PROVIDER"));
    }

    // ── Full pipeline (resolve_pack via filesystem) ───────────────────────

    #[test]
    fn resolve_minimal_pack() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.minimal",
                "name": "Minimal Pack",
                "version": "0.1.0",
                "personas": ["agents/bot.persona.md"]
            }"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("agents/bot.persona.md"),
            "---\nname: bot\ndisplay_name: Bot\ndescription: A test bot.\n---\nYou are Bot.\n",
        )
        .unwrap();

        let pack = resolve_pack(dir).unwrap();
        assert_eq!(pack.id, "com.test.minimal");
        assert_eq!(pack.personas.len(), 1);

        let p = &pack.personas[0];
        assert_eq!(p.name, "bot");
        assert_eq!(p.system_prompt, "You are Bot.\n");
        assert!(p.model.is_none());
        assert!(p.llm_provider.is_none());
        assert!(p.triggers.mentions); // built-in default
        assert!(p.mcp_servers.is_empty());
        assert!(p.runtime_env_vars.is_empty());
    }

    #[test]
    fn resolve_pack_with_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.instructions",
                "name": "Instructions Pack",
                "version": "1.0.0",
                "personas": ["agents/bot.persona.md"],
                "pack_instructions": "instructions.md"
            }"#,
        )
        .unwrap();

        std::fs::write(dir.join("instructions.md"), "Always be helpful.").unwrap();

        std::fs::write(
            dir.join("agents/bot.persona.md"),
            "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\nmodel: anthropic:claude-sonnet-4-20250514\n---\nYou are Bot.\n",
        )
        .unwrap();

        let pack = resolve_pack(dir).unwrap();
        let p = &pack.personas[0];

        // Prompt composed with instructions
        assert!(p.system_prompt.contains("You are Bot."));
        assert!(p.system_prompt.contains("# Team Instructions"));
        assert!(p.system_prompt.contains("Always be helpful."));

        // Model split into separate fields (V3 contract)
        assert_eq!(p.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(p.llm_provider.as_deref(), Some("anthropic"));

        // Env vars projected
        let env_map: HashMap<&str, &str> = p
            .runtime_env_vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(env_map["GOOSE_PROVIDER"], "anthropic");
        assert_eq!(env_map["GOOSE_MODEL"], "claude-sonnet-4-20250514");
    }

    #[test]
    fn resolve_multi_persona_pack() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.multi",
                "name": "Multi Pack",
                "version": "2.0.0",
                "personas": ["agents/pip.persona.md", "agents/lep.persona.md"],
                "defaults": {
                    "model": "anthropic:claude-sonnet-4-20250514",
                    "temperature": 0.7,
                    "thread_replies": true
                }
            }"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("agents/pip.persona.md"),
            "---\nname: pip\ndisplay_name: Pip\ndescription: The lead.\nmodel: anthropic:claude-4-opus-20250514\nsubscribe:\n  - '#reviews'\n---\nYou are Pip.\n",
        )
        .unwrap();

        std::fs::write(
            dir.join("agents/lep.persona.md"),
            "---\nname: lep\ndisplay_name: Lep\ndescription: The analyst.\ntemperature: 0.3\n---\nYou are Lep.\n",
        )
        .unwrap();

        let pack = resolve_pack(dir).unwrap();
        assert_eq!(pack.personas.len(), 2);

        let pip = pack.personas.iter().find(|p| p.name == "pip").unwrap();
        let lep = pack.personas.iter().find(|p| p.name == "lep").unwrap();

        // pip overrides model
        assert_eq!(pip.model.as_deref(), Some("claude-4-opus-20250514"));
        assert_eq!(pip.llm_provider.as_deref(), Some("anthropic"));
        // lep inherits model from defaults
        assert_eq!(lep.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(lep.llm_provider.as_deref(), Some("anthropic"));

        // pip inherits temperature from defaults
        assert_eq!(pip.temperature, Some(0.7));
        // lep overrides temperature
        assert_eq!(lep.temperature, Some(0.3));

        // pip has explicit subscribe
        assert_eq!(pip.subscribe, vec!["#reviews"]);
        // lep has no subscribe (empty from defaults)
        assert!(lep.subscribe.is_empty());

        // Both inherit thread_replies from defaults
        assert!(pip.thread_replies);
        assert!(lep.thread_replies);
    }

    // ── resolve_persona_by_name ──────────────────────────────────────────

    #[test]
    fn resolve_persona_by_name_found() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"1.0.0","personas":["agents/a.persona.md","agents/b.persona.md"]}"#,
        ).unwrap();
        std::fs::write(
            dir.join("agents/a.persona.md"),
            "---\nname: alpha\ndisplay_name: Alpha\ndescription: First.\n---\nAlpha prompt.\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/b.persona.md"),
            "---\nname: beta\ndisplay_name: Beta\ndescription: Second.\n---\nBeta prompt.\n",
        )
        .unwrap();

        let p = resolve_persona_by_name(dir, "beta").unwrap();
        assert_eq!(p.name, "beta");
        assert_eq!(p.display_name, "Beta");
        assert!(p.system_prompt.contains("Beta prompt."));
    }

    #[test]
    fn resolve_persona_by_name_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"1.0.0","personas":["agents/a.persona.md"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/a.persona.md"),
            "---\nname: alpha\ndisplay_name: Alpha\ndescription: First.\n---\n",
        )
        .unwrap();

        let err = resolve_persona_by_name(dir, "nonexistent").unwrap_err();
        assert!(matches!(err, PackError::PersonaNotFound(_)));
    }

    // ── model split ───────────────────────────────────────────────────────

    #[test]
    fn model_split_provider_and_id() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"1.0.0","personas":["agents/a.persona.md"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/a.persona.md"),
            "---\nname: a\ndisplay_name: A\ndescription: A.\nmodel: openai:gpt-4o\n---\n",
        )
        .unwrap();

        let pack = resolve_pack(dir).unwrap();
        let p = &pack.personas[0];
        assert_eq!(p.model.as_deref(), Some("gpt-4o"));
        assert_eq!(p.llm_provider.as_deref(), Some("openai"));
    }

    #[test]
    fn model_no_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"1.0.0","personas":["agents/a.persona.md"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/a.persona.md"),
            "---\nname: a\ndisplay_name: A\ndescription: A.\nmodel: gpt-4o\n---\n",
        )
        .unwrap();

        let pack = resolve_pack(dir).unwrap();
        let p = &pack.personas[0];
        assert_eq!(p.model.as_deref(), Some("gpt-4o"));
        assert!(p.llm_provider.is_none());
    }

    // ── Test helpers ──────────────────────────────────────────────────────

    fn stub_persona(
        model: Option<&str>,
        temperature: Option<f64>,
        max_context_tokens: Option<u64>,
    ) -> LoadedPersona {
        LoadedPersona {
            source_path: PathBuf::from("test.persona.md"),
            name: "test".into(),
            display_name: "Test".into(),
            description: "A test persona.".into(),
            avatar: None,
            model: model.map(str::to_owned),
            runtime: None,
            temperature,
            max_context_tokens,
            subscribe: vec![],
            triggers: None,
            thread_replies: true,
            broadcast_replies: false,
            skills: vec![],
            mcp_servers: vec![],
            hooks: None,
            prompt: "You are a test.".into(),
        }
    }
}
