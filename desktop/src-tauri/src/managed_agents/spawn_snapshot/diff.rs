//! Redacted field-by-field diff of two [`SpawnConfigSnapshot`]s.
//!
//! The walk is generic over the snapshot's canonical JSON: it compares leaves
//! by path and emits one entry per inequality. Adding a field to
//! [`SpawnConfigSnapshot`] therefore reaches the UI with no change here — the
//! only per-path knowledge in this module is [`policy_for`], which decides how
//! a leaf may be *shown*, never which leaves are compared.
//!
//! Raw values drive comparison; redaction happens strictly afterwards, when
//! the serializable entry is built. Comparing masked forms would let two
//! secrets with colliding suffixes read as "no drift".

use serde::Serialize;
use serde_json::{Map, Value};

use super::SpawnConfigSnapshot;
use crate::managed_agents::AcpAvailabilityStatus;

/// Synthetic field id for adapter-availability drift, which lives outside the
/// snapshot: it describes the environment around the process, not the config
/// the process was spawned with.
const ADAPTER_AVAILABILITY_FIELD: &str = "adapter_availability";

const MASK: &str = "••••";

/// One changed field. `field` is a dotted path built from serde field names,
/// with dynamic map keys appended verbatim (`env.OPENAI_API_KEY`). The UI
/// humanizes it generically and must never switch on its value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RestartDiffEntry {
    pub field: String,
    pub change: RestartChange,
}

/// How a changed field is presented. The UI switches on `kind` — a closed set
/// — and renders any `field` path.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RestartChange {
    /// Safe scalar or array shown verbatim. `null` means absent.
    Value { before: Value, after: Value },
    /// Large text shown as character counts only. `null` means absent.
    Text {
        before_chars: Option<usize>,
        after_chars: Option<usize>,
    },
    /// Secret-bearing leaf. `null` means absent.
    Masked {
        before: Option<String>,
        after: Option<String>,
    },
    /// Dynamic-map key present only on the new side. No payload — the value
    /// would be secret-bearing and the key name alone is the useful signal.
    Added,
    /// Dynamic-map key present only on the old side.
    Removed,
}

/// How a leaf at `path` may be displayed.
#[derive(Clone, Copy, PartialEq)]
enum MaskPolicy {
    /// Shown verbatim.
    Plain,
    /// Character counts only.
    Text,
    /// `••••` plus the last four characters when longer than eight.
    MaskedSuffix,
    /// `••••` and nothing else.
    MaskedBare,
}

/// The single redaction authority: the wire diff and the snapshot's `Debug`
/// both route every leaf through this.
///
/// A new snapshot field needs an arm here only if it can carry a credential or
/// is too large to render; everything else falls through to `Plain`.
fn policy_for(path: &str) -> MaskPolicy {
    match path {
        // Arbitrary user text — a rendered before/after would be unbounded as
        // well as unreadable.
        "system_prompt" | "team_instructions" => MaskPolicy::Text,
        // Arbitrary CLI arguments: `--token=...` is legal, so no part of the
        // value may be disclosed. Same for the relay URL — `normalize_relay_url`
        // rejects userinfo but deliberately preserves query strings, so
        // `wss://relay.example/ws?token=...` is a valid value.
        "args" | "relay_url" => MaskPolicy::MaskedBare,
        // NIP-OA auth tag: a credential, but a suffix tells the user which tag
        // they are looking at.
        "auth_tag" => MaskPolicy::MaskedSuffix,
        // Env values: consult the shared allowlist. Allowlisted keys (e.g.
        // `BUZZ_AGENT_THINKING_EFFORT`) render plain so the user sees the
        // actual enum values; every other env key stays masked.
        _ if path.starts_with("env.") => {
            let key = &path[4..];
            if crate::managed_agents::is_safe_to_reveal(key) {
                MaskPolicy::Plain
            } else {
                MaskPolicy::MaskedSuffix
            }
        }
        // Plain arm. Every path reaching it is already rendered verbatim in
        // the runtime UI today:
        //   acp_command / command / mcp_command — resolved binary names
        //   session_title                       — display chrome
        //   model / provider                    — catalog ids
        //   respond_to / respond_to_allowlist   — gate mode + pubkeys
        //   idle_timeout_seconds / max_turn_duration_seconds / parallelism
        //                                       — numeric limits
        //   adapter_availability                — an enum variant name
        _ => MaskPolicy::Plain,
    }
}

/// `••••` plus the last four characters, or a bare `••••` when the value is
/// short enough that a suffix would disclose too much of it.
///
/// Character-based throughout: byte slicing can panic on a multi-byte value or
/// disclose the wrong suffix.
fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    match chars.len() {
        len if len > 8 => format!("{MASK}{}", chars[len - 4..].iter().collect::<String>()),
        _ => MASK.to_string(),
    }
}

/// Character count of a text leaf; `None` when the leaf is absent.
fn char_count(value: &Value) -> Option<usize> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.chars().count()),
        // Fail closed on an unexpected shape: count it, never show it.
        other => Some(other.to_string().chars().count()),
    }
}

/// Masked rendering of a leaf; `None` when the leaf is absent.
fn masked(policy: MaskPolicy, value: &Value) -> Option<String> {
    match (policy, value) {
        (_, Value::Null) => None,
        (MaskPolicy::MaskedSuffix, Value::String(text)) => Some(mask(text)),
        // Fail closed: an unexpected shape under a redacting policy still
        // redacts rather than disclosing the raw value.
        _ => Some(MASK.to_string()),
    }
}

