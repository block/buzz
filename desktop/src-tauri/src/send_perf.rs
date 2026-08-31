//! Timing probe for the message-send path.
//!
//! The send spinner spans every awaited step between the composer clearing and
//! the relay accepting the event, and until now none of those steps logged
//! anything on either side of the bridge — so "the send still feels slow" could
//! only be answered by guessing. These lines name the phase that dominated.
//!
//! The desktop binary installs no `tracing` subscriber, so output goes straight
//! to stderr behind the repo's `buzz-desktop:` prefix; `tracing::info!` here
//! would be silent. Lines are emitted once per send (never per keystroke), so
//! the probe stays on by default without tripping the render-perf measurement
//! guidance in AGENTS.md.
//!
//! [`log_send_perf`] is the frontend's mirror into this same stream. WKWebView
//! drops `console` output produced before a Web Inspector attaches, so the
//! composer's `[send-perf]` summary would otherwise be invisible in a terminal
//! running `just dev` — and impossible to read in order against the backend
//! phases that explain it.

use std::time::Instant;

/// Emit one line: `buzz-desktop: send-perf: <scope> <fields>`.
///
/// `fields` is free-form `key=value` text; keeping the prefix and scope fixed
/// is what makes a `just dev` terminal greppable down to a single send.
pub fn log(scope: &str, fields: &str) {
    eprintln!("buzz-desktop: send-perf: {scope} {fields}");
}

/// Millisecond stopwatch for one phase of the send path.
pub struct Phase(Instant);

impl Phase {
    pub fn start() -> Self {
        Self(Instant::now())
    }

    /// Elapsed milliseconds to a tenth — finer than the differences being
    /// chased, and stable enough to compare two runs by eye.
    pub fn ms(&self) -> f64 {
        (self.0.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
    }
}

/// Write the frontend's `[send-perf]` summary into the backend stderr stream.
///
/// Called fire-and-forget from the composer; `payload` is the already-encoded
/// JSON summary, reproduced verbatim so the two halves of one send read as one
/// record. Async so the write never lands on the UI thread.
#[tauri::command]
pub async fn log_send_perf(label: String, payload: String) {
    log(&label, &payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_reports_elapsed_milliseconds_at_tenth_resolution() {
        let phase = Phase::start();
        let ms = phase.ms();

        assert!(ms >= 0.0, "elapsed must not be negative: {ms}");
        // A tenth-resolution value survives a round-trip through tenths.
        assert_eq!(ms, (ms * 10.0).round() / 10.0);
    }

    #[test]
    fn phase_is_monotonic_across_reads() {
        let phase = Phase::start();
        let first = phase.ms();
        let second = phase.ms();

        assert!(second >= first, "{second} < {first}");
    }
}
