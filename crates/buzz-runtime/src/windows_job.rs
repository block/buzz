//! Windows Job Object ownership for short-lived managed process trees.
//!
//! Children are created suspended, assigned to the Job Object, and only then
//! resumed. Closing the last handle is a crash-safe fallback; orderly shutdown
//! explicitly terminates and verifies that the object is empty.

use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_NO_MORE_FILES, FALSE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, OpenThread, ResumeThread, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    THREAD_SUSPEND_RESUME,
};

/// Windows creation flag that prevents child code from running before Job assignment.
pub const CREATE_SUSPENDED: u32 = 0x0000_0004;
/// Windows creation flag used by GUI parents to avoid allocating a console window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// An anonymous, kill-on-close Job Object for one managed adapter or MCP tree.
#[derive(Debug)]
pub struct WindowsJobObject {
    handle: OwnedHandle,
}

impl WindowsJobObject {
    /// Creates an empty Job Object configured for crash-safe tree cleanup.
    #[allow(unsafe_code)] // Win32 Job Object FFI exception per block/buzz#6047

    pub fn create_kill_on_close() -> io::Result<Self> {
        // SAFETY: null security/name pointers request an anonymous object. A
        // successful handle is immediately transferred to OwnedHandle.
        let raw = unsafe { CreateJobObjectW(null(), null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a newly owned, non-null Windows handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
        let job = Self { handle };

        // SAFETY: the input buffer is initialized, correctly sized, and lives
        // for the duration of SetInformationJobObject.
        let configured = unsafe {
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Adds `CREATE_SUSPENDED` and any caller flags to a Tokio command.
    pub fn prepare_command(command: &mut tokio::process::Command, additional_flags: u32) {
        command.creation_flags(CREATE_SUSPENDED | additional_flags);
    }
    /// Assigns the exact suspended child handle to this job, then resumes it.
    ///
    /// The child must have been configured through [`Self::prepare_command`].
    /// Assignment uses the process handle retained by Tokio, never a PID lookup,
    /// so PID reuse cannot retarget ownership to an unrelated process.
    #[allow(unsafe_code)] // Win32 Job Object FFI exception per block/buzz#6047

    pub fn assign_spawned_child_and_resume(&self, child: &tokio::process::Child) -> io::Result<()> {
        let pid = child
            .id()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "spawned child has no PID"))?;
        let process = child.raw_handle().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "spawned child has no process handle",
            )
        })?;

        // SAFETY: `process` is the exact live handle borrowed from `child`;
        // this job handle remains valid for the duration of the call.
        if unsafe { AssignProcessToJobObject(self.raw(), process.cast()) } == FALSE {
            return Err(io::Error::last_os_error());
        }

