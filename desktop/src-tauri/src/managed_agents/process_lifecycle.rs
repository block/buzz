//! Windows process-tree lifecycle primitives for managed agents.
//!
//! The Unix teardown uses `process_group(0)` + group signals (in `runtime.rs`).
//! Windows has no process groups, so the harness's 24 agent workers + MCP
//! servers are reaped two ways here:
//!   - [`JobHandle`] / [`create_job_for_child`] — the in-process stop path. A
//!     Job Object owns the tree and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` kills
//!     it when the handle drops.
//!   - [`taskkill_tree`] — the after-restart path, where only the PID survives
//!     in the record and no job handle is available.
//!
//! This module is `#[cfg(windows)]`-only; nothing here compiles on other
//! platforms.

use windows_sys::Win32::Foundation::HANDLE;

/// Win32 Job Object that owns the harness process and (via Windows' default
/// child-inheritance) every process it spawns. Dropping the handle with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` set kills the whole tree — the Windows
/// mirror of the Unix `process_group(0)` + group-signal teardown. This is what
/// guarantees the 24 agent workers + MCP servers die when we stop or when the
/// app exits, instead of being orphaned by a bare `Child::kill()`.
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
        // KILL_ON_JOB_CLOSE means the tree dies when the LAST handle closes.
        // We hold the only handle (not inheritable), so this reaps the tree.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// Create a Job Object, assign `pid` to it, and configure it to kill the whole
/// tree when the returned handle is dropped. Returns `None` on any failure so
/// the caller can fall back to `Child::kill()` — a degraded teardown beats a
/// failed spawn.
///
/// Assignment happens immediately after spawn, on the same parent thread. The
/// child (buzz-acp) does spawn its 24 workers before it connects to the relay,
/// so the window between our spawn and our assignment is NOT structurally empty.
/// What closes it is assign-latency: `OpenProcess` + `AssignProcessToJobObject`
/// are a few synchronous Win32 calls (microseconds), while buzz-acp must init
/// tokio, parse its config, and spawn 24 children (tens-to-hundreds of ms), so
/// the assign reliably wins before any worker exists. Once assigned, Windows
/// places every subsequently-spawned descendant in the job automatically.
///
/// `CREATE_SUSPENDED` -> assign -> `ResumeThread` would make the window airtight
/// regardless of child timing, but it requires raw `CreateProcessW`/`ResumeThread`
/// (materially more unsafe Win32) to close a microsecond race, so it is
/// deliberately not used here.
fn create_job_for_child(pid: u32) -> Option<JobHandle> {
    use std::ptr::null;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return None;
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == FALSE {
            CloseHandle(job);
            return None;
        }

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
    spawn_config_hash: u64,
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
        spawn_config_hash,
        setup_mode,
        adapter_availability,
        start_nonce,
        job,
    }
}
