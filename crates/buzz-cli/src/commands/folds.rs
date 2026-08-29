//! `buzz folds` — the accumulator control plane.
//!
//! A fold maintains a small, always-current artifact (digest) over a saved
//! selection of relay events. All state is signed relay events owned by the
//! caller's key: specs are kind-30640 addressable events (`d` = fold name,
//! last-write-wins) and artifact versions are immutable kind-4640 events —
//! both NIP-44 encrypted to self and author-only at the relay.
//!
//! Subcommands:
//! - `buzz folds set <name> --channel <uuid> …` — create or update a fold spec
//! - `buzz folds list`                          — list folds with latest-run state
//! - `buzz folds get <name>`                    — full spec + latest artifact meta
//! - `buzz folds delete <name>`                 — tombstone a spec (artifacts remain)
//! - `buzz folds estimate <name>`               — zero-spend preflight: price the exact run
//! - `buzz folds run <name>`                    — plan, invoke the model, validate, persist
//! - `buzz folds artifact <name>`               — latest artifact (or `--version`/`--history`)
//! - `buzz folds share <name> --channel <uuid>` — republish the artifact as a channel message
//!
//! Honesty rules carried by the engine (`buzz-accumulator`): estimates never
//! call a model; coverage records exactly what the model was shown; output
//! that breaks the artifact contract is refused and nothing persists; prior
//! log history is spliced back by the engine, so it cannot be rewritten.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use buzz_accumulator::{
    complete_run, plan_run, ArtifactPayload, FoldRunner, FoldSpec, Plan, Selection, Signal,
    SubprocessRunner,
};
use buzz_core::kind::{KIND_FOLD_ARTIFACT, KIND_FOLD_SPEC, KIND_PROFILE};
use nostr::nips::nip44;
use nostr::{EventBuilder, Kind, Tag, Timestamp};

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::read_or_stdin;

/// Exhaustive-read bound for a fold's artifact chain. The covered-signal set
/// is the union of `shown_ids` across the chain, so a silently truncated read
/// would re-show already-folded history; `query_all_bounded` errors instead.
const MAX_ARTIFACT_CHAIN: u32 = 2_000;

/// Largest artifact JSON the CLI will publish. NIP-44 v2 caps plaintext at
/// 65,535 bytes; staying under it with headroom keeps "encrypt then publish"
/// a step that cannot fail after the model spend.
const MAX_ARTIFACT_PLAINTEXT: usize = 60_000;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// --- encryption to self ------------------------------------------------------

fn encrypt_to_self(client: &BuzzClient, plaintext: &str) -> Result<String, CliError> {
    let keys = client.keys();
    nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|e| CliError::Other(format!("nip44 encrypt failed: {e}")))
}

fn decrypt_from_self(client: &BuzzClient, ciphertext: &str) -> Result<String, CliError> {
    let keys = client.keys();
    nip44::decrypt(keys.secret_key(), &keys.public_key(), ciphertext)
        .map_err(|e| CliError::Other(format!("nip44 decrypt failed: {e}")))
}

// --- spec envelope ------------------------------------------------------------

/// Decrypted content of a kind-30640 event: a live [`FoldSpec`] or a
/// tombstone. Addressable events cannot be deleted from an append-only store,
/// so `delete` publishes a `{"deleted":true}` replacement that hides the fold.
fn parse_spec_envelope(plaintext: &str) -> Result<Option<FoldSpec>, CliError> {
    let value: serde_json::Value = serde_json::from_str(plaintext)
        .map_err(|e| CliError::Other(format!("fold spec payload is not JSON: {e}")))?;
    if value.get("deleted").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| CliError::Other(format!("fold spec payload is malformed: {e}")))
}

/// Monotonic created_at for LWW replacement: strictly newer than the prior
/// head even when both writes land within one second.
fn monotonic_created_at(now: i64, prior: Option<i64>) -> i64 {
    match prior {
        Some(p) if p >= now => p + 1,
        _ => now,
    }
}

// --- relay reads --------------------------------------------------------------

fn event_field<'a>(ev: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    ev.get(key).and_then(|v| v.as_str())
}

/// Tag value for `key` from a raw event JSON value.
fn event_tag<'a>(ev: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    ev.get("tags")?.as_array()?.iter().find_map(|t| {
        let t = t.as_array()?;
        if t.first()?.as_str()? == key {
            t.get(1)?.as_str()
        } else {
            None
        }
    })
}

