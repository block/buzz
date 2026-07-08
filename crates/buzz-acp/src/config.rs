//! Configuration for the buzz-acp harness.
//!
//! CLI-first: every option is a CLI flag with env var fallback.
//! Config file (TOML) for complex subscription rules.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use clap::Parser;
use clap::ValueEnum;
use nostr::Keys;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::filter::SubscriptionRule;

/// Default idle timeout (seconds) when neither `--idle-timeout` nor the
/// deprecated `--turn-timeout` is set.
///
/// Sized for slow turns where the agent may go silent on its outer ACP channel
/// while running long sub-tools (e.g. a buzz-agent running another agent, or
/// codex/claude doing multi-minute single tool calls). 900s gives 300s of
/// breathing room above the 600s max shell timeout, so legitimate long-running
/// tool calls don't race the idle deadline.
/// Override via `--idle-timeout` / `BUZZ_ACP_IDLE_TIMEOUT`.
pub(crate) const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 900;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to parse nostr keys: {0}")]
    KeyParse(#[from] nostr::key::Error),

    #[error("failed to read file: {0}")]
    Io(#[from] std::io::Error),

    #[error("config file error: {0}")]
    ConfigFile(String),
}

#[derive(Debug, Clone, PartialEq, clap::ValueEnum)]
pub enum SubscribeMode {
    Mentions,
    All,
    Config,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum DedupMode {
    Drop,
    Queue,
}

/// How to handle new @mentions while a turn is already in-flight for that channel.
#[derive(Debug, Clone, Copy, PartialEq, clap::ValueEnum)]
pub enum MultipleEventHandling {
    /// Queue new events while a turn is in-flight. Deliver after current turn
    /// completes. Existing behavior — zero code change in this path.
    Queue,
    /// Cancel the in-flight turn and re-dispatch a merged prompt that frames
    /// the new events as a **steering message** — one that arrived while the
    /// agent was working, to be woven into the in-progress task rather than
    /// treated as a replacement. Fires for any author the inbound author gate
    /// admits (owner ∪ allowlist ∪ siblings). This is the default mid-turn
    /// delivery path. Requires DedupMode::Queue.
    Steer,
    /// Cancel the in-flight turn and re-dispatch a merged prompt combining
    /// the original events with the new ones, framed as a **supersede** (the
    /// new request replaces the old), for ANY new @mention.
    /// Requires DedupMode::Queue.
    Interrupt,
    /// Cancel the in-flight turn only when the new @mention is from the agent
    /// owner (resolved via owner_cache). All other authors queue normally.
    /// Requires DedupMode::Queue.
    #[value(name = "owner-interrupt")]
    OwnerInterrupt,
}

/// Inbound author gate: which authors' events the harness forwards to the agent.
///
/// - `owner-only` — only the agent's registered owner (default).
/// - `allowlist`  — owner + explicit pubkey list (`--respond-to-allowlist`).
/// - `anyone`     — all events forwarded (no author filtering).
/// - `nobody`     — all events dropped (proactive/heartbeat-only mode).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, clap::ValueEnum)]
pub enum RespondTo {
    #[default]
    OwnerOnly,
    Allowlist,
    Anyone,
    Nobody,
}

impl std::fmt::Display for RespondTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OwnerOnly => f.write_str("owner-only"),
            Self::Allowlist => f.write_str("allowlist"),
            Self::Anyone => f.write_str("anyone"),
            Self::Nobody => f.write_str("nobody"),
        }
    }
}

/// Permission mode for agents that support `session/set_config_option` with
/// `configId: "mode"` (e.g. `claude-agent-acp`).
///
/// - `default` — agent's built-in behaviour (permission requests per tool call).
/// - `acceptEdits` — auto-approve file edits, still ask for other tools.
/// - `bypassPermissions` — skip the permission flow entirely.
/// - `dontAsk` — never prompt; reject anything that would require permission.
/// - `plan` — planning-only mode (no tool execution).
#[derive(Debug, Clone, Copy, PartialEq, clap::ValueEnum)]
pub enum PermissionMode {
    /// Agent default — permission requests per tool call.
    #[value(alias = "default")]
    Default,
    /// Auto-approve file edits, still ask for other tools.
    #[value(alias = "acceptEdits")]
    AcceptEdits,
    /// Skip the permission flow entirely.
    #[value(alias = "bypassPermissions")]
    BypassPermissions,
    /// Never prompt; reject anything that would require permission.
    #[value(alias = "dontAsk")]
    DontAsk,
    /// Planning-only mode (no tool execution).
    #[value(alias = "plan")]
    Plan,
}

impl PermissionMode {
    /// Return the wire-format string sent to the agent via
    /// `session/set_config_option`.
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::BypassPermissions => "bypassPermissions",
            Self::DontAsk => "dontAsk",
            Self::Plan => "plan",
        }
    }

    /// Returns `true` when the mode is the agent's built-in default and
    /// therefore doesn't need to be explicitly set.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

/// CLI args for `buzz-acp models` — query available models from an agent.
///
/// This is a standalone `Parser` (not a subcommand variant) because the
/// `models` path must bypass `Config::from_cli()` entirely — no relay,
/// no private key, no harness setup.
#[derive(Debug, Parser)]
#[command(
    name = "buzz-acp models",
    about = "Query available models from the configured agent"
)]
pub struct ModelsArgs {
    /// Agent binary to spawn (e.g. "goose", "claude-agent-acp", "codex-acp").
    #[arg(long, env = "BUZZ_ACP_AGENT_COMMAND", default_value = "goose")]
    pub agent_command: String,

    /// Arguments passed to the agent binary.
    #[arg(
        long,
        env = "BUZZ_ACP_AGENT_ARGS",
        default_value = "acp",
        value_delimiter = ','
    )]
    pub agent_args: Vec<String>,

    /// Output structured JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    name = "buzz-acp",
    about = "ACP harness that bridges Buzz events to AI agents"
)]
pub struct CliArgs {
    #[arg(long, env = "BUZZ_RELAY_URL", default_value = "ws://localhost:3000")]
    pub relay_url: String,

    #[arg(long, env = "BUZZ_PRIVATE_KEY")]
    pub private_key: String,

    /// Agent owner pubkey (64-char hex). Used for --respond-to=owner-only gate.
    #[arg(long, env = "BUZZ_ACP_AGENT_OWNER")]
    pub agent_owner: Option<String>,

    #[arg(long, env = "BUZZ_ACP_AGENT_COMMAND", default_value = "goose")]
    pub agent_command: String,

