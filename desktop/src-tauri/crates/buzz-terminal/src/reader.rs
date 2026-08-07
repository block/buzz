//! Fence enforcement around `vte`'s `Processor`.
//!
//! Everything that reaches the terminal's parser goes through [`Feeder::feed`].
//! It is the single place both fences are applied, so there is no route by
//! which bytes become parser-visible without being charged.

use alacritty_terminal::vte::ansi::{Handler, Processor, StdSyncHandler};

use crate::fences::{
    slice_bytes_remaining, FenceStats, Fences, MAX_SLICE, OSC_BUDGET, SYNC_CAP, TAIL_CAP,
    TAIL_RESUME, WORK_BUDGET,
};
use crate::units::{Counting, CursorColumn};

/// Owns the parser and enforces F1/F2 on every byte fed to it.
pub struct Feeder {
    parser: Processor<StdSyncHandler>,
    fences: Fences,
    stats: FenceStats,
    /// Bytes charged since the last F2 reset.
    since_reset: usize,
    /// Bytes accepted but not yet parsed. Grows when arrival outruns
    /// retirement; drained by every [`Feeder::feed`] and [`Feeder::drain`].
    pending: Vec<u8>,
    /// How much of `pending` has already been parsed. Kept as an index rather
    /// than draining the front on every slice, so a large tail is not
    /// re-shuffled once per slice; the prefix is dropped in one go on the next
    /// enqueue.
    pending_at: usize,
    /// The grid the weights are computed against. Tracked here rather than
    /// read from the `Term` because `feed` only has the handler, and kept in
    /// sync by [`Feeder::resize`]: a stale grid misprices every O(cells)
    /// callback for as long as it is wrong.
    columns: usize,
    lines: usize,
    /// Whether the parser is part-way through an escape sequence that has not
    /// yet dispatched. Governs how the next slice is metered -- see
    /// [`crate::fences::slice_bytes_remaining`].
    mid_escape: bool,
    /// Deepest scrollback this feeder has ever been configured for.
    ///
    /// A high-water mark rather than the current depth, and the difference is
    /// not conservatism for its own sake -- the rows are still there. Upstream
    /// frees history lazily: `Storage::shrink_lines` truncates only once the
    /// buffer exceeds the new length by `MAX_CACHE_SIZE`, so immediately after
    /// a decrease the grid still owns rows that a reset must walk. Pricing at
    /// the new depth would charge for a grid that does not exist yet.
    ///
    /// Never lowered, so it needs no clearing transition and cannot go stale
    /// in the unsafe direction. The cost is that a session which shrinks its
    /// scrollback keeps paying the deep price for the rest of its life; the
    /// alternative is a bound that is wrong immediately after every shrink.
    scrollback: usize,
}

impl Feeder {
    pub fn new(fences: Fences, columns: usize, lines: usize, scrollback: usize) -> Self {
        Self {
            parser: Processor::new(),
            fences,
            stats: FenceStats::default(),
            since_reset: 0,
            pending: Vec::new(),
            pending_at: 0,
            mid_escape: false,
            columns,
            lines,
            scrollback,
        }
    }

    /// Track a geometry change, so the cost weights describe the current grid.
    ///
    /// Takes the whole [`crate::Size`] rather than a column/line pair on
    /// purpose. Scrollback is as load-bearing as the other two -- it is most
    /// of RIS's price and therefore most of the slice derivation -- and a
    /// signature that accepted only the dimensions let a caller change the
    /// depth on the `Term` while the feeder kept charging the construction
    /// value. One argument, one ownership boundary, no way to update two of
    /// three.
    pub fn resize(&mut self, size: crate::Size) {
        self.columns = size.columns;
        self.lines = size.screen_lines;
        // Grows only. See the field: a decrease does not immediately free the
        // rows a reset has to walk.
        self.scrollback = self.scrollback.max(size.scrollback);
    }

    pub fn stats(&self) -> FenceStats {
        self.stats
    }

    pub fn reset_stats(&mut self) {
        self.stats.reset();
    }

    /// Bytes currently buffered inside a synchronized update.
    pub fn pending_sync_bytes(&self) -> usize {
        self.parser.sync_bytes_count()
    }

    /// Bytes accepted but not yet parsed, because a previous [`Feeder::feed`]
    /// spent its work budget before reaching them.
    pub fn pending_bytes(&self) -> usize {
        self.pending.len() - self.pending_at
    }

    /// Whether the pending tail has reached [`TAIL_CAP`].
    ///
    /// Deliberately derived from the current depth rather than latched. A
    /// latch is a state the fence owns and could fail to clear, which is
    /// exactly how a paused reader strands a child mid-teardown; a reader that
    /// simply stops asking resumes by default.
    ///
    /// **No production consumer today, and not an oversight.** The runtime
    /// reader pumps [`Feeder::drain`] to completion after every read
    /// (`terminal_runtime.rs`), so the tail is empty between iterations and
    /// this can never go true -- measured 0 bytes high-water against 8 MiB of
    /// pure RIS, the densest atom there is. It exists for a future reader
    /// that defers pumping, and such a reader **must** consult it: without
    /// the pump loop the same stream reaches [`TAIL_CAP`] in 257 reads of
    /// 16 KiB.
    ///
    /// The numbers are here rather than "nothing calls this" because the
    /// signal and the loop are one fact from two sides. Delete the loop and
    /// this predicate stops being unreachable in the same instant it starts
    /// being needed.
    pub fn tail_full(&self) -> bool {
        self.pending_bytes() >= TAIL_CAP
    }

