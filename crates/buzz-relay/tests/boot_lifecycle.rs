use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde_json::Value;

use buzz_relay::lifecycle::StartupPhase;
use buzz_relay::state::REDIS_BOOTSTRAP_FAILURE;

const VALID_RELAY_PRIVATE_KEY: &str =
    "0000000000000000000000000000000000000000000000000000000000000001";
const CHILD_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CAPTURE_BYTES: u64 = 1024 * 1024;

struct RelayProcess {
    child: Option<Child>,
    stdout: Option<JoinHandle<CapturedStream>>,
    stderr: Option<JoinHandle<CapturedStream>>,
    scratch_dir: std::path::PathBuf,
}

struct CapturedStream {
    retained: Vec<u8>,
    total_bytes: u64,
}

impl RelayProcess {
    fn spawn(environment: &[(&str, &str)]) -> Self {
        let scratch_dir =
            std::env::temp_dir().join(format!("buzz-boot-lifecycle-{}", uuid::Uuid::new_v4()));
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-relay"));
        command
            .env_clear()
            .env("RUST_BACKTRACE", "0")
            .env("RUST_LOG", "buzz_relay=info")
            .env("BUZZ_GIT_REPO_PATH", scratch_dir.join("repos"))
            .env("BUZZ_GIT_PACK_CACHE_PATH", scratch_dir.join("pack-cache"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in environment {
            command.env(name, value);
        }
        let mut child = command.spawn().expect("spawn buzz-relay child process");
        let stdout = child.stdout.take().expect("relay stdout pipe");
        let stderr = child.stderr.take().expect("relay stderr pipe");
        Self {
            child: Some(child),
            stdout: Some(thread::spawn(move || capture_stream(stdout))),
            stderr: Some(thread::spawn(move || capture_stream(stderr))),
            scratch_dir,
        }
    }

    fn try_wait(&mut self) -> Option<ExitStatus> {
        self.child
            .as_mut()
            .expect("relay child")
            .try_wait()
            .expect("poll relay child")
    }

    fn wait(mut self, timeout: Duration) -> Output {
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = self.try_wait() {
                break status;
            }
            if Instant::now() >= deadline {
                let child = self.child.as_mut().expect("relay child");
                let _ = child.kill();
                let _ = child.wait();
                panic!("buzz-relay child exceeded {timeout:?}");
            }
            thread::sleep(Duration::from_millis(10));
        };
        self.child.take();
        let output = Output {
            status,
            stdout: join_capture(self.stdout.take(), "stdout"),
            stderr: join_capture(self.stderr.take(), "stderr"),
        };
        let _ = std::fs::remove_dir_all(&self.scratch_dir);
        output
    }

    fn terminate(mut self) -> Output {
        self.child
            .as_mut()
            .expect("relay child")
            .kill()
            .expect("terminate exact relay child");
        self.wait(Duration::from_secs(2))
    }
}

impl Drop for RelayProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.scratch_dir);
    }
}

fn capture_stream(mut stream: impl std::io::Read) -> CapturedStream {
    let mut retained = Vec::new();
    let mut total_bytes = 0_u64;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = stream.read(&mut chunk).expect("read relay output pipe");
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(u64::try_from(read).expect("read size fits u64"));
        let remaining = usize::try_from(MAX_CAPTURE_BYTES)
            .expect("capture ceiling fits usize")
            .saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    CapturedStream {
        retained,
        total_bytes,
    }
}

fn join_capture(capture: Option<JoinHandle<CapturedStream>>, stream: &str) -> Vec<u8> {
    let capture = capture
        .expect("relay capture thread")
        .join()
        .expect("relay capture thread must not panic");
    assert!(
        capture.total_bytes <= MAX_CAPTURE_BYTES,
        "relay {stream} exceeded {MAX_CAPTURE_BYTES} bytes: {}",
        capture.total_bytes,
    );
    capture.retained
}

fn run_relay(environment: &[(&str, &str)]) -> Output {
    RelayProcess::spawn(environment).wait(CHILD_TIMEOUT)
}

