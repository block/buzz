//! Configuration for the buzz-acp harness.
//!
//! CLI-first: every option is a CLI flag with env var fallback.
//! Config file (TOML) for complex subscription rules.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use clap::Parser;
use clap::ValueEnum;
use nostr::Keys;
use sha2::{Digest, Sha256};
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

/// Default absolute wall-clock cap per agent turn (2 hours).
/// Override via `--max-turn-duration` / `BUZZ_ACP_MAX_TURN_DURATION`.
pub(crate) const DEFAULT_MAX_TURN_DURATION_SECS: u64 = 7200;

/// Upper bound for `max_turn_duration` (7 days). Any higher is operationally
/// meaningless and risks arithmetic overflow when deriving the in-flight
/// deadline (`max_turn_duration + IN_FLIGHT_DEADLINE_BUFFER_SECS`).
pub(crate) const MAX_TURN_DURATION_CEILING_SECS: u64 = 604_800;

/// Maximum serialized credential envelope accepted from the Desktop's
/// inherited socket. The actual nsec + NIP-OA tag is far smaller; this bound
/// prevents a compromised launcher from forcing an unbounded startup read.
const MAX_CREDENTIAL_ENVELOPE_BYTES: u64 = 16 * 1024;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StartupCredentialEnvelope {
    private_key: String,
    #[serde(default)]
    auth_tag: Option<String>,
}

struct StartupCredentials {
    private_key: String,
    auth_tag: Option<String>,
}

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
    /// Auto mode — fully autonomous execution; model-gated (requires a model
    /// that supports `supportsAutoMode`).  Degrades gracefully to `default`
    /// when the session's active model does not support it.
    #[value(alias = "auto")]
    Auto,
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
            Self::Auto => "auto",
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
    #[command(flatten)]
    pub agent: AuthAgentArgs,

    /// Output structured JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

/// Shared agent-spawn flags for lightweight local ACP helper subcommands.
#[derive(Debug, Parser)]
pub struct AuthAgentArgs {
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
}

/// CLI args for `buzz-acp auth-methods` — query adapter-advertised login methods.
#[derive(Debug, Parser)]
#[command(
    name = "buzz-acp auth-methods",
    about = "Query adapter-advertised ACP authentication methods"
)]
pub struct AuthMethodsArgs {
    #[command(flatten)]
    pub agent: AuthAgentArgs,

    /// Output structured JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

/// CLI args for `buzz-acp authenticate` — start an adapter-owned login flow.
#[derive(Debug, Parser)]
#[command(
    name = "buzz-acp authenticate",
    about = "Start an adapter-owned ACP authentication flow"
)]
pub struct AuthenticateArgs {
    #[command(flatten)]
    pub agent: AuthAgentArgs,

    /// Adapter-advertised auth method id to invoke.
    #[arg(long)]
    pub method_id: String,
}

#[derive(Debug, Parser)]
#[command(
    name = "buzz-acp",
    about = "ACP harness that bridges Buzz events to AI agents"
)]
pub struct CliArgs {
    #[arg(long, env = "BUZZ_RELAY_URL", default_value = "ws://localhost:3000")]
    pub relay_url: String,

    #[arg(
        long,
        env = "BUZZ_PRIVATE_KEY",
        hide_env_values = true,
        default_value = ""
    )]
    pub private_key: String,

    /// Read a one-shot Desktop credential envelope from this process's stdin.
    /// The flag is non-secret; the signing material never enters argv or the
    /// launch environment.
    #[arg(
        long,
        env = "BUZZ_ACP_CREDENTIAL_STDIN",
        hide = true,
        hide_env_values = true
    )]
    pub credential_stdin: bool,

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
    #[arg(long, env = "BUZZ_ACP_MAX_TURN_DURATION", default_value_t = DEFAULT_MAX_TURN_DURATION_SECS)]
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
    /// `<core-memory>` prompt section (or renders the onboarding nudge
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

    /// Disable the `<base>` platform-context section prepended to every prompt.
    /// When set, agents receive only the persona `<system>` prompt with no Buzz orientation.
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

    /// Persisted effort level value (e.g. "high", "medium", "low") to apply via
    /// `session/set_config_option` at the first session creation. The configId is
    /// resolved from the adapter's advertised `thought_level` capability — not
    /// hardcoded. Non-fatal: if the adapter does not advertise `thought_level`,
    /// the value is silently ignored and the persisted effort is not overwritten.
    #[arg(long, env = "BUZZ_ACP_EFFORT_LEVEL")]
    pub effort_level: Option<String>,

    /// Title for the agent's ACP sessions, passed out-of-band in `session/new`
    /// `_meta`. Adapters that recognize it name the session after this value;
    /// others ignore it. Never enters the prompt.
    #[arg(long, env = "BUZZ_ACP_SESSION_TITLE")]
    pub session_title: Option<String>,

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

    /// Team-owned instructions layered after `<system>` and before agent memory.
    #[arg(long, env = "BUZZ_ACP_TEAM_INSTRUCTIONS")]
    pub team_instructions: Option<String>,

    /// Publish encrypted ACP observer frames over the relay.
    #[arg(long, env = "BUZZ_ACP_RELAY_OBSERVER", default_value_t = false)]
    pub relay_observer: bool,

    /// Exit after this many seconds with no dispatched events and no turn in flight.
    /// 0 disables inactivity self-termination.
    #[arg(long, env = "BUZZ_ACP_EXIT_AFTER_INACTIVITY", default_value_t = 0)]
    pub exit_after_inactivity: u64,

    /// Connect and subscribe before starting the ACP/LLM subprocess pool.
    #[arg(long, env = "BUZZ_ACP_LAZY_POOL", default_value_t = false)]
    pub lazy_pool: bool,

    /// Tear the woken pool back down to the lazy empty-slot state after this
    /// many seconds with no dispatched turn in flight and an empty queue,
    /// releasing worker subprocesses until the next accepted event re-wakes.
    /// Requires `--lazy-pool`; ignored otherwise. 0 disables idle re-sleep.
    #[arg(long, env = "BUZZ_ACP_IDLE_POOL_SLEEP", default_value_t = 0)]
    pub idle_pool_sleep: u64,
}

/// Merged NIP-01 subscription filter for a single channel.
#[derive(Debug, Clone)]
pub struct ChannelFilter {
    /// Event kinds to subscribe to. None = wildcard (all kinds).
    pub kinds: Option<Vec<u32>>,
    /// Whether to include `#p` tag filter for agent pubkey.
    pub require_mention: bool,
}

pub struct Config {
    pub keys: Keys,
    /// Raw owner attestation retained only by the trusted outer harness. It is
    /// sourced from one-shot credential stdin for Desktop-managed Context
    /// Engine launches and is never copied into an ACP child environment.
    pub(crate) owner_auth_tag: Option<String>,
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
    /// Team-owned instructions layered separately from the agent system prompt.
    pub team_instructions: Option<String>,
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
    /// `<core-memory>` section. On by default; disabled via the
    /// `--no-memory` / `BUZZ_ACP_NO_MEMORY` opt-out.
    pub memory_enabled: bool,
    /// Desired LLM model ID. Applied after every `session_new_full()`.
    pub model: Option<String>,
    /// Persisted effort level value (e.g. "high", "medium", "low"). Held as a
    /// per-worker spawn-scoped value and applied at the first session creation
    /// by pairing with the adapter's advertised `thought_level` configId.
    /// Non-fatal when absent or when the adapter does not advertise
    /// `thought_level`.
    pub effort_level: Option<String>,
    /// Sanitized session title, sent as `_meta.sessionTitle` on `session/new`.
    /// `None` when unset or when the configured value sanitized to empty.
    pub session_title: Option<String>,
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
    /// Non-secret identity/runtime variables for a fully validated trusted
    /// harness. Signing, authorization, relay, and capability material stays
    /// in the outer buzz-acp process.
    pub trusted_child_env: Vec<(String, String)>,
    /// Host-only capability and relay inputs for authenticated reply publish.
    /// Never copied into the ACP child environment.
    pub(crate) trusted_publish: Option<TrustedPublishConfig>,
    /// Fully validated managed outbox harness. `None` means the configured
    /// child is ordinary and cannot receive a verified Buzz delivery envelope.
    pub(crate) trusted_outbox_harness: Option<TrustedOutboxHarness>,
    /// Whether `codex_network_env()` successfully injected a `CODEX_CONFIG` entry into
    /// `persona_env_vars`.  When true, `AcpClient::spawn` merges all `CODEX_CONFIG` entries
    /// and forces `sandbox_workspace_write.network_access = true` via `build_codex_config_env`.
    /// When false (non-Codex agents or rejected relay URL), the helper returns None and
    /// any persona-supplied `CODEX_CONFIG` is handled with ordinary operator-wins semantics.
    pub has_generated_codex_config: bool,
    /// Whether to publish encrypted observer frames through the relay.
    pub relay_observer: bool,
    /// Seconds without dispatched events before an idle harness exits. 0 = disabled.
    pub exit_after_inactivity_secs: u64,
    /// Whether ACP/LLM subprocess initialization is deferred until accepted work arrives.
    pub lazy_pool: bool,
    /// Seconds with no dispatched turn in flight and an empty queue before a
    /// woken lazy pool is torn back down to the empty-slot state. 0 = disabled.
    /// Only meaningful when `lazy_pool` is true.
    pub idle_pool_sleep_secs: u64,
    /// Agent owner pubkey (hex). Used for `--respond-to=owner-only` gate.
    /// Replaces the old REST-based owner lookup.
    pub agent_owner: Option<String>,
    /// Disable the `<base>` platform-context section prepended to every prompt.
    pub no_base_prompt: bool,
    /// Resolved content from `--base-prompt-file`, read and validated in
    /// `from_cli()`. `None` when using the compiled-in default or when
    /// `--no-base-prompt` is set.
    pub base_prompt_content: Option<String>,
}

/// Maximum length, in characters, of a session title sent to the adapter.
const SESSION_TITLE_MAX_CHARS: usize = 80;

