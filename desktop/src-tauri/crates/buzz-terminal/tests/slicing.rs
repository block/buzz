//! The work-denominated slicing seam: what bounds one lock hold, what bounds
//! the queue behind it, and what proves the work was actually done.
//!
//! **This is half a suite.** The adversarial resize and overflow cases live
//! in `slicing_adversarial.rs`, split out for the file-size ratchet; the two
//! files are one set of contracts. A mutation check scoped with
//! `--test slicing` covers 20 of 76 package tests and can report a confident
//! pass while the killing fixture sits in the sibling file. Dropping the
//! scrollback debt does exactly that, then dies under the package.
//!
//! Mutation checks run the package, never a file: `cargo test -p buzz-terminal`.
//!
//! Every assertion here is an **exact** expected value, never a `> 0`. A fix
//! that bounds the lock by *dropping* work instead of deferring it reports a
//! beautiful latency and a perfect screen-content receipt -- DECALN fills the
//! grid with `E`, and the second DECALN overwrites the first, so grid content
//! saturates after one of ten thousand. `completed_units == expected` is the
//! only predicate that separates "deferred the work" from "skipped it", and
//! `> 0` is satisfied by a seam that executed exactly one unit.

use buzz_terminal::fences::{
    max_atom_work, max_drain_work, slice_bytes_remaining, Fences, MAX_SLICE, SYNC_CAP, TAIL_CAP,
    WORK_BUDGET,
};
use buzz_terminal::{Size, Terminal};

const COLUMNS: usize = 200;
const LINES: usize = 50;
const CELLS: u64 = (COLUMNS * LINES) as u64;

fn terminal() -> Terminal {
    Terminal::new(
        Size {
            columns: COLUMNS,
            screen_lines: LINES,
            scrollback: 100,
        },
        Fences::ALL,
    )
    .0
}

/// A deliberately tiny grid, for the arms that must fill the 4 MiB tail.
///
/// Filling the cap is cheap; *draining* it is not, and on a 200x50 grid a
/// full tail of DECALN is ~1e10 work units of real parsing. The cap is a
/// property of the byte depth, not of the grid, so a small grid exercises the
/// same thresholds in seconds instead of minutes -- but it does change what
/// is being tested, so it is named rather than reused silently: these arms
/// test the *depth* predicates, and the arms above test the work bound.
fn tiny() -> Terminal {
    Terminal::new(
        Size {
            columns: 10,
            screen_lines: 2,
            scrollback: 10,
        },
        Fences::ALL,
    )
    .0
}

/// Feed until the tail reaches its cap, or give up.
///
/// Bounded on purpose. A test that loops until a predicate goes true hangs
/// forever when the predicate is what broke, which turns a killed mutant into
/// a wedged CI job -- and a suite that hangs instead of failing is a suite
/// nobody can bisect.
fn fill_tail(term: &mut Terminal, payload: &[u8]) -> bool {
    for _ in 0..10_000 {
        if term.tail_full() {
            return true;
        }
        term.feed(payload);
    }
    false
}

/// Pump to completion, counting acquisitions. A drain that needed no second
/// call returns 1.
fn pump(term: &mut Terminal, bytes: &[u8]) -> usize {
    let mut calls = 1;
    let mut more = term.feed(bytes);
    while more {
        more = term.drain();
        calls += 1;
    }
    calls
}

/// One `feed` may not spend an unbounded amount of work, however much the
/// stream asks for.
///
/// Kills: deleting the `spent >= WORK_BUDGET` break, which restores the
/// unbounded hold this whole seam exists to prevent. Deliberately asserts on
/// *work* rather than wall time -- a time assertion is a flake on a loaded
/// machine, and the work bound is the thing the code actually promises.
#[test]
fn one_feed_spends_at_most_one_budget_plus_a_slice() {
    let mut term = terminal();
    let decalns = 10_000;
    term.feed(&b"\x1b#8".repeat(decalns));

    let spent = term.stats().completed_work;
    // Two terms, both irreducible: the budget is checked between slices, and
    // a slice is sized so it holds at most one budget of the densest payload;
    // and the callback that crosses the line cannot be preempted.
    let ceiling = max_drain_work(COLUMNS, LINES, 100);
    assert!(
        spent <= ceiling,
        "one feed spent {spent} work, over budget+overshoot ({ceiling})",
    );
    assert!(
        term.pending_bytes() > 0,
        "10000 DECALNs is {} work and the budget is {WORK_BUDGET}; if nothing \
         is pending the seam ran the whole payload in one hold",
        decalns as u64 * CELLS,
    );
}

