//! Exercise the production request/worker/commit boundary with held I/O and a
//! real managed-section file. No sleeps, live relay, Tauri process or home writes.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn burst_coalesces_reads_and_commits_only_the_latest_workspace() {
    let gate = NestRegenGate::new();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(&file, "# User instructions\n").unwrap();
    let current = Mutex::new("wss://before.example");
    let reads = AtomicUsize::new(0);
    let active = AtomicUsize::new(0);
    let (started, mut starts) = mpsc::unbounded_channel();
    let worker = gate
        .request(|generation| {
            let relay = *current.lock().unwrap();
            let (release, released) = oneshot::channel();
            reads.fetch_add(1, Ordering::SeqCst);
            assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
            started.send((generation, release)).unwrap();
            let gate = &gate;
            let active = &active;
            let file = &file;
            async move {
                released.await.unwrap();
                let content = render_dynamic_section(&[], &[], &HashSet::new(), relay);
                gate.commit(file, &content, generation).unwrap();
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .unwrap();
    tokio::pin!(worker);
    let (first, release_first) = tokio::select! {
        value = starts.recv() => value.unwrap(),
        () = &mut worker => panic!("worker completed before its read"),
    };
    assert_eq!(first, 1);

    // Model startup backfill and a workspace switch while the first snapshot
    // read is held. New requests must not even start their expensive callback.
    *current.lock().unwrap() = "wss://latest.example";
    for _ in 0..289 {
        assert!(gate
            .request(|_| async { panic!("duplicate worker") })
            .is_none());
    }
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    release_first.send(()).unwrap();
    let (latest, release_latest) = tokio::select! {
        value = starts.recv() => value.unwrap(),
        () = &mut worker => panic!("latest request was lost"),
    };
    assert_eq!(latest, 290);
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert_eq!(fs::read_to_string(&file).unwrap(), "# User instructions\n");
    release_latest.send(()).unwrap();
    worker.await;

    let content = fs::read_to_string(&file).unwrap();
    assert!(content.starts_with("# User instructions"));
    assert!(content.contains("wss://latest.example"));
    assert!(!content.contains("wss://before.example"));
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert!(!gate.state.lock().unwrap().running);
}

#[tokio::test]
async fn trigger_during_follow_up_and_after_idle_is_not_lost() {
    let gate = NestRegenGate::new();
    let passes = AtomicUsize::new(0);
    gate.request(|generation| {
        let pass = passes.fetch_add(1, Ordering::SeqCst);
        if pass < 2 {
            assert!(gate
                .request(|_| async { panic!("second worker") })
                .is_none());
        }
        async move {
            assert_eq!(generation, pass as u64 + 1);
            Ok(())
        }
    })
    .unwrap()
    .await;
    assert_eq!(passes.load(Ordering::SeqCst), 3);
    assert!(!gate.state.lock().unwrap().running);

    // A request after idle must acquire ownership, not remain stranded dirty.
    gate.request(|generation| async move {
        assert_eq!(generation, 4);
        Ok(())
    })
    .unwrap()
    .await;
    assert!(!gate.state.lock().unwrap().running);
}

#[tokio::test]
async fn failed_pass_runs_pending_latest_and_releases_owner_for_next_trigger() {
    let gate = NestRegenGate::new();
    let passes = AtomicUsize::new(0);
    gate.request(|_| {
        if passes.fetch_add(1, Ordering::SeqCst) == 0 {
            assert!(gate
                .request(|_| async { panic!("second worker") })
                .is_none());
        }
        async { Err("controlled regeneration failure".into()) }
    })
    .unwrap()
    .await;
    assert_eq!(passes.load(Ordering::SeqCst), 2);
    assert!(!gate.state.lock().unwrap().running);
    gate.request(|_| async { Ok(()) }).unwrap().await;
    assert!(!gate.state.lock().unwrap().running);
}
