//! `deploy`: provision the agent as a `systemd --user` unit on the host.
//!
//! `--user` rather than a system unit keeps the flow root-free and puts the env
//! file beside the harness credentials that already live in the deploying
//! user's home (`~/.claude`, `~/.config/goose`).
//!
//! Deploy is also the *start* path — `start_managed_agent` re-enters
//! `deploy_to_provider` — so everything here must be idempotent.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::install::{self, Payload, Tool};
use crate::protocol::{Failure, Secret, SshConfig};
use crate::ssh::{quote, Session};

/// The templated unit, installed once per host and instantiated per agent.
const UNIT_TEMPLATE: &str = include_str!("../assets/buzz-acp@.service");

/// Verbatim copy of the desktop's `env_vars::RESERVED_ENV_KEYS`. The desktop
/// already strips these from user env; re-checking here means a leak needs two
/// independent failures rather than one, and this binary ships and updates
/// separately from the desktop that fills the payload.
const RESERVED_ENV_KEYS: &[&str] = &[
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_API_TOKEN",
    "BUZZ_ACP_PRIVATE_KEY",
    "BUZZ_ACP_API_TOKEN",
    "BUZZ_RELAY_URL",
    "BUZZ_ACP_AGENT_COMMAND",
    "BUZZ_ACP_AGENT_ARGS",
    "BUZZ_ACP_MCP_COMMAND",
    "CLAUDE_CODE_EXECUTABLE",
    "BUZZ_ACP_RESPOND_TO",
    "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
    "BUZZ_ACP_AGENT_OWNER",
    "BUZZ_ACP_SETUP_PAYLOAD",
    "BUZZ_MANAGED_AGENT",
    "BUZZ_MANAGED_AGENT_START_NONCE",
];

/// The deploy payload, as `deploy_payload_json` serializes it.
///
/// Deliberately not `Debug`. `Secret` redacts itself, but `env_vars` routinely
/// holds `ANTHROPIC_API_KEY` and friends in plain `String`s, so a derived
/// `Debug` would put provider credentials one `{:?}` away from a log line.
pub struct Agent {
    pub name: String,
    /// The agent's minted Nostr pubkey — the desktop record's own primary key,
    /// and the only stable identifier in the payload. See [`Agent::slug`].
    pub pubkey: String,
    pub relay_url: String,
    pub private_key_nsec: Secret,
    pub auth_tag: Option<String>,
    /// The pinned harness command. See [`Agent::from_request`].
    pub agent_command: String,
    pub agent_args: Vec<String>,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub idle_timeout_seconds: Option<u64>,
    pub max_turn_duration_seconds: Option<u64>,
    pub parallelism: u64,
    pub respond_to: String,
    pub respond_to_allowlist: Vec<String>,
    pub env_vars: BTreeMap<String, String>,
    /// A path on the **desktop** machine to a Linux `buzz-acp` to install on
    /// the host when the host resolves none. Optional, and absent it changes
    /// nothing: deploy resolves `buzz-acp` on the host or fails with exit 90
    /// exactly as it always has. See [`crate::install`].
    pub buzz_acp_binary: Option<String>,
    /// The same, for the `buzz` CLI. A remote agent's own system prompt tells
    /// it to answer with `buzz messages send --reply-to …`, and a local agent
    /// gets that command because the desktop bundles the CLI and prepends
    /// `~/.local/bin` to the spawned harness's `PATH`. This field is how the
    /// remote side reaches the same parity.
    ///
    /// Unlike `buzz_acp_binary`, its absence on a host that has no CLI is a
    /// warning rather than a failure: the harness runs without it.
    pub buzz_cli_binary: Option<String>,
}

impl Agent {
    pub fn from_request(request: &serde_json::Value) -> Result<Self, String> {
        let agent = request.get("agent").ok_or("request is missing 'agent'")?;
        let string = |key: &str| {
            agent
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };

        let private_key_nsec: Secret = agent
            .get("private_key_nsec")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|_| "'private_key_nsec' must be a string".to_string())?
            .unwrap_or_default();
        // Fail closed. A deploy that lets the host mint its own key produces an
        // agent that looks deployed and is permanently unreachable: presence,
        // mentions, `!shutdown`, badges and the NIP-OA auth tag all key off the
        // pubkey the desktop minted. Mirrors the desktop's own
        // `spawn_key_refusal`.
        if private_key_nsec.is_empty() {
            return Err(
                "refusing to deploy without the agent's minted private key: the remote agent \
                 would run under an identity no desktop surface recognizes"
                    .to_string(),
            );
        }

        // The remote harness choice reaches the host ONLY as this pin — the
        // desktop resolves it from the remote catalog at create time and ships
        // it verbatim. A blank value means the pin was lost on the way, and the
        // host would silently run `buzz-agent` instead of the harness the user
        // picked, so refuse rather than substitute.
        let agent_command = string("agent_command").ok_or(
            "deploy payload carries no 'agent_command': the harness pin was lost before it \
             reached the host (see instanceInputForDefinition provider branch)",
        )?;

        // The one stable identifier in the payload, and what every host-side
        // name is keyed on. Refused rather than defaulted for the same reason
        // the minted key is: without it two agents that merely share a display
        // name would share one unit, one env file and one `backend_agent_id`,
        // and the second deploy would overwrite the first agent's identity.
        //
        // Validated to the shape the desktop always mints (`to_hex()` of a
        // Nostr public key) because the fragment taken from it becomes a
        // filename and a systemd instance name. Anything else is a payload bug,
        // and sanitizing it would trade a loud failure for a silent collision.
        let pubkey = string("pubkey").ok_or(
            "deploy payload carries no 'pubkey': the remote unit would be keyed on the agent's \
             display name, which two agents can share",
        )?;
        if pubkey.len() != 64 || !pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "'pubkey' is not a 64-character hex Nostr public key: {} characters",
                pubkey.len()
            ));
        }
        let pubkey = pubkey.to_lowercase();

        Ok(Self {
            name: string("name").ok_or("'name' is required")?,
            pubkey,
            relay_url: string("relay_url").ok_or("'relay_url' is required")?,
            private_key_nsec,
            auth_tag: string("auth_tag"),
            agent_command,
            // `agent_args` must be the remote entry's default args. The
            // desktop's local branch sends `[]` on purpose so spawn re-resolves
            // them live, but a provider-backed record never spawns locally, so
            // `[]` here would mean "no args" for any harness the local
            // default-args table does not know.
            agent_args: crate::discover::string_list(agent.get("agent_args")),
            system_prompt: string("system_prompt"),
            model: string("model"),
            provider: string("provider"),
            // `turn_timeout_seconds` is deliberately not read: the payload still
            // carries it, but `BUZZ_ACP_TURN_TIMEOUT` is deprecated and ignored
            // by the harness (`buzz-acp::config`), and local spawn does not
            // write it either. `idle_timeout_seconds` and
            // `max_turn_duration_seconds` are the live controls.
            idle_timeout_seconds: agent.get("idle_timeout_seconds").and_then(|v| v.as_u64()),
            max_turn_duration_seconds: agent
                .get("max_turn_duration_seconds")
                .and_then(|v| v.as_u64()),
            parallelism: agent
                .get("parallelism")
                .and_then(|v| v.as_u64())
                .filter(|p| *p > 0)
                .unwrap_or(1),
            respond_to: string("respond_to").unwrap_or_else(|| "owner-only".to_string()),
            respond_to_allowlist: crate::discover::string_list(agent.get("respond_to_allowlist")),
            env_vars: env_map(agent.get("env_vars")),
            // Read from the same `agent` block as everything else, but neither
            // is agent configuration: nothing about them reaches the env file
            // or the unit. They are the desktop handing the provider copies of
            // the host-side tools to install if the host turns out not to have
            // them.
            buzz_acp_binary: string("buzz_acp_binary"),
            buzz_cli_binary: string("buzz_cli_binary"),
        })
    }

    /// The systemd instance name, and the `agent_id` the desktop persists in
    /// `record.backend_agent_id`.
    ///
    /// It becomes both a filename and a unit instance name, so it follows the
    /// desktop's own `util::slugify` rule. The name is the readable half; the
    /// **pubkey fragment is the identity**, and it is what makes the name safe
    /// to read: two agents may legitimately be called "Research Bot" on one SSH
    /// account, and keying on the name alone gave them one unit, one env file
    /// and one `backend_agent_id` — so the second deploy silently overwrote the
    /// first agent's minted nsec, and starting either record then drove
    /// whichever identity was written last.
    ///
    /// [`PUBKEY_FRAGMENT`] characters of a 256-bit key are far more than a
    /// per-host unit namespace needs to stay collision-free, and short enough
    /// to leave the name legible in `systemctl --user status`.
    pub fn slug(&self) -> String {
        let sanitized: String = self
            .name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        // ASCII by construction, so a byte slice cannot split a character.
        let stem = sanitized.trim_matches('-');
        let stem = &stem[..stem.len().min(32)];
        let stem = stem.trim_end_matches('-');
        let stem = if stem.is_empty() { "agent" } else { stem };
        // Hex and lowercased by `from_request`, so this is already unit-safe.
        format!("{stem}-{}", &self.pubkey[..PUBKEY_FRAGMENT])
    }

    pub fn agent_id(&self) -> String {
        format!("buzz-acp@{}", self.slug())
    }
}

/// How much of the agent's pubkey identifies its unit. See [`Agent::slug`].
const PUBKEY_FRAGMENT: usize = 12;