/// Every deferred byte is eventually executed -- exactly once, and all of it.
///
/// Kills: bounding the hold by dropping the remainder instead of keeping it
/// (`self.pending.clear()` in place of the tail), which passes any latency
/// gate and any grid-content check. The unit count is the only witness.
#[test]
fn a_deferred_tail_executes_every_unit_exactly_once() {
    let mut term = terminal();
    let decalns = 10_000;

    let calls = pump(&mut term, &b"\x1b#8".repeat(decalns));

    assert!(
        calls > 1,
        "a payload this dense must have needed a second call"
    );
    assert_eq!(
        term.stats().completed_units,
        decalns as u64,
        "every DECALN must execute exactly once: no drops, no double-parse",
    );
    assert_eq!(term.stats().completed_work, decalns as u64 * CELLS);
    assert_eq!(term.pending_bytes(), 0, "nothing may be left behind");
}

/// The tail drains without another `feed` -- a reader with nothing new to
/// read must still be able to retire what it already accepted.
///
/// Kills: draining only from `feed`, which strands the tail whenever the
/// child goes quiet (`cat bigfile` then no more output: the last screenful
/// never appears).
#[test]
fn a_tail_drains_without_a_second_feed() {
    let mut term = terminal();
    let decalns = 2_000;
    assert!(term.feed(&b"\x1b#8".repeat(decalns)), "expected a tail");

    // Never feed again. Only drain.
    while term.drain() {}

    assert_eq!(term.stats().completed_units, decalns as u64);
    assert_eq!(term.pending_bytes(), 0);
}

/// A slice is cut only at a byte boundary the parser has already passed, so
/// an escape sequence split across two slices still executes once.
///
/// Kills: cutting mid-sequence and restarting the parser, or double-feeding
/// the straddling bytes. `\x1b#8` is 3 bytes and slices are a multiple of
/// neither, so at this length hundreds of sequences straddle a cut.
#[test]
fn a_sequence_split_across_slices_executes_exactly_once() {
    let mut term = terminal();
    let decalns = 3_000;
    pump(&mut term, &b"\x1b#8".repeat(decalns));
    assert_eq!(
        term.stats().completed_units,
        decalns as u64,
        "a straddling sequence was dropped or executed twice",
    );

    // Same payload, delivered one byte per feed: every sequence straddles.
    let mut byte_at_a_time = terminal();
    for chunk in b"\x1b#8".repeat(decalns).chunks(1) {
        byte_at_a_time.feed(chunk);
    }
    while byte_at_a_time.drain() {}
    assert_eq!(byte_at_a_time.stats().completed_units, decalns as u64);
}

/// The tail is a bound on the queue, and the breach counter is loud.
///
/// Kills: a silent cap -- a tail that grows past `TAIL_CAP` without saying
/// so is indistinguishable from a reader that is obeying backpressure, which
/// is exactly the confusion that hides an unbounded queue.
#[test]
fn an_overrun_tail_is_capped_and_counted() {
    let mut term = tiny();
    assert!(!term.tail_full(), "a fresh terminal is not full");
    assert!(term.tail_drained(), "a fresh terminal is drained");
    assert_eq!(term.stats().tail_breaches, 0);

    // A reader that ignores `tail_full` and keeps shovelling.
    assert!(
        fill_tail(&mut term, &b"\x1b#8".repeat(20_000)),
        "the tail never reached its cap: the queue is not bounded",
    );

    assert!(term.pending_bytes() >= TAIL_CAP);
    assert!(
        term.stats().tail_breaches > 0,
        "reaching the cap must be counted, not absorbed silently",
    );
    assert!(!term.tail_drained(), "a full tail is not a drained tail");
}