/// Fetch the head kind-30640 event for `name` (raw JSON), if any.
async fn fetch_spec_event(
    client: &BuzzClient,
    name: &str,
) -> Result<Option<serde_json::Value>, CliError> {
    let me = client.keys().public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [KIND_FOLD_SPEC],
        "authors": [me],
        "#d": [name],
        "limit": 4,
    });
    let raw = client.query(&filter).await?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay returned invalid JSON: {e}")))?;
    let events = value
        .as_array()
        .ok_or_else(|| CliError::Other("relay response is not an array".into()))?;
    // The relay stores only the NIP-33 head, but pick the newest defensively.
    Ok(events
        .iter()
        .max_by_key(|ev| ev.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0))
        .cloned())
}

/// Load the live spec for `name`. `None` when absent or tombstoned.
async fn load_spec(client: &BuzzClient, name: &str) -> Result<Option<FoldSpec>, CliError> {
    match fetch_spec_event(client, name).await? {
        None => Ok(None),
        Some(ev) => {
            let content = event_field(&ev, "content")
                .ok_or_else(|| CliError::Other("spec event has no content".into()))?;
            parse_spec_envelope(&decrypt_from_self(client, content)?)
        }
    }
}

/// Load every live fold spec owned by this key.
async fn load_all_specs(client: &BuzzClient) -> Result<Vec<FoldSpec>, CliError> {
    let me = client.keys().public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [KIND_FOLD_SPEC],
        "authors": [me],
        "limit": 500,
    });
    let events = client.query_all_bounded(filter, 500).await?;
    let mut specs = Vec::new();
    let mut skipped = 0usize;
    for ev in &events {
        let Some(content) = event_field(ev, "content") else {
            skipped += 1;
            continue;
        };
        // Skip undecryptable or malformed entries rather than failing the
        // whole listing; `get` on the specific name surfaces the error.
        let Ok(plain) = decrypt_from_self(client, content) else {
            skipped += 1;
            continue;
        };
        match parse_spec_envelope(&plain) {
            Ok(Some(spec)) => specs.push(spec),
            Ok(None) => {}
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} unreadable fold-spec event(s)");
    }
    specs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(specs)
}

/// Load the full artifact chain for `fold`, sorted by version ascending.
///
/// Artifact events carry no plaintext tags (the fold name stays private), so
/// this reads the caller's whole kind-4640 set (bounded, erroring on overflow
/// rather than silently truncating the covered-ids union) and filters by the
/// decrypted payload's `fold` field.
async fn load_artifacts(client: &BuzzClient, fold: &str) -> Result<Vec<ArtifactPayload>, CliError> {
    let me = client.keys().public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [KIND_FOLD_ARTIFACT],
        "authors": [me],
        "limit": 500,
    });
    let events = client.query_all_bounded(filter, MAX_ARTIFACT_CHAIN).await?;
    let mut artifacts = Vec::new();
    let mut skipped = 0usize;
    for ev in &events {
        let Some(content) = event_field(ev, "content") else {
            skipped += 1;
            continue;
        };
        let Ok(plain) = decrypt_from_self(client, content) else {
            skipped += 1;
            continue;
        };
        let Ok(payload) = serde_json::from_str::<ArtifactPayload>(&plain) else {
            skipped += 1;
            continue;
        };
        if payload.fold == fold {
            artifacts.push(payload);
        }
    }
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} unreadable artifact event(s)");
    }
    artifacts.sort_by_key(|a| a.version);
    Ok(artifacts)
}

/// Latest artifact + union of shown ids over the whole chain.
fn chain_state(artifacts: &[ArtifactPayload]) -> (Option<&ArtifactPayload>, BTreeSet<String>) {
    let covered = artifacts
        .iter()
        .flat_map(|a| a.shown_ids.iter().cloned())
        .collect();
    (artifacts.last(), covered)
}