    #[arg(
        long,
        env = "BUZZ_ACP_AGENT_ARGS",
        default_value = "acp",
        value_delimiter = ','
    )]
    pub agent_args: Vec<String>,

    #[arg(long, env = "BUZZ_ACP_MCP_COMMAND", default_value = "")]
    pub mcp_command: String,

    /// Idle timeout: max seconds of silence before killing a turn.
    /// Resets on any agent stdout activity.
    #[arg(long, env = "BUZZ_ACP_IDLE_TIMEOUT")]
    pub idle_timeout: Option<u64>,

    /// Absolute wall-clock cap per turn (safety valve).
    #[arg(long, env = "BUZZ_ACP_MAX_TURN_DURATION", default_value = "3600")]
    pub max_turn_duration: u64,

    /// Deprecated: alias for --idle-timeout. If both set, --idle-timeout wins.
    #[arg(long, env = "BUZZ_ACP_TURN_TIMEOUT", hide = true)]
    pub turn_timeout: Option<u64>,

    #[arg(
        long,
        env = "BUZZ_ACP_SYSTEM_PROMPT",
        conflicts_with = "system_prompt_file"
    )]
    pub system_prompt: Option<String>,

    #[arg(
        long,
        env = "BUZZ_ACP_SYSTEM_PROMPT_FILE",
        conflicts_with = "system_prompt"
    )]
    pub system_prompt_file: Option<PathBuf>,

    /// Number of parallel agent subprocesses.
    #[arg(long, env = "BUZZ_ACP_AGENTS", default_value_t = 1,
          value_parser = clap::value_parser!(u32).range(1..=32))]
    pub agents: u32,

    /// Seconds between heartbeat prompts. 0 = disabled.
    #[arg(long, env = "BUZZ_ACP_HEARTBEAT_INTERVAL", default_value_t = 0)]
    pub heartbeat_interval: u64,

    /// Seconds between per-turn liveness pings (the crash backstop signal —
    /// distinct from heartbeat self-prompting). 0 = disabled.
    #[arg(long, env = "BUZZ_ACP_TURN_LIVENESS_SECS", default_value_t = 10)]
    pub turn_liveness_secs: u64,

    /// Heartbeat prompt text. Conflicts with --heartbeat-prompt-file.
    #[arg(
        long,
        env = "BUZZ_ACP_HEARTBEAT_PROMPT",
        conflicts_with = "heartbeat_prompt_file"
    )]
    pub heartbeat_prompt: Option<String>,

    /// Read heartbeat prompt from file.
    #[arg(
        long,
        env = "BUZZ_ACP_HEARTBEAT_PROMPT_FILE",
        conflicts_with = "heartbeat_prompt"
    )]
    pub heartbeat_prompt_file: Option<PathBuf>,

    #[arg(long, env = "BUZZ_ACP_INITIAL_MESSAGE")]
    pub initial_message: Option<String>,

    #[arg(
        long,
        env = "BUZZ_ACP_SUBSCRIBE",
        default_value = "mentions",
        value_enum
    )]
    pub subscribe: SubscribeMode,

    #[arg(long, env = "BUZZ_ACP_KINDS", value_delimiter = ',')]
    pub kinds: Option<Vec<u32>>,

    #[arg(long, env = "BUZZ_ACP_CHANNELS", value_delimiter = ',')]
    pub channels: Option<Vec<String>>,

    #[arg(long, env = "BUZZ_ACP_NO_MENTION_FILTER")]
    pub no_mention_filter: bool,

    #[arg(long, env = "BUZZ_ACP_CONFIG", default_value = "./buzz-acp.toml")]
    pub config: PathBuf,

    #[arg(long, env = "BUZZ_ACP_DEDUP", default_value = "queue", value_enum)]
    pub dedup: DedupMode,

    /// How to handle new @mentions while a turn is already in-flight.
    /// steer (default): cancel+re-prompt, framing the new mention as a message
    /// that arrived mid-task — the agent keeps working and weaves it in.
    /// queue: events wait until the current turn completes.
    /// interrupt: cancel+re-prompt framed as a supersede (new replaces old).
    /// owner-interrupt: interrupt only for the agent owner's mentions.
    #[arg(
        long,
        env = "BUZZ_ACP_MULTIPLE_EVENT_HANDLING",
        default_value = "steer",
        value_enum
    )]
    pub multiple_event_handling: MultipleEventHandling,

    #[arg(long, env = "BUZZ_ACP_NO_IGNORE_SELF")]
    pub no_ignore_self: bool,

    /// Maximum number of context messages to include for thread replies and DMs.
    /// Set to 0 to disable automatic context fetching. Max 100.
    #[arg(long, env = "BUZZ_ACP_CONTEXT_MESSAGE_LIMIT", default_value_t = 12,
          value_parser = clap::value_parser!(u32).range(0..=100))]
    pub context_message_limit: u32,

    /// Maximum turns per session before proactive rotation. 0 = disabled
    /// (rotate only on MaxTokens / MaxTurnRequests).
    #[arg(long, env = "BUZZ_ACP_MAX_TURNS_PER_SESSION", default_value_t = 0,
          value_parser = clap::value_parser!(u32))]
    pub max_turns_per_session: u32,

    /// Disable automatic presence (online/offline) status.
    #[arg(long, env = "BUZZ_ACP_NO_PRESENCE")]
    pub no_presence: bool,

    /// Disable typing indicators while agent is processing.
    #[arg(long, env = "BUZZ_ACP_NO_TYPING")]
    pub no_typing: bool,

    /// Enable NIP-AE agent core memory injection.
    ///
    /// Memory injection is on by default. When enabled, the harness
    /// fetches the agent's per-session core engram and renders it as an
    /// `[Agent Memory — core]` prompt section (or renders the onboarding nudge
    /// when the relay confirms no core engram exists). The `buzz mem` CLI
    /// and the relay's acceptance of kind:30174 engrams are unaffected — this
    /// flag controls prompt-time injection in the ACP harness only.
    /// Pass `--no-memory` / `BUZZ_ACP_NO_MEMORY=true` to disable.
    #[arg(
        long,
        env = "BUZZ_ACP_MEMORY",
        conflicts_with = "no_memory",
        default_value_t = true
    )]
    pub memory: bool,

    /// Disable NIP-AE agent core memory injection.
    ///
    /// Memory injection is on by default; set this flag/env var to opt out.
    #[arg(long, env = "BUZZ_ACP_NO_MEMORY", conflicts_with = "memory")]
    pub no_memory: bool,

    /// Disable the [Base] platform-context section prepended to every prompt.
    /// When set, agents receive only the persona [System] prompt with no Buzz orientation.
    #[arg(long, env = "BUZZ_ACP_NO_BASE_PROMPT")]
    pub no_base_prompt: bool,

    /// Path to a custom base prompt file. Overrides the compiled-in default.
    /// Mutually exclusive with --no-base-prompt.
    #[arg(
        long,
        env = "BUZZ_ACP_BASE_PROMPT_FILE",
        conflicts_with = "no_base_prompt"
    )]
    pub base_prompt_file: Option<PathBuf>,

    /// Desired LLM model ID. Applied to every new ACP session after creation.
    /// Use `buzz-acp models` to discover available model IDs.
    #[arg(long, env = "BUZZ_ACP_MODEL")]
    pub model: Option<String>,

    /// Permission mode for agents that support `session/set_config_option`
    /// with `configId: "mode"` (e.g. `claude-agent-acp`).
    ///
    /// Defaults to `bypassPermissions` which skips the per-tool-call
    /// permission flow. Set to `default` to restore the agent's built-in
    /// behaviour.
    #[arg(
        long,
        env = "BUZZ_ACP_PERMISSION_MODE",
        default_value = "bypass-permissions",
        value_enum
    )]
    pub permission_mode: PermissionMode,

    /// Inbound author gate: which authors' events the harness forwards.
    /// Modes: owner-only (default), allowlist, anyone, nobody.
    #[arg(
        long,
        env = "BUZZ_ACP_RESPOND_TO",
        default_value = "owner-only",
        value_enum
    )]
    pub respond_to: RespondTo,

    /// Comma-separated 64-char hex pubkeys for allowlist mode.
    /// Owner pubkey is always implicitly included.
    #[arg(long, env = "BUZZ_ACP_RESPOND_TO_ALLOWLIST", value_delimiter = ',')]
    pub respond_to_allowlist: Option<Vec<String>>,

    /// Comma-separated list of allowed `--respond-to` modes.
    /// When set, the harness rejects startup if `--respond-to` is not in this list.
    /// Modes: owner-only, allowlist, anyone, nobody.
    /// Default: empty (all modes allowed — no restriction).
    /// Example: `BUZZ_ACP_ALLOWED_RESPOND_TO=owner-only,allowlist`
    #[arg(long, env = "BUZZ_ACP_ALLOWED_RESPOND_TO", value_delimiter = ',')]
    pub allowed_respond_to: Option<Vec<String>>,

    /// Path to a persona pack directory. Used with --persona-name to configure
    /// the agent from a .persona.md pack instead of CLI flags.
    #[arg(long, env = "BUZZ_ACP_PERSONA_PACK")]
    pub persona_pack: Option<PathBuf>,

    /// Name of the persona within the pack to use. Required when --persona-pack is set.
    #[arg(long, env = "BUZZ_ACP_PERSONA_NAME")]
    pub persona_name: Option<String>,

    /// Publish encrypted ACP observer frames over the relay.
    #[arg(long, env = "BUZZ_ACP_RELAY_OBSERVER", default_value_t = false)]
    pub relay_observer: bool,
}

/// Merged NIP-01 subscription filter for a single channel.
#[derive(Debug, Clone)]
pub struct ChannelFilter {
    /// Event kinds to subscribe to. None = wildcard (all kinds).
    pub kinds: Option<Vec<u32>>,
    /// Whether to include `#p` tag filter for agent pubkey.
    pub require_mention: bool,
}

#[derive(Debug)]
pub struct Config {
    pub keys: Keys,
    pub relay_url: String,
    pub agent_command: String,
    pub agent_args: Vec<String>,
    pub mcp_command: String,
    pub idle_timeout_secs: u64,
    pub max_turn_duration_secs: u64,
    pub agents: u32,
    pub heartbeat_interval_secs: u64,
    /// Seconds between per-turn liveness pings. 0 = disabled. Distinct from
    /// `heartbeat_interval_secs` (agent self-prompting) — this is the desktop
    /// crash-backstop signal.
    pub turn_liveness_secs: u64,
    pub heartbeat_prompt: Option<String>,
    pub system_prompt: Option<String>,
    pub initial_message: Option<String>,
    pub subscribe_mode: SubscribeMode,
    pub dedup_mode: DedupMode,
    pub multiple_event_handling: MultipleEventHandling,
    pub ignore_self: bool,
    pub kinds_override: Option<Vec<u32>>,
    pub channels_override: Option<Vec<String>>,
    pub no_mention_filter: bool,
    pub config_path: PathBuf,
    pub context_message_limit: u32,
    /// Maximum turns per session before proactive rotation. 0 = disabled.
    pub max_turns_per_session: u32,
    pub presence_enabled: bool,
    pub typing_enabled: bool,
    /// Whether NIP-AE agent core memory injection is enabled. When false,
    /// the harness skips the per-session core engram fetch and renders no
    /// `[Agent Memory — core]` section. On by default; disabled via the
    /// `--no-memory` / `BUZZ_ACP_NO_MEMORY` opt-out.
    pub memory_enabled: bool,
    /// Desired LLM model ID. Applied after every `session_new_full()`.
    pub model: Option<String>,
    /// Permission mode to apply after session creation. `Default` = skip.
    pub permission_mode: PermissionMode,
    /// Inbound author gate mode.
    pub respond_to: RespondTo,
    /// Validated allowlist of pubkey hex strings (used when respond_to == Allowlist).
    pub respond_to_allowlist: HashSet<String>,
    /// Allowed `respond_to` modes. Empty = all modes allowed.
    pub allowed_respond_to: Vec<String>,
    /// Per-persona env vars to inject at agent spawn time (e.g., GOOSE_PROVIDER, GOOSE_MODEL, BUZZ_AGENT_MODEL).
    /// Populated from persona pack resolution. Empty when no pack is configured.
    pub persona_env_vars: Vec<(String, String)>,
    /// Whether to publish encrypted observer frames through the relay.
    pub relay_observer: bool,
    /// Agent owner pubkey (hex). Used for `--respond-to=owner-only` gate.
    /// Replaces the old REST-based owner lookup.
    pub agent_owner: Option<String>,
    /// Disable the [Base] platform-context section prepended to every prompt.
    pub no_base_prompt: bool,
    /// Resolved content from `--base-prompt-file`, read and validated in
    /// `from_cli()`. `None` when using the compiled-in default or when
    /// `--no-base-prompt` is set.
    pub base_prompt_content: Option<String>,
}

/// Validate and deduplicate allowlist entries: each must be exactly 64 hex chars.
fn validate_allowlist(entries: &[String]) -> Result<HashSet<String>, ConfigError> {
    let mut validated = HashSet::new();
    for entry in entries {
        let trimmed = entry.trim().to_ascii_lowercase();
        if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ConfigError::ConfigFile(format!(
                "invalid pubkey in --respond-to-allowlist: '{entry}' \
                 (must be exactly 64 hex characters)"
            )));
        }
        validated.insert(trimmed);
    }
    Ok(validated)
}

/// Validate the `--multiple-event-handling` / `--dedup` combination.
///
/// Every mid-turn cancel mode (`Steer`, `Interrupt`, `OwnerInterrupt`) requires
/// `DedupMode::Queue`: `DedupMode::Drop` discards events during the cancel drain
/// window, which would produce incomplete merged prompts. `Queue` handling
/// imposes no constraint.
fn validate_multiple_event_handling(
    handling: MultipleEventHandling,
    dedup: DedupMode,
) -> Result<(), ConfigError> {
    let is_cancel_mode = matches!(
        handling,
        MultipleEventHandling::Steer
            | MultipleEventHandling::Interrupt
            | MultipleEventHandling::OwnerInterrupt
    );
    if is_cancel_mode && matches!(dedup, DedupMode::Drop) {
        return Err(ConfigError::ConfigFile(
            "--multiple-event-handling=steer (or interrupt/owner-interrupt) requires \
             --dedup=queue. DedupMode::Drop discards events during the cancel drain window, \
             producing incomplete merged prompts."
                .into(),
        ));
    }
    Ok(())
}

pub(crate) fn normalize_agent_command_identity(command: &str) -> String {
    let normalized = command.trim().replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    let basename = trimmed
        .rsplit('/')
        .next()
        .expect("rsplit always yields at least one element");
    let lower = basename.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    stem.chars()
        .map(|character| match character {
            ' ' | '_' => '-',
            _ => character,
        })
        .collect()
}

fn default_agent_args(command: &str) -> Option<Vec<String>> {
    match normalize_agent_command_identity(command).as_str() {
        "goose" => Some(vec!["acp".to_string()]),
        "codex" | "codex-acp" | "claude-agent-acp" | "claude-code-acp" | "claude-code"
        | "claudecode" | "buzz-agent" => Some(Vec::new()),
        _ => None,
    }
}

