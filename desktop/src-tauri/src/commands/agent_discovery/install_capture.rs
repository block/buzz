//! Bounded capture of an install command's output.
//!
//! One drain per stream feeds a [`Capture`], which holds two independently
//! bounded views of the same bytes: a small one sized for an error toast and a
//! large one sized for the install log file. Both are shared with the draining
//! reader rather than returned by it, so whatever arrived before a stall is
//! readable at the ceiling — exactly when the output matters most.

use std::collections::VecDeque;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How much of each end a capture keeps, and how it names what was cut.
#[derive(Clone, Copy)]
struct Caps {
    head: usize,
    tail: usize,
    marker: fn(usize) -> String,
}

/// Sized for a UI error message: enough to identify the failure, small enough
/// to read in a toast.
const UI_CAPS: Caps = Caps {
    head: 512,
    tail: 1024,
    marker: |omitted| format!("... ({omitted} bytes omitted) ..."),
};

/// Sized for the log file, where the budget is disk rather than screen. At this
/// cap a real install log is complete in practice; the marker names the cases
/// where it is not, so the file never implies completeness it does not have.
const LOG_CAPS: Caps = Caps {
    head: 128 * 1024,
    tail: 128 * 1024,
    marker: |omitted| format!("... [{omitted} bytes omitted at cap] ..."),
};

/// Bounded capture of one stream: its first `head` bytes, its last `tail`
/// bytes, and the total byte count. Output of any size costs a fixed amount of
/// memory, so an installer that prints megabytes cannot grow the process.
struct BoundedOutput {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
    caps: Caps,
}

type SharedOutput = Arc<Mutex<BoundedOutput>>;

impl BoundedOutput {
    fn shared(caps: Caps) -> SharedOutput {
        Arc::new(Mutex::new(Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            total: 0,
            caps,
        }))
    }

    /// Absorb one read. Chunk boundaries are irrelevant to the result: the head
    /// fills first, the remainder rolls through the tail window.
    fn push(&mut self, chunk: &[u8]) {
        self.total += chunk.len();
        let head_room = self
            .caps
            .head
            .saturating_sub(self.head.len())
            .min(chunk.len());
        let (head_part, tail_part) = chunk.split_at(head_room);
        self.head.extend_from_slice(head_part);
        self.tail.extend(tail_part);
        while self.tail.len() > self.caps.tail {
            self.tail.pop_front();
        }
    }

    fn render(&self) -> String {
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        if self.total <= self.caps.head + self.caps.tail {
            // Nothing was dropped, so head followed by tail is the whole stream.
            let mut whole = self.head.clone();
            whole.extend_from_slice(&tail);
            return decode(&whole);
        }
        // Both ends are cut at arbitrary byte offsets, so trim any partial
        // character rather than emitting replacement chars, then drop the
        // partial *token* each cut left behind. The marker counts every dropped
        // byte, including both trims.
        let head = erode_head(utf8_prefix(&self.head));
        let tail = erode_tail(utf8_suffix(&tail));
        let omitted = self.total - head.len() - tail.len();
        format!(
            "{}\n{}\n{}",
            decode(head),
            (self.caps.marker)(omitted),
            decode(tail)
        )
    }
}

/// The two bounded views of one stream, filled by a single drain.
pub(super) struct Capture {
    ui: SharedOutput,
    log: SharedOutput,
}

impl Capture {
    pub(super) fn new() -> Self {
        Self {
            ui: BoundedOutput::shared(UI_CAPS),
            log: BoundedOutput::shared(LOG_CAPS),
        }
    }

    /// What the UI shows for this stream.
    pub(super) fn ui(&self) -> String {
        render(&self.ui)
    }

    /// What the install log records for this stream.
    pub(super) fn log(&self) -> String {
        render(&self.log)
    }

    fn push(&self, chunk: &[u8]) {
        for sink in [&self.ui, &self.log] {
            if let Ok(mut sink) = sink.lock() {
                sink.push(chunk);
            }
        }
    }
}

/// Called with each complete line an install prints, for the live output line
/// in the UI. Shared across both drain threads of one attempt.
pub(super) type LineObserver = Arc<dyn Fn(&str) + Send + Sync>;

/// Render a capture even if its drain thread panicked mid-write — a poisoned
/// lock must not cost the diagnostics.
fn render(sink: &SharedOutput) -> String {
    sink.lock().unwrap_or_else(|p| p.into_inner()).render()
}

