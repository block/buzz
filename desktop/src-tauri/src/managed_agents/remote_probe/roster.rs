//! Durable agent-roster enumeration for a harness found on a probed host.
//!
//! Host discovery answers "which harnesses run here". This answers the next
//! question: "which *named agents* does that harness hold, and which one is its
//! primary". A resident harness commonly contains several durable agents, and
//! connecting the host must expose them for selection without enrolling the
//! whole stack — the user picks one, several, or none.
//!
//! Two boundaries keep this from becoming a harness-specific branch inside Buzz:
//!
//! - **Neutral output.** Callers see [`RemoteAgentCandidate`], which has no
//!   OpenClaw vocabulary in it. Nothing downstream — storage, UI, or the connect
//!   command — learns which harness produced a candidate beyond its id string.
//! - **Table-driven input.** Everything harness-specific lives in
//!   [`ROSTER_RECIPES`]: one remote command and one parser per harness. Adding a
//!   harness is a row, not a code path, which is the same shape the host probe
//!   uses for its binary table.
//!
//! Ephemeral, per-turn workers are deliberately absent. A harness's durable
//! roster is what it has *configured*; a worker spawned to service one request
//! is internal to its parent and is not an enrollment candidate. For OpenClaw
//! that distinction is free — `agents list` reports configured agents only.

use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{
    classify_ssh_failure, failure_message, first_line, ssh_probe_args, wait_with_timeout,
    HostProbeErrorKind,
};
use crate::managed_agents::ssh_config::{resolve_ssh_binary, SshHost};

/// Wall-clock ceiling for one roster query.
///
/// Shorter than the host probe's budget: that one runs a loop over every known
/// harness binary, while this runs a single command whose harness has already
/// been proven present. A harness that cannot answer in this long is reported as
/// timed out rather than allowed to wedge the connect dialog.
const ROSTER_TIMEOUT: Duration = Duration::from_secs(15);

/// Marks the start of parseable output, so a login shell's banners, MOTD, or
/// rc-file chatter can be discarded.
const ROSTER_START: &str = "__BUZZ_ROSTER_START__";

/// Marks the end. Required, not optional: without it a session that died
/// mid-command is indistinguishable from a harness that genuinely holds no
/// agents, and "you have no agents" is a much worse lie than "the query was cut
/// off". The host probe learned this the hard way.
const ROSTER_END: &str = "__BUZZ_ROSTER_END__";

/// One durable, named agent a harness reports.
///
/// Harness-neutral by construction. `agent_id` is the harness's own identifier
/// for routing a message to exactly this agent; everything else is presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentCandidate {
    /// Harness that reported this agent, matching `RemoteHarness::id`.
    pub harness_id: String,
    /// The harness's own agent identifier. This is the routing key: a reply must
    /// be produced by this exact agent, never a parent or sibling.
    pub agent_id: String,
    /// Best available human label. Falls back to `agent_id` when the harness
    /// reports no name, which is normal for a primary agent.
    pub display_name: String,
    /// True when the harness identifies this candidate as its primary. If the
    /// harness reports no default and has no `main` candidate, none are marked.
    pub is_primary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// How many routing bindings the harness already has for this agent. Purely
    /// informational: a bound agent is still a legal candidate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_count: Option<u32>,
}

/// Outcome of one roster query.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessRosterResult {
    pub host: String,
    pub harness_id: String,
    pub ok: bool,
    pub duration_ms: u64,
    /// False when Buzz has no recipe for this harness. Distinct from `ok:
    /// false`: nothing is wrong with the host, Buzz simply cannot enumerate
    /// that harness yet, and the UI should offer manual entry instead of an
    /// error.
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<HostProbeErrorKind>,
    pub candidates: Vec<RemoteAgentCandidate>,
}

/// How to enumerate one harness's durable agents.
struct RosterRecipe {
    harness_id: &'static str,
    /// Remote command. Must be a literal containing no single quote: it is
    /// single-quoted into a login-shell invocation, and no user input is ever
    /// interpolated into it. [`recipes_are_quote_safe`] enforces both halves.
    command: &'static str,
    parse: fn(&str) -> Result<Vec<RemoteAgentCandidate>, String>,
}

const ROSTER_RECIPES: &[RosterRecipe] = &[RosterRecipe {
    harness_id: "openclaw",
    command: "openclaw agents list --json",
    parse: parse_openclaw_roster,
}];

