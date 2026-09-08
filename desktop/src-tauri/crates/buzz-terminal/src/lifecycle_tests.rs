//! Lifecycle gates: the session dies, the grandchild dies with it, and the
//! login `argv[0]` the child actually receives is the one we computed.
//!
//! Every test here drives a real PTY and a real process tree. A mock child
//! would let us assert that we *called* `kill`, which is the half we already
//! know; the property under test is what the kernel does with a process group
//! we do not fully control.

use crate::env_fence::fence_env;
use crate::lifecycle::{shutdown, shutdown_draining, DrainingReader, Shutdown, TERM_GRACE};
use crate::path::user_shell_path;
use crate::shell::{login_argv0, resolve_shell};
use portable_pty::{native_pty_system, Child, CommandBuilder, PtyPair, PtySize};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Upper bound on any wait in this file.
///
/// Every wait here is bounded, and that is not caution -- it is the lesson
/// from a probe of an interactive child that read to EOF and hung for 300 s.
/// A PTY master does not reach EOF while any process holds the slave open, so
/// "read until the child is done" is not a terminating program. Bound the
/// read, or poll for the observable effect.
const BOUND: Duration = Duration::from_secs(10);

/// Self-destruct deadline, in seconds, for fixture processes built to ignore
/// signals.
///
/// Comfortably longer than [`BOUND`], so it can never end a process while the
/// test is still observing it -- a watchdog that fires inside the observation
/// window would make a *failing* implementation look correct. Short enough
/// that a crashed run does not leave a core spinning until reboot.
const WATCHDOG: u64 = 60;

fn open_pty() -> PtyPair {
    native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty")
}

/// Drains the PTY master in the background for as long as it stays open.
///
/// Not hygiene -- a correctness requirement, and the cause of a 300 s hang in
/// the first version of this file. A PTY has a small kernel buffer, and a
/// child writing into a master nobody reads blocks in `write()` once it fills.
/// A process blocked in an uninterruptible tty write does not die promptly on
/// `SIGKILL`: the signal is delivered, but the kernel finishes tearing down
/// the tty session first, so `wait()` sits there while the reap completes.
/// Measured directly with a `forkpty` C probe: with the master undrained, a
/// `SIGKILL`ed child took **606 ms** to be reaped. Every terminal in the
/// product drains its master continuously -- that is what a renderer *is* --
/// so a test that doesn't is modelling a configuration that never ships.
///
/// The consequence is worth stating for the embedder: **shutdown must not be
/// called after the reader has stopped.** Tear the session down while output
/// is still being consumed, or the grace period is spent waiting on a
/// self-inflicted stall.
fn drain(pair: &PtyPair) {
    let mut reader = pair.master.try_clone_reader().expect("reader");
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
    });
}

/// Spawns `script` under `/bin/sh` on a real PTY, fully fenced.
fn spawn_script(pair: &PtyPair, script: &str) -> Box<dyn Child + Send + Sync> {
    let shell = resolve_shell(std::env::var("SHELL").ok().as_deref());
    let mut cmd = CommandBuilder::new("/bin/sh");
    fence_env(&mut cmd, &user_shell_path(), &shell);
    cmd.arg("-c");
    cmd.arg(script);
    let child = pair.slave.spawn_command(cmd).expect("spawn");
    drain(pair);
    child
}

/// True while `pid` exists. `kill(pid, 0)` performs the permission and
/// existence checks without delivering a signal.
fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 delivers nothing; both arguments are integers.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Polls `f` until it returns true or `BOUND` elapses; returns whether it did.
///
/// Polling for the observable state rather than sleeping a fixed duration: a
/// sleep long enough to be reliable is slow, and a sleep short enough to be
/// fast is a race that fails on a loaded machine. Both are worse than asking.
fn poll_until(mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + BOUND;
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// Reads a file until it is non-empty or `BOUND` elapses.
fn read_when_written(path: &std::path::Path) -> Option<String> {
    let mut found = None;
    poll_until(|| match std::fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => {
            found = Some(text.trim().to_owned());
            true
        }
        _ => false,
    });
    found
}

