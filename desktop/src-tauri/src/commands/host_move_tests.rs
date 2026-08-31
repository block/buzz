use super::*;
use buzz_core_pkg::host_execution::Receipt;

struct Fixture {
    dir: tempfile::TempDir,
    owner: Keys,
    source: Keys,
    destination: Keys,
    store: Store,
    id: String,
}
const RELAY: &str = "wss://relay.example";
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let owner = Keys::generate();
        let source = Keys::generate();
        let destination = Keys::generate();
        let mut store = Store::open_dir(dir.path(), &owner.public_key().to_hex(), RELAY).unwrap();
        let agent = Keys::generate().public_key().to_hex();
        let stop_req = Command {
            v: 1,
            operation: "ab".repeat(16),
            relay: RELAY.into(),
            agent: agent.clone(),
            expires_at: 400,
            action: Action::Stop {
                run: "cd".repeat(16),
            },
        };
        let start_req = Command {
            v: 1,
            operation: "ef".repeat(16),
            relay: RELAY.into(),
            agent,
            expires_at: 400,
            action: Action::Start {
                runtime: "buzz-agent".into(),
                revision: "aa".repeat(32),
            },
        };
        let stop = pending(
            &owner,
            host::registration(&owner, source.public_key(), 100).unwrap(),
            &stop_req,
            100,
        )
        .unwrap();
        let destination_pending = pending(
            &owner,
            host::registration(&owner, destination.public_key(), 100).unwrap(),
            &start_req,
            100,
        )
        .unwrap();
        let stop_id = stop.command.id.to_hex();
        let intent = MoveIntent {
            authorization: owner
                .sign_schnorr(&authorization(&stop_id, &destination_pending))
                .to_string(),
            stop: stop_id.clone(),
            destination: destination_pending,
            start: None,
            error: None,
        };
        store.journal.sent.insert(stop_id, stop);
        store
            .journal
            .moves
            .insert(stop_req.operation.clone(), intent);
        Self {
            dir,
            owner,
            source,
            destination,
            store,
            id: stop_req.operation,
        }
    }
    fn receipt(&mut self, result: Outcome) {
        let stop_id = self.store.journal.moves[&self.id].stop.clone();
        let stop = self.store.journal.sent.get_mut(&stop_id).unwrap();
        let req = request(&self.owner, stop, RELAY).unwrap();
        let receipt = Receipt {
            v: 1,
            command: stop_id,
            run: req.run().into(),
            request: req,
            observed_at: 110,
            outcome: result,
        };
        stop.receipt =
            Some(host_execution::receipt(&self.source, &stop.registration, &receipt, 111).unwrap());
    }
    fn release(&mut self) -> Result<(), String> {
        let intent = self.store.journal.moves[&self.id].clone();
        release_to_outbox(&mut self.store, &self.owner, RELAY, &self.id, &intent, 500)
    }
}

#[test]
fn only_authoritative_stopped_releases_same_agent_new_generation() {
    for result in [
        None,
        Some(Outcome::Accepted),
        Some(Outcome::RootExited),
        Some(Outcome::Unknown),
        Some(Outcome::Rejected),
    ] {
        let mut f = Fixture::new();
        if let Some(result) = result {
            f.receipt(result);
        }
        super::super::host_start::validate_journal(&f.store, &f.owner, RELAY).unwrap();
        assert!(f.release().is_err());
        assert_eq!(f.store.journal.sent.len(), 1);
        assert_eq!(
            progress(&f.store, &f.owner, RELAY).unwrap()[0].status,
            "stop_unconfirmed"
        );
    }
    let mut f = Fixture::new();
    f.receipt(Outcome::Stopped);
    f.release().unwrap();
    let intent = &f.store.journal.moves[&f.id];
    let src = request(&f.owner, &f.store.journal.sent[&intent.stop], RELAY).unwrap();
    let dst = request(
        &f.owner,
        &f.store.journal.sent[intent.start.as_ref().unwrap()],
        RELAY,
    )
    .unwrap();
    assert_eq!(src.agent, dst.agent);
    assert_ne!(src.run(), dst.run());
    assert_eq!(
        dst.expires_at, 800,
        "TTL begins at release, not Stop queue time"
    );
    assert_eq!(f.store.journal.sent.len(), 2);
    assert!(
        f.release().is_err(),
        "second release never changes command bytes"
    );
    super::super::host_start::validate_journal(&f.store, &f.owner, RELAY).unwrap();
}

