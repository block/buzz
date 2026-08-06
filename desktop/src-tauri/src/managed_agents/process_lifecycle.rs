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

use std::{collections::HashMap, os::windows::io::AsRawHandle, time::Duration};

use windows_sys::Win32::Foundation::HANDLE;

use super::runtime::{
    next_verified_windows_descendants, terminate_if_windows_identity_matches,
    WindowsIdentityObservation, WindowsProcessIdentity, WindowsProcessSnapshotEntry,
};

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

impl JobHandle {
    /// Terminate every process assigned to this job and wait for the job's
    /// active-process count to reach zero. Keeping the job handle open during
    /// the wait prevents the containment boundary from disappearing early.
    pub(crate) fn terminate_and_wait(&self, timeout: Duration) -> Result<(), String> {
        use windows_sys::Win32::Foundation::FALSE;
        use windows_sys::Win32::System::JobObjects::{
            JobObjectBasicAccountingInformation, QueryInformationJobObject, TerminateJobObject,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        };

        unsafe {
            if TerminateJobObject(self.0, 1) == FALSE {
                return Err(format!(
                    "failed to terminate Windows Job Object: error {}",
                    windows_sys::Win32::Foundation::GetLastError()
                ));
            }
        }
        let started = std::time::Instant::now();
        loop {
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            let queried = unsafe {
                QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            };
            if queried == FALSE {
                return Err(format!(
                    "failed to query Windows Job Object cleanup: error {}",
                    unsafe { windows_sys::Win32::Foundation::GetLastError() }
                ));
            }
            if info.ActiveProcesses == 0 {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                return Err(format!(
                    "Windows Job Object still owns {} process(es) after {} ms",
                    info.ActiveProcesses,
                    timeout.as_millis()
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Create a Job Object, assign `child` through its stable process handle, and
/// configure it to kill the whole tree when the returned handle is dropped.
/// Using the child handle avoids reopening a possibly recycled numeric PID.
///
/// Assignment happens immediately after spawn, on the same parent thread. The
/// long-running ACP harness is not suspended here, so a child could still exit
/// or create a descendant before assignment; assignment failure therefore
/// degrades only to identity-bound teardown. Readiness probes use the stricter
/// suspended-spawn path below and never execute before job assignment.
fn create_job_for_child(child: &std::process::Child) -> Result<JobHandle, String> {
    use std::ptr::null;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return Err(format!(
                "failed to create Windows Job Object: error {}",
                windows_sys::Win32::Foundation::GetLastError()
            ));
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
            let error = windows_sys::Win32::Foundation::GetLastError();
            CloseHandle(job);
            return Err(format!(
                "failed to configure Windows Job Object: error {error}"
            ));
        }

        let process = child.as_raw_handle() as HANDLE;
        let assigned = AssignProcessToJobObject(job, process);
        if assigned == FALSE {
            let error = windows_sys::Win32::Foundation::GetLastError();
            CloseHandle(job);
            return Err(format!(
                "failed to assign child {} to Windows Job Object: error {error}",
                child.id()
            ));
        }

        Ok(JobHandle(job))
    }
}

struct OwnedProcessHandle(HANDLE);

impl Drop for OwnedProcessHandle {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

fn creation_time_from_handle(handle: HANDLE) -> Result<u64, String> {
    use windows_sys::Win32::Foundation::{FALSE, FILETIME};
    use windows_sys::Win32::System::Threading::GetProcessTimes;

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    if ok == FALSE {
        return Err(format!(
            "failed to query Windows process creation time: error {}",
            unsafe { windows_sys::Win32::Foundation::GetLastError() }
        ));
    }
    Ok(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
}

pub(crate) fn child_process_identity(child: &std::process::Child) -> Result<u64, String> {
    creation_time_from_handle(child.as_raw_handle() as HANDLE)
}

fn open_process_handle(pid: u32, access: u32) -> Result<Option<OwnedProcessHandle>, String> {
    use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, FALSE};
    use windows_sys::Win32::System::Threading::OpenProcess;

    let handle = unsafe { OpenProcess(access, FALSE, pid) };
    if handle.is_null() {
        let error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        if error == ERROR_INVALID_PARAMETER {
            return Ok(None);
        }
        return Err(format!(
            "failed to open Windows process {pid}: error {error}"
        ));
    }
    Ok(Some(OwnedProcessHandle(handle)))
}

fn identity_from_pid(pid: u32) -> WindowsIdentityObservation {
    use windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;

    let Ok(handle) = open_process_handle(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
        return WindowsIdentityObservation::Unverified;
    };
    let Some(handle) = handle else {
        return WindowsIdentityObservation::Exited;
    };
    match creation_time_from_handle(handle.0) {
        Ok(creation_time) => {
            WindowsIdentityObservation::Verified(WindowsProcessIdentity { pid, creation_time })
        }
        Err(_) => WindowsIdentityObservation::Unverified,
    }
}

pub(crate) fn process_identity_matches(pid: u32, expected_creation_time: u64) -> bool {
    use windows_sys::Win32::Foundation::{FALSE, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let Ok(Some(handle)) = open_process_handle(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
        return false;
    };
    let Ok(creation_time) = creation_time_from_handle(handle.0) else {
        return false;
    };
    let mut exit_code = 0;
    let queried = unsafe { GetExitCodeProcess(handle.0, &mut exit_code) };
    queried != FALSE && exit_code == STILL_ACTIVE as u32 && creation_time == expected_creation_time
}

fn resume_suspended_process(pid: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "failed to snapshot Windows threads for suspended probe {pid}: error {}",
                windows_sys::Win32::Foundation::GetLastError()
            ));
        }
        let mut entry = THREADENTRY32::default();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut found = None;
        if Thread32First(snapshot, &mut entry) != FALSE {
            loop {
                if entry.th32OwnerProcessID == pid {
                    found = Some(entry.th32ThreadID);
                    break;
                }
                if Thread32Next(snapshot, &mut entry) == FALSE {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        let Some(thread_id) = found else {
            return Err(format!(
                "failed to find primary thread for suspended Windows probe {pid}"
            ));
        };
        let thread = OpenThread(THREAD_SUSPEND_RESUME, FALSE, thread_id);
        if thread.is_null() {
            return Err(format!(
                "failed to open primary thread for suspended Windows probe {pid}: error {}",
                windows_sys::Win32::Foundation::GetLastError()
            ));
        }
        let previous = ResumeThread(thread);
        let error = windows_sys::Win32::Foundation::GetLastError();
        CloseHandle(thread);
        if previous == u32::MAX {
            return Err(format!(
                "failed to resume suspended Windows probe {pid}: error {error}"
            ));
        }
        Ok(())
    }
}

/// Spawn a probe suspended, bind it to a fresh kill-on-close Job Object, then
/// resume its primary thread. Assignment failures occur before user code can
/// execute, so no probe is allowed to run outside its dedicated containment
/// boundary.
pub(crate) fn spawn_probe_in_job(
    command: &mut std::process::Command,
) -> Result<(std::process::Child, JobHandle), String> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn suspended Windows readiness probe: {error}"))?;
    let job = match create_job_for_child(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    if let Err(error) = resume_suspended_process(child.id()) {
        drop(job);
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok((child, job))
}

/// Snapshot `(pid, parent_pid)` for every visible process without launching a
/// helper executable.
fn process_snapshot() -> Result<Vec<WindowsProcessSnapshotEntry>, String> {
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
                let identity = match identity_from_pid(entry.th32ProcessID) {
                    WindowsIdentityObservation::Verified(identity) => Some(identity),
                    WindowsIdentityObservation::Exited | WindowsIdentityObservation::Unverified => {
                        None
                    }
                };
                entries.push(WindowsProcessSnapshotEntry {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    identity,
                });
                if Process32NextW(snapshot, &mut entry) == FALSE {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        Ok(entries)
    }
}

struct KnownProcess {
    identity: WindowsProcessIdentity,
    handle: OwnedProcessHandle,
    depth: usize,
    terminated: bool,
}

fn open_verified_process(
    expected: WindowsProcessIdentity,
) -> Result<Option<OwnedProcessHandle>, String> {
    use windows_sys::Win32::System::Threading::{
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    let Some(handle) = open_process_handle(
        expected.pid,
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
    )?
    else {
        return Ok(None);
    };
    let observed = creation_time_from_handle(handle.0)
        .map(|creation_time| {
            WindowsIdentityObservation::Verified(WindowsProcessIdentity {
                pid: expected.pid,
                creation_time,
            })
        })
        .unwrap_or(WindowsIdentityObservation::Unverified);
    terminate_if_windows_identity_matches(expected, observed, || Ok(()))?;
    Ok(Some(handle))
}

fn terminate_open_process(process: &mut KnownProcess) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{FALSE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};

    let observed = creation_time_from_handle(process.handle.0)
        .map(|creation_time| {
            WindowsIdentityObservation::Verified(WindowsProcessIdentity {
                pid: process.identity.pid,
                creation_time,
            })
        })
        .unwrap_or(WindowsIdentityObservation::Unverified);
    terminate_if_windows_identity_matches(process.identity, observed, || {
        let terminated = unsafe { TerminateProcess(process.handle.0, 1) };
        if terminated == FALSE
            && unsafe { WaitForSingleObject(process.handle.0, 0) } != WAIT_OBJECT_0
        {
            return Err(format!(
                "failed to terminate verified Windows process {}: error {}",
                process.identity.pid,
                unsafe { windows_sys::Win32::Foundation::GetLastError() }
            ));
        }
        Ok(())
    })?;
    if unsafe { WaitForSingleObject(process.handle.0, 1_000) } != WAIT_OBJECT_0 {
        return Err(format!(
            "verified Windows process {} did not exit within one second",
            process.identity.pid
        ));
    }
    process.terminated = true;
    Ok(())
}

fn capture_verified_descendants(
    entries: &[WindowsProcessSnapshotEntry],
    known: &mut HashMap<u32, KnownProcess>,
    errors: &mut Vec<String>,
) -> usize {
    let mut added_total = 0;
    loop {
        let identities = known
            .iter()
            .map(|(pid, process)| (*pid, process.identity))
            .collect::<HashMap<_, _>>();
        let (candidates, unverified) = next_verified_windows_descendants(entries, &identities);
        for pid in unverified {
            let message = format!(
                "refusing to terminate descendant PID {pid}: stable identity was unavailable"
            );
            if !errors.contains(&message) {
                errors.push(message);
            }
        }
        if candidates.is_empty() {
            break;
        }
        let mut added = 0;
        for identity in candidates {
            let parent_pid = entries
                .iter()
                .find(|entry| entry.identity == Some(identity))
                .map(|entry| entry.parent_pid);
            let Some(parent_pid) = parent_pid else {
                continue;
            };
            let depth = known
                .get(&parent_pid)
                .map(|parent| parent.depth + 1)
                .unwrap_or(1);
            match open_verified_process(identity) {
                Ok(Some(handle)) => {
                    known.insert(
                        identity.pid,
                        KnownProcess {
                            identity,
                            handle,
                            depth,
                            terminated: false,
                        },
                    );
                    added += 1;
                }
                Ok(None) => {
                    // Natural exit between enumeration and opening is safe.
                }
                Err(error) => errors.push(error),
            }
        }
        added_total += added;
        if added == 0 {
            break;
        }
    }
    added_total
}

/// Terminate the Windows process tree only when the root still has the
/// creation identity persisted at spawn time. Every descendant is likewise
/// opened and identity-checked before termination; handles remain open until
/// teardown ends so none of their PIDs can be recycled under the algorithm.
pub fn terminate_process_tree(pid: u32, expected_creation_time: Option<u64>) -> Result<(), String> {
    let expected_creation_time = expected_creation_time.ok_or_else(|| {
        format!(
            "refusing to terminate Windows process tree {pid}: no stable root identity was recorded"
        )
    })?;
    let root_identity = WindowsProcessIdentity {
        pid,
        creation_time: expected_creation_time,
    };
    let Some(root_handle) = open_verified_process(root_identity)? else {
        return Ok(());
    };
    let mut known = HashMap::from([(
        pid,
        KnownProcess {
            identity: root_identity,
            handle: root_handle,
            depth: 0,
            terminated: false,
        },
    )]);
    let mut errors = Vec::new();

    // Capture all descendants visible before stopping the root. Their open
    // handles bind the enumerated PIDs to those exact process instances.
    let before = process_snapshot()?;
    capture_verified_descendants(&before, &mut known, &mut errors);
    if let Some(root) = known.get_mut(&pid) {
        terminate_open_process(root)?;
    }

    // A descendant can race the first snapshot. Re-snapshot after root exit,
    // capture descendants of every handle-bound process, terminate deepest
    // first, and repeat until a full pass finds no new process. The bound is a
    // cleanup-failure boundary, never permission to kill an ambiguous PID.
    let mut clean_passes = 0;
    for _ in 0..8 {
        let snapshot = process_snapshot()?;
        let added = capture_verified_descendants(&snapshot, &mut known, &mut errors);
        let mut order = known
            .iter()
            .filter_map(|(candidate, process)| {
                (*candidate != pid && !process.terminated).then_some((process.depth, *candidate))
            })
            .collect::<Vec<_>>();
        order.sort_unstable_by(|left, right| right.cmp(left));
        for (_, candidate) in order {
            if let Some(process) = known.get_mut(&candidate) {
                if let Err(error) = terminate_open_process(process) {
                    errors.push(error);
                }
            }
        }
        if added == 0 {
            clean_passes += 1;
            if clean_passes == 2 {
                break;
            }
        } else {
            clean_passes = 0;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if clean_passes < 2 {
        errors.push(format!(
            "managed Windows process tree {pid} did not quiesce within eight identity-checked passes"
        ));
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

/// Record the stable process identity, assign a freshly-spawned harness to a
/// Job Object, and package it into a [`ManagedAgentProcess`]. Identity failure
/// aborts and reaps the child so an unidentifiable process is never registered.
/// Job assignment may still degrade to identity-bound native teardown.
pub fn finish_spawn(
    mut child: std::process::Child,
    log_path: std::path::PathBuf,
    spawn_config: super::spawn_snapshot::SpawnConfigSnapshot,
    setup_mode: bool,
    adapter_availability: Option<super::AcpAvailabilityStatus>,
    start_nonce: String,
    agent_name: &str,
) -> Result<super::ManagedAgentProcess, String> {
    let process_identity = match child_process_identity(&child) {
        Ok(identity) => identity,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "failed to record Windows process identity for agent {agent_name}: {error}"
            ));
        }
    };
    let job = match create_job_for_child(&child) {
        Ok(job) => Some(job),
        Err(error) => {
            eprintln!(
                "buzz-desktop: failed to assign agent {agent_name} to a Job Object; \
                 teardown remains identity-bound: {error}"
            );
            None
        }
    };
    Ok(super::ManagedAgentProcess {
        child,
        log_path,
        spawn_config,
        setup_mode,
        adapter_availability,
        start_nonce,
        process_identity,
        job,
    })
}
