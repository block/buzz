//! Named Windows Job Object identity for detached durable jobs.
//!
//! # Safety
//!
//! This module is exempted from the crate's `#![deny(unsafe_code)]` policy
//! because Win32 Job Object management has no safe Rust wrapper (security
//! decision tracked in block/buzz#6047). Every
//! `unsafe` block is a direct FFI call with three invariants, each held by
//! construction:
//!
//! * Handles returned by `CreateJobObjectW`/`OpenJobObjectW`/`OpenProcess`
//!   are owned solely by this module and closed exactly once on every path
//!   (including errors) via `CloseHandle`.
//! * Structs passed to query/set calls (`JOBOBJECT_*`) are stack-allocated
//!   with `size_of::<T>()` as the byte length, matching the ABI layout
//!   `windows_sys` re-declares.
//! * Pointer arguments to FFI calls are derived from initialized locals or
//!   NUL-terminated wide strings; no pointer arithmetic is performed.
//!
//! Windows-only (`#[cfg(windows)]`); not compiled on other targets.

use std::io;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, FALSE, HANDLE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation, OpenJobObjectW,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_QUERY, JOB_OBJECT_TERMINATE,
};
use windows_sys::Win32::System::Memory::LocalFree;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

const OWNER_ONLY_JOB_SDDL: &str = "D:P(A;;GA;;;OW)";

pub(crate) fn job_name(runtime_id: &str, job_id: uuid::Uuid, attempt: u32) -> String {
    format!("Local\\BuzzJob-{runtime_id}-{job_id}-{attempt}")
}

pub(crate) struct NamedJobObject {
    handle: HANDLE,
    name: String,
}

impl std::fmt::Debug for NamedJobObject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NamedJobObject")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl NamedJobObject {
    /// Creates a new protected, current-owner-only named job and assigns this
    /// runner before it launches the governed command tree.
    pub(crate) fn create_for_current(name: String) -> io::Result<Self> {
        let wide_name = wide(&name);
        let wide_sddl = wide(OWNER_ONLY_JOB_SDDL);
        let mut descriptor = null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide_sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if converted == FALSE {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: FALSE,
        };
        let handle = unsafe { CreateJobObjectW(&mut attributes, wide_name.as_ptr()) };
        let create_error = unsafe { GetLastError() };
        unsafe {
            LocalFree(descriptor as _);
        }
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        if create_error == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "durable job object already exists",
            ));
        }

        let configured = unsafe {
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == FALSE {
            let error = io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
        let assigned = unsafe { AssignProcessToJobObject(handle, GetCurrentProcess()) };
        if assigned == FALSE {
            let error = io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
        Ok(Self { handle, name })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn has_other_active_processes(&self) -> io::Result<bool> {
        let mut accounting = unsafe { zeroed::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() };
        let queried = unsafe {
            QueryInformationJobObject(
                self.handle,
                JobObjectBasicAccountingInformation,
                std::ptr::addr_of_mut!(accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            )
        };
        if queried == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(accounting.ActiveProcesses > 1)
    }

    pub(crate) fn terminate_all(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.handle, 137) } == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    /// Leaves the handle for process teardown after the governed command has
    /// been reaped. Dropping it while the runner is still assigned would make
    /// KILL_ON_JOB_CLOSE terminate the successful runner itself.
    pub(crate) fn disarm(self) {
        std::mem::forget(self);
    }
}

impl Drop for NamedJobObject {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
        }
    }
}

/// Reopens the exact named object and proves that the recorded runner PID is a
/// member. This is the Windows half of PID/start-marker fencing.
pub(crate) fn verify_member(name: &str, runner_pid: u32) -> io::Result<()> {
    with_verified_job(name, runner_pid, |_| Ok(()))
}

/// Terminates only after reopening the named object and verifying membership.
pub(crate) fn terminate_verified(name: &str, runner_pid: u32) -> io::Result<()> {
    with_verified_job(name, runner_pid, |job| {
        if unsafe { TerminateJobObject(job, 137) } == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
}
pub(crate) fn is_empty(name: &str) -> io::Result<bool> {
    let wide_name = wide(name);
    let job = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, wide_name.as_ptr()) };
    if job.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
            return Ok(true);
        }
        return Err(error);
    }
    let mut accounting = unsafe { zeroed::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() };
    let queried = unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicAccountingInformation,
            std::ptr::addr_of_mut!(accounting).cast(),
            size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            null_mut(),
        )
    };
    unsafe {
        CloseHandle(job);
    }
    if queried == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(accounting.ActiveProcesses == 0)
}

fn with_verified_job<T>(
    name: &str,
    runner_pid: u32,
    action: impl FnOnce(HANDLE) -> io::Result<T>,
) -> io::Result<T> {
    let wide_name = wide(name);
    let job = unsafe {
        OpenJobObjectW(
            JOB_OBJECT_QUERY | JOB_OBJECT_TERMINATE,
            FALSE,
            wide_name.as_ptr(),
        )
    };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, runner_pid) };
    if process.is_null() {
        let error = io::Error::last_os_error();
        unsafe { CloseHandle(job) };
        return Err(error);
    }
    let mut member = FALSE;
    let checked = unsafe { IsProcessInJob(process, job, &mut member) };
    unsafe {
        CloseHandle(process);
    }
    if checked == FALSE || member == FALSE {
        let error = if checked == FALSE {
            io::Error::last_os_error()
        } else {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runner is not a job member",
            )
        };
        unsafe { CloseHandle(job) };
        return Err(error);
    }
    let result = action(job);
    unsafe { CloseHandle(job) };
    result
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_job_is_reopenable_and_contains_current_runner() {
        let name = job_name("test-runtime", uuid::Uuid::new_v4(), 1);
        let job = NamedJobObject::create_for_current(name.clone()).expect("create named job");
        assert_eq!(job.name(), name);
        verify_member(job.name(), std::process::id()).expect("verify current process membership");
        job.disarm();
    }
}
