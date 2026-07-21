//! Admission gate for relay HTTP bridge requests.
//!
//! When the relay answers 429, every relay-backed Tauri command must hold new
//! HTTP requests until the quota window clears — matching the TS-side gate in
//! `relayRateLimitGate.ts` that already governs WebSocket operations. Gating
//! here, at the single Rust choke point all relay HTTP flows share, covers
//! every command (including raw `invoke()` call sites that bypass
//! `invokeTauri`) without touching dozens of TS callers.
//!
//! Mirrors the TS gate's semantics: overlapping hints never shrink the window,
//! and a hint-less 429 arms the same 10-second default.

use std::sync::Mutex;
use tokio::time::{sleep_until, Duration, Instant};

/// Minimum gate duration when the relay provides no `retry in Ns` hint.
/// Deliberately equal to `DEFAULT_RATE_LIMIT_SECONDS` in `relayRateLimitGate.ts`
/// so both halves of the client back off for the same window.
const DEFAULT_RATE_LIMIT_SECONDS: u64 = 10;

static GATE_EXPIRY: Mutex<Option<Instant>> = Mutex::new(None);

/// Arm (or extend) the admission gate from a relay 429.
///
/// `retry_in_seconds` is the parsed `retry in Ns` hint, if the relay provided
/// one. The expiry only ever moves forward: a shorter hint arriving under a
/// longer active window is ignored, so overlapping 429s never schedule a
/// premature retry.
pub fn activate_rate_limit(retry_in_seconds: Option<u64>) {
    let secs = match retry_in_seconds {
        Some(s) if s > 0 => s,
        _ => DEFAULT_RATE_LIMIT_SECONDS,
    };
    let new_expiry = Instant::now() + Duration::from_secs(secs);
    let mut guard = GATE_EXPIRY.lock().unwrap_or_else(|e| e.into_inner());
    match *guard {
        Some(current) if new_expiry <= current => {}
        _ => *guard = Some(new_expiry),
    }
}

/// Wait until the admission gate is clear.
///
/// Returns immediately when no gate is active. Loops after sleeping because a
/// concurrent 429 may extend the expiry while this caller is parked.
pub async fn wait_for_rate_limit() {
    loop {
        let expiry = {
            let guard = GATE_EXPIRY.lock().unwrap_or_else(|e| e.into_inner());
            match *guard {
                Some(expiry) if expiry > Instant::now() => Some(expiry),
                _ => None,
            }
        };
        match expiry {
            Some(expiry) => sleep_until(expiry).await,
            None => return,
        }
    }
}

/// Reset the gate. Test-only: production never clears an armed window early.
#[cfg(test)]
pub fn reset_rate_limit_gate() {
    *GATE_EXPIRY.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    // The gate is a process-wide static shared by every test in this binary,
    // so all gate tests serialize on one async lock to keep armed expiries
    // from bleeding between parallel test threads.
    pub(crate) static TEST_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test(start_paused = true)]
    async fn wait_returns_immediately_when_gate_is_inactive() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        let start = Instant::now();
        wait_for_rate_limit().await;
        assert_eq!(
            Instant::now(),
            start,
            "inactive gate must not consume any (paused) time"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn hintless_429_arms_the_ten_second_default() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        activate_rate_limit(None);
        let start = Instant::now();
        wait_for_rate_limit().await;
        assert_eq!(Instant::now() - start, Duration::from_secs(10));
        reset_rate_limit_gate();
    }

    #[tokio::test(start_paused = true)]
    async fn shorter_hint_never_shrinks_an_active_window() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        activate_rate_limit(Some(8));
        activate_rate_limit(Some(1));
        let start = Instant::now();
        wait_for_rate_limit().await;
        assert_eq!(
            Instant::now() - start,
            Duration::from_secs(8),
            "the 1s hint must not shorten the active 8s window"
        );
        reset_rate_limit_gate();
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_429_extends_the_window_for_parked_waiters() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        activate_rate_limit(Some(2));
        let start = Instant::now();
        let waiter = tokio::spawn(async {
            wait_for_rate_limit().await;
        });
        // Extend while the waiter is parked on the first expiry.
        tokio::time::sleep(Duration::from_secs(1)).await;
        activate_rate_limit(Some(4));
        waiter.await.unwrap();
        assert_eq!(
            Instant::now() - start,
            Duration::from_secs(5),
            "waiter must respect the extension armed mid-sleep (1s + 4s)"
        );
        reset_rate_limit_gate();
    }

    /// Acceptance: a 429 from one relay-backed command withholds the next
    /// relay-backed command until the hinted window expires, then it resumes.
    ///
    /// Drives the production `query_relay` path end-to-end against a loopback
    /// HTTP server — NIP-98 signing, `relay_error_message` classification and
    /// gate arming, and the admission wait all execute for real. Real time is
    /// required (the request crosses actual TCP), so the hint is kept at 1s.
    #[tokio::test]
    async fn http_429_withholds_next_relay_command_until_expiry_then_resumes() {
        use std::io::{Read, Write};

        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        // First request → 429 with a 1s retry hint; every later request → 200 [].
        let server = std::thread::spawn(move || {
            let responses = [
                "HTTP/1.1 429 Too Many Requests\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: 53\r\n\
                 Connection: close\r\n\r\n\
                 {\"error\":\"rate-limited: quota exceeded; retry in 1s\"}",
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: 2\r\n\
                 Connection: close\r\n\r\n\
                 []",
            ];
            for i in 0..2 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(responses[i.min(1)].as_bytes());
                let _ = stream.flush();
            }
        });

        let state = crate::app_state::build_app_state();
        *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
        let filters = [serde_json::json!({ "kinds": [1], "limit": 1 })];

        let started = std::time::Instant::now();

        // Command 1: the relay answers 429 — the caller sees the typed error
        // and the admission gate arms for the hinted 1s window.
        let err = crate::relay::query_relay(&state, &filters)
            .await
            .expect_err("first command must surface the 429");
        assert!(
            err.starts_with("relay rate-limited: retry in 1s"),
            "429 must map to the typed rate-limited error, got: {err}"
        );

        // Command 2: must be withheld until the window expires, then resume
        // and succeed against the now-healthy relay.
        let events = crate::relay::query_relay(&state, &filters)
            .await
            .expect("second command must resume and succeed after expiry");
        assert!(events.is_empty());

        assert!(
            started.elapsed() >= Duration::from_secs(1),
            "second command ran {}ms after the 429 — it must wait out the full 1s window",
            started.elapsed().as_millis()
        );

        server.join().unwrap();
        reset_rate_limit_gate();
    }
}
