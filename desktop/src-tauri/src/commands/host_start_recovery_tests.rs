//! Durable outbox/restart tests with a controllable relay admission clock.
//! The relay predicate mirrors ingest's +/-900 seconds; no DB/service is used.
use super::*;
use buzz_core_pkg::{
    host,
    host_execution::{self, Action, Command, Outcome, Receipt},
};
use nostr::{JsonUtil, Keys};

fn observed(owner: &Keys, executor: &Keys) -> (Pending, Receipt) {
    let registration = host::registration(owner, executor.public_key(), 99).unwrap();
    let request = Command {
        v: 1,
        operation: "ab".repeat(16),
        relay: "wss://relay.example".into(),
        agent: Keys::generate().public_key().to_hex(),
        expires_at: 400,
        action: Action::Start {
            runtime: "buzz-agent".into(),
            revision: "cd".repeat(32),
        },
    };
    let command = host_execution::command(owner, &registration, &request, 100).unwrap();
    let result = Receipt {
        v: 1,
        command: command.id.to_hex(),
        run: request.run().into(),
        request,
        observed_at: 101,
        outcome: Outcome::Spawned,
    };
    let receipt = host_execution::receipt(executor, &registration, &result, 101).unwrap();
    (
        Pending {
            registration,
            command,
            receipt: Some(receipt),
            published: false,
            error: None,
            supersedes: None,
        },
        result,
    )
}

// Exact timestamp condition of handlers/ingest.rs, plus real signature/routing
// checks. Kept here rather than altering relay admission to accommodate retries.
fn admit(p: &Pending, owner: &Keys, now: u64) -> Result<(), String> {
    let event = p.receipt.as_ref().ok_or("missing")?;
    host_execution::validate_transport(event, &p.registration, owner.public_key())?;
    if now.abs_diff(event.created_at.as_secs()) > 900 {
        return Err("timestamp drift".into());
    }
    Ok(())
}

#[tokio::test]
async fn late_never_accepted_receipt_survives_restart_without_new_execution() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Keys::generate();
    let executor = Keys::generate();
    let (pending, observation) = observed(&owner, &executor);
    let original = pending.receipt.clone().unwrap();
    let command = pending.command.as_json();
    let id = pending.command.id.to_hex();
    let relay = observation.request.relay.clone();
    let mut store = Store::open_dir(dir.path(), "owner", &relay).unwrap();
    store.journal.received.insert(id.clone(), pending);
    store.save().unwrap();
    retry_pending(&mut store.journal.received, true, |_| async {
        Err("never reached relay".into())
    })
    .await;
    store.save().unwrap();
    drop(store);

    // More than the admission window AND the command lifetime, after restart.
    let now = 101 + 901;
    let mut store = Store::open_dir(dir.path(), "owner", &relay).unwrap();
    assert_eq!(
        admit(&store.journal.received[&id], &owner, now),
        Err("timestamp drift".into())
    );
    assert!(
        host_execution::decrypt_command(
            &executor,
            &store.journal.received[&id].registration,
            &store.journal.received[&id].command,
            &relay,
            now
        )
        .is_err(),
        "expired intent cannot execute"
    );
    prepare_receipt(
        store.journal.received.get_mut(&id).unwrap(),
        &owner,
        &executor,
        &relay,
        &[],
        now,
    )
    .unwrap();
    store.save().unwrap();
    let renewed = store.journal.received[&id].receipt.clone().unwrap();
    assert_ne!(renewed.id, original.id);
    assert_eq!(renewed.created_at.as_secs(), now);
    assert_eq!(
        renewed.content, original.content,
        "same encrypted proof, no new observation"
    );
    assert_eq!(renewed.tags, original.tags);
    assert_eq!(store.journal.received[&id].command.as_json(), command);
    drop(store); // crash after preparing, before send

    let mut store = Store::open_dir(dir.path(), "owner", &relay).unwrap();
    let pending = store.journal.received.get_mut(&id).unwrap();
    prepare_receipt(pending, &owner, &executor, &relay, &[], now + 1).unwrap();
    assert_eq!(
        pending.receipt.as_ref().unwrap().as_json(),
        renewed.as_json()
    );
    retry_pending(&mut store.journal.received, true, |p| {
        let owner = &owner;
        async move { admit(&p, owner, now + 1) }
    })
    .await;
    assert!(store.journal.received[&id].published);
    let pending = &store.journal.received[&id];
    // What the source learns from accepted history is the original exact result.
    assert_eq!(
        host_execution::decrypt_receipt(
            &owner,
            &pending.registration,
            &renewed,
            &pending.command,
            &observation.request
        )
        .unwrap(),
        observation
    );
    store.save().unwrap();
    drop(store);
    assert!(
        Store::open_dir(dir.path(), "owner", &relay)
            .unwrap()
            .journal
            .received[&id]
            .published
    );
}

