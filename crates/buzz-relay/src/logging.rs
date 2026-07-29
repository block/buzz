//! Non-blocking stdout log writer.
//!
//! Why this exists: the relay logs JSON lines to stdout, which in Kubernetes
//! is a pipe drained by the container runtime's log copier. If that copier
//! stalls (node disk-IO pressure, log rotation), the ~64KB pipe fills and the
//! next `write(2)` blocks **while holding Rust's global stdout lock**. Every
//! runtime worker that logs then queues behind it, the whole Tokio runtime
//! parks, liveness probes time out, SIGTERM (an async task) can never run,
//! and kubelet SIGKILLs the pod after the full grace period. Reproduced
//! deterministically by SIGSTOP-ing a pipe consumer; see
//! `RESEARCH/RELAY_STALL_LOCAL_REPRO_2026_07_27.md` (buzz nest).
//!
//! The fix moves the only blocking syscall onto one dedicated OS thread
//! behind a bounded queue ([`tracing_appender::non_blocking`], lossy mode).
//! When the sink stalls, log lines are dropped instead of the relay dying.
//! Drops are observable via the `buzz_log_lines_dropped_total` counter —
//! which doubles as a permanent detector for copier stalls.
//!
//! Loss policy is deliberate: there is no lossless bounded non-blocking
//! design under an indefinitely stalled consumer. Availability outranks log
//! completeness. Non-lossy mode would recreate the deadlock at queue
//! capacity instead of pipe capacity.

use std::time::Duration;

use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};

/// Bounded queue size, in lines.
///
/// Derived from measured production line sizes (bb-block fleet, 2026-07-28,
/// 26k-line sample: p50=248B, largest observed 641B — p99 equaled max in
/// that sample, so the true tail may be somewhat larger): 4096 × 641B ≈
/// 2.6MB of queue memory against a 2Gi pod limit, several minutes of
/// steady-state log volume, with ample headroom for a fatter tail. The
/// buffer absorbs bursts; it is not sized to
/// ride out a sustained copier outage — once the sink is stalled, preserving
/// more backlog has diminishing value.
///
/// If a new log site can emit large unbounded fields (request bodies, event
/// payloads), cap the field at the call site — the queue is line-count
/// bounded, so line size is the memory multiplier.
const BUFFERED_LINES_LIMIT: usize = 4096;

/// How often the drop counter is polled into the metrics recorder.
const DROP_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Upper bound on how long guard drop may wait for the upstream flush.
///
/// The upstream [`WorkerGuard::drop`] is internally bounded at ~1.1s on its
/// healthy paths (100ms shutdown send + 1s flush wait), so 2s only fires when
/// the wedge described on [`BoundedWorkerGuard`] is actually happening.
const GUARD_DROP_TIMEOUT: Duration = Duration::from_secs(2);

/// A [`WorkerGuard`] whose drop is guaranteed to return in bounded time even
/// when stdout itself is wedged.
///
/// Why upstream drop is not safe here: when the queue is full at shutdown,
/// `WorkerGuard::drop` (tracing-appender 0.2.5, `non_blocking.rs:296`) times
/// out its 100ms shutdown send and then calls `println!` — into the exact
/// stdout whose lock the worker thread is holding while parked in `write(2)`
/// on the full pipe. That deadlocks the dropping thread forever, recreating
/// the original SIGKILL-on-shutdown failure in the one code path this module
/// exists to protect.
///
/// The fix: run the upstream drop on a sacrificial named thread and wait at
/// most [`GUARD_DROP_TIMEOUT`]. Healthy shutdown keeps the full flush (the
/// upstream drop finishes in ≲1.1s and we return as soon as it does). If the
/// pipe is wedged, we abandon the helper thread and return — queued lines are
/// lost, which is this module's stated loss policy, and process exit reaps
/// the thread.
pub struct BoundedWorkerGuard {
    inner: Option<WorkerGuard>,
}