/// Fetch the signals a selection matches over `[since, until_exclusive)`.
///
/// One relay query per compiled filter (multi-channel union filters are not
/// reliable), each read exhaustively up to `limit` — a window with more
/// matches errors loudly (narrow `--since`/`--until` or raise `--limit`)
/// instead of silently keeping only the newest page, which would leave older
/// backlog permanently unfolded while reporting the fold as covered. The
/// engine dedupes and orders during materialization.
async fn fetch_signals(
    client: &BuzzClient,
    selection: &Selection,
    since: i64,
    until_exclusive: i64,
    limit: u32,
) -> Result<Vec<Signal>, CliError> {
    let mut signals = Vec::new();
    for filter in selection.compile_filters(since, until_exclusive, limit as usize) {
        let events = client.query_all_bounded(filter, limit).await?;
        for ev in &events {
            let (Some(id), Some(pubkey)) = (event_field(ev, "id"), event_field(ev, "pubkey"))
            else {
                continue;
            };
            signals.push(Signal {
                id: id.to_string(),
                pubkey: pubkey.to_string(),
                kind: ev.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                created_at: ev.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0),
                content: event_field(ev, "content").unwrap_or_default().to_string(),
                channel: event_tag(ev, "h").map(str::to_string),
            });
        }
    }
    Ok(signals)
}

