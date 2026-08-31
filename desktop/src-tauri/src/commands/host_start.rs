//! Owner-operated Desktop Start transport. No host-key login and no source
//! identity/config/file transfer. Command/receipt payloads are encrypted; routing
//! envelopes and local retry bookkeeping contain no launch secrets.
use super::host_start_store::{prepare_receipt, retry_pending, Pending, Store};
use crate::app_state::AppState;
use buzz_core_pkg::{
    host,
    host_execution::{self, Action, Command, Receipt},
};
use nostr::{Event, JsonUtil, Keys, Timestamp};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

pub(super) fn scope(state: &AppState, owner: &str, relay: &str) -> Result<(Keys, String), String> {
    let keys = super::hosts::owner_keys(state, owner)?;
    let relay =
        buzz_core_pkg::relay::normalize_relay_url(relay).map_err(|_| "invalid Start community")?;
    crate::relay::assert_expected_relay_scope(
        Some(&relay),
        &crate::relay::relay_api_base_url_with_override(state),
    )?;
    Ok((keys, relay))
}

pub(super) async fn current_registration(
    state: &AppState,
    owner: &Keys,
    relay: &str,
    id: &str,
) -> Result<Event, String> {
    let events = crate::relay::query_private_host_at_with_keys(state, &crate::relay::relay_http_base_url(relay),
        &[serde_json::json!({"kinds":[50000], "ids":[id], "#p":[owner.public_key().to_hex()], "limit":2})], owner, None)
        .await.map_err(|_| "cannot verify destination registration")?;
    let [reg] = events.as_slice() else {
        return Err("destination registration absent or revoked".into());
    };
    let binding = host::validate(reg)?;
    if reg.id.to_hex() != id
        || binding.label != "registration"
        || binding.owner != owner.public_key()
    {
        return Err("destination registration mismatch".into());
    }
    Ok(reg.clone())
}

pub(super) fn request(owner: &Keys, pending: &Pending, relay: &str) -> Result<Command, String> {
    host_execution::validate_transport(
        &pending.command,
        &pending.registration,
        owner.public_key(),
    )?;
    if pending.command.kind.as_u16() != 50001 {
        return Err("invalid Start command".into());
    }
    let binding = host::validate(&pending.registration)?;
    let text =
        nostr::nips::nip44::decrypt(owner.secret_key(), &binding.host, &pending.command.content)
            .map_err(|_| "invalid Start ciphertext")?;
    let req: Command = serde_json::from_str(&text).map_err(|_| "invalid Start request")?;
    req.validate()?;
    if req.relay != relay {
        return Err("invalid Start scope".into());
    }
    Ok(req)
}

