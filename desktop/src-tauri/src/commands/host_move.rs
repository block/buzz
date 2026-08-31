//! Selected-run Move is a dependency in the existing Start outbox, not another
//! execution ledger. Only a verified Stopped receipt releases the destination.
use super::host_start::{current_attempt, current_registration, history, request, scope};
use super::host_start_store::{Pending, Store};
use crate::app_state::AppState;
use buzz_core_pkg::{
    host,
    host_execution::{self, Action, Command, Outcome},
};
use nostr::{Event, JsonUtil, Keys, PublicKey, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MoveIntent {
    stop: String,
    // A signed but NEVER published Start template. Its operation ID is reserved;
    // its TTL is refreshed once, on release, before persistence/publication.
    destination: Pending,
    // Domain-separated owner signature binds the two commands. Swapping a
    // destination or borrowing another Move's Stop cannot authorize release.
    authorization: String,
    start: Option<String>,
    error: Option<String>,
}

fn authorization(stop: &str, destination: &Pending) -> nostr::secp256k1::Message {
    nostr::secp256k1::Message::from_digest(
        Sha256::digest(format!(
            "buzz.host.move.v1\n{stop}\n{}\n{}",
            destination.command.id,
            destination.supersedes.as_deref().unwrap_or("")
        ))
        .into(),
    )
}

fn pending(owner: &Keys, registration: Event, req: &Command, now: u64) -> Result<Pending, String> {
    Ok(Pending {
        command: host_execution::command(owner, &registration, req, now)?,
        registration,
        receipt: None,
        published: false,
        error: None,
        supersedes: None,
    })
}

fn outcome(owner: &Keys, pending: &Pending, relay: &str) -> Result<Option<Outcome>, String> {
    let req = request(owner, pending, relay)?;
    pending
        .receipt
        .as_ref()
        .map(|receipt| {
            host_execution::decrypt_receipt(
                owner,
                &pending.registration,
                receipt,
                &pending.command,
                &req,
            )
            .map(|r| r.outcome)
        })
        .transpose()
}

pub(super) fn validate_moves(store: &Store, owner: &Keys, relay: &str) -> Result<(), String> {
    let mut sources = std::collections::HashSet::new();
    let mut reservations = std::collections::HashSet::new();
    for (id, intent) in &store.journal.moves {
        let stop = store
            .journal
            .sent
            .get(&intent.stop)
            .ok_or("Move Stop missing")?;
        let src = request(owner, stop, relay)?;
        let dst = request(owner, &intent.destination, relay)?;
        let source_host = host::validate(&stop.registration)?.host;
        let destination_host = host::validate(&intent.destination.registration)?.host;
        if id != &src.operation
            || !matches!(src.action, Action::Stop { .. })
            || !matches!(dst.action, Action::Start { .. })
            || src.agent != dst.agent
            || source_host == destination_host
            || src.operation == dst.operation
            || intent.destination.receipt.is_some()
            || intent.destination.published
            || !sources.insert((src.agent.clone(), source_host, src.run().to_owned()))
        {
            return Err("Move binding corrupt".into());
        }
        let sig = intent
            .authorization
            .parse::<nostr::secp256k1::schnorr::Signature>()
            .map_err(|_| "Move authorization invalid")?;
        let key = owner
            .public_key()
            .xonly()
            .map_err(|_| "Move owner invalid")?;
        nostr::SECP256K1
            .verify_schnorr(
                &sig,
                &authorization(&intent.stop, &intent.destination),
                &key,
            )
            .map_err(|_| "Move authorization invalid")?;
        if let Some(start) = &intent.start {
            let released = store.journal.sent.get(start).ok_or("Move Start missing")?;
            let req = request(owner, released, relay)?;
            if outcome(owner, stop, relay)? != Some(Outcome::Stopped)
                || req.operation != dst.operation
                || req.agent != dst.agent
                || req.action != dst.action
                || released.registration != intent.destination.registration
                || released.supersedes != intent.destination.supersedes
            {
                return Err("Move released without exact confirmed Stop".into());
            }
        } else if !reservations.insert((dst.agent.clone(), destination_host)) {
            return Err("Move destination has competing reservations".into());
        }
    }
    Ok(())
}

pub(super) fn check_reservation(
    store: &Store,
    owner: &Keys,
    relay: &str,
    agent: &str,
    host_key: PublicKey,
) -> Result<(), String> {
    for intent in store.journal.moves.values().filter(|m| m.start.is_none()) {
        if request(owner, &intent.destination, relay)?.agent == agent
            && host::validate(&intent.destination.registration)?.host == host_key
        {
            return Err("Destination reserved by Move; confirmed source Stop is required".into());
        }
    }
    Ok(())
}

// Advisory availability is re-read natively before Stop and again before Start.
// The actual executor independently revalidates configuration at the spawn seam.
async fn destination_config(
    state: &AppState,
    owner: &Keys,
    relay: &str,
    reg: &Event,
    req: &Command,
) -> Result<(), String> {
    let Action::Start { runtime, revision } = &req.action else {
        return Err("invalid Move destination".into());
    };
    let host_key = host::validate(reg)?.host;
    let mut events = history(
        state,
        owner,
        relay,
        serde_json::json!({
            "kinds":[50000], "authors":[host_key.to_hex()], "#p":[owner.public_key().to_hex()],
            "#e":[reg.id.to_hex()], "#l":["profile"], "limit":1000
        }),
    )
    .await?;
    events.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let event = events
        .first()
        .ok_or("Destination capability profile unavailable")?;
    let report = host::decrypt_report(owner, reg, event)?;
    if !report.accepts_start
        || !report
            .provisioned
            .iter()
            .any(|c| c.agent == req.agent && c.runtime == *runtime && c.revision == *revision)
    {
        return Err(
            "Destination setup changed or Start receiver unavailable; source is not restarted"
                .into(),
        );
    }
    Ok(())
}

/// Persist one owner-authorized Move and exact-run Stop before publication. A
/// repeated click/restart reuses that intent; a different destination is not a retry.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn queue_host_move(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
    source_registration: serde_json::Value,
    run: String,
    destination_registration: serde_json::Value,
    agent: String,
    runtime: String,
    revision: String,
) -> Result<String, String> {
    super::host_start::require_preview()?;
    let (owner, relay) = scope(&state, &expected_owner, &expected_relay)?;
    let source =
        Event::from_json(source_registration.to_string()).map_err(|_| "invalid Move source")?;
    let destination = Event::from_json(destination_registration.to_string())
        .map_err(|_| "invalid Move destination")?;
    let mut store = Store::open(&app, &expected_owner, &relay)?;
    super::host_start::validate_journal(&store, &owner, &relay)?;
    // Retry before availability reads: unavailable/revoked hosts must not erase intent.
    let existing = store.journal.moves.iter().find_map(|(id, intent)| {
        let stop = store.journal.sent.get(&intent.stop)?;
        let src = request(&owner, stop, &relay).ok()?;
        (src.agent == agent
            && src.run() == run
            && host::validate(&stop.registration).ok()?.host == host::validate(&source).ok()?.host)
            .then(|| (id.clone(), intent.clone()))
    });
    if let Some((id, mut intent)) = existing {
        if intent.destination.registration.id != destination.id {
            return Err("This run already has a saved Move to another destination".into());
        }
        // An explicit retry after proven Stop may adopt newly provisioned config.
        // Unknown Stop never changes its planned destination or creates a Start.
        let stop = store
            .journal
            .sent
            .get(&intent.stop)
            .ok_or("Move Stop missing")?;
        if intent.start.is_none() && outcome(&owner, stop, &relay)? == Some(Outcome::Stopped) {
            let reg =
                current_registration(&state, &owner, &relay, &destination.id.to_hex()).await?;
            let mut req = request(&owner, &intent.destination, &relay)?;
            if req.action
                != (Action::Start {
                    runtime: runtime.clone(),
                    revision: revision.clone(),
                })
            {
                req.action = Action::Start { runtime, revision };
                let now = Timestamp::now().as_secs();
                req.expires_at = now + host_execution::COMMAND_TTL;
                destination_config(&state, &owner, &relay, &reg, &req).await?;
                let mut updated = pending(&owner, reg, &req, now)?;
                updated.supersedes = intent.destination.supersedes.clone();
                intent.authorization = owner
                    .sign_schnorr(&authorization(&intent.stop, &updated))
                    .to_string();
                intent.destination = updated;
                intent.error = None;
                scope(&state, &expected_owner, &relay)?;
                store.journal.moves.insert(id.clone(), intent);
                validate_moves(&store, &owner, &relay)?;
                store.save()?;
            }
        }
        return Ok(id);
    }
    let source = current_registration(&state, &owner, &relay, &source.id.to_hex()).await?;
    let destination =
        current_registration(&state, &owner, &relay, &destination.id.to_hex()).await?;
    let destination_host = host::validate(&destination)?.host;
    let source_host = host::validate(&source)?.host.to_hex();
    let runs = super::get_presence_runs(
        state.clone(),
        expected_owner.clone(),
        relay.clone(),
        vec![agent.clone(), destination_host.to_hex()],
    )
    .await?;
    let now = Timestamp::now().as_secs();
    if !runs.get(&agent).is_some_and(|runs| {
        runs.iter().any(|r| {
            r.run == run
                && r.status != "offline"
                && r.expires_at > now
                && r.location.as_ref().is_some_and(|l| l.host == source_host)
        })
    }) {
        return Err("Selected active instance changed; refresh before Move".into());
    }
    if host::validate(&source)?.host == destination_host {
        return Err("Choose a different host".into());
    }
    if !runs.get(&destination_host.to_hex()).is_some_and(|runs| {
        runs.iter()
            .any(|r| r.status != "offline" && r.expires_at > now)
    }) {
        return Err("Destination availability unconfirmed; source was not stopped".into());
    }
    if runs.get(&agent).is_some_and(|runs| {
        runs.iter().any(|r| {
            r.status != "offline"
                && r.expires_at > now
                && r.location
                    .as_ref()
                    .is_some_and(|l| l.host == destination_host.to_hex())
        })
    }) {
        return Err(
            "Agent already has an active destination instance; source was not stopped".into(),
        );
    }
    check_reservation(&store, &owner, &relay, &agent, destination_host)?;
    // A destination with an unresolved prior Start is not silently adopted.
    let supersedes = if let Some(previous) =
        current_attempt(&store, &owner, &relay, &agent, destination_host)?
    {
        let req = request(&owner, previous, &relay)?;
        if outcome(&owner, previous, &relay)? != Some(Outcome::Rejected)
            && !super::host_start::confirmed_stop(&state, &owner, &relay, previous, &req).await?
        {
            return Err("Destination has an unresolved Start; reconcile it before Move".into());
        }
        Some(previous.command.id.to_hex())
    } else {
        None
    };
    let now = Timestamp::now().as_secs();
    let stop_req = Command {
        v: 1,
        operation: uuid::Uuid::new_v4().simple().to_string(),
        relay: relay.clone(),
        agent: agent.clone(),
        expires_at: now + host_execution::COMMAND_TTL,
        action: Action::Stop { run },
    };
    let start_req = Command {
        v: 1,
        operation: uuid::Uuid::new_v4().simple().to_string(),
        relay: relay.clone(),
        agent,
        expires_at: now + host_execution::COMMAND_TTL,
        action: Action::Start { runtime, revision },
    };
    let stop = pending(&owner, source, &stop_req, now)?;
    let mut destination = pending(&owner, destination, &start_req, now)?;
    destination.supersedes = supersedes;
    destination_config(
        &state,
        &owner,
        &relay,
        &destination.registration,
        &start_req,
    )
    .await?;
    scope(&state, &expected_owner, &relay)?;
    let id = stop.command.id.to_hex();
    let intent = MoveIntent {
        authorization: owner
            .sign_schnorr(&authorization(&id, &destination))
            .to_string(),
        stop: id.clone(),
        destination,
        start: None,
        error: None,
    };
    store.journal.sent.insert(id, stop);
    store
        .journal
        .moves
        .insert(stop_req.operation.clone(), intent);
    validate_moves(&store, &owner, &relay)?;
    store.save()?;
    Ok(stop_req.operation)
}

