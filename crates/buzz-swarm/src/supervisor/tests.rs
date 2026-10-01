use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_path(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("buzz-swarm-{tag}-{}-{n}", std::process::id()))
}

fn shell_plan(name: &str, script: &str, restart: Restart, max_restarts: u32) -> AgentPlan {
    AgentPlan {
        name: name.to_owned(),
        pubkey: "ab".repeat(32),
        relay_url: "wss://buzz.example.com".to_owned(),
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_owned(), script.to_owned()],
        workdir: std::env::temp_dir(),
        env: BTreeMap::new(),
        env_remove: Vec::new(),
        restart,
        max_restarts,
    }
}

async fn run_one(plan: AgentPlan, shutdown: watch::Receiver<u8>) -> (String, Outcome) {
    supervise(
        plan,
        shutdown,
        Options {
            grace: Duration::from_secs(2),
        },
    )
    .await
}

#[test]
fn restart_policy_covers_every_combination() {
    // (policy, success, strikes, max) -> restart?
    let cases = [
        (Restart::Never, true, 0, 10, false),
        (Restart::Never, false, 0, 10, false),
        (Restart::OnFailure, true, 0, 10, false),
        (Restart::OnFailure, false, 0, 10, true),
        (Restart::OnFailure, false, 9, 10, true),
        (Restart::OnFailure, false, 10, 10, false),
        (Restart::OnFailure, false, 0, 0, false),
        (Restart::Always, true, 0, 10, true),
        (Restart::Always, false, 0, 10, true),
        (Restart::Always, true, 10, 10, false),
    ];
    for (policy, success, strikes, max, wants_restart) in cases {
        let decision = decide(policy, success, strikes, max);
        assert_eq!(
            matches!(decision, Next::Restart(_)),
            wants_restart,
            "{policy:?} success={success} strikes={strikes} max={max}"
        );
    }
}

#[test]
fn backoff_grows_and_is_capped() {
    assert_eq!(backoff(0), Duration::from_secs(1));
    assert_eq!(backoff(1), Duration::from_secs(2));
    assert_eq!(backoff(5), Duration::from_secs(32));
    assert_eq!(backoff(6), BACKOFF_CAP);
    // No overflow panic for an absurd strike count.
    assert_eq!(backoff(4_000), BACKOFF_CAP);
}

#[tokio::test]
async fn a_clean_exit_is_not_restarted() {
    let marker = temp_path("clean");
    let script = format!("echo run >> {}; exit 0", marker.display());
    let (_tx, rx) = watch::channel(RUN);
    let (name, outcome) = run_one(shell_plan("clean", &script, Restart::OnFailure, 5), rx).await;

    assert_eq!(name, "clean");
    assert_eq!(outcome, Outcome::Completed);
    let runs = std::fs::read_to_string(&marker).expect("marker");
    assert_eq!(runs.lines().count(), 1, "a clean exit must not be retried");
    let _ = std::fs::remove_file(&marker);
}

#[tokio::test]
async fn failures_are_retried_up_to_the_budget_then_reported() {
    let marker = temp_path("fail");
    let script = format!("echo run >> {}; exit 3", marker.display());
    let (_tx, rx) = watch::channel(RUN);
    let (_, outcome) = run_one(shell_plan("fail", &script, Restart::OnFailure, 1), rx).await;

    assert_eq!(
        outcome,
        Outcome::GaveUp {
            status: "exit status: 3".to_owned()
        }
    );
    let runs = std::fs::read_to_string(&marker).expect("marker");
    assert_eq!(
        runs.lines().count(),
        2,
        "expected the original run plus one restart"
    );
    let _ = std::fs::remove_file(&marker);
}

#[tokio::test]
async fn shutdown_reaps_the_whole_process_group() {
    let pidfile = temp_path("group");
    // The shell backgrounds a grandchild in the same process group and waits.
    let script = format!("sleep 30 & echo $! > {}; wait", pidfile.display());
    let (tx, rx) = watch::channel(RUN);
    let supervised = tokio::spawn(run_one(
        shell_plan("group", &script, Restart::Always, 5),
        rx,
    ));

    let grandchild = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&pidfile) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    break pid;
                }
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("grandchild did not start");
    assert!(is_alive(grandchild), "grandchild should be running");

    tx.send(DRAIN).expect("signal shutdown");
    let (_, outcome) = tokio::time::timeout(Duration::from_secs(10), supervised)
        .await
        .expect("supervisor did not stop within the grace period")
        .expect("task");
    assert_eq!(
        outcome,
        Outcome::Stopped,
        "restart: always must not survive shutdown"
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while is_alive(grandchild) && Instant::now() < deadline {
        sleep(Duration::from_millis(20)).await;
    }
    assert!(!is_alive(grandchild), "grandchild was orphaned by shutdown");
    let _ = std::fs::remove_file(&pidfile);
}

