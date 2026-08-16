//! Keeps the machine awake while local managed agents are running.
//!
//! Two platforms have a real mechanism:
//!
//! * **macOS** — an IOKit `PreventUserIdleSystemSleep` power assertion.
//! * **Windows** — a Win32 *power request* (`PowerCreateRequest` +
//!   `PowerSetRequest(PowerRequestSystemRequired)`).
//!
//! The Windows side deliberately does **not** use `SetThreadExecutionState`.
//! That API is per-thread and its effect dies with the calling thread, but
//! [`acquire`] and [`release`] are reached from Tauri command handlers running
//! on a worker pool and from the shutdown hook — potentially three different
//! threads. A `SetThreadExecutionState` pair would therefore either evaporate
//! (the acquiring worker thread retires while an agent is still running) or
//! leak (release lands on a thread that never set the flag). A power request is
//! a kernel object owned by the *process*: any thread may create, set, clear,
//! and close it, and the handle can be stored in shared state. The alternative
//! — an owned dedicated thread parked for the lifetime of the block — costs a
//! thread plus a wake-up channel to achieve the same thing, so the power
//! request wins.
//!
//! Linux is a deliberate, documented no-op: there is no portable
//! inhibit mechanism (logind vs. elogind vs. none), so [`acquire`] succeeds
//! without holding anything and no cap timer is armed.
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
    /// macOS `IOPMAssertionID` for a `PreventUserIdleSystemSleep` assertion.
    #[cfg(target_os = "macos")]
    IoKitAssertion(u32),
    /// Windows power-request `HANDLE`, held as an integer so the state stays
    /// `Send` without an `unsafe impl` (the raw pointer is reconstituted only
    /// at the call site).
    #[cfg(windows)]
    PowerRequest(isize),
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
        #[cfg(target_os = "macos")]
        if let Self::IoKitAssertion(id) = self {
            unsafe {
                macos::IOPMAssertionRelease(*id);
            }
        }

        #[cfg(windows)]
        if let Self::PowerRequest(handle) = self {
            use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
            use windows_sys::Win32::System::Power::{
                PowerClearRequest, PowerRequestSystemRequired,
            };

            let handle = *handle as HANDLE;
            unsafe {
                // Clear before close: closing an un-cleared request also drops
                // it, but clearing first keeps `powercfg /requests` accurate if
                // the close ever fails.
                PowerClearRequest(handle, PowerRequestSystemRequired);
                CloseHandle(handle);
            }
        }

        #[cfg(test)]
        if let Self::Inert(released) = self {
            released.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

// ── macOS implementation ────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod macos {
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOPMAssertionCreateWithName(
            assertion_type: *const std::ffi::c_void, // CFStringRef
            level: u32,                              // IOPMAssertionLevel
            name: *const std::ffi::c_void,           // CFStringRef
            assertion_id: *mut u32,                  // IOPMAssertionID
        ) -> i32; // IOReturn

        pub fn IOPMAssertionRelease(assertion_id: u32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFStringCreateWithCString(
            alloc: *const std::ffi::c_void,
            c_str: *const std::ffi::c_char,
            encoding: u32,
        ) -> *const std::ffi::c_void;
        pub fn CFRelease(cf: *const std::ffi::c_void);
    }
}

#[cfg(target_os = "macos")]
const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;

#[cfg(target_os = "macos")]
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

/// Ask the OS for a sleep block. `Ok(None)` means this platform has no
/// mechanism (see the module docs) — that is success, not failure.
#[cfg(target_os = "macos")]
fn platform_begin() -> Result<Option<SleepBlock>, String> {
    let assertion_type = c"PreventUserIdleSystemSleep".as_ptr();
    let reason = c"Buzz \u{2014} agents are active".as_ptr();

    unsafe {
        let cf_type = macos::CFStringCreateWithCString(
            std::ptr::null(),
            assertion_type,
            K_CF_STRING_ENCODING_UTF8,
        );
        let cf_reason =
            macos::CFStringCreateWithCString(std::ptr::null(), reason, K_CF_STRING_ENCODING_UTF8);

        if cf_type.is_null() || cf_reason.is_null() {
            if !cf_type.is_null() {
                macos::CFRelease(cf_type);
            }
            if !cf_reason.is_null() {
                macos::CFRelease(cf_reason);
            }
            return Err("Failed to create CFString for IOKit assertion".into());
        }

        let mut assertion_id: u32 = 0;
        let ret = macos::IOPMAssertionCreateWithName(
            cf_type,
            K_IOPM_ASSERTION_LEVEL_ON,
            cf_reason,
            &mut assertion_id,
        );

        macos::CFRelease(cf_type);
        macos::CFRelease(cf_reason);

        if ret != 0 {
            return Err(format!(
                "IOPMAssertionCreateWithName failed with IOReturn {ret}"
            ));
        }

        Ok(Some(SleepBlock::IoKitAssertion(assertion_id)))
    }
}

// ── Windows implementation ──────────────────────────────────────────────────

/// `POWER_REQUEST_CONTEXT_VERSION` from `winnt.h`. Not re-exported by
/// `windows-sys`, so it is spelled out here.
#[cfg(windows)]
const POWER_REQUEST_CONTEXT_VERSION: u32 = 0;

/// Ask the OS for a sleep block. `Ok(None)` means this platform has no
/// mechanism (see the module docs) — that is success, not failure.
///
/// `PowerRequestSystemRequired` is the direct analogue of the macOS
/// `PreventUserIdleSystemSleep` assertion: it blocks *idle* sleep only. It
/// deliberately does not block a user-initiated sleep, a lid close, or the
/// low-power transition on a Modern Standby machine, and it does not keep the
/// display awake (that would need `PowerRequestDisplayRequired`).
#[cfg(windows)]
fn platform_begin() -> Result<Option<SleepBlock>, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Power::{
        PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
    };
    use windows_sys::Win32::System::Threading::{
        POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
    };

    // `PowerCreateRequest` copies the reason string, so a stack-local buffer
    // that lives across the call is enough.
    let mut reason: Vec<u16> = "Buzz \u{2014} agents are active"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let context = REASON_CONTEXT {
        Version: POWER_REQUEST_CONTEXT_VERSION,
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 {
            SimpleReasonString: reason.as_mut_ptr(),
        },
    };

    let request: HANDLE = unsafe { PowerCreateRequest(&context) };
    if request.is_null() || request == INVALID_HANDLE_VALUE {
        let code = unsafe { GetLastError() };
        return Err(format!(
            "PowerCreateRequest failed with GetLastError {code}"
        ));
    }

    if unsafe { PowerSetRequest(request, PowerRequestSystemRequired) } == 0 {
        let code = unsafe { GetLastError() };
        unsafe {
            CloseHandle(request);
        }
        return Err(format!("PowerSetRequest failed with GetLastError {code}"));
    }

    Ok(Some(SleepBlock::PowerRequest(request as isize)))
}

// ── Platforms with no mechanism ─────────────────────────────────────────────

/// Ask the OS for a sleep block. Linux and friends have no portable inhibit
/// mechanism, so this is a documented no-op that reports success without
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

    #[cfg(any(target_os = "macos", windows))]
    fn block_identity(guard: &PreventSleepState) -> Option<isize> {
        guard.block.as_ref().map(|block| match block {
            #[cfg(target_os = "macos")]
            SleepBlock::IoKitAssertion(id) => *id as isize,
            #[cfg(windows)]
            SleepBlock::PowerRequest(handle) => *handle,
            SleepBlock::Inert(_) => 0,
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