pub(super) async fn advance_moves(
    state: &AppState,
    owner: &Keys,
    relay: &str,
    store: &mut Store,
) -> Result<(), String> {
    let ids: Vec<_> = store.journal.moves.keys().cloned().collect();
    for id in ids {
        let intent = store.journal.moves.get(&id).ok_or("Move missing")?.clone();
        if intent.start.is_some() {
            continue;
        }
        let stop = store
            .journal
            .sent
            .get(&intent.stop)
            .ok_or("Move Stop missing")?;
        if outcome(owner, stop, relay)? != Some(Outcome::Stopped) {
            continue;
        }
        let result = release(state, owner, relay, store, &id, &intent).await;
        if let Err(error) = result {
            store
                .journal
                .moves
                .get_mut(&id)
                .ok_or("Move missing")?
                .error = Some(error);
        }
        store.save()?;
    }
    Ok(())
}

async fn release(
    state: &AppState,
    owner: &Keys,
    relay: &str,
    store: &mut Store,
    id: &str,
    intent: &MoveIntent,
) -> Result<(), String> {
    // Revocation on either leg blocks new side effects even after a saved receipt.
    let stop = store
        .journal
        .sent
        .get(&intent.stop)
        .ok_or("Move Stop missing")?;
    current_registration(state, owner, relay, &stop.registration.id.to_hex()).await?;
    current_registration(
        state,
        owner,
        relay,
        &intent.destination.registration.id.to_hex(),
    )
    .await?;
    let req = request(owner, &intent.destination, relay)?;
    destination_config(state, owner, relay, &intent.destination.registration, &req).await?;
    scope(state, &owner.public_key().to_hex(), relay)?;
    release_to_outbox(store, owner, relay, id, intent, Timestamp::now().as_secs())
}

