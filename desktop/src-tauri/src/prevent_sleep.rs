//! Keeps the machine awake while local managed agents are running.
//!
//! macOS and Windows have a real mechanism, reached through the `keepawake`
//! crate rather than local FFI — `AGENTS.md` forbids `unsafe` code, and a safe
//! wrapper also retires the IOKit `extern "C"` block this module used to carry.
//!
//! The guard is owned by a **dedicated thread** for the lifetime of the block.
//! Windows' underlying request is thread-local — its effect dies with the
//! thread that set it — while [`acquire`] and [`release`] are reached from
//! Tauri command handlers on a worker pool and from the shutdown hook,
//! potentially three different threads. Binding the request to any of those
//! would either evaporate (the acquiring worker retires while an agent is
//! still running) or leak (release lands on a thread that never set it). The
//! thread costs a wake-up channel and buys a lifetime that matches the block.
//!
//! Linux is a deliberate, documented no-op. keepawake can inhibit there, but
//! only through a live D-Bus session; the crate is target-gated to macOS and
//! Windows so a Linux build gains no runtime D-Bus requirement. [`acquire`]
//! succeeds without holding anything and no cap timer is armed.
//!
//! Whatever a platform hands back is stored as a [`SleepBlock`], whose `Drop`
//! performs the matching release. Every teardown path — explicit
//! [`release`], the [`INACTIVITY_CAP_SECONDS`] safety valve, and process exit —
//! goes through that single `Drop`, so no path can clear the bookkeeping while
//! leaving the OS-level request outstanding.

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

/// Tracks the OS-level power request that prevents idle sleep while local
/// managed agents are running.
#[derive(Default)]
pub struct PreventSleepState {
    block: Option<SleepBlock>,
    timer_handle: Option<tauri::async_runtime::JoinHandle<()>>,
    timer_generation: u64,
}

/// A live OS-level "do not idle-sleep" request. Dropping it releases the
/// request; there is no other release path.
enum SleepBlock {
    /// A dedicated thread owning the `keepawake` guard for the whole lifetime
    /// of the block. See [`platform_begin`] for why the guard cannot simply
    /// live in this struct.
    #[cfg(any(target_os = "macos", windows))]
    Worker {
        /// Dropping this disconnects the channel the worker is parked on,
        /// which is the signal to release.
        stop: Option<std::sync::mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    },
    /// Test-only inert block. Dropping it bumps the counter, which lets the
    /// platform-independent bookkeeping be asserted on every OS.
    #[cfg(test)]
    Inert(Arc<std::sync::atomic::AtomicUsize>),
}

impl Drop for SleepBlock {
    // One `if let` per platform rather than a `match`: on platforms with no
    // mechanism the enum has no variants at all, and an arm-less `match` over
    // `&mut Self` does not compile there (`min_exhaustive_patterns` does not
    // see through the reference). The flip side is that in a non-test build
    // each platform leaves exactly one variant, making its `if let`
    // irrefutable — which is correct, not a mistake.
    #[allow(irrefutable_let_patterns)]
    fn drop(&mut self) {
        #[cfg(any(target_os = "macos", windows))]
        if let Self::Worker { stop, thread } = self {
            // Dropping the sender disconnects the channel the worker is parked
            // on. Joining afterwards is what makes this synchronous: it
            // guarantees the guard has actually been dropped, on the thread
            // that created it, before this `drop` returns.
            drop(stop.take());
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }

        #[cfg(test)]
        if let Self::Inert(released) = self {
            released.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

// ── Platform implementation ─────────────────────────────────────────────────

/// Ask the OS for a sleep block. `Ok(None)` means this platform has no
/// mechanism (see the module docs) — that is success, not failure.
///
/// The guard is created on, and dropped by, a thread this function owns.
/// Windows' underlying request is thread-local: its effect dies with the
/// thread that set it. [`acquire`] and [`release`] are reached from Tauri
/// command handlers on a worker pool and from the shutdown hook — potentially
/// three different threads — so binding the request to any of those would
/// either evaporate (the acquiring worker retires while agents still run) or
/// leak (release lands on a thread that never set it). One dedicated thread,
/// parked for the block's whole lifetime, is what makes the lifetime correct.
#[cfg(any(target_os = "macos", windows))]
fn platform_begin() -> Result<Option<SleepBlock>, String> {
    use std::sync::mpsc;

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let thread = std::thread::Builder::new()
        .name("buzz-prevent-sleep".to_string())
        .spawn(move || {
            // `idle(true)` is the direct analogue of the previous macOS
            // `PreventUserIdleSystemSleep` assertion: block *idle* sleep only.
            // `display(false)` keeps the screen free to blank, and
            // `sleep(false)` leaves a user-initiated sleep or lid close alone.
            let awake = keepawake::Builder::default()
                .display(false)
                .idle(true)
                .sleep(false)
                .reason("Agents are active")
                .app_name("Buzz")
                .app_reverse_domain("xyz.block.buzz.app")
                .create();

            match awake {
                Ok(awake) => {
                    if ready_tx.send(Ok(())).is_err() {
                        // Nobody is waiting any more; drop the guard and go.
                        return;
                    }
                    // Blocks until the sender is dropped by `SleepBlock::drop`.
                    // The guard is dropped here, on the thread that made it.
                    let _ = stop_rx.recv();
                    drop(awake);
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("keepawake failed: {error}")));
                }
            }
        })
        .map_err(|error| format!("failed to spawn the sleep-block thread: {error}"))?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Some(SleepBlock::Worker {
            stop: Some(stop_tx),
            thread: Some(thread),
        })),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(_) => {
            let _ = thread.join();
            Err("the sleep-block thread exited before reporting readiness".to_string())
        }
    }
}

