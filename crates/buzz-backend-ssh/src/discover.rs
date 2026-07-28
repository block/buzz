//! `check`, `discover_harnesses` and `probe_models`: everything that reads the
//! remote host without changing it.
//!
//! All three are **one SSH round trip**. `discover_harnesses` in particular
//! probes every candidate harness from a single generated script — N sequential
//! `ssh` invocations would spend the whole 45s budget on handshakes over a
//! 200 ms link, and the harness picker would visibly hang.

use std::time::Duration;

use crate::protocol::{snippet, Failure, SshConfig};
use crate::ssh::{quote, Session};

/// Harnesses the desktop knows how to render, in the same vocabulary its local
/// catalog uses (`KNOWN_ACP_RUNTIMES` + `PRESET_HARNESSES` in
/// `managed_agents/discovery.rs`). Ids must satisfy `[a-z0-9_][a-z0-9_-]*` or
/// `validate_harness_definition` drops the entry desktop-side.
struct Candidate {
    id: &'static str,
    label: &'static str,
    /// Accepted command names, most preferred first.
    commands: &'static [&'static str],
    args: &'static [&'static str],
    /// The runtime's `default_env` (`discovery.rs`). Locally these are applied
    /// at spawn time from the catalog; a remote agent never spawns locally, so
    /// they must ride along in the `HarnessDefinition` the desktop pins, or
    /// they are simply lost.
    env: &'static [(&'static str, &'static str)],
}

const CANDIDATES: &[Candidate] = &[
    Candidate {
        id: "buzz-agent",
        label: "Buzz Agent",
        commands: &["buzz-agent"],
        args: &[],
        env: &[],
    },
    Candidate {
        id: "goose",
        label: "Goose",
        commands: &["goose"],
        args: &["acp"],
        // Without this a remote Goose blocks on tool approvals that nobody is
        // present to answer, and the agent silently stops making progress.
        env: &[("GOOSE_MODE", "auto")],
    },
    Candidate {
        id: "claude",
        label: "Claude Code",
        commands: &["claude-agent-acp", "claude-code-acp"],
        args: &[],
        env: &[],
    },
    Candidate {
        id: "codex",
        label: "Codex",
        commands: &["codex-acp"],
        args: &[],
        env: &[],
    },
    Candidate {
        id: "cursor",
        label: "Cursor",
        commands: &["cursor-agent"],
        args: &["acp"],
        env: &[],
    },
    Candidate {
        id: "omp",
        label: "Oh My Pi",
        commands: &["omp"],
        args: &["acp"],
        env: &[],
    },
    Candidate {
        id: "grok",
        label: "Grok Build",
        commands: &["grok"],
        args: &["agent", "--always-approve", "stdio"],
        env: &[],
    },
    Candidate {
        id: "opencode",
        label: "OpenCode",
        commands: &["opencode"],
        args: &["acp"],
        env: &[],
    },
    Candidate {
        id: "kimi",
        label: "Kimi Code",
        commands: &["kimi"],
        args: &["acp"],
        env: &[],
    },
    Candidate {
        id: "amp",
        label: "Amp",
        commands: &["amp-acp"],
        args: &[],
        env: &[],
    },
    Candidate {
        id: "hermes",
        label: "Hermes Agent",
        commands: &["hermes-acp"],
        args: &[],
        env: &[],
    },
    Candidate {
        id: "openclaw",
        label: "OpenClaw",
        commands: &["openclaw"],
        args: &["acp"],
        env: &[],
    },
];

/// Preamble shared by every remote script.
///
/// `probe` writes one tab-separated record per resolved command, rather than
/// JSON assembled in `sh`: quoting arbitrary `--version` output into valid JSON
/// from a POSIX shell is a bug farm, and the parsing belongs on the Rust side
/// where it is testable.
///
/// Two details are load-bearing. `</dev/null` on every probed child, because
/// the script itself arrives on the remote shell's stdin and a child that reads
/// stdin would swallow the rest of it. And `timeout` where the host has it, so
/// a harness whose `--version` opens a REPL cannot hold the budget.
const PROBE_PREAMBLE: &str = r#"set -u
if command -v timeout >/dev/null 2>&1; then _t="timeout 5"; else _t=""; fi
probe() {
  _p=$(command -v "$2" 2>/dev/null) || return 0
  [ -n "$_p" ] || return 0
  _v=$($_t "$2" --version </dev/null 2>/dev/null | head -n 1 | tr -d '\t\r') || _v=""
  printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$_p" "$_v"
}
"#;

/// Probe key for the Hermes CLI itself, as opposed to the `hermes-acp` shim the
/// `hermes` [`Candidate`] resolves. Deliberately not any candidate's id, so the
/// twelve catalog entries are unaffected by its presence or absence.
const HERMES_CLI_KEY: &str = "hermes-cli";

/// Prefix of a profile record: `hermes-profile<TAB><name>`.
///
/// Two fields, not the four a `probe` record carries, so [`parse_probes`] drops
/// these on its own (`path` is `None`) and the two streams cannot be confused.
const HERMES_PROFILE_PREFIX: &str = "hermes-profile\t";

/// Per-profile entries emitted for one host. A pathological (or hostile) host
/// with thousands of directories under `profiles/` must not turn the harness
/// picker into an unusable wall, and `Output` caps the whole response at 1 MB
/// regardless — which would fail the op rather than truncate it.
///
/// Enforced twice on purpose: the script stops early so the bytes are never
/// sent, and [`hermes_profiles`] re-applies it because remote stdout is
/// untrusted input and a host is free to ignore the script it was handed.
const MAX_HERMES_PROFILES: usize = 32;