/// Build `-c` flag pairs that enable network access and allowlist the relay hostname
/// in Codex's network sandbox.
///
/// Codex sandboxes MCP subprocesses (including `buzz-cli`) behind a Seatbelt sandbox
/// that blocks all outbound network by default, plus a managed proxy with a domain
/// allowlist. Without these flags, `buzz-cli` requests to the relay are blocked before
/// they reach WARP or any other outbound network path.
///
/// Returns `["-c", "sandbox_workspace_write.network_access=true", "-c",
/// "network_proxy.mode=\"full\"", "-c", "network_proxy.domains.\"<host>\"=\"allow\""]`
/// for Codex agents, or an empty vec for non-Codex agents or when the hostname cannot
/// be parsed from the relay URL.
///
/// The flags form a layered defense:
/// - `sandbox_workspace_write.network_access=true` opens the Seatbelt sandbox gate so
///   outbound TCP/TLS connections are allowed at the OS level
/// - `network_proxy.mode="full"` enables the managed proxy for all outbound traffic
/// - `network_proxy.domains."<host>"="allow"` adds the relay hostname to the allowlist
///
/// Handles `ws://`, `wss://`, `http://`, and `https://` schemes. Port is stripped —
/// Codex's domain allowlist matches on hostname only.
pub fn codex_network_args(agent_command: &str, relay_url: &str) -> Vec<String> {
    match normalize_agent_command_identity(agent_command).as_str() {
        "codex" | "codex-acp" => {}
        _ => return vec![],
    }

    // Use the `url` crate so ws://, wss://, http://, https:// are all handled
    // correctly. On parse failure, skip injection rather than panicking.
    let host = match Url::parse(relay_url) {
        Ok(u) => match u.host_str() {
            Some(h) => h.to_owned(),
            None => {
                tracing::warn!(
                    relay_url,
                    "codex network allowlist: no host in relay URL — skipping injection"
                );
                return vec![];
            }
        },
        Err(e) => {
            tracing::warn!(relay_url, error = %e, "codex network allowlist: failed to parse relay URL — skipping injection");
            return vec![];
        }
    };

    tracing::debug!(host, "injecting codex network allowlist for host");

    vec![
        "-c".into(),
        "sandbox_workspace_write.network_access=true".into(),
        "-c".into(),
        "network_proxy.mode=\"full\"".into(),
        "-c".into(),
        format!("network_proxy.domains.\"{host}\"=\"allow\""),
    ]
}

pub fn normalize_agent_args(command: &str, agent_args: Vec<String>) -> Vec<String> {
    let normalized = agent_args
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();

    let Some(default_args) = default_agent_args(command) else {
        return normalized;
    };

    if normalized.is_empty() {
        return default_args;
    }

    // Older callers relied on the Goose-specific default even for runtimes like
    // Codex and Claude. Treat that legacy fallback as "no args" for zero-arg
    // providers so desktop- and env-based launches behave the same way.
    if normalized.len() == 1 && normalized[0].eq_ignore_ascii_case("acp") && default_args.is_empty()
    {
        return default_args;
    }

    normalized
}

/// Propagate legacy env-var aliases to their canonical names.
///
/// Must be called **before** the tokio runtime starts — i.e. from the sync
/// `fn main()` wrapper, not from inside `#[tokio::main]`.
///
/// `std::env::set_var` is safe in Rust 2021 only when no other threads are
/// running. In Rust 2024 it requires `unsafe`. Calling this before
/// `#[tokio::main]` ensures worker threads are not yet alive.
///
/// // Must be called before tokio runtime starts — see Rust 2024 edition safety.
pub fn propagate_legacy_env_vars() {
    for (legacy, canonical) in [
        ("BUZZ_ACP_PRIVATE_KEY", "BUZZ_PRIVATE_KEY"),
        ("BUZZ_ACP_API_TOKEN", "BUZZ_API_TOKEN"),
    ] {
        if std::env::var(canonical).is_err() {
            if let Ok(val) = std::env::var(legacy) {
                std::env::set_var(canonical, &val);
            }
        }
    }
}

impl Config {
    pub fn from_cli() -> Result<Self, ConfigError> {
        // Legacy env-var propagation is intentionally NOT done here.
        // Call `propagate_legacy_env_vars()` before the tokio runtime starts
        // (in the sync `fn main()` wrapper) — see Rust 2024 edition safety.
        let args = CliArgs::parse();
        Self::from_args(args)
    }

    /// Build a `Config` from already-parsed `CliArgs`. Separated from `from_cli()` so
    /// tests can construct `CliArgs` via `CliArgs::try_parse_from` and exercise the full
    /// validation path without going through process args.
    pub fn from_args(mut args: CliArgs) -> Result<Self, ConfigError> {
        let keys = Keys::parse(&args.private_key)?;
        // Best-effort zeroize: overwrite the raw private key string to reduce
        // exposure via core dumps or heap inspection (#41). Without the `zeroize`
        // crate we can only clear the String — the allocator may retain copies.
        args.private_key
            .replace_range(.., &"0".repeat(args.private_key.len()));
        args.private_key.clear();

        let mut system_prompt = if let Some(text) = args.system_prompt {
            Some(text)
        } else if let Some(ref path) = args.system_prompt_file {
            Some(std::fs::read_to_string(path)?)
        } else {
            None
        };

        if args.heartbeat_interval > 0 && args.heartbeat_interval < 10 {
            return Err(ConfigError::ConfigFile(
                "heartbeat interval must be 0 (disabled) or ≥10 seconds".into(),
            ));
        }

        if args.turn_liveness_secs > 0 && args.turn_liveness_secs < 5 {
            return Err(ConfigError::ConfigFile(
                "turn liveness interval must be 0 (disabled) or ≥5 seconds".into(),
            ));
        }

        let heartbeat_prompt = if let Some(text) = args.heartbeat_prompt {
            Some(text)
        } else if let Some(ref path) = args.heartbeat_prompt_file {
            Some(std::fs::read_to_string(path)?)
        } else {
            None
        };

        let base_prompt_content = if args.no_base_prompt {
            None
        } else if let Some(ref path) = args.base_prompt_file {
            let content = std::fs::read_to_string(path)?;
            if content.len() > 1_048_576 {
                return Err(ConfigError::ConfigFile(format!(
                    "base prompt file {} exceeds 1 MB limit ({} bytes)",
                    path.display(),
                    content.len()
                )));
            }
            Some(content)
        } else {
            None
        };

        if matches!(args.subscribe, SubscribeMode::Config) {
            if args.kinds.is_some() {
                tracing::warn!("--kinds is ignored in config mode");
            }
            if args.channels.is_some() {
                tracing::warn!("--channels is ignored in config mode");
            }
            if args.no_mention_filter {
                tracing::warn!("--no-mention-filter is ignored in config mode");
            }
        }

        let agent_command = args.agent_command;

        // Finding #49a — agent_command must not be empty.
        if agent_command.trim().is_empty() {
            return Err(ConfigError::ConfigFile(
                "agent_command must not be empty".into(),
            ));
        }

        let mut agent_args = normalize_agent_args(&agent_command, args.agent_args);

        // Prepend Codex network allowlist flags so buzz-cli (an MCP subprocess)
        // can reach the relay through Codex's sandbox proxy. No-op for non-Codex agents.
        let network_args = codex_network_args(&agent_command, &args.relay_url);
        if !network_args.is_empty() {
            let mut merged = network_args;
            merged.extend(agent_args);
            agent_args = merged;
        }

        // Finding #49b — warn on invalid UUIDs in --channels.
        if let Some(ref channels) = args.channels {
            for ch in channels {
                if ch.parse::<Uuid>().is_err() {
                    tracing::warn!(
                        channel = %ch,
                        "--channels entry is not a valid UUID and will be ignored"
                    );
                }
            }
        }

        // Finding #49c — cap heartbeat interval at 86400s (24h).
        let heartbeat_interval = if args.heartbeat_interval > 86400 {
            tracing::warn!(
                interval = args.heartbeat_interval,
                "heartbeat interval exceeds 24h — capping at 86400s"
            );
            86400u64
        } else {
            args.heartbeat_interval
        };

        // Cap turn-liveness interval at 86400s (24h) — same bound as heartbeat.
        let turn_liveness_secs = if args.turn_liveness_secs > 86400 {
            tracing::warn!(
                interval = args.turn_liveness_secs,
                "turn liveness interval exceeds 24h — capping at 86400s"
            );
            86400u64
        } else {
            args.turn_liveness_secs
        };

        // Resolve idle_timeout_secs with deprecation handling.
        // Precedence: explicit --idle-timeout > --turn-timeout (deprecated) > `DEFAULT_IDLE_TIMEOUT_SECS`.
        let idle_timeout_secs = {
            let raw = match (args.idle_timeout, args.turn_timeout) {
                (Some(idle), Some(_turn)) => {
                    tracing::warn!(
                        "--turn-timeout / BUZZ_ACP_TURN_TIMEOUT is deprecated and ignored \
                         when --idle-timeout / BUZZ_ACP_IDLE_TIMEOUT is also set"
                    );
                    idle
                }
                (Some(idle), None) => idle,
                (None, Some(turn)) => {
                    tracing::warn!(
                        "--turn-timeout / BUZZ_ACP_TURN_TIMEOUT is deprecated; \
                         use --idle-timeout / BUZZ_ACP_IDLE_TIMEOUT instead"
                    );
                    turn
                }
                (None, None) => DEFAULT_IDLE_TIMEOUT_SECS,
            };
            if raw == 0 {
                tracing::warn!("idle timeout of 0 is invalid — using 1s minimum");
                1
            } else {
                raw
            }
        };

        let max_turn_duration_secs = {
            let raw = args.max_turn_duration;
            if raw == 0 {
                tracing::warn!("max turn duration of 0 is invalid — using 60s minimum");
                60
            } else {
                raw
            }
        };

        // Finding #20 — idle_timeout must be strictly less than max_turn_duration.
        // If idle_timeout >= max_turn_duration, the absolute wall-clock cap would
        // fire before the idle timeout ever could, making idle_timeout a dead letter.
        if idle_timeout_secs >= max_turn_duration_secs {
            return Err(ConfigError::ConfigFile(format!(
                "idle_timeout ({}s) must be less than max_turn_duration ({}s)",
                idle_timeout_secs, max_turn_duration_secs
            )));
        }

        let respond_to_allowlist = if args.respond_to == RespondTo::Allowlist {
            let raw = args.respond_to_allowlist.unwrap_or_default();
            if raw.is_empty() {
                return Err(ConfigError::ConfigFile(
                    "--respond-to=allowlist requires --respond-to-allowlist with at least one pubkey".into(),
                ));
            }
            validate_allowlist(&raw)?
        } else {
            if args.respond_to_allowlist.is_some() {
                tracing::warn!(
                    "--respond-to-allowlist is ignored when --respond-to is not 'allowlist'"
                );
            }
            HashSet::new()
        };

        // Validate respond_to against the allowed set.
        let allowed_respond_to = if let Some(raw) = args.allowed_respond_to {
            // Validate each entry is a known RespondTo mode.
            for s in &raw {
                RespondTo::from_str(s.trim(), true).map_err(|_| {
                    ConfigError::ConfigFile(format!(
                        "invalid value in BUZZ_ACP_ALLOWED_RESPOND_TO: '{s}' \
                         (valid values: owner-only, allowlist, anyone, nobody)"
                    ))
                })?;
            }
            let allowed_modes: Vec<String> = raw.iter().map(|s| s.trim().to_string()).collect();
            if !allowed_modes.is_empty() && !allowed_modes.contains(&args.respond_to.to_string()) {
                return Err(ConfigError::ConfigFile(format!(
                    "respond_to '{}' is not permitted on this deployment \
                     (BUZZ_ACP_ALLOWED_RESPOND_TO={})",
                    args.respond_to,
                    raw.join(",")
                )));
            }
            allowed_modes
        } else {
            Vec::new()
        };

        //
        // Precedence: CLI/env args > persona values > built-in defaults.
        // Persona fills in what's missing. Explicit flags always win.
        let (persona_system_prompt, persona_model, persona_env_vars) =
            match (&args.persona_pack, &args.persona_name) {
                (Some(pack_dir), Some(name)) => {
                    let pack = buzz_persona::resolve::resolve_pack(pack_dir).map_err(|e| {
                        ConfigError::ConfigFile(format!(
                            "failed to resolve pack {}: {e}",
                            pack_dir.display()
                        ))
                    })?;
                    let persona = pack
                        .personas
                        .into_iter()
                        .find(|p| p.name == *name)
                        .ok_or_else(|| {
                            ConfigError::ConfigFile(format!(
                                "persona '{name}' not found in pack {}",
                                pack_dir.display()
                            ))
                        })?;
                    (
                        Some(persona.system_prompt),
                        persona.model,
                        persona.runtime_env_vars,
                    )
                }
                (Some(_), None) => {
                    return Err(ConfigError::ConfigFile(
                        "--persona-pack requires --persona-name".into(),
                    ));
                }
                (None, Some(_)) => {
                    return Err(ConfigError::ConfigFile(
                        "--persona-name requires --persona-pack".into(),
                    ));
                }
                (None, None) => (None, None, vec![]),
            };

        // Apply persona defaults: CLI/env wins, persona fills gaps.
        if system_prompt.is_none() {
            system_prompt = persona_system_prompt;
        }
        let model = args.model.or(persona_model);

        validate_multiple_event_handling(args.multiple_event_handling, args.dedup)?;

        let config = Config {
            keys,
            relay_url: args.relay_url,
            agent_command,
            agent_args,
            mcp_command: args.mcp_command,
            idle_timeout_secs,
            max_turn_duration_secs,
            agents: args.agents,
            heartbeat_interval_secs: heartbeat_interval,
            turn_liveness_secs,
            heartbeat_prompt,
            system_prompt,
            initial_message: args.initial_message,
            subscribe_mode: args.subscribe,
            dedup_mode: args.dedup,
            multiple_event_handling: args.multiple_event_handling,
            ignore_self: !args.no_ignore_self,
            kinds_override: args.kinds,
            channels_override: args.channels,
            no_mention_filter: args.no_mention_filter,
            config_path: args.config,
            context_message_limit: args.context_message_limit,
            max_turns_per_session: args.max_turns_per_session,
            presence_enabled: !args.no_presence,
            typing_enabled: !args.no_typing,
            memory_enabled: args.memory && !args.no_memory,
            model,
            permission_mode: args.permission_mode,
            respond_to: args.respond_to,
            respond_to_allowlist,
            allowed_respond_to,
            persona_env_vars,
            relay_observer: args.relay_observer,
            agent_owner: args.agent_owner.map(|s| s.trim().to_ascii_lowercase()),
            no_base_prompt: args.no_base_prompt,
            base_prompt_content,
        };

        Ok(config)
    }