/// Ask the OS for a sleep block. Linux and friends have no portable inhibit
/// mechanism we want to depend on — keepawake's Linux path needs a live D-Bus
/// session — so this stays a documented no-op that reports success without
/// holding anything (and therefore without arming the cap timer).
#[cfg(not(any(target_os = "macos", windows)))]
fn platform_begin() -> Result<Option<SleepBlock>, String> {
    Ok(None)
}

// ── Shared bookkeeping ──────────────────────────────────────────────────────

/// One hour covers a silent long-running tool call while bounding idle keep-awake time.
const INACTIVITY_CAP_SECONDS: u64 = 60 * 60;

fn arm_cap_timer(
    guard: &mut PreventSleepState,
    state: &Arc<Mutex<PreventSleepState>>,
    app_handle: &AppHandle,
) {
    if let Some(handle) = guard.timer_handle.take() {
        handle.abort();
    }

    guard.timer_generation = guard.timer_generation.wrapping_add(1);
    let generation = guard.timer_generation;
    let handle = app_handle.clone();
    let timer_state = Arc::clone(state);
    let timer_task = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(INACTIVITY_CAP_SECONDS)).await;
        if expire_if_current(&timer_state, generation) {
            let _ = handle.emit("prevent-sleep-expired", ());
        }
    });
    guard.timer_handle = Some(timer_task);
}

fn expire_if_current(state: &Arc<Mutex<PreventSleepState>>, generation: u64) -> bool {
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };

    if guard.timer_generation != generation {
        return false;
    }

    guard.timer_handle = None;
    // Dropping the block is the release, on every platform.
    guard.block = None;

    true
}

/// Ensure a sleep block is held, creating one if necessary.
///
/// Returns whether a block is now held — i.e. whether the caller should arm the
/// inactivity cap timer. `false` means the platform has no mechanism, so there
/// is nothing to cap.
fn ensure_block(guard: &mut PreventSleepState) -> Result<bool, String> {
    if guard.block.is_none() {
        guard.block = platform_begin()?;
    }
    Ok(guard.block.is_some())
}

/// Acquire an OS sleep block if not already held.
/// Refreshes the inactivity cap whenever a block is held.
pub fn acquire(
    state: &Arc<Mutex<PreventSleepState>>,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|e| e.to_string())?;

    // Start (or refresh) the inactivity cap timer only if a block is actually
    // held — on a platform with no mechanism there is nothing to cap.
    if ensure_block(&mut guard)? {
        arm_cap_timer(&mut guard, state, app_handle);
    }

    Ok(())
}