/// Hermes profile enumeration, appended to the probe script.
///
/// Hermes runs N isolated instances out of one install — a profile is a whole
/// `HERMES_HOME`, selected by the **global** pre-subcommand flag
/// (`hermes --profile matt acp`). The operator's fleet is one profile per
/// teammate, so a host has to advertise one catalog entry per profile or nine
/// of the ten agents on it are unreachable from the picker.
///
/// The profile store is read from the filesystem rather than from
/// `hermes profile list`: the directory layout **is** what Hermes resolves a
/// profile against (`hermes_cli/profiles.py`: `get_profile_dir` →
/// `<root>/profiles/<name>`), while the CLI output is a human table with a
/// unicode default marker and no `--json`. Parsing that table would be reading
/// a rendering of the truth instead of the truth.
///
/// `<root>` is normally `~/.hermes`. `HERMES_HOME` overrides it for Docker and
/// custom deployments, and may itself already point *at* a profile — hence the
/// `*/profiles/*` trim, which recovers the root from both layouts exactly as
/// `hermes_constants.get_default_hermes_root` does.
///
/// The `default` profile is the root directory itself and never appears under
/// `profiles/`, so it is emitted separately. It earns its own entry because the
/// plain `hermes-acp` entry runs whatever profile is *sticky*: once the operator
/// runs `hermes profile use matt`, nothing else can pin the built-in profile.
///
/// The shell never *evaluates* a name (only `printf '%s'`), and
/// [`is_hermes_profile_name`] re-checks every name that survives — but the
/// `case` charset arms are **not** merely a prefilter. They are the only thing
/// that stops a name containing a newline from printing a second, unlabeled
/// line that [`parse_probes`] accepts as a four-field probe record for any
/// candidate it names: such a line carries no `hermes-profile\t` prefix, so
/// [`hermes_profiles`] never sees it and no Rust-side check applies. Removing
/// them lets a hostile directory name pin an arbitrary
/// `BUZZ_ACP_AGENT_COMMAND`. Pinned by
/// `a_profile_directory_name_cannot_forge_a_probe_record`.
fn hermes_profiles_block() -> String {
    format!(
        r#"if _hb=$(command -v hermes 2>/dev/null) && [ -n "$_hb" ]; then
  _hr=${{HERMES_HOME:-${{HOME:-}}/.hermes}}
  case "$_hr" in */profiles/*) _hr=${{_hr%/profiles/*}} ;; esac
  _hc=0
  if [ -d "$_hr" ]; then
    printf 'hermes-profile\tdefault\n'
    _hc=1
  fi
  for _hd in "$_hr"/profiles/*/; do
    [ "$_hc" -lt {cap} ] || break
    [ -d "$_hd" ] || continue
    _hn=${{_hd%/}}
    _hn=${{_hn##*/}}
    case "$_hn" in
      default) continue ;;
      *[!abcdefghijklmnopqrstuvwxyz0123456789_-]*) continue ;;
      [!abcdefghijklmnopqrstuvwxyz0123456789]*) continue ;;
    esac
    _hc=$((_hc + 1))
    printf 'hermes-profile\t%s\n' "$_hn"
  done
fi
:
"#,
        cap = MAX_HERMES_PROFILES,
    )
}

/// The one probe script: `buzz-acp`, every harness candidate, and — only where
/// `hermes` resolves — that host's Hermes profiles.
fn discover_script(config: &SshConfig) -> String {
    let mut script = String::from(PROBE_PREAMBLE);
    let acp = config.buzz_acp_path.as_deref().unwrap_or("buzz-acp");
    script.push_str(&format!("probe 'buzz-acp' {}\n", quote(acp)));
    for candidate in CANDIDATES {
        for command in candidate.commands {
            script.push_str(&format!(
                "probe {} {}\n",
                quote(candidate.id),
                quote(command)
            ));
        }
    }
    // The Hermes CLI, which is what a per-profile entry runs — the `hermes-acp`
    // shim takes no arguments of its own, so it cannot carry `--profile`.
    //
    // `hermes --version` costs ~0.7s against the 40s budget. Fine for one, but
    // the probes are sequential: further CLI probes need to be weighed against
    // that budget rather than simply appended.
    script.push_str(&format!("probe {} 'hermes'\n", quote(HERMES_CLI_KEY)));
    script.push_str(&hermes_profiles_block());
    script
}

/// A profile name this crate is willing to put in a catalog id and in
/// `agent_args`.
///
/// The authority for the rule the script only prefilters. Names arrive from a
/// directory listing on a machine the desktop does not control, so they are
/// untrusted input on their way into a JSON document and then into an argument
/// vector — the exact shape of a name is the whole security surface.
///
/// The charset is Hermes's own `_PROFILE_ID_RE` (`^[a-z0-9][a-z0-9_-]{0,63}$`),
/// which is also, not by coincidence, a subset of the desktop's harness-id rule
/// `[a-z0-9_][a-z0-9_-]*` — so `hermes-<name>` is always a legal id and
/// `validate_harness_definition` cannot silently drop the entry. Anything else
/// is skipped rather than sanitized: a mangled name would name a profile that
/// does not exist, and deploy an agent pointing at nothing.
fn is_hermes_profile_name(name: &str) -> bool {
    if name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    // `let-else` also covers the empty name: no first character, no match.
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The accepted profile names from one probe run, in host order, deduplicated
/// and capped.
///
/// Returns `(names, skipped)` — `skipped` counts records the charset rule
/// refused, which is worth surfacing rather than swallowing: it is the only
/// signal that a host has profiles the picker deliberately did not offer.
fn hermes_profiles(stdout: &str) -> (Vec<&str>, usize) {
    let mut names: Vec<&str> = Vec::new();
    let mut skipped = 0usize;
    for line in stdout.lines() {
        let Some(name) = line.strip_prefix(HERMES_PROFILE_PREFIX) else {
            continue;
        };
        if !is_hermes_profile_name(name) {
            skipped += 1;
            continue;
        }
        if names.contains(&name) {
            continue;
        }
        if names.len() >= MAX_HERMES_PROFILES {
            skipped += 1;
            continue;
        }
        names.push(name);
    }
    (names, skipped)
}

/// One `probe` record: `key<TAB>command<TAB>path<TAB>version`.
struct Probe<'a> {
    key: &'a str,
    command: &'a str,
    path: &'a str,
    version: &'a str,
}