#[test]
fn restart_ack_loss_and_peer_survive_without_duplicate_or_global_fence() {
    let mut f = Fixture::new();
    let template = f.store.journal.moves[&f.id].destination.clone();
    let req = request(&f.owner, &template, RELAY).unwrap();
    assert!(check_reservation(
        &f.store,
        &f.owner,
        RELAY,
        &req.agent,
        f.destination.public_key()
    )
    .is_err());
    assert!(
        check_reservation(&f.store, &f.owner, RELAY, &req.agent, f.source.public_key()).is_ok()
    );
    let mut peer_req = req.clone();
    peer_req.agent = Keys::generate().public_key().to_hex();
    peer_req.operation = "12".repeat(16);
    let peer = pending(&f.owner, template.registration.clone(), &peer_req, 100).unwrap();
    let peer_id = peer.command.id.to_hex();
    let peer_bytes = peer.command.as_json();
    f.store.journal.sent.insert(peer_id.clone(), peer);
    f.store.save().unwrap();
    drop(f.store);
    f.store = Store::open_dir(f.dir.path(), &f.owner.public_key().to_hex(), RELAY).unwrap();
    assert!(f.release().is_err());
    f.receipt(Outcome::Stopped);
    f.release().unwrap();
    f.store.save().unwrap();
    let start_id = f.store.journal.moves[&f.id].start.clone().unwrap();
    let bytes = f.store.journal.sent[&start_id].command.as_json();
    drop(f.store);
    f.store = Store::open_dir(f.dir.path(), &f.owner.public_key().to_hex(), RELAY).unwrap();
    assert!(f.release().is_err());
    assert_eq!(f.store.journal.sent[&start_id].command.as_json(), bytes);
    assert_eq!(f.store.journal.sent[&peer_id].command.as_json(), peer_bytes);
    assert_eq!(f.store.journal.sent.len(), 3);
}

#[test]
fn stale_receipt_swapped_destination_wrong_scope_and_corrupt_release_fail_closed() {
    let mut f = Fixture::new();
    f.receipt(Outcome::Stopped);
    let intent = f.store.journal.moves[&f.id].clone();
    let stopped = f.store.journal.sent[&intent.stop].receipt.clone().unwrap();
    let other = Fixture::new();
    f.store.journal.sent.get_mut(&intent.stop).unwrap().receipt = other.store.journal.sent
        [&other.store.journal.moves[&other.id].stop]
        .receipt
        .clone();
    assert!(f.release().is_err());
    f.store.journal.sent.get_mut(&intent.stop).unwrap().receipt = Some(stopped);
    assert!(validate_moves(&f.store, &f.owner, "wss://foreign.example").is_err());
    assert!(validate_moves(&f.store, &Keys::generate(), RELAY).is_err());
    f.store.journal.moves.get_mut(&f.id).unwrap().destination =
        other.store.journal.moves[&other.id].destination.clone();
    assert!(validate_moves(&f.store, &f.owner, RELAY).is_err());
    f.store.journal.moves.insert(f.id.clone(), intent);
    f.release().unwrap();
    f.store
        .journal
        .sent
        .get_mut(&f.store.journal.moves[&f.id].stop)
        .unwrap()
        .receipt = None;
    assert!(validate_moves(&f.store, &f.owner, RELAY).is_err());
}

#[test]
fn destination_rejection_reports_source_stopped_never_moved_or_restarted() {
    let mut f = Fixture::new();
    f.receipt(Outcome::Stopped);
    f.release().unwrap();
    let start_id = f.store.journal.moves[&f.id].start.clone().unwrap();
    let start = f.store.journal.sent.get_mut(&start_id).unwrap();
    let req = request(&f.owner, start, RELAY).unwrap();
    let receipt = Receipt {
        v: 1,
        command: start_id,
        run: req.run().into(),
        request: req,
        observed_at: 501,
        outcome: Outcome::Rejected,
    };
    start.receipt =
        Some(host_execution::receipt(&f.destination, &start.registration, &receipt, 502).unwrap());
    assert_eq!(
        progress(&f.store, &f.owner, RELAY).unwrap()[0].status,
        "stopped_start_rejected"
    );
    assert_eq!(f.store.journal.sent.len(), 2);
    super::super::host_start::validate_journal(&f.store, &f.owner, RELAY).unwrap();
}

#[test]
fn renewed_stop_receipt_preserves_observation_and_releases_move_once() {
    let mut f = Fixture::new();
    f.receipt(Outcome::Stopped);
    let intent = f.store.journal.moves[&f.id].clone();
    let stop = f.store.journal.sent.get_mut(&intent.stop).unwrap();
    let command = stop.command.clone();
    let original = stop.receipt.clone().unwrap();
    super::super::host_start_store::prepare_receipt(stop, &f.owner, &f.source, RELAY, &[], 1200)
        .unwrap();
    let renewed = stop.receipt.as_ref().unwrap();
    assert_ne!(renewed.id, original.id);
    assert_eq!(renewed.content, original.content);
    assert_eq!(renewed.tags, original.tags);
    assert_eq!(stop.command, command);
    let req = request(&f.owner, stop, RELAY).unwrap();
    let observation =
        host_execution::decrypt_receipt(&f.owner, &stop.registration, renewed, &command, &req)
            .unwrap();
    assert_eq!(observation.observed_at, 110);
    assert_eq!(observation.outcome, Outcome::Stopped);
    f.store.save().unwrap();
    drop(f.store);
    f.store = Store::open_dir(f.dir.path(), &f.owner.public_key().to_hex(), RELAY).unwrap();
    release_to_outbox(&mut f.store, &f.owner, RELAY, &f.id, &intent, 1201).unwrap();
    let released = f.store.journal.moves[&f.id].clone();
    assert!(release_to_outbox(&mut f.store, &f.owner, RELAY, &f.id, &released, 1202).is_err());
    assert_eq!(f.store.journal.sent.len(), 2);
    super::super::host_start::validate_journal(&f.store, &f.owner, RELAY).unwrap();
}