fn change_for(policy: MaskPolicy, before: &Value, after: &Value) -> RestartChange {
    match policy {
        MaskPolicy::Plain => RestartChange::Value {
            before: before.clone(),
            after: after.clone(),
        },
        MaskPolicy::Text => RestartChange::Text {
            before_chars: char_count(before),
            after_chars: char_count(after),
        },
        MaskPolicy::MaskedSuffix | MaskPolicy::MaskedBare => RestartChange::Masked {
            before: masked(policy, before),
            after: masked(policy, after),
        },
    }
}

/// Lexicographically sorted union of both maps' keys, so entry order — and
/// therefore the UI's "first N plus and-N-more" truncation — is stable.
fn key_union<'a>(before: &'a Map<String, Value>, after: &'a Map<String, Value>) -> Vec<&'a str> {
    let mut keys: Vec<&str> = before
        .keys()
        .chain(after.keys())
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn child_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn walk(
    path: &str,
    before: Option<&Value>,
    after: Option<&Value>,
    out: &mut Vec<RestartDiffEntry>,
) {
    match (before, after) {
        (before, after) if before == after => {}
        // Present on one side only. Struct fields are always present (`None`
        // serializes as `null`), so this is dynamic-map membership.
        (None, Some(_)) => out.push(RestartDiffEntry {
            field: path.to_string(),
            change: RestartChange::Added,
        }),
        (Some(_), None) => out.push(RestartDiffEntry {
            field: path.to_string(),
            change: RestartChange::Removed,
        }),
        (Some(Value::Object(before)), Some(Value::Object(after))) => {
            for key in key_union(before, after) {
                walk(&child_path(path, key), before.get(key), after.get(key), out);
            }
        }
        // Everything else is a leaf: scalars, and arrays (atomic — `args`
        // changed as a whole, never `args.0`).
        (before, after) => out.push(RestartDiffEntry {
            field: path.to_string(),
            change: change_for(
                policy_for(path),
                before.unwrap_or(&Value::Null),
                after.unwrap_or(&Value::Null),
            ),
        }),
    }
}

/// The redacted diff of two snapshots, in stable path order.
fn diff(before: &SpawnConfigSnapshot, after: &SpawnConfigSnapshot) -> Vec<RestartDiffEntry> {
    let mut entries = Vec::new();
    walk(
        "",
        Some(&before.canonical()),
        Some(&after.canonical()),
        &mut entries,
    );
    entries
}

fn availability_value(status: Option<&AcpAvailabilityStatus>) -> Value {
    status
        .and_then(|status| serde_json::to_value(status).ok())
        .unwrap_or(Value::Null)
}

/// What a tracked runtime was launched with, paired with what a launch would
/// use now. Absent (`None` at the call site) for every agent this workspace
/// tracks no live pair for — stopped, or `runtime_pid`-adopted across an app
/// restart, whose spawn config was never stamped and so can never be shown to
/// have drifted.
pub(crate) struct TrackedSpawnState<'a> {
    pub stamped: &'a SpawnConfigSnapshot,
    pub current: &'a SpawnConfigSnapshot,
    pub stamped_availability: Option<&'a AcpAvailabilityStatus>,
    pub current_availability: Option<AcpAvailabilityStatus>,
}

/// The final restart-diff for one agent — the single source of both the wire
/// field and the badge, which is `!result.is_empty()`.
///
/// Empty for an un-stamped agent (see [`TrackedSpawnState`]) and for an
/// orphaned instance: `spawn_agent_child` refuses to spawn an orphan before
/// any side effect, so "Restart required" would offer an action guaranteed to
/// fail. The UI surfaces `persona_orphaned` instead.
pub(crate) fn eligible_restart_diff(
    persona_orphaned: bool,
    tracked: Option<TrackedSpawnState<'_>>,
) -> Vec<RestartDiffEntry> {
    let Some(tracked) = tracked.filter(|_| !persona_orphaned) else {
        return Vec::new();
    };
    let mut entries = diff(tracked.stamped, tracked.current);
    if crate::managed_agents::availability_drift(
        tracked.stamped_availability,
        tracked.current_availability.clone(),
    ) {
        entries.push(RestartDiffEntry {
            field: ADAPTER_AVAILABILITY_FIELD.to_string(),
            change: RestartChange::Value {
                before: availability_value(tracked.stamped_availability),
                after: availability_value(tracked.current_availability.as_ref()),
            },
        });
    }
    entries
}

/// The canonical snapshot with every leaf passed through [`policy_for`],
/// rendered as JSON text. Backs `SpawnConfigSnapshot`'s manual `Debug` so a
/// log line can never disclose what the wire diff redacts.
pub(crate) fn redacted_canonical(value: &Value) -> String {
    fn redact(path: &str, value: &Value) -> Value {
        match value {
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, child)| (key.clone(), redact(&child_path(path, key), child)))
                    .collect(),
            ),
            leaf => match policy_for(path) {
                MaskPolicy::Plain => leaf.clone(),
                MaskPolicy::Text => char_count(leaf).map_or(Value::Null, |count| {
                    Value::String(format!("<{count} chars>"))
                }),
                policy => masked(policy, leaf).map_or(Value::Null, Value::String),
            },
        }
    }
    redact("", value).to_string()
}

#[cfg(test)]
mod tests;