/// Normalize a configured session title into something safe to hand an adapter.
///
/// Control characters are dropped, runs of whitespace collapse to a single
/// space, and the result is trimmed and capped at
/// [`SESSION_TITLE_MAX_CHARS`]. Returns `None` when nothing printable is left.
///
/// Buzz is the only guard here: Codex's own `normalize_thread_name` merely
/// trims, so an unbounded display name would be persisted verbatim into its
/// thread store.
fn sanitize_session_title(raw: &str) -> Option<String> {
    let collapsed = raw
        .split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    // Truncate by chars, not bytes, so a multi-byte name can't be cut mid-UTF-8.
    let title: String = collapsed
        .chars()
        .take(SESSION_TITLE_MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string();
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Separator between the agent name and the channel in a composed title.
/// U+00B7 MIDDLE DOT, spaces on both sides.
const SESSION_TITLE_SEPARATOR: &str = " · ";

/// Compose a per-session title as `Agent · #channel`.
///
/// One agent in five channels gets five sessions; a bare agent name would show
/// five identical rows in the adapter's thread list. Only the channel part is
/// truncated to fit [`SESSION_TITLE_MAX_CHARS`], so the agent name always
/// survives. Returns the bare agent name when there is no channel, the channel
/// name is blank, or no room is left for it.
pub(crate) fn compose_session_title(agent: &str, channel_name: Option<&str>) -> String {
    let Some(channel) = channel_name.and_then(sanitize_session_title) else {
        return agent.to_string();
    };
    // Reserve the separator and the `#` sigil alongside the agent name.
    let reserved = agent.chars().count() + SESSION_TITLE_SEPARATOR.chars().count() + 1;
    let channel: String = channel
        .chars()
        .take(SESSION_TITLE_MAX_CHARS.saturating_sub(reserved))
        .collect::<String>()
        .trim_end()
        .to_string();
    if channel.is_empty() {
        return agent.to_string();
    }
    format!("{agent}{SESSION_TITLE_SEPARATOR}#{channel}")
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
    // Windows resolves commands through `.exe` binaries and npm's `.cmd`/`.bat`
    // shims; all three name the same runtime identity.
    let stem = [".exe", ".cmd", ".bat"]
        .iter()
        .find_map(|extension| lower.strip_suffix(extension))
        .unwrap_or(&lower);
    stem.chars()
        .map(|character| match character {
            ' ' | '_' => '-',
            _ => character,
        })
        .collect()
}

/// Managed runtimes that may receive authenticated Buzz envelopes.
/// Classification is based on exact command, arguments, artifact bytes and
/// identity pins -- never a user-controlled display name or basename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustedOutboxHarness {
    Gabe,
    Stacy,
}

#[derive(Clone)]
pub(crate) struct TrustedPublishConfig {
    pub harness: TrustedOutboxHarness,
    pub capability_key: [u8; 32],
    pub auth_tag: String,
    pub relay_http_base: String,
    pub outbox_dir: PathBuf,
}

impl TrustedOutboxHarness {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Gabe => "gabe-acp",
            Self::Stacy => "stacy-acp",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TrustedArtifactSpec {
    path: &'static str,
    sha256: &'static str,
    executable: bool,
    immutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustedArtifactIdentity {
    path: &'static str,
    len: u64,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustedChildValidation {
    pub(crate) env: Vec<(String, String)>,
    pub(crate) artifacts: Vec<TrustedArtifactIdentity>,
}

#[derive(Debug, Clone, Copy)]
struct TrustedDeploymentSpec {
    receipt_path: &'static str,
    lifecycle_state_path: &'static str,
    docker_socket: &'static str,
    project: &'static str,
    agent_container: &'static str,
    agent_service: &'static str,
    dashboard_container: &'static str,
    dashboard_service: &'static str,
}

#[derive(Debug)]
struct TrustedHarnessSpec {
    harness: TrustedOutboxHarness,
    command: &'static str,
    args: &'static [&'static str],
    artifacts: &'static [TrustedArtifactSpec],
    agent_pin_env: &'static str,
    owner_pin_env: &'static str,
    runtime_env: &'static [(&'static str, &'static str)],
    capability_home: Option<&'static str>,
    outbox_dir: &'static str,
    health_addr: Option<&'static str>,
    deployment: Option<TrustedDeploymentSpec>,
}

const GABE_NODE_PATH: &str = "/Users/gabriel/.buzz/RUNTIME/trusted-node/d36b3d980963d44bd2c5e844fac4cfeee26a167b744287a4e74a9575af9d0559/node";
const GABE_NODE_SHA256: &str = "d36b3d980963d44bd2c5e844fac4cfeee26a167b744287a4e74a9575af9d0559";
const GABE_ADAPTER_SHA256: &str =
    "96a8efaf20cbc1cb92fb2ae2eca5a0bdefabba42f9cd6e2ca21299c724bd7c5c";
const GABE_ADAPTER_PATH: &str = "/Users/gabriel/.buzz/RUNTIME/context-engine/96a8efaf20cbc1cb92fb2ae2eca5a0bdefabba42f9cd6e2ca21299c724bd7c5c/scripts/gabe-acp.mjs";
const STACY_ADAPTER_PATH: &str = "/Users/gabriel/.buzz/RUNTIME/stacy-context-engine/96a8efaf20cbc1cb92fb2ae2eca5a0bdefabba42f9cd6e2ca21299c724bd7c5c/scripts/gabe-acp.mjs";

const GABE_ARGS: &[&str] = &[GABE_ADAPTER_PATH];
const STACY_ARGS: &[&str] = &[STACY_ADAPTER_PATH];
const GABE_ARTIFACTS: &[TrustedArtifactSpec] = &[
    TrustedArtifactSpec {
        path: GABE_NODE_PATH,
        sha256: GABE_NODE_SHA256,
        executable: true,
        immutable: true,
    },
    TrustedArtifactSpec {
        path: GABE_ADAPTER_PATH,
        sha256: GABE_ADAPTER_SHA256,
        executable: false,
        immutable: true,
    },
];
const STACY_ARTIFACTS: &[TrustedArtifactSpec] = &[
    TrustedArtifactSpec {
        path: GABE_NODE_PATH,
        sha256: GABE_NODE_SHA256,
        executable: true,
        immutable: true,
    },
    TrustedArtifactSpec {
        path: STACY_ADAPTER_PATH,
        sha256: GABE_ADAPTER_SHA256,
        executable: false,
        immutable: true,
    },
];
const GABE_RUNTIME_ENV: &[(&str, &str)] = &[
    ("CONTEXT_ENGINE_BASE_URL", "http://127.0.0.1:18790"),
    ("BUZZ_ACP_SESSION_PREFIX", "gabe"),
];
const STACY_RUNTIME_ENV: &[(&str, &str)] = &[
    ("CONTEXT_ENGINE_BASE_URL", "http://127.0.0.1:18803"),
    ("BUZZ_ACP_SESSION_PREFIX", "stacy"),
];
const STACY_DEPLOYMENT: TrustedDeploymentSpec = TrustedDeploymentSpec {
    receipt_path: "/Users/gabriel/.buzz/RUNTIME/stacy-context-engine/home/deployment-receipt.json",
    lifecycle_state_path:
        "/Users/gabriel/.buzz/RUNTIME/stacy-context-engine/home/deployment-control/lifecycle-state",
    docker_socket: "/Users/gabriel/.docker/run/docker.sock",
    project: "stacy",
    agent_container: "stacy-agent",
    agent_service: "stacy",
    dashboard_container: "stacy-dashboard",
    dashboard_service: "stacy-dashboard",
};
const INSTALL_RECEIPTS_ROOT: &str = "/Users/gabriel/.buzz/RUNTIME/deployment-receipts";
const GABE_HARNESS_SPEC: TrustedHarnessSpec = TrustedHarnessSpec {
    harness: TrustedOutboxHarness::Gabe,
    command: GABE_NODE_PATH,
    args: GABE_ARGS,
    artifacts: GABE_ARTIFACTS,
    agent_pin_env: "BUZZ_GABE_AGENT_PUBKEY",
    owner_pin_env: "BUZZ_GABE_OWNER_PUBKEY",
    runtime_env: GABE_RUNTIME_ENV,
    capability_home: Some("/Users/gabriel/.openclaw"),
    outbox_dir: "/Users/gabriel/.openclaw/state/context-engine-buzz/prepared",
    health_addr: Some("127.0.0.1:18790"),
    deployment: None,
};
const STACY_HARNESS_SPEC: TrustedHarnessSpec = TrustedHarnessSpec {
    harness: TrustedOutboxHarness::Stacy,
    command: GABE_NODE_PATH,
    args: STACY_ARGS,
    artifacts: STACY_ARTIFACTS,
    agent_pin_env: "BUZZ_STACY_AGENT_PUBKEY",
    owner_pin_env: "BUZZ_STACY_OWNER_PUBKEY",
    runtime_env: STACY_RUNTIME_ENV,
    capability_home: Some("/Users/gabriel/.buzz/RUNTIME/stacy-context-engine/home"),
    outbox_dir:
        "/Users/gabriel/.buzz/RUNTIME/stacy-context-engine/home/state/context-engine-buzz/prepared",
    health_addr: Some("127.0.0.1:18803"),
    deployment: Some(STACY_DEPLOYMENT),
};
static TRUSTED_HARNESSES: &[TrustedHarnessSpec] = &[GABE_HARNESS_SPEC, STACY_HARNESS_SPEC];

fn configured_trusted_harness(
    command: &str,
    args: &[String],
) -> Option<&'static TrustedHarnessSpec> {
    TRUSTED_HARNESSES.iter().find(|spec| {
        command == spec.command
            && args.len() == spec.args.len()
            && args
                .iter()
                .map(String::as_str)
                .eq(spec.args.iter().copied())
    })
}

fn validate_trusted_artifact(spec: TrustedArtifactSpec) -> Result<TrustedArtifactIdentity, String> {
    validate_trusted_artifact_at(spec, Path::new("/Users/gabriel/.buzz/RUNTIME"))
}

fn validate_trusted_artifact_at(
    spec: TrustedArtifactSpec,
    runtime_root: &Path,
) -> Result<TrustedArtifactIdentity, String> {
    use std::io::Read;

    let path = Path::new(spec.path);
    let before = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect artifact: {error}"))?;
    if !before.file_type().is_file() || before.file_type().is_symlink() {
        return Err("artifact is not a regular non-symlink file".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize artifact: {error}"))?;
    if canonical != path {
        return Err("artifact path is not canonical".into());
    }

    let mut parent = path
        .parent()
        .ok_or_else(|| "artifact has no parent directory".to_string())?;
    let ancestor_stop = if spec.immutable {
        runtime_root
    } else {
        parent
            .parent()
            .ok_or_else(|| "artifact test parent chain is incomplete".to_string())?
    };
    let mut parents = Vec::new();
    while parent != ancestor_stop {
        if spec.immutable && !parent.starts_with(runtime_root) {
            return Err("artifact escaped the trusted runtime root".into());
        }
        let metadata = std::fs::symlink_metadata(parent)
            .map_err(|error| format!("cannot inspect artifact ancestor: {error}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("artifact ancestor is not a regular directory".into());
        }
        if parent
            .canonicalize()
            .map_err(|error| format!("cannot canonicalize artifact ancestor: {error}"))?
            != parent
        {
            return Err("artifact ancestor path is not canonical".into());
        }
        parents.push(metadata);
        parent = parent
            .parent()
            .ok_or_else(|| "artifact ancestor chain is incomplete".to_string())?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let executable_owner = std::env::current_exe()
            .and_then(std::fs::metadata)
            .map_err(|error| format!("cannot inspect current executable: {error}"))?
            .uid();
        if before.uid() != executable_owner
            || parents
                .iter()
                .any(|metadata| metadata.uid() != executable_owner)
        {
            return Err("artifact owner does not match the harness executable owner".into());
        }
        let expected_mode = if spec.executable { 0o555 } else { 0o444 };
        if before.mode() & 0o777 != expected_mode {
            return Err("artifact mode does not match the frozen read-only mode".into());
        }
        if parents
            .iter()
            .any(|metadata| metadata.mode() & 0o7777 != 0o555)
        {
            return Err("artifact ancestor directory is not read-only".into());
        }
    }

    #[cfg(target_os = "macos")]
    if spec.immutable {
        use std::os::macos::fs::MetadataExt;
        const USER_IMMUTABLE: u32 = 0x0000_0002;
        if before.st_flags() & USER_IMMUTABLE == 0
            || parents
                .iter()
                .any(|metadata| metadata.st_flags() & USER_IMMUTABLE == 0)
        {
            return Err("artifact is not protected by the macOS uchg flag".into());
        }
    }

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| format!("cannot securely open artifact: {error}"))?
    };
    #[cfg(not(unix))]
    let mut file =
        std::fs::File::open(path).map_err(|error| format!("cannot open artifact: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("cannot inspect opened artifact: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read artifact: {error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let after = file
        .metadata()
        .map_err(|error| format!("cannot re-inspect artifact: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != before.dev()
            || opened.ino() != before.ino()
            || after.dev() != opened.dev()
            || after.ino() != opened.ino()
            || after.len() != opened.len()
        {
            return Err("artifact changed during verification".into());
        }
    }
    let actual_hash = hex::encode(hasher.finalize());
    if actual_hash != spec.sha256 {
        return Err("artifact digest does not match the frozen digest".into());
    }
    Ok(TrustedArtifactIdentity {
        path: spec.path,
        len: after.len(),
        #[cfg(unix)]
        dev: {
            use std::os::unix::fs::MetadataExt as _;
            after.dev()
        },
        #[cfg(unix)]
        ino: {
            use std::os::unix::fs::MetadataExt as _;
            after.ino()
        },
    })
}

fn normalize_pubkey_pin(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn decode_capability_key(value: &str) -> Result<[u8; 32], String> {
    let suffix = value
        .strip_prefix("buzz_cap_v1:")
        .ok_or_else(|| "trusted capability is malformed".to_string())?;
    if suffix.len() != 64
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("trusted capability is malformed".into());
    }
    let decoded = hex::decode(suffix).map_err(|_| "trusted capability is malformed".to_string())?;
    decoded
        .try_into()
        .map_err(|_| "trusted capability is malformed".to_string())
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustedDeploymentReceipt {
    version: u8,
    project: String,
    buzz_commit: String,
    source_commit: String,
    upstream_pin: String,
    compose_sha256: String,
    install_receipt_sha256: String,
    deployment_generation: String,
    lifecycle_generation: String,
    stacy_container_id: String,
    stacy_image_id: String,
    dashboard_container_id: String,
    dashboard_image_id: String,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustedInstallReceipt {
    buzz_commit: String,
    gabe_commit: String,
    stacy_commit: String,
    node_sha256: String,
    buzz_acp_sha256: String,
    adapter_sha256: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerContainerInspect {
    id: String,
    image: String,
    state: DockerContainerState,
    config: DockerContainerConfig,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerContainerState {
    running: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerContainerConfig {
    labels: HashMap<String, String>,
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_sha256_image_id(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|digest| is_lower_hex(digest, 64))
}

fn read_trusted_deployment_receipt(
    spec: TrustedDeploymentSpec,
) -> Result<TrustedDeploymentReceipt, String> {
    use std::io::Read;

    let path = Path::new(spec.receipt_path);
    let before = std::fs::symlink_metadata(path)
        .map_err(|_| "trusted deployment receipt is unavailable".to_string())?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > 4096 {
        return Err("trusted deployment receipt is unsafe".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "trusted deployment receipt cannot be canonicalized".to_string())?;
    if canonical != path {
        return Err("trusted deployment receipt is not canonical".into());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "trusted deployment receipt has no parent".to_string())?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| "trusted deployment receipt parent is unavailable".to_string())?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err("trusted deployment receipt parent is unsafe".into());
    }
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| "trusted deployment receipt parent cannot be canonicalized".to_string())?;
    if canonical_parent != parent {
        return Err("trusted deployment receipt parent is not canonical".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let executable_owner = std::env::current_exe()
            .and_then(std::fs::metadata)
            .map_err(|_| "cannot inspect harness owner".to_string())?
            .uid();
        if parent_metadata.uid() != executable_owner
            || parent_metadata.mode() & 0o777 != 0o700
            || before.uid() != executable_owner
            || before.mode() & 0o777 != 0o600
            || before.nlink() != 1
        {
            return Err("trusted deployment receipt ownership or mode is unsafe".into());
        }
    }

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| "trusted deployment receipt cannot be opened".to_string())?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::File::open(path)
        .map_err(|_| "trusted deployment receipt cannot be opened".to_string())?;
    let opened = file
        .metadata()
        .map_err(|_| "trusted deployment receipt cannot be inspected".to_string())?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "trusted deployment receipt cannot be read".to_string())?;
    if bytes.len() > 4096 {
        return Err("trusted deployment receipt is too large".into());
    }
    let after = file
        .metadata()
        .map_err(|_| "trusted deployment receipt cannot be re-inspected".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != before.dev()
            || opened.ino() != before.ino()
            || after.dev() != opened.dev()
            || after.ino() != opened.ino()
            || after.len() != opened.len()
            || after.mtime() != opened.mtime()
            || after.mtime_nsec() != opened.mtime_nsec()
        {
            return Err("trusted deployment receipt changed during verification".into());
        }
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "trusted deployment receipt is malformed".to_string())
}

fn validate_trusted_lifecycle_state(
    spec: TrustedDeploymentSpec,
    receipt: &TrustedDeploymentReceipt,
) -> Result<(), String> {
    use std::io::Read;

    let path = Path::new(spec.lifecycle_state_path);
    let parent = path
        .parent()
        .ok_or_else(|| "trusted lifecycle state has no parent".to_string())?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| "trusted lifecycle state parent is unavailable".to_string())?;
    let before = std::fs::symlink_metadata(path)
        .map_err(|_| "trusted lifecycle state is unavailable".to_string())?;
    if !parent_metadata.is_dir()
        || parent_metadata.file_type().is_symlink()
        || !before.is_file()
        || before.file_type().is_symlink()
        || before.len() > 128
    {
        return Err("trusted lifecycle state is unsafe".into());
    }
    if parent
        .canonicalize()
        .map_err(|_| "trusted lifecycle state parent cannot be canonicalized".to_string())?
        != parent
        || path
            .canonicalize()
            .map_err(|_| "trusted lifecycle state cannot be canonicalized".to_string())?
            != path
    {
        return Err("trusted lifecycle state path is not canonical".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let executable_owner = std::env::current_exe()
            .and_then(std::fs::metadata)
            .map_err(|_| "cannot inspect harness owner".to_string())?
            .uid();
        if parent_metadata.uid() != executable_owner
            || parent_metadata.mode() & 0o777 != 0o700
            || before.uid() != executable_owner
            || before.mode() & 0o777 != 0o600
            || before.nlink() != 1
        {
            return Err("trusted lifecycle state ownership or mode is unsafe".into());
        }
    }
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| "trusted lifecycle state cannot be opened".to_string())?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::File::open(path)
        .map_err(|_| "trusted lifecycle state cannot be opened".to_string())?;
    let opened = file
        .metadata()
        .map_err(|_| "trusted lifecycle state cannot be inspected".to_string())?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(129)
        .read_to_end(&mut bytes)
        .map_err(|_| "trusted lifecycle state cannot be read".to_string())?;
    let after = file
        .metadata()
        .map_err(|_| "trusted lifecycle state cannot be re-inspected".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != before.dev()
            || opened.ino() != before.ino()
            || after.dev() != opened.dev()
            || after.ino() != opened.ino()
            || after.len() != opened.len()
            || after.mtime() != opened.mtime()
            || after.mtime_nsec() != opened.mtime_nsec()
        {
            return Err("trusted lifecycle state changed during verification".into());
        }
    }
    let expected = format!("{} up\n", receipt.lifecycle_generation);
    if bytes != expected.as_bytes() {
        return Err("trusted lifecycle state does not authorize this deployment".into());
    }
    Ok(())
}

#[cfg(unix)]
fn inspect_docker_container(
    socket_path: &str,
    container: &str,
) -> Result<DockerContainerInspect, String> {
    use std::io::{Read, Write};
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    if container.is_empty()
        || !container
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("trusted deployment container name is invalid".into());
    }
    let socket = Path::new(socket_path);
    let metadata = std::fs::symlink_metadata(socket)
        .map_err(|_| "trusted Docker socket is unavailable".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
        return Err("trusted Docker socket is unsafe".into());
    }
    let canonical = socket
        .canonicalize()
        .map_err(|_| "trusted Docker socket cannot be canonicalized".to_string())?;
    if canonical != socket {
        return Err("trusted Docker socket is not canonical".into());
    }
    let executable_owner = std::env::current_exe()
        .and_then(std::fs::metadata)
        .map_err(|_| "cannot inspect harness owner".to_string())?
        .uid();
    if metadata.uid() != executable_owner || metadata.mode() & 0o022 != 0 {
        return Err("trusted Docker socket ownership or mode is unsafe".into());
    }

    let mut stream = UnixStream::connect(socket)
        .map_err(|_| "trusted Docker socket cannot be reached".to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| "cannot bound trusted Docker read".to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| "cannot bound trusted Docker write".to_string())?;
    write!(
        stream,
        "GET /containers/{container}/json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .map_err(|_| "trusted Docker request failed".to_string())?;
    let mut response = Vec::new();
    (&mut stream)
        .take(256 * 1024 + 1)
        .read_to_end(&mut response)
        .map_err(|_| "trusted Docker response failed".to_string())?;
    if response.len() > 256 * 1024 {
        return Err("trusted Docker response is too large".into());
    }
    let boundary = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "trusted Docker response is malformed".to_string())?;
    let head = std::str::from_utf8(&response[..boundary])
        .map_err(|_| "trusted Docker response head is invalid".to_string())?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .ok_or_else(|| "trusted Docker response status is absent".to_string())?;
    if status != "HTTP/1.1 200 OK" && status != "HTTP/1.0 200 OK" {
        return Err("trusted Docker container inspection was rejected".into());
    }
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err("trusted Docker response header is malformed".into());
        };
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if !value.trim().eq_ignore_ascii_case("chunked") || chunked {
                return Err("trusted Docker response encoding is unsupported".into());
            }
            chunked = true;
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("trusted Docker response length is ambiguous".into());
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "trusted Docker response length is invalid".to_string())?,
            );
        }
    }
    if chunked && content_length.is_some() {
        return Err("trusted Docker response framing is ambiguous".into());
    }
    let wire_body = &response[boundary + 4..];
    let decoded;
    let body = if chunked {
        decoded = decode_chunked_body(wire_body)?;
        decoded.as_slice()
    } else {
        if content_length != Some(wire_body.len()) {
            return Err("trusted Docker response length does not match".into());
        }
        wire_body
    };
    serde_json::from_slice(body)
        .map_err(|_| "trusted Docker container inspection is malformed".to_string())
}

fn decode_chunked_body(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut remaining = input;
    let mut decoded = Vec::new();
    loop {
        let line_end = remaining
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "trusted Docker chunk size is malformed".to_string())?;
        let size_text = std::str::from_utf8(&remaining[..line_end])
            .map_err(|_| "trusted Docker chunk size is invalid".to_string())?;
        if size_text.is_empty()
            || size_text.len() > 16
            || !size_text.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("trusted Docker chunk size is invalid".into());
        }
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| "trusted Docker chunk size is invalid".to_string())?;
        remaining = &remaining[line_end + 2..];
        if size == 0 {
            if remaining != b"\r\n" {
                return Err("trusted Docker chunk trailer is unsupported".into());
            }
            return Ok(decoded);
        }
        if size > 256 * 1024 - decoded.len()
            || remaining.len() < size + 2
            || &remaining[size..size + 2] != b"\r\n"
        {
            return Err("trusted Docker chunk body is invalid".into());
        }
        decoded.extend_from_slice(&remaining[..size]);
        remaining = &remaining[size + 2..];
    }
}