pub fn env_map(value: Option<&serde_json::Value>) -> BTreeMap<String, String> {
    value
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// A well-formed POSIX env var name. Mirrors the desktop's own boundary check;
/// a malformed key would let a value smuggle an extra assignment into the file,
/// or — on the left side of a shell `export`, where quoting cannot help — an
/// extra command.
pub fn is_well_formed_env_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One `KEY="value"` line for a systemd `EnvironmentFile`.
///
/// systemd unquotes C-style escapes inside double quotes, so `\` and `"` are
/// the two characters that must be escaped. Control characters are refused
/// outright: a newline would split the assignment into a second, attacker-
/// chosen line.
fn env_line(key: &str, value: &str) -> Result<String, String> {
    if value.chars().any(|c| c.is_control()) {
        return Err(format!(
            "env var '{key}' contains a control character and cannot be written to the unit's \
             environment file"
        ));
    }
    let escaped = value.replace('\\', r"\\").replace('"', "\\\"");
    Ok(format!("{key}=\"{escaped}\"\n"))
}

/// The env file body: the local spawn contract from `runtime.rs`, transcribed.
///
/// Values resolved on the host — the absolute harness path, `buzz-acp` itself,
/// `git-credential-nostr`, `PATH` — are appended by the remote script, not
/// here. Everything in this string is known locally.
fn env_file_body(agent: &Agent) -> Result<String, String> {
    let mut body = String::new();
    let mut push = |key: &str, value: &str| -> Result<(), String> {
        body.push_str(&env_line(key, value)?);
        Ok(())
    };

    push("BUZZ_PRIVATE_KEY", agent.private_key_nsec.expose())?;
    push("BUZZ_RELAY_URL", &agent.relay_url)?;
    if let Some(auth_tag) = &agent.auth_tag {
        push("BUZZ_AUTH_TAG", auth_tag)?;
    }
    push("BUZZ_ACP_AGENT_ARGS", &agent.agent_args.join(","))?;
    // MCP does not reach the host yet: `mcp_command` is local catalog metadata,
    // and mirroring that table here would drift. Empty rather than omitted,
    // matching what local spawn writes when it does not apply.
    push("BUZZ_ACP_MCP_COMMAND", "")?;
    // Lazy defers the *pool warm*, not the process: buzz-acp connects,
    // subscribes, and queues accepted work, then the first flushable event
    // wakes all `BUZZ_ACP_AGENTS` slots. Nothing is dropped, and the one-shot
    // cold start is cheaper than what eager costs here — a `Restart=always`
    // unit re-pays N serial spawns on every restart, and a deployed-but-idle
    // agent holds N harness subprocesses that are never reaped. That makes
    // this the restore case (see `restore.rs`, "eager on restore buys
    // nothing"), not the interactive-create case that spawns eager locally.
    push("BUZZ_ACP_LAZY_POOL", "true")?;
    push("BUZZ_ACP_AGENTS", &agent.parallelism.to_string())?;
    push("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "steer")?;
    push("BUZZ_ACP_DEDUP", "queue")?;
    push("BUZZ_ACP_RELAY_OBSERVER", "true")?;
    push("BUZZ_ACP_RESPOND_TO", &agent.respond_to)?;
    if agent.respond_to == "allowlist" {
        if agent.respond_to_allowlist.is_empty() {
            return Err(
                "respond-to mode 'allowlist' requires at least one pubkey in the allowlist"
                    .to_string(),
            );
        }
        push(
            "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
            &agent.respond_to_allowlist.join(","),
        )?;
    }
    if let Some(prompt) = &agent.system_prompt {
        push("BUZZ_ACP_SYSTEM_PROMPT", prompt)?;
    }
    if let Some(model) = &agent.model {
        push("BUZZ_ACP_MODEL", model)?;
    }
    // The harness-native half of the same selection: `BUZZ_ACP_MODEL` is what
    // buzz-acp reads, these are what the harness underneath it reads, and local
    // spawn writes both.
    for (key, value) in metadata_env(agent) {
        push(key, value)?;
    }
    // Only when the user set them, so the harness's own defaults win otherwise.
    if let Some(idle) = agent.idle_timeout_seconds {
        push("BUZZ_ACP_IDLE_TIMEOUT", &idle.to_string())?;
    }
    if let Some(max_turn) = agent.max_turn_duration_seconds {
        push("BUZZ_ACP_MAX_TURN_DURATION", &max_turn.to_string())?;
    }

    // `BUZZ_MANAGED_AGENT` is deliberately absent: it is the desktop's marker
    // for reclaiming orphaned local children, and systemd owns this lifecycle.

    // User env last, so it overrides everything above — systemd applies the
    // later assignment for a repeated key, matching the local layering.
    for (key, value) in &agent.env_vars {
        if !is_well_formed_env_key(key) {
            return Err(format!("env var name '{key}' is not a valid identifier"));
        }
        if RESERVED_ENV_KEYS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(key))
        {
            return Err(format!(
                "env var '{key}' is reserved and cannot be overridden"
            ));
        }
        push(key, value)?;
    }
    Ok(body)
}

/// The remote half of `runtime_metadata_env_vars` (`runtime.rs`).
///
/// Local spawn writes the effective model and provider into each runtime's own
/// `model_env_var` / `provider_env_var`. Without this a remote Goose would see
/// `BUZZ_ACP_MODEL` but no `GOOSE_MODEL`, and fall back to whatever
/// `~/.config/goose/config.yaml` on the host says — the user's model pick
/// silently ignored.
///
/// Keyed by command rather than harness id because the id is a create-time
/// desktop concept and the env file is written from the pin. Runtimes absent
/// here (Claude, Codex) declare no such vars in `KNOWN_ACP_RUNTIMES` either.
fn metadata_env(agent: &Agent) -> Vec<(&'static str, &str)> {
    const RUNTIME_ENV: &[(&str, &str, &str)] = &[
        ("goose", "GOOSE_MODEL", "GOOSE_PROVIDER"),
        ("buzz-agent", "BUZZ_AGENT_MODEL", "BUZZ_AGENT_PROVIDER"),
    ];

    // The pin may be a bare name or an absolute path; local spawn's
    // `known_acp_runtime` matches on the file name either way.
    let command = agent
        .agent_command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&agent.agent_command);
    let Some((_, model_key, provider_key)) =
        RUNTIME_ENV.iter().find(|(name, _, _)| *name == command)
    else {
        return Vec::new();
    };

    [
        (*model_key, agent.model.as_deref()),
        (*provider_key, agent.provider.as_deref()),
    ]
    .into_iter()
    .filter_map(|(key, value)| Some((key, value?)))
    .collect()
}

/// `wss://relay` → `https://relay`, for the git credential helper's scope.
fn relay_http_base_url(relay_url: &str) -> String {
    let trimmed = relay_url.trim().trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        trimmed.to_string()
    }
}

/// The optional desktop-side copies of the host-side tools, already read and
/// encoded. Both default to absent, which is the case in which nothing about
/// the deploy changes.
#[derive(Default)]
struct Pushes {
    acp: Option<Payload>,
    cli: Option<Payload>,
}

/// The `buzz-acp` command to resolve on the host: the operator's pin, or the
/// bare name. Shared by the probe and the deploy script so the two ask the same
/// question — see `install::resolve`.
fn acp_command(config: &SshConfig) -> String {
    quote(config.buzz_acp_path.as_deref().unwrap_or(install::ACP.name))
}

/// The remote script. One round trip: resolve (or install), write, install,
/// start.
///
/// Every secret reaches the host inside this script, which travels on the SSH
/// stdin channel. Nothing secret is ever an argument — not to `ssh`, and not to
/// any command the script runs — because the remote `ps` is world-readable. The
/// env file is written under `umask 077`, `chmod 600`, and moved into place
/// atomically.
///
/// `push` carries the optional desktop-side copies of the two host-side tools.
/// `buzz-acp` is resolved *first*, so `$acp` — and therefore the unit's
/// `ExecStart` — names the copy this same pass installed. The pushed bytes are
/// not secret, but they share the stream with the minted nsec, so they travel
/// base64-encoded and never as raw bytes (`install`).
///
/// The `buzz` CLI is resolved in the same pass and by the same machinery, with
/// one deliberate difference: a host that has neither the CLI nor a pushed copy
/// gets a `WARNING:` line and the deploy continues, because the harness does
/// not depend on the CLI (`install::Missing`). Nothing substitutes `$cli` into
/// the unit — the CLI is reached through the env file's `PATH`, which is what
/// makes the install destination resolvable for the harness's children.
fn deploy_script(
    agent: &Agent,
    config: &SshConfig,
    unit: &str,
    push: &Pushes,
) -> Result<String, String> {
    let slug = agent.slug();
    let acp = acp_command(config);
    let command = quote(&agent.agent_command);
    let relay_http = relay_http_base_url(&agent.relay_url);
    let resolve_acp = install::resolve_or_install(install::ACP, &acp, push.acp.as_ref());
    let resolve_cli =
        install::resolve_or_install(install::CLI, &quote(install::CLI.name), push.cli.as_ref());

    let mut script = String::from("set -eu\numask 077\n");
    // The harness name is bound once and thereafter referenced only as
    // `"$harness_name"`. Interpolating it into the double-quoted error message
    // would be a command-injection hole: `quote()` makes a value inert as an
    // *argument*, but inside double quotes its single quotes are literal and a
    // `$(...)` would still run. Expansion results are not re-scanned.
    script.push_str(&format!(
        r#"harness_name={command}
{resolve_acp}
harness=$(command -v "$harness_name" 2>/dev/null) || {{ echo "harness $harness_name not found on the server's PATH" >&2; exit 91; }}
claude_cli=""
case "${{harness##*/}}" in
  claude-agent-acp|claude-code-acp)
    if [ -x "$HOME/.local/bin/claude" ]; then
      claude_cli="$HOME/.local/bin/claude"
    else
      claude_cli=$(command -v claude 2>/dev/null || true)
    fi
    if [ -z "$claude_cli" ]; then echo "Claude Code CLI not found in ~/.local/bin or on the server's PATH" >&2; exit 95; fi
    ;;
esac
{resolve_cli}
cred=$(command -v git-credential-nostr 2>/dev/null || true)
conf="$HOME/.config/buzz-acp"
units="$HOME/.config/systemd/user"
mkdir -p "$conf" "$units"
env_file="$conf/{slug}.env"
tmp="$env_file.new"
"#
    ));

    // No body line can terminate the heredoc: every line is `KEY="..."` and
    // `env_line` refuses control characters. The quoted delimiter suppresses
    // expansion, so a value is never interpreted by the shell.
    script.push_str("{\n");
    script.push_str("printf 'BUZZ_ACP_AGENT_COMMAND=\"%s\"\\n' \"$harness\"\n");
    // The harness's `PATH`, and the reason an installed `buzz` is a command the
    // agent can actually run.
    //
    // This is the remote half of the desktop's own contract: local spawn
    // prepends `~/.local/bin` to the spawned harness's `PATH` so the agent can
    // run the CLI its system prompt tells it to reply with
    // (`managed_agents::runtime::path::build_augmented_path`). The unit runs
    // under `systemd --user`, whose `PATH` is the user manager's — no profile,
    // no login shell, and on many distributions no `~/.local/bin` — so without
    // this line the install destination is a directory the harness cannot name,
    // and both tools would be installed and unreachable.
    //
    // It is composed HERE, by the host's shell, and not written into the unit
    // as `Environment=PATH=$HOME/.local/bin:$PATH`: systemd expands no variable
    // in `Environment=` or an `EnvironmentFile`, so that form would hand the
    // harness the five literal characters `$PATH`. `$PATH` on the right is the
    // non-interactive SSH shell's, captured at deploy time, which is the same
    // `PATH` `install::resolve` just searched — so anything `command -v` found
    // stays findable for the agent.
    //
    // `install::resolve` can only land on this `PATH` or on `~/.local/bin`, so
    // these two segments cover every copy either tool can resolve to; there is
    // nothing further to add from `$acp` or `$cli`.
    script.push_str("printf 'PATH=\"%s\"\\n' \"$HOME/.local/bin:$PATH\"\n");
    script.push_str("cat <<'BUZZ_ENV_EOF'\n");
    script.push_str(&env_file_body(agent)?);
    script.push_str("BUZZ_ENV_EOF\n");
    // Match local desktop spawn's `configure_runtime_cli`: the adapter
    // bundles a point-in-time Claude binary, while the native launcher follows
    // Claude Code updates. Preserve the stable launcher path rather than
    // resolving its symlink so every new ACP child inherits the current native
    // version. This is emitted after the user-env heredoc as an authoritative
    // provider binding; the key is also reserved so a payload cannot spoof it.
    script.push_str(
        "if [ -n \"$claude_cli\" ]; then\n\
printf 'CLAUDE_CODE_EXECUTABLE=\"%s\"\\n' \"$claude_cli\"\n\
fi\n",
    );
    // Git over the relay's NIP-98 endpoint, only when the helper is installed.
    // NOSTR_PRIVATE_KEY mirrors BUZZ_PRIVATE_KEY, as it does locally.
    let helper_key = format!("credential.{relay_http}/git.helper");
    let use_http_path_key = format!("credential.{relay_http}/git.useHttpPath");
    let git_block: String = [
        ("NOSTR_PRIVATE_KEY", agent.private_key_nsec.expose()),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_CONFIG_COUNT", "2"),
        ("GIT_CONFIG_KEY_0", helper_key.as_str()),
        ("GIT_CONFIG_KEY_1", use_http_path_key.as_str()),
        ("GIT_CONFIG_VALUE_1", "true"),
    ]
    .into_iter()
    .map(|(key, value)| env_line(key, value))
    .collect::<Result<_, _>>()?;
    script.push_str(&format!(
        r#"if [ -n "$cred" ]; then
printf 'GIT_CONFIG_VALUE_0="%s"\n' "$cred"
cat <<'BUZZ_GIT_EOF'
{git_block}BUZZ_GIT_EOF
fi
}} > "$tmp"
chmod 600 "$tmp"
mv "$tmp" "$env_file"
"#
    ));

    // Without lingering the agent dies when this SSH session ends, which reads
    // as a flaky agent rather than a configuration problem. It also creates
    // `/run/user/$(id -u)`, so it must precede anything that talks to the user
    // bus. Best-effort: some hosts gate it behind polkit, and failing it must
    // not fail an otherwise good deploy.
    //
    // A non-interactive SSH command often gets no `XDG_RUNTIME_DIR`, without
    // which every `systemctl --user` fails with "Failed to connect to bus".
    script.push_str(
        r#"loginctl enable-linger "$(id -un)" >/dev/null 2>&1 || true
if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
  XDG_RUNTIME_DIR="/run/user/$(id -u)"
  export XDG_RUNTIME_DIR
fi
"#,
    );

    // Install the templated unit, reloading only when its content changed — a
    // `daemon-reload` per start is noise, and a missing one after a change
    // silently runs the old unit.
    //
    // `@BUZZ_ACP_BIN@` is substituted with parameter expansion rather than
    // `sed -i`, a GNU extension BSD and macOS hosts reject, and any in-place
    // `s///` over the template would need a delimiter no resolved path can
    // contain.
    //
    // The substituted value is quoted per systemd's own command-line syntax,
    // which is not the shell's. `ExecStart=` splits an unquoted value on
    // whitespace, so a `buzz-acp path on the server` pointing at
    // `/opt/buzz tools/buzz-acp` would make systemd run `/opt/buzz` with
    // `tools/buzz-acp` as an argument — the schema accepts such a path, and
    // `quote()` already makes it safe as a *shell* argument, which is a
    // different question. Inside double quotes systemd unquotes C-style
    // escapes, so `\` and `"` are escaped first; a filtering `sed` is portable
    // even where `sed -i` is not.
    //
    // The quotes are literals in `printf`'s single-quoted format rather than
    // shell-escaped inside the value, so the three interpolations stay plain
    // `%s` arguments — the unit template's own `%i`/`%h` specifiers ride
    // through untouched for the same reason.
    script.push_str(&format!(
        r#"unit_file="$units/buzz-acp@.service"
acp_unit=$(printf '%s' "$acp" | sed 's/[\\"]/\\&/g')
template=$(cat <<'BUZZ_UNIT_EOF'
{unit}BUZZ_UNIT_EOF
)
printf '%s"%s"%s\n' "${{template%%@BUZZ_ACP_BIN@*}}" "$acp_unit" "${{template#*@BUZZ_ACP_BIN@}}" > "$unit_file.new"
if cmp -s "$unit_file.new" "$unit_file"; then
  rm -f "$unit_file.new"
else
  mv "$unit_file.new" "$unit_file"
  systemctl --user daemon-reload
fi
systemctl --user enable --now {service} >/dev/null
# Redeploy is also the start path, so an already-running unit must pick up the
# rewritten env file rather than be left on the old one.
systemctl --user restart {service}
"#,
        service = quote(&format!("buzz-acp@{slug}.service")),
    ));
    Ok(script)
}

