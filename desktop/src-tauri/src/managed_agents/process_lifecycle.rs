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
/// For the harness spawn path ([`finish_spawn`]) assignment happens immediately
/// after a normal spawn. The child (buzz-acp) must init tokio, parse its config,
/// and spawn 24 children (tens-to-hundreds of ms) before any descendant exists,
/// so the microsecond `OpenProcess` + `AssignProcessToJobObject` reliably wins
/// that race. Once assigned, Windows places every subsequently-spawned
/// descendant in the job automatically.
///
/// The discovery path (`bounded_command`) runs arbitrary probe commands that
/// can background a descendant and exit in the same tick, so it cannot rely on
/// assign-latency. It spawns with `CREATE_SUSPENDED`, assigns the frozen child
/// here, then calls [`resume_process`] — no descendant can exist until the job
/// owns the root, closing the race by construction.
pub(crate) fn create_job_for_child(pid: u32) -> Option<JobHandle> {
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

/// Resume a process spawned with `CREATE_SUSPENDED` by resuming every thread it
/// owns. A fresh `CREATE_SUSPENDED` process has exactly one thread suspended at
/// its entry point; resuming it lets the process run. We enumerate via a
/// ToolHelp thread snapshot filtered to `pid` rather than tracking the initial
/// thread id (`std::process::Command` does not expose it), and resume each so
/// the walk is correct even in the pathological multi-thread case.
///
/// Returns `true` only if at least one owned thread was resumed. `false` means
/// no thread could be resumed — the caller must treat the child as unusable and
/// tear it down, since a still-suspended root would otherwise hang to the
/// deadline.
pub(crate) fn resume_process(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }

        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

        let mut resumed_any = false;
        let mut has_entry = Thread32First(snapshot, &mut entry);
        while has_entry != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    // ResumeThread returns u32::MAX on failure; any other value
                    // is the thread's previous suspend count.
                    if ResumeThread(thread) != u32::MAX {
                        resumed_any = true;
                    }
                    CloseHandle(thread);
                }
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            has_entry = Thread32Next(snapshot, &mut entry);
        }

        CloseHandle(snapshot);
        resumed_any
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