/// Read `pipe` to EOF, feeding fixed-size chunks into `capture` and each
/// complete line to `observer`. Read errors end the drain — a broken pipe means
/// the child is gone and there is nothing left to capture.
pub(super) fn drain_into(mut pipe: impl Read, capture: &Capture, observer: Option<&LineObserver>) {
    let mut chunk = [0u8; 8192];
    let mut lines = LineSplitter::default();
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => {
                capture.push(&chunk[..n]);
                if let Some(observe) = observer {
                    lines.feed(&chunk[..n], |line| observe(line));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

/// Reassembles lines from arbitrary read chunks. A partial trailing line is
/// held until its newline arrives, so an observer only ever sees complete
/// lines. The buffer is capped: a program that prints megabytes without a
/// newline must not grow it without bound.
#[derive(Default)]
struct LineSplitter {
    partial: Vec<u8>,
}

impl LineSplitter {
    /// Longest line reassembled. Beyond this the excess is dropped, since the
    /// consumer displays a single truncated line anyway.
    const MAX_LINE: usize = 4096;

    fn feed(&mut self, chunk: &[u8], mut emit: impl FnMut(&str)) {
        for byte in chunk {
            if *byte == b'\n' {
                let line = String::from_utf8_lossy(&self.partial).trim().to_string();
                self.partial.clear();
                if !line.is_empty() {
                    emit(&line);
                }
            } else if self.partial.len() < Self::MAX_LINE {
                self.partial.push(*byte);
            }
        }
    }
}

/// Rate limiter for the live output line: at most one event per
/// `min_interval`.
///
/// A line arriving inside the window is *held* rather than dropped, and the
/// newest held line replaces any older one. Dropping was wrong at two points:
/// the last line of an attempt — typically the failure that caused the retry —
/// vanished if it landed inside the window, and so did a new attempt's first
/// line when it arrived within 250ms of the previous attempt's last.
pub(super) struct Throttle {
    min_interval: Duration,
    state: Mutex<ThrottleState>,
}

#[derive(Default)]
struct ThrottleState {
    last_emitted: Option<Instant>,
    pending: Option<String>,
}

impl Throttle {
    pub(super) fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            state: Mutex::new(ThrottleState::default()),
        }
    }

    /// Offer one line. `Some` means emit it now; `None` means it is held as the
    /// newest pending line, to be emitted by [`Throttle::take_pending`] or
    /// replaced by a line that supersedes it.
    pub(super) fn offer(&self, line: &str, now: Instant) -> Option<String> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        if state
            .last_emitted
            .is_some_and(|prev| now.duration_since(prev) < self.min_interval)
        {
            state.pending = Some(line.to_string());
            return None;
        }
        state.last_emitted = Some(now);
        // Emitting a newer line makes the held one obsolete: the display shows
        // one line, and it must be the latest.
        state.pending = None;
        Some(line.to_string())
    }

    /// Take the held line, if the window closed on one.
    pub(super) fn take_pending(&self) -> Option<String> {
        self.state.lock().ok()?.pending.take()
    }

    /// Open the window for a new attempt, so its first line is emitted
    /// immediately instead of waiting out the previous attempt's window.
    pub(super) fn restart(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = ThrottleState::default();
        }
    }
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Drop a trailing partial UTF-8 sequence, keeping mid-stream invalid bytes for
/// the lossy decode to mark.
fn utf8_prefix(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes,
        Err(e) if e.error_len().is_none() => &bytes[..e.valid_up_to()],
        Err(_) => bytes,
    }
}

/// Drop leading UTF-8 continuation bytes — at most three can precede a
/// character start.
fn utf8_suffix(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .take(3)
        .take_while(|b| *b & 0b1100_0000 == 0b1000_0000)
        .count();
    &bytes[start..]
}

/// How far a cut edge looks for a token boundary. Sized past any credential
/// shape worth protecting (an `nsec1` key is 63 bytes, registry tokens are
/// shorter) and short enough that erosion costs a token rather than a chunk of
/// output. A cut inside a longer whitespace-free run is left alone: erasing
/// kilobytes of a single-token stream would cost more diagnostics than the
/// fragment could leak.
const MAX_ERODED_TOKEN: usize = 256;

/// Drop the partial token a head cut left at its end.
///
/// Redaction runs on the rendered text and matches whole tokens: a prefixed
/// secret up to the next whitespace, or an exact env value. A cut through the
/// middle of a secret leaves a fragment that matches neither and therefore
/// survives scrubbing, so the fragment is removed here instead — at the cut,
/// where it is still identifiable as partial. The omitted-byte marker counts
/// what this drops.
fn erode_head(bytes: &[u8]) -> &[u8] {
    let window = bytes.len().saturating_sub(MAX_ERODED_TOKEN);
    match bytes[window..].iter().rposition(u8::is_ascii_whitespace) {
        Some(last) => &bytes[..=window + last],
        None => bytes,
    }
}

/// Drop the partial token a tail cut left at its start — the direction that
/// matters most, since a fragment there has lost the `nsec1`-style prefix the
/// scrubber keys on. See [`erode_head`].
fn erode_tail(bytes: &[u8]) -> &[u8] {
    let window = MAX_ERODED_TOKEN.min(bytes.len());
    match bytes[..window].iter().position(u8::is_ascii_whitespace) {
        Some(first) => &bytes[first..],
        None => bytes,
    }
}

#[cfg(test)]
#[path = "install_capture_tests.rs"]
mod tests;