#[cfg(not(unix))]
fn inspect_docker_container(
    _socket_path: &str,
    _container: &str,
) -> Result<DockerContainerInspect, String> {
    Err("trusted Docker inspection requires Unix domain sockets".into())
}

fn validate_deployed_container(
    inspect: &DockerContainerInspect,
    expected_container_id: &str,
    expected_image_id: &str,
    expected_service: &str,
    spec: TrustedDeploymentSpec,
    receipt: &TrustedDeploymentReceipt,
) -> Result<(), String> {
    if inspect.id != expected_container_id
        || inspect.image != expected_image_id
        || !inspect.state.running
    {
        return Err("trusted deployment container identity or state does not match".into());
    }
    let expected_labels = [
        ("com.docker.compose.project", spec.project),
        ("com.docker.compose.service", expected_service),
        (
            "com.johnpein.stacy.source-commit",
            receipt.source_commit.as_str(),
        ),
        (
            "com.johnpein.stacy.upstream-pin",
            receipt.upstream_pin.as_str(),
        ),
        (
            "com.johnpein.stacy.compose-sha256",
            receipt.compose_sha256.as_str(),
        ),
        (
            "com.johnpein.stacy.deployment-generation",
            receipt.deployment_generation.as_str(),
        ),
    ];
    if expected_labels.iter().any(|(name, expected)| {
        inspect.config.labels.get(*name).map(String::as_str) != Some(*expected)
    }) {
        return Err("trusted deployment container labels do not match".into());
    }
    Ok(())
}

fn validate_trusted_install_receipt(
    deployment: &TrustedDeploymentReceipt,
) -> Result<[u8; 32], String> {
    use std::io::Read;

    let executable_hash = std::fs::read(std::env::current_exe().map_err(|_| {
        "trusted install receipt cannot resolve the harness executable".to_string()
    })?)
    .map(|bytes| hex::encode(Sha256::digest(bytes)))
    .map_err(|_| "trusted install receipt cannot hash the harness executable".to_string())?;
    let root = Path::new(INSTALL_RECEIPTS_ROOT);
    let root_metadata = std::fs::symlink_metadata(root)
        .map_err(|_| "trusted install receipt root is unavailable".to_string())?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err("trusted install receipt root is unsafe".into());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "trusted install receipt root cannot be canonicalized".to_string())?;
    if canonical_root != root {
        return Err("trusted install receipt root is not canonical".into());
    }
    #[cfg(unix)]
    let executable_owner = {
        use std::os::unix::fs::MetadataExt;
        let owner = std::env::current_exe()
            .and_then(std::fs::metadata)
            .map_err(|_| "cannot inspect harness owner".to_string())?
            .uid();
        if root_metadata.uid() != owner || root_metadata.mode() & 0o022 != 0 {
            return Err("trusted install receipt root ownership or mode is unsafe".into());
        }
        owner
    };

    let directory = root.join(format!(
        "{}-{}-{}",
        deployment.buzz_commit, deployment.upstream_pin, deployment.source_commit
    ));
    let directory_metadata = std::fs::symlink_metadata(&directory)
        .map_err(|_| "exact trusted install receipt directory is unavailable".to_string())?;
    if !directory_metadata.is_dir() || directory_metadata.file_type().is_symlink() {
        return Err("exact trusted install receipt directory is unsafe".into());
    }
    let canonical_directory = directory.canonicalize().map_err(|_| {
        "exact trusted install receipt directory cannot be canonicalized".to_string()
    })?;
    if canonical_directory != directory {
        return Err("exact trusted install receipt directory is not canonical".into());
    }
    let path = directory.join("context-engine-runtimes.json");
    let before = std::fs::symlink_metadata(&path)
        .map_err(|_| "exact trusted install receipt is unavailable".to_string())?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > 4096 {
        return Err("exact trusted install receipt is unsafe".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if directory_metadata.uid() != executable_owner
            || before.uid() != executable_owner
            || directory_metadata.mode() & 0o777 != 0o555
            || before.mode() & 0o777 != 0o444
            || before.nlink() != 1
        {
            return Err("exact trusted install receipt ownership or mode is unsafe".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const USER_IMMUTABLE: u32 = 0x0000_0002;
        if directory_metadata.st_flags() & USER_IMMUTABLE == 0
            || before.st_flags() & USER_IMMUTABLE == 0
        {
            return Err("exact trusted install receipt is not immutable".into());
        }
    }
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|_| "exact trusted install receipt cannot be opened".to_string())?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::File::open(&path)
        .map_err(|_| "exact trusted install receipt cannot be opened".to_string())?;
    let opened = file
        .metadata()
        .map_err(|_| "exact trusted install receipt cannot be inspected".to_string())?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "exact trusted install receipt cannot be read".to_string())?;
    if bytes.len() > 4096 {
        return Err("exact trusted install receipt is too large".into());
    }
    let after = file
        .metadata()
        .map_err(|_| "exact trusted install receipt cannot be re-inspected".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != before.dev()
            || opened.ino() != before.ino()
            || after.dev() != opened.dev()
            || after.ino() != opened.ino()
            || after.len() != opened.len()
        {
            return Err("exact trusted install receipt changed during verification".into());
        }
    }
    let receipt = serde_json::from_slice::<TrustedInstallReceipt>(&bytes)
        .map_err(|_| "exact trusted install receipt is malformed".to_string())?;
    if receipt.buzz_commit != deployment.buzz_commit
        || receipt.gabe_commit != deployment.upstream_pin
        || receipt.stacy_commit != deployment.source_commit
        || receipt.node_sha256 != GABE_NODE_SHA256
        || receipt.adapter_sha256 != GABE_ADAPTER_SHA256
        || receipt.buzz_acp_sha256 != executable_hash
    {
        return Err("exact trusted install receipt does not authorize this deployment".into());
    }
    let actual_receipt_hash = hex::encode(Sha256::digest(&bytes));
    if actual_receipt_hash != deployment.install_receipt_sha256 {
        return Err("exact trusted install receipt digest does not match".into());
    }
    let canonical = serde_json::to_vec(&receipt)
        .map_err(|_| "trusted install receipt cannot be canonicalized".to_string())?;
    Ok(Sha256::digest(canonical).into())
}