/// Resume is hysteretic: `tail_drained` does not go true the instant the tail
/// falls one byte below the cap.
///
/// Kills: `tail_drained() == !tail_full()`, which makes a reader flap between
/// paused and reading once per slice at exactly the moment it is most loaded.
#[test]
fn resume_waits_for_a_low_water_mark_not_merely_a_non_full_tail() {
    let mut term = tiny();
    assert!(
        fill_tail(&mut term, &b"\x1b#8".repeat(20_000)),
        "expected a full tail"
    );

    // Drain until the reader is allowed to resume, watching for a window in
    // which it is neither full nor drained -- that gap *is* the hysteresis.
    let mut saw_gap = false;
    for _ in 0..1_000_000 {
        if term.tail_drained() {
            break;
        }
        assert!(term.drain() || term.tail_drained());
        if !term.tail_full() && !term.tail_drained() {
            saw_gap = true;
        }
    }
    assert!(
        term.tail_drained(),
        "the tail never drained to the resume mark"
    );
    assert!(
        saw_gap,
        "no depth was both non-full and non-drained: the two thresholds are \
         the same value and the reader will flap",
    );
}

/// Close must not be held behind parser work.
///
/// Kills: draining the tail on close instead of discarding it. Measured
/// elsewhere in this project: teardown that finishes parsing before killing
/// the child costs ~600 ms on macOS, and no byte of that work reaches a
/// renderer -- publication is detached before shutdown drains.
#[test]
fn close_may_abandon_the_tail_and_says_how_much_it_dropped() {
    let mut term = terminal();
    term.feed(&b"\x1b#8".repeat(10_000));
    let stranded = term.pending_bytes();
    assert!(stranded > 0);

    let abandoned = term.abandon_tail();

    assert_eq!(abandoned, stranded);
    assert_eq!(term.pending_bytes(), 0);
    assert_eq!(
        term.stats().abandoned_bytes,
        stranded as u64,
        "dropped bytes must be counted: this is lossy by design and silent \
         loss is how it stops being by design",
    );
    assert!(
        term.tail_drained(),
        "an abandoned tail cannot strand a reader"
    );
}

/// The grid the weights are priced against tracks resizes.
///
/// Kills: dropping `Feeder::resize`. A stale grid misprices every O(cells)
/// charge for as long as it is wrong -- and it is wrong in the *unsafe*
/// direction whenever the window grows, which is the common case.
#[test]
fn a_resize_reprices_the_same_escape() {
    let mut small = terminal();
    small.feed_fully(b"\x1b#8");
    let before = small.stats().completed_work;
    assert_eq!(before, CELLS);

    small.resize(Size {
        columns: COLUMNS * 2,
        screen_lines: LINES,
        scrollback: 100,
    });
    small.reset_stats();
    small.feed_fully(b"\x1b#8");

    assert_eq!(
        small.stats().completed_work,
        CELLS * 2,
        "the same escape on a grid twice as wide must cost twice as much",
    );
    assert_eq!(small.stats().completed_units, 1, "still one callback");
}

/// A resize *between* slices of one payload reprices the remainder.
///
/// Kills: caching the slice size or the grid across a drain. The tail
/// outlives the call that accepted it, so a resize can land in the middle of
/// it -- the untouched remainder must be charged at the new grid, not the one
/// that was current when the bytes arrived.
#[test]
fn a_resize_mid_tail_reprices_the_remainder() {
    let mut term = terminal();
    let decalns = 4_000;
    assert!(term.feed(&b"\x1b#8".repeat(decalns)), "expected a tail");
    let done_before = term.stats().completed_units;
    let work_before = term.stats().completed_work;
    assert_eq!(work_before, done_before * CELLS);

    term.resize(Size {
        columns: COLUMNS * 2,
        screen_lines: LINES,
        scrollback: 100,
    });
    while term.drain() {}

    let after = term.stats();
    assert_eq!(after.completed_units, decalns as u64, "no unit may be lost");
    assert_eq!(
        after.completed_work,
        work_before + (decalns as u64 - done_before) * CELLS * 2,
        "the remainder must be priced at the resized grid",
    );
}

