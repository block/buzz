//! `buzz trust` — local work trails and NIP-FP work fingerprints
//! (`docs/nips/NIP-FP.md`).
//!
//! `trail` runs the SDK's sans-IO `buzz.work/v1` reader against the current
//! relay: request → contribution(s) → outcome(s); `recheck` verifies a saved
//! trail from its evidence section without any relay. `request`, `contribute`
//! and `outcome` write the typed messages it reads, deriving every reference
//! from fetched, signature-checked events.
//!
//! `fingerprint` signs a selection and prints it as a transport bundle
//! (pre-registration), or publishes it where kind:30340 is registered.
//! `verify` runs the SDK's sans-IO verifier against each origin's `/query`
//! and NIP-11 document, so every signature it reports was checked on bytes
//! fetched by this viewer.

use std::collections::BTreeMap;

use buzz_sdk::trust::{
    self, Entry, PartSpec, Policy, Request, Response, Scope, Selected, Trail, Verifier, WorkRole,
};
use nostr::{Event, EventId, PublicKey, Timestamp};

use crate::client::{normalize_relay_url, BuzzClient};
use crate::error::CliError;
use crate::TrustCmd;

/// Upper bound on discovery rounds. The verifier needs four; this guards
/// against a misbehaving state machine, not a relay.
const MAX_ROUNDS: usize = 8;

pub async fn dispatch(cmd: TrustCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        TrustCmd::Fingerprint { d, select, publish } => {
            cmd_fingerprint(client, &d, &select, publish).await
        }
        TrustCmd::Verify {
            bundle,
            cutoff,
            viewer_known,
        } => cmd_verify(client, &bundle, cutoff, &viewer_known).await,
        TrustCmd::Trail {
            request,
            contribution,
            actor,
            channel,
            repo,
            since,
            cutoff,
            text,
        } => {
            let entry = parse_entry(
                request.as_deref(),
                contribution.as_deref(),
                actor.as_deref(),
            )?;
            let scope = parse_scope(client.relay_url(), &channel, &repo, since, cutoff)?;
            cmd_trail(client, entry, scope, text).await
        }
        TrustCmd::Recheck { file } => cmd_recheck(&file),
        TrustCmd::Request {
            channel,
            title,
            task_type,
            criteria,
        } => {
            let channel = uuid::Uuid::parse_str(&channel)
                .map_err(|_| CliError::Usage(format!("--channel {channel}: not a UUID")))?;
            let builder = trust::build_work_request(channel, &title, &task_type, &criteria)
                .map_err(|e| CliError::Usage(e.to_string()))?;
            submit(client, client.sign_event(builder)?, "request").await
        }
        TrustCmd::Contribute {
            request,
            role,
            summary,
            state,
            artifact,
            artifact_revision,
            previous,
        } => {
            let role = parse_role(&role)?;
            let spec = PartSpec {
                summary,
                state,
                artifact,
                artifact_revision,
            };
            check_part_flags(role, &spec, previous.is_some())?;
            let request = fetch_verified(client, &request).await?;
            let previous = match previous {
                Some(id) => Some(fetch_verified(client, &id).await?),
                None => None,
            };
            let builder = trust::build_work_part(&request, role, &spec, previous.as_ref())
                .map_err(|e| CliError::Usage(e.to_string()))?;
            submit(client, client.sign_event(builder)?, "contribution").await
        }
        TrustCmd::Outcome {
            submission,
            claim,
            reason,
            criteria,
            evidence,
        } => {
            let target = fetch_verified(client, &submission).await?;
            let builder = trust::build_work_label(
                &target,
                client.relay_url(),
                &claim,
                &reason,
                &criteria,
                &evidence,
            )
            .map_err(|e| CliError::Usage(e.to_string()))?;
            submit(client, client.sign_event(builder)?, "outcome").await
        }
    }
}

fn parse_id(flag: &str, raw: &str) -> Result<EventId, CliError> {
    EventId::from_hex(raw.trim())
        .map_err(|e| CliError::Usage(format!("{flag} {raw}: not an event id: {e}")))
}

fn parse_entry(
    request: Option<&str>,
    contribution: Option<&str>,
    actor: Option<&str>,
) -> Result<Entry, CliError> {
    match (request, contribution, actor) {
        (Some(id), None, None) => Ok(Entry::Request(parse_id("--request", id)?)),
        (None, Some(id), None) => Ok(Entry::Contribution(parse_id("--contribution", id)?)),
        (None, None, Some(key)) => PublicKey::parse(key)
            .map(Entry::Actor)
            .map_err(|e| CliError::Usage(format!("--actor {key}: {e}"))),
        _ => Err(CliError::Usage(
            "give exactly one of --request, --contribution, --actor".into(),
        )),
    }
}