/// Persist a single immutable Start before any publication. Double click/restart
/// returns the existing attempt for that agent/destination, not another launch.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Explicit Tauri IPC fields; no opaque launch payload.
pub async fn queue_host_start(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
    registration: serde_json::Value,
    agent: String,
    runtime: String,
    revision: String,
    new_attempt_after: Option<String>,
) -> Result<String, String> {
    require_preview()?;
    let (owner, relay) = scope(&state, &expected_owner, &expected_relay)?;
    let supplied = Event::from_json(registration.to_string()).map_err(|_| "invalid destination")?;
    let reg = current_registration(&state, &owner, &relay, &supplied.id.to_hex()).await?;
    scope(&state, &expected_owner, &relay)?;
    let mut store = Store::open(&app, &expected_owner, &relay)?;
    let host = host::validate(&reg)?.host;
    validate_journal(&store, &owner, &relay)?;
    super::host_move::check_reservation(&store, &owner, &relay, &agent, host)?;
    let previous = current_attempt(&store, &owner, &relay, &agent, host)?;
    let supersedes = if let Some(pending) = previous {
        let req = request(&owner, pending, &relay)?;
        if new_attempt_after.is_none() {
            return Ok(req.operation);
        }
        if new_attempt_after.as_deref() != Some(req.operation.as_str()) {
            return Err("Start intent changed; refresh before creating a new attempt".into());
        }
        let rejected = pending
            .receipt
            .as_ref()
            .map(|event| {
                host_execution::decrypt_receipt(
                    &owner,
                    &pending.registration,
                    event,
                    &pending.command,
                    &req,
                )
            })
            .transpose()?
            .is_some_and(|r| r.outcome == host_execution::Outcome::Rejected);
        if !rejected && !confirmed_stop(&state, &owner, &relay, pending, &req).await? {
            return Err("New Start requires a signed confirmed Stop of the prior run; retry the saved operation instead".into());
        }
        Some(pending.command.id.to_hex())
    } else {
        if new_attempt_after.is_some() {
            return Err("Prior Start intent not found".into());
        }
        None
    };
    scope(&state, &expected_owner, &relay)?;
    let now = Timestamp::now().as_secs();
    let req = Command {
        v: 1,
        operation: uuid::Uuid::new_v4().simple().to_string(),
        relay,
        agent,
        expires_at: now + host_execution::COMMAND_TTL,
        action: Action::Start { runtime, revision },
    };
    let command = host_execution::command(&owner, &reg, &req, now)?;
    store.journal.sent.insert(
        command.id.to_hex(),
        Pending {
            registration: reg,
            command,
            receipt: None,
            published: false,
            error: None,
            supersedes,
        },
    );
    store.save()?;
    Ok(req.operation)
}

/// Private progress view: a relay ACK is explicitly not workload readiness.
#[derive(Serialize)]
pub struct StartProgress {
    operation: String,
    created_at: u64,
    current: bool,
    action: String,
    agent: String,
    host: String,
    run: String,
    status: String,
    error: Option<String>,
}

pub(super) async fn history(
    state: &AppState,
    owner: &Keys,
    relay: &str,
    mut filter: serde_json::Value,
) -> Result<Vec<Event>, String> {
    let mut result = Vec::new();
    loop {
        let page = crate::relay::query_private_host_at_with_keys(
            state,
            &crate::relay::relay_http_base_url(relay),
            &[filter.clone()],
            owner,
            None,
        )
        .await
        .map_err(|_| "Start history unavailable")?;
        if page.len() > 1000 || result.len() + page.len() > 4096 {
            return Err("Start history requires archival".into());
        }
        let mut page = page;
        page.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        if page
            .iter()
            .any(|event| result.iter().any(|old: &Event| old.id == event.id))
        {
            return Err("Start history cursor did not advance".into());
        }
        let done = page.len() < 1000;
        if let Some(last) = page.last() {
            filter["until"] = last.created_at.as_secs().into();
            filter["before_id"] = last.id.to_hex().into();
        }
        result.extend(page);
        if done {
            return Ok(result);
        }
    }
}

async fn publish(
    state: &AppState,
    owner: &Keys,
    relay: &str,
    reg: &Event,
    event: &Event,
) -> Result<(), String> {
    host_execution::validate_transport(event, reg, owner.public_key())?;
    let url = format!(
        "{}/events",
        crate::relay::relay_http_base_url(relay).trim_end_matches('/')
    );
    let bytes = event.as_json().into_bytes();
    crate::egress_guard::assert_no_key_backup_bytes(&bytes, "host Start transport")?;
    let auth = crate::relay::build_nip98_auth_header_for_keys(
        owner,
        &reqwest::Method::POST,
        &url,
        &bytes,
    )?;
    let response = state
        .media_fetch_client
        .post(url)
        .timeout(crate::relay::PRIVATE_HOST_REQUEST_TIMEOUT)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(bytes)
        .send()
        .await
        .map_err(|_| "Start publication unconfirmed")?;
    if !response.status().is_success() {
        return Err("Start publication rejected or unconfirmed".into());
    }
    let ack: crate::relay::SubmitEventResponse = crate::relay::parse_json_response(response)
        .await
        .map_err(|_| "invalid Start acknowledgement")?;
    if !ack.accepted || ack.event_id != event.id.to_hex() {
        return Err("Start publication unconfirmed".into());
    }
    Ok(())
}