/// A cooperative child exits on `SIGTERM`, so the polite arm is what ends it.
///
/// The distinction matters: `Killed` and `Terminated` both leave a dead
/// process, so asserting death alone would pass with `SIGTERM` deleted
/// entirely and the grace period reduced to a delay before `SIGKILL`.
#[test]
fn cooperative_child_exits_on_term_not_kill() {
    let pair = open_pty();
    let mut child = spawn_script(&pair, "sleep 30");
    let pid = child.process_id().expect("pid") as i32;

    let outcome = shutdown(&mut child).expect("shutdown");
    assert_eq!(
        outcome,
        Shutdown::Terminated,
        "a child that dies on SIGTERM must not have needed SIGKILL"
    );
    assert!(poll_until(|| !pid_alive(pid)), "child survived shutdown");
}

/// A child that ignores `SIGTERM` must still die, and the escalation must be
/// what kills it.
///
/// The fixture shape is load-bearing and my first one was vacuous. I wrote
/// `trap '' TERM; sleep 30`, which *looks* like a signal-ignoring child and
/// reported `Terminated` -- the polite arm, on a child built to defeat it.
/// The reason is that `sh` does not ignore a signal on its child's behalf: the
/// group `SIGTERM` reaches `sleep`, which has no trap and dies, and the shell
/// was blocked in `wait` on exactly that `sleep`, so it reaps it and exits
/// normally. The trap was real, the ignoring was real, and the process still
/// died on `SIGTERM` -- through a path the test wasn't looking at.
///
/// Had I not checked *which* arm fired, this would have passed for the wrong
/// reason and gone on "proving" an escalation it never exercised. The loop
/// keeps the shell itself alive: no blocking `wait` to be interrupted, so the
/// trap actually governs the shell's own fate and only `SIGKILL` can end it.
///
/// The readiness handshake closes a second, subtler version of the same
/// mistake. My loop fixture *still* reported `Terminated`, because `trap` is a
/// command the shell has to reach: a signal delivered in the interval between
/// `exec` and that line finds the default disposition and kills the shell
/// outright. Isolated with a `forkpty` probe -- identical binary, only the
/// delay before signalling changed: at 500 ms all four arms survived, at 2 ms
/// all four died with signal 15. A fixture that is only *probably* armed makes
/// this test a race whose failure mode is a false pass.
///
/// This is the arm that fails if the escalation is deleted -- and the
/// `WATCHDOG` is what makes that a *failure* rather than a hang. With
/// `SIGKILL` deleted, nothing we send can end a child that ignores `SIGTERM`,
/// so `shutdown`'s final `wait` blocks forever and the mutant is detected only
/// by the harness timing out. A test that detects a bug by never finishing is
/// indistinguishable from a broken test. The deadline converts it into a
/// bounded, reportable failure.
#[test]
fn signal_ignoring_child_is_killed_after_the_grace_period() {
    let dir = tempdir("buzz-terminal-trap");
    let ready = dir.join("armed");

    let pair = open_pty();
    // The readiness file is written *after* the trap is installed, so waiting
    // on it converts "probably armed by now" into an observed fact.
    let mut child = spawn_script(
        &pair,
        &format!(
            "trap '' TERM; (sleep {WATCHDOG}; kill -9 $$) & : > {}; \
             while :; do sleep 0.1; done",
            ready.display()
        ),
    );
    let pid = child.process_id().expect("pid") as i32;
    assert!(
        poll_until(|| ready.exists()),
        "child never armed its SIGTERM trap; signalling now would test a \
         startup race rather than the escalation"
    );

    let started = Instant::now();
    let outcome = shutdown(&mut child).expect("shutdown");
    let elapsed = started.elapsed();

    assert_eq!(
        outcome,
        Shutdown::Killed,
        "a SIGTERM-ignoring child must be escalated to SIGKILL"
    );
    assert!(poll_until(|| !pid_alive(pid)), "child survived SIGKILL");
    assert!(
        elapsed >= TERM_GRACE,
        "shutdown returned in {elapsed:?}, before the {TERM_GRACE:?} grace \
         period could have elapsed -- SIGTERM was never given its chance"
    );
    assert!(
        elapsed < BOUND,
        "shutdown took {elapsed:?}; the grace period is not bounded"
    );
}

