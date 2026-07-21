//! Admission gate for relay HTTP bridge requests.
//!
//! When the relay answers 429, every relay-backed HTTP request must hold new
//! sends until the quota window clears — matching the TS-side gate in
//! `relayRateLimitGate.ts` that already governs WebSocket operations.
//!
//! **Coverage:** all entry points in `relay.rs` (`query_relay_at`,
//! `submit_event`, `submit_signed_event`, `submit_signed_event_with_keys`,
//! `sync_managed_agent_profile`) and the three previously-direct senders
//! (`submit_engram_event` in snapshot import + team_snapshot, huddle STT)
//! all call `wait_for_rate_limit()` before `.send()`. Remaining relay-derived
//! sends (`/info`, media upload/download) operate on per-request credentials
//! that are not tied to the community quota principal; they are explicitly
//! outside this gate's admission domain.
//!
//! **Community scope:** the gate is reset on every `apply_workspace` call,
//! mirroring the TS gate's `resetRateLimitGate()` on community switch in
//! `useCommunityInit.ts`. A 429 from community A cannot stall community B.
//!
//! Mirrors the TS gate's semantics: overlapping hints never shrink the window,
//! and a hint-less 429 arms the same 10-second default.

use std::sync::Mutex;
use tokio::time::{sleep_until, Duration, Instant};

/// Minimum gate duration when the relay provides no `retry in Ns` hint.
/// Deliberately equal to `DEFAULT_RATE_LIMIT_SECONDS` in `relayRateLimitGate.ts`
/// so both halves of the client back off for the same window.
const DEFAULT_RATE_LIMIT_SECONDS: u64 = 10;

/// Maximum hint the gate will honour from a relay 429 response.
/// Prevents an untrusted relay from pinning traffic for an unreasonable window
/// or overflowing `Instant` arithmetic.
const MAX_HINT_SECONDS: u64 = 300;

static GATE_EXPIRY: Mutex<Option<Instant>> = Mutex::new(None);

/// Arm (or extend) the admission gate from a relay 429.
///
/// `retry_in_seconds` is the parsed `retry in Ns` hint, if the relay provided
/// one. Hints are capped at `MAX_HINT_SECONDS`; values of zero or `None` use
/// `DEFAULT_RATE_LIMIT_SECONDS`. The expiry only ever moves forward: a shorter
/// hint arriving under a longer active window is ignored, so overlapping 429s
/// never schedule a premature retry.
pub fn activate_rate_limit(retry_in_seconds: Option<u64>) {
    let secs = match retry_in_seconds {
        Some(s) if s > 0 => s.min(MAX_HINT_SECONDS),
        _ => DEFAULT_RATE_LIMIT_SECONDS,
    };
    let new_expiry = Instant::now()
        .checked_add(Duration::from_secs(secs))
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(DEFAULT_RATE_LIMIT_SECONDS));
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

/// Reset the gate on a workspace/community change.
///
/// Called by `apply_workspace` to ensure a 429 from community A does not stall
/// requests to community B. Mirrors `resetRateLimitGate()` in
/// `useCommunityInit.ts`.
pub fn reset_gate_for_workspace_change() {
    *GATE_EXPIRY.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Reset the gate. Test-only: production never clears an armed window early
/// except via `reset_gate_for_workspace_change`.
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

    // ── hint capping and overflow safety ─────────────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn hint_zero_uses_default() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        activate_rate_limit(Some(0));
        let start = Instant::now();
        wait_for_rate_limit().await;
        assert_eq!(
            Instant::now() - start,
            Duration::from_secs(DEFAULT_RATE_LIMIT_SECONDS),
            "hint=0 must use the default"
        );
        reset_rate_limit_gate();
    }

    #[tokio::test(start_paused = true)]
    async fn hint_at_max_is_honoured() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        activate_rate_limit(Some(MAX_HINT_SECONDS));
        let start = Instant::now();
        wait_for_rate_limit().await;
        assert_eq!(
            Instant::now() - start,
            Duration::from_secs(MAX_HINT_SECONDS),
            "hint at the cap must be honoured in full"
        );
        reset_rate_limit_gate();
    }

    #[tokio::test(start_paused = true)]
    async fn oversize_hint_is_clamped_to_max() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        // An oversize hint (including u64::MAX) must clamp rather than panic.
        activate_rate_limit(Some(u64::MAX));
        let start = Instant::now();
        wait_for_rate_limit().await;
        assert_eq!(
            Instant::now() - start,
            Duration::from_secs(MAX_HINT_SECONDS),
            "u64::MAX hint must clamp to MAX_HINT_SECONDS"
        );
        reset_rate_limit_gate();
    }

    // ── community / workspace boundary ───────────────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn workspace_change_clears_armed_gate() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        activate_rate_limit(Some(60));
        // Switch workspace — gate for community A must not stall community B.
        reset_gate_for_workspace_change();
        let start = Instant::now();
        wait_for_rate_limit().await;
        assert_eq!(
            Instant::now(),
            start,
            "gate must be clear immediately after workspace change"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn community_a_gate_does_not_block_community_b() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();
        // Community A gets a 429 with a 30s window.
        activate_rate_limit(Some(30));
        // Community switch.
        reset_gate_for_workspace_change();
        // Community B's first request must not wait.
        let start = Instant::now();
        wait_for_rate_limit().await;
        assert_eq!(
            Instant::now(),
            start,
            "community A's armed gate must not delay community B"
        );
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

        // Command 1: the relay answers 429 — the caller sees the typed error
        // and the admission gate arms for the hinted 1s window.
        let err = crate::relay::query_relay(&state, &filters)
            .await
            .expect_err("first command must surface the 429");
        assert!(
            err.starts_with("relay rate-limited: retry in 1s"),
            "429 must map to the typed rate-limited error, got: {err}"
        );

        // Measure from after command 1 returns so the timer only covers the
        // admission wait (not command 1's own network time).
        let after_first_429 = std::time::Instant::now();

        // Command 2: must be withheld until the window expires, then resume
        // and succeed against the now-healthy relay.
        let events = crate::relay::query_relay(&state, &filters)
            .await
            .expect("second command must resume and succeed after expiry");
        assert!(events.is_empty());

        let wait_elapsed = after_first_429.elapsed();
        assert!(
            wait_elapsed >= Duration::from_secs(1),
            "second command ran {}ms after the 429 — it must wait out the full 1s window",
            wait_elapsed.as_millis()
        );

        server.join().unwrap();
        reset_rate_limit_gate();
    }
}