/// The binaries to embed in this deploy's script, if any.
///
/// Empty whenever the payload names no path — the default, and the case in
/// which nothing about deploy changes. Otherwise the host is asked first which
/// tools it already resolves, because **deploy is the start path**: without the
/// probe, a desktop with the seams engaged would encode and stream tens of
/// megabytes on every agent start, forever, to a host that has had the binaries
/// since the first deploy. Reading the files is skipped in that case too.
///
/// One probe covers both tools, and it is skipped entirely when neither field
/// is set — so a payload with no push seams costs exactly the round trips it
/// always did.
///
/// A probe that cannot be answered is not fatal: the binaries are embedded and
/// the script's own resolution makes the real decision on the host.
fn payloads_to_push(
    agent: &Agent,
    config: &SshConfig,
    session: &Session,
) -> Result<Pushes, Failure> {
    let candidates = [
        (install::ACP, acp_command(config), &agent.buzz_acp_binary),
        (
            install::CLI,
            quote(install::CLI.name),
            &agent.buzz_cli_binary,
        ),
    ];
    let asked: Vec<(Tool, String)> = candidates
        .iter()
        .filter(|(_, _, path)| path.is_some())
        .map(|(tool, command, _)| (*tool, command.clone()))
        .collect();
    if asked.is_empty() {
        return Ok(Pushes::default());
    }

    let probe = session.run(&install::probe_script(&asked), Duration::from_secs(60))?;
    // Read and validate before the deploy script is built: a bad path, a non-ELF
    // file or an oversized one is the desktop's mistake, and it should be
    // reported as that rather than as a remote failure mid-provisioning. That
    // holds for the CLI too — the *absence* of a CLI is tolerable, but a
    // desktop that pointed the seam at the wrong file has a bug worth naming.
    let read = |tool: Tool, path: &Option<String>| -> Result<Option<Payload>, String> {
        match path.as_deref() {
            Some(path) if !install::probe_found(&probe.stdout, tool) => {
                Payload::read(tool, path).map(Some)
            }
            _ => Ok(None),
        }
    };
    Ok(Pushes {
        acp: read(install::ACP, &agent.buzz_acp_binary)?,
        cli: read(install::CLI, &agent.buzz_cli_binary)?,
    })
}