/// Slice size is derived from the worst atom the grid admits, because a fixed
/// byte count cannot bound a lock hold: `ESC c` is two bytes and resets both
/// grids plus scrollback.
///
/// Kills: replacing `slice_bytes_remaining` with a constant, or deriving it from
/// `cells` while the worst atom is larger than `cells`. Measured: 256 bytes
/// of DECALN is 1.6 ms at 200x50 and ~14 ms at 1600x50, so no one constant
/// serves both.
#[test]
fn slice_size_shrinks_as_the_worst_atom_grows() {
    let small = slice_bytes_remaining(80, 24, 0, 0, 0);
    let large = slice_bytes_remaining(1600, 50, 0, 0, 0);
    assert!(
        small > large,
        "a bigger grid makes each byte more expensive, so slices must shrink: \
         80x24 -> {small}, 1600x50 -> {large}",
    );
    assert!(
        slice_bytes_remaining(200, 50, 10_000, 0, 0) <= slice_bytes_remaining(200, 50, 0, 0, 0),
        "scrollback makes RIS more expensive, so it may only shrink slices",
    );
    for (columns, lines, scrollback) in [(80, 24, 0), (200, 50, 0), (400, 100, 0), (1600, 50, 0)] {
        assert!((1..=MAX_SLICE).contains(&slice_bytes_remaining(columns, lines, scrollback, 0, 0)));
        // One slice holds at most N/2 of the densest atom. Either that fits a
        // budget, or the floor binds -- and then the overshoot is stated by
        // `max_drain_work` rather than being an accident.
        let width = slice_bytes_remaining(columns, lines, scrollback, 0, 0);
        let worst = (width as u64 / 2) * max_atom_work(columns, lines, scrollback);
        assert!(
            worst <= WORK_BUDGET || width == 1,
            "{columns}x{lines}: a slice buys {worst} work against a \
             {WORK_BUDGET} budget without the MIN clamp to excuse it",
        );
    }
}

/// Work released by an F1 abort is counted.
///
/// Kills: leaving the `stop_sync` flush out of the accounting. F1 aborts a
/// runaway synchronized update by flushing its buffer through the handler --
/// those callbacks run, cost time, and hold the lock, so a scheduler that
/// does not see them is blind on exactly the path the fence created. The
/// escapes here are `ESC#8` so the flushed work is unmistakable against the
/// buffered bytes.
#[test]
fn work_flushed_by_a_sync_abort_is_counted() {
    let (mut term, _a) = Terminal::new(
        Size {
            columns: 80,
            screen_lines: 24,
            scrollback: 0,
        },
        Fences::SYNC_ONLY,
    );
    let cells = 80 * 24;

    // Open a synchronized update and never close it: F1 must abort it once
    // the buffer passes SYNC_CAP, flushing everything buffered so far.
    term.feed_fully(b"\x1b[?2026h");
    let decalns = SYNC_CAP / 3 + 1000;
    term.feed_fully(&b"\x1b#8".repeat(decalns));

    let stats = term.stats();
    assert!(stats.sync_aborts > 0, "the fence must have fired");
    // Every DECALN fed must be accounted for. The comparison is against the
    // *input*, not against the counters' own internal consistency: an
    // uncounted flush leaves both counters small together, so checking them
    // against each other would pass over the mutant.
    // Two bookkeeping callbacks besides the DECALNs: the `ESC[?2026h` that
    // opened the update, and the `unset_private_mode` that `stop_sync` emits
    // per abort to report the mode off (`vte-0.15.0/src/ansi.rs:353`).
    let bookkeeping = 1 + stats.sync_aborts;
    assert_eq!(
        stats.completed_units,
        decalns as u64 + bookkeeping,
        "every DECALN must be counted, including the ones released by the \
         abort, plus {bookkeeping} mode callbacks",
    );
    assert_eq!(
        stats.completed_work,
        decalns as u64 * cells + bookkeeping,
        "and their work: {decalns} DECALNs at {cells} cells each",
    );
}

/// Cheap traffic is not taxed by slicing: an ordinary screenful retires in
/// one call.
///
/// Kills: a budget so small, or a slice so small, that normal output pays the
/// deferral machinery. This is the companion to the DECALN arm -- a seam that
/// bounds the hold by making everything slow has not fixed anything.
#[test]
fn ordinary_output_needs_no_second_call() {
    let mut term = terminal();
    let line = b"\x1b[1;32mbuzz\x1b[0m substrate line of output 0123456789\r\n";
    let screenful = line.repeat(LINES);

    assert!(
        !term.feed(&screenful),
        "a screenful of ordinary output must retire in one call, not defer",
    );
    assert_eq!(term.pending_bytes(), 0);
    assert_eq!(term.stats().tail_breaches, 0);
}