fn recipe_for(harness_id: &str) -> Option<&'static RosterRecipe> {
    ROSTER_RECIPES
        .iter()
        .find(|recipe| recipe.harness_id == harness_id)
}

/// Build the remote command for a recipe.
///
/// `exec $SHELL -lc` — login but *not* interactive. An interactive shell sources
/// rc files where prompt frameworks and completion plugins live, several of
/// which block without a TTY; the host probe hung on exactly that. Login alone
/// still resolves the PATH a user's harness was installed into.
/// The markers are emitted with `echo`, not `printf '%s\n'`: the whole inner
/// command is single-quoted into the outer argv, so any single quote inside it
/// would terminate that quoting early and hand the remainder to the shell as
/// code. `echo` needs no quotes because a marker is a bare identifier.
/// [`the_assembled_command_has_exactly_one_quoted_region`] holds this line.
fn build_roster_command(recipe: &RosterRecipe) -> String {
    let inner = format!(
        "echo {ROSTER_START}; {} 2>/dev/null; echo {ROSTER_END}",
        recipe.command
    );
    format!("exec $SHELL -lc '{inner}'")
}

/// Extract the payload between the markers.
///
/// Returns `None` when either marker is missing — the caller turns that into a
/// truncation error rather than an empty roster.
fn extract_payload(stdout: &str) -> Option<&str> {
    let start = stdout.find(ROSTER_START)? + ROSTER_START.len();
    let rest = &stdout[start..];
    let end = rest.find(ROSTER_END)?;
    Some(rest[..end].trim())
}

/// OpenClaw's `agents list --json` row.
///
/// Only the fields Buzz uses are named; the harness emits more and is free to
/// add others. `identity_name` wins over `name` because it is what the harness
/// itself renders, and a primary agent typically has neither.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenClawAgentRow {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    identity_name: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    bindings: Option<u32>,
    #[serde(default)]
    is_default: Option<bool>,
}

/// Parse OpenClaw's durable roster.
///
/// Primary selection has a fallback: if no row is flagged `isDefault`, the row
/// with id `main` is treated as primary. The specification says "the harness
/// primary *or* `main`", and a roster with no preselection would push the choice
/// onto a user who has no basis for making it.
fn parse_openclaw_roster(payload: &str) -> Result<Vec<RemoteAgentCandidate>, String> {
    if payload.is_empty() {
        return Err("harness returned no roster output".to_string());
    }
    let rows: Vec<OpenClawAgentRow> = serde_json::from_str(payload)
        .map_err(|error| format!("could not parse the harness agent list: {error}"))?;

    let mut candidates: Vec<RemoteAgentCandidate> = Vec::with_capacity(rows.len());
    for row in rows {
        let agent_id = row.id.trim().to_string();
        // A row with no id cannot be routed to, so it is not a candidate. Being
        // silent about it is correct: it is malformed harness output, not a
        // user-actionable condition.
        if agent_id.is_empty() {
            continue;
        }
        if candidates
            .iter()
            .any(|existing| existing.agent_id == agent_id)
        {
            continue;
        }
        let display_name = row
            .identity_name
            .as_deref()
            .or(row.name.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&agent_id)
            .to_string();
        candidates.push(RemoteAgentCandidate {
            harness_id: "openclaw".to_string(),
            agent_id,
            display_name,
            is_primary: row.is_default.unwrap_or(false),
            model: row.model.filter(|value| !value.trim().is_empty()),
            workspace: row.workspace.filter(|value| !value.trim().is_empty()),
            binding_count: row.bindings,
        });
    }

    if candidates.is_empty() {
        return Err("the harness reported no configured agents".to_string());
    }

    if !candidates.iter().any(|candidate| candidate.is_primary) {
        if let Some(main) = candidates
            .iter_mut()
            .find(|candidate| candidate.agent_id == "main")
        {
            main.is_primary = true;
        }
    }

    // Primary first, then alphabetical. Stable order matters for a list the user
    // reads twice: once to choose, once to confirm.
    candidates.sort_by(|a, b| {
        b.is_primary.cmp(&a.is_primary).then_with(|| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
        })
    });
    Ok(candidates)
}

