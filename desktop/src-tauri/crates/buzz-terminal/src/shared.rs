//! The lock the reader and the renderer contend for, and the meter on it.
//!
//! This lives in the engine crate rather than in the embedder because the
//! property it exists to prove is a property of the emulator *and* the lock
//! together: F1 bounds how many bytes one `feed` releases into the parser,
//! which bounds how long the reader can hold this mutex, which bounds how long
//! the renderer waits for it. Split the lock out to the Tauri layer and the
//! gate can only be written where no fixture runs.
//!
//! Measured, not assumed. Under a 180 MB/s flood the reader's own hold is
//! p50 1 us while the renderer's *acquire* is p50 4245 us -- four orders apart,
//! because 0.389% of calls carry 96.4% of the lock time. Holding time is the
//! wrong quantity; waiting time is the one a human feels. So the two planes are
//! metered separately: pooling them would let the reader's millions of fast
//! acquires dilute the renderer's tail into a false pass.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use alacritty_terminal::sync::FairMutex;
use parking_lot::MutexGuard;

use crate::damage::{self, Encoder, Frame};
use crate::Terminal;

/// Number of latency buckets. Bucket `i` covers `[2^(i-1), 2^i)` microseconds,
/// so bucket 31 tops out around 35 minutes -- unreachable in practice, which is
/// the point: nothing is silently clamped into the last bucket.
const BUCKETS: usize = 32;

fn bucket_of(micros: u64) -> usize {
    (u64::BITS - micros.leading_zeros()) as usize
}

/// Upper bound of a bucket, in microseconds. Percentiles report this, so a
/// reported latency is never better than what was actually observed.
fn bucket_ceiling(bucket: usize) -> u64 {
    if bucket == 0 {
        0
    } else {
        (1u64 << bucket) - 1
    }
}

/// Lock-acquisition latencies for one plane, recorded without taking a second
/// lock -- an instrument that contends is measuring itself.
#[derive(Debug)]
pub struct AcquireMeter {
    acquisitions: AtomicU64,
    max_micros: AtomicU64,
    buckets: [AtomicU32; BUCKETS],
}

impl Default for AcquireMeter {
    fn default() -> Self {
        Self {
            acquisitions: AtomicU64::new(0),
            max_micros: AtomicU64::new(0),
            buckets: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}

impl AcquireMeter {
    fn record(&self, micros: u64) {
        self.acquisitions.fetch_add(1, Ordering::Relaxed);
        self.max_micros.fetch_max(micros, Ordering::Relaxed);
        self.buckets[bucket_of(micros)].fetch_add(1, Ordering::Relaxed);
    }

    /// Read the counters. Cheap and non-blocking; safe to call from a gate
    /// while the flood is still running.
    pub fn snapshot(&self) -> AcquireStats {
        AcquireStats {
            acquisitions: self.acquisitions.load(Ordering::Relaxed),
            max_micros: self.max_micros.load(Ordering::Relaxed),
            buckets: std::array::from_fn(|i| self.buckets[i].load(Ordering::Relaxed)),
        }
    }

    /// Clear the counters. Diagnostics are per-run.
    pub fn reset(&self) {
        self.acquisitions.store(0, Ordering::Relaxed);
        self.max_micros.store(0, Ordering::Relaxed);
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Relaxed);
        }
    }
}

/// A read of one plane's acquisition latencies.
///
/// `max_micros` is exact because the budget it answers to -- no acquire above
/// one frame at 60 Hz -- is a statement about a single worst event. The
/// distribution is bucketed by powers of two because the budget *it* answers to
/// has 80x of headroom (p95 measured at 49 us against 4 ms), and a factor-of-two
/// resolution against 80x of margin buys nothing for the memory it costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcquireStats {
    pub acquisitions: u64,
    pub max_micros: u64,
    buckets: [u32; BUCKETS],
}

impl AcquireStats {
    /// Latency at percentile `p` (0.0..=1.0), in microseconds, rounded up to
    /// the enclosing bucket's ceiling.
    pub fn percentile_micros(&self, p: f64) -> u64 {
        if self.acquisitions == 0 {
            return 0;
        }
        let target = (self.acquisitions as f64 * p).ceil() as u64;
        let mut seen = 0u64;
        for (bucket, count) in self.buckets.iter().enumerate() {
            seen += *count as u64;
            if seen >= target {
                return bucket_ceiling(bucket);
            }
        }
        self.max_micros
    }
}

/// A [`Terminal`] shared between the PTY reader and the renderer.
pub struct SharedTerminal {
    term: FairMutex<Terminal>,
    reader: AcquireMeter,
    renderer: AcquireMeter,
    closing: AtomicBool,
}

impl SharedTerminal {
    pub fn new(term: Terminal) -> Self {
        Self {
            term: FairMutex::new(term),
            reader: AcquireMeter::default(),
            renderer: AcquireMeter::default(),
            closing: AtomicBool::new(false),
        }
    }

    /// Acquisition latencies for the PTY-reader plane.
    pub fn reader_acquire(&self) -> &AcquireMeter {
        &self.reader
    }

    /// Acquisition latencies for the renderer plane. This is the one with a
    /// budget attached.
    pub fn renderer_acquire(&self) -> &AcquireMeter {
        &self.renderer
    }