fn parse_scope(
    origin: &str,
    channels: &[String],
    repos: &[String],
    since: Option<u64>,
    cutoff: Option<u64>,
) -> Result<Scope, CliError> {
    for channel in channels {
        uuid::Uuid::parse_str(channel)
            .map_err(|_| CliError::Usage(format!("--channel {channel}: not a UUID")))?;
    }
    for repo in repos {
        let parts: Vec<&str> = repo.splitn(3, ':').collect();
        let valid = matches!(parts.as_slice(), ["30617", owner, d]
            if owner.len() == 64
                && owner.chars().all(|c| c.is_ascii_hexdigit())
                && !d.is_empty());
        if !valid {
            return Err(CliError::Usage(format!(
                "--repo {repo}: expected 30617:<owner-hex>:<d>"
            )));
        }
    }
    let until = cutoff.unwrap_or_else(|| Timestamp::now().as_secs());
    if since.is_some_and(|s| s > until) {
        return Err(CliError::Usage("--since is after --cutoff".into()));
    }
    Ok(Scope {
        origin: origin.to_owned(),
        channels: channels.to_vec(),
        repos: repos.to_vec(),
        since,
        until,
    })
}

fn parse_role(role: &str) -> Result<WorkRole, CliError> {
    match role {
        "commitment" => Ok(WorkRole::Commitment),
        "update" => Ok(WorkRole::Update),
        "submission" => Ok(WorkRole::Submission),
        other => Err(CliError::Usage(format!(
            "--role {other}: not a contribution role"
        ))),
    }
}

/// Refuse flags the role does not carry instead of silently dropping them.
fn check_part_flags(role: WorkRole, spec: &PartSpec, previous: bool) -> Result<(), CliError> {
    let misplaced = match role {
        WorkRole::Update => (spec.state.is_none()).then_some("--state is required for an update"),
        _ if spec.state.is_some() => Some("--state applies to an update only"),
        _ => None,
    }
    .or_else(|| {
        let submission_only =
            spec.artifact.is_some() || spec.artifact_revision.is_some() || previous;
        match role {
            WorkRole::Submission if spec.artifact.is_none() || spec.artifact_revision.is_none() => {
                Some("--artifact and --artifact-revision are required for a submission")
            }
            WorkRole::Submission => None,
            _ if submission_only => {
                Some("--artifact, --artifact-revision and --previous apply to a submission only")
            }
            _ => None,
        }
    });
    misplaced.map_or(Ok(()), |m| Err(CliError::Usage(m.into())))
}

async fn cmd_trail(
    client: &BuzzClient,
    entry: Entry,
    scope: Scope,
    text: bool,
) -> Result<(), CliError> {
    let origin = scope.origin.clone();
    let mut trail = Trail::new(entry, scope);
    loop {
        let queries = trail.next_queries();
        if queries.is_empty() {
            break;
        }
        for query in queries {
            // A trail reads one origin: the relay the viewer is signed in to.
            let response = if normalize_relay_url(&query.origin) == origin {
                fetch(client, &query.request).await
            } else {
                Response::Failed(format!("origin {} is outside this read", query.origin))
            };
            // Observation time is evidence metadata only; the reduction
            // never reads it.
            trail
                .ingest_at(query.scope, response, Timestamp::now().as_secs())
                .map_err(|e| CliError::Other(e.to_string()))?;
        }
    }
    let trail = trail.finish();
    if text {
        print!("{}", trail.readable());
        Ok(())
    } else {
        print_json(&trail)
    }
}

/// Recheck a saved trail offline (`-` reads stdin).
pub fn cmd_recheck(file: &str) -> Result<(), CliError> {
    let text = if file == "-" {
        std::io::read_to_string(std::io::stdin())
    } else {
        std::fs::read_to_string(file)
    }
    .map_err(|e| CliError::Usage(format!("--file {file}: {e}")))?;
    let saved: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| CliError::Usage(format!("--file {file}: not JSON: {e}")))?;
    let check = trust::check_evidence(&saved).map_err(|e| CliError::Usage(e.to_string()))?;
    print_json(&check)?;
    if check.holds() {
        Ok(())
    } else {
        Err(CliError::Other(
            "saved trail does not recheck from its evidence".into(),
        ))
    }
}