    /// Whether a paused reader may resume: the tail has drained to the low
    /// water mark. Separate from `!tail_full()` so the reader does not flap
    /// between full and one-byte-below-full.
    pub fn tail_drained(&self) -> bool {
        self.pending_bytes() <= TAIL_RESUME
    }

    /// Discard the unparsed tail.
    ///
    /// For session close only, and lossless where it is used: publication is
    /// detached before shutdown drains, so this tail is bytes no renderer can
    /// consume. Draining the *PTY* remains lifecycle-critical -- this exists so
    /// parser work cannot hold teardown behind it.
    pub fn abandon_tail(&mut self) -> usize {
        let abandoned = self.pending_bytes();
        self.pending.clear();
        self.pending_at = 0;
        self.stats.abandoned_bytes += abandoned as u64;
        abandoned
    }

    /// Accept PTY output and parse what fits in one work budget.
    ///
    /// Returns whether bytes remain unparsed. Bytes beyond the budget are
    /// retained and parsed by [`Feeder::drain`], so this bounds the *lock
    /// hold*; it does not bound the queue. When arrival outruns retirement
    /// the tail grows to [`TAIL_CAP`] and [`Feeder::tail_full`] goes true,
    /// which is the reader's cue to stop reading the PTY and let the child
    /// block. No policy is applied here: a fence that dropped input to protect
    /// itself would corrupt the screen to avoid being slow.
    pub fn feed<H: Handler + CursorColumn>(&mut self, handler: &mut H, bytes: &[u8]) -> bool {
        self.enqueue(bytes);
        self.drain(handler);
        self.pending_bytes() > 0
    }

    /// Append to the pending tail, compacting the already-parsed prefix first.
    fn enqueue(&mut self, bytes: &[u8]) {
        if self.pending_at > 0 {
            self.pending.drain(..self.pending_at);
            self.pending_at = 0;
        }
        self.pending.extend_from_slice(bytes);
    }

    /// Parse from the pending tail until the work budget is spent.
    ///
    /// The budget is checked between parser slices, never inside a callback:
    /// vte's own mid-buffer stop is driven by `Perform::terminated()`, whose
    /// implementor in the ansi layer is private, so the cut has to be made
    /// from outside, and a callback already running cannot be preempted at
    /// all. One atom is therefore the irreducible overrun -- and it is not
    /// small: `ESC[65535Z` with tabstops cleared is 8 bytes and 82 ms at 1600
    /// columns, because upstream's `move_backward_tabs` rescans the row once
    /// per count when it finds no stop.
    ///
    /// What *is* bounded is the number of atoms per slice, and that bound
    /// holds from the first byte of a cold feeder: [`slice_bytes_remaining`] is derived
    /// from the densest work-per-byte upstream can produce on this grid, so
    /// no slice can contain more than one budget's worth of callbacks no
    /// matter what the payload is or what the feeder has seen before.
    ///
    /// Returns the work spent, which is at least the budget whenever the tail
    /// is still non-empty on return.
    pub fn drain<H: Handler + CursorColumn>(&mut self, handler: &mut H) -> u64 {
        // Slices are copied out of the tail rather than borrowed from it,
        // because `advance_slice` needs `&mut self` and the tail is part of
        // self. A stack buffer keeps that from allocating; the copy is a
        // memcpy against a parse two orders of magnitude more expensive.
        let mut buf = [0u8; MAX_SLICE];
        let mut spent: u64 = 0;
        while self.pending_at < self.pending.len() {
            // Size each slice against what is *left* of the budget, and
            // against what is actually in front of the parser. A slice can
            // only be as expensive as the callbacks it contains, and only an
            // escape can buy grid-sized work in two bytes -- so a plain run
            // is sliced against the plain-byte cost and stops at the next
            // `ESC`, which then gets a slice metered against the worst atom.
            // The drain therefore returns on the atom that crosses the
            // budget, not at the end of a slice that ran several more.
            //
            // Where one atom is worth more than the entire budget -- RIS at
            // any real scrollback depth -- that escape gets a one-byte slice.
            // That is the honest consequence of the law: nothing wider can
            // promise to stop after the crossing atom when a single atom
            // always crosses.
            // A slice is never wider than MAX_SLICE, so the scan for the next
            // escape stops there too: searching the whole tail would be
            // O(tail) per slice and O(tail^2) per drain, which measured as a
            // 7x throughput *regression* on plain text -- a bound that costs
            // more than the thing it bounds.
            let horizon = (self.pending_at + MAX_SLICE).min(self.pending.len());
            let next_escape = if self.mid_escape {
                // Already inside a sequence whose callback has not fired. Its
                // remaining bytes are *not* plain text -- `ESC` then `c` is a
                // grid reset -- so they keep the escape's metering. Without
                // this the byte after a lone `ESC` is priced as a character
                // and the atom rides into a wide slice with whatever follows
                // it, which is the post-atom overrun by another door.
                0
            } else {
                self.pending[self.pending_at..horizon]
                    .iter()
                    .position(|&b| b == 0x1b)
                    .unwrap_or(horizon - self.pending_at)
            };
            let width = slice_bytes_remaining(
                self.columns,
                self.lines,
                self.scrollback,
                spent,
                next_escape,
            );
            let end = (self.pending_at + width).min(self.pending.len());
            let len = end - self.pending_at;
            buf[..len].copy_from_slice(&self.pending[self.pending_at..end]);
            self.pending_at = end;
            let cost = self.advance_slice(handler, &buf[..len]);
            // A slice that contained an escape but dispatched nothing left the
            // parser mid-sequence. Work is the signal because it is the thing
            // being budgeted: a sequence that has not yet cost anything has
            // not yet run.
            self.mid_escape = (self.mid_escape || buf[..len].contains(&0x1b)) && cost == 0;
            spent = spent.saturating_add(cost);
            if spent >= WORK_BUDGET {
                break;
            }
        }
        if self.pending_at == self.pending.len() {
            self.pending.clear();
            self.pending_at = 0;
        }
        let depth = self.pending_bytes();
        self.stats.max_pending = self.stats.max_pending.max(depth);
        if depth >= TAIL_CAP {
            self.stats.tail_breaches += 1;
        }
        spent
    }