/// Resolve display names for transcript lines from kind-0 profiles.
async fn fetch_names(client: &BuzzClient, pubkeys: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    if pubkeys.is_empty() {
        return names;
    }
    let filter = serde_json::json!({
        "kinds": [KIND_PROFILE],
        "authors": pubkeys.iter().collect::<Vec<_>>(),
        "limit": pubkeys.len().min(500),
    });
    let Ok(raw) = client.query(&filter).await else {
        return names; // Names are cosmetic; a failed lookup falls back to pubkey prefixes.
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return names;
    };
    for ev in value.as_array().map(|a| a.as_slice()).unwrap_or_default() {
        let (Some(pk), Some(content)) = (event_field(ev, "pubkey"), event_field(ev, "content"))
        else {
            continue;
        };
        let Ok(profile) = serde_json::from_str::<serde_json::Value>(content) else {
            continue;
        };
        let name = profile
            .get("display_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| profile.get("name").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty());
        if let Some(name) = name {
            names.insert(pk.to_string(), name.to_string());
        }
    }
    names
}

// --- relay writes ---------------------------------------------------------------

/// Submit a signed event, mapping a dominated NIP-33 write to
/// [`CliError::Conflict`] via the shared write-response parser.
async fn submit_checked(
    client: &BuzzClient,
    event: nostr::Event,
    conflict_msg: &str,
) -> Result<String, CliError> {
    let id = event.id.to_hex();
    let raw = client.submit_event(event).await?;
    super::parse_write_response(&raw, conflict_msg)?;
    Ok(id)
}

/// Publish the spec envelope (live or tombstone) as the new kind-30640 head.
async fn publish_spec_envelope(
    client: &BuzzClient,
    name: &str,
    plaintext: &str,
) -> Result<(String, bool), CliError> {
    let prior = fetch_spec_event(client, name).await?;
    let replaced = prior.is_some();
    let prior_ts = prior.and_then(|ev| ev.get("created_at").and_then(|v| v.as_i64()));
    let created_at = monotonic_created_at(now_secs(), prior_ts);
    let ciphertext = encrypt_to_self(client, plaintext)?;
    let tag = Tag::parse(["d", name])
        .map_err(|e| CliError::Other(format!("failed to build d tag: {e}")))?;
    let builder = EventBuilder::new(Kind::Custom(KIND_FOLD_SPEC as u16), ciphertext)
        .tags([tag])
        .custom_created_at(Timestamp::from_secs(created_at as u64));
    let event = client.sign_event(builder)?;
    let id = submit_checked(
        client,
        event,
        "fold spec changed concurrently; re-read and retry",
    )
    .await?;
    Ok((id, replaced))
}

// --- shared preflight ---------------------------------------------------------

struct Preflight {
    spec: FoldSpec,
    prior: Option<ArtifactPayload>,
    plan: Plan,
    /// The exact half-open window this plan drew from. `estimate` reports it
    /// so a later `run --since … --until …` can replay the priced set.
    since: i64,
    until_exclusive: i64,
}

/// Load spec + artifact chain, fetch pending signals, and plan the run.
/// This is the zero-spend half shared by `estimate` and `run`.
async fn preflight(
    client: &BuzzClient,
    name: &str,
    since: Option<i64>,
    until: Option<i64>,
    limit: u32,
) -> Result<Preflight, CliError> {
    let spec = load_spec(client, name).await?.ok_or_else(|| {
        CliError::NotFound(format!("no fold named {name:?} (see `buzz folds list`)"))
    })?;
    let artifacts = load_artifacts(client, name).await?;
    let (prior, covered) = chain_state(&artifacts);
    let prior = prior.cloned();
    let since = since.unwrap_or(0);
    let until_exclusive = until.unwrap_or_else(|| now_secs() + 1);
    let fetched = fetch_signals(client, &spec.selection, since, until_exclusive, limit).await?;
    let authors: BTreeSet<String> = fetched.iter().map(|s| s.pubkey.clone()).collect();
    let names = fetch_names(client, &authors).await;
    let plan = plan_run(&spec, prior.as_ref(), &covered, fetched, &names)
        .map_err(|e| CliError::Other(e.to_string()))?;
    Ok(Preflight {
        spec,
        prior,
        plan,
        since,
        until_exclusive,
    })
}

fn plan_json(name: &str, pre: &Preflight) -> serde_json::Value {
    match &pre.plan {
        Plan::Cached => serde_json::json!({
            "fold": name,
            "action": "cached",
            "version": pre.prior.as_ref().map(|p| p.version),
            "note": "no new signals and the config is unchanged; the latest artifact already answers",
        }),
        Plan::Stalled { reason, pending } => serde_json::json!({
            "fold": name,
            "action": "stalled",
            "reason": reason,
            "pending_signals": pending,
        }),
        Plan::Ready(run) => serde_json::json!({
            "fold": name,
            "action": "ready",
            "model": pre.spec.model,
            "pending_signals": run.pending,
            "would_show": run.shown.len(),
            "truncated_to_budget": run.truncated,
            "est_input_tokens": run.estimate.est_input_tokens,
            "est_cost_usd": run.estimate.est_cost_usd,
            "window_fit": run.estimate.window_fit,
            "next_version": pre.prior.as_ref().map_or(1, |p| p.version + 1),
            // Replay window: `run --since <since> --until <until_exclusive>`
            // executes the same fetch this estimate priced.
            "since": pre.since,
            "until_exclusive": pre.until_exclusive,
        }),
    }
}

// --- subcommands ----------------------------------------------------------------

async fn cmd_set(
    client: &BuzzClient,
    name: String,
    channels: Vec<String>,
    authors: Vec<String>,
    kinds: Vec<u32>,
    model: String,
    instructions: Option<String>,
) -> Result<(), CliError> {
    let instructions = match instructions {
        Some(value) => read_or_stdin(&value)?,
        None => buzz_accumulator::schema::CHANNEL_DIGEST_PROMPT.to_string(),
    };
    let mut spec = FoldSpec {
        name: name.clone(),
        selection: Selection {
            channels,
            authors,
            kinds,
        },
        schema: buzz_accumulator::schema::CHANNEL_DIGEST_V1.name.to_string(),
        model,
        instructions,
    };
    spec.validate()
        .map_err(|e| CliError::Usage(e.to_string()))?;
    let plaintext = serde_json::to_string(&spec)
        .map_err(|e| CliError::Other(format!("spec serialization failed: {e}")))?;
    let (event_id, replaced) = publish_spec_envelope(client, &name, &plaintext).await?;
    println!(
        "{}",
        serde_json::json!({
            "fold": name,
            "event_id": event_id,
            "accepted": true,
            "replaced": replaced,
            "selection": spec.selection.describe(),
            "schema": spec.schema,
            "model": spec.model,
        })
    );
    Ok(())
}

async fn cmd_list(client: &BuzzClient) -> Result<(), CliError> {
    let specs = load_all_specs(client).await?;
    let mut out = Vec::new();
    for spec in &specs {
        let artifacts = load_artifacts(client, &spec.name).await?;
        let (latest, covered) = chain_state(&artifacts);
        out.push(serde_json::json!({
            "fold": spec.name,
            "selection": spec.selection.describe(),
            "schema": spec.schema,
            "model": spec.model,
            "latest_version": latest.map(|a| a.version),
            "covered_signals": covered.len(),
            "coverage_until": latest.and_then(|a| a.coverage_until),
        }));
    }
    println!("{}", serde_json::Value::Array(out));
    Ok(())
}

async fn cmd_get(client: &BuzzClient, name: String) -> Result<(), CliError> {
    let spec = load_spec(client, &name)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("no fold named {name:?}")))?;
    let artifacts = load_artifacts(client, &name).await?;
    let (latest, covered) = chain_state(&artifacts);
    println!(
        "{}",
        serde_json::json!({
            "fold": spec.name,
            "selection": spec.selection,
            "schema": spec.schema,
            "model": spec.model,
            "instructions": spec.instructions,
            "versions": artifacts.len(),
            "covered_signals": covered.len(),
            "latest": latest.map(|a| serde_json::json!({
                "version": a.version,
                "created_at": a.created_at,
                "coverage_since": a.coverage_since,
                "coverage_until": a.coverage_until,
                "shown_signals": a.shown_ids.len(),
                "truncated": a.truncated,
            })),
        })
    );
    Ok(())
}