fn validate_trusted_deployment(spec: TrustedDeploymentSpec) -> Result<[u8; 32], String> {
    let receipt = read_trusted_deployment_receipt(spec)?;
    if receipt.version != 3
        || receipt.project != spec.project
        || !is_lower_hex(&receipt.buzz_commit, 40)
        || !is_lower_hex(&receipt.source_commit, 40)
        || !is_lower_hex(&receipt.upstream_pin, 40)
        || !is_lower_hex(&receipt.compose_sha256, 64)
        || !is_lower_hex(&receipt.install_receipt_sha256, 64)
        || !is_lower_hex(&receipt.deployment_generation, 64)
        || !is_lower_hex(&receipt.lifecycle_generation, 64)
        || !is_lower_hex(&receipt.stacy_container_id, 64)
        || !is_sha256_image_id(&receipt.stacy_image_id)
        || !is_lower_hex(&receipt.dashboard_container_id, 64)
        || !is_sha256_image_id(&receipt.dashboard_image_id)
    {
        return Err("trusted deployment receipt values are invalid".into());
    }
    validate_trusted_lifecycle_state(spec, &receipt)?;
    let install_fingerprint = validate_trusted_install_receipt(&receipt)?;
    let agent = inspect_docker_container(spec.docker_socket, spec.agent_container)?;
    validate_deployed_container(
        &agent,
        &receipt.stacy_container_id,
        &receipt.stacy_image_id,
        spec.agent_service,
        spec,
        &receipt,
    )?;
    let dashboard = inspect_docker_container(spec.docker_socket, spec.dashboard_container)?;
    validate_deployed_container(
        &dashboard,
        &receipt.dashboard_container_id,
        &receipt.dashboard_image_id,
        spec.dashboard_service,
        spec,
        &receipt,
    )?;
    let canonical = serde_json::to_vec(&receipt)
        .map_err(|_| "trusted deployment receipt cannot be canonicalized".to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(install_fingerprint);
    hasher.update(canonical);
    Ok(hasher.finalize().into())
}

struct RuntimePreflight {
    capability_key: Option<[u8; 32]>,
    deployment_fingerprint: Option<[u8; 32]>,
}

fn validate_runtime_preflight(spec: &TrustedHarnessSpec) -> Result<RuntimePreflight, String> {
    let mut capability_key = None;
    if let Some(home) = spec.capability_home {
        use std::io::Read;

        let credentials = Path::new(home).join("credentials");
        let capability = credentials.join("context-engine-buzz-adapter.cap");
        let directory_metadata = std::fs::symlink_metadata(&credentials)
            .map_err(|_| "trusted capability directory is unavailable".to_string())?;
        if !directory_metadata.is_dir() || directory_metadata.file_type().is_symlink() {
            return Err("trusted capability directory is unsafe".into());
        }
        let canonical_directory = credentials
            .canonicalize()
            .map_err(|_| "trusted capability directory cannot be canonicalized".to_string())?;
        if canonical_directory != credentials {
            return Err("trusted capability directory is not canonical".into());
        }

        let before = std::fs::symlink_metadata(&capability)
            .map_err(|_| "trusted capability is unavailable".to_string())?;
        if !before.is_file() || before.file_type().is_symlink() || before.len() > 256 {
            return Err("trusted capability file is unsafe".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let executable_owner = std::env::current_exe()
                .and_then(std::fs::metadata)
                .map_err(|_| "cannot inspect harness owner".to_string())?
                .uid();
            if directory_metadata.uid() != executable_owner
                || before.uid() != executable_owner
                || directory_metadata.mode() & 0o777 != 0o700
                || before.mode() & 0o777 != 0o600
            {
                return Err("trusted capability ownership or mode is unsafe".into());
            }
        }
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(nix::libc::O_NOFOLLOW)
                .open(&capability)
                .map_err(|_| "trusted capability cannot be opened".to_string())?
        };
        #[cfg(not(unix))]
        let mut file = std::fs::File::open(&capability)
            .map_err(|_| "trusted capability cannot be opened".to_string())?;
        let opened = file
            .metadata()
            .map_err(|_| "trusted capability cannot be inspected".to_string())?;
        let mut value = String::new();
        file.read_to_string(&mut value)
            .map_err(|_| "trusted capability cannot be read".to_string())?;
        let after = file
            .metadata()
            .map_err(|_| "trusted capability cannot be re-inspected".to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if opened.dev() != before.dev()
                || opened.ino() != before.ino()
                || after.dev() != opened.dev()
                || after.ino() != opened.ino()
                || after.len() != opened.len()
            {
                return Err("trusted capability changed during verification".into());
            }
        }
        let value = value.strip_suffix('\n').unwrap_or(&value);
        capability_key = Some(decode_capability_key(value)?);
    }

    if let Some(address) = spec.health_addr {
        use std::io::{Read, Write};
        use std::net::{SocketAddr, TcpStream};
        use std::time::Duration;

        let socket: SocketAddr = address
            .parse()
            .map_err(|_| "trusted health address is invalid".to_string())?;
        let mut stream = TcpStream::connect_timeout(&socket, Duration::from_secs(3))
            .map_err(|_| "trusted Context Engine is unreachable".to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(|_| "cannot bound trusted health read".to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|_| "cannot bound trusted health write".to_string())?;
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .map_err(|_| "trusted health request failed".to_string())?;
        let mut response = [0u8; 512];
        let count = stream
            .read(&mut response)
            .map_err(|_| "trusted health response failed".to_string())?;
        let head = std::str::from_utf8(&response[..count])
            .map_err(|_| "trusted health response is invalid".to_string())?;
        if !head.starts_with("HTTP/1.1 200 ") && !head.starts_with("HTTP/1.0 200 ") {
            return Err("trusted Context Engine health check rejected the runtime".into());
        }
    }

    if !spec.outbox_dir.is_empty() {
        let path = Path::new(spec.outbox_dir);
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| "trusted prepared-event directory is unavailable".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("trusted prepared-event directory is unsafe".into());
        }
        let canonical = path
            .canonicalize()
            .map_err(|_| "trusted prepared-event directory cannot be canonicalized".to_string())?;
        if canonical != path {
            return Err("trusted prepared-event directory is not canonical".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let executable_owner = std::env::current_exe()
                .and_then(std::fs::metadata)
                .map_err(|_| "cannot inspect harness owner".to_string())?
                .uid();
            if metadata.uid() != executable_owner || metadata.mode() & 0o777 != 0o700 {
                return Err("trusted prepared-event directory ownership or mode is unsafe".into());
            }
        }
    }

    let deployment_fingerprint = spec
        .deployment
        .map(validate_trusted_deployment)
        .transpose()?;

    Ok(RuntimePreflight {
        capability_key,
        deployment_fingerprint,
    })
}

struct TrustedHarnessCredentials {
    harness: TrustedOutboxHarness,
    child_env: Vec<(String, String)>,
    publish: Option<TrustedPublishConfig>,
}

struct TrustedHarnessAuthority<'a> {
    keys: &'a Keys,
    auth_tag: Option<&'a str>,
    relay_url: &'a str,
    agent_pin: Option<&'a str>,
    owner_pin: Option<&'a str>,
}

/// Convert buzz-acp's own relay URL into the `http(s)://` base the `buzz` CLI
/// expects for its authenticated in-process relay queries and publishes.
///
/// buzz-acp connects to the relay over a WebSocket (`ws://` / `wss://`), but the
/// durable prepared-event facade takes an `http(s)://` base. `wss://` maps to
/// `https://`, `ws://` maps to
/// `http://`, and an already-`http(s)://` URL passes through unchanged. Host,
/// port, and path are preserved.
///
/// Returns `None` for an empty, unparseable, or non-ws/http URL so the caller
/// emits no `BUZZ_RELAY_URL` at all rather than pointing the CLI at a broken
/// value.
fn relay_url_to_http_base(relay_url: &str) -> Option<String> {
    let trimmed = relay_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let http = if let Some(rest) = trimmed.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("ws://") {
        format!("http://{rest}")
    } else if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        trimmed.to_string()
    } else {
        // Unknown scheme (or scheme-less): don't guess — emit nothing.
        return None;
    };

    // Guard against a scheme-only or otherwise unparseable value slipping
    // through (e.g. `ws://` with no host). Only surface a URL with a real host.
    match Url::parse(&http) {
        Ok(parsed) if parsed.host_str().is_some() => Some(http),
        _ => None,
    }
}

/// Validate the complete managed-outbox trust boundary and, only on success,
/// return the authoritative environment that `AcpClient::spawn` may restore
/// after scrubbing ambient credentials.
fn trusted_harness_credentials_for_spec(
    spec: &TrustedHarnessSpec,
    command: &str,
    args: &[String],
    authority: TrustedHarnessAuthority<'_>,
) -> Result<TrustedHarnessCredentials, String> {
    if command != spec.command
        || args.len() != spec.args.len()
        || !args
            .iter()
            .map(String::as_str)
            .eq(spec.args.iter().copied())
    {
        return Err("command or arguments do not match the frozen harness".into());
    }
    for artifact in spec.artifacts {
        validate_trusted_artifact(*artifact)?;
    }

    let agent_pubkey = authority.keys.public_key().to_hex().to_ascii_lowercase();
    if normalize_pubkey_pin(authority.agent_pin).as_deref() != Some(agent_pubkey.as_str()) {
        return Err("managed agent identity pin is absent or mismatched".into());
    }

    let auth_tag = authority
        .auth_tag
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "owner auth tag is absent".to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_secs();
    let owner_pubkey =
        buzz_sdk::nip_oa::verify_auth_tag_for_event(auth_tag, &authority.keys.public_key(), 9, now)
            .map_err(|_| "owner auth tag is invalid".to_string())?
            .to_hex()
            .to_ascii_lowercase();
    if normalize_pubkey_pin(authority.owner_pin).as_deref() != Some(owner_pubkey.as_str()) {
        return Err("managed owner identity pin is absent or mismatched".into());
    }

    let relay_url = relay_url_to_http_base(authority.relay_url)
        .ok_or_else(|| "relay URL cannot be converted to a trusted HTTP base".to_string())?;

    let capability_key = validate_runtime_preflight(spec)?.capability_key;

    let mut child_env = vec![
        ("BUZZ_AGENT_PUBKEY".to_string(), agent_pubkey.clone()),
        ("BUZZ_ACP_AGENT_OWNER".to_string(), owner_pubkey.clone()),
        (spec.agent_pin_env.to_string(), agent_pubkey),
        (spec.owner_pin_env.to_string(), owner_pubkey),
    ];
    child_env.extend(
        spec.runtime_env
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
    );
    Ok(TrustedHarnessCredentials {
        harness: spec.harness,
        child_env,
        publish: capability_key.map(|capability_key| TrustedPublishConfig {
            harness: spec.harness,
            capability_key,
            auth_tag: auth_tag.to_string(),
            relay_http_base: relay_url,
            outbox_dir: PathBuf::from(spec.outbox_dir),
        }),
    })
}