/// The property the whole module exists for: a **grandchild** must not outlive
/// the session.
///
/// The fixture is deliberately hostile, and the obvious version of this test
/// proves nothing. I first wrote `sleep 30 & echo $!; wait` and mutation L2 --
/// replacing `kill(-pid)` with `kill(pid)` -- **survived it**. The reason is
/// that killing a PTY session leader makes the kernel hang up the terminal and
/// `SIGHUP` the whole foreground group, so the grandchild dies either way.
/// Isolated with a `forkpty` probe: with the master held open (no fd-closure
/// hangup) and only `SIGKILL` to the shell's pid, the grandchild was gone
/// within 200 ms while the shell itself was still unreaped. The tty hangup was
/// doing the work my group signal was being credited for.
///
/// Two properties are therefore required of the grandchild, and each closes
/// one leak in the fixture:
///
/// - it **ignores `SIGHUP`**, so the tty hangup cannot end it for us; and
/// - it **busy-loops rather than sleeping**, so it is not blocked in a call
///   that the session teardown would interrupt anyway.
///
/// With both, the probe separates cleanly: pid-only leaves the grandchild
/// alive, `kill(-pgid)` does not. That is the only shape in which this test
/// can fail for the reason it claims to test.
///
/// The `WATCHDOG` is the price of that hostility. A grandchild built to
/// survive every signal we send also survives the harness: when this test
/// legitimately fails -- as it does under mutation L1 and L2 -- it leaves a
/// process spinning a core at PPID 1, and a panicking or killed test binary
/// cannot clean up after itself. So the child carries its own deadline.
/// `SIGKILL` because that is the one signal the fixture does not trap.
#[test]
fn grandchild_does_not_outlive_the_session() {
    let dir = tempdir("buzz-terminal-orphan");
    let pidfile = dir.join("grandchild.pid");
    let armed = dir.join("armed");

    let pair = open_pty();
    let mut child = spawn_script(
        &pair,
        &format!(
            "sh -c 'trap \"\" HUP TERM; (sleep {WATCHDOG}; kill -9 $$) & \
             : > {armed}; while :; do :; done' & \
             echo $! > {pidfile}; wait",
            armed = armed.display(),
            pidfile = pidfile.display()
        ),
    );
    let shell_pid = child.process_id().expect("pid") as i32;

    let grandchild: i32 = read_when_written(&pidfile)
        .expect("grandchild never reported its pid")
        .parse()
        .expect("pid is a number");
    assert!(
        poll_until(|| armed.exists()),
        "grandchild never armed its SIGHUP trap; the tty hangup would kill it \
         regardless of how we signal, and this test could not observe the \
         difference"
    );
    assert!(
        pid_alive(grandchild),
        "test setup: the grandchild must be running before we shut down"
    );
    assert_ne!(
        grandchild, shell_pid,
        "test setup: the grandchild must be a distinct process, or this \
         cannot tell a group signal from a pid signal"
    );

    shutdown(&mut child).expect("shutdown");

    assert!(
        poll_until(|| !pid_alive(shell_pid)),
        "the session leader survived shutdown"
    );
    assert!(
        poll_until(|| !pid_alive(grandchild)),
        "an orphaned grandchild ({grandchild}) outlived the session -- the \
         signal reached the shell's pid but not its process group"
    );
}

/// Shutting down an already-dead child is safe and reaps it.
///
/// Without the leading `try_wait`, this path signals a pid that the kernel may
/// already have released and reassigned.
#[test]
fn shutdown_of_an_exited_child_is_a_reap_not_a_signal() {
    let pair = open_pty();
    let mut child = spawn_script(&pair, "exit 0");
    assert!(
        poll_until(|| child.try_wait().ok().flatten().is_some()),
        "child did not exit"
    );
    assert_eq!(
        shutdown(&mut child).expect("shutdown"),
        Shutdown::AlreadyExited
    );
}