async fn cmd_delete(client: &BuzzClient, name: String) -> Result<(), CliError> {
    if load_spec(client, &name).await?.is_none() {
        return Err(CliError::NotFound(format!("no fold named {name:?}")));
    }
    let (event_id, _) = publish_spec_envelope(client, &name, "{\"deleted\":true}").await?;
    println!(
        "{}",
        serde_json::json!({
            "fold": name,
            "event_id": event_id,
            "accepted": true,
            "deleted": true,
            "note": "spec tombstoned; existing artifact versions remain (append-only)",
        })
    );
    Ok(())
}

async fn cmd_estimate(
    client: &BuzzClient,
    name: String,
    since: Option<i64>,
    until: Option<i64>,
    limit: u32,
) -> Result<(), CliError> {
    let pre = preflight(client, &name, since, until, limit).await?;
    println!("{}", plan_json(&name, &pre));
    Ok(())
}

async fn cmd_run(
    client: &BuzzClient,
    name: String,
    since: Option<i64>,
    until: Option<i64>,
    limit: u32,
) -> Result<(), CliError> {
    let pre = preflight(client, &name, since, until, limit).await?;
    let Plan::Ready(run) = &pre.plan else {
        // Cached/stalled runs spend nothing and change nothing — report as data.
        println!("{}", plan_json(&name, &pre));
        return Ok(());
    };
    eprintln!(
        "running fold {name:?}: {} signal(s), ~{} input tokens{} on {}",
        run.shown.len(),
        run.estimate.est_input_tokens,
        run.estimate
            .est_cost_usd
            .map(|c| format!(" (~${c:.4})"))
            .unwrap_or_default(),
        pre.spec.model,
    );
    let runner = SubprocessRunner::new();
    let output = runner
        .run(&run.model_input, &pre.spec.model)
        .map_err(|e| CliError::Other(e.to_string()))?;
    // The model spend is behind us: from here on, any failure salvages the
    // paid-for output to stdout instead of discarding it with the error.
    let salvage = |action: &str, reason: &str| {
        println!(
            "{}",
            serde_json::json!({
                "fold": name,
                "action": action,
                "reason": reason,
                "accepted": false,
                "model_output": output,
            })
        );
    };
    // Refusal path: nonconforming output persists nothing.
    let artifact = match complete_run(&pre.spec, pre.prior.as_ref(), run, &output, now_secs()) {
        Ok(artifact) => artifact,
        Err(e) => {
            salvage("refused", &e.to_string());
            return Err(CliError::Other(e.to_string()));
        }
    };
    let plaintext = match serde_json::to_string(&artifact) {
        Ok(p) => p,
        Err(e) => {
            let reason = format!("artifact serialization failed: {e}");
            salvage("unpublished", &reason);
            return Err(CliError::Other(reason));
        }
    };
    if plaintext.len() > MAX_ARTIFACT_PLAINTEXT {
        let reason = format!(
            "artifact JSON is {} bytes, over the {MAX_ARTIFACT_PLAINTEXT}-byte encrypted-event \
             ceiling; compact the fold before running again",
            plaintext.len()
        );
        salvage("unpublished", &reason);
        return Err(CliError::Other(reason));
    }
    // Version fence: if a concurrent run published while the model was
    // thinking, abort instead of forking the chain at the same version.
    let head = match load_artifacts(client, &name).await {
        Ok(chain) => chain.last().map(|a| a.version),
        Err(e) => {
            salvage(
                "unpublished",
                &format!("pre-publish chain re-read failed: {e}"),
            );
            return Err(e);
        }
    };
    if head.is_some_and(|v| v >= artifact.version) {
        let reason = format!(
            "a concurrent run already published v{} for this fold; nothing persisted — \
             re-run to fold what is still pending",
            head.unwrap_or_default()
        );
        salvage("unpublished", &reason);
        return Err(CliError::Other(reason));
    }
    // No plaintext tags: the fold name lives only inside the encrypted payload.
    let publish = async {
        let ciphertext = encrypt_to_self(client, &plaintext)?;
        let builder = EventBuilder::new(Kind::Custom(KIND_FOLD_ARTIFACT as u16), ciphertext);
        let event = client.sign_event(builder)?;
        submit_checked(client, event, "artifact write was reported duplicate").await
    };
    let event_id = match publish.await {
        Ok(id) => id,
        Err(e) => {
            salvage("unpublished", &format!("artifact publish failed: {e}"));
            return Err(e);
        }
    };
    println!(
        "{}",
        serde_json::json!({
            "fold": name,
            "action": "run",
            "event_id": event_id,
            "accepted": true,
            "version": artifact.version,
            "shown_signals": artifact.shown_ids.len(),
            "coverage_since": artifact.coverage_since,
            "coverage_until": artifact.coverage_until,
            "truncated_to_budget": artifact.truncated,
            "est_input_tokens": run.estimate.est_input_tokens,
            "est_cost_usd": run.estimate.est_cost_usd,
        })
    );
    Ok(())
}