/// Enumerate durable agents for `harness_id` on an ssh host.
///
/// Never returns `Err` for a host-side problem, matching the host probe: an
/// unreachable host is a reportable outcome the dialog renders, not an
/// exception.
pub fn probe_ssh_harness_agents(host: &SshHost, harness_id: &str) -> HarnessRosterResult {
    let started = Instant::now();
    let Some(recipe) = recipe_for(harness_id) else {
        return unsupported(&host.host, harness_id, started);
    };

    let mut command = Command::new(resolve_ssh_binary());
    // The same argument list the host probe uses, so trust behaviour cannot
    // drift between the two: `BatchMode=yes` and `StrictHostKeyChecking=yes`
    // mean this cannot prompt for a password or write a host key.
    command
        .args(ssh_probe_args(host))
        .arg(build_roster_command(recipe));

    run_roster(command, &host.host, recipe, started)
}

/// Enumerate durable agents for `harness_id` on this machine, using the
/// identical command so the result shape cannot diverge.
pub fn probe_local_harness_agents(harness_id: &str) -> HarnessRosterResult {
    let started = Instant::now();
    let Some(recipe) = recipe_for(harness_id) else {
        return unsupported(super::LOCALHOST_ID, harness_id, started);
    };

    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(build_roster_command(recipe));

    run_roster(command, super::LOCALHOST_ID, recipe, started)
}

fn unsupported(host: &str, harness_id: &str, started: Instant) -> HarnessRosterResult {
    HarnessRosterResult {
        host: host.to_string(),
        harness_id: harness_id.to_string(),
        ok: false,
        supported: false,
        duration_ms: started.elapsed().as_millis() as u64,
        error: Some(format!(
            "Buzz cannot list the agents of a '{harness_id}' harness yet. Enter the agent's \
             identity manually instead."
        )),
        error_kind: None,
        candidates: Vec::new(),
    }
}

fn run_roster(
    mut command: Command,
    host: &str,
    recipe: &RosterRecipe,
    started: Instant,
) -> HarnessRosterResult {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let base = |ok: bool| HarnessRosterResult {
        host: host.to_string(),
        harness_id: recipe.harness_id.to_string(),
        ok,
        supported: true,
        duration_ms: started.elapsed().as_millis() as u64,
        error: None,
        error_kind: None,
        candidates: Vec::new(),
    };

    let output = match wait_with_timeout(command, ROSTER_TIMEOUT) {
        Ok(Some(output)) => output,
        Ok(None) => {
            let kind = HostProbeErrorKind::TimedOut;
            return HarnessRosterResult {
                error: Some(failure_message(&kind, host, "")),
                error_kind: Some(kind),
                ..base(false)
            };
        }
        Err(error) => {
            return HarnessRosterResult {
                error: Some(format!("could not list agents on '{host}': {error}")),
                ..base(false)
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let Some(payload) = extract_payload(&stdout) else {
        // No opening marker at all means ssh itself failed; a missing closing
        // marker means the session died partway. Classify the first case, since
        // that is the one with an actionable remedy.
        let kind = if stdout.contains(ROSTER_START) {
            HostProbeErrorKind::Truncated
        } else {
            classify_ssh_failure(&stderr).unwrap_or(HostProbeErrorKind::Truncated)
        };
        return HarnessRosterResult {
            error: Some(failure_message(&kind, host, &stderr)),
            error_kind: Some(kind),
            ..base(false)
        };
    };

    match (recipe.parse)(payload) {
        Ok(candidates) => HarnessRosterResult {
            candidates,
            ..base(true)
        },
        Err(error) => {
            let detail = if stderr.trim().is_empty() {
                error
            } else {
                format!("{error} ({})", first_line(&stderr))
            };
            HarnessRosterResult {
                error: Some(format!(
                    "could not read the agent list on '{host}': {detail}"
                )),
                ..base(false)
            }
        }
    }
}

/// Every recipe is safe to embed in a single-quoted remote command.
///
/// Exposed for the test module: the guarantee that no roster command can escape
/// its quoting is what makes the "no user input reaches the remote shell" claim
/// checkable rather than aspirational.
#[cfg(test)]
pub(crate) fn recipes_are_quote_safe() -> bool {
    ROSTER_RECIPES
        .iter()
        .all(|recipe| !recipe.command.contains('\'') && !recipe.command.contains('\n'))
}

#[cfg(test)]
#[path = "roster_tests.rs"]
mod tests;