/// The login `argv[0]` the child **actually receives**, not the string we
/// computed.
///
/// This closes the gap flagged in `e8b567aa`: `login_argv0` and
/// `portable-pty`'s `as_command` (`cmdbuilder.rs:510-517`) were each verified
/// by reading, and agreement-by-reading is not observation.
///
/// Two things make the probe terminate where a naive one hangs. The child
/// writes `$0` to a **file** rather than the PTY -- so there is no terminal
/// echo to strip, no ANSI to parse, and no dependency on the interactive
/// shell ever reaching EOF. And the read is polled to a deadline. Credit to
/// Quinn (`fcfd69b0`), whose three failed PTY-parsing harnesses established
/// that the harness was the bug.
///
/// The explicit-prog row is the control that isolates login `argv[0]` as the
/// only variable: same shell, same PTY, same fence, no `-` prefix.
#[test]
fn default_prog_child_observes_the_login_argv0() {
    let dir = tempdir("buzz-terminal-argv0");
    let shell = "/bin/sh";

    let default_prog = observe_argv0(&dir.join("default"), shell, true);
    assert_eq!(
        default_prog,
        login_argv0(shell),
        "the child's $0 is not the login argv0 we computed"
    );
    assert!(
        default_prog.starts_with('-'),
        "a default-prog child must be a login shell: {default_prog:?}"
    );

    let explicit = observe_argv0(&dir.join("explicit"), shell, false);
    assert_eq!(
        explicit, shell,
        "control: an explicitly-invoked shell must not be given a login argv0"
    );
    assert_ne!(
        default_prog, explicit,
        "control and subject agree, so this test cannot observe the login \
         prefix at all"
    );
}

/// Spawns a `/bin/sh` that writes its own `$0` to `pidfile`, either as a
/// default program (login argv0 applied by portable-pty) or explicitly.
///
/// The default-prog child is an *interactive* shell with no `-c`, so it is
/// driven by writing to the PTY master -- the only way to give a login shell
/// a command is to type one.
fn observe_argv0(outfile: &std::path::Path, shell: &str, default_prog: bool) -> String {
    let pair = open_pty();
    let resolved = resolve_shell(Some(shell));
    let mut cmd = if default_prog {
        CommandBuilder::new_default_prog()
    } else {
        CommandBuilder::new(shell)
    };
    fence_env(&mut cmd, &user_shell_path(), &resolved);
    if !default_prog {
        cmd.arg("-c");
        cmd.arg(format!("printf '%s' \"$0\" > {}", outfile.display()));
    }

    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drain(&pair);
    drop(pair.slave);

    if default_prog {
        use std::io::Write;
        let mut writer = pair.master.take_writer().expect("writer");
        writeln!(writer, "printf '%s' \"$0\" > {}", outfile.display()).expect("write");
        writer.flush().expect("flush");
        // Dropping the writer closes the master's write side, which the shell
        // reads as end-of-input and exits on -- no `exit` command needed, and
        // nothing depends on the shell's rc files having run.
        drop(writer);
    }

    let observed = read_when_written(outfile);
    let _ = crate::lifecycle::shutdown(&mut child);
    observed.unwrap_or_else(|| panic!("child never reported $0 within {BOUND:?}"))
}

/// A fresh directory for a test's artifacts, replacing any prior run's.
fn tempdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Mari's noisy-child discriminator: the reader must still be draining
/// **while** the child is being terminated and reaped.
///
/// The portable contract is structural: `stop` must not be requested until
/// the child has been reaped. The recording reader checks the child PID at the
/// `stop` call, while the continuously noisy PTY makes the test exercise a
/// reader that is genuinely active rather than a quiet no-op.
///
/// The child is deliberately noisy: it floods the PTY continuously, so a
/// master that stops being read fills its kernel buffer within milliseconds
/// and the child blocks in `write()`. That is the state the drain law exists
/// to avoid, and a quiet child cannot produce it -- with nothing being
/// written, both orders look identical and the test proves nothing.
#[test]
fn reader_drains_through_termination_and_reap() {
    let pair = open_pty();

    // Flood, and keep flooding: `yes` writes until the pipe is closed or the
    // process dies, so there is always more output pending than the buffer
    // holds.
    let mut child = spawn_noisy(&pair);
    let pid = child.process_id().expect("pid") as i32;

    let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let stop_at: StopClock = std::sync::Arc::new(std::sync::Mutex::new(None));
    let reader = RecordingReader::spawn(&pair, pid, order.clone(), stop_at.clone());
    // Release the slave, exactly as the runtime does after spawning
    // (`terminal_runtime.rs:441`). Not hygiene: a PTY master does not reach
    // EOF while *any* process holds the slave open, and this test is one --
    // so with the slave retained the reader parks in `read()` forever after
    // the child is reaped, and every wait on it burns its whole bound. Linux
    // honours that rule strictly; Darwin ends the read when the session
    // leader exits, so the retained slave was invisible on the platform this
    // was written on and failed only in CI.
    drop(pair.slave);

    // Establish that this is a live draining reader, not a quiet fixture.
    assert!(
        poll_until(|| reader.total_bytes() > 4096),
        "test setup: the child is not producing enough output to fill the pty \
         buffer, so this cannot distinguish drain order"
    );

    let started = Instant::now();
    let outcome = shutdown_draining(&mut child, Box::new(reader)).expect("shutdown");
    // `stop` runs the instant `shutdown` returns, so this is the child's half
    // of the window and nothing else. Timing the whole call would fold reader
    // teardown into an assertion whose message is about child termination --
    // which is exactly how a stalled reader once read as a wedged child.
    let elapsed = stop_at
        .lock()
        .unwrap()
        .expect("stop was never called")
        .duration_since(started);

    assert_eq!(
        outcome,
        Shutdown::Terminated,
        "a `yes` pipeline dies on SIGTERM; SIGKILL here means it was wedged in \
         a tty write against an undrained master"
    );
    assert!(
        elapsed < TERM_GRACE,
        "shutdown took {elapsed:?}, at or beyond the {TERM_GRACE:?} grace \
         period: the child was blocked writing to an undrained master rather \
         than exiting on SIGTERM"
    );
    assert_eq!(
        *order.lock().unwrap(),
        ["begin_closing", "stop", "join"],
        "reader close must begin before termination and stop/join only after reap"
    );
}