async fn cmd_artifact(
    client: &BuzzClient,
    name: String,
    version: Option<u32>,
    history: bool,
    raw: bool,
) -> Result<(), CliError> {
    let artifacts = load_artifacts(client, &name).await?;
    if history {
        let out: Vec<serde_json::Value> = artifacts
            .iter()
            .map(|a| {
                serde_json::json!({
                    "version": a.version,
                    "created_at": a.created_at,
                    "model": a.model,
                    "shown_signals": a.shown_ids.len(),
                    "coverage_since": a.coverage_since,
                    "coverage_until": a.coverage_until,
                    "truncated": a.truncated,
                })
            })
            .collect();
        println!("{}", serde_json::Value::Array(out));
        return Ok(());
    }
    let artifact = match version {
        Some(v) => artifacts.iter().find(|a| a.version == v),
        None => artifacts.last(),
    }
    .ok_or_else(|| {
        CliError::NotFound(match version {
            Some(v) => format!("fold {name:?} has no version {v}"),
            None => format!("fold {name:?} has no artifact yet (run `buzz folds run {name}`)"),
        })
    })?;
    if raw {
        println!("{}", artifact.output);
    } else {
        println!(
            "{}",
            serde_json::to_value(artifact)
                .map_err(|e| CliError::Other(format!("artifact serialization failed: {e}")))?
        );
    }
    Ok(())
}

/// The share taint rule: an artifact may be republished into a channel only
/// when every channel that ever fed its chain (`artifact.channels`, the
/// union over all versions) is exactly that one channel — then every signal
/// that ever folded in is already visible to the audience by construction.
/// The check reads the artifact's provenance, not the live spec: a selection
/// edit cannot launder history folded from elsewhere. Author/kind narrowing
/// within the channel is fine; a second channel, a channel-less selection,
/// or a pre-provenance artifact is not.
fn share_allowed(artifact: &ArtifactPayload, channel: &str) -> bool {
    artifact.channels.len() == 1 && artifact.channels[0] == channel
}

async fn cmd_share(client: &BuzzClient, name: String, channel: String) -> Result<(), CliError> {
    // Provenance channels are canonicalized lowercase; match that.
    let channel = channel.trim().to_ascii_lowercase();
    if load_spec(client, &name).await?.is_none() {
        return Err(CliError::NotFound(format!("no fold named {name:?}")));
    }
    let artifacts = load_artifacts(client, &name).await?;
    let artifact = artifacts.last().ok_or_else(|| {
        CliError::NotFound(format!(
            "fold {name:?} has no artifact yet (run `buzz folds run {name}`)"
        ))
    })?;
    if !share_allowed(artifact, &channel) {
        return Err(CliError::Usage(format!(
            "refusing to share: this artifact's history folded events from channel(s) {:?}, \
             not exactly channel {channel} — sharing is only allowed into the single channel \
             an artifact has ever read from, so everything in it is already visible to the \
             audience",
            artifact.channels,
        )));
    }
    let channel_uuid = channel
        .parse::<uuid::Uuid>()
        .map_err(|e| CliError::Usage(format!("--channel must be a channel UUID: {e}")))?;
    let content = format!(
        "{}\n\n—\n_{} v{} · digest of this channel · {} signal(s) this run_",
        artifact.output,
        artifact.fold,
        artifact.version,
        artifact.shown_ids.len(),
    );
    let builder = buzz_sdk::build_message(channel_uuid, &content, None, &[], false, &[])
        .map_err(|e| CliError::Other(format!("build_message failed: {e}")))?;
    let event = client.sign_event(builder)?;
    let event_id = submit_checked(client, event, "share message was reported duplicate").await?;
    println!(
        "{}",
        serde_json::json!({
            "fold": name,
            "event_id": event_id,
            "accepted": true,
            "shared_to_channel": channel,
            "version": artifact.version,
        })
    );
    Ok(())
}