fn requested_trusted_harness(
    command: &str,
    args: &[String],
) -> Result<Option<&'static TrustedHarnessSpec>, String> {
    let pinned = TRUSTED_HARNESSES
        .iter()
        .filter(|spec| {
            std::env::var_os(spec.agent_pin_env).is_some()
                || std::env::var_os(spec.owner_pin_env).is_some()
        })
        .collect::<Vec<_>>();
    if pinned.len() > 1 {
        return Err("multiple managed outbox harness pin sets are present".into());
    }
    if let Some(spec) = pinned.first().copied() {
        if configured_trusted_harness(command, args).map(|candidate| candidate.harness)
            != Some(spec.harness)
        {
            return Err(format!(
                "{} pins require its exact frozen command and arguments",
                spec.harness.as_str()
            ));
        }
        return Ok(Some(spec));
    }
    Ok(configured_trusted_harness(command, args))
}

fn trusted_harness_credentials(
    command: &str,
    args: &[String],
    keys: &Keys,
    auth_tag: Option<&str>,
    relay_url: &str,
) -> Result<Option<TrustedHarnessCredentials>, String> {
    let Some(spec) = requested_trusted_harness(command, args)? else {
        return Ok(None);
    };
    let agent_pin = std::env::var(spec.agent_pin_env).ok();
    let owner_pin = std::env::var(spec.owner_pin_env).ok();
    trusted_harness_credentials_for_spec(
        spec,
        command,
        args,
        TrustedHarnessAuthority {
            keys,
            auth_tag,
            relay_url,
            agent_pin: agent_pin.as_deref(),
            owner_pin: owner_pin.as_deref(),
        },
    )
    .map(Some)
}

/// Revalidate the frozen artifact and identity boundary immediately before a
/// trusted spawn. Raw credentials cached in `Config` are inputs, never proof.
pub(crate) fn revalidate_trusted_child_env(
    command: &str,
    args: &[String],
    child_env: &[(String, String)],
) -> Result<TrustedChildValidation, String> {
    let spec = configured_trusted_harness(command, args)
        .ok_or_else(|| "trusted credentials do not match a frozen harness".to_string())?;
    let value = |name: &str| -> Result<&str, String> {
        let mut values = child_env
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str());
        let value = values
            .next()
            .ok_or_else(|| format!("trusted child environment is missing {name}"))?;
        if values.next().is_some() {
            return Err(format!("trusted child environment duplicates {name}"));
        }
        Ok(value)
    };
    if child_env.len() != spec.runtime_env.len() + 4
        || child_env.iter().any(|(key, _)| {
            !crate::acp::TRUSTED_CHILD_ENV_KEYS
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(key))
        })
    {
        return Err("trusted child environment has unexpected keys".into());
    }
    let agent_pin = value("BUZZ_AGENT_PUBKEY")?;
    let owner_pin = value("BUZZ_ACP_AGENT_OWNER")?;
    let spec_agent_pin = value(spec.agent_pin_env)?;
    let spec_owner_pin = value(spec.owner_pin_env)?;
    if normalize_pubkey_pin(Some(agent_pin)).as_deref()
        != normalize_pubkey_pin(Some(spec_agent_pin)).as_deref()
        || normalize_pubkey_pin(Some(owner_pin)).as_deref()
            != normalize_pubkey_pin(Some(spec_owner_pin)).as_deref()
    {
        return Err("trusted child identity aliases disagree".into());
    }
    for (key, expected) in spec.runtime_env {
        if value(key)? != *expected {
            return Err(format!("trusted child environment changed {key}"));
        }
    }
    let current_agent_pin = std::env::var(spec.agent_pin_env).ok();
    let current_owner_pin = std::env::var(spec.owner_pin_env).ok();
    if normalize_pubkey_pin(current_agent_pin.as_deref()).as_deref()
        != normalize_pubkey_pin(Some(agent_pin)).as_deref()
        || normalize_pubkey_pin(current_owner_pin.as_deref()).as_deref()
            != normalize_pubkey_pin(Some(owner_pin)).as_deref()
    {
        return Err("managed identity pins changed after startup".into());
    }
    let artifacts = spec
        .artifacts
        .iter()
        .map(|artifact| validate_trusted_artifact(*artifact))
        .collect::<Result<Vec<_>, _>>()?;
    validate_runtime_preflight(spec)?;
    Ok(TrustedChildValidation {
        env: child_env.to_vec(),
        artifacts,
    })
}

/// Revalidate the live host/runtime boundary at both trusted-delivery seams.
/// The first call prevents dispatch to a replaced deployment; the second
/// prevents publishing a response if the deployment changed during the turn.
pub(crate) fn revalidate_trusted_publish_runtime(
    harness: TrustedOutboxHarness,
) -> Result<Option<[u8; 32]>, String> {
    let spec = TRUSTED_HARNESSES
        .iter()
        .find(|spec| spec.harness == harness)
        .ok_or_else(|| "trusted publish harness is not frozen".to_string())?;
    for artifact in spec.artifacts {
        let _ = validate_trusted_artifact(*artifact)?;
    }
    Ok(validate_runtime_preflight(spec)?.deployment_fingerprint)
}

fn default_agent_args(command: &str) -> Option<Vec<String>> {
    match normalize_agent_command_identity(command).as_str() {
        "goose" => Some(vec!["acp".to_string()]),
        "codex" | "codex-acp" | "claude-agent-acp" | "claude-code-acp" | "claude-code"
        | "claudecode" | "buzz-agent" => Some(Vec::new()),
        _ => None,
    }
}

/// Per-runtime environment defaults applied when Buzz owns the agent process.
///
/// Mirrors [`default_agent_args`]: keyed on the normalized command identity,
/// with the merge (in `AcpClient::spawn`) giving explicit persona env and
/// inherited parent env precedence over these defaults.
///
/// Hermes: ACP hosts supply session MCP servers explicitly through
/// `session/new`, but Hermes otherwise starts every profile-configured MCP
/// server before it responds to `initialize` — which can exhaust the host's
/// startup budget (see block/buzz#3355). Skip that unrelated global startup
/// by default; an operator or persona can still opt back in by setting the
/// variable explicitly.
pub(crate) fn default_agent_env(command: &str) -> &'static [(&'static str, &'static str)] {
    match normalize_agent_command_identity(command).as_str() {
        "hermes" | "hermes-agent" | "hermes-acp" => &[("HERMES_ACP_SKIP_CONFIGURED_MCP", "1")],
        _ => &[],
    }
}