/// Spawns a child that floods the PTY without pause.
fn spawn_noisy(pair: &PtyPair) -> Box<dyn Child + Send + Sync> {
    let shell = resolve_shell(std::env::var("SHELL").ok().as_deref());
    let mut cmd = CommandBuilder::new("/bin/sh");
    fence_env(&mut cmd, &user_shell_path(), &shell);
    cmd.arg("-c");
    cmd.arg("while :; do echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; done");
    // Deliberately no `drain` here: this test owns the reader.
    pair.slave.spawn_command(cmd).expect("spawn")
}

/// A [`DrainingReader`] that records how much it read after close began.
struct RecordingReader {
    pid: i32,
    total: std::sync::Arc<std::sync::atomic::AtomicU64>,
    handle: std::thread::JoinHandle<()>,
    order: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
    /// When `stop` was called -- i.e. the instant `shutdown` returned.
    stop_at: StopClock,
}

/// Shared slot for the instant the reader was asked to stop.
type StopClock = std::sync::Arc<std::sync::Mutex<Option<Instant>>>;

impl RecordingReader {
    fn spawn(
        pair: &PtyPair,
        pid: i32,
        order: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
        stop_at: StopClock,
    ) -> Self {
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let total = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counter = total.clone();
        let handle = std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                counter.fetch_add(n as u64, Ordering::Relaxed);
            }
        });
        Self {
            pid,
            total,
            handle,
            order,
            stop_at,
        }
    }

    fn total_bytes(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
}

impl DrainingReader for RecordingReader {
    fn begin_closing(&self) {
        self.order.lock().unwrap().push("begin_closing");
    }

    fn stop(&self) {
        *self.stop_at.lock().unwrap() = Some(Instant::now());
        assert!(
            !pid_alive(self.pid),
            "reader stop must not be requested before the child is reaped"
        );
        self.order.lock().unwrap().push("stop");
    }

    fn join(self: Box<Self>) {
        self.order.lock().unwrap().push("join");
        // Bounded, and that is the whole point. The read loop ends when the
        // master reports EOF, which only happens once the reaped child has
        // released the slave -- so joining *before* termination blocks
        // forever. That is precisely the forbidden ordering (mutation L4),
        // and an unbounded join would "detect" it by hanging, which is
        // indistinguishable from a broken test. Waiting to a deadline and
        // abandoning the thread converts the hang into an assertion failure
        // the harness can report.
        //
        // The deadline must *assert*, not return. A silent abandon is
        // indistinguishable from a clean join, and that is not hypothetical:
        // it is how a 10 s stall in this fixture masqueraded as a
        // child-termination failure in the caller's timing assertion. The
        // caller's clock covers `shutdown()` only, so this is the sole gate
        // on reader teardown -- with a wedged reader, `shutdown()` still
        // returns in ~58 ms and every other assertion here passes.
        assert!(
            poll_until(|| self.handle.is_finished()),
            "reader thread never finished within {BOUND:?} after the child was \
             reaped: the master never reached EOF, so output was not being \
             drained through termination"
        );
        let _ = self.handle.join();
    }
}