pub async fn dispatch(cmd: crate::FoldsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::FoldsCmd;
    match cmd {
        FoldsCmd::Set {
            name,
            channel,
            author,
            kind,
            model,
            instructions,
        } => cmd_set(client, name, channel, author, kind, model, instructions).await,
        FoldsCmd::List => cmd_list(client).await,
        FoldsCmd::Get { name } => cmd_get(client, name).await,
        FoldsCmd::Delete { name } => cmd_delete(client, name).await,
        FoldsCmd::Estimate {
            name,
            since,
            until,
            limit,
        } => cmd_estimate(client, name, since, until, limit).await,
        FoldsCmd::Run {
            name,
            since,
            until,
            limit,
        } => cmd_run(client, name, since, until, limit).await,
        FoldsCmd::Artifact {
            name,
            version,
            history,
            raw,
        } => cmd_artifact(client, name, version, history, raw).await,
        FoldsCmd::Share { name, channel } => cmd_share(client, name, channel).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(channels: &[&str], authors: &[&str]) -> Selection {
        Selection {
            channels: channels.iter().map(|s| s.to_string()).collect(),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            kinds: vec![],
        }
    }

    fn artifact_with_channels(channels: &[&str]) -> ArtifactPayload {
        ArtifactPayload {
            fold: "x".into(),
            version: 1,
            output: "# Working Context\n\nS.\n\n# Log\n".into(),
            shown_ids: vec![],
            coverage_since: None,
            coverage_until: None,
            selection: selection(channels, &[]),
            channels: channels.iter().map(|s| s.to_string()).collect(),
            model: "haiku".into(),
            schema: "channel-digest@v1".into(),
            prompt_sha256: "0".repeat(64),
            truncated: false,
            created_at: 0,
        }
    }

    #[test]
    fn share_requires_exactly_the_target_channel_in_provenance() {
        assert!(share_allowed(&artifact_with_channels(&["ch1"]), "ch1"));
        assert!(!share_allowed(&artifact_with_channels(&["ch1"]), "ch2"));
        // A chain that EVER read a second channel stays untouchable, no
        // matter what the live spec reads now.
        assert!(!share_allowed(
            &artifact_with_channels(&["ch1", "ch2"]),
            "ch1"
        ));
        // Pre-provenance or author-only artifacts have no single-channel
        // proof: blocked.
        assert!(!share_allowed(&artifact_with_channels(&[]), "ch1"));
    }

    #[test]
    fn spec_envelope_roundtrip_and_tombstone() {
        let spec = FoldSpec {
            name: "x".into(),
            selection: selection(&["ch1"], &[]),
            schema: "channel-digest@v1".into(),
            model: "haiku".into(),
            instructions: "do it".into(),
        };
        let json = serde_json::to_string(&spec).expect("serialize");
        assert_eq!(parse_spec_envelope(&json).expect("parse"), Some(spec));
        assert_eq!(
            parse_spec_envelope("{\"deleted\":true}").expect("parse"),
            None
        );
        assert!(parse_spec_envelope("not json").is_err());
    }

    #[test]
    fn lww_created_at_is_strictly_monotonic() {
        assert_eq!(monotonic_created_at(100, None), 100);
        assert_eq!(monotonic_created_at(100, Some(50)), 100);
        assert_eq!(monotonic_created_at(100, Some(100)), 101);
        assert_eq!(monotonic_created_at(100, Some(140)), 141);
    }

    #[test]
    fn event_tag_reads_first_match() {
        let ev = serde_json::json!({
            "tags": [["f", "my-fold"], ["v", "3"], ["h", "chan"]],
        });
        assert_eq!(event_tag(&ev, "f"), Some("my-fold"));
        assert_eq!(event_tag(&ev, "v"), Some("3"));
        assert_eq!(event_tag(&ev, "x"), None);
    }
}