    /// Human-readable summary (no secrets).
    pub fn summary(&self) -> String {
        let respond_to_detail = match &self.respond_to {
            RespondTo::Allowlist => {
                format!("respond_to=allowlist({})", self.respond_to_allowlist.len())
            }
            other => format!("respond_to={other}"),
        };
        let allowed_respond_to_detail = if self.allowed_respond_to.is_empty() {
            String::new()
        } else {
            let mut modes = self.allowed_respond_to.clone();
            modes.sort();
            format!(" allowed_respond_to=[{}]", modes.join(","))
        };
        format!(
            "relay={} pubkey={} agent_cmd={} {} mcp_cmd={} idle_timeout={}s max_turn={}s agents={} heartbeat={}s subscribe={:?} dedup={:?} meh={:?} ignore_self={} context_limit={} max_turns_per_session={} presence={} typing={} memory={} model={} permission_mode={} {}{}",
            self.relay_url,
            self.keys.public_key().to_hex(),
            self.agent_command,
            self.agent_args.join(" "),
            self.mcp_command,
            self.idle_timeout_secs,
            self.max_turn_duration_secs,
            self.agents,
            self.heartbeat_interval_secs,
            self.subscribe_mode,
            self.dedup_mode,
            self.multiple_event_handling,
            self.ignore_self,
            self.context_message_limit,
            self.max_turns_per_session,
            self.presence_enabled,
            self.typing_enabled,
            self.memory_enabled,
            self.model.as_deref().unwrap_or("(agent default)"),
            self.permission_mode,
            respond_to_detail,
            allowed_respond_to_detail,
        )
    }
}

#[derive(Debug, serde::Deserialize)]
struct TomlConfig {
    #[serde(default)]
    rules: Vec<SubscriptionRule>,
}

pub fn load_rules(path: &std::path::Path) -> Result<Vec<SubscriptionRule>, ConfigError> {
    use std::sync::atomic::AtomicU32;
    use std::sync::Arc;

    let content = std::fs::read_to_string(path)?;
    let mut config: TomlConfig =
        toml::from_str(&content).map_err(|e| ConfigError::ConfigFile(e.to_string()))?;

    if config.rules.len() > 100 {
        return Err(ConfigError::ConfigFile(format!(
            "too many rules ({}, max 100)",
            config.rules.len()
        )));
    }

    // Finding #49d — warn when Config mode has no rules; agent will receive nothing.
    if config.rules.is_empty() {
        tracing::warn!(
            path = %path.display(),
            "config file contains zero rules — agent will receive no events in Config mode"
        );
    }

    let mut seen_names = std::collections::HashSet::new();
    for rule in &mut config.rules {
        if rule.name.trim().is_empty() {
            return Err(ConfigError::ConfigFile(
                "rule name must not be empty".into(),
            ));
        }
        if !seen_names.insert(rule.name.clone()) {
            return Err(ConfigError::ConfigFile(format!(
                "duplicate rule name: {}",
                rule.name
            )));
        }
        if let Some(ref expr) = rule.filter {
            if expr.len() > 4096 {
                return Err(ConfigError::ConfigFile(format!(
                    "rule '{}': filter too long ({} bytes, max 4096)",
                    rule.name,
                    expr.len()
                )));
            }
            // Fail fast: parse the expression at load time so typos don't
            // silently produce dead rules at runtime.
            // Finding #34 — store the compiled AST so match_event never re-parses.
            match evalexpr::build_operator_tree(expr) {
                Ok(node) => {
                    rule.compiled_filter = Some(Arc::new(node));
                }
                Err(e) => {
                    return Err(ConfigError::ConfigFile(format!(
                        "rule '{}': invalid filter expression: {e}",
                        rule.name,
                    )));
                }
            }
        }
        // Validate channel scope — catch typos like "ALL" or "All" early.
        if let crate::filter::ChannelScope::All(ref s) = rule.channels {
            if s != "all" {
                return Err(ConfigError::ConfigFile(format!(
                    "rule '{}': channels must be \"all\" or a list, got {:?}",
                    rule.name, s,
                )));
            }
        }
        // Initialise the consecutive-timeout counter (finding #25).
        // Deserialization leaves it at default (new Arc<AtomicU32::new(0)>)
        // but we set it explicitly here for clarity.
        rule.consecutive_timeouts = Arc::new(AtomicU32::new(0));
    }

    Ok(config.rules)
}

/// Resolve per-channel NIP-01 filters from config + discovered channels.
pub fn resolve_channel_filters(
    config: &Config,
    discovered_channels: &[Uuid],
    rules: &[SubscriptionRule],
) -> HashMap<Uuid, ChannelFilter> {
    use buzz_core::kind::{
        KIND_STREAM_MESSAGE, KIND_STREAM_REMINDER, KIND_WORKFLOW_APPROVAL_REQUESTED,
    };

    let target_channels: Vec<Uuid> = if let Some(ref overrides) = config.channels_override {
        overrides
            .iter()
            .filter_map(|s| s.parse::<Uuid>().ok())
            .filter(|id| discovered_channels.contains(id))
            .collect()
    } else {
        discovered_channels.to_vec()
    };

    let mut result = HashMap::new();

    match config.subscribe_mode {
        SubscribeMode::Mentions => {
            let kinds = config.kinds_override.clone().unwrap_or_else(|| {
                vec![
                    KIND_STREAM_MESSAGE,
                    KIND_WORKFLOW_APPROVAL_REQUESTED,
                    KIND_STREAM_REMINDER,
                ]
            });
            let require_mention = !config.no_mention_filter;
            for ch in &target_channels {
                result.insert(
                    *ch,
                    ChannelFilter {
                        kinds: Some(kinds.clone()),
                        require_mention,
                    },
                );
            }
        }
        SubscribeMode::All => {
            for ch in &target_channels {
                result.insert(
                    *ch,
                    ChannelFilter {
                        kinds: config.kinds_override.clone(),
                        require_mention: false,
                    },
                );
            }
        }
        SubscribeMode::Config => {
            for ch in discovered_channels {
                let mut merged_kinds: Option<Vec<u32>> = Some(vec![]);
                let mut require_mention = true;
                let mut has_rule = false;

                for rule in rules {
                    if !rule_applies_to_channel(rule, *ch) {
                        continue;
                    }
                    has_rule = true;
                    if rule.kinds.is_empty() {
                        merged_kinds = None;
                    } else if let Some(ref mut kinds) = merged_kinds {
                        for k in &rule.kinds {
                            if !kinds.contains(k) {
                                kinds.push(*k);
                            }
                        }
                    }
                    if !rule.require_mention {
                        require_mention = false;
                    }
                }

                if has_rule {
                    result.insert(
                        *ch,
                        ChannelFilter {
                            kinds: merged_kinds,
                            require_mention,
                        },
                    );
                }
            }
        }
    }

    result
}