#[tokio::test]
async fn accepted_but_ack_lost_uses_history_without_renewing_proof() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Keys::generate();
    let executor = Keys::generate();
    let (pending, observation) = observed(&owner, &executor);
    let original = pending.receipt.clone().unwrap();
    let id = pending.command.id.to_hex();
    let relay = &observation.request.relay;
    let mut store = Store::open_dir(dir.path(), "owner", relay).unwrap();
    store.journal.received.insert(id.clone(), pending);
    store.save().unwrap();
    store
        .retry_receipts(
            |p| std::future::ready(Ok(p)),
            |p| {
                let owner = &owner;
                async move {
                    admit(&p, owner, 101)?;
                    Err("accepted, but ACK lost".into())
                }
            },
        )
        .await
        .unwrap();
    drop(store);
    let mut store = Store::open_dir(dir.path(), "owner", relay).unwrap();
    store
        .retry_receipts(
            |mut p| {
                let result = prepare_receipt(
                    &mut p,
                    &owner,
                    &executor,
                    relay,
                    std::slice::from_ref(&original),
                    2000,
                )
                .map(|()| p);
                std::future::ready(result)
            },
            |_| async { panic!("accepted history must suppress another publication") },
        )
        .await
        .unwrap();
    assert!(store.journal.received[&id].published);
    assert_eq!(
        store.journal.received[&id]
            .receipt
            .as_ref()
            .unwrap()
            .as_json(),
        original.as_json()
    );
}

#[test]
fn history_of_an_earlier_envelope_also_resolves_ambiguous_delivery() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let (mut pending, observation) = observed(&owner, &executor);
    let original = pending.receipt.clone().unwrap();
    prepare_receipt(
        &mut pending,
        &owner,
        &executor,
        &observation.request.relay,
        &[],
        2000,
    )
    .unwrap();
    let renewed = pending.receipt.clone().unwrap();
    prepare_receipt(
        &mut pending,
        &owner,
        &executor,
        &observation.request.relay,
        &[original],
        3000,
    )
    .unwrap();
    assert!(pending.published);
    assert_eq!(pending.receipt.unwrap(), renewed);
}

#[test]
fn recovery_rejects_wrong_signer_scope_registration_missing_and_tampered_evidence() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let (pending, observation) = observed(&owner, &executor);
    let relay = &observation.request.relay;
    assert!(prepare_receipt(
        &mut pending.clone(),
        &owner,
        &Keys::generate(),
        relay,
        &[],
        2000
    )
    .is_err());
    assert!(prepare_receipt(
        &mut pending.clone(),
        &Keys::generate(),
        &executor,
        relay,
        &[],
        2000
    )
    .is_err());
    assert!(prepare_receipt(
        &mut pending.clone(),
        &owner,
        &executor,
        "wss://elsewhere.example",
        &[],
        2000
    )
    .is_err());
    let mut changed = pending.clone();
    changed.registration = host::registration(&owner, executor.public_key(), 100).unwrap();
    assert!(prepare_receipt(&mut changed, &owner, &executor, relay, &[], 2000).is_err());
    let mut changed = pending.clone();
    changed.receipt = None;
    assert!(prepare_receipt(&mut changed, &owner, &executor, relay, &[], 2000).is_err());
    let mut changed = pending;
    changed.receipt.as_mut().unwrap().content.push('x');
    assert!(prepare_receipt(&mut changed, &owner, &executor, relay, &[], 2000).is_err());
}