        resume_process_threads(pid)
    }

    /// Assigns a caller-owned suspended process when its wrapper does not expose
    /// the process handle (for example, RMCP's child transport), then resumes it.
    ///
    /// The PID must come directly from the successful suspended spawn. This
    /// method never terminates by PID; all later cleanup targets this Job Object.
    #[allow(unsafe_code)] // Win32 Job Object FFI exception per block/buzz#6047

    pub fn assign_spawned_pid_and_resume(&self, pid: u32) -> io::Result<()> {
        // SAFETY: OpenProcess validates the PID and requested access. The
        // returned handle is immediately transferred to OwnedHandle.
        let raw_process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, pid) };
        if raw_process.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: raw_process is non-null and newly owned by this call.
        let process = unsafe { OwnedHandle::from_raw_handle(raw_process.cast()) };

        // SAFETY: both handles are valid for this call.
        if unsafe { AssignProcessToJobObject(self.raw(), process.as_raw_handle().cast()) } == FALSE
        {
            return Err(io::Error::last_os_error());
        }

        resume_process_threads(pid)
    }

    /// Returns the number of processes currently associated with the job.
    #[allow(unsafe_code)] // Win32 Job Object FFI exception per block/buzz#6047

    pub fn active_process_count(&self) -> io::Result<u32> {
        // SAFETY: the output buffer is initialized, correctly sized, and lives
        // for the duration of QueryInformationJobObject.
        unsafe {
            let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
            if QueryInformationJobObject(
                self.raw(),
                JobObjectBasicAccountingInformation,
                std::ptr::addr_of_mut!(accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            ) == FALSE
            {
                return Err(io::Error::last_os_error());
            }
            Ok(accounting.ActiveProcesses)
        }
    }

    /// Terminates every process associated with this object.
    #[allow(unsafe_code)] // Win32 Job Object FFI exception per block/buzz#6047

    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: the Job Object handle is valid for this call.
        if unsafe { TerminateJobObject(self.raw(), 137) } == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Waits boundedly until the Job Object reports no active processes.
    pub async fn wait_empty(&self, timeout: Duration) -> io::Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.active_process_count()? == 0 {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Windows Job Object did not become empty before the deadline",
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Terminates the tree and only succeeds after bounded empty-tree proof.
    pub async fn terminate_and_wait_empty(&self, timeout: Duration) -> io::Result<()> {
        self.terminate()?;
        self.wait_empty(timeout).await
    }

    fn raw(&self) -> HANDLE {
        self.handle.as_raw_handle().cast()
    }
}

#[allow(unsafe_code)] // Win32 Job Object FFI exception per block/buzz#6047

fn resume_process_threads(pid: u32) -> io::Result<()> {
    // SAFETY: the returned snapshot is either INVALID_HANDLE_VALUE or a newly
    // owned snapshot handle.
    let raw_snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw_snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: checked above; OwnedHandle closes the snapshot exactly once.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw_snapshot.cast()) };
    // SAFETY: zeroed THREADENTRY32 with dwSize initialized is the documented
    // iteration contract for Thread32First/Thread32Next.
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0u32;

    // SAFETY: snapshot and entry pointers remain valid through iteration.
    let mut found = unsafe { Thread32First(snapshot.as_raw_handle().cast(), &mut entry) };
    if found == FALSE {
        // SAFETY: GetLastError has no preconditions.
        let error = unsafe { GetLastError() };
        if error == ERROR_NO_MORE_FILES {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "suspended child has no discoverable thread",
            ));
        }
        return Err(io::Error::from_raw_os_error(error as i32));
    }

    loop {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: OpenThread returns a new owned handle or null.
            let raw_thread =
                unsafe { OpenThread(THREAD_SUSPEND_RESUME, FALSE, entry.th32ThreadID) };
            if raw_thread.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `raw_thread` is newly owned and non-null.
            let thread = unsafe { OwnedHandle::from_raw_handle(raw_thread.cast()) };
            // SAFETY: thread has THREAD_SUSPEND_RESUME access.
            if unsafe { ResumeThread(thread.as_raw_handle().cast()) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            resumed = resumed.saturating_add(1);
        }

        // SAFETY: snapshot and entry remain valid.
        found = unsafe { Thread32Next(snapshot.as_raw_handle().cast(), &mut entry) };
        if found == FALSE {
            break;
        }
    }

    if resumed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "suspended child thread was not found",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    fn command(script: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("cmd.exe");
        command.args(["/D", "/S", "/C", script]);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command
    }

    async fn spawn_in(job: &WindowsJobObject, script: &str) -> tokio::process::Child {
        let mut command = command(script);
        WindowsJobObject::prepare_command(&mut command, CREATE_NO_WINDOW);
        let mut child = command.spawn().expect("spawn suspended child");
        if let Err(error) = job.assign_spawned_child_and_resume(&child) {
            let _ = child.start_kill();
            let _ = child.wait().await;
            panic!("assign and resume child: {error}");
        }
        child
    }

    #[tokio::test]
    async fn normal_exit_leaves_verified_empty_job() {
        let job = WindowsJobObject::create_kill_on_close().expect("create job");
        let mut child = spawn_in(&job, "exit /b 0").await;
        job.wait_empty(Duration::from_secs(5))
            .await
            .expect("normal exit emptied job");
        child.wait().await.expect("wait normally exited child");
    }

    #[tokio::test]
    async fn terminate_kills_descendant_and_verifies_empty_tree() {
        let job = WindowsJobObject::create_kill_on_close().expect("create job");
        let mut child = spawn_in(
            &job,
            "start \"\" /B ping.exe -t 127.0.0.1 >NUL & ping.exe -t 127.0.0.1 >NUL",
        )
        .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while job.active_process_count().expect("query job") < 2 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "descendant did not join job"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        job.terminate_and_wait_empty(Duration::from_secs(5))
            .await
            .expect("terminate and verify tree");
        let _ = child.wait().await;
        assert_eq!(job.active_process_count().expect("query job"), 0);
    }
    #[tokio::test]
    async fn kill_on_close_cleans_tree_when_owner_drops_abnormally() {
        let job = WindowsJobObject::create_kill_on_close().expect("create job");
        let mut child = spawn_in(&job, "ping.exe -t 127.0.0.1 >NUL").await;
        assert_eq!(job.active_process_count().expect("query job"), 1);
        drop(job);
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("kill-on-close did not stop child")
            .expect("wait child");
    }

    #[tokio::test]
    async fn terminating_one_job_does_not_touch_independent_job() {
        let adapter = WindowsJobObject::create_kill_on_close().expect("adapter job");
        let durable = WindowsJobObject::create_kill_on_close().expect("durable job");
        let mut adapter_child = spawn_in(&adapter, "ping.exe -t 127.0.0.1 >NUL").await;
        let mut durable_child = spawn_in(&durable, "ping.exe -t 127.0.0.1 >NUL").await;

        adapter
            .terminate_and_wait_empty(Duration::from_secs(5))
            .await
            .expect("terminate adapter tree");
        let _ = adapter_child.wait().await;
        assert_eq!(adapter.active_process_count().expect("query adapter"), 0);
        assert_eq!(durable.active_process_count().expect("query durable"), 1);

        durable
            .terminate_and_wait_empty(Duration::from_secs(5))
            .await
            .expect("cleanup durable tree");
        let _ = durable_child.wait().await;
    }
}
