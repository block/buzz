//! Windows process-tree lifecycle primitives for managed agents.
//!
//! The Unix teardown uses `process_group(0)` + group signals (in `runtime.rs`).
//! Windows has no process groups, so the harness's 24 agent workers + MCP
//! servers are reaped two ways here:
//!   - [`JobHandle`] / [`create_job_for_child`] — the in-process stop path. A
//!     Job Object owns the tree and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` kills
//!     it when the handle drops.
//!   - [`terminate_process_tree`] — the after-restart path, where only the PID
//!     survives in the record and no job handle is available. It uses Win32
//!     process enumeration/termination directly so teardown never launches an
//!     external helper while a runtime-management lock is held.
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

/// Snapshot `(pid, parent_pid)` for every visible process without launching a
/// helper executable.
fn process_snapshot() -> Result<Vec<(u32, u32)>, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "failed to snapshot Windows processes: error {}",
                windows_sys::Win32::Foundation::GetLastError()
            ));
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut entries = Vec::new();
        if Process32FirstW(snapshot, &mut entry) != FALSE {
            loop {
                entries.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(snapshot, &mut entry) == FALSE {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        Ok(entries)
    }
}

fn terminate_pid(pid: u32, wait: bool) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, FALSE, STILL_ACTIVE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, TerminateProcess, WaitForSingleObject,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, FALSE, pid);
        if process.is_null() {
            let error = windows_sys::Win32::Foundation::GetLastError();
            // The process may have exited between the snapshot and OpenProcess.
            if error == ERROR_INVALID_PARAMETER {
                return Ok(());
            }
            return Err(format!(
                "failed to open managed process {pid} for termination: error {error}"
            ));
        }
        if TerminateProcess(process, 1) == FALSE {
            let error = windows_sys::Win32::Foundation::GetLastError();
            let mut exit_code = STILL_ACTIVE as u32;
            let already_exited = GetExitCodeProcess(process, &mut exit_code) != FALSE
                && exit_code != STILL_ACTIVE as u32;
            CloseHandle(process);
            if already_exited {
                return Ok(());
            }
            return Err(format!(
                "failed to terminate managed process {pid}: error {error}"
            ));
        }
        if wait && WaitForSingleObject(process, 1_000) != WAIT_OBJECT_0 {
            CloseHandle(process);
            return Err(format!(
                "managed process {pid} did not exit within one second after termination"
            ));
        }
        CloseHandle(process);
        Ok(())
    }
}

/// Kill the entire process tree rooted at `pid` through Win32 APIs. This is
/// the after-restart equivalent of dropping the live Job Object handle. The
/// root is terminated and reaped first so it cannot create new descendants;
/// then descendants from snapshots taken before and after root termination are
/// terminated deepest-first.
pub fn terminate_process_tree(pid: u32) -> Result<(), String> {
    let mut entries = process_snapshot()?;
    terminate_pid(pid, true)?;
    let after = process_snapshot()?;
    // If `pid` was recycled immediately after the original root exited, do
    // not follow the new process's children. Otherwise the post-exit snapshot
    // closes the small window in which the root could create a last child
    // between the first snapshot and TerminateProcess.
    if !after.iter().any(|(candidate, _)| *candidate == pid) {
        entries.extend(after);
    }
    entries.sort_unstable();
    entries.dedup();

    let mut errors = Vec::new();
    for wave in super::runtime::descendant_process_waves(&entries, pid)
        .into_iter()
        .rev()
    {
        for descendant in wave {
            if let Err(error) = terminate_pid(descendant, false) {
                errors.push(error);
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to terminate part of managed process tree {pid}: {}",
            errors.join("; ")
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
