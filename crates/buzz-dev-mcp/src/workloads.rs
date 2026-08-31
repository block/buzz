//! Connection-owned shell work. EOF is a cancellation boundary, not permission
//! to drop the runtime while independently grouped shell children still run.
use std::io;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;
use tokio_util::task::task_tracker::TaskTrackerToken;
use tokio_util::task::TaskTracker;

#[derive(Default)]
pub(crate) struct Workloads {
    closed: Mutex<bool>,
    failed: std::sync::atomic::AtomicBool,
    tasks: TaskTracker,
    pub(crate) cancel: CancellationToken,
}

impl Workloads {
    pub(crate) fn enter(&self) -> Option<TaskTrackerToken> {
        let closed = self.closed.lock().ok()?;
        if *closed {
            None
        } else {
            // Admission and close are serialized: wait cannot miss a shell
            // whose handler was scheduled just as the transport ended.
            Some(self.tasks.token())
        }
    }

    pub(crate) fn close(&self) {
        if let Ok(mut closed) = self.closed.lock() {
            *closed = true;
        }
        self.cancel.cancel();
        self.tasks.close();
    }

    pub(crate) fn child(&self) -> OwnedWork<'_> {
        OwnedWork {
            owner: self,
            complete: false,
        }
    }

    pub(crate) async fn drain(&self) -> io::Result<()> {
        self.close();
        tokio::time::timeout(std::time::Duration::from_secs(3), self.tasks.wait())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "shell cleanup did not finish"))?;
        if self.failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(io::Error::other("owned shell cleanup unconfirmed"));
        }
        Ok(())
    }
}

/// Observe input EOF before rmcp's response-drain timeout. That gives the shell
/// owner time to kill its process group, wait its child and join output readers.
pub(crate) struct CancelOnEof<R> {
    pub(crate) reader: R,
    pub(crate) state: std::sync::Arc<super::shell::SharedState>,
}

impl<R: AsyncRead + Unpin> AsyncRead for CancelOnEof<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let remaining = buf.remaining();
        let result = Pin::new(&mut this.reader).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Err(_)))
            || (matches!(result, Poll::Ready(Ok(())))
                && remaining > 0
                && buf.filled().len() == before)
        {
            this.state.workloads.close();
        }
        result
    }
}

#[cfg(test)]
mod tests;

/// Dropping a shell future is not completion evidence.
pub(crate) struct OwnedWork<'a> {
    owner: &'a Workloads,
    complete: bool,
}
impl OwnedWork<'_> {
    pub(crate) async fn finish(&mut self, pid: Option<u32>, reaped: bool) {
        if !reaped {
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = pid {
            use nix::{errno::Errno, sys::signal::killpg, unistd::Pid};
            for _ in 0..50 {
                if killpg(Pid::from_raw(pid as i32), None) == Err(Errno::ESRCH) {
                    self.complete = true;
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
        // Other platforms lack a supported observation here; fail closed.
        #[cfg(not(unix))]
        let _ = pid;
    }
}
impl Drop for OwnedWork<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.owner
                .failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
}