#[test]
fn forged_history_cannot_confirm_delivery_and_invalid_lifetime_cannot_be_renewed() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let (mut pending, mut observation) = observed(&owner, &executor);
    let original = pending.receipt.clone().unwrap();
    let forged = nostr::EventBuilder::new(original.kind, original.content.clone())
        .tags(original.tags.clone())
        .allow_self_tagging()
        .sign_with_keys(&Keys::generate())
        .unwrap();
    prepare_receipt(
        &mut pending,
        &owner,
        &executor,
        &observation.request.relay,
        &[forged],
        1002,
    )
    .unwrap();
    assert!(!pending.published);
    assert_eq!(pending.receipt.as_ref().unwrap().created_at.as_secs(), 1002);
    observation.request.expires_at = 401; // original creation + COMMAND_TTL + 1
    let content = nostr::nips::nip44::encrypt(
        owner.secret_key(),
        &executor.public_key(),
        serde_json::to_string(&observation.request).unwrap(),
        nostr::nips::nip44::Version::V2,
    )
    .unwrap();
    pending.command = nostr::EventBuilder::new(pending.command.kind, content)
        .tags(pending.command.tags.clone())
        .allow_self_tagging()
        .custom_created_at(nostr::Timestamp::from(100))
        .sign_with_keys(&owner)
        .unwrap();
    observation.command = pending.command.id.to_hex();
    pending.receipt =
        Some(host_execution::receipt(&executor, &pending.registration, &observation, 101).unwrap());
    assert!(prepare_receipt(
        &mut pending,
        &owner,
        &executor,
        &observation.request.relay,
        &[],
        1002
    )
    .is_err());
}

#[tokio::test]
async fn publication_owner_persists_before_send_and_recovers_late_unaccepted_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Keys::generate();
    let executor = Keys::generate();
    let (pending, observation) = observed(&owner, &executor);
    let relay = &observation.request.relay;
    let id = pending.command.id.to_hex();
    let mut store = Store::open_dir(dir.path(), "owner", relay).unwrap();
    store.journal.received.insert(id.clone(), pending.clone());
    // An earlier revoked entry must not starve this one.
    store.journal.received.insert("a-revoked".into(), pending);
    store.save().unwrap();
    let disk = store.path.clone();
    let prepare = |mut p: Pending, now| {
        prepare_receipt(&mut p, &owner, &executor, relay, &[], now)?;
        Ok(p)
    };
    store
        .retry_receipts(
            |p| std::future::ready(prepare(p, 101)),
            |_| async { Err("never accepted: disconnected".into()) },
        )
        .await
        .unwrap();
    drop(store);
    let mut store = Store::open_dir(dir.path(), "owner", relay).unwrap();
    store
        .journal
        .received
        .get_mut("a-revoked")
        .unwrap()
        .registration = host::registration(&owner, executor.public_key(), 100).unwrap();
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    store
        .retry_receipts(
            |p| std::future::ready(prepare(p, 1002)),
            |p| {
                let (disk, id, owner, attempts) = (&disk, &id, &owner, &attempts);
                async move {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let durable: Journal =
                        serde_json::from_slice(&fs::read(disk).unwrap()).unwrap();
                    assert_eq!(
                        durable.received[id].receipt, p.receipt,
                        "fsync before publication"
                    );
                    admit(&p, owner, 1002)
                }
            },
        )
        .await
        .unwrap();
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(store.journal.received[&id].published);
    assert!(!store.journal.received["a-revoked"].published);
    assert!(store.journal.received["a-revoked"].error.is_some());
    let p = &store.journal.received[&id];
    assert_eq!(
        host_execution::decrypt_receipt(
            &owner,
            &p.registration,
            p.receipt.as_ref().unwrap(),
            &p.command,
            &observation.request
        )
        .unwrap(),
        observation
    );
}

#[tokio::test]
async fn failed_outbox_save_never_sends_a_renewed_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Keys::generate();
    let executor = Keys::generate();
    let (pending, observation) = observed(&owner, &executor);
    let relay = &observation.request.relay;
    let mut store = Store::open_dir(dir.path(), "owner", relay).unwrap();
    store
        .journal
        .received
        .insert(pending.command.id.to_hex(), pending);
    store.save().unwrap();
    store.path = dir.path().into(); // cannot atomically replace a directory
    let result = store
        .retry_receipts(
            |mut p| {
                let result =
                    prepare_receipt(&mut p, &owner, &executor, relay, &[], 1002).map(|()| p);
                std::future::ready(result)
            },
            |_| async { panic!("publication must not precede durable save") },
        )
        .await;
    assert!(result.is_err());
}
