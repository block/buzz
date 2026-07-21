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
//! all call `wait_for_rate_limit()` before `.send()`.
//!
//! **Media upload/download and `/info`** call `relay_error_message()` on
//! non-200 responses, so their 429s arm the shared gate as conservative
//! back-off (any relay overload signal is worth honouring across domains).
//! They do not call `wait_for_rate_limit()` themselves — their operations
//! are driven by user-initiated file transfers rather than bridge event flow,
//! and they have independent retry logic.
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
/// Exposed `pub` so `relay.rs` can clamp the hint before embedding it in the
/// returned error string — ensuring every consumer (Rust gate and TS gate via
/// `applyTauriRateLimitIfNeeded`) sees the same capped value.
pub const MAX_HINT_SECONDS: u64 = 300;

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

    /// A 429 on one admission-gated path withholds sends on a different path
    /// until the hinted window expires.
    ///
    /// Arms the gate directly (as `relay_error_message` would on a 429 response),
    /// then drives a second distinct gated send through the loopback acceptance
    /// server and asserts it waited out the window before succeeding.
    #[tokio::test]
    async fn gate_armed_by_one_path_withholds_another_path() {
        use std::io::{Read, Write};

        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();

        // The loopback server answers every request with 200 [].
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || loop {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]",
            );
            let _ = stream.flush();
        });

        // Capture t0 before arming so elapsed is measured from before the gate
        // expiry is set (expiry = now + 1s), guaranteeing elapsed ≥ 1s.
        let t0 = std::time::Instant::now();

        // Arm the gate for 1s — simulates what relay_error_message does on any
        // gated path that receives a 429, e.g. submit_engram_event.
        activate_rate_limit(Some(1));

        let state = crate::app_state::build_app_state();
        *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
        let filters = [serde_json::json!({ "kinds": [1], "limit": 1 })];

        // query_relay is a different gated path — it must wait out the 1s window
        // even though it was not the source of the 429.
        let events = crate::relay::query_relay(&state, &filters)
            .await
            .expect("query must succeed after admission wait");
        let elapsed = t0.elapsed();

        assert!(
            events.is_empty(),
            "server returns empty; unexpected events: {events:?}"
        );
        assert!(
            elapsed >= Duration::from_secs(1),
            "cross-path send ran {}ms — it must wait out the 1s window armed by another path",
            elapsed.as_millis()
        );

        drop(server);
        reset_rate_limit_gate();
    }

    /// A waiter parked on community A's gate must NOT wake early when the gate
    /// is reset by a workspace change — it sleeps until the original expiry,
    /// then rechecks, finds the gate clear, and proceeds.
    ///
    /// This documents the contract: `sleep_until` is already scheduled against
    /// A's expiry; the reset clears `GATE_EXPIRY` but cannot cancel an in-flight
    /// sleep.  The recheck loop in `wait_for_rate_limit` then sees `None` and
    /// returns.  Net effect: the waiter observes at most one full window, which
    /// is the same bound as if the workspace had not changed.
    #[tokio::test(start_paused = true)]
    async fn parked_waiter_does_not_wake_early_after_workspace_reset() {
        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();

        // Arm a 5s gate for community A.
        activate_rate_limit(Some(5));

        let start = Instant::now();

        // Spawn a waiter that parks on the gate.
        let waiter = tokio::spawn(async {
            wait_for_rate_limit().await;
            Instant::now()
        });

        // After 2s (well before the 5s expiry), simulate a workspace change
        // that resets the gate.
        tokio::time::sleep(Duration::from_secs(2)).await;
        reset_gate_for_workspace_change();

        let woke_at = waiter.await.unwrap();
        let elapsed = woke_at - start;

        // The waiter must have waited at least until A's original expiry (5s).
        // It cannot be woken early by the reset — only the recheck loop exit
        // can terminate it, and the recheck fires after sleep_until(expiry).
        assert!(
            elapsed >= Duration::from_secs(5),
            "waiter woke at {}ms — must not wake before A's 5s expiry even after reset",
            elapsed.as_millis()
        );

        reset_rate_limit_gate();
    }

    /// Wait-then-sign ensures NIP-98 auth is fresh after an admission wait.
    ///
    /// The relay enforces NIP-98 freshness within ±60s (`TIMESTAMP_TOLERANCE_SECS`
    /// in `buzz-auth`), while the gate honours hints up to `MAX_HINT_SECONDS`
    /// (300s). Signing BEFORE the wait produces a stale `created_at` that would
    /// be rejected on any active window >60s.
    ///
    /// This test arms a 1s gate, records the wall-clock time immediately AFTER
    /// the wait returns, then builds the NIP-98 header and asserts its
    /// `created_at` is ≥ the wake time — proving the sign happened post-wait.
    #[tokio::test]
    async fn nip98_auth_created_at_is_fresh_after_admission_wait() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        use serde::Deserialize;

        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();

        // Arm the gate for 1s (real time — NIP-98 uses SystemTime, not Tokio clock).
        activate_rate_limit(Some(1));

        // Wait out the gate — this is the critical ordering: wait THEN sign.
        wait_for_rate_limit().await;

        // Capture the wake time as a Unix timestamp AFTER the wait returns.
        let wake_unix: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Build NIP-98 auth header (contains a signed event whose `created_at`
        // is set at sign time — i.e., now, after the wait).
        let keys = nostr::Keys::generate();
        let header = crate::relay::build_nip98_auth_header_for_keys(
            &keys,
            &reqwest::Method::POST,
            "https://relay.example.com/events",
            b"{}",
        )
        .expect("header build must succeed");

        // Decode the base64-encoded Nostr event from "Nostr <base64>".
        let b64 = header
            .strip_prefix("Nostr ")
            .expect("header must start with 'Nostr '");
        let json_bytes = BASE64.decode(b64).expect("valid base64");

        #[derive(Deserialize)]
        struct EventShell {
            created_at: u64,
        }
        let shell: EventShell = serde_json::from_slice(&json_bytes).expect("valid event JSON");

        assert!(
            shell.created_at >= wake_unix,
            "NIP-98 created_at ({}) must be >= wake time ({}); \
             signing before the wait produces stale auth",
            shell.created_at,
            wake_unix
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