/// Recover origin outbox, receive destination commands, and publish exact signed
/// receipts. Called by the app-scoped receiver, serialized across windows/processes.
#[tauri::command]
pub async fn pump_host_start(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
) -> Result<StartSnapshot, String> {
    if !cfg!(feature = "remote-start-preview") {
        return Ok(StartSnapshot {
            operations: vec![],
            moves: vec![],
            errors: vec![],
        });
    }
    let (owner, relay) = scope(&state, &expected_owner, &expected_relay)?;
    let host_keys = super::hosts::host_keys(&owner)?;
    let mut store = Store::open(&app, &expected_owner, &relay)?;
    validate_journal(&store, &owner, &relay)?;
    // Read confirmed results before sending again: lost ACK never mints a request.
    let receipts = history(
        &state,
        &owner,
        &relay,
        serde_json::json!({"kinds":[50002], "#p":[expected_owner], "limit":1000}),
    )
    .await?;
    scope(&state, &expected_owner, &relay)?;
    for pending in store.journal.sent.values_mut() {
        let req = request(&owner, pending, &relay)?;
        if pending.receipt.is_none() {
            pending.receipt = receipts
                .iter()
                .find(|event| {
                    host_execution::decrypt_receipt(
                        &owner,
                        &pending.registration,
                        event,
                        &pending.command,
                        &req,
                    )
                    .is_ok()
                })
                .cloned();
        }
    }
    store.save()?;
    super::host_move::advance_moves(&state, &owner, &relay, &mut store).await?;
    let commands = history(&state, &owner, &relay, serde_json::json!({"kinds":[50001], "authors":[expected_owner], "#p":[expected_owner], "#x":[host_keys.public_key().to_hex()], "limit":1000})).await?;
    let mut receiver_errors = Vec::new();
    for command in commands {
        scope(&state, &expected_owner, &relay)?;
        if store.journal.received.contains_key(&command.id.to_hex()) {
            continue;
        }
        let Some(id) = command.tags.iter().find_map(|tag| {
            let t = tag.as_slice();
            (t.len() == 2 && t[0] == "e").then(|| t[1].clone())
        }) else {
            continue;
        };
        let reg = match current_registration(&state, &owner, &relay, &id).await {
            Ok(reg) => reg,
            Err(error) => {
                receiver_errors.push(error);
                continue;
            }
        };
        let Ok(req) = host_execution::decrypt_command(
            &host_keys,
            &reg,
            &command,
            &relay,
            command.created_at.as_secs(),
        ) else {
            continue;
        };
        // Both actions use the same exact-run native authority and signed receipts.
        if store.journal.sent.len() + store.journal.received.len() + store.journal.moves.len()
            >= 4096
        {
            return Err("Start outbox requires archival".into());
        }
        let value = serde_json::to_value(&command).map_err(|_| "invalid Start command")?;
        let value = match super::execute_host_command(
            app.clone(),
            state.clone(),
            expected_owner.clone(),
            relay.clone(),
            value,
        )
        .await
        {
            Ok(value) => value,
            Err(_) => {
                receiver_errors.push(
                    "Destination execution unconfirmed; immutable request retained for retry"
                        .into(),
                );
                continue;
            }
        };
        let receipt =
            Event::from_json(value.to_string()).map_err(|_| "invalid native Start receipt")?;
        host_execution::decrypt_receipt(&owner, &reg, &receipt, &command, &req)?;
        store.journal.received.insert(
            command.id.to_hex(),
            Pending {
                registration: reg,
                command,
                receipt: Some(receipt),
                published: false,
                error: None,
                supersedes: None,
            },
        );
        store.save()?;
    }
    // The durable publication owner fsyncs each prepared envelope before send.
    // One revoked/unpublishable operation must not starve another placement.
    store
        .retry_receipts(
            |mut pending| {
                let (state, owner, host_keys, relay, receipts, expected_owner) = (
                    &state,
                    &owner,
                    &host_keys,
                    &relay,
                    &receipts,
                    &expected_owner,
                );
                async move {
                    current_registration(state, owner, relay, &pending.registration.id.to_hex())
                        .await?;
                    scope(state, expected_owner, relay)?;
                    prepare_receipt(
                        &mut pending,
                        owner,
                        host_keys,
                        relay,
                        receipts,
                        Timestamp::now().as_secs(),
                    )?;
                    Ok(pending)
                }
            },
            |pending| publish_pending(&state, &owner, &relay, pending, true),
        )
        .await?;
    retry_pending(&mut store.journal.sent, false, |pending| {
        publish_pending(&state, &owner, &relay, pending, false)
    })
    .await;
    store.save()?;
    scope(&state, &expected_owner, &relay)?;
    app.manage(ReceiverHealth::default());
    if let Ok(mut health) = app.state::<ReceiverHealth>().0.lock() {
        *health = Some((expected_owner, relay.clone(), std::time::Instant::now()));
    }
    let operations = store
        .journal
        .sent
        .values()
        .map(|pending| {
            let req = request(&owner, pending, &relay)?;
            let result: Option<Receipt> = pending
                .receipt
                .as_ref()
                .map(|event| {
                    host_execution::decrypt_receipt(
                        &owner,
                        &pending.registration,
                        event,
                        &pending.command,
                        &req,
                    )
                })
                .transpose()?;
            let status = match result {
                Some(r) => serde_json::to_value(r.outcome)
                    .map_err(|_| "invalid outcome")?
                    .as_str()
                    .ok_or("invalid outcome")?
                    .to_string(),
                None if req.expires_at <= Timestamp::now().as_secs() => "unknown".into(),
                None if pending.published => "relay_accepted".into(),
                None => "queued".into(),
            };
            Ok(StartProgress {
                action: if matches!(req.action, Action::Start { .. }) {
                    "start"
                } else {
                    "stop"
                }
                .into(),
                current: !store.journal.sent.values().any(|other| {
                    other.supersedes.as_deref() == Some(pending.command.id.to_hex().as_str())
                }),
                created_at: pending.command.created_at.as_secs(),
                host: host::validate(&pending.registration)?.host.to_hex(),
                run: req.run().into(),
                operation: req.operation,
                agent: req.agent,
                status,
                error: pending.error.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let errors = store
        .journal
        .received
        .values()
        .filter_map(|pending| pending.error.clone())
        .chain(receiver_errors)
        .collect();
    let moves = super::host_move::progress(&store, &owner, &relay)?;
    Ok(StartSnapshot {
        operations,
        errors,
        moves,
    })
}

/// A preview build is intentionally required until real two-executor validation.
pub(super) fn require_preview() -> Result<(), String> {
    if cfg!(feature = "remote-start-preview") {
        Ok(())
    } else {
        Err("Remote Start preview is not enabled in this build".into())
    }
}

#[derive(Default)]
pub(super) struct ReceiverHealth(std::sync::Mutex<Option<(String, String, std::time::Instant)>>);

pub(super) fn receiver_healthy(app: &AppHandle, owner: &str, relay: &str) -> bool {
    cfg!(feature = "remote-start-preview")
        && app.try_state::<ReceiverHealth>().is_some_and(|state| {
            state.0.lock().is_ok_and(|health| {
                health.as_ref().is_some_and(|(o, r, at)| {
                    o == owner && r == relay && at.elapsed() < std::time::Duration::from_secs(15)
                })
            })
        })
}

/// Private transport snapshot. Per-operation failures never hide other outcomes.
#[derive(Serialize)]
pub struct StartSnapshot {
    operations: Vec<StartProgress>,
    moves: Vec<super::host_move::MoveProgress>,
    errors: Vec<String>,
}

pub(super) fn validate_journal(store: &Store, owner: &Keys, relay: &str) -> Result<(), String> {
    let mut superseded = std::collections::HashSet::new();
    for (id, pending) in store.journal.sent.iter().chain(&store.journal.received) {
        let req = request(owner, pending, relay)?;
        if *id != pending.command.id.to_hex() {
            return Err("Start outbox binding corrupt".into());
        }
        if let Some(receipt) = &pending.receipt {
            host_execution::decrypt_receipt(
                owner,
                &pending.registration,
                receipt,
                &pending.command,
                &req,
            )?;
        }
        if let Some(previous) = &pending.supersedes {
            if !superseded.insert(previous) {
                return Err("Start outbox has branched intents".into());
            }
            let mut visited = std::collections::HashSet::new();
            let mut cursor = Some(id);
            while let Some(next) = cursor {
                if !visited.insert(next) {
                    return Err("Start outbox has cyclic intents".into());
                }
                cursor = store
                    .journal
                    .sent
                    .get(next)
                    .and_then(|p| p.supersedes.as_ref());
            }
            let old = store
                .journal
                .sent
                .get(previous)
                .ok_or("Start outbox predecessor missing")?;
            let old_req = request(owner, old, relay)?;
            if !matches!(req.action, Action::Start { .. })
                || !matches!(old_req.action, Action::Start { .. })
                || previous == id
                || old_req.agent != req.agent
                || host::validate(&old.registration)?.host
                    != host::validate(&pending.registration)?.host
            {
                return Err("Start outbox predecessor corrupt".into());
            }
        }
    }
    if store
        .journal
        .received
        .values()
        .any(|pending| pending.receipt.is_none())
    {
        return Err("Start outbox missing receipt".into());
    }
    super::host_move::validate_moves(store, owner, relay)?;
    Ok(())
}

pub(super) fn current_attempt<'a>(
    store: &'a Store,
    owner: &Keys,
    relay: &str,
    agent: &str,
    host_key: nostr::PublicKey,
) -> Result<Option<&'a Pending>, String> {
    let mut current = None;
    for (id, pending) in &store.journal.sent {
        if !matches!(request(owner, pending, relay)?.action, Action::Start { .. })
            || request(owner, pending, relay)?.agent != agent
            || host::validate(&pending.registration)?.host != host_key
        {
            continue;
        }
        if store
            .journal
            .sent
            .values()
            .any(|p| p.supersedes.as_ref() == Some(id))
        {
            continue;
        }
        if current.replace(pending).is_some() {
            return Err("Multiple prior Start intents require reconciliation".into());
        }
    }
    Ok(current)
}

pub(super) async fn confirmed_stop(
    state: &AppState,
    owner: &Keys,
    relay: &str,
    pending: &Pending,
    previous: &Command,
) -> Result<bool, String> {
    let filter = |kind| serde_json::json!({"kinds":[kind], "#p":[owner.public_key().to_hex()], "#e":[pending.registration.id.to_hex()], "limit":1000});
    let commands = history(state, owner, relay, filter(50001)).await?;
    let receipts = history(state, owner, relay, filter(50002)).await?;
    for command in commands {
        let candidate = Pending {
            registration: pending.registration.clone(),
            command,
            receipt: None,
            published: false,
            error: None,
            supersedes: None,
        };
        let Ok(req) = request(owner, &candidate, relay) else {
            continue;
        };
        if req.agent != previous.agent
            || !matches!(&req.action, Action::Stop { run } if run == previous.run())
        {
            continue;
        }
        if receipts.iter().any(|event| {
            host_execution::decrypt_receipt(
                owner,
                &candidate.registration,
                event,
                &candidate.command,
                &req,
            )
            .is_ok_and(|receipt| receipt.outcome == host_execution::Outcome::Stopped)
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn publish_pending(
    state: &AppState,
    owner: &Keys,
    relay: &str,
    pending: Pending,
    receipt: bool,
) -> Result<(), String> {
    scope(state, &owner.public_key().to_hex(), relay)?;
    let req = request(owner, &pending, relay)?;
    let event = if receipt {
        let event = pending
            .receipt
            .as_ref()
            .ok_or("missing durable Start receipt")?;
        host_execution::decrypt_receipt(
            owner,
            &pending.registration,
            event,
            &pending.command,
            &req,
        )?;
        event
    } else {
        &pending.command
    };
    current_registration(state, owner, relay, &pending.registration.id.to_hex()).await?;
    scope(state, &owner.public_key().to_hex(), relay)?;
    publish(state, owner, relay, &pending.registration, event).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending(owner: &Keys, host: &Keys, operation: &str) -> Pending {
        let registration = host::registration(owner, host.public_key(), 100).unwrap();
        let req = Command {
            v: 1,
            operation: operation.repeat(16),
            relay: "wss://relay.example".into(),
            agent: owner.public_key().to_hex(),
            expires_at: 400,
            action: Action::Start {
                runtime: "goose".into(),
                revision: "ab".repeat(32),
            },
        };
        let command = host_execution::command(owner, &registration, &req, 100).unwrap();
        Pending {
            registration,
            command,
            receipt: None,
            published: false,
            error: None,
            supersedes: None,
        }
    }

    #[tokio::test]
    async fn start_and_receipt_block_key_backup_before_network() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("ws://{}", listener.local_addr().unwrap());
        let owner = Keys::generate();
        let host = Keys::generate();
        let pending = pending(&owner, &host, "ab");
        let state = crate::app_state::build_app_state();
        for (kind, signer) in [(50001, &owner), (50002, &host)] {
            let injected = nostr::EventBuilder::new(
                nostr::Kind::Custom(kind),
                "ncryptsec1synthetic-backup-injection",
            )
            .allow_self_tagging()
            .tags(pending.command.tags.clone())
            .sign_with_keys(signer)
            .unwrap();
            let error = publish(&state, &owner, &relay, &pending.registration, &injected)
                .await
                .unwrap_err();
            assert!(error.contains("blocked host Start transport"));
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn immutable_intent_chain_scoped_and_corruption_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Keys::generate();
        let host = Keys::generate();
        let relay = "wss://relay.example";
        let mut store = Store::open_dir(dir.path(), &owner.public_key().to_hex(), relay).unwrap();
        let first = pending(&owner, &host, "ab");
        let first_id = first.command.id.to_hex();
        store.journal.sent.insert(first_id.clone(), first);
        validate_journal(&store, &owner, relay).unwrap();
        assert!(validate_journal(&store, &owner, "wss://foreign.example").is_err());
        assert!(validate_journal(&store, &Keys::generate(), relay).is_err());
        let mut second = pending(&owner, &host, "cd");
        let second_id = second.command.id.to_hex();
        second.supersedes = Some(first_id.clone());
        store.journal.sent.insert(second_id.clone(), second);
        validate_journal(&store, &owner, relay).unwrap();
        assert_eq!(
            current_attempt(
                &store,
                &owner,
                relay,
                &owner.public_key().to_hex(),
                host.public_key()
            )
            .unwrap()
            .unwrap()
            .command
            .id
            .to_hex(),
            second_id
        );
        store.journal.sent.get_mut(&first_id).unwrap().supersedes = Some(second_id.clone());
        assert!(
            validate_journal(&store, &owner, relay).is_err(),
            "cycle cannot erase current intent"
        );
        store.journal.sent.get_mut(&first_id).unwrap().supersedes = None;
        store
            .journal
            .sent
            .get_mut(&second_id)
            .unwrap()
            .command
            .content = "corrupt".into();
        assert!(validate_journal(&store, &owner, relay).is_err());
    }
}