/// Fetch one event by id from the current relay and check its id and
/// signature. Writers derive references only from events checked here.
pub(crate) async fn fetch_verified(client: &BuzzClient, id: &str) -> Result<Event, CliError> {
    let id = parse_id("event", id)?;
    let raw = client
        .query(&serde_json::json!({ "ids": [id.to_hex()], "limit": 1 }))
        .await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
    let value = events
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NotFound(format!("event {id} not found or not readable")))?;
    let event: Event = serde_json::from_value(value)
        .map_err(|e| CliError::Other(format!("event {id} is not a valid event: {e}")))?;
    if event.id != id || event.verify().is_err() {
        return Err(CliError::Other(format!(
            "event {id} failed id or signature verification"
        )));
    }
    Ok(event)
}

/// Submit and require the relay's acceptance.
async fn submit(client: &BuzzClient, event: Event, what: &str) -> Result<(), CliError> {
    let id = event.id.to_hex();
    let raw = client.submit_event(event).await?;
    let response: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e} ({raw})")))?;
    if response
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        let message = response.get("message").and_then(serde_json::Value::as_str);
        return Err(CliError::Other(format!(
            "relay rejected the {what}: {}",
            message.unwrap_or(&raw)
        )));
    }
    print_json(&serde_json::json!({ "id": id, "relay": response }))
}

/// Parse `<event-id>[=<origin>]`; the origin defaults to the current relay.
fn parse_selection(raw: &str, default_origin: &str) -> Result<Selected, CliError> {
    let (id, origin) = match raw.split_once('=') {
        Some((id, origin)) => (id, normalize_relay_url(origin)),
        None => (raw, default_origin.to_owned()),
    };
    let id = EventId::from_hex(id.trim())
        .map_err(|e| CliError::Usage(format!("--select {raw}: not an event id: {e}")))?;
    Ok(Selected { id, origin })
}

async fn cmd_fingerprint(
    client: &BuzzClient,
    d: &str,
    select: &[String],
    publish: bool,
) -> Result<(), CliError> {
    let selections = select
        .iter()
        .map(|raw| parse_selection(raw, client.relay_url()))
        .collect::<Result<Vec<_>, _>>()?;
    let builder = trust::build_work_fingerprint(d, &selections)
        .map_err(|e| CliError::Usage(e.to_string()))?;
    let event = client.sign_event(builder)?;
    if !publish {
        return print_json(&trust::transport_bundle(&event));
    }
    submit(client, event, "fingerprint").await
}

fn read_bundle(path: &str) -> Result<serde_json::Value, CliError> {
    let raw = if path == "-" {
        std::io::read_to_string(std::io::stdin())
            .map_err(|e| CliError::Other(format!("failed to read stdin: {e}")))?
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| CliError::Usage(format!("failed to read {path}: {e}")))?
    };
    serde_json::from_str(&raw).map_err(|e| CliError::Usage(format!("bundle is not JSON: {e}")))
}

