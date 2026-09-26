#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use nostr::{Keys, ToBech32};
use rustix::process::{kill_process, kill_process_group, test_kill_process, Pid, Signal};

// Always clean up the real processes, including when an assertion fails.
struct Running {
    swarm: Child,
    pid_file: PathBuf,
}

impl Running {
    fn harness_pid(&self) -> Option<Pid> {
        fs::read_to_string(&self.pid_file)
            .ok()?
            .trim()
            .parse::<i32>()
            .ok()
            .and_then(Pid::from_raw)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(pid) = self.harness_pid() {
            let _ = kill_process_group(pid, Signal::KILL);
        }
        let _ = self.swarm.kill();
        let _ = self.swarm.wait();
    }
}

#[test]
fn blocked_stdout_does_not_prevent_shutdown_or_orphan_the_harness() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let harness = directory.path().join("harness");
    fs::write(
        &harness,
        "#!/bin/sh\ntrap '' TERM\necho $$ > harness.pid\nwhile :; do printf '%08000d\\n' 0; done\n",
    )
    .expect("fixture harness");
    fs::set_permissions(&harness, fs::Permissions::from_mode(0o700)).expect("executable");
    let config = directory.path().join("swarm.yaml");
    fs::write(
        &config,
        "owner:\n  nsec: {env: TEST_OWNER}\ndefaults:\n  relays: [wss://relay.example.com]\n  harness: ./harness\nagents:\n  - name: noisy\n",
    )
    .expect("swarm config");
    let swarm = Command::new(env!("CARGO_BIN_EXE_buzz-swarm"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "TEST_OWNER",
            Keys::generate()
                .secret_key()
                .to_bech32()
                .expect("owner key"),
        )
        .args([
            "--config",
            config.to_str().expect("config path"),
            "start",
            "--grace",
            "1",
        ])
        // Keep stdout open but never read it. The fixture quickly fills the
        // pipe and blocks; Swarm still has to supervise and kill it on time.
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start swarm");
    let mut running = Running {
        swarm,
        pid_file: directory.path().join("workspaces/noisy/harness.pid"),
    };
    let startup_deadline = Instant::now() + Duration::from_secs(5);
    let harness_pid = loop {
        if let Some(pid) = running.harness_pid() {
            break pid;
        }
        assert!(Instant::now() < startup_deadline, "harness did not start");
        assert!(running.swarm.try_wait().expect("poll startup").is_none());
        sleep(Duration::from_millis(10));
    };
    // The PID is written before output begins. Let even a slow fixture fill
    // the OS pipe; the original log multiplexer hung during writer drain.
    sleep(Duration::from_millis(300));
    let swarm_pid = Pid::from_raw(i32::try_from(running.swarm.id()).expect("swarm PID"))
        .expect("nonzero swarm PID");
    kill_process(swarm_pid, Signal::TERM).expect("request shutdown");
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = running.swarm.try_wait().expect("poll shutdown") {
            break status;
        }
        assert!(Instant::now() < deadline, "Swarm hung with blocked stdout");
        sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "shutdown failed: {status}");
    assert_eq!(
        test_kill_process(harness_pid),
        Err(rustix::io::Errno::SRCH),
        "Swarm exited but its harness survived"
    );
    // The harness is reaped. Do not signal its former PID during cleanup.
    fs::remove_file(&running.pid_file).expect("remove PID record");
}
