use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

/// Tracks the macOS IOKit power assertion that prevents idle sleep
/// while local managed agents are running.
#[derive(Default)]
pub struct PreventSleepState {
    assertion_id: Option<u32>,
    timer_handle: Option<tauri::async_runtime::JoinHandle<()>>,
    timer_generation: u64,
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

    #[cfg(target_os = "macos")]
    if let Some(id) = guard.assertion_id.take() {
        unsafe {
            macos::IOPMAssertionRelease(id);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        guard.assertion_id = None;
    }

    true
}

/// Create a `PreventUserIdleSystemSleep` assertion if not already held.
/// Refreshes the inactivity cap when the assertion is already held.
pub fn acquire(
    state: &Arc<Mutex<PreventSleepState>>,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|e| e.to_string())?;

    if guard.assertion_id.is_some() {
        arm_cap_timer(&mut guard, state, app_handle);
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let assertion_type = c"PreventUserIdleSystemSleep".as_ptr();
        let reason = c"Buzz \u{2014} agents are active".as_ptr();

        unsafe {
            let cf_type = macos::CFStringCreateWithCString(
                std::ptr::null(),
                assertion_type,
                K_CF_STRING_ENCODING_UTF8,
            );
            let cf_reason = macos::CFStringCreateWithCString(
                std::ptr::null(),
                reason,
                K_CF_STRING_ENCODING_UTF8,
            );

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

            guard.assertion_id = Some(assertion_id);
        }
    }

    // Start the inactivity cap timer only if an assertion was actually created.
    if guard.assertion_id.is_some() {
        arm_cap_timer(&mut guard, state, app_handle);
    }

    Ok(())
}

/// Release the power assertion if held. Cancel the cap timer.
pub fn release(state: &Arc<Mutex<PreventSleepState>>) {
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    if let Some(handle) = guard.timer_handle.take() {
        handle.abort();
    }
    guard.timer_generation = guard.timer_generation.wrapping_add(1);

    #[cfg(target_os = "macos")]
    if let Some(id) = guard.assertion_id.take() {
        unsafe {
            macos::IOPMAssertionRelease(id);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        guard.assertion_id = None;
    }
}

/// Returns `true` if a power assertion is currently held.
#[allow(dead_code)]
pub fn is_held(state: &Arc<Mutex<PreventSleepState>>) -> bool {
    state
        .lock()
        .map(|g| g.assertion_id.is_some())
        .unwrap_or(false)
}