fn release_to_outbox(
    store: &mut Store,
    owner: &Keys,
    relay: &str,
    id: &str,
    intent: &MoveIntent,
    now: u64,
) -> Result<(), String> {
    let stop = store
        .journal
        .sent
        .get(&intent.stop)
        .ok_or("Move Stop missing")?;
    if intent.start.is_some() || outcome(owner, stop, relay)? != Some(Outcome::Stopped) {
        return Err("Move requires exact confirmed Stop before release".into());
    }
    let mut req = request(owner, &intent.destination, relay)?;
    req.expires_at = now + host_execution::COMMAND_TTL;
    let mut start = pending(owner, intent.destination.registration.clone(), &req, now)?;
    start.supersedes = intent.destination.supersedes.clone();
    let start_id = start.command.id.to_hex();
    store.journal.sent.insert(start_id.clone(), start);
    let entry = store.journal.moves.get_mut(id).ok_or("Move missing")?;
    entry.start = Some(start_id);
    entry.error = None;
    // The caller persists BOTH release and outbox before retry_pending publishes.
    validate_moves(store, owner, relay)
}

#[derive(Serialize)]
pub(super) struct MoveProgress {
    operation: String,
    agent: String,
    source_host: String,
    source_run: String,
    destination_host: String,
    destination_run: String,
    status: String,
    error: Option<String>,
}

