//! Windows process-tree lifecycle primitives for managed agents.
//!
//! Windows has no process groups. A non-killing Job Object may be retained as
//! process-tree identity while Desktop is connected, but closing Desktop's
//! handle must never own runtime lifetime. Explicit stop uses [`taskkill_tree`]
//! only after the generation-scoped control shutdown has authenticated.
//!
//! This module is `#[cfg(windows)]`-only; nothing here compiles on other
//! platforms.

use windows_sys::Win32::Foundation::HANDLE;

/// Win32 Job Object used only to group the harness process tree while Desktop
/// is connected. It deliberately does not set
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; dropping Desktop must leave the
/// durable runtime alive.
pub struct JobHandle(HANDLE);

// The handle is owned exclusively by this wrapper; moving it across threads is
// sound (the spawn path in restore.rs runs in a thread scope).
unsafe impl Send for JobHandle {}

impl std::fmt::Debug for JobHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JobHandle(..)")
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// Create a non-killing Job Object and assign `pid` to it. The handle is an
/// observation/cleanup aid only; process lifetime remains independent of
/// Desktop.
fn create_job_for_child(pid: u32) -> Option<JobHandle> {
    use std::ptr::null;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return None;
        }

        // Do not set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE. Closing Desktop's
        // handle is an ordinary controller disconnect, not a stop request.

        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, pid);
        if process.is_null() {
            CloseHandle(job);
            return None;
        }
        let assigned = AssignProcessToJobObject(job, process);
        CloseHandle(process);
        if assigned == FALSE {
            CloseHandle(job);
            return None;
        }

        Some(JobHandle(job))
    }
}

/// Kill the entire process tree rooted at `pid` via `taskkill /T`, the closest
/// equivalent to the Unix process-group kill. Used on the after-restart path
/// where no job handle survived. `CREATE_NO_WINDOW` keeps taskkill's own
/// console from flashing.
pub fn taskkill_tree(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("failed to run taskkill for pid {pid}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "taskkill exited with status {status} for pid {pid}"
        ))
    }
}

/// Assign a freshly-spawned harness `child` to a Job Object and package it into
/// a [`ManagedAgentProcess`]. On job-assignment failure the process is still
/// returned with `job: None` — teardown then falls back to `Child::kill()`,
/// which kills only the harness (a degraded teardown beats a failed spawn).
pub fn finish_spawn(
    child: std::process::Child,
    log_path: std::path::PathBuf,
    spawn_config: super::spawn_snapshot::SpawnConfigSnapshot,
    setup_mode: bool,
    adapter_availability: Option<super::AcpAvailabilityStatus>,
    start_nonce: String,
    agent_name: &str,
) -> super::ManagedAgentProcess {
    let job = create_job_for_child(child.id());
    if job.is_none() {
        eprintln!(
            "buzz-desktop: failed to assign agent {agent_name} to a Job Object; \
             teardown will fall back to killing only the harness process"
        );
    }
    super::ManagedAgentProcess {
        child,
        log_path,
        spawn_config,
        setup_mode,
        adapter_availability,
        start_nonce,
        job,
    }
}