impl Drop for BoundedWorkerGuard {
    fn drop(&mut self) {
        let Some(guard) = self.inner.take() else {
            return;
        };
        // ManuallyDrop so that if the thread cannot be spawned, dropping the
        // unused closure leaks the guard instead of running the wedge-prone
        // upstream drop on this thread. Leaked guard = lost queued lines,
        // never a hang.
        let guard = std::mem::ManuallyDrop::new(guard);
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("log-guard-drop".into())
            .spawn(move || {
                drop(std::mem::ManuallyDrop::into_inner(guard));
                let _ = done_tx.send(());
            });
        if spawned.is_ok() {
            // Ok(()) = flush completed; Err(Timeout) = wedged, abandon it;
            // Err(Disconnected) = helper finished (send lost the race).
            let _ = done_rx.recv_timeout(GUARD_DROP_TIMEOUT);
        }
    }
}

/// Wrap stdout in a lossy bounded non-blocking writer.
///
/// The returned [`BoundedWorkerGuard`] must be bound to a **named** variable
/// in `main` (e.g. `_log_guard`) and held for the process lifetime: binding
/// it to bare `_` drops it immediately, shutting down the writer thread and
/// silently discarding all subsequent log output. On drop the guard flushes
/// buffered lines when stdout is healthy, and gives up after
/// [`GUARD_DROP_TIMEOUT`] when it is not — shutdown is bounded either way.
pub fn non_blocking_stdout() -> (NonBlocking, BoundedWorkerGuard) {
    let (writer, guard) = NonBlockingBuilder::default()
        .lossy(true)
        .buffered_lines_limit(BUFFERED_LINES_LIMIT)
        .finish(std::io::stdout());
    (writer, BoundedWorkerGuard { inner: Some(guard) })
}