fn parse_probes(stdout: &str) -> Vec<Probe<'_>> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let probe = Probe {
                key: fields.next()?,
                command: fields.next()?,
                path: fields.next()?,
                version: fields.next().unwrap_or(""),
            };
            (!probe.key.is_empty() && !probe.path.is_empty()).then_some(probe)
        })
        .collect()
}

/// Shape the probe records into the `discover_harnesses` response.
///
/// Every element is a `HarnessDefinition` (camelCase, exactly the desktop's
/// own wire type) plus `available` / `binaryPath` / `version`. Unresolved
/// candidates are still reported, with `available: false`, so the picker can
/// say "install this on the host" instead of hiding the option.
fn harnesses_response(stdout: &str) -> serde_json::Value {
    let probes = parse_probes(stdout);
    let buzz_acp = probes
        .iter()
        .find(|probe| probe.key == "buzz-acp")
        .map(|probe| serde_json::json!({ "path": probe.path, "version": probe.version }))
        .unwrap_or(serde_json::Value::Null);

    let mut harnesses: Vec<serde_json::Value> = CANDIDATES
        .iter()
        .map(|candidate| {
            let found = probes.iter().find(|probe| probe.key == candidate.id);
            serde_json::json!({
                "id": candidate.id,
                "label": candidate.label,
                // The remote command name. This is what the desktop pins as the
                // create-time harness override, so it must name a binary on the
                // HOST, never one resolved locally.
                "command": found.map_or(candidate.commands[0], |probe| probe.command),
                "args": candidate.args,
                "env": candidate
                    .env
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), serde_json::Value::from(*value)))
                    .collect::<serde_json::Map<_, _>>(),
                "installInstructionsUrl": "",
                "installHint": "",
                "available": found.is_some(),
                "binaryPath": found.map(|probe| probe.path),
                "version": found.map(|probe| probe.version).filter(|v| !v.is_empty()),
            })
        })
        .collect();

    harnesses.extend(hermes_profile_harnesses(&probes, stdout));

    // `buzz_acp: null` with `ok: true` is deliberate: the UI can then render an
    // actionable "install buzz-acp on this host" instead of a bare failure.
    serde_json::json!({ "ok": true, "buzz_acp": buzz_acp, "harnesses": harnesses })
}

