//! Ordinary recovery uses the real persisted ledger, never clearing its fence.
use super::*;

fn start(operation: &str) -> Command {
    Command {
        v: 1,
        operation: operation.repeat(16),
        relay: "wss://community-b.example".into(),
        agent: nostr::Keys::generate().public_key().to_hex(),
        expires_at: nostr::Timestamp::now().as_secs() + 120,
        action: Action::Start {
            runtime: "buzz-agent".into(),
            revision: "ff".repeat(32),
        },
    }
}

#[test]
fn ordinary_rejected_start_recovers_after_reopen_without_losing_fence() {
    let dir = tempfile::tempdir().unwrap();
    let initial = start("aa");
    let mut stop = initial.clone();
    stop.operation = "bb".repeat(16);
    stop.action = Action::Stop {
        run: initial.operation.clone(),
    };
    let mut failed = initial.clone();
    failed.operation = "cc".repeat(16);
    let stopped_id = "22".repeat(32);
    let rejected_id = "33".repeat(32);
    {
        let mut ledger = Ledger::open(dir.path(), "placement").unwrap();
        ledger.begin(&"11".repeat(32), &initial).unwrap();
        ledger.finish(&initial.operation, Outcome::Spawned).unwrap();
        ledger.begin(&stopped_id, &stop).unwrap();
        ledger.finish(&stop.operation, Outcome::Stopped).unwrap();
        let admission = Admission::Local {
            predecessor: &stopped_id,
        };
        admission.validate_predecessor(&ledger, &failed).unwrap();
        ledger.begin(&rejected_id, &failed).unwrap();
        ledger.finish(&failed.operation, Outcome::Rejected).unwrap();
    }
    let mut ledger = Ledger::open(dir.path(), "placement").unwrap();
    assert!(
        ledger.is_fenced(),
        "automatic/legacy launch remains blocked"
    );
    let mut retry = failed.clone();
    retry.operation = "dd".repeat(16);
    retry.action = Action::Start {
        runtime: "buzz-agent".into(),
        revision: "ee".repeat(32),
    };
    assert!(Admission::Local {
        predecessor: &stopped_id
    }
    .validate_predecessor(&ledger, &retry)
    .is_err());
    let admission = Admission::Local {
        predecessor: &rejected_id,
    };
    admission.validate_predecessor(&ledger, &retry).unwrap();
    assert!(matches!(
        ledger.begin(&"44".repeat(32), &retry).unwrap(),
        Begin::Execute
    ));
    // A second callback captured before the first intent cannot create another
    // attempt even when both started from the same definite rejection.
    assert!(admission.validate_predecessor(&ledger, &retry).is_err());
    assert!(ledger.is_fenced());
    ledger.finish(&retry.operation, Outcome::Spawned).unwrap();
    assert_eq!(
        ledger
            .replay(&rejected_id, &failed)
            .unwrap()
            .unwrap()
            .outcome,
        Outcome::Rejected
    );
    assert!(local_start_predecessor(ledger.current().unwrap()).is_err());
}

#[test]
fn ordinary_recovery_never_relabels_uncertainty_as_rejection() {
    let request = start("aa");
    for outcome in [
        Outcome::Accepted,
        Outcome::Unknown,
        Outcome::RootExited,
        Outcome::Spawned,
        Outcome::Listening,
        Outcome::Ready,
    ] {
        let mut entry = Entry {
            command_id: "11".repeat(32),
            request: request.clone(),
            outcome: outcome.clone(),
            observed_at: 1,
        };
        assert!(local_start_predecessor(&entry).is_err(), "{outcome:?}");
        entry.request.action = Action::Stop {
            run: "bb".repeat(16),
        };
        assert!(local_start_predecessor(&entry).is_err(), "Stop {outcome:?}");
    }
}

#[test]
fn recovery_serializes_across_controllers_and_rechecks_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let first = start("aa");
    let id = "11".repeat(32);
    let mut ledger = Ledger::open(dir.path(), "placement").unwrap();
    ledger.begin(&id, &first).unwrap();
    ledger.finish(&first.operation, Outcome::Rejected).unwrap();
    let mut next = first.clone();
    next.operation = "bb".repeat(16);
    let admission = Admission::Local { predecessor: &id };
    admission.validate_predecessor(&ledger, &next).unwrap();
    let path = dir.path().to_owned();
    assert!(
        std::thread::spawn(move || Ledger::open(&path, "placement").is_err())
            .join()
            .unwrap()
    );
    ledger.begin(&"22".repeat(32), &next).unwrap();
    // Simulate a crash in the intent/spawn window, not a definite rejection.
    drop(ledger);
    let ledger = Ledger::open(dir.path(), "placement").unwrap();
    assert!(admission.validate_predecessor(&ledger, &next).is_err());
    assert!(local_start_predecessor(ledger.current().unwrap()).is_err());
    assert!(ledger.is_fenced());
    assert_eq!(
        ledger
            .replay(&"22".repeat(32), &next)
            .unwrap()
            .unwrap()
            .outcome,
        Outcome::Unknown
    );
}

#[test]
fn post_spawn_or_root_exit_cannot_be_reclassified_for_local_retry() {
    for outcome in [Outcome::Spawned, Outcome::Unknown] {
        let dir = tempfile::tempdir().unwrap();
        let request = start("aa");
        let mut ledger = Ledger::open(dir.path(), "placement").unwrap();
        ledger.begin(&"11".repeat(32), &request).unwrap();
        ledger.finish(&request.operation, outcome.clone()).unwrap();
        assert!(ledger
            .finish(&request.operation, Outcome::Rejected)
            .is_err());
        drop(ledger);
        let ledger = Ledger::open(dir.path(), "placement").unwrap();
        assert_eq!(ledger.current().unwrap().outcome, outcome);
        assert!(local_start_predecessor(ledger.current().unwrap()).is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let mut stop = start("aa");
    stop.action = Action::Stop {
        run: "bb".repeat(16),
    };
    let mut ledger = Ledger::open(dir.path(), "placement").unwrap();
    ledger.begin(&"11".repeat(32), &stop).unwrap();
    ledger.finish(&stop.operation, Outcome::RootExited).unwrap();
    assert!(ledger.finish(&stop.operation, Outcome::Rejected).is_err());
    drop(ledger);
    let ledger = Ledger::open(dir.path(), "placement").unwrap();
    assert!(local_start_predecessor(ledger.current().unwrap()).is_err());
}
