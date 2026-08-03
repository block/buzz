use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use buzz_runtime::{
    argv_sha256, read_runner_receipt, write_job_spec, JobSpec, JobStartRequest, RunnerReceiptState,
    StoreHandle,
};
use chrono::Utc;
use uuid::Uuid;

const SECRET_SENTINEL: &str = "SECRET_SENTINEL_MUST_NOT_PERSIST";

fn owner_directory(path: &std::path::Path) {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).expect("create owner-only directory");
}

#[cfg(unix)]
fn fake_driver(root: &std::path::Path) -> (std::path::PathBuf, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let executable = root.join("fake-lh");
    std::fs::write(
        &executable,
        b"#!/bin/sh\n\
printf 'captured stdout first\\n'\n\
printf 'captured stderr first\\n' >&2\n\
printf 'SECRET_SENTINEL_'\n\
printf 'SECRET_SENTINEL_' >&2\n\
sleep 3\n\
printf 'MUST_NOT_PERSIST\\n'\n\
printf 'MUST_NOT_PERSIST\\n' >&2\n\
printf 'captured stdout last\\n'\n\
printf 'captured stderr last\\n' >&2\n\
printf 'private=%s provider=%s\\n' \"${BUZZ_PRIVATE_KEY-unset}\" \"${OPENAI_API_KEY-unset}\"\n\
printf 'receipt=%s model=%s\\n' \"${BUZZ_RUNTIME_RECEIPT-unset}\" \"${BUZZ_RUNTIME_MODEL_TOKEN-unset}\"\n\
printf 'unrelated=%s\\n' \"${UNRELATED_CONFIG-unset}\"\n\
printf 'safe-path=%s safe-home=%s safe-tmp=%s safe-lang=%s\\n' \"$PATH\" \"$HOME\" \"$TMPDIR\" \"$LANG\"\n",
    )
    .expect("write fake driver");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .expect("make fake driver executable");
    (executable, Vec::new())
}

#[cfg(windows)]
fn fake_driver(root: &std::path::Path) -> (std::path::PathBuf, Vec<String>) {
    let script = root.join("fake-lh.cmd");
    std::fs::write(
        &script,
        "@echo captured stdout first\r\n\
@echo captured stderr first 1>&2\r\n\
@<nul set /p =SECRET_SENTINEL_\r\n\
@<nul set /p =SECRET_SENTINEL_ 1>&2\r\n\
@ping -n 4 127.0.0.1 >NUL\r\n\
@echo MUST_NOT_PERSIST\r\n\
@echo MUST_NOT_PERSIST 1>&2\r\n\
@echo captured stdout last\r\n\
@echo captured stderr last 1>&2\r\n\
@if defined BUZZ_PRIVATE_KEY (echo private=leaked) else if defined OPENAI_API_KEY (echo provider=leaked) else if defined BUZZ_RUNTIME_RECEIPT (echo receipt=leaked) else if defined BUZZ_RUNTIME_MODEL_TOKEN (echo model=leaked) else if defined UNRELATED_CONFIG (echo unrelated=leaked) else echo private=unset provider=unset receipt=unset model=unset unrelated=unset\r\n\
@echo safe-path=%PATH% safe-home=%HOME% safe-tmp=%TMP% safe-lang=%LANG%\r\n",
    )
    .expect("write fake driver");
    let executable = std::path::PathBuf::from(
        std::env::var_os("COMSPEC").unwrap_or_else(|| "C:\\Windows\\System32\\cmd.exe".into()),
    );
    (
        executable,
        vec!["/C".into(), script.to_string_lossy().into_owned()],
    )
}

fn assert_tree_excludes(path: &std::path::Path, needle: &[u8]) {
    for entry in std::fs::read_dir(path).expect("read runtime artifact directory") {
        let entry = entry.expect("read runtime artifact entry");
        let file_type = entry.file_type().expect("read runtime artifact type");
        if file_type.is_dir() {
            assert_tree_excludes(&entry.path(), needle);
        } else if file_type.is_file() {
            let bytes = std::fs::read(entry.path()).expect("read runtime artifact");
            assert!(
                !bytes.windows(needle.len()).any(|window| window == needle),
                "secret persisted in {}",
                entry.path().display()
            );
        }
    }
}