pub fn deploy(
    request: &serde_json::Value,
    config: &SshConfig,
    session: &Session,
) -> Result<serde_json::Value, Failure> {
    let agent = Agent::from_request(request)?;
    let push = payloads_to_push(&agent, config, session)?;
    let script = deploy_script(&agent, config, UNIT_TEMPLATE, &push)?;
    let output = session.run(&script, Duration::from_secs(300))?;
    if !output.ok() {
        return Err(output.failure().into());
    }
    // A successful deploy's remote stderr is otherwise dropped as host noise,
    // so the script's non-fatal complaints — today, "this host has no buzz CLI"
    // — would be invisible without this. They go to *this* process's stderr,
    // which `invoke_provider` logs on success and shows in the error on
    // failure, rather than into the response: the op succeeded, and a warning
    // is not a result.
    for warning in install::warnings(&output.stderr) {
        eprintln!("buzz-backend-ssh: {warning}");
    }
    Ok(serde_json::json!({ "ok": true, "agent_id": agent.agent_id() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NSEC: &str = "nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
    /// A 64-hex Nostr pubkey, in the shape `record.pubkey` always carries.
    const PUBKEY: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
    /// The fragment of [`PUBKEY`] every host-side name is keyed on.
    const PUBKEY_SLUG: &str = "3bf0c63fcb93";

    fn request() -> serde_json::Value {
        serde_json::json!({
            "op": "deploy",
            "provider_config": { "ssh_host": "vps", "ssh_user": "ubuntu" },
            "agent": {
                "name": "Research Bot",
                "pubkey": PUBKEY,
                "relay_url": "wss://relay.example/ws",
                "private_key_nsec": NSEC,
                "auth_tag": "tag-abc",
                "agent_command": "goose",
                "agent_args": ["acp"],
                "system_prompt": "be brief",
                "model": "claude-sonnet-5",
                "provider": "anthropic",
                "parallelism": 3,
                "respond_to": "owner-only",
                "respond_to_allowlist": [],
                "env_vars": { "ANTHROPIC_API_KEY": "sk-ant-secret" },
            },
        })
    }

    fn config() -> SshConfig {
        SshConfig {
            host: "vps".into(),
            ..SshConfig::default()
        }
    }

    /// `Agent` is intentionally not `Debug` (see its doc comment), so tests
    /// unwrap the error by hand rather than through `unwrap_err`.
    fn rejection(request: &serde_json::Value) -> String {
        match Agent::from_request(request) {
            Err(error) => error,
            Ok(agent) => panic!("expected a rejection, got agent {}", agent.agent_id()),
        }
    }

    #[test]
    fn deploy_fails_closed_without_the_minted_key() {
        let mut request = request();
        request["agent"]["private_key_nsec"] = serde_json::json!("");
        let error = rejection(&request);
        assert!(error.contains("minted private key"), "{error}");

        request["agent"]
            .as_object_mut()
            .unwrap()
            .remove("private_key_nsec");
        assert!(rejection(&request).contains("minted private key"));
    }

    #[test]
    fn deploy_refuses_a_payload_whose_harness_pin_was_lost() {
        // Without the pin the host would fall back to `buzz-agent` and the
        // user's harness choice would vanish silently.
        let mut request = request();
        request["agent"]["agent_command"] = serde_json::json!("");
        let error = rejection(&request);
        assert!(error.contains("harness pin"), "{error}");
    }

    #[test]
    fn the_pinned_harness_is_what_the_unit_runs() {
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        // Resolved to an absolute path on the HOST, and written into the env
        // file systemd re-reads on every restart — that is what makes the pin
        // durable rather than a one-shot argument.
        assert!(script.contains("harness_name='goose'"));
        assert!(script.contains(r#"harness=$(command -v "$harness_name""#));
        assert!(script.contains("printf 'BUZZ_ACP_AGENT_COMMAND=\"%s\"\\n' \"$harness\""));
        assert!(script.contains(r#"BUZZ_ACP_AGENT_ARGS="acp""#));
        // And a missing harness is a failure, never a substitution.
        assert!(script.contains("exit 91"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_claude_adapters_prefer_the_stable_native_launcher() {
        for (index, adapter) in ["claude-agent-acp", "claude-code-acp"]
            .into_iter()
            .enumerate()
        {
            let root = sandbox_host(&format!("claude-cli-{index}"), HostAcp::Installed);
            let bin = root.join("bin");
            let adapter_path = seed_stub(&bin, adapter, "#!/bin/sh\nexit 0\n");
            seed_stub(&bin, "claude", "#!/bin/sh\nexit 0\n");
            let claude = seed_stub(&root.join(".local/bin"), "claude", "#!/bin/sh\nexit 0\n");

            let mut request = request();
            request["agent"]["agent_command"] = if index == 0 {
                serde_json::json!(adapter)
            } else {
                serde_json::json!(adapter_path)
            };
            let agent = Agent::from_request(&request).unwrap();
            let script =
                deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
            let output = run_in_sandbox(&root, &script);
            assert!(
                output.status.success(),
                "deploy script failed for {adapter}: {}",
                String::from_utf8_lossy(&output.stderr)
            );

            let env_file = root
                .join(".config/buzz-acp")
                .join(format!("{}.env", agent.slug()));
            let written = std::fs::read_to_string(env_file).unwrap();
            assert!(
                written.contains(&format!("CLAUDE_CODE_EXECUTABLE=\"{}\"", claude.display())),
                "{written}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_remote_claude_adapter_falls_back_to_the_hosts_path() {
        let root = sandbox_host("claude-cli-path", HostAcp::Installed);
        let bin = root.join("bin");
        seed_stub(&bin, "claude-agent-acp", "#!/bin/sh\nexit 0\n");
        let claude = seed_stub(&bin, "claude", "#!/bin/sh\nexit 0\n");

        let mut request = request();
        request["agent"]["agent_command"] = serde_json::json!("claude-agent-acp");
        let agent = Agent::from_request(&request).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        let output = run_in_sandbox(&root, &script);
        assert!(output.status.success());

        let written = std::fs::read_to_string(
            root.join(".config/buzz-acp")
                .join(format!("{}.env", agent.slug())),
        )
        .unwrap();
        assert!(
            written.contains(&format!("CLAUDE_CODE_EXECUTABLE=\"{}\"", claude.display())),
            "{written}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_remote_claude_adapter_requires_the_vendor_cli() {
        let root = sandbox_host("claude-cli-missing", HostAcp::Installed);
        seed_stub(&root.join("bin"), "claude-agent-acp", "#!/bin/sh\nexit 0\n");

        let mut request = request();
        request["agent"]["agent_command"] = serde_json::json!("claude-agent-acp");
        let agent = Agent::from_request(&request).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        let output = run_in_sandbox(&root, &script);

        assert_eq!(output.status.code(), Some(95));
        assert!(String::from_utf8_lossy(&output.stderr).contains("Claude Code CLI not found"));
        assert!(!root.join(".config/buzz-acp").exists());
    }

    /// A Hermes per-profile pin, end to end through the deploy path.
    ///
    /// `discover_harnesses` emits `["--profile", <name>, "acp"]`, and the args
    /// reach the host as ONE comma-joined `BUZZ_ACP_AGENT_ARGS` that `buzz-acp`
    /// re-splits on `,` (`config.rs`, `value_delimiter`). That round trip is
    /// only lossless because a profile name cannot contain a comma — which is
    /// what `is_hermes_profile_name` guarantees — so pin the whole chain here
    /// rather than trusting the two halves independently.
    #[test]
    fn a_hermes_profile_pin_reaches_the_host_intact() {
        let mut request = request();
        request["agent"]["agent_command"] = serde_json::json!("hermes");
        request["agent"]["agent_args"] =
            serde_json::json!(["--profile", "msig-web-analyst", "acp"]);
        let agent = Agent::from_request(&request).unwrap();
        assert_eq!(
            agent.agent_args,
            ["--profile", "msig-web-analyst", "acp"],
            "provider args are pinned verbatim, never re-resolved"
        );

        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        assert!(script.contains("harness_name='hermes'"));
        assert!(
            script.contains(r#"BUZZ_ACP_AGENT_ARGS="--profile,msig-web-analyst,acp""#),
            "{script}"
        );
    }

    #[test]
    fn secrets_travel_in_the_script_body_and_never_on_an_argv() {
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        assert!(script.contains(&format!("BUZZ_PRIVATE_KEY=\"{NSEC}\"")));
        assert!(script.contains(&format!("NOSTR_PRIVATE_KEY=\"{NSEC}\"")));
        assert!(script.contains("ANTHROPIC_API_KEY=\"sk-ant-secret\""));

        // Every secret-bearing line lives inside a quoted heredoc, so the
        // remote shell never expands it and it never becomes an argument to
        // anything. The commands the script *runs* carry no secret at all.
        for line in script.lines() {
            if line.contains(NSEC) || line.contains("sk-ant-secret") {
                assert!(
                    line.starts_with("BUZZ_")
                        || line.starts_with("NOSTR_")
                        || line.starts_with("ANTHROPIC_"),
                    "secret escaped the heredoc body: {line}"
                );
            }
        }
        assert!(script.contains("umask 077"));
        assert!(script.contains("chmod 600 \"$tmp\""));
    }

    #[test]
    fn the_env_file_transcribes_the_local_spawn_contract() {
        let agent = Agent::from_request(&request()).unwrap();
        let body = env_file_body(&agent).unwrap();
        for expected in [
            r#"BUZZ_RELAY_URL="wss://relay.example/ws""#,
            r#"BUZZ_AUTH_TAG="tag-abc""#,
            r#"BUZZ_ACP_LAZY_POOL="true""#,
            r#"BUZZ_ACP_AGENTS="3""#,
            r#"BUZZ_ACP_MULTIPLE_EVENT_HANDLING="steer""#,
            r#"BUZZ_ACP_DEDUP="queue""#,
            r#"BUZZ_ACP_RELAY_OBSERVER="true""#,
            r#"BUZZ_ACP_RESPOND_TO="owner-only""#,
            r#"BUZZ_ACP_SYSTEM_PROMPT="be brief""#,
            r#"BUZZ_ACP_MODEL="claude-sonnet-5""#,
            r#"BUZZ_ACP_MCP_COMMAND="""#,
        ] {
            assert!(body.contains(expected), "missing {expected}");
        }
        // Local process-ownership marker: meaningless where systemd owns the
        // lifecycle, so it is never written.
        assert!(!body.contains("BUZZ_MANAGED_AGENT"));
        // Unset timeouts are omitted so the harness's own defaults win.
        assert!(!body.contains("BUZZ_ACP_IDLE_TIMEOUT"));
        assert!(!body.contains("BUZZ_ACP_MAX_TURN_DURATION"));
        // No allowlist key unless the mode asks for one.
        assert!(!body.contains("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    }

    #[test]
    fn the_harness_sees_the_same_model_env_local_spawn_would_set() {
        // `runtime_metadata_env_vars` parity: buzz-acp reads BUZZ_ACP_MODEL,
        // but Goose itself reads GOOSE_MODEL/GOOSE_PROVIDER. Emitting only the
        // former leaves the host's ~/.config/goose/config.yaml deciding the
        // model, silently overriding the user's pick.
        let body = env_file_body(&Agent::from_request(&request()).unwrap()).unwrap();
        assert!(body.contains(r#"GOOSE_MODEL="claude-sonnet-5""#), "{body}");
        assert!(body.contains(r#"GOOSE_PROVIDER="anthropic""#), "{body}");

        // An absolute pin resolves to the same runtime — `known_acp_runtime`
        // matches on the file name locally, so this must too.
        let mut request = request();
        request["agent"]["agent_command"] = serde_json::json!("/home/ubuntu/.local/bin/goose");
        let body = env_file_body(&Agent::from_request(&request).unwrap()).unwrap();
        assert!(body.contains(r#"GOOSE_MODEL="claude-sonnet-5""#), "{body}");

        // Runtimes that declare no model/provider env upstream get none here.
        request["agent"]["agent_command"] = serde_json::json!("claude-code-acp");
        let body = env_file_body(&Agent::from_request(&request).unwrap()).unwrap();
        assert!(!body.contains("GOOSE_MODEL"));
        assert!(body.contains(r#"BUZZ_ACP_MODEL="claude-sonnet-5""#));

        // And an unset field writes no key at all, so the harness default wins.
        request["agent"]["agent_command"] = serde_json::json!("goose");
        request["agent"]["provider"] = serde_json::json!("");
        let body = env_file_body(&Agent::from_request(&request).unwrap()).unwrap();
        assert!(body.contains("GOOSE_MODEL"));
        assert!(!body.contains("GOOSE_PROVIDER"));
    }

    #[test]
    fn the_deprecated_turn_timeout_is_never_written() {
        // The payload still carries `turn_timeout_seconds` (upstream
        // `deploy_payload_json`), but `BUZZ_ACP_TURN_TIMEOUT` is deprecated and
        // ignored by the harness, and local spawn does not write it either.
        let mut request = request();
        request["agent"]["turn_timeout_seconds"] = serde_json::json!(320);
        let body = env_file_body(&Agent::from_request(&request).unwrap()).unwrap();
        assert!(!body.contains("TURN_TIMEOUT"), "{body}");
    }

    #[test]
    fn timeouts_are_emitted_only_when_set() {
        let mut request = request();
        request["agent"]["idle_timeout_seconds"] = serde_json::json!(900);
        request["agent"]["max_turn_duration_seconds"] = serde_json::json!(3600);
        let body = env_file_body(&Agent::from_request(&request).unwrap()).unwrap();
        assert!(body.contains(r#"BUZZ_ACP_IDLE_TIMEOUT="900""#));
        assert!(body.contains(r#"BUZZ_ACP_MAX_TURN_DURATION="3600""#));
    }

    #[test]
    fn user_env_is_written_last_so_it_overrides() {
        let mut request = request();
        request["agent"]["env_vars"] = serde_json::json!({ "GOOSE_MODE": "auto" });
        let body = env_file_body(&Agent::from_request(&request).unwrap()).unwrap();
        let user = body.find("GOOSE_MODE").unwrap();
        assert!(body.find("BUZZ_ACP_MODEL").unwrap() < user);
    }

    #[test]
    fn reserved_and_malformed_env_keys_are_refused() {
        for key in [
            "BUZZ_PRIVATE_KEY",
            "buzz_relay_url",
            "BUZZ_MANAGED_AGENT",
            "CLAUDE_CODE_EXECUTABLE",
        ] {
            let mut request = request();
            request["agent"]["env_vars"] = serde_json::json!({ key: "x" });
            let error = env_file_body(&Agent::from_request(&request).unwrap()).unwrap_err();
            assert!(error.contains("reserved"), "{key}: {error}");
        }
        let mut request = request();
        request["agent"]["env_vars"] = serde_json::json!({ "BAD KEY": "x" });
        assert!(env_file_body(&Agent::from_request(&request).unwrap())
            .unwrap_err()
            .contains("not a valid identifier"));
    }

    #[test]
    fn env_values_cannot_forge_an_extra_assignment() {
        // A newline would end the assignment and start a line of the value's
        // own choosing — including a line that re-sets a reserved key.
        let error = env_line("X", "a\nBUZZ_PRIVATE_KEY=nsec1evil").unwrap_err();
        assert!(error.contains("control character"));
        // Quotes and backslashes are escaped rather than refused.
        assert_eq!(env_line("X", r#"a"b\c"#).unwrap(), "X=\"a\\\"b\\\\c\"\n");
    }

    #[test]
    fn allowlist_mode_requires_an_allowlist() {
        let mut request = request();
        request["agent"]["respond_to"] = serde_json::json!("allowlist");
        assert!(env_file_body(&Agent::from_request(&request).unwrap()).is_err());

        request["agent"]["respond_to_allowlist"] = serde_json::json!(["abc123"]);
        let body = env_file_body(&Agent::from_request(&request).unwrap()).unwrap();
        assert!(body.contains(r#"BUZZ_ACP_RESPOND_TO_ALLOWLIST="abc123""#));
    }

    #[test]
    fn slugs_are_unit_safe_and_stable_across_redeploys() {
        let agent = Agent::from_request(&request()).unwrap();
        let slug = agent.slug();
        assert!(
            slug.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{slug}"
        );
        assert_eq!(slug, format!("research-bot-{PUBKEY_SLUG}"));
        // Redeploy is the start path: the same agent must yield the same unit
        // and the same agent_id, or start would provision a duplicate.
        assert_eq!(slug, Agent::from_request(&request()).unwrap().slug());
        assert_eq!(agent.agent_id(), format!("buzz-acp@{slug}"));
    }

    /// The collision this key exists to prevent: one SSH account, two agents a
    /// user called the same thing. Keyed on the name they shared a unit, an env
    /// file and an `agent_id`, so the second deploy overwrote the first's nsec.
    #[test]
    fn two_agents_with_one_name_get_distinct_units() {
        let deployed = |pubkey: &str| {
            let mut request = request();
            request["agent"]["pubkey"] = serde_json::json!(pubkey);
            let agent = Agent::from_request(&request).unwrap();
            (agent.slug(), agent.agent_id())
        };
        let other = "e88a691e98d9987c964521dff60025f60700378a4879180dcbbb4a5027850411";
        let (first_slug, first_id) = deployed(PUBKEY);
        let (second_slug, second_id) = deployed(other);
        assert_ne!(first_slug, second_slug);
        assert_ne!(first_id, second_id);
        // Both still name the agent a human recognizes.
        assert!(first_slug.starts_with("research-bot-"), "{first_slug}");
        assert!(second_slug.starts_with("research-bot-"), "{second_slug}");
    }

    #[test]
    fn a_name_with_nothing_usable_still_produces_a_legal_instance_name() {
        let mut request = request();
        request["agent"]["name"] = serde_json::json!("!!!");
        let slug = Agent::from_request(&request).unwrap().slug();
        assert_eq!(slug, format!("agent-{PUBKEY_SLUG}"));
    }

    /// The identity is refused rather than defaulted: a payload without it
    /// would silently fall back to name-keyed units, which is the collision.
    #[test]
    fn deploy_refuses_a_payload_with_no_usable_agent_identity() {
        let mut request = request();
        request["agent"].as_object_mut().unwrap().remove("pubkey");
        assert!(rejection(&request).contains("no 'pubkey'"));

        for bad in ["", "abc123", &"z".repeat(64)] {
            request["agent"]["pubkey"] = serde_json::json!(bad);
            let error = rejection(&request);
            assert!(
                error.contains("pubkey"),
                "accepted {bad:?} with error {error}"
            );
        }

        // Case is normalized, so the same key never yields two units.
        request["agent"]["pubkey"] = serde_json::json!(PUBKEY.to_uppercase());
        assert_eq!(
            Agent::from_request(&request).unwrap().slug(),
            Agent::from_request(&self::request()).unwrap().slug()
        );
    }

    #[test]
    fn redeploy_is_idempotent() {
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        // One templated unit per host, reloaded only when its content changed.
        assert!(script.contains(r#"unit_file="$units/buzz-acp@.service""#));
        assert!(script.contains(r#"if cmp -s "$unit_file.new" "$unit_file""#));
        assert_eq!(script.matches("daemon-reload").count(), 1);
        // Enable is idempotent; restart makes an already-running unit adopt the
        // rewritten env file.
        assert!(script.contains("systemctl --user enable --now 'buzz-acp@research-bot-"));
        assert!(script.contains("systemctl --user restart 'buzz-acp@research-bot-"));
        // The env file is replaced atomically, so a failed write never leaves a
        // half-written identity behind.
        assert!(script.contains(r#"mv "$tmp" "$env_file""#));
    }

    #[test]
    fn the_unit_template_substitutes_a_resolved_buzz_acp() {
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        assert!(UNIT_TEMPLATE.contains("ExecStart=@BUZZ_ACP_BIN@"));
        assert!(UNIT_TEMPLATE.contains("EnvironmentFile=%h/.config/buzz-acp/%i.env"));
        // Substitution is parameter expansion, not `sed -i`: in-place editing
        // is a GNU extension that BSD and macOS hosts reject.
        assert!(!script.contains("sed -i"));
        assert!(script.contains(
            r#"printf '%s"%s"%s\n' "${template%%@BUZZ_ACP_BIN@*}" "$acp_unit" "${template#*@BUZZ_ACP_BIN@}""#
        ));
        // Lingering, or the agent dies when this SSH session ends. It also
        // creates /run/user/$(id -u), so it must precede any bus traffic.
        let linger = script.find("loginctl enable-linger").unwrap();
        assert!(linger < script.find("systemctl --user").unwrap());
        // A non-interactive SSH command often has no XDG_RUNTIME_DIR, and
        // without it every `systemctl --user` fails to reach the bus.
        assert!(script.contains(r#"if [ -z "${XDG_RUNTIME_DIR:-}" ]; then"#));
    }

    /// Run the generated script against a real `/bin/sh` in a sandbox, with
    /// `systemctl`/`loginctl` stubbed out.
    ///
    /// Substring assertions prove the script *says* the right things; only
    /// executing it proves it *is* a valid shell program that produces the
    /// right files. Everything below — quoting, heredoc framing, the
    /// `@BUZZ_ACP_BIN@` expansion, `set -eu` interactions — is the kind of
    /// defect no `contains` check catches.
    fn run_deploy_script(
        sandbox: &str,
        request: &serde_json::Value,
    ) -> (std::process::Output, std::path::PathBuf) {
        let root = sandbox_host(sandbox, HostAcp::Installed);
        let agent = Agent::from_request(request).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        (run_in_sandbox(&root, &script), root)
    }

    /// Whether the sandboxed "host" already has `buzz-acp` on its PATH. The
    /// install path only engages on a host that does not.
    #[derive(PartialEq)]
    enum HostAcp {
        Installed,
        Missing,
    }

    /// Build the fake host: a `$HOME` with a stubbed `bin` on its PATH.
    fn sandbox_host(sandbox: &str, acp: HostAcp) -> std::path::PathBuf {
        // Named per test rather than keyed on the thread id, which the test
        // harness recycles once a thread finishes.
        let root =
            std::env::temp_dir().join(format!("buzz-deploy-{}-{sandbox}", std::process::id()));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // Stub every host binary the script resolves, so the run is hermetic.
        let mut stubs = vec![
            ("goose", "#!/bin/sh\nexit 0\n"),
            ("git-credential-nostr", "#!/bin/sh\nexit 0\n"),
            // Record the systemd calls instead of making them.
            (
                "systemctl",
                "#!/bin/sh\nprintf 'systemctl %s\\n' \"$*\" >> \"$HOME/systemd.log\"\n",
            ),
            (
                "loginctl",
                "#!/bin/sh\nprintf 'loginctl %s\\n' \"$*\" >> \"$HOME/systemd.log\"\n",
            ),
        ];
        if acp == HostAcp::Installed {
            stubs.push(("buzz-acp", "#!/bin/sh\nexit 0\n"));
        }
        // Note `buzz` is NOT stubbed: the sandbox host has no CLI unless a test
        // seeds one with `seed_stub`, so every run through here also exercises
        // the degradation path — a warning, and a deploy that still succeeds.
        for (name, body) in stubs {
            seed_stub(&bin, name, body);
        }
        root
    }

    /// Drop an executable stub into `dir`.
    fn seed_stub(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// Feed `script` to a real `/bin/sh` exactly as `ssh` feeds it to the
    /// remote one: on stdin, with the sandbox as `$HOME`.
    fn run_in_sandbox(root: &std::path::Path, script: &str) -> std::process::Output {
        let bin = root.join("bin");
        std::process::Command::new("/bin/sh")
            .arg("-s")
            .env_clear()
            .env("HOME", root)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(script.as_bytes())
                    .unwrap();
                child.wait_with_output()
            })
            .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn the_generated_script_actually_runs_and_provisions_the_host() {
        let (output, root) = run_deploy_script("provision", &request());
        assert!(
            output.status.success(),
            "deploy script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let slug = Agent::from_request(&request()).unwrap().slug();
        let env_file = root.join(".config/buzz-acp").join(format!("{slug}.env"));
        let written = std::fs::read_to_string(&env_file).unwrap();

        // The harness pin, resolved to an absolute path on the host, and
        // written where systemd re-reads it on every restart.
        assert!(
            written.contains(&format!(
                "BUZZ_ACP_AGENT_COMMAND=\"{}\"",
                root.join("bin/goose").display()
            )),
            "{written}"
        );
        assert!(written.contains(&format!("BUZZ_PRIVATE_KEY=\"{NSEC}\"")));
        assert!(written.contains("ANTHROPIC_API_KEY=\"sk-ant-secret\""));
        assert!(!written.contains("CLAUDE_CODE_EXECUTABLE"));
        // The git block only lands because the stub helper exists, and it
        // carries the helper's resolved path.
        assert!(written.contains(&format!(
            "GIT_CONFIG_VALUE_0=\"{}\"",
            root.join("bin/git-credential-nostr").display()
        )));
        assert!(written.contains(&format!("NOSTR_PRIVATE_KEY=\"{NSEC}\"")));
        // Every line is a well-formed assignment: no heredoc marker leaked in,
        // and no value split across lines.
        for line in written.lines() {
            assert!(
                line.split_once('=')
                    .is_some_and(|(_, v)| v.starts_with('"') && v.ends_with('"') && v.len() >= 2),
                "malformed env line: {line}"
            );
        }

        // Only the owner can read the minted key.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&env_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "env file is group/world accessible");
        }

        // The unit landed with a real path in ExecStart, and no placeholder.
        let unit =
            std::fs::read_to_string(root.join(".config/systemd/user/buzz-acp@.service")).unwrap();
        assert!(unit.contains(&format!(
            "ExecStart=\"{}\"",
            root.join("bin/buzz-acp").display()
        )));
        assert!(!unit.contains("@BUZZ_ACP_BIN@"));
        assert!(!root
            .join(".config/systemd/user/buzz-acp@.service.new")
            .exists());

        let calls = std::fs::read_to_string(root.join("systemd.log")).unwrap();
        assert!(calls.contains("loginctl enable-linger"));
        assert!(calls.contains("systemctl --user daemon-reload"));
        assert!(calls.contains(&format!(
            "systemctl --user enable --now buzz-acp@{slug}.service"
        )));
        assert!(calls.contains(&format!("systemctl --user restart buzz-acp@{slug}.service")));
    }

    #[cfg(unix)]
    #[test]
    fn a_second_deploy_reuses_the_unit_and_skips_the_reload() {
        let (first, root) = run_deploy_script("redeploy", &request());
        assert!(first.status.success());
        std::fs::remove_file(root.join("systemd.log")).unwrap();

        // Redeploy is the start path, so this is what `start_managed_agent`
        // does on every start. The unit content is unchanged, so systemd must
        // not be reloaded — but the env file must still be rewritten and the
        // service restarted onto it.
        let (second, _) = run_deploy_script("redeploy", &request());
        assert!(
            second.status.success(),
            "redeploy failed: {}",
            String::from_utf8_lossy(&second.stderr)
        );
        let calls = std::fs::read_to_string(root.join("systemd.log")).unwrap();
        assert!(!calls.contains("daemon-reload"), "{calls}");
        assert!(calls.contains("restart"), "{calls}");
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_harness_stops_the_deploy_before_anything_is_written() {
        let mut request = request();
        request["agent"]["agent_command"] = serde_json::json!("not-installed-anywhere");
        let (output, root) = run_deploy_script("missing-harness", &request);
        assert_eq!(output.status.code(), Some(91));
        assert!(String::from_utf8_lossy(&output.stderr).contains("not-installed-anywhere"));
        // `set -e` plus ordering: nothing is provisioned on a failed resolve.
        assert!(!root.join(".config/buzz-acp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn shell_metacharacters_in_the_payload_stay_inert() {
        // Regression: `quote()` makes a value inert as an *argument*, but
        // interpolating that quoted form into a double-quoted string (an error
        // message, say) leaves its single quotes literal and lets a `$(...)`
        // in the payload execute. Every field below is attacker-influenced, so
        // this test runs the script for real and checks that none of the
        // command substitutions fired.
        let canary = std::env::temp_dir().join(format!("buzz-pwned-{}", std::process::id()));
        let _ = std::fs::remove_file(&canary);
        let payload = format!("$(touch {})", canary.display());

        let mut request = request();
        request["agent"]["name"] = serde_json::json!(format!("bot {payload}"));
        request["agent"]["agent_command"] = serde_json::json!(payload);
        request["agent"]["relay_url"] = serde_json::json!(format!("wss://relay/{payload}"));
        request["agent"]["model"] = serde_json::json!(payload.clone());
        request["agent"]["env_vars"] = serde_json::json!({ "EVIL": payload.clone() });

        let (output, _) = run_deploy_script("injection", &request);
        // The harness does not exist, so the deploy stops — the point is that
        // it stops without having executed the payload.
        assert_eq!(output.status.code(), Some(91));
        assert!(
            !canary.exists(),
            "payload executed on the host: command injection in the deploy script"
        );

        // And the slug stays a legal systemd instance name regardless.
        let agent = Agent::from_request(&request).unwrap();
        assert!(agent
            .slug()
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    }

    /// A binary to push: a legal ELF header followed by every byte sequence
    /// that would end a heredoc, escape the script, or run a command if the
    /// transport were anything other than base64. Both tools' delimiters are in
    /// there, so the same bytes are hostile to whichever tool carries them.
    fn canary_binary(canary: &std::path::Path) -> Vec<u8> {
        let mut bytes = b"\x7fELF\x02\x01\x01\x00".to_vec();
        bytes.extend_from_slice(format!("$(touch {})\n", canary.display()).as_bytes());
        bytes.extend_from_slice(format!("`touch {}`\n", canary.display()).as_bytes());
        bytes.extend_from_slice(b"BUZZ_ACP_B64_EOF\nrm -rf \"$HOME\"\n");
        bytes.extend_from_slice(b"BUZZ_CLI_B64_EOF\nrm -rf \"$HOME\"\n");
        bytes.extend_from_slice(b"\0'\"\r\n$HOME ${HOME}\n");
        bytes.extend_from_slice(&(0u8..=255).collect::<Vec<u8>>());
        bytes
    }

    fn push_payload(tool: Tool, name: &str, bytes: &[u8]) -> Payload {
        let path = std::env::temp_dir().join(format!(
            "buzz-push-{}-{}-{name}",
            tool.name,
            std::process::id()
        ));
        std::fs::write(&path, bytes).unwrap();
        Payload::read(tool, &path.display().to_string()).unwrap()
    }

    /// The `Pushes` a desktop that set only `BUZZ_ACP_PUSH_BINARY` produces.
    fn acp_push(payload: Payload) -> Pushes {
        Pushes {
            acp: Some(payload),
            cli: None,
        }
    }

    #[test]
    fn the_pushed_binaries_are_optional_fields_that_change_nothing_when_absent() {
        // The seams must be invisible: a payload without the fields produces
        // the script the crate produced before they existed.
        let mut request = request();
        let agent = Agent::from_request(&request).unwrap();
        assert!(agent.buzz_acp_binary.is_none());
        assert!(agent.buzz_cli_binary.is_none());
        let without = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        assert!(without.contains("exit 90"));
        assert!(!without.contains("base64 -d"));

        // A blank string is "absent", not "push nothing".
        for field in ["buzz_acp_binary", "buzz_cli_binary"] {
            request["agent"][field] = serde_json::json!("   ");
        }
        let agent = Agent::from_request(&request).unwrap();
        assert!(agent.buzz_acp_binary.is_none());
        assert!(agent.buzz_cli_binary.is_none());

        request["agent"]["buzz_acp_binary"] = serde_json::json!("/opt/buzz-acp");
        request["agent"]["buzz_cli_binary"] = serde_json::json!("/opt/buzz");
        let agent = Agent::from_request(&request).unwrap();
        assert_eq!(agent.buzz_acp_binary, Some("/opt/buzz-acp".to_string()));
        assert_eq!(agent.buzz_cli_binary, Some("/opt/buzz".to_string()));
    }

    /// The exact-equality pin: with both fields absent the script is *byte*
    /// identical to the one the crate emitted before either seam existed —
    /// modulo the CLI resolution block, which is unconditional and therefore
    /// spelled out here in full rather than asserted about.
    ///
    /// Substring assertions cannot see an accidental extra line; this can.
    #[test]
    fn the_script_for_a_payload_with_neither_field_is_pinned_exactly() {
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        let slug = agent.slug();

        let expected = format!(
            r#"set -eu
umask 077
harness_name='goose'
acp=$(command -v 'buzz-acp' 2>/dev/null || true)
if [ -z "$acp" ] && [ -x "$HOME/.local/bin/buzz-acp" ]; then acp="$HOME/.local/bin/buzz-acp"; fi
if [ -z "$acp" ]; then echo "buzz-acp not found on the server's PATH or in ~/.local/bin — install it, or set 'buzz-acp path on the server'" >&2; exit 90; fi
harness=$(command -v "$harness_name" 2>/dev/null) || {{ echo "harness $harness_name not found on the server's PATH" >&2; exit 91; }}
claude_cli=""
case "${{harness##*/}}" in
  claude-agent-acp|claude-code-acp)
    if [ -x "$HOME/.local/bin/claude" ]; then
      claude_cli="$HOME/.local/bin/claude"
    else
      claude_cli=$(command -v claude 2>/dev/null || true)
    fi
    if [ -z "$claude_cli" ]; then echo "Claude Code CLI not found in ~/.local/bin or on the server's PATH" >&2; exit 95; fi
    ;;
esac
cli=$(command -v 'buzz' 2>/dev/null || true)
if [ -z "$cli" ] && [ -x "$HOME/.local/bin/buzz" ]; then cli="$HOME/.local/bin/buzz"; fi
if [ -z "$cli" ]; then echo "WARNING: no 'buzz' CLI on the server's PATH or in ~/.local/bin — agents on this host cannot reply with 'buzz messages send' and will degrade to slower replies; install it there, or set BUZZ_CLI_PUSH_BINARY on the desktop and redeploy" >&2; fi
cred=$(command -v git-credential-nostr 2>/dev/null || true)
conf="$HOME/.config/buzz-acp"
units="$HOME/.config/systemd/user"
mkdir -p "$conf" "$units"
env_file="$conf/{slug}.env"
tmp="$env_file.new"
{{
printf 'BUZZ_ACP_AGENT_COMMAND="%s"\n' "$harness"
printf 'PATH="%s"\n' "$HOME/.local/bin:$PATH"
cat <<'BUZZ_ENV_EOF'
{env}BUZZ_ENV_EOF
if [ -n "$claude_cli" ]; then
printf 'CLAUDE_CODE_EXECUTABLE="%s"\n' "$claude_cli"
fi
if [ -n "$cred" ]; then
printf 'GIT_CONFIG_VALUE_0="%s"\n' "$cred"
cat <<'BUZZ_GIT_EOF'
NOSTR_PRIVATE_KEY="{NSEC}"
GIT_TERMINAL_PROMPT="0"
GIT_CONFIG_COUNT="2"
GIT_CONFIG_KEY_0="credential.https://relay.example/ws/git.helper"
GIT_CONFIG_KEY_1="credential.https://relay.example/ws/git.useHttpPath"
GIT_CONFIG_VALUE_1="true"
BUZZ_GIT_EOF
fi
}} > "$tmp"
chmod 600 "$tmp"
mv "$tmp" "$env_file"
loginctl enable-linger "$(id -un)" >/dev/null 2>&1 || true
if [ -z "${{XDG_RUNTIME_DIR:-}}" ]; then
  XDG_RUNTIME_DIR="/run/user/$(id -u)"
  export XDG_RUNTIME_DIR
fi
unit_file="$units/buzz-acp@.service"
acp_unit=$(printf '%s' "$acp" | sed 's/[\\"]/\\&/g')
template=$(cat <<'BUZZ_UNIT_EOF'
{unit}BUZZ_UNIT_EOF
)
printf '%s"%s"%s\n' "${{template%%@BUZZ_ACP_BIN@*}}" "$acp_unit" "${{template#*@BUZZ_ACP_BIN@}}" > "$unit_file.new"
if cmp -s "$unit_file.new" "$unit_file"; then
  rm -f "$unit_file.new"
else
  mv "$unit_file.new" "$unit_file"
  systemctl --user daemon-reload
fi
systemctl --user enable --now 'buzz-acp@{slug}.service' >/dev/null
# Redeploy is also the start path, so an already-running unit must pick up the
# rewritten env file rather than be left on the old one.
systemctl --user restart 'buzz-acp@{slug}.service'
"#,
            env = env_file_body(&agent).unwrap(),
            unit = UNIT_TEMPLATE,
        );
        assert_eq!(script, expected);
    }

    #[test]
    fn a_pushed_binary_never_displaces_the_secret_discipline() {
        let canary = std::env::temp_dir().join("buzz-never");
        let payload = push_payload(install::ACP, "discipline", &canary_binary(&canary));
        let sha256 = payload.sha256().to_string();
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &acp_push(payload)).unwrap();

        // The install block is additive: everything the secret path relies on
        // is still exactly where it was.
        assert!(script.starts_with("set -eu\numask 077\n"));
        assert!(script.contains("chmod 600 \"$tmp\""));
        assert!(script.contains(&format!("BUZZ_PRIVATE_KEY=\"{NSEC}\"")));
        // The binary is resolved/installed BEFORE the unit is templated, so
        // `$acp` — and therefore ExecStart — names the copy just installed.
        let install = script.find("base64 -d").unwrap();
        assert!(install < script.find("unit_file=").unwrap());
        // The hash travels in the clear (it is a fingerprint, not a secret) and
        // the encoded bytes carry nothing the shell reads as syntax.
        assert!(script.contains(&sha256));
    }

    #[cfg(unix)]
    #[test]
    fn a_pushed_binary_installs_atomically_and_only_after_it_verifies() {
        let canary = std::env::temp_dir().join(format!("buzz-push-pwned-{}", std::process::id()));
        let _ = std::fs::remove_file(&canary);
        let bytes = canary_binary(&canary);
        let payload = push_payload(install::ACP, "install", &bytes);

        // A host with no `buzz-acp` at all — the only case the push engages.
        let root = sandbox_host("install", HostAcp::Missing);
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &acp_push(payload)).unwrap();
        let output = run_in_sandbox(&root, &script);
        assert!(
            output.status.success(),
            "install deploy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Byte-identical after a round trip through base64, a heredoc, and a
        // real `/bin/sh` — including the NULs, quotes, `$(...)` and the literal
        // heredoc delimiter embedded in the payload.
        let installed = root.join(".local/bin/buzz-acp");
        assert_eq!(std::fs::read(&installed).unwrap(), bytes);
        assert!(
            !canary.exists(),
            "the pushed binary's contents executed on the host"
        );

        // Executable, and no temp file left behind.
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&installed).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "installed binary is not 755");
        assert!(!leftover_temp_files(&root.join(".local/bin"), install::ACP));

        // And the unit points at the copy this pass installed, in the same
        // deploy — install first, resolve second.
        let unit =
            std::fs::read_to_string(root.join(".config/systemd/user/buzz-acp@.service")).unwrap();
        assert!(
            unit.contains(&format!("ExecStart=\"{}\"", installed.display())),
            "{unit}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_corrupted_push_aborts_before_the_mv_and_leaves_nothing_runnable() {
        let canary = std::env::temp_dir().join("buzz-never");
        let payload = push_payload(install::ACP, "mismatch", &canary_binary(&canary));
        let sha256 = payload.sha256().to_string();
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &acp_push(payload)).unwrap();
        // Stand in for a payload damaged in flight: the host is told to expect
        // a digest the decoded bytes cannot produce.
        let script = script.replace(&sha256, &"a".repeat(64));

        let root = sandbox_host("mismatch", HostAcp::Missing);
        let output = run_in_sandbox(&root, &script);
        assert_eq!(output.status.code(), Some(94));
        assert!(String::from_utf8_lossy(&output.stderr).contains("sha256"));

        // Nothing installed, and — the property that matters — no half-written
        // executable left in the directory systemd's ExecStart would name.
        assert!(!root.join(".local/bin/buzz-acp").exists());
        assert!(!leftover_temp_files(&root.join(".local/bin"), install::ACP));
        // The deploy stopped there: no env file, no unit.
        assert!(!root.join(".config/buzz-acp").exists());
        assert!(!root.join(".config/systemd").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_host_without_buzz_acp_and_no_pushed_binary_still_fails_with_todays_guidance() {
        // The un-pushed path is unchanged: exit 90 and the same message, so a
        // user who never sets the seam sees exactly what they saw before.
        let root = sandbox_host("no-acp", HostAcp::Missing);
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        let output = run_in_sandbox(&root, &script);
        assert_eq!(output.status.code(), Some(90));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("buzz-acp not found on the server's PATH"),
            "{stderr}"
        );
        assert!(!root.join(".local/bin/buzz-acp").exists());
        assert!(!root.join(".config/buzz-acp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_host_binary_is_never_replaced_by_the_pushed_one() {
        // Staleness rule: push-when-missing only. Deploy is the start path, so
        // a version-comparing rule would reinstall underneath a running fleet
        // on every start — and a desktop pinned to an older artifact would
        // downgrade the host.
        let canary = std::env::temp_dir().join("buzz-never");
        let payload = push_payload(install::ACP, "keep", &canary_binary(&canary));
        let root = sandbox_host("keep", HostAcp::Installed);
        let existing = std::fs::read(root.join("bin/buzz-acp")).unwrap();

        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &acp_push(payload)).unwrap();
        let output = run_in_sandbox(&root, &script);
        assert!(
            output.status.success(),
            "deploy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(std::fs::read(root.join("bin/buzz-acp")).unwrap(), existing);
        assert!(
            !root.join(".local/bin/buzz-acp").exists(),
            "a host that already had buzz-acp got a second copy installed"
        );
        let unit =
            std::fs::read_to_string(root.join(".config/systemd/user/buzz-acp@.service")).unwrap();
        assert!(unit.contains(&format!(
            "ExecStart=\"{}\"",
            root.join("bin/buzz-acp").display()
        )));
    }

    #[cfg(unix)]
    #[test]
    fn a_second_deploy_keeps_the_binary_the_first_one_installed() {
        // The install destination — `~/.local/bin` — is NOT on a
        // non-interactive SSH PATH, which is exactly why the env file below
        // pins `PATH="$HOME/.local/bin:$PATH"` itself. Resolution has to say so
        // too: with a bare `command -v`, the probe answered "missing" forever
        // and every deploy re-streamed and replaced the binary. Deploy is the
        // start path, so that is every agent start, underneath a running fleet.
        //
        // `an_existing_host_binary_is_never_replaced_by_the_pushed_one` cannot
        // see this: it seeds the stub into the sandbox's `bin`, which IS on the
        // sandbox PATH.
        use std::os::unix::fs::MetadataExt;

        let canary = std::env::temp_dir().join("buzz-never");
        let payload = push_payload(install::ACP, "twice", &canary_binary(&canary));
        let root = sandbox_host("twice", HostAcp::Missing);
        let agent = Agent::from_request(&request()).unwrap();
        let installed = root.join(".local/bin/buzz-acp");

        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &acp_push(payload)).unwrap();
        let first = run_in_sandbox(&root, &script);
        assert!(
            first.status.success(),
            "first deploy failed: {}",
            String::from_utf8_lossy(&first.stderr)
        );
        let inode = std::fs::metadata(&installed).unwrap().ino();

        // What the desktop asks before deploy #2. It must now answer "the host
        // has it", which is what keeps the payload off the wire — the file is
        // not even read, let alone encoded and streamed.
        let probe = run_in_sandbox(
            &root,
            &install::probe_script(&[(install::ACP, acp_command(&config()))]),
        );
        assert!(
            install::probe_found(&String::from_utf8_lossy(&probe.stdout), install::ACP),
            "the probe did not see the binary the previous deploy installed"
        );

        // And even the worst case — a script that still carries the payload —
        // resolves to the installed copy instead of replacing it.
        let second = run_in_sandbox(&root, &script);
        assert!(
            second.status.success(),
            "second deploy failed: {}",
            String::from_utf8_lossy(&second.stderr)
        );
        assert_eq!(
            std::fs::metadata(&installed).unwrap().ino(),
            inode,
            "the second deploy replaced the binary the first one installed"
        );

        // The unit still names it, so idempotence is real and not just quiet.
        let unit =
            std::fs::read_to_string(root.join(".config/systemd/user/buzz-acp@.service")).unwrap();
        assert!(
            unit.contains(&format!("ExecStart=\"{}\"", installed.display())),
            "{unit}"
        );
    }

    /// `ExecStart=` splits an unquoted value on whitespace, so a configured
    /// `buzz-acp path on the server` inside a directory with a space in it
    /// would make systemd run the first word and pass the rest as arguments.
    /// The schema accepts such a path and `quote()` makes it a safe *shell*
    /// argument, which is a different question from what systemd parses.
    #[cfg(unix)]
    #[test]
    fn a_resolved_path_containing_whitespace_stays_one_word_in_exec_start() {
        let root = sandbox_host("spaced-acp", HostAcp::Missing);
        let acp = seed_stub(&root.join("buzz tools"), "buzz-acp", "#!/bin/sh\nexit 0\n");
        let config = SshConfig {
            buzz_acp_path: Some(acp.display().to_string()),
            ..config()
        };
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config, UNIT_TEMPLATE, &Pushes::default()).unwrap();
        let output = run_in_sandbox(&root, &script);
        assert!(
            output.status.success(),
            "deploy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let unit =
            std::fs::read_to_string(root.join(".config/systemd/user/buzz-acp@.service")).unwrap();
        assert!(
            unit.contains(&format!("ExecStart=\"{}\"\n", acp.display())),
            "{unit}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_configured_absolute_path_still_resolves_the_copy_deploy_installed() {
        // `buzz_acp_path` is an absolute path the operator picked, but an
        // install always lands in `~/.local/bin`. Resolving only what was
        // configured would never find it, so the host would re-install on every
        // single start, forever.
        use std::os::unix::fs::MetadataExt;

        let canary = std::env::temp_dir().join("buzz-never");
        let payload = push_payload(install::ACP, "configured", &canary_binary(&canary));
        let root = sandbox_host("configured", HostAcp::Missing);
        let config = SshConfig {
            buzz_acp_path: Some("/opt/buzz-acp".into()),
            ..config()
        };
        let agent = Agent::from_request(&request()).unwrap();
        let installed = root.join(".local/bin/buzz-acp");

        let script = deploy_script(&agent, &config, UNIT_TEMPLATE, &acp_push(payload)).unwrap();
        assert!(run_in_sandbox(&root, &script).status.success());
        let inode = std::fs::metadata(&installed).unwrap().ino();

        let probe = run_in_sandbox(
            &root,
            &install::probe_script(&[(install::ACP, acp_command(&config))]),
        );
        assert!(
            install::probe_found(&String::from_utf8_lossy(&probe.stdout), install::ACP),
            "the probe missed the install because the configured path is elsewhere"
        );
        assert!(run_in_sandbox(&root, &script).status.success());
        assert_eq!(std::fs::metadata(&installed).unwrap().ino(), inode);
    }

    #[cfg(unix)]
    #[test]
    fn a_payload_that_decodes_to_garbage_aborts_before_anything_is_installed() {
        let canary = std::env::temp_dir().join("buzz-never");
        let payload = push_payload(install::ACP, "decode", &canary_binary(&canary));
        let head = payload.encoded()[..8].to_string();
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &acp_push(payload)).unwrap();
        // Stand in for a stream truncated in flight. `!` is outside the base64
        // alphabet, so `base64 -d` rejects the body outright — the exit-93
        // branch, which the sha256 test cannot reach because a payload that
        // fails to decode never gets as far as being hashed.
        let corrupt = script.replacen(&head, "!!!!!!!!", 1);
        assert_ne!(corrupt, script, "the encoded body was not corrupted");

        let root = sandbox_host("decode", HostAcp::Missing);
        let output = run_in_sandbox(&root, &corrupt);
        assert_eq!(output.status.code(), Some(93));
        assert!(String::from_utf8_lossy(&output.stderr).contains("decode"));
        assert!(!root.join(".local/bin/buzz-acp").exists());
        assert!(!leftover_temp_files(&root.join(".local/bin"), install::ACP));
        // The `|| { ... }` really does bind to the heredoc-fed command: the
        // deploy stopped here rather than running on with a corrupt file.
        assert!(!root.join(".config/systemd").exists());
    }

    #[test]
    fn a_payload_that_names_both_binaries_carries_both() {
        // The two tools share one script and one pass. They must not share a
        // heredoc delimiter, a temp file or a shell variable, or the second
        // body would terminate the first and the host would be handed a
        // half-decoded binary as commands.
        let canary = std::env::temp_dir().join("buzz-never");
        let bytes = canary_binary(&canary);
        let acp = push_payload(install::ACP, "both-acp", &bytes);
        let cli = push_payload(install::CLI, "both-cli", &bytes);
        let agent = Agent::from_request(&request()).unwrap();
        let push = Pushes {
            acp: Some(acp),
            cli: Some(cli),
        };
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &push).unwrap();

        for delimiter in ["BUZZ_ACP_B64_EOF", "BUZZ_CLI_B64_EOF"] {
            assert_eq!(
                script.matches(&format!("<<'{delimiter}'")).count(),
                1,
                "one heredoc opener per tool"
            );
        }
        assert!(script.contains(r#"acp_tmp="$acp_dir/.buzz-acp.tmp.$$""#));
        assert!(script.contains(r#"cli_tmp="$cli_dir/.buzz.tmp.$$""#));
        assert!(script.contains(r#"mv "$acp_tmp" "$acp_dir/buzz-acp""#));
        assert!(script.contains(r#"mv "$cli_tmp" "$cli_dir/buzz""#));
        // Order still holds: the harness is resolved (or installed) before the
        // unit is templated from `$acp`, and the CLI never reaches the unit.
        let acp_install = script.find(r#"mv "$acp_tmp""#).unwrap();
        let cli_install = script.find(r#"mv "$cli_tmp""#).unwrap();
        let unit = script.find("unit_file=").unwrap();
        assert!(acp_install < cli_install);
        assert!(cli_install < unit);
        // Nothing substitutes the CLI into the unit — it is reached through the
        // env file's PATH, not through `ExecStart`.
        assert!(!script.contains("@BUZZ_CLI_BIN@"));
        assert!(!script[unit..].contains("$cli"));
    }

    #[cfg(unix)]
    #[test]
    fn a_pushed_cli_lands_where_the_harness_can_run_it() {
        // The gap this whole change exists to close: a remote agent is told by
        // its own system prompt to answer with `buzz messages send`, and the
        // SSH deploy shipped only `buzz-acp`. Install it, and — the half that
        // makes the install worth anything — leave it on the `PATH` the unit
        // hands the harness.
        let canary = std::env::temp_dir().join(format!("buzz-cli-pwned-{}", std::process::id()));
        let _ = std::fs::remove_file(&canary);
        let bytes = canary_binary(&canary);
        let payload = push_payload(install::CLI, "cli-install", &bytes);

        // A host that HAS buzz-acp: the CLI install is the only thing under
        // test, and the deploy must run all the way through to the restart.
        let root = sandbox_host("cli-install", HostAcp::Installed);
        let agent = Agent::from_request(&request()).unwrap();
        // The `Pushes` a desktop that set only `BUZZ_CLI_PUSH_BINARY` produces.
        let pushes = Pushes {
            acp: None,
            cli: Some(payload),
        };
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &pushes).unwrap();
        let output = run_in_sandbox(&root, &script);
        assert!(
            output.status.success(),
            "cli deploy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Nothing to warn about: the CLI is there now.
        assert!(
            install::warnings(&String::from_utf8_lossy(&output.stderr)).is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Byte-identical through base64, a heredoc and a real `/bin/sh` —
        // NULs, quotes, `$(...)` and both delimiters included.
        let installed = root.join(".local/bin/buzz");
        assert_eq!(std::fs::read(&installed).unwrap(), bytes);
        assert!(
            !canary.exists(),
            "the pushed CLI's contents executed on the host"
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&installed).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "the installed CLI is not executable");
        assert!(!leftover_temp_files(&root.join(".local/bin"), install::CLI));

        // The CLI is reached through the env file, never through the unit: the
        // harness's `PATH` starts at the install destination.
        let slug = agent.slug();
        let env_file = root.join(".config/buzz-acp").join(format!("{slug}.env"));
        let written = std::fs::read_to_string(&env_file).unwrap();
        let path_line = written
            .lines()
            .find(|line| line.starts_with("PATH="))
            .unwrap_or_else(|| panic!("no PATH line in the env file:\n{written}"));
        assert!(
            path_line.starts_with(&format!("PATH=\"{}", root.join(".local/bin").display())),
            "{path_line}"
        );
        let unit =
            std::fs::read_to_string(root.join(".config/systemd/user/buzz-acp@.service")).unwrap();
        assert!(!unit.contains("/.local/bin/buzz\""), "{unit}");
    }

    #[cfg(unix)]
    #[test]
    fn the_env_file_path_is_the_install_destination_ahead_of_the_hosts_own() {
        // Local spawn prepends `~/.local/bin` to the harness's PATH
        // (`managed_agents::runtime::path::build_augmented_path`); this is the
        // remote half of that contract, and it is the only reason an installed
        // tool is a command the agent can name. `systemd --user` expands
        // nothing in an `EnvironmentFile`, so the line has to be composed by
        // the host's shell at deploy time — which is what this proves: the
        // value is literal, resolved, and still carries the host's own PATH.
        let (output, root) = run_deploy_script("path-line", &request());
        assert!(
            output.status.success(),
            "deploy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let slug = Agent::from_request(&request()).unwrap().slug();
        let written =
            std::fs::read_to_string(root.join(".config/buzz-acp").join(format!("{slug}.env")))
                .unwrap();
        let path_line = written
            .lines()
            .find(|line| line.starts_with("PATH="))
            .unwrap_or_else(|| panic!("no PATH line in the env file:\n{written}"));
        assert_eq!(
            path_line,
            format!(
                "PATH=\"{}:{}:/usr/bin:/bin\"",
                root.join(".local/bin").display(),
                root.join("bin").display()
            ),
            "{written}"
        );
        // Not the five literal characters systemd would hand through verbatim.
        assert!(!written.contains("$PATH"));
        assert!(!written.contains("$HOME"));
    }

    #[cfg(unix)]
    #[test]
    fn a_host_without_the_cli_deploys_anyway_and_says_what_was_lost() {
        // The asymmetry, end to end. `buzz-acp` missing stops the deploy; the
        // CLI missing must not — the harness does not depend on it — but it
        // cannot be silent either, or the operator learns about it the way this
        // change was discovered: by watching an agent hunt the filesystem.
        let root = sandbox_host("no-cli", HostAcp::Installed);
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        let output = run_in_sandbox(&root, &script);
        assert!(
            output.status.success(),
            "a missing CLI stopped the deploy: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Provisioned in full: the agent runs, it just replies the slow way.
        let slug = agent.slug();
        assert!(root
            .join(".config/buzz-acp")
            .join(format!("{slug}.env"))
            .exists());
        let calls = std::fs::read_to_string(root.join("systemd.log")).unwrap();
        assert!(calls.contains(&format!("systemctl --user restart buzz-acp@{slug}.service")));

        // And the one line `deploy` forwards to the desktop names both what is
        // missing and how to fix it.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let warnings = install::warnings(&stderr);
        assert_eq!(warnings.len(), 1, "{stderr}");
        assert!(warnings[0].contains("buzz messages send"), "{stderr}");
        assert!(warnings[0].contains("BUZZ_CLI_PUSH_BINARY"), "{stderr}");
        assert!(!root.join(".local/bin/buzz").exists());
    }

    /// Any `.<tool>.tmp.*` still sitting in `dir`. A half-written binary that
    /// survives a failed install is the failure mode the temp-file dance exists
    /// to prevent, and it is per-tool because the two install into the same
    /// directory.
    fn leftover_temp_files(dir: &std::path::Path, tool: Tool) -> bool {
        let prefix = format!(".{}.tmp.", tool.name);
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
    }

    #[test]
    fn relay_urls_map_to_their_http_origin_for_git_auth() {
        assert_eq!(
            relay_http_base_url("wss://relay.example/"),
            "https://relay.example"
        );
        assert_eq!(
            relay_http_base_url("ws://localhost:8080"),
            "http://localhost:8080"
        );
        assert_eq!(
            relay_http_base_url("https://relay.example"),
            "https://relay.example"
        );
    }

    #[test]
    fn git_credential_env_is_emitted_only_when_the_helper_exists() {
        let agent = Agent::from_request(&request()).unwrap();
        let script = deploy_script(&agent, &config(), UNIT_TEMPLATE, &Pushes::default()).unwrap();
        assert!(script.contains(r#"cred=$(command -v git-credential-nostr 2>/dev/null || true)"#));
        assert!(script.contains(r#"if [ -n "$cred" ]; then"#));
        assert!(
            script.contains(r#"GIT_CONFIG_KEY_0="credential.https://relay.example/ws/git.helper""#)
        );
    }
}