/// The work bound holds on the **first drain of a fresh feeder**, for the
/// densest payload upstream offers.
///
/// Kills: sizing slices from observed density. A learned bound is not a bound
/// on the first slice -- a cold feeder has seen nothing, so it hands the
/// parser a wide slice, and a wide slice of `ESC c` spends many budgets
/// before anything checks. This is the arm that a warm-up-based scheduler
/// passes on the second call and fails on the first, so it asserts on a
/// terminal that has never parsed a byte.
#[test]
fn a_cold_feeder_bounds_its_very_first_slice() {
    for (columns, lines) in [(80, 24), (200, 50), (400, 100), (1600, 50)] {
        for (label, atom) in [("RIS", &b"\x1bc"[..]), ("DECALN", &b"\x1b#8"[..])] {
            let (mut term, _a) = Terminal::new(
                Size {
                    columns,
                    screen_lines: lines,
                    scrollback: 100,
                },
                Fences::ALL,
            );
            // Never fed before: `density`-style state, if any existed, is at
            // its initial value.
            term.feed(&atom.repeat(5_000));

            let spent = term.stats().completed_work;
            let ceiling = max_drain_work(columns, lines, 100);
            assert!(
                spent <= ceiling,
                "{label} at {columns}x{lines}: first drain of a cold feeder \
                 spent {spent} work, over budget+overshoot ({ceiling})",
            );
            assert!(term.pending_bytes() > 0, "{label}: expected a tail");
        }
    }
}

/// Exact price of every escape whose cost the grid can amplify.
///
/// One table, exact `completed_work` per escape, at two widths so a weight
/// that dropped its `columns` factor cannot hide. Kills, one row each:
///
/// * `delete_chars`/`insert_blank` charged by `N` -- their cost *falls* as N
///   rises (the swap loop runs `columns - end` times), so N=1 is the worst
///   case and pricing by N is backwards.
/// * `erase_chars` charged raw `N` -- upstream clamps to the row, so
///   `ESC[65535X` on an 80-column grid touches 80 cells, not 65535.
/// * `scroll_up`/`delete_lines` losing their `columns` factor -- the rows are
///   reset, and a row reset is O(columns).
/// * `clear_line`, `decaln`, `clear_screen` mispriced by an axis.
///
/// Exact equality, never a bound: a `<=` assertion passes for every weight
/// smaller than the truth, which is the direction that hurts.
#[test]
fn every_amplifiable_escape_is_priced_exactly() {
    for (columns, lines) in [(80usize, 24usize), (400, 50)] {
        let cells = (columns * lines) as u64;
        let c = columns as u64;
        let cases: &[(&str, String, u64)] = &[
            ("decaln", "\u{1b}#8".into(), cells),
            ("clear_screen", "\u{1b}[2J".into(), cells),
            ("clear_line", "\u{1b}[2K".into(), c),
            ("erase_chars N=1", "\u{1b}[1X".into(), 1),
            ("erase_chars N=20", "\u{1b}[20X".into(), 20),
            ("erase_chars N=huge", "\u{1b}[65535X".into(), c),
            ("delete_chars N=1", "\u{1b}[1P".into(), c),
            ("delete_chars N=huge", "\u{1b}[65535P".into(), c),
            ("insert_blank N=1", "\u{1b}[1@".into(), c),
            ("scroll_up N=1", "\u{1b}[1S".into(), c),
            ("scroll_up N=5", "\u{1b}[5S".into(), 5 * c),
            ("scroll_up N=huge", "\u{1b}[65535S".into(), lines as u64 * c),
            ("scroll_down N=1", "\u{1b}[1T".into(), c),
            ("scroll_down N=4", "\u{1b}[4T".into(), 4 * c),
            (
                "scroll_down N=huge",
                "\u{1b}[65535T".into(),
                lines as u64 * c,
            ),
            ("delete_lines N=3", "\u{1b}[3M".into(), 3 * c),
            (
                "delete_lines N=huge",
                "\u{1b}[65535M".into(),
                lines as u64 * c,
            ),
            ("insert_lines N=1", "\u{1b}[1L".into(), c),
            ("insert_lines N=6", "\u{1b}[6L".into(), 6 * c),
            (
                "insert_lines N=huge",
                "\u{1b}[65535L".into(),
                lines as u64 * c,
            ),
            ("put_tab N=1", "\t".into(), c),
            ("fwd_tabs N=1", "\u{1b}[1I".into(), c),
            ("fwd_tabs N=huge", "\u{1b}[65535I".into(), c),
            ("insert_blank N=huge", "\u{1b}[65535@".into(), c),
            ("clear_line ESC[0K", "\u{1b}[0K".into(), c),
            ("clear_line ESC[1K", "\u{1b}[1K".into(), c),
            ("clear_screen ESC[0J", "\u{1b}[0J".into(), cells),
            ("clear_screen ESC[1J", "\u{1b}[1J".into(), cells),
            ("sgr", "\u{1b}[m".into(), 1),
            ("goto", "\u{1b}[1;1H".into(), 1),
        ];
        for (label, seq, expected) in cases {
            let (mut term, _a) = Terminal::new(
                Size {
                    columns,
                    screen_lines: lines,
                    scrollback: 0,
                },
                Fences::ALL,
            );
            // Home first so nothing scrolls, then measure only the escape.
            term.feed_fully(b"\x1b[1;1H");
            term.reset_stats();
            term.feed_fully(seq.as_bytes());

            assert_eq!(term.stats().completed_units, 1, "{label}: one callback");
            assert_eq!(
                term.stats().completed_work,
                *expected,
                "{label} at {columns}x{lines} priced wrong",
            );
        }
    }
}