pub(super) fn progress(
    store: &Store,
    owner: &Keys,
    relay: &str,
) -> Result<Vec<MoveProgress>, String> {
    store
        .journal
        .moves
        .iter()
        .map(|(id, intent)| {
            let stop = store
                .journal
                .sent
                .get(&intent.stop)
                .ok_or("Move Stop missing")?;
            let src = request(owner, stop, relay)?;
            let dst = request(owner, &intent.destination, relay)?;
            let host_key = host::validate(&intent.destination.registration)?.host;
            // Destination recovery uses the ordinary explicit Start supersession chain.
            let current = if intent.start.is_some() {
                current_attempt(store, owner, relay, &dst.agent, host_key)?
            } else {
                None
            };
            let result = current
                .map(|p| outcome(owner, p, relay))
                .transpose()?
                .flatten();
            let stopped = outcome(owner, stop, relay)? == Some(Outcome::Stopped);
            let status = match (stopped, intent.start.is_some(), result) {
                (false, _, _)
                    if stop.receipt.is_some() || src.expires_at <= Timestamp::now().as_secs() =>
                {
                    "stop_unconfirmed"
                }
                (false, _, _) => "stopping",
                (true, false, _) => "stopped_waiting_destination",
                (true, true, Some(Outcome::Rejected)) => "stopped_start_rejected",
                (true, true, Some(Outcome::Spawned)) => "destination_spawned",
                (true, true, Some(Outcome::Listening)) => "destination_listening",
                (true, true, Some(Outcome::Ready)) => "destination_ready",
                (true, true, Some(_)) => "stopped_start_unknown",
                (true, true, None) => {
                    if current
                        .map(|p| request(owner, p, relay))
                        .transpose()?
                        .is_some_and(|r| r.expires_at <= Timestamp::now().as_secs())
                    {
                        "stopped_start_unknown"
                    } else {
                        "starting"
                    }
                }
            };
            Ok(MoveProgress {
                operation: id.clone(),
                agent: src.agent.clone(),
                source_host: host::validate(&stop.registration)?.host.to_hex(),
                source_run: src.run().into(),
                destination_host: host_key.to_hex(),
                destination_run: current
                    .map(|p| request(owner, p, relay))
                    .transpose()?
                    .map_or(dst.operation, |r| r.operation),
                status: status.into(),
                error: intent
                    .error
                    .clone()
                    .or_else(|| stop.error.clone())
                    .or_else(|| current.and_then(|p| p.error.clone())),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "host_move_tests.rs"]
mod tests;