    /// Parse one slice, applying both fences to it. Returns the work it cost.
    fn advance_slice<H: Handler + CursorColumn>(&mut self, handler: &mut H, bytes: &[u8]) -> u64 {
        let mut spent: u64 = 0;
        let sync_before = self.parser.sync_bytes_count();
        {
            let mut counting = Counting::new(handler, self.columns, self.lines, self.scrollback);
            self.parser.advance(&mut counting, bytes);
            self.stats.completed_units =
                self.stats.completed_units.saturating_add(counting.units());
            self.stats.completed_work = self.stats.completed_work.saturating_add(counting.work());
            spent = spent.saturating_add(counting.work());
        }
        let sync_after = self.parser.sync_bytes_count();

        // Charge exactly the bytes the parser could see, by route:
        //
        // * the buffer shrank  -> a synchronized update ended and released
        //   `sync_before` buffered bytes plus whatever of `bytes` followed it.
        //   Charging only `bytes` here is the "omitted flush accounting"
        //   mutation: it under-charges by the whole buffered frame.
        // * the buffer grew    -> these bytes were swallowed into the buffer
        //   and are not yet parser-visible. Charging them now is the "raw
        //   counting" mutation: it over-charges, and resets the parser in the
        //   middle of a legitimate frame, destroying content.
        // * neither            -> ordinary unsynchronized input.
        let charged = if sync_after < sync_before {
            let released = sync_before + bytes.len() - sync_after;
            self.note_release(released);
            released
        } else if sync_after > sync_before {
            // Buffered, not yet visible. Charged when it is released.
            0
        } else {
            bytes.len()
        };
        self.charge(charged);

        // F1: a synchronized update may not buffer without bound. One abort
        // per breach; the released bytes are parser-visible and are charged.
        if self.fences.sync_abort && self.parser.sync_bytes_count() >= SYNC_CAP {
            let released = self.parser.sync_bytes_count();
            // Counted too: aborting flushes the buffered frame through the
            // handler, so these are units the lock hold paid for. Leaving them
            // out would undercount exactly on the fenced path.
            {
                let mut counting =
                    Counting::new(handler, self.columns, self.lines, self.scrollback);
                self.parser.stop_sync(&mut counting);
                self.stats.completed_units =
                    self.stats.completed_units.saturating_add(counting.units());
                self.stats.completed_work =
                    self.stats.completed_work.saturating_add(counting.work());
                spent = spent.saturating_add(counting.work());
            }
            self.stats.sync_aborts += 1;
            self.note_release(released);
            self.charge(released);
        }

        // F2: rebuild the parser once the budget is spent. Unconditional --
        // a fresh `Processor` is the only way to discard parser state that a
        // hostile stream is holding open, and it must not depend on the
        // parser agreeing that it is in a bad state.
        if self.fences.osc_budget && self.since_reset >= OSC_BUDGET {
            self.parser = Processor::new();
            self.stats.osc_resets += 1;
            self.since_reset = 0;
        }

        spent
    }

    fn charge(&mut self, bytes: usize) {
        self.since_reset += bytes;
        self.stats.charged_bytes += bytes as u64;
    }

    fn note_release(&mut self, bytes: usize) {
        if bytes > self.stats.max_release {
            self.stats.max_release = bytes;
        }
    }
}