/// RIS is priced with its history axis, not just its cells.
///
/// Kills: charging `cells`, or dropping the history term.
///
/// On the alt-screen arm, honestly labelled: the active-`history_size()`
/// mispricing it was written against is **unrepresentable in this design**,
/// not merely untested. `Counting` holds `scrollback` as a scalar copied at
/// construction and has no path to a live grid, so there is no way to write
/// the mutant. The arm is kept as a regression witness -- if a `Term`
/// reference is ever wired into the wrapper it becomes load-bearing the same
/// day -- and both arms are evaluated before either can report, so the
/// primary cannot short-circuit the alt.
#[test]
fn ris_is_priced_for_both_grids_and_the_scrollback_it_walks() {
    let (columns, lines) = (80usize, 24usize);
    let cells = (columns * lines) as u64;
    let mut observed = vec![];
    for scrollback in [0usize, 100, 10_000] {
        for (label, prefix) in [("primary", ""), ("alt screen", "\u{1b}[?1049h")] {
            let (mut term, _a) = Terminal::new(
                Size {
                    columns,
                    screen_lines: lines,
                    scrollback,
                },
                Fences::ALL,
            );
            term.feed_fully(prefix.as_bytes());
            term.reset_stats();
            term.feed_fully(b"\x1bc");
            observed.push((label, scrollback, term.stats().completed_work));
        }
    }
    // One comparison over the whole vector, not a loop of comparisons.
    // Collecting first stops an arm from being *skipped*; asserting the
    // vectors is what stops a failure from being *truncated* to the first
    // mismatch. Otherwise the alt-screen receipt still never prints, which
    // was the point of collecting.
    let expected: Vec<_> = observed
        .iter()
        .map(|&(label, scrollback, _)| {
            (label, scrollback, 2 * cells + (scrollback * columns) as u64)
        })
        .collect();
    assert_eq!(
        observed, expected,
        "RIS must be priced on configured depth, identically on both grids",
    );
}