    /// Feed PTY output into the emulator. Reader plane.
    ///
    /// Returns whether a tail remains: one acquisition parses one work
    /// budget, then **drops the lock** so the renderer can have it. The
    /// caller pumps [`SharedTerminal::drain`] until it returns false. Doing
    /// the whole buffer under one acquisition is what an unbounded hold *is*,
    /// so it is not offered here.
    pub fn feed(&self, bytes: &[u8]) -> bool {
        let mut term = self.acquire(&self.reader);
        if self.closing.load(Ordering::Acquire) {
            false
        } else {
            term.feed(bytes)
        }
    }

    /// Parse more of the pending tail under a fresh acquisition. Reader
    /// plane. Returns whether any remains.
    pub fn drain(&self) -> bool {
        let mut term = self.acquire(&self.reader);
        if self.closing.load(Ordering::Acquire) {
            false
        } else {
            term.drain()
        }
    }

    /// Feed and pump to completion, re-acquiring between slices.
    pub fn feed_fully(&self, bytes: &[u8]) {
        let mut more = self.feed(bytes);
        while more {
            more = self.drain();
        }
    }

    /// Atomically enter close mode and discard parser work. Subsequent PTY
    /// bytes are raw-drained by the embedder and never reach callbacks.
    pub fn begin_closing(&self) -> usize {
        self.closing.store(true, Ordering::Release);
        self.acquire(&self.reader).abandon_tail()
    }

    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }

    /// Sample damage and encode a frame. Renderer plane.
    ///
    /// The lock covers the copy only; `encode` -- hashing, span grouping,
    /// allocation -- runs after the guard drops, which is worth ~75x in hold
    /// time. The `Encoder` is the caller's because its dedup state is per
    /// consumer, and passing it in keeps the encode off this lock by
    /// construction rather than by remembering to.
    pub fn render(&self, encoder: &mut Encoder) -> Frame {
        let raw = {
            let mut term = self.acquire(&self.renderer);
            damage::capture(&mut term)
        };
        encoder.encode(raw)
    }

    /// Copy the whole viewport for a subscriber that arrived mid-stream.
    /// Renderer plane.
    ///
    /// Attach, reattach, and the successor side of a resize all need the
    /// screen as it stands, not the next thing to change on it. Crucially this
    /// leaves damage alone, so taking a snapshot for a newcomer cannot steal
    /// the incumbent renderer's pending rows -- see [`damage::capture_all`].
    ///
    /// Costs a full grid copy under the lock, so call it on attach rather than
    /// per frame.
    pub fn snapshot(&self, encoder: &mut Encoder) -> Frame {
        let raw = {
            let mut term = self.acquire(&self.renderer);
            damage::capture_all(&mut term)
        };
        encoder.encode(raw)
    }

    /// Move the viewport through scrollback. Renderer plane.
    ///
    /// Positive moves into history; see [`crate::Terminal::scroll`]. Returns
    /// whether it moved, so the caller can skip capture and publication for
    /// the momentum tail that arrives after history has run out.
    pub fn scroll(&self, lines: i32) -> bool {
        self.acquire(&self.renderer).scroll(lines)
    }

    /// Return the viewport to the live edge. Renderer plane. Returns whether
    /// it moved, so an unscrolled terminal costs one comparison per keystroke
    /// and no repaint.
    pub fn scroll_to_bottom(&self) -> bool {
        self.acquire(&self.renderer).scroll_to_bottom()
    }

    /// Apply a coalesced resize. Renderer plane: this competes with the
    /// renderer for the same lock and can hold it for milliseconds.
    pub fn resize(&self, size: crate::Size) -> crate::Viewport {
        self.acquire(&self.renderer).resize(size)
    }

    /// Take the lock for something the methods above don't cover (input,
    /// reading stats). Metered on the renderer plane, since anything
    /// that isn't the read loop competes with the renderer for the same lock.
    pub fn lock(&self) -> MutexGuard<'_, Terminal> {
        self.acquire(&self.renderer)
    }

    /// Modes the renderer/input boundary needs to report alongside frames.
    pub fn input_modes(&self) -> (bool, bool) {
        let term = self.acquire(&self.renderer);
        let mode = term.term().mode();
        (
            mode.contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE),
            mode.contains(alacritty_terminal::term::TermMode::FOCUS_IN_OUT),
        )
    }

    fn acquire(&self, meter: &AcquireMeter) -> MutexGuard<'_, Terminal> {
        let started = Instant::now();
        let guard = self.term.lock();
        meter.record(started.elapsed().as_micros() as u64);
        guard
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fences, Size};

    #[test]
    fn closing_abandons_tail_and_permanently_refuses_parser_callbacks() {
        let (terminal, _actions) = Terminal::new(Size::default(), Fences::ALL);
        let shared = SharedTerminal::new(terminal);
        let payload = b"\x1b#8".repeat(10_000);

        assert!(shared.feed(&payload), "fixture must create parser tail");
        let before = shared.lock().stats();
        let abandoned = shared.begin_closing();
        assert!(abandoned > 0, "close must abandon without draining first");
        assert!(shared.is_closing());

        assert!(!shared.feed(b"parser callback after close"));
        assert!(!shared.drain());
        let after = shared.lock().stats();
        assert_eq!(after.completed_units, before.completed_units);
        assert_eq!(
            after.abandoned_bytes,
            before.abandoned_bytes + abandoned as u64
        );
    }
}
