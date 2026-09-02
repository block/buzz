use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde_json::Value;

use buzz_relay::lifecycle::StartupPhase;

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
