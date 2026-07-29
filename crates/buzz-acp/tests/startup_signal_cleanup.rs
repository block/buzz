#![cfg(unix)]

use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct TestProcess {
    harness: Child,
    adapter_pgid: Option<Pid>,
    temp_dir: PathBuf,
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = self.harness.kill();
        let _ = self.harness.wait();
        if let Some(pgid) = self.adapter_pgid {
            let _ = killpg(pgid, Signal::SIGKILL);
        }
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

#[test]
fn repeated_sigterm_during_blocked_eager_initialize_reaps_adapter_process_group() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let temp_dir =
        std::env::temp_dir().join(format!("buzz-acp-sigterm-{}-{nonce}", std::process::id()));
    fs::create_dir(&temp_dir).expect("create test directory");
    let adapter_script = temp_dir.join("blocked-adapter.sh");
    let adapter_pid_file = temp_dir.join("adapter.pid");
    fs::write(
        &adapter_script,
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$$\" > \"$BUZZ_TEST_ADAPTER_PID_FILE\"\nexec /bin/sleep 300\n",
    )
    .expect("write blocked adapter");
    let mut permissions = fs::metadata(&adapter_script)
        .expect("adapter metadata")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&adapter_script, permissions).expect("make adapter executable");

    let harness = Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
        .env_clear()
        .env("BUZZ_TEST_ADAPTER_PID_FILE", &adapter_pid_file)
        .arg("--private-key")
        .arg("0000000000000000000000000000000000000000000000000000000000000001")
        .arg("--agent-command")
        .arg(&adapter_script)
        .arg("--agent-args")
        .arg("ignored")
        .arg("--relay-url")
        .arg("ws://127.0.0.1:9")
        .arg("--no-presence")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn buzz-acp harness");
    let mut process = TestProcess {
        harness,
        adapter_pgid: None,
        temp_dir,
    };

    assert!(
        wait_until(|| adapter_pid_file.exists(), Duration::from_secs(10)),
        "blocked adapter did not publish its PID"
    );
    let adapter_pid: i32 = fs::read_to_string(&adapter_pid_file)
        .expect("read adapter PID")
        .trim()
        .parse()
        .expect("parse adapter PID");
    let adapter_pgid = Pid::from_raw(adapter_pid);
    process.adapter_pgid = Some(adapter_pgid);
    assert_eq!(
        killpg(adapter_pgid, None::<Signal>),
        Ok(()),
        "adapter process group must exist before SIGTERM"
    );

    kill(Pid::from_raw(process.harness.id() as i32), Signal::SIGTERM).expect("signal harness");
    // The listener must remain installed after the graceful request. A second
    // signal escalates cleanup without taking the default process-exit path,
    // which would abandon process-group verification.
    std::thread::sleep(Duration::from_millis(20));
    if process.harness.try_wait().ok().flatten().is_none() {
        kill(Pid::from_raw(process.harness.id() as i32), Signal::SIGTERM)
            .expect("repeat signal harness during cleanup");
    }
    assert!(
        wait_until(
            || process.harness.try_wait().ok().flatten().is_some(),
            Duration::from_secs(10),
        ),
        "harness did not exit after SIGTERM"
    );
    assert!(
        wait_until(
            || killpg(adapter_pgid, None::<Signal>) == Err(Errno::ESRCH),
            Duration::from_secs(5),
        ),
        "adapter process group {adapter_pid} survived harness SIGTERM"
    );
    process.adapter_pgid = None;
}