async fn cmd_verify(
    client: &BuzzClient,
    bundle: &str,
    cutoff: Option<u64>,
    viewer_known: &[String],
) -> Result<(), CliError> {
    let input = read_bundle(bundle)?;
    let policy = if viewer_known.is_empty() {
        Policy::default()
    } else {
        let keys = viewer_known
            .iter()
            .map(|k| {
                PublicKey::parse(k).map_err(|e| CliError::Usage(format!("--viewer-known {k}: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Policy::viewer_known(keys)
    };
    let cutoff = cutoff.unwrap_or_else(|| Timestamp::now().as_secs());
    let mut verifier =
        Verifier::new(&input, policy, cutoff).map_err(|e| CliError::Usage(e.to_string()))?;

    // One client per origin. Only the configured relay gets this agent's
    // owner attestation header; other origins see the bare key.
    let mut clients: BTreeMap<String, BuzzClient> = BTreeMap::new();
    for _ in 0..MAX_ROUNDS {
        let queries = verifier.next_queries();
        if queries.is_empty() {
            break;
        }
        for query in queries {
            let origin = normalize_relay_url(&query.origin);
            let target = if origin == client.relay_url() {
                client
            } else {
                if !clients.contains_key(&origin) {
                    let keys = client.keys().clone();
                    clients.insert(
                        origin.clone(),
                        BuzzClient::new(origin.clone(), keys, None, None)?,
                    );
                }
                clients
                    .get(&origin)
                    .ok_or_else(|| CliError::Other("client cache".into()))?
            };
            let response = fetch(target, &query.request).await;
            verifier
                .ingest(query.scope, response)
                .map_err(|e| CliError::Other(e.to_string()))?;
        }
    }
    print_json(&verifier.finish())
}

/// Answer one verifier request. Failures become [`Response::Failed`]: the
/// report marks what they would have found as unknown.
async fn fetch(client: &BuzzClient, request: &Request) -> Response {
    match request {
        Request::RelayInfo => match client.get_public("/").await {
            Ok(raw) => Response::RelayInfo(
                serde_json::from_str::<serde_json::Value>(&raw)
                    .ok()
                    .and_then(|doc| doc.get("self")?.as_str().map(str::to_owned))
                    .and_then(|hex| PublicKey::from_hex(&hex).ok()),
            ),
            Err(e) => Response::Failed(e.to_string()),
        },
        Request::Events(filter) => {
            let filter = match serde_json::to_value(filter) {
                Ok(filter) => filter,
                Err(e) => return Response::Failed(format!("filter serialization failed: {e}")),
            };
            match client.query(&filter).await {
                Ok(raw) => match serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
                    Ok(events) => Response::Events(events),
                    Err(e) => Response::Failed(format!("response is not an event array: {e}")),
                },
                Err(e) => Response::Failed(e.to_string()),
            }
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    let out = serde_json::to_string_pretty(value)
        .map_err(|e| CliError::Other(format!("serialization failed: {e}")))?;
    println!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "c11048f3136e1dfd5c0b79267bcf6ea11630ea64e25bd3808d55a1c337b068db";

    #[test]
    fn selection_origin_defaults_to_current_relay_and_normalizes() {
        let s = parse_selection(ID, "https://a.example").expect("default origin");
        assert_eq!(s.origin, "https://a.example");
        let s = parse_selection(&format!("{ID}=wss://b.example/"), "https://a.example")
            .expect("explicit origin");
        assert_eq!(s.origin, "https://b.example");
        assert!(parse_selection("nope", "https://a.example").is_err());
    }

    #[test]
    fn trail_entry_needs_exactly_one_start() {
        assert_eq!(
            parse_entry(Some(ID), None, None).expect("request"),
            Entry::Request(EventId::from_hex(ID).expect("id"))
        );
        assert!(matches!(
            parse_entry(None, Some(ID), None),
            Ok(Entry::Contribution(_))
        ));
        assert!(parse_entry(Some(ID), Some(ID), None).is_err());
        assert!(parse_entry(None, None, None).is_err());
        assert!(parse_entry(None, None, Some("not-a-key")).is_err());
    }

    #[test]
    fn trail_scope_is_explicit_and_validated() {
        let owner = "a".repeat(64);
        let scope = parse_scope(
            "https://a.example",
            &["7a601c18-b045-4726-9b2b-efe9605e4f27".into()],
            &[format!("30617:{owner}:repo")],
            Some(10),
            Some(20),
        )
        .expect("scope");
        assert_eq!((scope.since, scope.until), (Some(10), 20));
        assert_eq!(scope.origin, "https://a.example");
        assert!(parse_scope("o", &["general".into()], &[], None, None).is_err());
        assert!(parse_scope("o", &[], &["30617:short:repo".into()], None, None).is_err());
        assert!(parse_scope("o", &[], &[format!("30618:{owner}:repo")], None, None).is_err());
        assert!(parse_scope("o", &[], &[], Some(30), Some(20)).is_err());
    }

    #[test]
    fn contribution_flags_must_fit_the_role() {
        let spec = |state: Option<&str>, artifact: bool| PartSpec {
            summary: "s".into(),
            state: state.map(str::to_owned),
            artifact: artifact.then(|| "inline: x".to_owned()),
            artifact_revision: artifact.then(|| "sha256:00".to_owned()),
        };
        assert!(check_part_flags(WorkRole::Commitment, &spec(None, false), false).is_ok());
        assert!(check_part_flags(WorkRole::Commitment, &spec(None, true), false).is_err());
        assert!(check_part_flags(WorkRole::Commitment, &spec(None, false), true).is_err());
        assert!(check_part_flags(WorkRole::Update, &spec(None, false), false).is_err());
        assert!(check_part_flags(WorkRole::Update, &spec(Some("blocked"), false), false).is_ok());
        assert!(check_part_flags(WorkRole::Submission, &spec(None, false), false).is_err());
        assert!(check_part_flags(WorkRole::Submission, &spec(None, true), true).is_ok());
        assert!(
            check_part_flags(WorkRole::Submission, &spec(Some("working"), true), false).is_err()
        );
    }
}
