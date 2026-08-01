#![cfg(target_os = "windows")]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn process_is_gone(pid: u32) -> bool {
    Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 1 }} else {{ exit 0 }}"
            ),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn production_wrapper_preserves_boundary_and_contains_descendants() {
    let dir = tempfile::tempdir().expect("launcher boundary directory");
    let output = dir.path().join("launcher.json");
    let script = dir.path().join("launcher-probe.ps1");
    std::fs::write(
        &script,
        r#"$stdinEof=([Console]::In.ReadToEnd()).Length -eq 0
Write-Output 'launcher-stdout-marker'
[Console]::Error.WriteLine('launcher-stderr-marker')
$descendant=Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command','while ($true) { Start-Sleep -Seconds 1 }') -WindowStyle Hidden -PassThru
$payload=[ordered]@{
  cwd=(Get-Location).Path
  value=$env:BUZZ_LAUNCHER_TEST
  removed=$env:BUZZ_LAUNCHER_REMOVE
  inherited=$env:USERPROFILE
  stdinEof=$stdinEof
  programEnvelope=$env:BUZZ_MANAGED_LAUNCH_PROGRAM_WIDE
  argsEnvelope=$env:BUZZ_MANAGED_LAUNCH_ARGS_WIDE
  args=@($args)
  descendantPid=$descendant.Id
}
$payload|ConvertTo-Json -Compress|Set-Content -Encoding utf8 -LiteralPath $env:BUZZ_LAUNCHER_OUTPUT
while ($true) { Start-Sleep -Seconds 1 }"#,
    )
    .expect("write launcher boundary probe");

    let log = dir.path().join("launcher.log");
    let mut command = Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
    command
        .env_clear()
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("two words")
        .arg("")
        .arg("wide-雪")
        .current_dir(dir.path())
        .env("SystemRoot", r"C:\Windows")
        .env("BUZZ_LAUNCHER_TEST", "kept")
        .env("BUZZ_LAUNCHER_OUTPUT", &output)
        .env("BUZZ_LAUNCHER_REMOVE", "must-not-survive")
        .env_remove("BUZZ_LAUNCHER_REMOVE")
        // Case-varied user collisions must not replace the private envelope.
        .env("buzz_managed_launch_program_wide", "malicious")
        .env("BuZz_MaNaGeD_LaUnCh_ArGs_WiDe", "malicious")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut process = buzz_lib::managed_launcher::spawn_contained(
        command,
        std::path::Path::new(env!("CARGO_BIN_EXE_buzz-desktop")),
        &log,
        true,
    )
    .expect("spawn through production managed wrapper");
    let launcher_pid = process.launcher_pid();

    let deadline = Instant::now() + Duration::from_secs(10);
    while !output.exists() {
        assert!(Instant::now() < deadline, "production launcher timed out");
        std::thread::sleep(Duration::from_millis(20));
    }

    let bytes = {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match std::fs::read(&output) {
                Ok(bytes) => break bytes,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                Err(error) => panic!("launcher output: {error}"),
            }
        }
    };
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let payload: serde_json::Value = serde_json::from_slice(bytes).expect("launcher JSON");
    let descendant_pid = payload["descendantPid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .expect("descendant PID");

    assert_eq!(payload["cwd"], dir.path().display().to_string());
    assert_eq!(payload["value"], "kept");
    assert_eq!(payload["removed"], serde_json::Value::Null);
    assert_eq!(payload["inherited"], serde_json::Value::Null);
    assert_eq!(payload["stdinEof"], true);
    assert_eq!(payload["programEnvelope"], serde_json::Value::Null);
    assert_eq!(payload["argsEnvelope"], serde_json::Value::Null);
    assert_eq!(payload["args"][0], "two words");
    assert_eq!(payload["args"][1], "");
    assert_eq!(payload["args"][2], "wide-雪");
    let log_text = std::fs::read_to_string(&log).expect("managed launcher log");
    assert_eq!(log_text.matches("launcher-stdout-marker").count(), 1);
    assert_eq!(log_text.matches("launcher-stderr-marker").count(), 1);
    assert!(
        process.active_process_count().expect("query owned Job") >= 2,
        "owned Job did not contain the launcher/target tree"
    );

    let started = Instant::now();
    let (status, remaining_members) = process
        .terminate_checked()
        .expect("checked production-boundary termination");
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(5), "Stop took {elapsed:?}");
    assert!(
        !status.success(),
        "terminated launcher unexpectedly succeeded"
    );
    assert_eq!(remaining_members, 0, "Job membership was not zero");

    let gone_deadline = Instant::now() + Duration::from_secs(5);
    while !(process_is_gone(launcher_pid) && process_is_gone(descendant_pid)) {
        assert!(
            Instant::now() < gone_deadline,
            "launcher or descendant survived checked termination"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    println!(
        "NATIVE_PRODUCTION_BOUNDARY_PROOF elapsed_ms={} job_members={} launcher_pid={} descendant_pid={} survivors=0",
        elapsed.as_millis(),
        remaining_members,
        launcher_pid,
        descendant_pid
    );
}
