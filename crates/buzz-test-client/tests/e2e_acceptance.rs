//! The `buzz-acceptance` probe against a running relay: every check passes,
//! through the shared runner and through the binary.
//!
//! Requires a running relay, so it is `#[ignore]`d. Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_acceptance -- --ignored
//! ```

use std::io::Read;
use std::net::UdpSocket;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use buzz_test_client::acceptance::{self, Config, Outcome, CHECKS};
use nostr::Keys;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".into())
}

#[tokio::test]
#[ignore]
async fn every_acceptance_check_passes_against_a_live_relay() {
    let report = acceptance::run(&Config {
        relay_url: relay_url(),
        connect_addr: None,
        keys: Keys::generate(),
        assertion_file: None,
        stage_timeout: Duration::from_secs(10),
    })
    .await;

    let summary = serde_json::to_string_pretty(&report).unwrap();
    assert!(report.passed, "{summary}");
    let names: Vec<&str> = report.checks.iter().map(|c| c.check).collect();
    assert_eq!(names, CHECKS, "{summary}");
    for check in &report.checks {
        assert_eq!(check.result, Outcome::Success, "{summary}");
    }
    assert!(report.relay_version.is_some(), "{summary}");
}

/// Runs the binary with `args`, metrics sent to a local listener. Returns
/// its output and every datagram that arrived.
fn probe(args: &[&str]) -> (Output, Vec<String>) {
    probe_with_env(args, &[])
}

/// [`probe`] with extra environment variables. The child inherits none of the
/// probe's own settings from the test's environment.
fn probe_with_env(args: &[&str], envs: &[(&str, &str)]) -> (Output, Vec<String>) {
    let agent = UdpSocket::bind("127.0.0.1:0").unwrap();
    agent
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let addr = agent.local_addr().unwrap().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-acceptance"))
        .args(args)
        .args(["--statsd-addr", &addr, "--tag", "env:test"])
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_PRIVATE_KEY_FILE")
        .env_remove("BUZZ_ACCEPTANCE_ASSERTION_FILE")
        .env_remove("BUZZ_ACCEPTANCE_STATSD_ADDR")
        .env_remove("BUZZ_ACCEPTANCE_CONNECT_ADDR")
        .env_remove("BUZZ_RELAY_URL")
        .envs(envs.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Drain both pipes while the child runs, so a chatty child cannot fill a
    // pipe and stall before the deadline.
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("buzz-acceptance did not exit within 120 s");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let output = Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    };
    let mut lines = Vec::new();
    let mut buf = [0u8; 2048];
    while let Ok(n) = agent.recv(&mut buf) {
        lines.push(String::from_utf8_lossy(&buf[..n]).into_owned());
    }
    (output, lines)
}

/// Reads `pipe` to its end on a thread, keeping at most 4 MiB.
fn drain(pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut pipe = pipe;
        let mut kept = Vec::new();
        let _ = (&mut pipe).take(4 << 20).read_to_end(&mut kept);
        let _ = std::io::copy(&mut pipe, &mut std::io::sink());
        kept
    })
}

fn report(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not a JSON report ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
#[ignore]
fn the_binary_passes_against_a_live_relay_and_sends_metrics() {
    let (output, lines) = probe(&["--relay-url", &relay_url()]);
    let report = report(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["passed"], true);
    assert!(
        lines.iter().any(
            |l| l.starts_with("buzz.acceptance.runs:1|c|#result:success,")
                && l.ends_with(",env:test")
        ),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with(
            "buzz.acceptance.check.runs:1|c|#check:fanout,result:success,stage:none,"
        )),
        "{lines:?}"
    );
}

#[test]
#[ignore]
fn the_binary_exits_1_and_reports_when_a_check_fails() {
    let (output, lines) = probe(&[
        "--relay-url",
        "ws://127.0.0.1:1",
        "--stage-timeout-secs",
        "2",
    ]);
    let report = report(&output);
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert_eq!(report["passed"], false);
    assert_eq!(report["checks"][1]["failed_stage"], "ws_connect");
    assert!(
        lines
            .iter()
            .any(|l| l == "buzz.acceptance.runs:1|c|#result:failure,env:test"),
        "{lines:?}"
    );
}

#[test]
#[ignore]
fn the_binary_exits_2_when_it_cannot_run() {
    let dir = std::env::temp_dir().join(format!("acceptance-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad_key_file = dir.join("bad-key");
    std::fs::write(&bad_key_file, "not-a-key\n").unwrap();
    let bad_key_file = bad_key_file.to_str().unwrap();
    let missing_key_file = dir.join("missing");
    let missing_key_file = missing_key_file.to_str().unwrap();
    let relay = relay_url();

    // Each case must stop before the run: no report, no metrics.
    let cases: [(&[&str], &str); 4] = [
        (&["--private-key", "not-a-key"], "private key:"),
        // The key file is read and parsed, not ignored for a new key.
        (&["--private-key-file", bad_key_file], "private key:"),
        (
            &["--private-key-file", missing_key_file],
            "read private key file",
        ),
        (
            &[
                "--private-key",
                "not-a-key",
                "--private-key-file",
                bad_key_file,
            ],
            "cannot be used with",
        ),
    ];
    for (extra, error) in cases {
        let mut args = vec!["--relay-url", relay.as_str()];
        args.extend_from_slice(extra);
        let (output, lines) = probe(&args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{extra:?}: {stderr}");
        assert!(stderr.contains(error), "{extra:?}: {stderr}");
        assert!(output.stdout.is_empty(), "{extra:?}");
        assert!(lines.is_empty(), "{extra:?}: {lines:?}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore]
fn the_binary_authenticates_with_the_key_from_its_key_file() {
    let dir = std::env::temp_dir().join(format!("acceptance-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let key_file = dir.join("key");
    let keys = Keys::generate();
    std::fs::write(
        &key_file,
        format!("{}\n", keys.secret_key().to_secret_hex()),
    )
    .unwrap();

    // The chart passes the path through the environment, so this does too.
    let (output, _) = probe_with_env(
        &["--relay-url", &relay_url()],
        &[("BUZZ_PRIVATE_KEY_FILE", key_file.to_str().unwrap())],
    );
    let report = report(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["passed"], true);
    // A probe that ignored the file would run as a new key.
    assert_eq!(report["pubkey"], keys.public_key().to_hex(), "{report}");
    std::fs::remove_dir_all(dir).unwrap();
}