fn scrape_metrics(port: u16) -> std::io::Result<String> {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(b"GET /metrics HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn wait_for_relay_metrics(process: &mut RelayProcess, port: u16) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        assert!(
            process.try_wait().is_none(),
            "relay exited before its metrics endpoint became usable"
        );
        if let Ok(response) = scrape_metrics(port) {
            if response.contains("buzz_audit_enabled") {
                return response;
            }
        }
        assert!(
            Instant::now() < deadline,
            "relay metrics did not become scrapeable within 8s"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn assert_no_startup_lifecycle_metrics(scrape: &str) {
    for line in scrape.lines() {
        let Some(name) = line
            .strip_prefix("# HELP ")
            .or_else(|| line.strip_prefix("# TYPE "))
            .and_then(|rest| rest.split_ascii_whitespace().next())
        else {
            continue;
        };
        assert!(
            !["startup", "boot", "lifecycle"]
                .iter()
                .any(|term| name.contains(term))
                && !StartupPhase::ALL
                    .iter()
                    .any(|phase| name.contains(phase.as_str())),
            "logs-only lifecycle contract emitted metric family {name}"
        );
    }
}

fn lifecycle_events(output: &Output) -> Vec<Value> {
    let mut events: Vec<Value> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .chain(output.stderr.split(|byte| *byte == b'\n'))
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(|event| event["event_name"] == "buzz_process_lifecycle")
        .collect();
    events.sort_by_key(|event| event["sequence"].as_u64());
    events
}

fn lifecycle_events_from(bytes: &[u8]) -> Vec<Value> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(|event| event["event_name"] == "buzz_process_lifecycle")
        .collect()
}

fn assert_accounting(events: &[Value]) {
    assert!(!events.is_empty(), "child emitted no lifecycle events");
    let boot_id = events[0]["process_boot_id"]
        .as_str()
        .expect("process_boot_id");
    let mut counts = BTreeMap::<String, (usize, usize)>::new();
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["schema_version"], 1);
        assert_eq!(event["sequence"], u64::try_from(index + 1).unwrap());
        assert_eq!(event["process_boot_id"], boot_id);
        assert_eq!(event["track"], "startup");
        let count = counts
            .entry(event["phase"].as_str().expect("phase").to_owned())
            .or_default();
        match event["edge"].as_str() {
            Some("started") => count.0 += 1,
            Some("terminal") => count.1 += 1,
            other => panic!("unexpected lifecycle edge: {other:?}"),
        }
    }
    assert!(
        counts
            .values()
            .all(|(started, terminal)| *started == 1 && *terminal == 1),
        "every started phase must have one terminal: {counts:?}"
    );
}

fn assert_terminal(events: &[Value], phase: &str, status: &str, reason: Option<&str>) {
    let terminal = events
        .iter()
        .find(|event| event["phase"] == phase && event["edge"] == "terminal")
        .unwrap_or_else(|| panic!("missing {phase} terminal"));
    assert_eq!(terminal["status"], status);
    match reason {
        Some(reason) => assert_eq!(terminal["reason"], reason),
        None => assert!(terminal["reason"].is_null()),
    }
}

fn phases(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .filter(|event| event["edge"] == "started")
        .map(|event| event["phase"].as_str().expect("phase"))
        .collect()
}

#[test]
fn invalid_config_terminalizes_at_main_even_with_logs_disabled() {
    let output = run_relay(&[
        ("RUST_LOG", "off"),
        ("BUZZ_BIND_ADDR", "not-a-socket-address"),
    ]);
    assert!(!output.status.success());
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_eq!(
        phases(&events),
        [
            "process_telemetry",
            "crypto_init",
            "tracing_init",
            "config_load"
        ]
    );
    assert_terminal(&events, "config_load", "failed", Some("config_invalid"));
    assert_terminal(
        &events,
        "process_telemetry",
        "failed",
        Some("config_invalid"),
    );
    assert_eq!(lifecycle_events_from(&output.stderr), events);
    assert!(lifecycle_events_from(&output.stdout).is_empty());
}

#[test]
#[cfg(unix)]
fn config_filesystem_failure_has_a_bounded_terminal() {
    let output = run_relay(&[
        ("RUST_LOG", "off"),
        ("BUZZ_GIT_REPO_PATH", "/dev/null/not-a-directory"),
    ]);
    assert!(!output.status.success());
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_terminal(&events, "config_load", "failed", Some("config_invalid"));
    assert_terminal(
        &events,
        "process_telemetry",
        "failed",
        Some("config_invalid"),
    );
}

#[test]
fn invalid_config_value_has_the_same_bounded_terminal() {
    let output = run_relay(&[("RUST_LOG", "off"), ("BUZZ_DRAIN_JITTER_MS", "bogus")]);
    assert!(!output.status.success());
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_terminal(&events, "config_load", "failed", Some("config_invalid"));
    assert_terminal(
        &events,
        "process_telemetry",
        "failed",
        Some("config_invalid"),
    );
}

