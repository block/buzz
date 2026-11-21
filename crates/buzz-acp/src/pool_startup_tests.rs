//! Exercise pool admission with real ACP subprocesses. No relay or provider.
use super::*;

#[tokio::test]
async fn first_ready_worker_serves_before_the_remaining_workers_start() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("adapter.py");
    let launches = temp.path().join("launches");
    std::fs::write(
        &script,
        r#"import json, os, sys
with open(os.environ['LAUNCHES'], 'a') as file:
    file.write('started\n')
for line in sys.stdin:
    message = json.loads(line)
    if message.get('method') == 'initialize':
        print(json.dumps({'jsonrpc': '2.0', 'id': message['id'],
                          'result': {'protocolVersion': 1,
                                     'agentInfo': {'name': 'startup-probe'}}}), flush=True)
"#,
    )
    .unwrap();
    let startup = PoolStartup {
        agents: 3,
        command: "python3".into(),
        args: vec![script.to_str().unwrap().into()],
        extra_env: vec![("LAUNCHES".into(), launches.to_str().unwrap().into())],
        has_generated_codex_config: false,
        model: None,
        effort_level: None,
        observer: None,
    };
    let mut pool = initialize_agent_pool(&startup, None).await.unwrap();
    let started = std::fs::read_to_string(&launches).unwrap();
    let live = pool.live_count();
    let ready = pool.any_idle();
    let remaining = (!pool.slot_alive(1), !pool.slot_alive(2));
    shutdown_agent_pool(&mut pool).await;
    assert_eq!(started.lines().count(), 1);
    assert_eq!(live, 1);
    assert!(ready, "the returned worker must be available for dispatch");
    assert_eq!(
        remaining,
        (true, true),
        "startup preserves the slots for background refill"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn deferred_initialize_deadline_reaps_worker_and_restores_retry_capacity() {
    use nix::errno::Errno;
    use nix::sys::wait::{waitpid, WaitPidFlag};
    use nix::unistd::Pid;

    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("adapter.py");
    let launches = temp.path().join("launches");
    let initializing = temp.path().join("initializing");
    std::fs::write(
        &script,
        r#"import json, os, sys, time
with open(os.environ['LAUNCHES'], 'a') as file:
    file.write(str(os.getpid()) + '\n')
with open(os.environ['LAUNCHES']) as file:
    hang = len(file.readlines()) == 2
for line in sys.stdin:
    message = json.loads(line)
    if message.get('method') == 'initialize':
        if hang:
            with open(os.environ['INITIALIZING'], 'w') as file:
                file.write(str(os.getpid()))
            while True:
                print(json.dumps({'jsonrpc': '2.0', 'method': 'fixture/progress'}), flush=True)
                time.sleep(0.01)
        print(json.dumps({'jsonrpc': '2.0', 'id': message['id'],
                          'result': {'protocolVersion': 1,
                                     'agentInfo': {'name': 'startup-probe'}}}), flush=True)
"#,
    )
    .unwrap();
    let startup = PoolStartup {
        agents: 2,
        command: "python3".into(),
        args: vec![script.to_str().unwrap().into()],
        extra_env: vec![
            ("LAUNCHES".into(), launches.to_str().unwrap().into()),
            ("INITIALIZING".into(), initializing.to_str().unwrap().into()),
        ],
        has_generated_codex_config: false,
        model: None,
        effort_level: None,
        observer: None,
    };
    let mut pool = initialize_agent_pool(&startup, None).await.unwrap();
    assert_eq!(pool.live_count(), 1);
    let mut circuits: Vec<_> = (0..2)
        .map(|_| SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        })
        .collect();
    let config = build_mcp_servers_tests::test_config();
    let (tx, mut rx) = mpsc::channel(2);
    let mut tasks = tokio::task::JoinSet::new();
    for should_succeed in [false, true] {
        assert!(circuits[1].can_refill());
        circuits[1].respawn_in_flight = true;
        let guard = RespawnGuard::new(1, tx.clone());
        let command = startup.command.clone();
        let args = startup.args.clone();
        let env = startup.extra_env.clone();
        tasks.spawn(async move {
            guard.send(
                spawn_and_init(
                    &command,
                    &args,
                    &env,
                    false,
                    1,
                    None,
                    Duration::from_secs(1),
                )
                .await,
            );
        });
        let wake = tokio::time::timeout(
            Duration::from_secs(5),
            recovery_wake::wait(&mut rx, None, None),
        )
        .await;
        if wake.is_err() {
            tasks.shutdown().await;
            shutdown_agent_pool(&mut pool).await;
        }
        let recovery_wake::RecoveryWake::Respawn(result) =
            wake.expect("deferred initialize exceeded the injected deadline")
        else {
            panic!("expected a respawn result");
        };
        assert_eq!(result.result.is_ok(), should_succeed);
        if !should_succeed {
            let error = result.result.as_ref().err().unwrap().to_string();
            assert!(error.contains("Request timeout"), "{error}");
            let pid: i32 = std::fs::read_to_string(&initializing)
                .expect("the worker must reach initialize before timing out")
                .parse()
                .unwrap();
            assert_eq!(
                waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)),
                Err(Errno::ECHILD),
                "the timeout must reap the worker before returning its slot"
            );
        }
        handle_respawn_result(*result, &mut circuits, &mut pool, &config);
        assert!(!circuits[1].respawn_in_flight);
        assert_eq!(pool.slot_alive(1), should_succeed);
        if !should_succeed {
            assert!(!circuits[1].can_refill(), "failure retains the cooldown");
            // Elapse the existing circuit cooldown without waiting five minutes.
            circuits[1].open_until = Some(std::time::Instant::now());
        }
        tasks.join_next().await.unwrap().unwrap();
    }
    assert_eq!(pool.live_count(), 2);
    assert!(pool.any_idle(), "the recovered slot is available for work");
    shutdown_agent_pool(&mut pool).await;
}