#[test]
fn detached_runner_ignores_turn_deadlines_drains_streams_and_redacts_runtime_secret() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let runtime = temp.path().join("runtime");
    let workspace = temp.path().join("workspace");
    owner_directory(&runtime);
    owner_directory(&workspace);
    drop(StoreHandle::open(runtime.join("runtime.sqlite3")).expect("open runtime database"));
    let (executable, argv) = fake_driver(temp.path());
    let job_id = Uuid::new_v4();
    let spec = JobSpec {
        runtime_id: "test-runtime".into(),
        job_id,
        attempt: 1,
        executable,
        argv_sha256: argv_sha256(&argv).expect("hash argv"),
        request: JobStartRequest {
            channel_id: Uuid::new_v4(),
            source_event_id: None,
            driver: "lh".into(),
            argv,
            cwd: workspace.to_string_lossy().into_owned(),
            summary: "exercise durable runner".into(),
        },
        created_at: Utc::now(),
    };
    let spec_path = write_job_spec(&runtime, &spec).expect("write owner-only spec");
    let sentinel = SECRET_SENTINEL;
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    command
        .arg("__job-runner")
        .arg(&spec_path)
        .env("BUZZ_PRIVATE_KEY", sentinel)
        .env("BUZZ_RELAY_URL", "wss://sentinel.invalid")
        .env("BUZZ_RUNTIME_RECEIPT", sentinel)
        .env("BUZZ_RUNTIME_MODEL_TOKEN", sentinel)
        .env("OPENAI_API_KEY", sentinel)
        .env("BUZZ_ACP_IDLE_TIMEOUT", "1")
        .env("UNRELATED_CONFIG", "should-not-inherit")
        .env("BUZZ_ACP_MAX_TURN_DURATION", "2")
        .env(
            "PATH",
            std::env::var_os("PATH").expect("test process has PATH"),
        )
        .env("HOME", "safe-home")
        .env("TMPDIR", "safe-tmp")
        .env("TMP", "safe-tmp")
        .env("LANG", "safe-lang")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0200);
    }

    let started = Instant::now();
    let status = command.status().expect("run hidden job runner");
    assert!(status.success(), "runner failed: {status}");
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "fake job did not cross configured ACP hard-turn deadline"
    );

    let receipt = read_runner_receipt(&runtime, job_id, 1).expect("read terminal receipt");
    assert_eq!(receipt.state, RunnerReceiptState::Succeeded);
    assert_eq!(receipt.exit_code, Some(0));
    assert!(receipt.finished_at.is_some());

    let attempt = runtime.join("jobs").join(job_id.to_string()).join("1");
    let stdout = std::fs::read_to_string(attempt.join("stdout.log")).expect("read stdout log");
    let stderr = std::fs::read_to_string(attempt.join("stderr.log")).expect("read stderr log");
    let spec_json = std::fs::read_to_string(&spec_path).expect("read spec");
    let receipt_json =
        std::fs::read_to_string(attempt.join("runner-receipt.json")).expect("read receipt");
    assert!(stdout.contains("captured stdout first"));
    assert!(stdout.contains("[REDACTED]"));
    assert!(stdout.contains("captured stdout last"));
    assert!(stderr.contains("captured stderr first"));
    assert!(stderr.contains("[REDACTED]"));
    assert!(stderr.contains("captured stderr last"));
    let redacted = stdout.find("[REDACTED]").expect("redaction marker");
    assert!(
        stdout.find("captured stdout first").expect("first output") < redacted
            && redacted < stdout.find("captured stdout last").expect("last output")
    );
    assert!(stdout.contains("private=unset provider=unset"));
    assert!(stdout.contains("receipt=unset model=unset"));
    assert!(stdout.contains("unrelated=unset"));
    assert!(stdout.contains("safe-path="));
    assert!(stdout.contains("safe-home=safe-home"));
    assert!(stdout.contains("safe-tmp=safe-tmp"));
    assert!(stdout.contains("safe-lang=safe-lang"));
    assert!(!spec_json.contains(sentinel));
    assert!(!receipt_json.contains(sentinel));
    assert_tree_excludes(&runtime, sentinel.as_bytes());
    assert_tree_excludes(&runtime, b"should-not-inherit");
}

#[cfg(unix)]
#[test]
fn successful_driver_with_live_descendant_is_failed_and_tree_is_reaped() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;

    let temp = tempfile::tempdir().expect("temporary directory");
    let runtime = temp.path().join("runtime");
    let workspace = temp.path().join("workspace");
    owner_directory(&runtime);
    owner_directory(&workspace);
    let descendant_pid_path = temp.path().join("descendant.pid");
    let executable = temp.path().join("forking-lh");
    std::fs::write(
        &executable,
        b"#!/bin/sh\ntrap '' HUP\nsleep 30 </dev/null >/dev/null 2>/dev/null &\nprintf '%s' \"$!\" > \"$1\"\nsleep 1\nexit 0\n",
    )
    .expect("write forking driver");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .expect("make forking driver executable");
    let argv = vec![descendant_pid_path.to_string_lossy().into_owned()];
    let job_id = Uuid::new_v4();
    let spec = JobSpec {
        runtime_id: "descendant-test-runtime".into(),
        job_id,
        attempt: 1,
        executable,
        argv_sha256: argv_sha256(&argv).expect("hash argv"),
        request: JobStartRequest {
            channel_id: Uuid::new_v4(),
            source_event_id: None,
            driver: "lh".into(),
            argv,
            cwd: workspace.to_string_lossy().into_owned(),
            summary: "reject false driver success".into(),
        },
        created_at: Utc::now(),
    };
    let spec_path = write_job_spec(&runtime, &spec).expect("write owner-only spec");
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    command
        .arg("__job-runner")
        .arg(&spec_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut runner = command.spawn().expect("spawn hidden job runner");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !descendant_pid_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let descendant_pid: u32 = std::fs::read_to_string(&descendant_pid_path)
        .expect("driver recorded descendant pid")
        .parse()
        .expect("parse descendant pid");
    let descendant_marker =
        buzz_runtime::process_start_marker(descendant_pid).expect("capture descendant identity");

    let status = runner.wait().expect("wait for job runner");
    assert!(
        !status.success(),
        "runner must not report process success while a descendant survives"
    );
    let receipt = read_runner_receipt(&runtime, job_id, 1).expect("read terminal receipt");
    assert_eq!(receipt.state, RunnerReceiptState::Failed);
    assert_eq!(
        receipt.error_code.as_deref(),
        Some("driver_descendants_survived")
    );
    let reap_deadline = Instant::now() + Duration::from_secs(2);
    while buzz_runtime::process_matches_marker(descendant_pid, &descendant_marker)
        && Instant::now() < reap_deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !buzz_runtime::process_matches_marker(descendant_pid, &descendant_marker),
        "surviving descendant must be terminated with the governed process group"
    );
}