/// CBT is charged for exactly the cells it scans -- an equality, in both
/// directions.
///
/// Kills: delegating `move_backward_tabs` verbatim, and deleting the
/// fixed-point break. With tabstops cleared and the cursor at the right
/// margin, upstream never advances the cursor, so its `col == 0` exit is
/// unreachable and all N iterations rescan the row -- `ESC[3g ESC[65535Z` is
/// 8 bytes for 82 ms at 1600 columns.
///
/// Two traps this had to be written around, both of which I walked into
/// first:
///
/// * **The cursor is not the witness.** Deleting the break lands on the same
///   column; only the cost differs. A fixture checking where the cursor ended
///   up passes over the mutant.
/// * **An upper bound is not the witness either.** Deleting the break makes
///   the loop run without charging -- measured `work == 1` for 29 ms of real
///   scanning -- so `spent <= bound` *passes*. Under-charging is exactly the
///   direction that hurts, and only an equality sees it.
///
/// The expected value is the scan the source performs: with no stop below the
/// cursor, one pass over `cursor_column` cells, then a permanent fixed point.
/// Both arms come to `columns` -- the telescoping sum of a walk, or one
/// failed pass -- which is the bound this whole change buys.
#[test]
fn the_worst_atom_is_charged_for_exactly_what_it_scans() {
    for columns in [80usize, 400, 1600] {
        let (mut term, _a) = Terminal::new(
            Size {
                columns,
                screen_lines: 50,
                scrollback: 0,
            },
            Fences::ALL,
        );
        // Adversarial for cost: every tabstop gone, cursor at the right
        // margin, count far past the width.
        term.feed_fully(format!("\u{1b}[3g\u{1b}[1;{columns}H").as_bytes());
        term.reset_stats();

        term.feed_fully(b"\x1b[65535Z");

        assert_eq!(term.stats().completed_units, 1, "one escape, one callback");
        assert_eq!(
            term.stats().completed_work,
            1 + (columns as u64 - 1),
            "one failed scan over the whole prefix, then a permanent fixed \
             point: the charge is the escape plus that one scan. A loop that \
             kept going would charge this much per iteration, 65535 times",
        );
        // The real guard on the loop: with a stop reachable, the charge must
        // equal the distance actually travelled. A break-less loop scans the
        // row 65535 times and charges for one crossing.
        let (mut walk, _a) = Terminal::new(
            Size {
                columns,
                screen_lines: 50,
                scrollback: 0,
            },
            Fences::ALL,
        );
        // Default tabstops every 8: from the right margin a huge count walks
        // to column 0, crossing every column on the way.
        walk.feed_fully(format!("\u{1b}[1;{columns}H").as_bytes());
        walk.reset_stats();
        walk.feed_fully(b"\x1b[65535Z");

        assert_eq!(walk.term().grid().cursor.point.column.0, 0);
        assert_eq!(
            walk.stats().completed_work,
            1 + (columns as u64 - 1),
            "the charge must be the distance travelled: one unit for the \
             escape plus one per column crossed",
        );
    }
}

/// CBT at column 0 is free, and stays free.
///
/// Kills: removing the `before == 0` guard. Upstream has its own `col == 0`
/// break, so deleting the wrapper's copy is invisible to the cursor and
/// invisible to timing -- it only shows up as work charged for a scan over
/// zero cells that the wrapper attributed to itself. The left margin is also
/// the position both earlier sweeps of this op homed to, which is why it is
/// the position where a defect hides best.
#[test]
fn the_worst_atom_costs_nothing_at_the_left_margin() {
    for columns in [80usize, 400] {
        let (mut term, _a) = Terminal::new(
            Size {
                columns,
                screen_lines: 50,
                scrollback: 0,
            },
            Fences::ALL,
        );
        term.feed_fully(b"\x1b[3g\x1b[1;1H");
        term.reset_stats();
        term.feed_fully(b"\x1b[65535Z");

        assert_eq!(
            term.stats().completed_work,
            1,
            "at column 0 there is nothing to the left to scan, so the escape \
             costs one unit and no cells",
        );
        assert_eq!(term.term().grid().cursor.point.column.0, 0);
    }
}

/// The other adversary: every tabstop *set*, which maximises the number of
/// delegated single steps rather than the length of one scan.
///
/// Kills: pricing CBT per-step-times-width. Cleared tabstops attack the
/// clamp; all-set attacks the break, forcing `columns - 1` steps of one
/// column each. The two layouts peak in different terms and neither may
/// exceed the bound, so both are here -- a suite that tested only the famous
/// one would miss the shape it chose against.
#[test]
fn the_worst_atom_is_bounded_under_the_layout_that_maximises_steps() {
    for columns in [80usize, 400] {
        let (mut term, _a) = Terminal::new(
            Size {
                columns,
                screen_lines: 50,
                scrollback: 100,
            },
            Fences::ALL,
        );
        // A tabstop in every column, then start from the right margin.
        term.feed_fully(b"\x1b[3g");
        for c in 1..=columns {
            term.feed_fully(format!("\u{1b}[1;{c}H\u{1b}H").as_bytes());
        }
        term.feed_fully(format!("\u{1b}[1;{columns}H").as_bytes());
        term.reset_stats();

        term.feed_fully(b"\x1b[65535Z");

        assert_eq!(term.stats().completed_units, 1);
        assert_eq!(
            term.stats().completed_work,
            1 + (columns as u64 - 1),
            "with a stop in every column the walk crosses each of them once, \
             so the charge is exact: one unit for the escape plus one per \
             column crossed. An inequality here would not catch a 2x \
             overcharge -- which lands on 159, not 160, because the escape's \
             own unit is charged separately and is not doubled",
        );
        assert_eq!(
            term.term().grid().cursor.point.column.0,
            0,
            "with a stop in every column the cursor must walk all the way",
        );
    }
}

