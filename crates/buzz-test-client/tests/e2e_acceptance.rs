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
    let agent = UdpSocket::bind("127.0.0.1:0").unwrap();
    agent
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let addr = agent.local_addr().unwrap().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-acceptance"))
        .args(args)
        .args(["--statsd-addr", &addr, "--tag", "env:test"])
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_RELAY_URL")
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
    let (output, lines) = probe(&["--relay-url", &relay_url(), "--private-key", "not-a-key"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(lines.is_empty(), "{lines:?}");
}