#[tokio::test]
async fn a_leader_exit_cleans_up_its_descendants() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let pidfile = scratch.path().join("child.pid");
    let script = format!("sleep 30 & echo $! > '{}'; exit 0", pidfile.display());
    let (_tx, rx) = watch::channel(RUN);
    let (_, outcome) = tokio::time::timeout(
        Duration::from_secs(5),
        run_one(shell_plan("orphan", &script, Restart::Never, 0), rx),
    )
    .await
    .expect("supervisor hung after leader exited");
    assert_eq!(outcome, Outcome::Completed);
    let pid = std::fs::read_to_string(pidfile)
        .expect("pidfile")
        .trim()
        .parse()
        .expect("pid");
    let deadline = Instant::now() + Duration::from_secs(3);
    while is_alive(pid) && Instant::now() < deadline {
        sleep(Duration::from_millis(20)).await;
    }
    assert!(!is_alive(pid), "surviving descendant");
}

#[tokio::test]
async fn shutdown_kills_a_descendant_that_ignores_sigterm_after_its_leader_exits() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let pidfile = scratch.path().join("child.pid");
    let script = format!(
        "trap 'exit 0' TERM; (trap '' TERM; exec sleep 30) & echo $! > '{}'; wait",
        pidfile.display()
    );
    let (tx, rx) = watch::channel(RUN);
    let task = tokio::spawn(run_one(
        shell_plan("stubborn", &script, Restart::Never, 0),
        rx,
    ));
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(&pidfile) {
                if let Ok(pid) = value.trim().parse::<i32>() {
                    break pid;
                }
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("child did not start");
    tx.send(DRAIN).expect("signal");
    let (_, outcome) = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("hung")
        .expect("task");
    assert_eq!(outcome, Outcome::Stopped);
    let deadline = Instant::now() + Duration::from_secs(3);
    while is_alive(pid) && Instant::now() < deadline {
        sleep(Duration::from_millis(20)).await;
    }
    assert!(!is_alive(pid), "SIGTERM-ignoring descendant survived");
}

#[tokio::test]
async fn a_missing_binary_fails_the_agent_without_retrying() {
    let mut plan = shell_plan("ghost", "exit 0", Restart::Always, 5);
    plan.program = PathBuf::from("/nonexistent/buzz-acp");
    let (_tx, rx) = watch::channel(RUN);
    let (_, outcome) = run_one(plan, rx).await;
    assert!(matches!(outcome, Outcome::SpawnFailed { .. }));
}

#[tokio::test]
async fn agents_are_not_started_when_shutdown_arrives_first() {
    let (tx, rx) = watch::channel(RUN);
    tx.send(DRAIN).expect("shutdown");
    let marker = temp_path("never");
    let script = format!("echo run >> {}", marker.display());
    let (_, outcome) = run_one(shell_plan("never", &script, Restart::Always, 5), rx).await;
    assert_eq!(outcome, Outcome::Stopped);
    assert!(!marker.exists(), "no process should have been spawned");
}

fn is_alive(pid: i32) -> bool {
    Pid::from_raw(pid).is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

#[test]
fn summary_fails_only_on_real_failures() {
    let ok = Summary {
        outcomes: vec![
            ("a".into(), Outcome::Completed),
            ("b".into(), Outcome::Stopped),
        ],
    };
    assert!(!ok.failed());
    let bad = Summary {
        outcomes: vec![(
            "a".into(),
            Outcome::GaveUp {
                status: "with code 1".into(),
            },
        )],
    };
    assert!(bad.failed());
    let worse = Summary {
        outcomes: vec![(
            "a".into(),
            Outcome::SpawnFailed {
                error: "missing binary".into(),
            },
        )],
    };
    assert!(worse.failed());
}

async fn child_pid(path: &std::path::Path) -> i32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if let Ok(pid) = contents.trim().parse() {
                    return pid;
                }
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("child did not start")
}

async fn assert_stopped(pid: i32) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while is_alive(pid) {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("descendant survived cleanup");
}

#[tokio::test]
async fn cancellation_kills_the_owned_process_group() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let pidfile = scratch.path().join("child.pid");
    let script = format!("sleep 30 & echo $! > '{}'; wait", pidfile.display());
    let (_tx, rx) = watch::channel(RUN);
    let task = tokio::spawn(run_one(
        shell_plan("cancelled", &script, Restart::Never, 0),
        rx,
    ));
    let pid = child_pid(&pidfile).await;
    task.abort();
    assert!(task.await.expect_err("cancelled task").is_cancelled());
    assert_stopped(pid).await;
}

#[tokio::test]
async fn grace_expiry_and_second_signal_force_a_stubborn_group_to_exit() {
    for force in [false, true] {
        let scratch = tempfile::tempdir().expect("tempdir");
        let pidfile = scratch.path().join("child.pid");
        let script = format!(
            "trap '' TERM; sleep 30 & echo $! > '{}'; wait",
            pidfile.display()
        );
        let (tx, rx) = watch::channel(RUN);
        let task = tokio::spawn(supervise(
            shell_plan("stubborn", &script, Restart::Always, 5),
            rx,
            Options {
                grace: if force {
                    Duration::from_secs(30)
                } else {
                    Duration::from_millis(50)
                },
            },
        ));
        let pid = child_pid(&pidfile).await;
        tx.send(DRAIN).expect("request graceful stop");
        if force {
            tx.send(KILL).expect("force stop");
        }
        let (_, outcome) = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("group did not stop")
            .expect("supervisor task");
        assert_eq!(outcome, Outcome::Stopped);
        assert_stopped(pid).await;
    }
}