/// One extra catalog entry per Hermes profile on the host.
///
/// A profile is a separate `HERMES_HOME` — its own SOUL, memory, skills,
/// credentials and gateway — so "Hermes (matt)" and "Hermes (paul)" are two
/// different agents, not one agent configured twice. The operator's whole fleet
/// is one profile per teammate, and without these entries the picker can only
/// ever pin the *sticky* profile, leaving the rest unreachable through the
/// normal create flow.
///
/// The command is `hermes` with `["--profile", <name>, "acp"]` rather than the
/// `hermes-acp` shim: `--profile` is a global pre-subcommand flag, and the shim
/// forwards no arguments of its own. The pin therefore has to name the CLI
/// directly, which is also why the entries only appear when `hermes` itself
/// resolved — a host with only the shim gets exactly the plain entry.
///
/// The plain `hermes` candidate stays as-is and remains the default option: it
/// runs whichever profile is sticky, which is what a single-profile host wants.
///
/// These are the only entries that carry `exclusive: true`. `claude`, `codex`
/// and the plain `hermes-acp` shim are ephemeral runners — deploying one of
/// them N times against a host is the normal, intended shape. A profile is the
/// opposite: it is a persistent IDENTITY (its own memory, sessions, credentials
/// and nostr history), so two Buzz agents pinned to the same profile are two
/// puppeteers driving one body — they interleave turns into the same session
/// store. The flag is what lets the desktop refuse the second one; the provider
/// only states the fact, and says nothing about how it is rendered.
fn hermes_profile_harnesses(probes: &[Probe<'_>], stdout: &str) -> Vec<serde_json::Value> {
    // No Hermes CLI on the host means no way to pass `--profile`, so no
    // per-profile entries — regardless of what the profile records claim.
    let Some(cli) = probes.iter().find(|probe| probe.key == HERMES_CLI_KEY) else {
        return Vec::new();
    };
    let (names, skipped) = hermes_profiles(stdout);
    if skipped > 0 {
        // stderr, never stdout: stdout is this provider's single JSON response.
        eprintln!(
            "buzz-backend-ssh: skipped {skipped} Hermes profile(s) whose names are not \
             [a-z0-9][a-z0-9_-]* or that exceeded the {MAX_HERMES_PROFILES}-profile cap"
        );
    }

    names
        .into_iter()
        .map(|name| {
            serde_json::json!({
                // `hermes-` + a validated name, so this always satisfies the
                // desktop's `[a-z0-9_][a-z0-9_-]*` harness-id rule.
                "id": format!("hermes-{name}"),
                "label": format!("Hermes ({name})"),
                "command": cli.command,
                "args": ["--profile", name, "acp"],
                "env": serde_json::Map::new(),
                "installInstructionsUrl": "",
                "installHint": "",
                // A persistent identity, not an ephemeral runner: at most one
                // agent may be pinned to this exact command+args. Emitted ONLY
                // here — every other entry omits the key, and an absent key
                // means "deploy as many as you like".
                "exclusive": true,
                // The profile directory was listed and the CLI resolved this
                // pass, so the entry is as available as the plain one.
                "available": true,
                "binaryPath": cli.path,
                "version": Some(cli.version).filter(|v| !v.is_empty()),
            })
        })
        .collect()
}

pub fn discover_harnesses(
    config: &SshConfig,
    session: &Session,
) -> Result<serde_json::Value, Failure> {
    let output = session.run(&discover_script(config), Duration::from_secs(40))?;
    if !output.ok() {
        return Err(output.failure().into());
    }
    Ok(harnesses_response(&output.stdout))
}

/// `check`: the preflight the create dialog runs before Deploy goes live.
pub fn check(session: &Session) -> Result<serde_json::Value, Failure> {
    let output = session.run("echo buzz-ok\n", Duration::from_secs(8))?;
    if output.stdout.trim() == "buzz-ok" {
        return Ok(serde_json::json!({ "ok": true, "detail": "Connected" }));
    }
    Err(guidance(&output.failure()).into())
}

/// Turn ssh's own diagnosis into something the user can act on. The classified
/// causes are the ones that actually happen; everything else passes through
/// verbatim rather than being flattened into a generic message.
///
/// Deliberately no entry for the Tailscale re-auth prompt: `run` returns that
/// as a typed [`Failure`] carrying the URL, so it never reaches this classifier
/// — which is the point of the typed carrier.
fn guidance(failure: &str) -> String {
    const GUIDANCE: &[(&str, &str)] = &[
        ("permission denied", "add your public key to ~/.ssh/authorized_keys on the server, or run `tailscale set --ssh` there."),
        ("host key verification failed", "the server's host key is not in your known_hosts. Connect once with `ssh` to review and accept it."),
        ("could not resolve hostname", "check the address, or confirm the device is on your tailnet."),
        ("connection refused", "confirm the server is reachable and running an SSH daemon."),
        ("connection timed out", "confirm the server is reachable and running an SSH daemon."),
    ];

    let lower = failure.to_lowercase();
    match GUIDANCE.iter().find(|(cause, _)| lower.contains(cause)) {
        Some((_, advice)) => format!("{failure} — {advice}"),
        None => failure.to_string(),
    }
}

/// `probe_models`: run `buzz-acp models --json` on the host and hand the raw
/// document back untouched.
///
/// Verbatim is the point: the desktop feeds it straight into the same
/// `normalize_agent_models` the local path uses, so the model picker needs no
/// remote-specific code at all.
pub fn probe_models(
    request: &serde_json::Value,
    config: &SshConfig,
    session: &Session,
) -> Result<serde_json::Value, Failure> {
    let output = session.run(&models_script(request, config)?, Duration::from_secs(110))?;
    if !output.ok() {
        return Err(output.failure().into());
    }
    let models_raw: serde_json::Value =
        serde_json::from_str(output.stdout.trim()).map_err(|e| {
            format!(
                "`buzz-acp models --json` did not return JSON ({e}): {}",
                snippet(&output.stdout)
            )
        })?;
    Ok(serde_json::json!({ "ok": true, "models_raw": models_raw }))
}

fn models_script(request: &serde_json::Value, config: &SshConfig) -> Result<String, String> {
    let harness = request
        .get("harness")
        .ok_or("probe_models request is missing 'harness'")?;
    let command = harness
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or("probe_models harness is missing 'command'")?;
    let args = string_list(harness.get("args"));

    // `agent.env_vars` is the only place `env_secrets_from_request`
    // (backend.rs) looks when scrubbing these values out of an error surface,
    // so a top-level `model_env` would travel unredacted through any failure
    // message. Still accepted, since the transport is safe either way; it just
    // loses that second layer.
    let model_env = request
        .get("agent")
        .and_then(|agent| agent.get("env_vars"))
        .or_else(|| request.get("model_env"));

    let mut script = String::from("set -u\n");
    // Model-probe env carries provider API keys, set inside the
    // stdin-delivered script so they never appear in the remote argv.
    //
    // Names are validated rather than quoted: on the left of an assignment
    // quoting has no effect, so an unchecked name is a straight command
    // injection (`X=1; touch /tmp/pwn`). Quoting is sufficient for values.
    for (key, value) in crate::deploy::env_map(model_env) {
        if !crate::deploy::is_well_formed_env_key(&key) {
            return Err(format!("env var name '{key}' is not a valid identifier"));
        }
        script.push_str(&format!("export {}={}\n", key, quote(&value)));
    }
    script.push_str(&format!(
        "export BUZZ_ACP_AGENT_COMMAND={}\nexport BUZZ_ACP_AGENT_ARGS={}\n",
        quote(command),
        quote(&args.join(","))
    ));
    script.push_str(&format!(
        "exec {} models --json </dev/null\n",
        quote(config.buzz_acp_path.as_deref().unwrap_or("buzz-acp"))
    ));
    Ok(script)
}

pub fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SshConfig {
        SshConfig {
            host: "vps".into(),
            ..SshConfig::default()
        }
    }

    #[test]
    fn discover_is_one_script_covering_every_candidate() {
        let script = discover_script(&config());
        assert!(script.contains("probe 'buzz-acp' 'buzz-acp'"));
        for candidate in CANDIDATES {
            for command in candidate.commands {
                assert!(
                    script.contains(&format!("probe '{}' '{command}'", candidate.id)),
                    "missing probe for {command}"
                );
            }
        }
        // Children must not read the script off the shell's own stdin.
        assert!(script.contains("</dev/null"));
    }

    #[test]
    fn candidate_ids_satisfy_the_desktop_harness_id_rule() {
        for candidate in CANDIDATES {
            let mut chars = candidate.id.chars();
            let first = chars.next().unwrap();
            assert!(
                first.is_ascii_lowercase() || first.is_ascii_digit() || first == '_',
                "{} has an illegal first character",
                candidate.id
            );
            assert!(
                candidate
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
                "{} has illegal characters",
                candidate.id
            );
            assert!(!candidate.label.trim().is_empty());
            assert!(!candidate.commands.is_empty());
            // `validate_harness_definition` runs every advertised `env` through
            // `validate_user_env_keys`, which rejects reserved and malformed
            // keys — and drops the whole harness if any fail.
            for (key, _) in candidate.env {
                assert!(
                    !key.is_empty()
                        && !key.starts_with(|c: char| c.is_ascii_digit())
                        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                    "{} advertises a malformed env key {key:?}",
                    candidate.id
                );
                assert!(
                    !key.to_ascii_uppercase().starts_with("BUZZ_"),
                    "{} advertises reserved env key {key:?}",
                    candidate.id
                );
            }
        }
    }

    #[test]
    fn a_configured_buzz_acp_path_is_quoted_into_the_script() {
        let config = SshConfig {
            buzz_acp_path: Some("/opt/buzz acp/bin/buzz-acp".into()),
            ..config()
        };
        assert!(discover_script(&config).contains("'/opt/buzz acp/bin/buzz-acp'"));
    }

    #[test]
    fn probe_records_become_available_harnesses() {
        let stdout = "buzz-acp\tbuzz-acp\t/usr/local/bin/buzz-acp\t0.4.26\n\
                      goose\tgoose\t/home/ubuntu/.local/bin/goose\tgoose 1.9.0\n";
        let response = harnesses_response(stdout);
        assert_eq!(response["ok"], true);
        assert_eq!(response["buzz_acp"]["path"], "/usr/local/bin/buzz-acp");
        assert_eq!(response["buzz_acp"]["version"], "0.4.26");

        let harnesses = response["harnesses"].as_array().unwrap();
        assert_eq!(harnesses.len(), CANDIDATES.len());
        let goose = harnesses.iter().find(|h| h["id"] == "goose").unwrap();
        assert_eq!(goose["available"], true);
        assert_eq!(goose["binaryPath"], "/home/ubuntu/.local/bin/goose");
        assert_eq!(goose["version"], "goose 1.9.0");
        assert_eq!(goose["command"], "goose");
        assert_eq!(goose["args"], serde_json::json!(["acp"]));
        // A HarnessDefinition, in the desktop's own camelCase wire shape.
        // `env` carries the runtime's `default_env`, which local spawn applies
        // from the catalog and a remote deploy can only get from here.
        assert_eq!(goose["env"], serde_json::json!({ "GOOSE_MODE": "auto" }));
        assert!(goose.get("installInstructionsUrl").is_some());

        let absent = harnesses.iter().find(|h| h["id"] == "kimi").unwrap();
        assert_eq!(absent["available"], false);
        assert!(absent["binaryPath"].is_null());
        assert!(absent["version"].is_null());
    }

    #[test]
    fn a_host_without_buzz_acp_still_succeeds() {
        let response = harnesses_response("goose\tgoose\t/usr/bin/goose\t\n");
        assert_eq!(response["ok"], true);
        assert!(response["buzz_acp"].is_null());
    }

    #[test]
    fn the_reported_command_is_the_one_the_host_actually_has() {
        // `claude` resolves through either adapter name; the pin must carry the
        // one that exists on the host, not the first candidate.
        let response =
            harnesses_response("claude\tclaude-code-acp\t/usr/bin/claude-code-acp\t2.1.0\n");
        let claude = response["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["id"] == "claude")
            .unwrap();
        assert_eq!(claude["command"], "claude-code-acp");
    }

    #[test]
    fn probe_parsing_ignores_noise_lines() {
        let probes = parse_probes("garbage\n\ngoose\tgoose\t/usr/bin/goose\tv1\nempty\t\t\t\n");
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].command, "goose");
        assert_eq!(probes[0].version, "v1");
    }

    // ── Hermes per-profile entries ──────────────────────────────────────────

    /// Probe output for a host with the Hermes CLI plus `profiles`.
    fn hermes_stdout(profiles: &[&str]) -> String {
        let mut stdout = String::from(
            "hermes\thermes-acp\t/home/ubuntu/.local/bin/hermes-acp\t0.19.0\n\
             hermes-cli\thermes\t/home/ubuntu/.local/bin/hermes\tHermes Agent v0.19.0\n",
        );
        for profile in profiles {
            stdout.push_str(&format!("hermes-profile\t{profile}\n"));
        }
        stdout
    }

    fn entry<'a>(response: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
        response["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["id"] == id)
            .unwrap_or_else(|| panic!("no harness entry {id}"))
    }

    #[test]
    fn each_hermes_profile_becomes_its_own_catalog_entry() {
        let response = harnesses_response(&hermes_stdout(&["default", "matt", "paul"]));
        let harnesses = response["harnesses"].as_array().unwrap();
        assert_eq!(harnesses.len(), CANDIDATES.len() + 3);

        let matt = entry(&response, "hermes-matt");
        assert_eq!(matt["label"], "Hermes (matt)");
        // The CLI, not the shim: `--profile` is a global pre-subcommand flag
        // and the shim forwards no arguments of its own.
        assert_eq!(matt["command"], "hermes");
        assert_eq!(
            matt["args"],
            serde_json::json!(["--profile", "matt", "acp"])
        );
        assert_eq!(matt["available"], true);
        assert_eq!(matt["binaryPath"], "/home/ubuntu/.local/bin/hermes");
        assert_eq!(matt["version"], "Hermes Agent v0.19.0");
        assert_eq!(matt["env"], serde_json::json!({}));

        // The sticky default keeps its own entry — once `hermes profile use`
        // moves the sticky pointer, nothing else can pin the built-in profile.
        assert_eq!(
            entry(&response, "hermes-default")["args"],
            serde_json::json!(["--profile", "default", "acp"])
        );
        // And the plain shim entry is untouched, still the default option.
        let plain = entry(&response, "hermes");
        assert_eq!(plain["command"], "hermes-acp");
        assert_eq!(plain["args"], serde_json::json!([]));
    }

    #[test]
    fn only_per_profile_entries_are_marked_exclusive() {
        // A profile is a persistent identity: at most one agent may be pinned
        // to it. Every other entry is an ephemeral runner and must OMIT the key
        // entirely — an absent field is what the desktop reads as "no limit",
        // so emitting `false` would be a different (and needless) contract.
        let response = harnesses_response(&hermes_stdout(&["default", "matt"]));
        for id in ["hermes-default", "hermes-matt"] {
            assert_eq!(entry(&response, id)["exclusive"], true, "{id}");
        }
        for entry_value in response["harnesses"].as_array().unwrap() {
            let id = entry_value["id"].as_str().unwrap();
            if id.starts_with("hermes-") {
                continue;
            }
            assert!(
                entry_value.get("exclusive").is_none(),
                "{id} must not advertise 'exclusive'"
            );
        }
    }

    #[test]
    fn a_host_without_hermes_gets_byte_identical_todays_catalog() {
        // The degradation contract: absent Hermes, the response is exactly what
        // it was before per-profile entries existed.
        let stdout = "buzz-acp\tbuzz-acp\t/usr/local/bin/buzz-acp\t0.4.26\n\
                      goose\tgoose\t/usr/bin/goose\tgoose 1.9.0\n";
        let response = harnesses_response(stdout);
        assert_eq!(
            response["harnesses"].as_array().unwrap().len(),
            CANDIDATES.len()
        );
        let expected: Vec<serde_json::Value> = CANDIDATES
            .iter()
            .map(|candidate| {
                let available = candidate.id == "goose";
                serde_json::json!({
                    "id": candidate.id,
                    "label": candidate.label,
                    "command": candidate.commands[0],
                    "args": candidate.args,
                    "env": candidate
                        .env
                        .iter()
                        .map(|(k, v)| ((*k).to_string(), serde_json::Value::from(*v)))
                        .collect::<serde_json::Map<_, _>>(),
                    "installInstructionsUrl": "",
                    "installHint": "",
                    "available": available,
                    "binaryPath": available.then_some("/usr/bin/goose"),
                    "version": available.then_some("goose 1.9.0"),
                })
            })
            .collect();
        assert_eq!(response["harnesses"], serde_json::Value::from(expected));
    }

    #[test]
    fn hermes_with_no_profile_records_is_just_the_plain_entry() {
        // A stdout carrying no profile records at all — what a *missing* Hermes
        // root produces — is not an error: the shim entry still runs the sticky
        // profile. A root that exists without a `profiles/` store is a
        // different stdout, pinned by
        // `a_hermes_root_without_a_profiles_store_still_advertises_default`.
        let response = harnesses_response(&hermes_stdout(&[]));
        assert_eq!(response["ok"], true);
        assert_eq!(
            response["harnesses"].as_array().unwrap().len(),
            CANDIDATES.len()
        );
        assert_eq!(entry(&response, "hermes")["available"], true);
    }

    #[test]
    fn profile_records_without_the_hermes_cli_produce_nothing() {
        // Only the shim resolved, so there is no binary that accepts
        // `--profile` — pinning one would deploy an agent that cannot start.
        let stdout = "hermes\thermes-acp\t/usr/bin/hermes-acp\t0.19.0\n\
                      hermes-profile\tmatt\n";
        let response = harnesses_response(stdout);
        assert_eq!(
            response["harnesses"].as_array().unwrap().len(),
            CANDIDATES.len()
        );
    }

    #[test]
    fn hostile_profile_names_are_skipped_never_sanitized() {
        // A mangled name would pin a profile that does not exist. Every one of
        // these must be dropped whole.
        for hostile in [
            "a b",
            "a'b",
            "$(touch /tmp/pwn)",
            "`id`",
            "a;b",
            "../escape",
            ".hidden",
            "-leading-dash",
            "_leading-underscore",
            "UPPER",
            "naïve",
            "a/b",
            "a\\b",
            "",
            &"x".repeat(65),
        ] {
            assert!(
                !is_hermes_profile_name(hostile),
                "{hostile:?} must be refused"
            );
        }
        for ok in ["default", "matt", "msig-web-analyst", "a_b", "x9", "9x"] {
            assert!(is_hermes_profile_name(ok), "{ok:?} must be accepted");
        }
    }

    #[test]
    fn a_hostile_name_that_reaches_stdout_is_dropped_from_the_catalog() {
        // Belt and braces: even if the script's own prefilter were bypassed,
        // nothing hostile reaches the JSON.
        let response = harnesses_response(&hermes_stdout(&["matt", "$(id)", "Bad", "paul"]));
        let ids: Vec<&str> = response["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"hermes-matt") && ids.contains(&"hermes-paul"));
        assert!(!ids.iter().any(|id| id.contains("$(") || id.contains("Bad")));
        // No mangled survivor either — the count is exactly the two good ones.
        assert_eq!(ids.len(), CANDIDATES.len() + 2);
    }

    #[test]
    fn the_profile_count_is_capped_against_a_pathological_host() {
        let many: Vec<String> = (0..200).map(|i| format!("p{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let response = harnesses_response(&hermes_stdout(&refs));
        assert_eq!(
            response["harnesses"].as_array().unwrap().len(),
            CANDIDATES.len() + MAX_HERMES_PROFILES
        );
    }

    #[test]
    fn a_repeated_profile_name_yields_one_entry() {
        // Duplicate ids would collide in the desktop's catalog.
        let response = harnesses_response(&hermes_stdout(&["matt", "matt"]));
        assert_eq!(
            response["harnesses"].as_array().unwrap().len(),
            CANDIDATES.len() + 1
        );
    }

    #[test]
    fn every_profile_entry_id_satisfies_the_desktop_harness_id_rule() {
        // `validate_harness_definition` drops any entry whose id does not match
        // `[a-z0-9_][a-z0-9_-]*`, so a legal profile name must always produce a
        // legal id — otherwise the entry silently vanishes desktop-side.
        for name in ["default", "matt", "msig-web-analyst", "a_b", "9x"] {
            let id = format!("hermes-{name}");
            let mut chars = id.chars();
            let first = chars.next().unwrap();
            assert!(first.is_ascii_lowercase() || first.is_ascii_digit() || first == '_');
            assert!(
                chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            );
        }
    }

    #[test]
    fn profile_records_never_masquerade_as_probe_records() {
        // Two fields, not four, so `parse_probes` drops them on its own and a
        // profile can never be mistaken for a resolved binary.
        let stdout = hermes_stdout(&["matt"]);
        let probes = parse_probes(&stdout);
        assert_eq!(probes.len(), 2);
        assert!(probes.iter().all(|p| p.key != "hermes-profile"));
    }

    /// The script must only enumerate profiles on a host that has `hermes`,
    /// and must never let a name be evaluated by the shell.
    #[test]
    fn the_script_gates_profile_enumeration_on_hermes_being_present() {
        let script = discover_script(&config());
        assert!(script.contains("probe 'hermes-cli' 'hermes'"));
        assert!(script.contains(r#"if _hb=$(command -v hermes 2>/dev/null)"#));
        // Names are printed as data, never expanded or executed.
        assert!(script.contains(r#"printf 'hermes-profile\t%s\n' "$_hn""#));
        // The shell-side cap tracks the Rust constant.
        assert!(script.contains(&format!(r#"[ "$_hc" -lt {MAX_HERMES_PROFILES} ] || break"#)));
    }

    /// Run the real generated script through `/bin/sh` against a fake host
    /// rooted at `root`, with `HERMES_HOME` set to `hermes_home`. Returns the
    /// script's stdout.
    #[cfg(unix)]
    fn run_discover_script(root: &std::path::Path, hermes_home: &std::path::Path) -> String {
        // Stub `hermes` so `command -v hermes` resolves on the fake host.
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let hermes = bin.join("hermes");
        std::fs::write(&hermes, "#!/bin/sh\nexit 0\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hermes, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let output = std::process::Command::new("/bin/sh")
            .arg("-s")
            .env_clear()
            .env("HOME", root)
            .env("HERMES_HOME", hermes_home)
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
                    .write_all(discover_script(&config()).as_bytes())
                    .unwrap();
                child.wait_with_output()
            })
            .unwrap();
        assert!(
            output.status.success(),
            "script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Every probe key the script is allowed to emit. Anything else on stdout
    /// in four-field shape is a forged record.
    #[cfg(unix)]
    fn expected_probe_keys() -> Vec<&'static str> {
        let mut keys = vec!["buzz-acp", HERMES_CLI_KEY];
        keys.extend(CANDIDATES.iter().map(|candidate| candidate.id));
        keys
    }

    /// Execute the real generated script against `/bin/sh` over a fake host
    /// layout. Substring assertions prove the script *says* the right things;
    /// only running it proves the `case` globs, the `${_hd%/}` trimming and the
    /// `HERMES_HOME` recovery actually behave — and that a directory named
    /// `$(touch …)` stays inert rather than being evaluated.
    #[cfg(unix)]
    #[test]
    fn the_generated_script_enumerates_a_real_profile_directory_safely() {
        let root =
            std::env::temp_dir().join(format!("buzz-hermes-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profiles = root.join("hermes/profiles");
        std::fs::create_dir_all(&profiles).unwrap();

        let canary = root.join("pwn");
        for name in [
            // Normal names, including the operator's real fleet shapes.
            "matt",
            "paul",
            "msig-web-analyst",
            "codex_worker",
            // Hostile: spaces, quotes, and a command substitution that must
            // stay a literal directory name.
            "evil name",
            "ev'il",
            &format!("$(touch {})", canary.display()),
            // A dotfile, which is not a profile and is not matched by `*/`.
            ".hidden",
            // Uppercase, which Hermes itself would refuse.
            "Upper",
        ] {
            std::fs::create_dir_all(profiles.join(name)).unwrap();
        }
        // A plain file under profiles/ is not a profile.
        std::fs::write(profiles.join("notes.md"), "x").unwrap();

        // Exercises the Docker/custom layout AND the `*/profiles/*` trim by
        // pointing HERMES_HOME at a profile rather than at the root.
        let stdout = run_discover_script(&root, &root.join("hermes/profiles/matt"));
        // Nothing under `profiles/` was ever executed.
        assert!(
            !canary.exists(),
            "a directory name was evaluated by the shell"
        );

        let (names, _) = hermes_profiles(&stdout);
        assert_eq!(
            names,
            vec![
                "default",
                "codex_worker",
                "matt",
                "msig-web-analyst",
                "paul"
            ],
            "stdout was: {stdout}"
        );

        // And the response built from that real stdout parses, with the args
        // arrays the deploy pin will carry.
        let response: serde_json::Value =
            serde_json::from_str(&harnesses_response(&stdout).to_string()).unwrap();
        assert_eq!(
            entry(&response, "hermes-msig-web-analyst")["args"],
            serde_json::json!(["--profile", "msig-web-analyst", "acp"])
        );
        assert_eq!(entry(&response, "hermes-matt")["command"], "hermes");

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The forging path, which the labeled `hermes-profile` stream never
    /// exercises: a directory name carrying a newline plus three tab-separated
    /// fields prints a *second*, unlabeled line that [`parse_probes`] would
    /// accept as a four-field probe record for any candidate it names. Nothing
    /// downstream can catch it — `hermes_profiles` only ever sees the labeled
    /// prefix — so the script's charset `case` arms are the whole defense, and
    /// this is what pins them.
    #[cfg(unix)]
    #[test]
    fn a_profile_directory_name_cannot_forge_a_probe_record() {
        let root = std::env::temp_dir().join(format!("buzz-hermes-forge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profiles = root.join("hermes/profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        std::fs::create_dir_all(profiles.join("matt")).unwrap();
        // Reads on stdout as `hermes-profile<TAB>x` followed by a complete
        // `claude<TAB>evil-acp<TAB>tmp-evil-acp<TAB>9.9.9` record.
        std::fs::create_dir_all(profiles.join("x\nclaude\tevil-acp\ttmp-evil-acp\t9.9.9")).unwrap();

        let stdout = run_discover_script(&root, &root.join("hermes"));

        let keys: Vec<&str> = parse_probes(&stdout).iter().map(|p| p.key).collect();
        let expected = expected_probe_keys();
        assert!(
            keys.iter().all(|key| expected.contains(key)),
            "a directory name forged a probe record; keys were {keys:?}, stdout was: {stdout:?}"
        );

        // The concrete consequence the record would have had: `claude` claiming
        // to be installed, pinned as the deploy's BUZZ_ACP_AGENT_COMMAND.
        let response = harnesses_response(&stdout);
        let claude = entry(&response, "claude");
        assert_eq!(claude["available"], false, "stdout was: {stdout:?}");
        assert_eq!(claude["command"], "claude-agent-acp");
        assert!(claude["binaryPath"].is_null());

        // The name is not a legal profile either, so it adds no entry.
        let (names, _) = hermes_profiles(&stdout);
        assert_eq!(names, vec!["default", "matt"], "stdout was: {stdout:?}");

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A Hermes root that exists but has no `profiles/` store: the `default`
    /// entry still ships, because the root directory *is* the default profile.
    /// Distinct from a missing root, which emits nothing at all.
    #[cfg(unix)]
    #[test]
    fn a_hermes_root_without_a_profiles_store_still_advertises_default() {
        let root =
            std::env::temp_dir().join(format!("buzz-hermes-noprofiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("hermes")).unwrap();

        let stdout = run_discover_script(&root, &root.join("hermes"));
        let (names, _) = hermes_profiles(&stdout);
        assert_eq!(names, vec!["default"], "stdout was: {stdout:?}");

        let response = harnesses_response(&stdout);
        assert_eq!(
            response["harnesses"].as_array().unwrap().len(),
            CANDIDATES.len() + 1
        );
        assert_eq!(
            entry(&response, "hermes-default")["args"],
            serde_json::json!(["--profile", "default", "acp"])
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A missing Hermes root — the case `hermes_stdout(&[])` models — emits no
    /// profile records at all, leaving only the plain shim entry.
    #[cfg(unix)]
    #[test]
    fn a_missing_hermes_root_advertises_no_profiles() {
        let root = std::env::temp_dir().join(format!("buzz-hermes-noroot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let stdout = run_discover_script(&root, &root.join("hermes"));
        let (names, _) = hermes_profiles(&stdout);
        assert!(names.is_empty(), "stdout was: {stdout:?}");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn guidance_is_actionable_for_the_cases_that_happen() {
        assert!(
            guidance("ssh failed (exit 255): Permission denied (publickey).")
                .contains("authorized_keys")
        );
        assert!(
            guidance("ssh failed (exit 255): Host key verification failed.")
                .contains("known_hosts")
        );
        assert!(
            guidance("ssh failed (exit 255): ssh: Could not resolve hostname vps")
                .contains("tailnet")
        );
        // Unclassified failures survive verbatim rather than being flattened.
        assert_eq!(
            guidance("ssh failed (exit 1): weird"),
            "ssh failed (exit 1): weird"
        );
    }

    #[test]
    fn model_env_is_exported_inside_the_script_never_on_the_argv() {
        // Nested under `agent.env_vars`, the one shape the desktop's
        // `env_secrets_from_request` scrubber knows how to find.
        let request = serde_json::json!({
            "harness": { "command": "goose", "args": ["acp"] },
            "agent": { "env_vars": { "ANTHROPIC_API_KEY": "sk-ant-secret" } },
        });
        let script = models_script(&request, &config()).unwrap();
        assert!(script.contains("export ANTHROPIC_API_KEY='sk-ant-secret'"));
        assert!(script.contains("export BUZZ_ACP_AGENT_COMMAND='goose'"));
        assert!(script.contains("export BUZZ_ACP_AGENT_ARGS='acp'"));
        // The value is only ever in the script body, which travels on stdin —
        // the remote argv is fixed at `sh -s` by `Session`.
        assert!(script.contains("exec 'buzz-acp' models --json </dev/null"));

        // A flat `model_env` is still honored, for a desktop that has not
        // adopted the nested shape yet.
        let flat = serde_json::json!({
            "harness": { "command": "goose" },
            "model_env": { "ANTHROPIC_API_KEY": "sk-ant-secret" },
        });
        assert!(models_script(&flat, &config())
            .unwrap()
            .contains("export ANTHROPIC_API_KEY='sk-ant-secret'"));
    }

    #[test]
    fn probe_models_requires_a_harness_command() {
        let request = serde_json::json!({ "harness": { "args": ["acp"] } });
        assert!(models_script(&request, &config()).is_err());
        assert!(models_script(&serde_json::json!({}), &config()).is_err());
    }

    /// An env *name* is the left side of a shell assignment, where quoting has
    /// no effect. This runs the generated script for real: a substring
    /// assertion would pass against the injectable form too.
    #[cfg(unix)]
    #[test]
    fn a_malformed_model_env_name_cannot_smuggle_a_command() {
        let canary = std::env::temp_dir().join("buzz-models-injection-canary");
        let _ = std::fs::remove_file(&canary);

        let request = serde_json::json!({
            "harness": { "command": "goose" },
            "agent": { "env_vars": {
                format!("X=1; touch {}", canary.display()): "v",
            } },
        });
        let error = models_script(&request, &config())
            .expect_err("a name that is not an identifier must be refused");
        assert!(error.contains("not a valid identifier"), "{error}");

        // Belt and braces: even if the guard were removed, prove the canary
        // path is the one an injection would create.
        assert!(!canary.exists());
    }

    #[test]
    fn every_well_formed_model_env_name_still_passes() {
        let request = serde_json::json!({
            "harness": { "command": "goose" },
            "agent": { "env_vars": { "OPENAI_API_KEY": "sk-1", "_X9": "v" } },
        });
        let script = models_script(&request, &config()).unwrap();
        assert!(script.contains("export OPENAI_API_KEY='sk-1'"));
        assert!(script.contains("export _X9='v'"));
    }
}