#[test]
fn configured_otlp_terminalizes_tracing_before_a_later_failure() {
    let output = run_relay(&[
        ("RUST_LOG", "off"),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4317"),
        ("BUZZ_BIND_ADDR", "not-a-socket-address"),
    ]);
    assert!(!output.status.success());
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_terminal(&events, "tracing_init", "succeeded", None);
    assert_terminal(&events, "config_load", "failed", Some("config_invalid"));
}

#[test]
fn missing_key_stops_before_metrics_bind() {
    let output = run_relay(&[]);
    assert!(!output.status.success());
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_eq!(
        phases(&events),
        [
            "process_telemetry",
            "crypto_init",
            "tracing_init",
            "config_load",
            "key_load"
        ]
    );
    assert_terminal(&events, "key_load", "failed", Some("missing"));
    assert_terminal(&events, "process_telemetry", "failed", Some("missing"));
}

#[test]
fn invalid_key_uses_a_bounded_reason_without_leaking_the_value() {
    let secret = "private-key-material-that-must-not-appear";
    let output = run_relay(&[("BUZZ_RELAY_PRIVATE_KEY", secret)]);
    assert!(!output.status.success());
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_terminal(&events, "key_load", "failed", Some("required_invalid"));
    let combined = [output.stdout, output.stderr].concat();
    assert!(!String::from_utf8_lossy(&combined).contains(secret));
}

#[test]
fn occupied_metrics_port_has_a_typed_bind_terminal() {
    let occupied = TcpListener::bind(("0.0.0.0", 0)).expect("bind occupied port");
    let port = occupied.local_addr().expect("occupied address").port();
    let port = port.to_string();
    let output = run_relay(&[
        ("BUZZ_RELAY_PRIVATE_KEY", VALID_RELAY_PRIVATE_KEY),
        ("BUZZ_METRICS_PORT", &port),
    ]);
    assert!(!output.status.success());
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_terminal(&events, "metrics_bind", "failed", Some("bind"));
    assert_terminal(&events, "process_telemetry", "failed", Some("bind"));
}

#[test]
fn otlp_build_failure_is_degraded_without_leaking_endpoint_credentials() {
    let secret = "telemetry-secret-marker";
    let endpoint = format!("https://telemetry-user:{secret}@[");
    let fake_database = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake database");
    let database_url = format!(
        "postgres://buzz@127.0.0.1:{}/buzz",
        fake_database.local_addr().expect("database address").port()
    );
    let reserved = TcpListener::bind(("127.0.0.1", 0)).expect("reserve metrics port");
    let port = reserved.local_addr().expect("metrics address").port();
    drop(reserved);
    let port_value = port.to_string();
    let mut process = RelayProcess::spawn(&[
        ("OTEL_EXPORTER_OTLP_ENDPOINT", &endpoint),
        ("RUST_LOG", "buzz_relay=warn"),
        ("BUZZ_RELAY_PRIVATE_KEY", VALID_RELAY_PRIVATE_KEY),
        ("BUZZ_METRICS_PORT", &port_value),
        ("DATABASE_URL", &database_url),
    ]);
    let scrape = wait_for_relay_metrics(&mut process, port);
    assert_no_startup_lifecycle_metrics(&scrape);
    let output = process.terminate();
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_terminal(&events, "tracing_init", "degraded", Some("exporter_build"));
    assert_terminal(
        &events,
        "process_telemetry",
        "degraded",
        Some("exporter_build"),
    );
    let combined = [output.stdout, output.stderr].concat();
    assert!(!String::from_utf8_lossy(&combined).contains(secret));
}

#[test]
fn successful_main_emits_complete_lifecycle_without_startup_metrics() {
    let fake_database = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake database");
    let database_url = format!(
        "postgres://buzz@127.0.0.1:{}/buzz",
        fake_database.local_addr().expect("database address").port()
    );
    let reserved = TcpListener::bind(("127.0.0.1", 0)).expect("reserve metrics port");
    let port = reserved.local_addr().expect("metrics address").port();
    drop(reserved);
    let port_value = port.to_string();
    let mut process = RelayProcess::spawn(&[
        ("RUST_LOG", "off"),
        ("BUZZ_RELAY_PRIVATE_KEY", VALID_RELAY_PRIVATE_KEY),
        ("BUZZ_METRICS_PORT", &port_value),
        ("DATABASE_URL", &database_url),
    ]);
    let scrape = wait_for_relay_metrics(&mut process, port);
    assert_no_startup_lifecycle_metrics(&scrape);

    let output = process.terminate();
    let events = lifecycle_events(&output);
    assert_accounting(&events);
    assert_terminal(&events, "crypto_init", "succeeded", None);
    assert_terminal(&events, "tracing_init", "succeeded", None);
    assert_terminal(&events, "config_load", "succeeded", None);
    assert_terminal(&events, "key_load", "succeeded", None);
    assert_terminal(&events, "metrics_bind", "succeeded", None);
    assert_terminal(&events, "process_telemetry", "succeeded", None);
}