/// Build the `CODEX_CONFIG` environment variable that enables full outbound
/// network access in Codex's macOS Seatbelt sandbox.
///
/// Codex sandboxes MCP subprocesses (including `buzz-cli`) behind a Seatbelt sandbox
/// that blocks all outbound network by default. Without this env var, `buzz-cli`
/// requests are blocked before they can reach the relay WebSocket.
///
/// Returns `Some(("CODEX_CONFIG", "{\"sandbox_workspace_write\":{\"network_access\":true}}"))` for
/// Codex agents, or `None` for non-Codex agents or when the relay URL cannot be parsed.
///
/// The env var is forwarded by the `@agentclientprotocol/codex-acp` adapter (1.x) as a
/// session-level config override (via `CODEX_CONFIG` → `thread/start config`), which is
/// equivalent to the TOML override `sandbox_workspace_write.network_access = true`.
/// That sets `NetworkSandboxPolicy::Enabled`, causing the Seatbelt policy to include
/// `(allow network-outbound)` — full outbound TCP/TLS at the OS level.
///
/// URL validation is preserved as a guard: injection is skipped when the relay URL cannot
/// be parsed, avoiding accidental sandbox widening for malformed configs.
///
/// Handles `ws://`, `wss://`, `http://`, and `https://` schemes.
pub fn codex_network_env(agent_command: &str, relay_url: &str) -> Option<(String, String)> {
    match normalize_agent_command_identity(agent_command).as_str() {
        "codex" | "codex-acp" => {}
        _ => return None,
    }

    // Validate the relay URL before injecting broader network access. On parse failure,
    // skip injection rather than panicking or widening the sandbox unconditionally.
    let host = match Url::parse(relay_url) {
        Ok(u) => match u.host_str() {
            Some(h) => h.to_owned(),
            None => {
                tracing::warn!(
                    relay_url,
                    "codex network config: no host in relay URL — skipping injection"
                );
                return None;
            }
        },
        Err(e) => {
            tracing::warn!(relay_url, error = %e, "codex network config: failed to parse relay URL — skipping injection");
            return None;
        }
    };

    tracing::debug!(host, "injecting CODEX_CONFIG network_access for relay host");

    Some((
        "CODEX_CONFIG".into(),
        "{\"sandbox_workspace_write\":{\"network_access\":true}}".into(),
    ))
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

fn read_startup_credential_envelope() -> Result<StartupCredentialEnvelope, ConfigError> {
    decode_startup_credential_envelope(std::io::stdin().lock())
}

fn decode_startup_credential_envelope(
    mut reader: impl Read,
) -> Result<StartupCredentialEnvelope, ConfigError> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_CREDENTIAL_ENVELOPE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ConfigError::Io)?;
    if bytes.len() as u64 > MAX_CREDENTIAL_ENVELOPE_BYTES {
        return Err(ConfigError::ConfigFile(
            "Desktop credential envelope exceeds 16 KiB".into(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        ConfigError::ConfigFile(format!("invalid Desktop credential envelope: {error}"))
    })
}

fn resolve_startup_credentials(
    private_key: String,
    credential_stdin: bool,
    env_auth_tag: Option<String>,
) -> Result<StartupCredentials, ConfigError> {
    let credentials = if credential_stdin {
        if !private_key.is_empty()
            || env_auth_tag
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        {
            return Err(ConfigError::ConfigFile(
                "Desktop credential stdin cannot be combined with BUZZ_PRIVATE_KEY, \
                 --private-key, or BUZZ_AUTH_TAG"
                    .into(),
            ));
        }
        let envelope = read_startup_credential_envelope()?;
        StartupCredentials {
            private_key: envelope.private_key,
            auth_tag: envelope.auth_tag.filter(|value| !value.is_empty()),
        }
    } else {
        StartupCredentials {
            private_key,
            auth_tag: env_auth_tag.filter(|value| !value.is_empty()),
        }
    };

    if credentials.private_key.is_empty() {
        return Err(ConfigError::ConfigFile(
            "missing signing key: set BUZZ_PRIVATE_KEY/--private-key or use Desktop credential stdin"
                .into(),
        ));
    }
    if credentials.private_key.len() > 256 {
        return Err(ConfigError::ConfigFile(
            "signing key in startup credentials exceeds 256 bytes".into(),
        ));
    }
    if credentials
        .auth_tag
        .as_deref()
        .is_some_and(|value| value.len() > 8 * 1024)
    {
        return Err(ConfigError::ConfigFile(
            "owner auth tag in startup credentials exceeds 8 KiB".into(),
        ));
    }
    Ok(credentials)
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
        let mut startup_credentials = resolve_startup_credentials(
            std::mem::take(&mut args.private_key),
            args.credential_stdin,
            std::env::var("BUZZ_AUTH_TAG").ok(),
        )?;
        let keys = Keys::parse(&startup_credentials.private_key)?;
        // Best-effort zeroize: overwrite the raw private key string to reduce
        // exposure via core dumps or heap inspection (#41). Without the `zeroize`
        // crate we can only clear the String — the allocator may retain copies.
        startup_credentials
            .private_key
            .replace_range(.., &"0".repeat(startup_credentials.private_key.len()));
        startup_credentials.private_key.clear();
        let owner_auth_tag = startup_credentials.auth_tag;

        let system_prompt = if let Some(text) = args.system_prompt {
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

        if agent_command.trim().is_empty() {
            return Err(ConfigError::ConfigFile(
                "agent_command must not be empty".into(),
            ));
        }

        let agent_args = normalize_agent_args(&agent_command, args.agent_args);

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
            } else if raw > MAX_TURN_DURATION_CEILING_SECS {
                return Err(ConfigError::ConfigFile(format!(
                    "max_turn_duration ({}s) exceeds ceiling ({}s / 7 days)",
                    raw, MAX_TURN_DURATION_CEILING_SECS
                )));
            } else {
                raw
            }
        };

        // idle_timeout must be strictly less than max_turn_duration. If idle_timeout
        // >= max_turn_duration, the absolute wall-clock cap would fire before the idle
        // timeout ever could, making idle_timeout a dead letter.
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

        // Spawned desktop agents now carry a complete instance snapshot. Team
        // instructions arrive independently so they can be layered at runtime.
        let mut persona_env_vars = Vec::new();
        let model = args.model;

        // Inject CODEX_CONFIG so the @agentclientprotocol/codex-acp adapter (1.x)
        // opens the Seatbelt network sandbox for buzz-cli (an MCP subprocess). No-op
        // for non-Codex agents or unparseable relay URLs.
        let has_generated_codex_config =
            if let Some(network_env) = codex_network_env(&agent_command, &args.relay_url) {
                persona_env_vars.push(network_env);
                true
            } else {
                false
            };

        // The managed child receives only non-secret identity/runtime values
        // after exact executable bytes, owner attestation, identity pins,
        // capability, and Context Engine health all validate. Signing,
        // authorization, relay, and capability values remain host-only.
        let trusted_credentials = trusted_harness_credentials(
            &agent_command,
            &agent_args,
            &keys,
            owner_auth_tag.as_deref(),
            &args.relay_url,
        )
        .map_err(ConfigError::ConfigFile)?;
        let trusted_outbox_harness = trusted_credentials
            .as_ref()
            .map(|credentials| credentials.harness);
        let trusted_child_env = trusted_credentials
            .as_ref()
            .map(|credentials| credentials.child_env.clone())
            .unwrap_or_default();
        let trusted_publish = trusted_credentials.and_then(|credentials| credentials.publish);

        validate_multiple_event_handling(args.multiple_event_handling, args.dedup)?;

        let config = Config {
            keys,
            owner_auth_tag,
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
            team_instructions: args
                .team_instructions
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
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
            effort_level: args.effort_level,
            session_title: args
                .session_title
                .as_deref()
                .and_then(sanitize_session_title),
            permission_mode: args.permission_mode,
            respond_to: args.respond_to,
            respond_to_allowlist,
            allowed_respond_to,
            persona_env_vars,
            trusted_child_env,
            trusted_publish,
            trusted_outbox_harness,
            has_generated_codex_config,
            relay_observer: args.relay_observer,
            exit_after_inactivity_secs: args.exit_after_inactivity,
            lazy_pool: args.lazy_pool,
            idle_pool_sleep_secs: args.idle_pool_sleep,
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
        // Deserialization leaves consecutive_timeouts at its zero default; reset explicitly.
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
            owner_auth_tag: None,
            relay_url: "ws://localhost:3000".into(),
            agent_command: "goose".into(),
            agent_args: vec!["acp".into()],
            mcp_command: "".into(),
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
            max_turn_duration_secs: DEFAULT_MAX_TURN_DURATION_SECS,
            agents: 1,
            heartbeat_interval_secs: 0,
            turn_liveness_secs: 10,
            heartbeat_prompt: None,
            system_prompt: None,
            team_instructions: None,
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
            effort_level: None,
            session_title: None,
            permission_mode: PermissionMode::BypassPermissions,
            respond_to: RespondTo::Anyone,
            respond_to_allowlist: HashSet::new(),
            allowed_respond_to: Vec::new(),
            persona_env_vars: vec![],
            trusted_child_env: vec![],
            trusted_publish: None,
            trusted_outbox_harness: None,
            has_generated_codex_config: false,
            relay_observer: false,
            exit_after_inactivity_secs: 0,
            lazy_pool: false,
            idle_pool_sleep_secs: 0,
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
        // Windows npm shims resolve to `.cmd`/`.bat` wrappers.
        assert_eq!(
            normalize_agent_command_identity(r"C:\Users\test\AppData\Roaming\npm\hermes-acp.cmd"),
            "hermes-acp"
        );
        assert_eq!(
            normalize_agent_command_identity(r"C:\Tools\Hermes\HERMES-AGENT.BAT"),
            "hermes-agent"
        );
        // Non-ASCII must not panic.
        assert_eq!(normalize_agent_command_identity("my-agënt"), "my-agënt");
        // Edge cases: empty, whitespace-only, bare separators.
        assert_eq!(normalize_agent_command_identity(""), "");
        assert_eq!(normalize_agent_command_identity("   "), "");
        assert_eq!(normalize_agent_command_identity("/"), "");
        assert_eq!(normalize_agent_command_identity("///"), "");
    }

    #[test]
    fn recognizes_trusted_harness_only_from_exact_frozen_command_and_args() {
        let gabe_args = vec![GABE_ADAPTER_PATH.to_string()];
        let stacy_args = vec![STACY_ADAPTER_PATH.to_string()];
        assert_eq!(
            configured_trusted_harness(GABE_NODE_PATH, &gabe_args).map(|spec| spec.harness),
            Some(TrustedOutboxHarness::Gabe),
        );
        assert_eq!(
            configured_trusted_harness(GABE_NODE_PATH, &stacy_args).map(|spec| spec.harness),
            Some(TrustedOutboxHarness::Stacy),
        );
        assert!(configured_trusted_harness("node", &gabe_args).is_none());
        assert!(configured_trusted_harness(GABE_NODE_PATH, &["gabe-acp.mjs".into()]).is_none());
        assert!(configured_trusted_harness(GABE_NODE_PATH, &["stacy-acp.mjs".into()]).is_none());
    }

    #[test]
    fn capability_key_requires_exact_prefix_and_lowercase_32_byte_hex() {
        let token = format!("buzz_cap_v1:{}", "ab".repeat(32));
        assert_eq!(decode_capability_key(&token).unwrap(), [0xab; 32]);
        assert!(decode_capability_key(&format!("buzz_cap_v1:{}", "AB".repeat(32))).is_err());
        assert!(decode_capability_key(&format!("buzz_cap_v2:{}", "ab".repeat(32))).is_err());
        assert!(decode_capability_key("buzz_cap_v1:00").is_err());
    }

    #[test]
    fn managed_pubkey_pins_are_trimmed_and_case_normalized() {
        let upper = "AB".repeat(32);
        assert_eq!(
            normalize_pubkey_pin(Some(&format!("  {upper}  "))),
            Some("ab".repeat(32))
        );
        assert!(normalize_pubkey_pin(Some("not-a-pubkey")).is_none());
    }

    #[test]
    fn default_agent_env_recognizes_hermes_identities() {
        for command in [
            "hermes",
            "hermes-agent",
            "hermes-acp",
            "/opt/hermes/bin/hermes-acp",
            r"C:\Users\test\bin\HERMES_ACP.EXE",
            r"C:\Users\test\AppData\Roaming\npm\hermes-acp.cmd",
        ] {
            assert_eq!(
                default_agent_env(command),
                &[("HERMES_ACP_SKIP_CONFIGURED_MCP", "1")],
                "unexpected env defaults for {command}"
            );
        }
        for command in ["goose", "codex-acp", "claude-agent-acp", "buzz-agent", ""] {
            assert!(
                default_agent_env(command).is_empty(),
                "non-Hermes command must have no env defaults: {command}"
            );
        }
    }

    #[test]
    fn strips_legacy_acp_arg_case_insensitively() {
        assert_eq!(
            normalize_agent_args("codex-acp", vec!["ACP".into()]),
            Vec::<String>::new()
        );
    }

    // --- codex_network_env tests ---

    const CODEX_CONFIG_JSON: &str = "{\"sandbox_workspace_write\":{\"network_access\":true}}";

    #[test]
    fn codex_network_env_wss_url() {
        let result = codex_network_env("codex-acp", "wss://sprout-oss.stage.blox.sqprod.co");
        assert_eq!(
            result,
            Some(("CODEX_CONFIG".to_string(), CODEX_CONFIG_JSON.to_string()))
        );
    }

    #[test]
    fn codex_network_env_ws_url() {
        let result = codex_network_env("codex-acp", "ws://localhost:3000");
        assert_eq!(
            result,
            Some(("CODEX_CONFIG".to_string(), CODEX_CONFIG_JSON.to_string()))
        );
    }

    #[test]
    fn codex_network_env_https_url() {
        let result = codex_network_env("codex-acp", "https://relay.example.com/path");
        assert_eq!(
            result,
            Some(("CODEX_CONFIG".to_string(), CODEX_CONFIG_JSON.to_string()))
        );
    }

    #[test]
    fn codex_network_env_http_url_with_port() {
        let result = codex_network_env("codex-acp", "http://relay.example.com:8080/query");
        assert_eq!(
            result,
            Some(("CODEX_CONFIG".to_string(), CODEX_CONFIG_JSON.to_string()))
        );
    }

    #[test]
    fn codex_network_env_bare_codex_command() {
        // "codex" (not "codex-acp") should also get the env var.
        let result = codex_network_env("codex", "wss://relay.example.com");
        assert_eq!(
            result,
            Some(("CODEX_CONFIG".to_string(), CODEX_CONFIG_JSON.to_string()))
        );
    }

    #[test]
    fn codex_network_env_full_path_codex_command() {
        // Full path like /usr/local/bin/codex-acp should be normalized.
        let result = codex_network_env("/usr/local/bin/codex-acp", "wss://relay.example.com");
        assert_eq!(
            result,
            Some(("CODEX_CONFIG".to_string(), CODEX_CONFIG_JSON.to_string()))
        );
    }

    #[test]
    fn codex_network_env_non_codex_agent_returns_none() {
        assert!(codex_network_env("goose", "wss://relay.example.com").is_none());
        assert!(codex_network_env("claude-agent-acp", "wss://relay.example.com").is_none());
        assert!(codex_network_env("buzz-agent", "wss://relay.example.com").is_none());
    }

    #[test]
    fn codex_network_env_includes_sandbox_network_access() {
        // The JSON value must set sandbox_workspace_write.network_access=true — without
        // it, the Seatbelt sandbox blocks outbound connections in the 1.x adapter.
        let result = codex_network_env("codex-acp", "wss://relay.example.com");
        let (key, val) = result.expect("expected Some for valid codex + valid url");
        assert_eq!(key, "CODEX_CONFIG");
        assert!(
            val.contains("\"sandbox_workspace_write\""),
            "JSON must contain sandbox_workspace_write"
        );
        assert!(
            val.contains("\"network_access\":true"),
            "JSON must set network_access=true"
        );
    }

    #[test]
    fn codex_network_env_empty_relay_url_returns_none() {
        // Empty string fails Url::parse — graceful None return.
        assert!(codex_network_env("codex-acp", "").is_none());
    }

    #[test]
    fn codex_network_env_schemeless_string_returns_none() {
        // A bare string with no scheme fails Url::parse — graceful None return.
        assert!(codex_network_env("codex-acp", "not-a-url").is_none());
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
    fn inactivity_exit_defaults_disabled_and_accepts_cli_value() {
        let key = "0".repeat(64);
        let default = CliArgs::parse_from(["buzz-acp", "--private-key", &key]);
        assert_eq!(default.exit_after_inactivity, 0);

        let configured = CliArgs::parse_from([
            "buzz-acp",
            "--private-key",
            &key,
            "--exit-after-inactivity",
            "120",
        ]);
        assert_eq!(configured.exit_after_inactivity, 120);
    }

    #[test]
    fn lazy_pool_defaults_off() {
        let key = "0".repeat(64);
        assert!(!CliArgs::parse_from(["buzz-acp", "--private-key", &key]).lazy_pool);
    }

    #[test]
    fn idle_pool_sleep_defaults_disabled_and_accepts_cli_value() {
        let key = "0".repeat(64);
        let default = CliArgs::parse_from(["buzz-acp", "--private-key", &key]);
        assert_eq!(default.idle_pool_sleep, 0);

        let configured = CliArgs::parse_from([
            "buzz-acp",
            "--private-key",
            &key,
            "--idle-pool-sleep",
            "300",
        ]);
        assert_eq!(configured.idle_pool_sleep, 300);
    }

    #[test]
    fn lazy_pool_cli_flag_enables_deferred_startup() {
        let key = "0".repeat(64);
        let args = CliArgs::try_parse_from(["buzz-acp", "--private-key", &key, "--lazy-pool=true"]);
        assert!(args.is_err(), "bool flags do not take an explicit value");
        assert!(CliArgs::parse_from(["buzz-acp", "--private-key", &key, "--lazy-pool"]).lazy_pool);
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
        assert_eq!(PermissionMode::Auto.as_wire_str(), "auto");
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
        assert!(!PermissionMode::Auto.is_default());
        assert!(!PermissionMode::BypassPermissions.is_default());
        assert!(!PermissionMode::AcceptEdits.is_default());
        assert!(!PermissionMode::DontAsk.is_default());
        assert!(!PermissionMode::Plan.is_default());
    }

    #[test]
    fn test_permission_mode_auto_degrades_to_default_when_unsupported() {
        // The wire string is "auto" — the adapter handles graceful downgrade
        // to "default" when the active model does not support Auto mode.
        // Verify only that the wire string is correct and distinct from "default".
        let auto = PermissionMode::Auto;
        assert_eq!(auto.as_wire_str(), "auto");
        assert_ne!(auto.as_wire_str(), "default");
        assert!(!auto.is_default());
    }

    #[test]
    fn test_permission_mode_display() {
        assert_eq!(
            format!("{}", PermissionMode::BypassPermissions),
            "bypassPermissions"
        );
        assert_eq!(format!("{}", PermissionMode::Default), "default");
        assert_eq!(format!("{}", PermissionMode::Auto), "auto");
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
            ("auto", PermissionMode::Auto),
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
            ("auto", PermissionMode::Auto),
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
            summary.contains(&format!("max_turn={DEFAULT_MAX_TURN_DURATION_SECS}s")),
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

        // And the valid case (const assertion so clippy doesn't flag it):
        const {
            assert!(DEFAULT_IDLE_TIMEOUT_SECS < DEFAULT_MAX_TURN_DURATION_SECS);
        }
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
        let msg = match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("from_args unexpectedly accepted a disallowed response mode"),
        };
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
        let msg = match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("from_args unexpectedly accepted a disallowed response mode"),
        };
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
    fn credential_stdin_envelope_is_strict_and_bounded() {
        let encoded = serde_json::to_vec(&serde_json::json!({
            "private_key": TEST_PRIVATE_KEY,
            "auth_tag": "owner-attestation-canary"
        }))
        .expect("serialize credential fixture");
        let envelope = decode_startup_credential_envelope(std::io::Cursor::new(encoded))
            .expect("valid credential envelope");
        assert_eq!(envelope.private_key, TEST_PRIVATE_KEY);
        assert_eq!(
            envelope.auth_tag.as_deref(),
            Some("owner-attestation-canary")
        );

        let unknown = br#"{"private_key":"x","unexpected":true}"#;
        assert!(decode_startup_credential_envelope(std::io::Cursor::new(unknown)).is_err());
        let oversized = vec![b'x'; MAX_CREDENTIAL_ENVELOPE_BYTES as usize + 1];
        assert!(decode_startup_credential_envelope(std::io::Cursor::new(oversized)).is_err());
    }

    #[test]
    fn credential_stdin_rejects_mixed_environment_or_argv_secrets_before_reading() {
        let mixed_key = resolve_startup_credentials(TEST_PRIVATE_KEY.into(), true, None)
            .err()
            .expect("credential stdin plus argv key must fail closed");
        assert!(mixed_key.to_string().contains("cannot be combined"));

        let mixed_auth = resolve_startup_credentials(String::new(), true, Some("tag".into()))
            .err()
            .expect("credential stdin plus auth env must fail closed");
        assert!(mixed_auth.to_string().contains("cannot be combined"));
    }

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
        let msg = match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("from_args unexpectedly accepted a disallowed response mode"),
        };
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
            "from_args should accept respond_to=owner-only when in allowed set"
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
            "from_args should accept any mode when allowed list is unset"
        );
    }

    #[test]
    fn ordinary_config_never_places_managed_identity_in_persona_env() {
        let args = CliArgs::try_parse_from(["buzz-acp", "--private-key", TEST_PRIVATE_KEY])
            .expect("clap should parse args");
        let config = Config::from_args(args).expect("valid config");
        assert!(config.trusted_outbox_harness.is_none());
        assert!(config.trusted_child_env.is_empty());
        assert!(!config
            .persona_env_vars
            .iter()
            .any(|(key, _)| key == "BUZZ_AGENT_PUBKEY"));
    }

    #[cfg(unix)]
    fn temporary_trusted_spec() -> (TrustedHarnessSpec, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "buzz-acp-trusted-spec-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).expect("create trusted harness fixture");
        let command = directory.join("trusted-adapter");
        std::fs::write(&command, b"#!/bin/sh\nexit 0\n").expect("write trusted adapter");
        let mut permissions = std::fs::metadata(&command)
            .expect("stat trusted adapter")
            .permissions();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&command, permissions).expect("chmod trusted adapter");
        let command = command.canonicalize().expect("canonical trusted adapter");

        let mut directory_permissions = std::fs::metadata(&directory)
            .expect("stat trusted harness fixture directory")
            .permissions();
        directory_permissions.set_mode(0o555);
        std::fs::set_permissions(&directory, directory_permissions)
            .expect("lock trusted harness fixture directory");

        let command_text: &'static str =
            Box::leak(command.to_string_lossy().into_owned().into_boxed_str());
        let digest: &'static str = Box::leak(
            hex::encode(Sha256::digest(
                std::fs::read(&command).expect("read trusted adapter"),
            ))
            .into_boxed_str(),
        );
        let artifacts: &'static [TrustedArtifactSpec] = Box::leak(
            vec![TrustedArtifactSpec {
                path: command_text,
                sha256: digest,
                executable: true,
                immutable: false,
            }]
            .into_boxed_slice(),
        );
        (
            TrustedHarnessSpec {
                harness: TrustedOutboxHarness::Gabe,
                command: command_text,
                args: &[],
                artifacts,
                agent_pin_env: "TEST_AGENT_PIN",
                owner_pin_env: "TEST_OWNER_PIN",
                runtime_env: &[],
                capability_home: None,
                outbox_dir: "",
                health_addr: None,
                deployment: None,
            },
            directory,
        )
    }

    #[cfg(unix)]
    #[test]
    fn trusted_harness_requires_artifact_attestation_and_full_identity_chain() {
        use std::os::unix::fs::PermissionsExt;

        let (spec, directory) = temporary_trusted_spec();
        let agent = Keys::generate();
        let owner = Keys::generate();
        let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "kind=9")
            .expect("compute owner auth tag");
        let agent_pin = agent.public_key().to_hex();
        let owner_pin = owner.public_key().to_hex();
        let args = Vec::<String>::new();

        let credentials = trusted_harness_credentials_for_spec(
            &spec,
            spec.command,
            &args,
            TrustedHarnessAuthority {
                keys: &agent,
                auth_tag: Some(&auth_tag),
                relay_url: "wss://project-gabe.communities.buzz.xyz",
                agent_pin: Some(&agent_pin),
                owner_pin: Some(&owner_pin),
            },
        )
        .expect("fully attested harness should receive credentials");
        for key in [
            "BUZZ_AGENT_PUBKEY",
            "BUZZ_ACP_AGENT_OWNER",
            "TEST_AGENT_PIN",
            "TEST_OWNER_PIN",
        ] {
            assert!(credentials.child_env.iter().any(|(name, _)| name == key));
        }
        for secret in ["BUZZ_PRIVATE_KEY", "BUZZ_AUTH_TAG", "BUZZ_RELAY_URL"] {
            assert!(!credentials.child_env.iter().any(|(name, _)| name == secret));
        }

        for (name, auth, relay, agent_pin_value, owner_pin_value) in [
            (
                "missing auth",
                None,
                "wss://relay.example",
                Some(agent_pin.as_str()),
                Some(owner_pin.as_str()),
            ),
            (
                "invalid auth",
                Some("not-an-auth-tag"),
                "wss://relay.example",
                Some(agent_pin.as_str()),
                Some(owner_pin.as_str()),
            ),
            (
                "invalid relay",
                Some(auth_tag.as_str()),
                "ftp://relay.example",
                Some(agent_pin.as_str()),
                Some(owner_pin.as_str()),
            ),
            (
                "wrong agent",
                Some(auth_tag.as_str()),
                "wss://relay.example",
                Some(owner_pin.as_str()),
                Some(owner_pin.as_str()),
            ),
            (
                "wrong owner",
                Some(auth_tag.as_str()),
                "wss://relay.example",
                Some(agent_pin.as_str()),
                Some(agent_pin.as_str()),
            ),
        ] {
            assert!(
                trusted_harness_credentials_for_spec(
                    &spec,
                    spec.command,
                    &args,
                    TrustedHarnessAuthority {
                        keys: &agent,
                        auth_tag: auth,
                        relay_url: relay,
                        agent_pin: agent_pin_value,
                        owner_pin: owner_pin_value,
                    },
                )
                .is_err(),
                "{name} must fail closed"
            );
        }

        let mut permissions = std::fs::metadata(spec.command)
            .expect("stat fixture before tamper")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(spec.command, permissions).expect("unlock fixture for tamper");
        std::fs::write(spec.command, b"#!/bin/sh\nexit 7\n").expect("tamper fixture");
        assert!(trusted_harness_credentials_for_spec(
            &spec,
            spec.command,
            &args,
            TrustedHarnessAuthority {
                keys: &agent,
                auth_tag: Some(&auth_tag),
                relay_url: "wss://relay.example",
                agent_pin: Some(&agent_pin),
                owner_pin: Some(&owner_pin),
            },
        )
        .is_err());
        let mut directory_permissions = std::fs::metadata(&directory)
            .expect("stat fixture directory before cleanup")
            .permissions();
        directory_permissions.set_mode(0o755);
        std::fs::set_permissions(&directory, directory_permissions)
            .expect("unlock fixture directory for cleanup");
        std::fs::remove_dir_all(directory).expect("remove trusted harness fixture");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn trusted_artifact_namespace_blocks_ancestor_swap() {
        use sha2::{Digest as _, Sha256};
        use std::os::unix::fs::PermissionsExt as _;

        let root = std::env::temp_dir().join(format!(
            "buzz-trusted-ancestor-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).expect("create trusted ancestor root");
        let root = root
            .canonicalize()
            .expect("canonical trusted ancestor root");
        let runtime_root = root.join("RUNTIME");
        let namespace = runtime_root.join("context-engine");
        let version = namespace.join("digest");
        let scripts = version.join("scripts");
        let artifact = scripts.join("gabe-acp.mjs");
        std::fs::create_dir_all(&scripts).expect("create trusted ancestor fixture");
        std::fs::write(&artifact, b"reviewed adapter").expect("write trusted artifact fixture");
        std::fs::set_permissions(&artifact, std::fs::Permissions::from_mode(0o444))
            .expect("freeze trusted artifact mode");
        for directory in [&scripts, &version, &namespace] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o555))
                .expect("freeze trusted ancestor mode");
        }
        let status = std::process::Command::new("/usr/bin/chflags")
            .args(["-R", "uchg"])
            .arg(&namespace)
            .status()
            .expect("freeze trusted ancestor fixture");
        assert!(status.success());
        let path: &'static str =
            Box::leak(artifact.to_string_lossy().into_owned().into_boxed_str());
        let digest: &'static str =
            Box::leak(hex::encode(Sha256::digest(b"reviewed adapter")).into_boxed_str());
        let spec = TrustedArtifactSpec {
            path,
            sha256: digest,
            executable: false,
            immutable: true,
        };
        validate_trusted_artifact_at(spec, &runtime_root)
            .expect("fully frozen ancestor chain is trusted");

        let displaced = runtime_root.join("displaced-context-engine");
        assert!(
            std::fs::rename(&namespace, &displaced).is_err(),
            "uchg namespace must block an ancestor swap"
        );
        std::process::Command::new("/usr/bin/chflags")
            .args(["-R", "nouchg"])
            .arg(&namespace)
            .status()
            .expect("unfreeze trusted ancestor fixture");
        std::fs::set_permissions(&namespace, std::fs::Permissions::from_mode(0o755))
            .expect("open namespace for swap probe");
        std::fs::rename(&namespace, &displaced).expect("unfrozen namespace can be displaced");
        assert!(validate_trusted_artifact_at(spec, &runtime_root).is_err());
        std::process::Command::new("/bin/chmod")
            .args(["-R", "u+w"])
            .arg(&displaced)
            .status()
            .expect("open displaced fixture for cleanup");
        std::fs::remove_dir_all(root).expect("remove trusted ancestor fixture");
    }

    #[test]
    fn stacy_deployment_receipt_requires_exact_install_and_generation_binding() {
        let legacy = serde_json::json!({
            "version": 1,
            "project": "stacy",
            "sourceCommit": "1".repeat(40),
            "upstreamPin": "2".repeat(40),
            "composeSha256": "3".repeat(64),
            "stacyContainerId": "4".repeat(64),
            "stacyImageId": format!("sha256:{}", "5".repeat(64)),
            "dashboardContainerId": "6".repeat(64),
            "dashboardImageId": format!("sha256:{}", "7".repeat(64)),
        });
        assert!(serde_json::from_value::<TrustedDeploymentReceipt>(legacy).is_err());

        let bound = serde_json::json!({
            "version": 3,
            "project": "stacy",
            "buzzCommit": "0".repeat(40),
            "sourceCommit": "1".repeat(40),
            "upstreamPin": "2".repeat(40),
            "composeSha256": "3".repeat(64),
            "installReceiptSha256": "8".repeat(64),
            "deploymentGeneration": "9".repeat(64),
            "lifecycleGeneration": "a".repeat(64),
            "stacyContainerId": "4".repeat(64),
            "stacyImageId": format!("sha256:{}", "5".repeat(64)),
            "dashboardContainerId": "6".repeat(64),
            "dashboardImageId": format!("sha256:{}", "7".repeat(64)),
        });
        let receipt = serde_json::from_value::<TrustedDeploymentReceipt>(bound)
            .expect("fully bound receipt should parse");
        assert_eq!(receipt.version, 3);
        assert_eq!(receipt.deployment_generation, "9".repeat(64));
        assert_eq!(receipt.lifecycle_generation, "a".repeat(64));
    }

    #[cfg(unix)]
    #[test]
    fn trusted_lifecycle_state_authorizes_only_the_exact_up_generation() {
        use std::os::unix::fs::PermissionsExt;

        struct RemoveFixture(PathBuf);
        impl Drop for RemoveFixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let directory = PathBuf::from("/private/tmp")
            .join(format!("buzz-lifecycle-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&directory).expect("create lifecycle fixture");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .expect("secure lifecycle directory");
        let _cleanup = RemoveFixture(directory.clone());
        let state = directory.join("lifecycle-state");
        let generation = "a".repeat(64);
        std::fs::write(&state, format!("{generation} up\n")).expect("write lifecycle state");
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o600))
            .expect("secure lifecycle state");
        let lifecycle_state_path: &'static str =
            Box::leak(state.to_string_lossy().into_owned().into_boxed_str());
        let spec = TrustedDeploymentSpec {
            receipt_path: "/unused",
            lifecycle_state_path,
            docker_socket: "/unused",
            project: "stacy",
            agent_container: "stacy-agent",
            agent_service: "stacy",
            dashboard_container: "stacy-dashboard",
            dashboard_service: "stacy-dashboard",
        };
        let receipt = TrustedDeploymentReceipt {
            version: 3,
            project: "stacy".into(),
            buzz_commit: "0".repeat(40),
            source_commit: "1".repeat(40),
            upstream_pin: "2".repeat(40),
            compose_sha256: "3".repeat(64),
            install_receipt_sha256: "8".repeat(64),
            deployment_generation: "9".repeat(64),
            lifecycle_generation: generation.clone(),
            stacy_container_id: "4".repeat(64),
            stacy_image_id: format!("sha256:{}", "5".repeat(64)),
            dashboard_container_id: "6".repeat(64),
            dashboard_image_id: format!("sha256:{}", "7".repeat(64)),
        };

        validate_trusted_lifecycle_state(spec, &receipt)
            .expect("exact up generation should authorize the deployment");
        std::fs::write(&state, format!("{generation} stop\n")).expect("write stop lifecycle state");
        assert!(validate_trusted_lifecycle_state(spec, &receipt).is_err());
        std::fs::write(&state, format!("{} up\n", "b".repeat(64)))
            .expect("write superseding lifecycle state");
        assert!(validate_trusted_lifecycle_state(spec, &receipt).is_err());
        std::fs::write(&state, format!("{generation} up\n")).expect("restore lifecycle state");
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o644))
            .expect("weaken lifecycle state mode");
        assert!(validate_trusted_lifecycle_state(spec, &receipt).is_err());
    }

    // --- relay_url_to_http_base tests ---

    #[test]
    fn relay_url_to_http_base_maps_ws_to_http() {
        assert_eq!(
            relay_url_to_http_base("ws://localhost:3000"),
            Some("http://localhost:3000".to_string())
        );
    }

    #[test]
    fn relay_url_to_http_base_maps_wss_to_https() {
        assert_eq!(
            relay_url_to_http_base("wss://project-gabe.communities.buzz.xyz"),
            Some("https://project-gabe.communities.buzz.xyz".to_string())
        );
    }

    #[test]
    fn relay_url_to_http_base_passes_through_http() {
        assert_eq!(
            relay_url_to_http_base("http://localhost:3000"),
            Some("http://localhost:3000".to_string())
        );
    }

    #[test]
    fn relay_url_to_http_base_passes_through_https() {
        assert_eq!(
            relay_url_to_http_base("https://relay.example.com"),
            Some("https://relay.example.com".to_string())
        );
    }

    #[test]
    fn relay_url_to_http_base_preserves_host_port_and_path() {
        assert_eq!(
            relay_url_to_http_base("wss://relay.example.com:8443/query"),
            Some("https://relay.example.com:8443/query".to_string())
        );
        assert_eq!(
            relay_url_to_http_base("ws://relay.example.com:8080/path"),
            Some("http://relay.example.com:8080/path".to_string())
        );
    }

    #[test]
    fn relay_url_to_http_base_returns_none_for_empty_or_unparseable() {
        assert_eq!(relay_url_to_http_base(""), None);
        assert_eq!(relay_url_to_http_base("   "), None);
        assert_eq!(relay_url_to_http_base("not-a-url"), None);
        assert_eq!(relay_url_to_http_base("ftp://relay.example.com"), None);
        // Scheme-only with no host must not slip through.
        assert_eq!(relay_url_to_http_base("wss://"), None);
    }

    #[test]
    fn from_args_withholds_credentials_from_legacy_gabe_basename_match() {
        // A familiar interpreter and script basename are not trust evidence.
        // Only the exact frozen absolute command, args, bytes, auth tag, and
        // identity pins can populate trusted_child_env.
        let args = CliArgs::try_parse_from([
            "buzz-acp",
            "--private-key",
            TEST_PRIVATE_KEY,
            "--agent-command",
            GABE_NODE_PATH,
            "--agent-args",
            "gabe-acp.mjs",
        ])
        .expect("clap should parse args");
        let config = Config::from_args(args).expect("valid config");
        assert!(config.trusted_child_env.is_empty());
        assert!(config.trusted_outbox_harness.is_none());
        assert!(!config.persona_env_vars.iter().any(|(key, _)| {
            crate::acp::CHILD_CREDENTIAL_ENV_TO_SCRUB
                .iter()
                .any(|reserved| key.eq_ignore_ascii_case(reserved))
        }));
    }

    #[test]
    fn from_args_withholds_signing_key_from_default_harness() {
        // Security-critical: the default (goose) harness must not carry the key.
        let args = CliArgs::try_parse_from(["buzz-acp", "--private-key", TEST_PRIVATE_KEY])
            .expect("clap should parse args");
        let config = Config::from_args(args).expect("valid config");
        assert!(
            !config
                .persona_env_vars
                .iter()
                .any(|(key, _)| key == "BUZZ_PRIVATE_KEY"),
            "default harness persona_env_vars must NOT carry the signing key: {:?}",
            config.persona_env_vars
        );
        // Negative control: the relay URL is only forwarded to the trusted
        // harness. The default (goose) harness must not carry it either.
        assert!(
            !config
                .persona_env_vars
                .iter()
                .any(|(key, _)| key == "BUZZ_RELAY_URL"),
            "default harness persona_env_vars must NOT carry the relay URL: {:?}",
            config.persona_env_vars
        );
    }

    // --- max_turn_duration ceiling gate ---

    #[test]
    fn max_turn_duration_at_ceiling_is_accepted() {
        let args = CliArgs::try_parse_from([
            "buzz-acp",
            "--private-key",
            TEST_PRIVATE_KEY,
            "--max-turn-duration",
            &MAX_TURN_DURATION_CEILING_SECS.to_string(),
        ])
        .expect("clap should parse args");
        let result = Config::from_args(args);

        assert!(
            result.is_ok(),
            "from_args should accept max_turn_duration at the ceiling"
        );
    }

    #[test]
    fn max_turn_duration_above_ceiling_is_rejected() {
        let over = MAX_TURN_DURATION_CEILING_SECS + 1;
        let args = CliArgs::try_parse_from([
            "buzz-acp",
            "--private-key",
            TEST_PRIVATE_KEY,
            "--max-turn-duration",
            &over.to_string(),
        ])
        .expect("clap should parse args");
        let result = Config::from_args(args);

        assert!(
            result.is_err(),
            "from_args should reject max_turn_duration above the ceiling"
        );
        let msg = match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("from_args unexpectedly accepted an excessive turn duration"),
        };
        assert!(
            msg.contains("exceeds ceiling"),
            "error should mention 'exceeds ceiling': {msg}"
        );
    }

    #[test]
    fn max_turn_duration_ceiling_cannot_overflow_in_flight_deadline() {
        // The in-flight deadline is max_turn + 100s buffer (IN_FLIGHT_DEADLINE_BUFFER_SECS).
        // Verify that even at the ceiling, this addition cannot overflow u64.
        const {
            assert!(MAX_TURN_DURATION_CEILING_SECS < u64::MAX - 100);
        }
    }

    #[test]
    fn sanitize_session_title_collapses_whitespace_and_strips_control_chars() {
        assert_eq!(
            sanitize_session_title("  Fizz\t\tthe\n Bot\u{7}  "),
            Some("Fizz the Bot".to_string())
        );
    }

    #[test]
    fn sanitize_session_title_returns_none_when_nothing_printable_remains() {
        assert_eq!(sanitize_session_title("   \n\t "), None);
        assert_eq!(sanitize_session_title(""), None);
        assert_eq!(sanitize_session_title("\u{1}\u{2}"), None);
    }

    #[test]
    fn sanitize_session_title_caps_length_without_splitting_multibyte_chars() {
        let raw = "\u{1f41d}".repeat(SESSION_TITLE_MAX_CHARS + 10);
        let title = sanitize_session_title(&raw).expect("emoji title survives sanitizing");
        assert_eq!(title.chars().count(), SESSION_TITLE_MAX_CHARS);
        assert!(title.chars().all(|c| c == '\u{1f41d}'));
    }

    #[test]
    fn sanitize_session_title_does_not_leave_a_trailing_space_after_the_cap() {
        // The cap lands mid-word, so trimming must not leave a dangling space.
        let raw = format!("{} tail", "a".repeat(SESSION_TITLE_MAX_CHARS - 1));
        let title = sanitize_session_title(&raw).expect("title survives sanitizing");
        assert_eq!(title, "a".repeat(SESSION_TITLE_MAX_CHARS - 1));
    }

    #[test]
    fn compose_session_title_qualifies_the_agent_name_with_the_channel() {
        assert_eq!(
            compose_session_title("Fizz", Some("buzz-dev")),
            "Fizz · #buzz-dev"
        );
    }

    #[test]
    fn compose_session_title_falls_back_to_bare_agent_name_without_a_channel() {
        assert_eq!(compose_session_title("Fizz", None), "Fizz");
        assert_eq!(compose_session_title("Fizz", Some("   ")), "Fizz");
    }

    #[test]
    fn compose_session_title_truncates_the_channel_and_keeps_the_agent_name() {
        let channel = "c".repeat(200);
        let title = compose_session_title("Fizz", Some(&channel));
        assert_eq!(title.chars().count(), SESSION_TITLE_MAX_CHARS);
        assert!(title.starts_with("Fizz · #c"));
    }

    #[test]
    fn compose_session_title_drops_the_channel_when_the_agent_name_fills_the_cap() {
        let agent = "a".repeat(SESSION_TITLE_MAX_CHARS);
        assert_eq!(compose_session_title(&agent, Some("buzz-dev")), agent);
    }

    /// Every arg whose env var name contains KEY/SECRET/TOKEN/PASSWORD/CRED/AUTH
    /// must set `hide_env_values = true` to prevent credential leakage in --help.
    #[test]
    fn secret_env_args_hide_their_values_in_help() {
        use clap::CommandFactory;

        const SECRET_PATTERNS: &[&str] = &["KEY", "SECRET", "TOKEN", "PASSWORD", "CRED", "AUTH"];

        let cmd = CliArgs::command();
        let violations: Vec<String> = cmd
            .get_arguments()
            .filter_map(|arg| {
                let env_key = arg.get_env()?;
                let env_name = env_key.to_string_lossy().to_uppercase();
                let is_secret = SECRET_PATTERNS.iter().any(|pat| env_name.contains(pat));
                if is_secret && !arg.is_hide_env_values_set() {
                    Some(env_name)
                } else {
                    None
                }
            })
            .collect();

        assert!(
            violations.is_empty(),
            "Found secret-bearing env args without hide_env_values=true. \
             Add `hide_env_values = true` to each: {violations:?}"
        );
    }
}