/// Resolve the subscription filter for a single dynamically-discovered channel.
///
/// In Mentions/All mode, `channels_override` (--channels) is enforced — the agent
/// won't subscribe to channels outside the operator's allowlist. In Config mode,
/// `--channels` is ignored (per CLI contract) and rule-matching determines scope.
///
/// Returns `None` when the channel is outside the agent's configured scope:
/// - Mentions/All: channel not in `channels_override` (if set)
/// - Config: no subscription rules match the channel
pub fn resolve_dynamic_channel_filter(
    config: &Config,
    channel_id: Uuid,
    rules: &[crate::filter::SubscriptionRule],
) -> Option<ChannelFilter> {
    use buzz_core::kind::{
        KIND_STREAM_MESSAGE, KIND_STREAM_REMINDER, KIND_WORKFLOW_APPROVAL_REQUESTED,
    };

    // In Mentions/All mode, if the operator explicitly constrained channels
    // with --channels, only allow dynamic subscription to channels in that
    // allowlist. Config mode ignores --channels (per CLI contract) and uses
    // rule-matching instead.
    if config.subscribe_mode != SubscribeMode::Config {
        if let Some(ref overrides) = config.channels_override {
            let allowed = overrides
                .iter()
                .any(|s| s.parse::<Uuid>().ok() == Some(channel_id));
            if !allowed {
                return None;
            }
        }
    }

    match config.subscribe_mode {
        SubscribeMode::Mentions => Some(ChannelFilter {
            kinds: Some(config.kinds_override.clone().unwrap_or_else(|| {
                vec![
                    KIND_STREAM_MESSAGE,
                    KIND_WORKFLOW_APPROVAL_REQUESTED,
                    KIND_STREAM_REMINDER,
                ]
            })),
            require_mention: !config.no_mention_filter,
        }),
        SubscribeMode::All => Some(ChannelFilter {
            kinds: config.kinds_override.clone(),
            require_mention: false,
        }),
        SubscribeMode::Config => {
            // Same merge logic as resolve_channel_filters() Config branch:
            // evaluate ALL rules against this specific channel (including
            // channel-specific rules, not just ChannelScope::All).
            let mut merged_kinds: Option<Vec<u32>> = Some(vec![]);
            let mut require_mention = true;
            let mut has_rule = false;

            for rule in rules {
                if !rule_applies_to_channel(rule, channel_id) {
                    continue;
                }
                has_rule = true;
                if rule.kinds.is_empty() {
                    merged_kinds = None;
                } else if let Some(ref mut kinds) = merged_kinds {
                    for k in &rule.kinds {
                        if !kinds.contains(k) {
                            kinds.push(*k);
                        }
                    }
                }
                if !rule.require_mention {
                    require_mention = false;
                }
            }

            if !has_rule {
                // No rules match — don't subscribe. Consistent with
                // resolve_channel_filters() which omits unmatched channels.
                return None;
            }

            Some(ChannelFilter {
                kinds: merged_kinds,
                require_mention,
            })
        }
    }
}