/// Release the sleep block if held. Cancel the cap timer.
pub fn release(state: &Arc<Mutex<PreventSleepState>>) {
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    if let Some(handle) = guard.timer_handle.take() {
        handle.abort();
    }
    guard.timer_generation = guard.timer_generation.wrapping_add(1);

    // Dropping the block is the release, on every platform.
    guard.block = None;
}

/// Returns `true` if a sleep block is currently held.
#[allow(dead_code)]
pub fn is_held(state: &Arc<Mutex<PreventSleepState>>) -> bool {
    state.lock().map(|g| g.block.is_some()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Build a state holding an inert block, plus the counter its `Drop` bumps.
    fn state_with_inert_block() -> (Arc<Mutex<PreventSleepState>>, Arc<AtomicUsize>) {
        let released = Arc::new(AtomicUsize::new(0));
        let state = Arc::new(Mutex::new(PreventSleepState {
            block: Some(SleepBlock::Inert(Arc::clone(&released))),
            timer_handle: None,
            timer_generation: 7,
        }));
        (state, released)
    }

    #[test]
    fn release_drops_the_block_and_invalidates_the_timer() {
        let (state, released) = state_with_inert_block();
        assert!(is_held(&state));

        release(&state);

        assert_eq!(
            released.load(Ordering::SeqCst),
            1,
            "block released exactly once"
        );
        assert!(!is_held(&state));
        // The in-flight timer's generation must no longer match, so a late
        // expiry cannot fire an event for a block that is already gone.
        assert!(!expire_if_current(&state, 7));
    }

    #[test]
    fn expire_ignores_a_stale_generation_and_keeps_the_block() {
        let (state, released) = state_with_inert_block();

        assert!(!expire_if_current(&state, 6));

        assert_eq!(released.load(Ordering::SeqCst), 0);
        assert!(is_held(&state), "a stale timer must not release the block");
    }

    #[test]
    fn expire_releases_the_block_once_for_the_current_generation() {
        let (state, released) = state_with_inert_block();

        assert!(expire_if_current(&state, 7));
        assert_eq!(released.load(Ordering::SeqCst), 1);
        assert!(!is_held(&state));

        // A duplicate expiry for the same generation must not double-release.
        assert!(expire_if_current(&state, 7));
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }

    /// The bug this module was fixed for: on Windows `acquire` used to leave
    /// the state empty, so nothing was held and the inactivity cap was never
    /// armed. `ensure_block` must report `true` — that return value is exactly
    /// what gates `arm_cap_timer` in [`acquire`].
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn ensure_block_holds_a_real_os_block_and_is_idempotent() {
        let mut guard = PreventSleepState::default();

        assert!(
            ensure_block(&mut guard).expect("platform_begin should succeed"),
            "a supported platform must report a held block so the cap timer arms"
        );
        assert!(guard.block.is_some());

        let before = block_identity(&guard);
        assert!(ensure_block(&mut guard).expect("second call should succeed"));
        assert_eq!(
            before,
            block_identity(&guard),
            "an already-held block must be reused, not replaced and leaked"
        );

        // Dropping the guard releases the real OS request.
    }

    /// Identity of the held block — the owning worker's thread id. A second
    /// `ensure_block` must reuse it rather than spawn a second thread and
    /// strand the first one holding a request nothing will ever release.
    #[cfg(any(target_os = "macos", windows))]
    fn block_identity(guard: &PreventSleepState) -> Option<String> {
        guard.block.as_ref().map(|block| match block {
            SleepBlock::Worker { thread, .. } => thread
                .as_ref()
                .map(|thread| format!("{:?}", thread.thread().id()))
                .unwrap_or_else(|| "detached".to_string()),
            SleepBlock::Inert(_) => "inert".to_string(),
        })
    }

    /// Platforms with no mechanism report "not held" so no cap timer is armed,
    /// but they must not surface an error to the caller.
    #[cfg(not(any(target_os = "macos", windows)))]
    #[test]
    fn unsupported_platforms_are_a_silent_success() {
        let mut guard = PreventSleepState::default();
        assert!(!ensure_block(&mut guard).expect("no-op platform must not error"));
        assert!(guard.block.is_none());
    }
}