/// Boot gates that need a live Postgres to reach the code under test. Named
/// `postgres_tests` so `.config/nextest.toml`'s `postgres-ci` default filter
/// discovers them structurally; the wrapper hands each test its own database
/// through `DATABASE_URL`.
mod postgres_tests {
    use super::*;

    const REDIS_BOOTSTRAP_BUDGET: Duration = Duration::from_secs(5);
    const REDIS_BOOTSTRAP_SCHEDULING_SLACK: Duration = Duration::from_secs(3);

    /// A TCP peer that completes Redis's metadata handshake, then reads and
    /// holds PING without replying. This distinguishes a bounded checkout +
    /// PING from a refused connection, which returns before the bootstrap
    /// timeout is exercised.
    struct HangingRedisPeer {
        redis_url: String,
        request_received: mpsc::Receiver<Vec<u8>>,
        stop: mpsc::Sender<()>,
        worker: Option<JoinHandle<()>>,
    }

    impl HangingRedisPeer {
        fn spawn() -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake Redis peer");
            listener
                .set_nonblocking(true)
                .expect("set fake Redis listener nonblocking");
            let port = listener.local_addr().expect("fake Redis address").port();
            let (request_tx, request_received) = mpsc::channel();
            let (stop, stop_rx) = mpsc::channel();
            let worker = thread::spawn(move || loop {
                if stop_rx.try_recv().is_ok() {
                    return;
                }
                let (mut stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("accept fake Redis connection: {error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .expect("bound fake Redis read");
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 4096];
                    match stream.read(&mut chunk) {
                        Ok(0) => return,
                        Ok(read) => {
                            request.extend_from_slice(&chunk[..read]);
                            assert!(
                                request.len() <= 4096,
                                "fake Redis peer received an oversized request"
                            );
                            let setinfo_commands = request
                                .windows(b"SETINFO".len())
                                .filter(|window| *window == b"SETINFO")
                                .count();
                            if setinfo_commands >= 2 {
                                // redis-rs pipelines CLIENT SETINFO lib-name
                                // and lib-ver while establishing a connection.
                                // Complete that handshake so the relay reaches
                                // its explicit bootstrap PING, then hold it.
                                stream
                                    .write_all(b"+OK\r\n+OK\r\n")
                                    .expect("reply to Redis client handshake");
                                request.clear();
                                continue;
                            }
                            if request
                                .windows(b"PING".len())
                                .any(|window| window == b"PING")
                            {
                                let _ = request_tx.send(request);
                                let _ = stop_rx.recv_timeout(CHILD_TIMEOUT);
                                return;
                            }
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            if stop_rx.try_recv().is_ok() {
                                return;
                            }
                        }
                        Err(error) => panic!("read fake Redis request: {error}"),
                    }
                }
            });
            Self {
                redis_url: format!("redis://127.0.0.1:{port}"),
                request_received,
                stop,
                worker: Some(worker),
            }
        }

        fn redis_url(&self) -> &str {
            &self.redis_url
        }