/// Stopping CBT early does not change where the cursor lands.
///
/// The companion to the two cost tests above: they assert the work fell,
/// this asserts the behaviour did not move. Kills: stopping at something that
/// is *not* a fixed point -- `min(N, 1)`, or breaking whenever a scan fails
/// even though an earlier step still had stops to find. Cases are the ones
/// the exhaustive probe found interesting: no stops, one stop mid-row, and
/// default stops, each from the right margin with a count past the width.
#[test]
fn stopping_the_worst_atom_early_preserves_its_semantics() {
    let columns = 40usize;
    let cursor_column = |setup: &str| -> usize {
        let (mut term, _a) = Terminal::new(
            Size {
                columns,
                screen_lines: 3,
                scrollback: 0,
            },
            Fences::ALL,
        );
        term.feed_fully(setup.as_bytes());
        term.term().grid().cursor.point.column.0
    };

    // No stops: the cursor cannot move, whatever the count.
    assert_eq!(cursor_column("\u{1b}[3g\u{1b}[1;40H\u{1b}[65535Z"), 39);
    assert_eq!(cursor_column("\u{1b}[3g\u{1b}[1;40H\u{1b}[40Z"), 39);
    // One stop at column 20 (1-based 21): reachable once, then stuck.
    let one_stop = "\u{1b}[3g\u{1b}[1;21H\u{1b}H\u{1b}[1;40H";
    assert_eq!(
        cursor_column(&format!("{one_stop}\u{1b}[65535Z")),
        cursor_column(&format!("{one_stop}\u{1b}[40Z")),
    );
    // Default stops every 8: a large count walks all the way to column 0.
    assert_eq!(cursor_column("\u{1b}[1;40H\u{1b}[65535Z"), 0);
}

/// A stream of atoms each worth more than the whole budget still drains, and
/// every drain makes progress.
///
/// The liveness half of the bound. `max_drain_work` says how much one drain
/// may cost; it says nothing about whether the loop terminates, and an
/// oversized atom is exactly where a work-denominated scheduler could refuse
/// to start one -- spending its budget checking, never advancing, and hanging
/// the terminal with a full tail. RIS on a 10k-scrollback grid is ~16x the
/// budget, so this is not hypothetical.
///
/// Kills: any yield that can decline to start work -- a `width` that reaches
/// 0, a `remaining`-scaled slice that underflows to nothing, a guard that
/// skips a slice deemed too expensive for what is left of the budget. Each of
/// those is a plausible thing to reach for when an atom costs more than the
/// whole budget, and each hangs a terminal on legitimate input.
///
/// Note on a mutant it does *not* kill: moving the budget check from after
/// the slice to before it is **equivalent**, not a defect -- `spent` is zero
/// at entry, so the first slice runs either way. Recorded because I wrote
/// this test believing it caught that, ran the mutant, and it lived.
#[test]
fn atoms_larger_than_the_budget_still_make_progress() {
    for (columns, lines, scrollback) in [(80usize, 24usize, 10_000usize), (200, 50, 10_000)] {
        let (mut term, _a) = Terminal::new(
            Size {
                columns,
                screen_lines: lines,
                scrollback,
            },
            Fences::ALL,
        );
        let atoms = 200usize;
        let bound = max_drain_work(columns, lines, scrollback);
        assert!(
            bound > WORK_BUDGET * 4,
            "this arm is only meaningful where one atom dwarfs the budget",
        );

        let mut more = term.feed(&b"\x1bc".repeat(atoms));
        // `feed` already drained once; seed the baseline with its work or the
        // first delta measured below silently doubles.
        let mut previous = term.stats().completed_work;
        let mut worst = previous;
        let mut calls = 1;
        while more {
            let before = term.pending_bytes();
            more = term.drain();
            assert!(
                term.pending_bytes() < before,
                "no progress: the tail stuck at {before} bytes",
            );
            let now = term.stats().completed_work;
            worst = worst.max(now - previous);
            previous = now;
            calls += 1;
            assert!(calls < 10_000, "drain did not terminate");
        }

        assert_eq!(term.stats().completed_units, atoms as u64, "lost units");
        assert_eq!(term.pending_bytes(), 0);
        assert!(
            worst <= bound,
            "{columns}x{lines}: worst drain spent {worst}, over the stated \
             bound {bound}",
        );
    }
}