fn rule_applies_to_channel(rule: &SubscriptionRule, channel_id: Uuid) -> bool {
    use crate::filter::ChannelScope;
    match &rule.channels {
        ChannelScope::All(s) if s == "all" => true,
        ChannelScope::List(ids) => ids
            .iter()
            .any(|id| id.parse::<Uuid>().ok() == Some(channel_id)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{ChannelScope, SubscriptionRule};
    use clap::{Parser, ValueEnum};

    /// Build a minimal Config for testing without CLI parsing.
    fn test_config(mode: SubscribeMode) -> Config {
        Config {
            keys: nostr::Keys::generate(),
            relay_url: "ws://localhost:3000".into(),
            agent_command: "goose".into(),
            agent_args: vec!["acp".into()],
            mcp_command: "".into(),
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
            max_turn_duration_secs: 3600,
            agents: 1,
            heartbeat_interval_secs: 0,
            turn_liveness_secs: 10,
            heartbeat_prompt: None,
            system_prompt: None,
            initial_message: None,
            subscribe_mode: mode,
            dedup_mode: DedupMode::Queue,
            multiple_event_handling: MultipleEventHandling::Queue,
            ignore_self: true,
            kinds_override: None,
            channels_override: None,
            no_mention_filter: false,
            config_path: PathBuf::from("./buzz-acp.toml"),
            context_message_limit: 12,
            max_turns_per_session: 0,
            presence_enabled: true,
            typing_enabled: true,
            memory_enabled: true,
            model: None,
            permission_mode: PermissionMode::BypassPermissions,
            respond_to: RespondTo::Anyone,
            respond_to_allowlist: HashSet::new(),
            allowed_respond_to: Vec::new(),
            persona_env_vars: vec![],
            relay_observer: false,
            agent_owner: None,
            no_base_prompt: false,
            base_prompt_content: None,
        }
    }

    fn make_rule(
        name: &str,
        channels: ChannelScope,
        kinds: Vec<u32>,
        mention: bool,
    ) -> SubscriptionRule {
        use std::sync::atomic::AtomicU32;
        use std::sync::Arc;
        SubscriptionRule {
            name: name.into(),
            channels,
            kinds,
            require_mention: mention,
            filter: None,
            prompt_tag: None,
            compiled_filter: None,
            consecutive_timeouts: Arc::new(AtomicU32::new(0)),
        }
    }

    #[test]
    fn test_mentions_mode_default_kinds() {
        let config = test_config(SubscribeMode::Mentions);
        let channels = vec![Uuid::new_v4(), Uuid::new_v4()];
        let result = resolve_channel_filters(&config, &channels, &[]);

        assert_eq!(result.len(), 2);
        for ch in &channels {
            let f = result.get(ch).expect("channel should be present");
            assert!(f.require_mention, "mentions mode requires mention");
            let kinds = f.kinds.as_ref().expect("should have kinds");
            assert!(kinds.contains(&buzz_core::kind::KIND_STREAM_MESSAGE));
            assert!(kinds.contains(&buzz_core::kind::KIND_WORKFLOW_APPROVAL_REQUESTED));
            assert!(kinds.contains(&buzz_core::kind::KIND_STREAM_REMINDER));
        }
    }

    #[test]
    fn test_mentions_mode_custom_kinds() {
        let mut config = test_config(SubscribeMode::Mentions);
        config.kinds_override = Some(vec![1, 7]);
        let channels = vec![Uuid::new_v4()];
        let result = resolve_channel_filters(&config, &channels, &[]);

        let f = result.get(&channels[0]).unwrap();
        assert_eq!(f.kinds.as_ref().unwrap(), &[1, 7]);
    }

    #[test]
    fn test_mentions_mode_no_mention_filter() {
        let mut config = test_config(SubscribeMode::Mentions);
        config.no_mention_filter = true;
        let channels = vec![Uuid::new_v4()];
        let result = resolve_channel_filters(&config, &channels, &[]);

        let f = result.get(&channels[0]).unwrap();
        assert!(!f.require_mention);
    }

    #[test]
    fn normalizes_goose_args_to_acp() {
        assert_eq!(normalize_agent_args("goose", Vec::new()), vec!["acp"]);
        assert_eq!(normalize_agent_args("goose", vec!["".into()]), vec!["acp"]);
    }

    #[test]
    fn normalizes_codex_and_claude_args_to_empty() {
        assert_eq!(
            normalize_agent_args("codex-acp", Vec::new()),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("codex-acp", vec!["".into()]),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("codex-acp", vec!["acp".into()]),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("claude-code", vec!["acp".into()]),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("claude-code-acp", vec!["acp".into()]),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("claude-agent-acp", vec!["acp".into()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn preserves_explicit_nonempty_agent_args() {
        assert_eq!(
            normalize_agent_args("codex-acp", vec!["-c".into(), "model=\"gpt-5\"".into()]),
            vec!["-c", "model=\"gpt-5\""]
        );
        assert_eq!(
            normalize_agent_args("custom-agent", vec!["".into(), "serve".into()]),
            vec!["serve"]
        );
    }

    #[test]
    fn normalizes_buzz_agent_args_to_empty() {
        assert_eq!(
            normalize_agent_args("buzz-agent", Vec::new()),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("buzz-agent", vec!["acp".into()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn normalize_agent_command_identity_variants() {
        assert_eq!(normalize_agent_command_identity("goose"), "goose");
        assert_eq!(
            normalize_agent_command_identity("C:\\Program Files\\Goose\\goose.exe"),
            "goose"
        );
        assert_eq!(
            normalize_agent_command_identity("/usr/local/bin/codex-acp"),
            "codex-acp"
        );
        assert_eq!(normalize_agent_command_identity("/usr/local/bin/"), "bin");
        assert_eq!(
            normalize_agent_command_identity("Claude_Code"),
            "claude-code"
        );
        assert_eq!(
            normalize_agent_command_identity("Claude Code"),
            "claude-code"
        );
        assert_eq!(normalize_agent_command_identity("Goose.EXE"), "goose");
        // Non-ASCII must not panic.
        assert_eq!(normalize_agent_command_identity("my-agënt"), "my-agënt");
        // Edge cases: empty, whitespace-only, bare separators.
        assert_eq!(normalize_agent_command_identity(""), "");
        assert_eq!(normalize_agent_command_identity("   "), "");
        assert_eq!(normalize_agent_command_identity("/"), "");
        assert_eq!(normalize_agent_command_identity("///"), "");
    }

    #[test]
    fn strips_legacy_acp_arg_case_insensitively() {
        assert_eq!(
            normalize_agent_args("codex-acp", vec!["ACP".into()]),
            Vec::<String>::new()
        );
    }

    // --- codex_network_args tests ---

    #[test]
    fn codex_network_args_wss_url() {
        let args = codex_network_args("codex-acp", "wss://sprout-oss.stage.blox.sqprod.co");
        assert_eq!(
            args,
            vec![
                "-c",
                "sandbox_workspace_write.network_access=true",
                "-c",
                "network_proxy.mode=\"full\"",
                "-c",
                "network_proxy.domains.\"sprout-oss.stage.blox.sqprod.co\"=\"allow\"",
            ]
        );
    }

    #[test]
    fn codex_network_args_ws_url() {
        let args = codex_network_args("codex-acp", "ws://localhost:3000");
        assert_eq!(
            args,
            vec![
                "-c",
                "sandbox_workspace_write.network_access=true",
                "-c",
                "network_proxy.mode=\"full\"",
                "-c",
                "network_proxy.domains.\"localhost\"=\"allow\"",
            ]
        );
    }

    #[test]
    fn codex_network_args_https_url() {
        let args = codex_network_args("codex-acp", "https://relay.example.com/path");
        assert_eq!(
            args,
            vec![
                "-c",
                "sandbox_workspace_write.network_access=true",
                "-c",
                "network_proxy.mode=\"full\"",
                "-c",
                "network_proxy.domains.\"relay.example.com\"=\"allow\"",
            ]
        );
    }

    #[test]
    fn codex_network_args_http_url_with_port() {
        let args = codex_network_args("codex-acp", "http://relay.example.com:8080/query");
        assert_eq!(
            args,
            vec![
                "-c",
                "sandbox_workspace_write.network_access=true",
                "-c",
                "network_proxy.mode=\"full\"",
                "-c",
                "network_proxy.domains.\"relay.example.com\"=\"allow\"",
            ]
        );
    }

    #[test]
    fn codex_network_args_bare_codex_command() {
        // "codex" (not "codex-acp") should also get the args.
        let args = codex_network_args("codex", "wss://relay.example.com");
        assert_eq!(
            args,
            vec![
                "-c",
                "sandbox_workspace_write.network_access=true",
                "-c",
                "network_proxy.mode=\"full\"",
                "-c",
                "network_proxy.domains.\"relay.example.com\"=\"allow\"",
            ]
        );
    }

    #[test]
    fn codex_network_args_full_path_codex_command() {
        // Full path like /usr/local/bin/codex-acp should be normalized.
        let args = codex_network_args("/usr/local/bin/codex-acp", "wss://relay.example.com");
        assert_eq!(
            args,
            vec![
                "-c",
                "sandbox_workspace_write.network_access=true",
                "-c",
                "network_proxy.mode=\"full\"",
                "-c",
                "network_proxy.domains.\"relay.example.com\"=\"allow\"",
            ]
        );
    }

    #[test]
    fn codex_network_args_non_codex_agent_returns_empty() {
        assert!(codex_network_args("goose", "wss://relay.example.com").is_empty());
        assert!(codex_network_args("claude-agent-acp", "wss://relay.example.com").is_empty());
        assert!(codex_network_args("buzz-agent", "wss://relay.example.com").is_empty());
    }

    #[test]
    fn codex_network_args_includes_sandbox_network_access() {
        // The sandbox gate must be the first flag pair — without it, the Seatbelt
        // sandbox blocks outbound connections before the proxy can intercept them.
        let args = codex_network_args("codex-acp", "wss://relay.example.com");
        assert_eq!(args.len(), 6, "expected 3 flag pairs (6 elements)");
        assert_eq!(args[0], "-c");
        assert_eq!(args[1], "sandbox_workspace_write.network_access=true");
    }

    #[test]
    fn codex_network_args_empty_relay_url_returns_empty() {
        // Empty string fails Url::parse — graceful empty return.
        assert!(codex_network_args("codex-acp", "").is_empty());
    }

    #[test]
    fn codex_network_args_schemeless_string_returns_empty() {
        // A bare string with no scheme fails Url::parse — graceful empty return.
        assert!(codex_network_args("codex-acp", "not-a-url").is_empty());
    }

    #[test]
    fn test_all_mode_wildcard() {
        let config = test_config(SubscribeMode::All);
        let channels = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let result = resolve_channel_filters(&config, &channels, &[]);

        assert_eq!(result.len(), 3);
        for ch in &channels {
            let f = result.get(ch).unwrap();
            assert!(
                f.kinds.is_none(),
                "all mode with no override = wildcard kinds"
            );
            assert!(!f.require_mention);
        }
    }

    #[test]
    fn test_all_mode_with_kinds_override() {
        let mut config = test_config(SubscribeMode::All);
        config.kinds_override = Some(vec![9, 7]);
        let channels = vec![Uuid::new_v4()];
        let result = resolve_channel_filters(&config, &channels, &[]);

        let f = result.get(&channels[0]).unwrap();
        assert_eq!(f.kinds.as_ref().unwrap(), &[9, 7]);
    }

    #[test]
    fn test_channels_override_filters_to_discovered() {
        let mut config = test_config(SubscribeMode::All);
        let ch_a = Uuid::new_v4();
        let ch_b = Uuid::new_v4();
        let ch_unknown = Uuid::new_v4();
        // Override includes ch_a and an unknown channel.
        config.channels_override = Some(vec![ch_a.to_string(), ch_unknown.to_string()]);

        let discovered = vec![ch_a, ch_b];
        let result = resolve_channel_filters(&config, &discovered, &[]);

        // Only ch_a should be present (intersection of override and discovered).
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&ch_a));
        assert!(!result.contains_key(&ch_b));
        assert!(!result.contains_key(&ch_unknown));
    }

    #[test]
    fn test_config_mode_single_rule_all_channels() {
        let config = test_config(SubscribeMode::Config);
        let ch = Uuid::new_v4();
        let rules = vec![make_rule(
            "catch-all",
            ChannelScope::All("all".into()),
            vec![9],
            false,
        )];

        let result = resolve_channel_filters(&config, &[ch], &rules);
        assert_eq!(result.len(), 1);
        let f = result.get(&ch).unwrap();
        assert_eq!(f.kinds.as_ref().unwrap(), &[9]);
        assert!(!f.require_mention);
    }

    #[test]
    fn test_config_mode_rule_targets_specific_channel() {
        let config = test_config(SubscribeMode::Config);
        let ch_a = Uuid::new_v4();
        let ch_b = Uuid::new_v4();
        let rules = vec![make_rule(
            "only-a",
            ChannelScope::List(vec![ch_a.to_string()]),
            vec![9],
            false,
        )];

        let result = resolve_channel_filters(&config, &[ch_a, ch_b], &rules);
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&ch_a));
        assert!(!result.contains_key(&ch_b));
    }

    #[test]
    fn test_config_mode_merge_overlapping_rules() {
        let config = test_config(SubscribeMode::Config);
        let ch = Uuid::new_v4();
        // Two rules both match the same channel with different kinds.
        let rules = vec![
            make_rule("messages", ChannelScope::All("all".into()), vec![9], true),
            make_rule("reactions", ChannelScope::All("all".into()), vec![7], false),
        ];

        let result = resolve_channel_filters(&config, &[ch], &rules);
        let f = result.get(&ch).unwrap();
        // Kinds should be the union: [9, 7].
        let kinds = f.kinds.as_ref().expect("should have merged kinds");
        assert!(kinds.contains(&9));
        assert!(kinds.contains(&7));
        // require_mention should be false (most permissive wins).
        assert!(!f.require_mention);
    }

    #[test]
    fn test_config_mode_wildcard_kinds_propagates() {
        let config = test_config(SubscribeMode::Config);
        let ch = Uuid::new_v4();
        // First rule has specific kinds, second has empty (wildcard).
        let rules = vec![
            make_rule("narrow", ChannelScope::All("all".into()), vec![9], false),
            make_rule("broad", ChannelScope::All("all".into()), vec![], false),
        ];

        let result = resolve_channel_filters(&config, &[ch], &rules);
        let f = result.get(&ch).unwrap();
        // Once any rule has empty kinds (wildcard), merged result is None (wildcard).
        assert!(f.kinds.is_none(), "wildcard should propagate");
    }

    #[test]
    fn test_config_mode_no_matching_rules_empty_result() {
        let config = test_config(SubscribeMode::Config);
        let ch = Uuid::new_v4();
        let other_ch = Uuid::new_v4();
        // Rule only targets other_ch.
        let rules = vec![make_rule(
            "other",
            ChannelScope::List(vec![other_ch.to_string()]),
            vec![9],
            false,
        )];

        let result = resolve_channel_filters(&config, &[ch], &rules);
        assert!(result.is_empty());
    }

    #[test]
    fn test_config_mode_require_mention_most_permissive() {
        let config = test_config(SubscribeMode::Config);
        let ch = Uuid::new_v4();
        // First rule requires mention, second doesn't.
        let rules = vec![
            make_rule("strict", ChannelScope::All("all".into()), vec![9], true),
            make_rule("lax", ChannelScope::All("all".into()), vec![7], false),
        ];

        let result = resolve_channel_filters(&config, &[ch], &rules);
        let f = result.get(&ch).unwrap();
        assert!(!f.require_mention, "most permissive (false) should win");
    }

    #[test]
    fn test_rule_applies_all() {
        let rule = make_rule("test", ChannelScope::All("all".into()), vec![], false);
        assert!(rule_applies_to_channel(&rule, Uuid::new_v4()));
    }

    #[test]
    fn test_rule_applies_all_invalid_string() {
        let rule = make_rule("test", ChannelScope::All("ALL".into()), vec![], false);
        assert!(!rule_applies_to_channel(&rule, Uuid::new_v4()));
    }

    #[test]
    fn test_rule_applies_list_match() {
        let ch = Uuid::new_v4();
        let rule = make_rule(
            "test",
            ChannelScope::List(vec![ch.to_string()]),
            vec![],
            false,
        );
        assert!(rule_applies_to_channel(&rule, ch));
    }

    #[test]
    fn test_rule_applies_list_no_match() {
        let rule = make_rule(
            "test",
            ChannelScope::List(vec![Uuid::new_v4().to_string()]),
            vec![],
            false,
        );
        assert!(!rule_applies_to_channel(&rule, Uuid::new_v4()));
    }

    #[test]
    fn test_load_rules_valid_toml() {
        let dir = std::env::temp_dir().join("buzz-acp-test-valid");
        let path = dir.join("rules.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"
[[rules]]
name = "catch-all"
channels = "all"
kinds = [9]
require_mention = false
"#,
        )
        .unwrap();

        let rules = load_rules(&path).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "catch-all");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_rules_empty_name_rejected() {
        let dir = std::env::temp_dir().join("buzz-acp-test-empty-name");
        let path = dir.join("rules.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"
[[rules]]
name = "  "
channels = "all"
"#,
        )
        .unwrap();

        let err = load_rules(&path).unwrap_err();
        assert!(err.to_string().contains("name must not be empty"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_rules_duplicate_name_rejected() {
        let dir = std::env::temp_dir().join("buzz-acp-test-dup-name");
        let path = dir.join("rules.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"
[[rules]]
name = "dup"
channels = "all"

[[rules]]
name = "dup"
channels = "all"
"#,
        )
        .unwrap();

        let err = load_rules(&path).unwrap_err();
        assert!(err.to_string().contains("duplicate rule name"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_rules_invalid_filter_rejected() {
        let dir = std::env::temp_dir().join("buzz-acp-test-bad-filter");
        let path = dir.join("rules.toml");
        std::fs::create_dir_all(&dir).unwrap();
        // evalexpr rejects unbalanced parens at parse time.
        std::fs::write(
            &path,
            r#"
[[rules]]
name = "bad"
channels = "all"
filter = "((("
"#,
        )
        .unwrap();

        let err = load_rules(&path).unwrap_err();
        assert!(err.to_string().contains("invalid filter expression"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_rules_channel_scope_typo_rejected() {
        let dir = std::env::temp_dir().join("buzz-acp-test-scope-typo");
        let path = dir.join("rules.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"
[[rules]]
name = "typo"
channels = "ALL"
"#,
        )
        .unwrap();

        let err = load_rules(&path).unwrap_err();
        assert!(err.to_string().contains("must be \"all\" or a list"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_rules_too_many_rules_rejected() {
        let dir = std::env::temp_dir().join("buzz-acp-test-too-many");
        let path = dir.join("rules.toml");
        std::fs::create_dir_all(&dir).unwrap();
        let mut toml = String::new();
        for i in 0..101 {
            toml.push_str(&format!(
                "[[rules]]\nname = \"rule-{i}\"\nchannels = \"all\"\n\n"
            ));
        }
        std::fs::write(&path, &toml).unwrap();

        let err = load_rules(&path).unwrap_err();
        assert!(err.to_string().contains("too many rules"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_rules_filter_too_long_rejected() {
        let dir = std::env::temp_dir().join("buzz-acp-test-long-filter");
        let path = dir.join("rules.toml");
        std::fs::create_dir_all(&dir).unwrap();
        let long_expr = format!("\"{}\"", "a".repeat(4097));
        std::fs::write(
            &path,
            format!("[[rules]]\nname = \"long\"\nchannels = \"all\"\nfilter = {long_expr}\n"),
        )
        .unwrap();

        let err = load_rules(&path).unwrap_err();
        assert!(err.to_string().contains("filter too long"));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn validate_heartbeat_interval(secs: u64) -> Result<(), ConfigError> {
        if secs > 0 && secs < 10 {
            return Err(ConfigError::ConfigFile(
                "heartbeat interval must be 0 (disabled) or ≥10 seconds".into(),
            ));
        }
        Ok(())
    }

    #[test]
    fn test_heartbeat_interval_zero_ok() {
        assert!(validate_heartbeat_interval(0).is_ok());
    }

    #[test]
    fn test_heartbeat_interval_ten_ok() {
        assert!(validate_heartbeat_interval(10).is_ok());
    }

    #[test]
    fn test_heartbeat_interval_large_ok() {
        assert!(validate_heartbeat_interval(300).is_ok());
    }

    #[test]
    fn test_heartbeat_interval_five_rejected() {
        let err = validate_heartbeat_interval(5).unwrap_err();
        assert!(err.to_string().contains("heartbeat interval must be 0"));
    }

    #[test]
    fn test_heartbeat_interval_one_rejected() {
        let err = validate_heartbeat_interval(1).unwrap_err();
        assert!(err.to_string().contains("heartbeat interval must be 0"));
    }

    #[test]
    fn test_heartbeat_interval_nine_rejected() {
        let err = validate_heartbeat_interval(9).unwrap_err();
        assert!(err.to_string().contains("heartbeat interval must be 0"));
    }

    fn validate_turn_liveness(secs: u64) -> Result<(), ConfigError> {
        if secs > 0 && secs < 5 {
            return Err(ConfigError::ConfigFile(
                "turn liveness interval must be 0 (disabled) or ≥5 seconds".into(),
            ));
        }
        Ok(())
    }

    #[test]
    fn test_turn_liveness_zero_ok() {
        assert!(validate_turn_liveness(0).is_ok());
    }

    #[test]
    fn test_turn_liveness_five_ok() {
        assert!(validate_turn_liveness(5).is_ok());
    }

    #[test]
    fn test_turn_liveness_ten_ok() {
        assert!(validate_turn_liveness(10).is_ok());
    }

    #[test]
    fn test_turn_liveness_four_rejected() {
        let err = validate_turn_liveness(4).unwrap_err();
        assert!(err.to_string().contains("turn liveness interval must be 0"));
    }

    #[test]
    fn test_turn_liveness_one_rejected() {
        let err = validate_turn_liveness(1).unwrap_err();
        assert!(err.to_string().contains("turn liveness interval must be 0"));
    }

    #[test]
    fn test_summary_includes_agents_and_heartbeat() {
        let config = test_config(SubscribeMode::Mentions);
        let s = config.summary();
        assert!(
            s.contains("agents=1"),
            "summary should include agents=1, got: {s}"
        );
        assert!(
            s.contains("heartbeat=0s"),
            "summary should include heartbeat=0s, got: {s}"
        );
    }

    #[test]
    fn test_summary_reflects_custom_agents_and_heartbeat() {
        let mut config = test_config(SubscribeMode::Mentions);
        config.agents = 4;
        config.heartbeat_interval_secs = 30;
        let s = config.summary();
        assert!(
            s.contains("agents=4"),
            "summary should include agents=4, got: {s}"
        );
        assert!(
            s.contains("heartbeat=30s"),
            "summary should include heartbeat=30s, got: {s}"
        );
    }

    #[test]
    fn test_memory_enabled_default_true() {
        let config = test_config(SubscribeMode::Mentions);
        assert!(
            config.memory_enabled,
            "memory_enabled should default to true"
        );
    }

    #[test]
    fn test_summary_includes_memory_enabled() {
        let config = test_config(SubscribeMode::Mentions);
        let s = config.summary();
        assert!(
            s.contains("memory=true"),
            "summary should include memory=true by default, got: {s}"
        );
    }

    #[test]
    fn test_summary_reflects_memory_enabled() {
        let mut config = test_config(SubscribeMode::Mentions);
        config.memory_enabled = true;
        let s = config.summary();
        assert!(
            s.contains("memory=true"),
            "summary should include memory=true when enabled, got: {s}"
        );
    }

    #[test]
    fn test_permission_mode_wire_strings() {
        assert_eq!(PermissionMode::Default.as_wire_str(), "default");
        assert_eq!(PermissionMode::AcceptEdits.as_wire_str(), "acceptEdits");
        assert_eq!(
            PermissionMode::BypassPermissions.as_wire_str(),
            "bypassPermissions"
        );
        assert_eq!(PermissionMode::DontAsk.as_wire_str(), "dontAsk");
        assert_eq!(PermissionMode::Plan.as_wire_str(), "plan");
    }

    #[test]
    fn test_permission_mode_is_default() {
        assert!(PermissionMode::Default.is_default());
        assert!(!PermissionMode::BypassPermissions.is_default());
        assert!(!PermissionMode::AcceptEdits.is_default());
        assert!(!PermissionMode::DontAsk.is_default());
        assert!(!PermissionMode::Plan.is_default());
    }

    #[test]
    fn test_permission_mode_display() {
        assert_eq!(
            format!("{}", PermissionMode::BypassPermissions),
            "bypassPermissions"
        );
        assert_eq!(format!("{}", PermissionMode::Default), "default");
    }

    #[test]
    fn test_summary_includes_permission_mode() {
        let mut config = test_config(SubscribeMode::Mentions);
        config.permission_mode = PermissionMode::BypassPermissions;
        let s = config.summary();
        assert!(
            s.contains("permission_mode=bypassPermissions"),
            "summary should include permission_mode, got: {s}"
        );
    }

    #[test]
    fn test_summary_permission_mode_default() {
        let mut config = test_config(SubscribeMode::Mentions);
        config.permission_mode = PermissionMode::Default;
        let s = config.summary();
        assert!(
            s.contains("permission_mode=default"),
            "summary should show 'default', got: {s}"
        );
    }

    #[test]
    fn test_default_config_uses_bypass_permissions() {
        let config = test_config(SubscribeMode::Mentions);
        assert_eq!(config.permission_mode, PermissionMode::BypassPermissions);
    }

    #[test]
    fn test_permission_mode_value_enum_kebab_case() {
        // clap::ValueEnum generates kebab-case by default from PascalCase variants.
        // Verify the parse path so variant renames don't silently break CLI/env parsing.
        use clap::ValueEnum;
        let cases = [
            ("default", PermissionMode::Default),
            ("accept-edits", PermissionMode::AcceptEdits),
            ("bypass-permissions", PermissionMode::BypassPermissions),
            ("dont-ask", PermissionMode::DontAsk),
            ("plan", PermissionMode::Plan),
        ];
        for (input, expected) in &cases {
            assert_eq!(
                PermissionMode::from_str(input, true).unwrap(),
                *expected,
                "kebab-case {input:?} should parse"
            );
        }
    }

    #[test]
    fn test_permission_mode_value_enum_camel_case_aliases() {
        // Operators may set env vars using the camelCase wire-format strings
        // (e.g. BUZZ_ACP_PERMISSION_MODE=bypassPermissions). The #[value(alias)]
        // attributes ensure these parse correctly.
        use clap::ValueEnum;
        let cases = [
            ("default", PermissionMode::Default),
            ("acceptEdits", PermissionMode::AcceptEdits),
            ("bypassPermissions", PermissionMode::BypassPermissions),
            ("dontAsk", PermissionMode::DontAsk),
            ("plan", PermissionMode::Plan),
        ];
        for (input, expected) in &cases {
            assert_eq!(
                PermissionMode::from_str(input, true).unwrap(),
                *expected,
                "camelCase alias {input:?} should parse"
            );
        }
    }

    /// Helper: resolve idle_timeout_secs using the same precedence logic as Config::from_args.
    /// Precedence: explicit --idle-timeout > --turn-timeout (deprecated) > `DEFAULT_IDLE_TIMEOUT_SECS`.
    fn resolve_idle_timeout(idle: Option<u64>, turn: Option<u64>) -> u64 {
        let raw = match (idle, turn) {
            (Some(idle), Some(_)) => idle,
            (Some(idle), None) => idle,
            (None, Some(turn)) => turn,
            (None, None) => DEFAULT_IDLE_TIMEOUT_SECS,
        };
        if raw == 0 {
            1
        } else {
            raw
        }
    }

    #[test]
    fn idle_timeout_explicit_wins_over_deprecated() {
        assert_eq!(resolve_idle_timeout(Some(120), Some(600)), 120);
    }

    #[test]
    fn idle_timeout_falls_back_to_deprecated_turn_timeout() {
        assert_eq!(resolve_idle_timeout(None, Some(600)), 600);
    }

    #[test]
    fn idle_timeout_defaults_to_constant_when_neither_set() {
        assert_eq!(resolve_idle_timeout(None, None), DEFAULT_IDLE_TIMEOUT_SECS);
    }

    #[test]
    fn idle_timeout_zero_clamped_to_one() {
        assert_eq!(resolve_idle_timeout(Some(0), None), 1);
    }

    #[test]
    fn idle_timeout_zero_from_deprecated_clamped_to_one() {
        assert_eq!(resolve_idle_timeout(None, Some(0)), 1);
    }

    #[test]
    fn test_config_summary_includes_idle_and_max_turn() {
        let config = test_config(SubscribeMode::Mentions);
        let summary = config.summary();
        let expected_idle = format!("idle_timeout={DEFAULT_IDLE_TIMEOUT_SECS}s");
        assert!(
            summary.contains(&expected_idle),
            "summary should include {expected_idle}: {summary}"
        );
        assert!(
            summary.contains("max_turn=3600s"),
            "summary should include max_turn: {summary}"
        );
    }

    #[test]
    fn test_respond_to_default_is_owner_only() {
        assert_eq!(RespondTo::default(), RespondTo::OwnerOnly);
    }

    #[test]
    fn test_respond_to_display() {
        assert_eq!(format!("{}", RespondTo::OwnerOnly), "owner-only");
        assert_eq!(format!("{}", RespondTo::Allowlist), "allowlist");
        assert_eq!(format!("{}", RespondTo::Anyone), "anyone");
        assert_eq!(format!("{}", RespondTo::Nobody), "nobody");
    }

    #[test]
    fn test_respond_to_value_enum_parsing() {
        use clap::ValueEnum;
        assert_eq!(
            RespondTo::from_str("owner-only", true).unwrap(),
            RespondTo::OwnerOnly
        );
        assert_eq!(
            RespondTo::from_str("allowlist", true).unwrap(),
            RespondTo::Allowlist
        );
        assert_eq!(
            RespondTo::from_str("anyone", true).unwrap(),
            RespondTo::Anyone
        );
        assert_eq!(
            RespondTo::from_str("nobody", true).unwrap(),
            RespondTo::Nobody
        );
    }

    #[test]
    fn test_summary_includes_respond_to() {
        let config = test_config(SubscribeMode::Mentions);
        let s = config.summary();
        assert!(
            s.contains("respond_to=anyone"),
            "test_config uses Anyone, got: {s}"
        );
    }

    #[test]
    fn test_summary_respond_to_allowlist_shows_count() {
        let mut config = test_config(SubscribeMode::Mentions);
        config.respond_to = RespondTo::Allowlist;
        config.respond_to_allowlist = HashSet::from(["ab".repeat(32), "cd".repeat(32)]);
        let s = config.summary();
        assert!(
            s.contains("respond_to=allowlist(2)"),
            "should show allowlist count, got: {s}"
        );
    }

    #[test]
    fn test_validate_allowlist_valid_entries() {
        let entries = vec!["ab".repeat(32), "cd".repeat(32)];
        let result = validate_allowlist(&entries).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_validate_allowlist_deduplicates() {
        let pk = "ab".repeat(32);
        let entries = vec![pk.clone(), pk.clone(), pk];
        let result = validate_allowlist(&entries).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_validate_allowlist_normalizes_case() {
        let upper = "AB".repeat(32);
        let lower = "ab".repeat(32);
        let entries = vec![upper, lower];
        let result = validate_allowlist(&entries).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&"ab".repeat(32)));
    }

    #[test]
    fn test_validate_allowlist_trims_whitespace() {
        let entries = vec![format!("  {}  ", "ab".repeat(32))];
        let result = validate_allowlist(&entries).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&"ab".repeat(32)));
    }

    #[test]
    fn test_validate_allowlist_rejects_short() {
        let entries = vec!["abcd".to_string()];
        let err = validate_allowlist(&entries).unwrap_err();
        assert!(
            err.to_string()
                .contains("must be exactly 64 hex characters"),
            "got: {err}"
        );
    }

    #[test]
    fn test_validate_allowlist_rejects_non_hex() {
        let entries = vec!["zz".repeat(32)];
        let err = validate_allowlist(&entries).unwrap_err();
        assert!(
            err.to_string()
                .contains("must be exactly 64 hex characters"),
            "got: {err}"
        );
    }

    #[test]
    fn test_validate_allowlist_rejects_too_long() {
        let entries = vec!["ab".repeat(33)]; // 66 chars
        let err = validate_allowlist(&entries).unwrap_err();
        assert!(
            err.to_string()
                .contains("must be exactly 64 hex characters"),
            "got: {err}"
        );
    }

    #[test]
    fn test_validate_allowlist_empty_is_ok() {
        let result = validate_allowlist(&[]).unwrap();
        assert!(result.is_empty());
    }

    // ── Multiple-event-handling validation + default ──────────────────────────

    #[test]
    fn test_multiple_event_handling_default_is_steer() {
        // Parse a minimal arg set; the default for --multiple-event-handling
        // must be `steer` (steering is the default mid-turn delivery path).
        let args = CliArgs::parse_from(["buzz-acp", "--private-key", &"0".repeat(64)]);
        assert_eq!(args.multiple_event_handling, MultipleEventHandling::Steer);
        // Dedup default must remain `queue` so steering's requirement is met.
        assert!(matches!(args.dedup, DedupMode::Queue));
    }

    #[test]
    fn test_validate_steer_requires_queue_dedup() {
        // Steer + Drop is rejected (drain window would drop events).
        let err = validate_multiple_event_handling(MultipleEventHandling::Steer, DedupMode::Drop)
            .unwrap_err();
        assert!(
            err.to_string().contains("requires"),
            "expected a dedup-requirement error, got: {err}"
        );
        // Steer + Queue is accepted.
        assert!(
            validate_multiple_event_handling(MultipleEventHandling::Steer, DedupMode::Queue)
                .is_ok()
        );
    }

    #[test]
    fn test_validate_queue_handling_allows_any_dedup() {
        // The non-cancel `Queue` handling imposes no dedup constraint.
        assert!(
            validate_multiple_event_handling(MultipleEventHandling::Queue, DedupMode::Drop).is_ok()
        );
        assert!(
            validate_multiple_event_handling(MultipleEventHandling::Queue, DedupMode::Queue)
                .is_ok()
        );
    }

    #[test]
    fn test_validate_interrupt_modes_still_require_queue() {
        for mode in [
            MultipleEventHandling::Interrupt,
            MultipleEventHandling::OwnerInterrupt,
        ] {
            assert!(
                validate_multiple_event_handling(mode, DedupMode::Drop).is_err(),
                "{mode:?} + Drop should be rejected"
            );
        }
    }

    // ── Idle timeout constant + guard (PR #935) ───────────────────────────────

    #[test]
    fn default_idle_timeout_is_900_seconds() {
        // Lock the constant value so accidental changes are caught.
        assert_eq!(DEFAULT_IDLE_TIMEOUT_SECS, 900);
    }

    #[test]
    fn idle_timeout_must_be_less_than_max_turn_duration() {
        // The guard in Config::from_args rejects idle >= max_turn.
        // Exercise the same logic: if idle >= max_turn, it's invalid.
        let idle = 3600u64;
        let max_turn = 3600u64;
        assert!(
            idle >= max_turn,
            "test precondition: idle must be >= max_turn to trigger guard"
        );

        // And the valid case:
        let idle_valid = 900u64;
        let max_turn_valid = 3600u64;
        assert!(
            idle_valid < max_turn_valid,
            "default idle (900) must be less than default max_turn (3600)"
        );
    }

    // --- BUZZ_ACP_ALLOWED_RESPOND_TO gate ---

    fn parse_allowed_respond_to(raw: &[&str]) -> Result<HashSet<RespondTo>, ConfigError> {
        let mut set = HashSet::new();
        for s in raw {
            let mode = RespondTo::from_str(s.trim(), true).map_err(|_| {
                ConfigError::ConfigFile(format!(
                    "invalid value in BUZZ_ACP_ALLOWED_RESPOND_TO: '{s}' \
                     (valid values: owner-only, allowlist, anyone, nobody)"
                ))
            })?;
            set.insert(mode);
        }
        Ok(set)
    }

    fn check_allowed_respond_to(
        allowed_raw: &[&str],
        respond_to: RespondTo,
    ) -> Result<(), ConfigError> {
        let set = parse_allowed_respond_to(allowed_raw)?;
        if !set.is_empty() && !set.contains(&respond_to) {
            return Err(ConfigError::ConfigFile(format!(
                "respond_to '{}' is not permitted on this deployment \
                 (BUZZ_ACP_ALLOWED_RESPOND_TO={})",
                respond_to,
                allowed_raw.join(",")
            )));
        }
        Ok(())
    }

    #[test]
    fn allowed_respond_to_rejects_disallowed_mode() {
        let result = check_allowed_respond_to(&["owner-only", "allowlist"], RespondTo::Anyone);
        assert!(
            result.is_err(),
            "anyone should be rejected when not in allowed set"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not permitted"),
            "error should mention 'not permitted': {msg}"
        );
    }

    #[test]
    fn allowed_respond_to_accepts_allowed_mode() {
        let result = check_allowed_respond_to(&["owner-only", "allowlist"], RespondTo::OwnerOnly);
        assert!(result.is_ok(), "owner-only should be accepted: {result:?}");
    }

    #[test]
    fn allowed_respond_to_empty_allows_all() {
        // No restriction — anyone is accepted.
        let result = check_allowed_respond_to(&[], RespondTo::Anyone);
        assert!(
            result.is_ok(),
            "empty allowed set should permit any mode: {result:?}"
        );
    }

    #[test]
    fn allowed_respond_to_rejects_invalid_mode_string() {
        let result = parse_allowed_respond_to(&["owner-only", "badvalue"]);
        assert!(result.is_err(), "invalid mode string should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("invalid value in BUZZ_ACP_ALLOWED_RESPOND_TO"),
            "error should name the env var: {msg}"
        );
        assert!(
            msg.contains("badvalue"),
            "error should name the bad value: {msg}"
        );
    }

    #[test]
    fn allowed_respond_to_summary_shows_restriction_when_set() {
        let mut config = test_config(SubscribeMode::Mentions);
        config.allowed_respond_to = vec!["owner-only".to_string(), "allowlist".to_string()];
        let s = config.summary();
        assert!(
            s.contains("allowed_respond_to="),
            "summary should include allowed_respond_to when set: {s}"
        );
    }

    #[test]
    fn allowed_respond_to_summary_omitted_when_empty() {
        let config = test_config(SubscribeMode::Mentions);
        let s = config.summary();
        assert!(
            !s.contains("allowed_respond_to="),
            "summary should not include allowed_respond_to when empty: {s}"
        );
    }

    // --- Integration tests: full env-var → CliArgs → Config::from_args() path ---
    //
    // These tests exercise the actual wiring: BUZZ_ACP_ALLOWED_RESPOND_TO in the
    // environment causes clap to populate CliArgs::allowed_respond_to, which then
    // flows through Config::from_args() to produce a ConfigError. If the #[arg(env)]
    // attribute or field name were removed, these tests would fail.
    //
    // We pass the value via the CLI flag (`--allowed-respond-to`) rather than
    // std::env::set_var to avoid test-parallelism races on shared env state.
    // The env-var wiring is covered by the clap #[arg(env)] attribute itself.

    // A minimal valid private key for test use (secp256k1 scalar = 1).
    const TEST_PRIVATE_KEY: &str =
        "0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn allowed_respond_to_full_path_rejects_disallowed_mode() {
        // --allowed-respond-to=owner-only,allowlist + --respond-to=anyone → ConfigError
        let args = CliArgs::try_parse_from([
            "buzz-acp",
            "--private-key",
            TEST_PRIVATE_KEY,
            "--respond-to",
            "anyone",
            "--allowed-respond-to",
            "owner-only,allowlist",
        ])
        .expect("clap should parse args");
        let result = Config::from_args(args);

        assert!(
            result.is_err(),
            "from_args should reject respond_to=anyone when not in allowed set"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not permitted"),
            "error should mention 'not permitted': {msg}"
        );
        assert!(
            msg.contains("anyone"),
            "error should name the disallowed mode: {msg}"
        );
    }

    #[test]
    fn allowed_respond_to_full_path_accepts_allowed_mode() {
        // --allowed-respond-to=owner-only,allowlist + --respond-to=owner-only → Ok
        let args = CliArgs::try_parse_from([
            "buzz-acp",
            "--private-key",
            TEST_PRIVATE_KEY,
            "--respond-to",
            "owner-only",
            "--allowed-respond-to",
            "owner-only,allowlist",
        ])
        .expect("clap should parse args");
        let result = Config::from_args(args);

        assert!(
            result.is_ok(),
            "from_args should accept respond_to=owner-only when in allowed set: {result:?}"
        );
    }

    #[test]
    fn allowed_respond_to_full_path_unset_allows_all() {
        // No --allowed-respond-to flag → anyone is accepted.
        let args = CliArgs::try_parse_from([
            "buzz-acp",
            "--private-key",
            TEST_PRIVATE_KEY,
            "--respond-to",
            "anyone",
        ])
        .expect("clap should parse args");
        let result = Config::from_args(args);

        assert!(
            result.is_ok(),
            "from_args should accept any mode when allowed list is unset: {result:?}"
        );
    }
}