/// Periodically export the writer's cumulative drop count as the monotonic
/// counter `buzz_log_lines_dropped_total`.
///
/// Any positive rate means the stdout pipe is (or was) stalled and lines
/// were shed to protect the runtime — alert on it. Deliberately does NOT
/// log: the log channel is exactly what has failed when this fires.
pub fn spawn_drop_counter_poller(errors: ErrorCounter) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(DROP_POLL_INTERVAL);
        let mut last = 0usize;
        loop {
            interval.tick().await;
            let current = errors.dropped_lines();
            let delta = current.saturating_sub(last);
            if delta > 0 {
                metrics::counter!("buzz_log_lines_dropped_total").increment(delta as u64);
            }
            last = current;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    /// A writer that blocks in `write` while `blocked` is true — the shape of
    /// a full stdout pipe with a stalled consumer.
    #[derive(Clone)]
    struct StallableWriter {
        state: Arc<(Mutex<bool>, Condvar)>,
        bytes_written: Arc<AtomicUsize>,
    }

    impl StallableWriter {
        fn new(blocked: bool) -> Self {
            Self {
                state: Arc::new((Mutex::new(blocked), Condvar::new())),
                bytes_written: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn unblock(&self) {
            let (lock, cvar) = &*self.state;
            *lock.lock().unwrap() = false;
            cvar.notify_all();
        }
    }

    impl Write for StallableWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let (lock, cvar) = &*self.state;
            let mut blocked = lock.lock().unwrap();
            while *blocked {
                blocked = cvar.wait(blocked).unwrap();
            }
            self.bytes_written.fetch_add(buf.len(), Ordering::SeqCst);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn lossy_writer(sink: StallableWriter, limit: usize) -> (NonBlocking, WorkerGuard) {
        NonBlockingBuilder::default()
            .lossy(true)
            .buffered_lines_limit(limit)
            .finish(sink)
    }

    /// The invariant that motivated this module: with the sink fully stalled
    /// and the queue driven past capacity, async tasks must stay responsive,
    /// drops must be counted, and output must resume when the sink unblocks.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocked_sink_never_stalls_runtime_and_recovers() {
        let sink = StallableWriter::new(true);
        let limit = 8;
        let (writer, guard) = lossy_writer(sink.clone(), limit);
        let errors = writer.error_counter();

        // Drive the queue well past capacity while the sink is stalled.
        // (The worker thread is wedged in `write`, so nothing drains.)
        let mut w = writer.clone();
        for i in 0..(limit * 4) {
            let line = format!("{{\"n\":{i}}}\n");
            let _ = w.write(line.as_bytes());
        }

        // Liveness: an async round-trip must complete promptly even though
        // the log sink is fully wedged. This is the exact property the
        // blocking stdout writer violated.
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = tx.send(());
        });
        tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .expect("async runtime stalled while log sink was blocked")
            .unwrap();

        // Overflow was shed and counted, bounding memory at `limit` lines.
        assert!(
            errors.dropped_lines() > 0,
            "expected dropped lines while sink was stalled"
        );

        // Recovery: unblock the sink; queued lines drain to the writer.
        sink.unblock();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while sink.bytes_written.load(Ordering::SeqCst) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "sink did not receive any bytes after unblocking"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(guard);
    }

    /// Normal path: tracing events written through the non-blocking writer
    /// reach the sink intact (whole-line, valid JSON) — and the guard flush
    /// delivers lines still queued at shutdown.
    #[test]
    fn tracing_events_reach_sink_end_to_end() {
        use tracing_subscriber::{fmt, prelude::*};

        let sink = StallableWriter::new(false);
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

        // Wrap the sink so we can inspect the exact bytes.
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>, StallableWriter);
        impl Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                self.1.write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.1.flush()
            }
        }

        let (writer, guard) = NonBlockingBuilder::default()
            .lossy(true)
            .buffered_lines_limit(BUFFERED_LINES_LIMIT)
            .finish(Capture(captured.clone(), sink));

        let subscriber = tracing_subscriber::registry()
            .with(fmt::layer().json().flatten_event(true).with_writer(writer));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(probe = "delivery", "non-blocking writer end-to-end");
        });

        // Guard drop flushes the queue.
        drop(guard);

        let bytes = captured.lock().unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let line = text
            .lines()
            .find(|l| l.contains("non-blocking writer end-to-end"))
            .expect("log line did not reach the sink");
        let parsed: serde_json::Value =
            serde_json::from_str(line).expect("log line reached sink but is not valid JSON");
        assert_eq!(parsed["probe"], "delivery");
    }

    /// An oversized line is still admitted whole-line atomically (or dropped
    /// whole): the queue is line-count bounded, so one giant line must never
    /// be split into partial writes.
    #[test]
    fn oversized_line_is_whole_line_atomic() {
        let sink = StallableWriter::new(false);
        let (writer, guard) = lossy_writer(sink.clone(), 4);

        let big = format!("{{\"big\":\"{}\"}}\n", "x".repeat(1024 * 1024));
        let mut w = writer.clone();
        let n = w.write(big.as_bytes()).unwrap();
        assert_eq!(n, big.len(), "write must report the full line accepted");

        drop(guard); // flush
        assert_eq!(
            sink.bytes_written.load(Ordering::SeqCst),
            big.len(),
            "sink must receive the entire line, not a fragment"
        );
    }

    /// Healthy-shutdown path of the bounded guard: dropping it must still
    /// deliver the upstream flush (queued lines reach the sink).
    #[test]
    fn bounded_guard_flushes_on_healthy_shutdown() {
        let sink = StallableWriter::new(false);
        let (writer, inner) = lossy_writer(sink.clone(), 8);
        let bounded = BoundedWorkerGuard { inner: Some(inner) };

        let line = b"{\"probe\":\"bounded-guard-flush\"}\n";
        let mut w = writer.clone();
        w.write_all(line).unwrap();

        drop(bounded);
        assert_eq!(
            sink.bytes_written.load(Ordering::SeqCst),
            line.len(),
            "bounded guard drop must flush queued lines on a healthy sink"
        );
    }

    /// Regression for the review finding on PR #3256: upstream
    /// `WorkerGuard::drop` deadlocks when the queue is full and stdout is
    /// wedged — its 100ms shutdown send times out and it then `println!`s
    /// into the very stdout whose lock the worker holds while parked in
    /// `write(2)` on the full pipe.
    ///
    /// This test re-executes itself as a child process whose stdout is a
    /// pipe that is deliberately **never read** (the exact production
    /// failure: stalled container-runtime log copier). The child saturates
    /// the pipe *and* the bounded queue (proven via `dropped_lines() > 0`,
    /// reported over stderr — stdout can't carry a readiness signal once
    /// wedged), then drops the guard and exits. The parent asserts a clean
    /// exit within the deadline. With the upstream guard drop run inline,
    /// this test hangs the child and fails on the deadline (verified).
    #[test]
    fn guard_drop_bounded_with_wedged_stdout_through_process_exit() {
        const CHILD_ENV: &str = "BUZZ_TEST_LOG_GUARD_WEDGE_CHILD";
        if std::env::var_os(CHILD_ENV).is_some() {
            wedged_child();
        }

        // libtest names tests relative to the crate root: "logging::tests::…"
        // (module_path!() = "buzz_relay::logging::tests").
        let test_name = format!(
            "{}::guard_drop_bounded_with_wedged_stdout_through_process_exit",
            module_path!()
                .split_once("::")
                .expect("module path has crate prefix")
                .1
        );
        let mut child =
            std::process::Command::new(std::env::current_exe().expect("current test binary path"))
                .args([test_name.as_str(), "--exact", "--nocapture"])
                .env(CHILD_ENV, "1")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn wedged child");

        // CRITICAL: keep the read end of the child's stdout open but never
        // read from it — that is the wedge. Reading (e.g. via
        // `wait_with_output`) would drain the pipe, un-wedge the worker, and
        // test nothing. Closing it would turn the child's writes into EPIPE
        // instead of blocking.
        let _wedged_stdout = child.stdout.take().expect("child stdout handle");

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().expect("poll child") {
                break status;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("child did not exit within deadline: guard drop wedged on blocked stdout");
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        // Child has exited; its small stderr output is fully buffered in the
        // pipe, so this read is bounded.
        let mut stderr = String::new();
        use std::io::Read as _;
        child
            .stderr
            .take()
            .expect("child stderr handle")
            .read_to_string(&mut stderr)
            .expect("read child stderr");
        assert!(
            stderr.contains("SATURATED"),
            "child exited without reaching queue saturation — test proved nothing:\n{stderr}"
        );
        assert!(
            status.success(),
            "wedged child exited unsuccessfully ({status:?}):\n{stderr}"
        );
    }

    /// Child half of the wedged-shutdown regression. Runs with stdout piped
    /// and never read; must saturate pipe + queue, drop the guard, and exit.
    fn wedged_child() -> ! {
        // Production constructor: real stdout, real limits.
        let (writer, guard) = non_blocking_stdout();
        let errors = writer.error_counter();

        // Fill the (unread) stdout pipe, then the bounded queue, until lines
        // are provably dropping — i.e. the queue is full and the worker is
        // parked in write(2). Generously bounded loop so a broken setup
        // fails instead of spinning forever.
        let line = format!("{{\"pad\":\"{}\"}}\n", "x".repeat(100));
        let mut w = writer.clone();
        for _ in 0..500_000 {
            let _ = w.write(line.as_bytes());
            if errors.dropped_lines() > 0 {
                break;
            }
        }
        if errors.dropped_lines() == 0 {
            eprintln!("FAILED to saturate queue");
            std::process::exit(2);
        }
        // stdout is wedged by construction; readiness goes out on stderr.
        eprintln!("SATURATED dropped={}", errors.dropped_lines());

        let start = std::time::Instant::now();
        drop(guard);
        eprintln!("GUARD_DROPPED elapsed_ms={}", start.elapsed().as_millis());
        std::process::exit(0);
    }

    /// The drop-counter poller translates cumulative dropped_lines into
    /// monotonic counter increments without ever logging.
    #[tokio::test]
    async fn drop_counter_deltas_are_computed_from_cumulative_count() {
        // Exercise the delta logic directly (the spawn wrapper is trivial).
        let sink = StallableWriter::new(true);
        let (writer, _guard) = lossy_writer(sink.clone(), 2);
        let errors = writer.error_counter();

        let mut w = writer.clone();
        for _ in 0..10 {
            let _ = w.write(b"{\"x\":1}\n");
        }
        let first = errors.dropped_lines();
        assert!(first > 0);

        for _ in 0..10 {
            let _ = w.write(b"{\"x\":1}\n");
        }
        let second = errors.dropped_lines();
        assert!(second > first, "cumulative counter must keep growing");
        assert_eq!(second.saturating_sub(first), 10);
        sink.unblock();
    }
}