        fn assert_request_received(&self, logs: &str) {
            let request = self
                .request_received
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or_else(|_| panic!("relay must reach the fake Redis peer: {logs}"));
            assert!(!request.is_empty(), "fake Redis peer read an empty request");
        }
    }

    impl Drop for HangingRedisPeer {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            if let Some(worker) = self.worker.take() {
                worker.join().expect("fake Redis peer must not panic");
            }
        }
    }

    fn reserve_closed_port() -> u16 {
        let reserved = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
        let port = reserved.local_addr().expect("reserved address").port();
        drop(reserved);
        port
    }

    /// Runs the relay until it exits on its own, or kills it once `timeout`
    /// passes. Unlike `run_relay`, a relay that keeps serving is a result to
    /// assert on rather than a panic, which is the whole point here.
    fn run_until_exit(environment: &[(&str, &str)], timeout: Duration) -> (bool, Output) {
        let mut process = RelayProcess::spawn(environment);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if process.try_wait().is_some() {
                return (true, process.wait(Duration::from_secs(2)));
            }
            thread::sleep(Duration::from_millis(20));
        }
        (false, process.terminate())
    }

    /// Redis is required for pub/sub fan-out, presence, and typing, but nothing
    /// in boot ever opened a command connection: `deadpool_redis` pools dial
    /// lazily and `PubSubManager::new` only allocates channels, so "Redis
    /// pub/sub connected" was logged against a dead port. A relay could
    /// therefore boot with Redis unreachable, bind its health listener, and —
    /// now that readiness answers from local lifecycle alone — advertise ready
    /// for the rest of its life. The bootstrap gate is the one-time proof that
    /// the command path has connected at least once, and it has to land before
    /// the listener binds, because binding is the one-way latch that makes this
    /// pod routable.
    ///
    /// The git conformance probe is disabled so the only remaining startup-fatal
    /// gate is the one under test.
    #[test]
    #[ignore = "requires PostgreSQL"]
    fn unreachable_redis_fails_boot_before_the_health_listener_binds() {
        let database_url = std::env::var("DATABASE_URL")
            .expect("postgres lane provides DATABASE_URL for each test process");
        let redis_url = format!("redis://127.0.0.1:{}", reserve_closed_port());
        let metrics_port = reserve_closed_port().to_string();
        let health_port = reserve_closed_port();
        let health_port_value = health_port.to_string();

        let (exited, output) = run_until_exit(
            &[
                ("BUZZ_RELAY_PRIVATE_KEY", VALID_RELAY_PRIVATE_KEY),
                ("BUZZ_METRICS_PORT", &metrics_port),
                ("BUZZ_HEALTH_PORT", &health_port_value),
                ("DATABASE_URL", &database_url),
                ("REDIS_URL", &redis_url),
                ("BUZZ_GIT_CONFORMANCE_PROBE", "false"),
            ],
            CHILD_TIMEOUT,
        );
        let logs = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        assert!(
            !logs.contains("Health probe listener started"),
            "the Redis bootstrap gate must run before the health listener binds: {logs}"
        );
        assert!(
            exited && !output.status.success(),
            "an unreachable Redis command path must be startup-fatal: {logs}"
        );
        assert!(
            logs.contains(REDIS_BOOTSTRAP_FAILURE),
            "the failure must name the gate that rejected boot: {logs}"
        );
        assert!(
            TcpListener::bind(("0.0.0.0", health_port)).is_ok(),
            "the health port must never have been bound"
        );
    }

    /// A peer that accepts the socket but withholds its Redis response exercises
    /// the outer timeout around both lazy pool checkout and PING. Removing or
    /// narrowing that timeout makes this test kill a still-running relay at the
    /// deadline instead of observing a startup failure.
    #[test]
    #[ignore = "requires PostgreSQL"]
    fn hanging_redis_peer_times_out_before_the_health_listener_binds() {
        let database_url = std::env::var("DATABASE_URL")
            .expect("postgres lane provides DATABASE_URL for each test process");
        let redis_peer = HangingRedisPeer::spawn();
        let metrics_port = reserve_closed_port().to_string();
        let health_port = reserve_closed_port();
        let health_port_value = health_port.to_string();
        let timeout = REDIS_BOOTSTRAP_BUDGET + REDIS_BOOTSTRAP_SCHEDULING_SLACK;
        let started_at = Instant::now();

        let (exited, output) = run_until_exit(
            &[
                ("BUZZ_RELAY_PRIVATE_KEY", VALID_RELAY_PRIVATE_KEY),
                ("BUZZ_METRICS_PORT", &metrics_port),
                ("BUZZ_HEALTH_PORT", &health_port_value),
                ("DATABASE_URL", &database_url),
                ("REDIS_URL", redis_peer.redis_url()),
                ("BUZZ_GIT_CONFORMANCE_PROBE", "false"),
            ],
            timeout,
        );
        let elapsed = started_at.elapsed();
        let logs = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        redis_peer.assert_request_received(&logs);

        assert!(
            elapsed >= REDIS_BOOTSTRAP_BUDGET,
            "the fake peer must hold the request through the bootstrap budget: {elapsed:?}: {logs}"
        );
        assert!(
            exited && elapsed < timeout && !output.status.success(),
            "the outer bootstrap timeout must terminate the relay within scheduling slack: {elapsed:?}: {logs}"
        );
        assert!(
            logs.contains(REDIS_BOOTSTRAP_FAILURE),
            "the timeout must report the bounded Redis bootstrap failure: {logs}"
        );
        assert!(
            !logs.contains("Health probe listener started"),
            "the health listener must not bind before Redis bootstrap succeeds: {logs}"
        );
        assert!(
            TcpListener::bind(("0.0.0.0", health_port)).is_ok(),
            "the health port must never have been bound"
        );
    }
}
