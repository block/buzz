//! Adversarial slicing cases for resize debt, oversized atoms, and arithmetic extremes.
//!
//! **This is half a suite.** The remaining work-bound cases live in
//! `slicing.rs`; the two files are one set of contracts split only for the
//! file-size ratchet. A mutation check scoped with `--test slicing_adversarial`
//! covers 4 of 76 package tests and can report a confident pass while the
//! killing fixture sits in the sibling file. Sizing from the whole budget does
//! exactly that, then dies under the package.
//!
//! Mutation checks run the package, never a file: `cargo test -p buzz-terminal`.

use buzz_terminal::fences::{
    max_atom_work, max_drain_work, slice_bytes_remaining, Fences, WORK_BUDGET,
};
use buzz_terminal::{Size, Terminal};

/// A scrollback change reprices RIS *and* the slicing derived from it.
///
/// Kills: updating the feeder's columns and lines on resize but not its
/// scrollback -- and, separately, a repair that reprices the charge while
/// leaving slice width stale. Those are different failures and neither
/// observable sees the other: fix only the charge and the drain count stays
/// wrong; fix only the derivation and the charge stays wrong.
///
/// Two properties, because one is not enough:
///
/// * The exact RIS charge at the new depth. Direct, and it is what a
///   pricing-only repair passes.
/// * Equality with a terminal *constructed* at the new depth, across work
///   and drain count. A resized feeder that is genuinely repaired is
///   indistinguishable from one that was born there. This is stronger than a
///   hand-picked threshold and immune to `WORK_BUDGET`/`MIN_SLICE` moving,
///   since both arms move together -- and the sanity arm proves the
///   comparison is deterministic before it is used to judge anything.
///
/// `completed_units` is deliberately *not* the discriminator here: it reads
/// 200 in both arms, because the same callbacks run either way and only their
/// cost and slicing differ. It is asserted anyway as the invariant that must
/// hold -- no unit lost or duplicated across a resize -- while carrying none
/// of the discrimination.
#[test]
fn a_scrollback_change_reprices_the_densest_atom_and_the_slicing() {
    let shallow = Size {
        columns: 200,
        screen_lines: 50,
        scrollback: 100,
    };
    let deep = Size {
        scrollback: 10_000,
        ..shallow
    };
    let cells = (deep.columns * deep.screen_lines) as u64;

    // Preconditions, asserted rather than assumed, because both are easy to
    // break by "generalising" this fixture later:
    //
    // * The geometry must let the *scheduling* fields separate. They only do
    //   when the two depths land on different slice widths, and the deep side
    //   is always floored -- so the shallow side must not be. At 1600x50 the
    //   visible grid alone floors every depth from 0 upward, and three of the
    //   four observables below go silently inert.
    // * The payload must be RIS. It is the only escape reaching the only
    //   weight carrying a scrollback term (`units::reset_state`); DECALN and
    //   every other atom are priced on cells or columns and are blind to
    //   depth, so a conforming repair would show work identical to the
    //   control and the assertions here would invert into false failures.
    assert!(
        slice_bytes_remaining(
            shallow.columns,
            shallow.screen_lines,
            shallow.scrollback,
            0,
            0
        ) > 1,
        "geometry cannot discriminate: the shallow arm is already floored",
    );
    assert_eq!(
        slice_bytes_remaining(deep.columns, deep.screen_lines, deep.scrollback, 0, 0),
        1,
    );

    // How a terminal at `size` retires 200 RIS: work, and how many
    // acquisitions it took. Both are feeder behaviour, not helper output.
    let run = |size: Size, resize_from: Option<Size>| {
        let (mut term, _a) = Terminal::new(resize_from.unwrap_or(size), Fences::ALL);
        if resize_from.is_some() {
            term.resize(size);
        }
        term.reset_stats();
        let mut drains = 1;
        let mut more = term.feed(&b"c".repeat(200));
        while more {
            more = term.drain();
            drains += 1;
        }
        (
            term.stats().completed_units,
            term.stats().completed_work,
            drains,
        )
    };

    let control = run(deep, None);
    let sanity = run(deep, None);
    assert_eq!(
        control, sanity,
        "two terminals built the same way must agree before this comparison          can judge anything",
    );

    let resized = run(deep, Some(shallow));
    assert_eq!(resized.0, 200, "no unit may be lost or duplicated");
    assert_eq!(
        resized, control,
        "a feeder resized to a depth must be indistinguishable from one          constructed at it -- in charge and in how many acquisitions it took",
    );

    // The exact charge, stated rather than inferred from the equality: a
    // repair that made both arms equally *wrong* would pass the comparison.
    let (mut term, _a) = Terminal::new(shallow, Fences::ALL);
    term.resize(deep);
    term.reset_stats();
    term.feed_fully(b"c");
    assert_eq!(
        term.stats().completed_work,
        2 * cells + (deep.scrollback * deep.columns) as u64,
    );

    // Shrinking retains the debt, and the fixture proves retention rather
    // than merely permitting it.
    //
    // `>= fresh` alone is the predicate three of us proposed and all three
    // withdrew: a feeder that dropped the debt reads *exactly* equal to a
    // fresh shallow one, so `>=` passes on the unrepaired state. Strictness
    // on the pricing field is what rejects it. The scheduling fields are
    // asserted directionally with per-field signs -- `first_units` inverts,
    // because a narrower slice retires fewer atoms per un-preemptable drain,
    // which is the fence working -- but none of them is the discriminator:
    // they separate only when the two depths straddle the slice floor, and
    // `completed_work` separates at every positive depth gap.
    //
    // Every comparison is against the fresh control's own field, never a
    // literal: a constant or geometry change must move both sides together,
    // or the fixture starts asserting the arithmetic of the day it was
    // written.
    let measure = |term: &mut Terminal| {
        term.reset_stats();
        let mut drains = 1;
        let mut more = term.feed(&b"\x1bc".repeat(200));
        let first_units = term.stats().completed_units;
        let first_pending = term.pending_bytes();
        while more {
            more = term.drain();
            drains += 1;
        }
        (
            first_units,
            first_pending,
            drains,
            term.stats().completed_units,
            term.stats().completed_work,
        )
    };

    // The terminal under test stays alive past its measurement, so the
    // geometry arm below runs on the feeder that actually shrank rather than
    // on a lookalike that only ever grew.
    let (mut shrunk_term, _a) = Terminal::new(shallow, Fences::ALL);
    shrunk_term.resize(deep);
    shrunk_term.resize(shallow);
    let shrunk = measure(&mut shrunk_term);

    let (mut fresh_term, _a) = Terminal::new(shallow, Fences::ALL);
    let fresh = measure(&mut fresh_term);

    assert_eq!(
        shrunk.3, fresh.3,
        "no unit may be lost on the way down either"
    );
    assert!(
        shrunk.4 > fresh.4,
        "a feeder that has been deep must still price deep after shrinking: \
         {} against a fresh shallow {}. Equality here is the signature of a \
         feeder that dropped the debt, which is indistinguishable from one \
         that never had it",
        shrunk.4,
        fresh.4,
    );
    assert!(
        shrunk.0 <= fresh.0,
        "narrower slices retire fewer atoms per drain: {} against {}",
        shrunk.0,
        fresh.0,
    );
    assert!(
        shrunk.1 >= fresh.1,
        "and leave more pending after the first call: {} against {}",
        shrunk.1,
        fresh.1,
    );
    assert!(
        shrunk.2 >= fresh.2,
        "and take more drains to finish: {} against {}",
        shrunk.2,
        fresh.2,
    );

    // The debt survives a later resize on a different axis. Two things make
    // this arm bite, and it was inert without either:
    //
    // * It runs on the terminal that actually went shallow -> deep ->
    //   shallow. A lookalike that only ever grew passes it while an
    //   implementation that retains on shrink and drops on the next geometry
    //   change fails.
    // * The resize carries the *shallow* depth. Passing the debt's own value
    //   back in means `max(debt, new)` and a plain assignment agree, so the
    //   arm cannot tell them apart -- which is how it survived a mutant that
    //   retained only when columns and lines were unchanged.
    shrunk_term.resize(Size {
        columns: shallow.columns * 2,
        screen_lines: shallow.screen_lines,
        scrollback: shallow.scrollback,
    });
    shrunk_term.reset_stats();
    shrunk_term.feed_fully(b"\x1bc");
    assert_eq!(
        shrunk_term.stats().completed_work,
        2 * (shallow.columns * 2 * shallow.screen_lines) as u64
            + (deep.scrollback * shallow.columns * 2) as u64,
        "a columns resize must keep the deep scrollback debt, not fall back \
         to the current shallow depth",
    );
}

/// One oversized atom per drain -- no callback runs after the one that
/// crosses the budget.
///
/// Kills: sizing slices from the *whole* budget rather than what remains of
/// it. RIS at any real scrollback depth is worth more than an entire budget,
/// so a slice wide enough for several callbacks runs several: measured
/// `completed_units == 3` for `ESC c` followed by `Xmore`, where the law
/// permits exactly one. The fix makes slice width a function of `remaining`,
/// which is a single byte once an atom this size is in play.
///
/// Also asserts the tail survives it: yielding after the crossing atom is
/// only correct if what follows is still parsed, exactly once.
#[test]
fn an_oversized_atom_yields_before_the_next_callback() {
    let size = Size {
        columns: 400,
        screen_lines: 100,
        scrollback: 10_000,
    };
    let (mut term, _a) = Terminal::new(size, Fences::ALL);
    let ris_work =
        2 * (size.columns * size.screen_lines) as u64 + (size.scrollback * size.columns) as u64;
    assert!(
        ris_work > WORK_BUDGET,
        "this arm needs an atom bigger than the whole budget",
    );

    let more = term.feed(b"\x1bcXmore");

    assert!(more, "the drain must yield with a tail");
    assert_eq!(
        term.stats().completed_units,
        1,
        "exactly the crossing atom ran: a callback after it is post-atom \
         overrun, which is the thing the budget cannot preempt and therefore \
         must not start",
    );
    assert_eq!(term.stats().completed_work, ris_work);

    while term.drain() {}
    assert_eq!(
        term.stats().completed_units,
        1 + 5,
        "the five characters after it must still be parsed, exactly once",
    );
    assert_eq!(term.pending_bytes(), 0);
}

/// Extreme dimensions saturate rather than wrapping or panicking.
///
/// Kills: `columns * lines` in `usize` before the cast. `Size` is unclamped
/// and reaches the weight path from a caller, so this product is a reachable
/// overflow -- a debug panic inside the accounting path, or a release wrap
/// that reports the most expensive callback in the emulator as one of the
/// cheapest. Saturating is the only one of the three that fails safe.
#[test]
fn extreme_dimensions_saturate_instead_of_wrapping() {
    let huge = usize::MAX / 2;
    assert_eq!(max_atom_work(huge, huge, huge), u64::MAX);
    assert_eq!(max_drain_work(huge, huge, huge), u64::MAX);

    // The *direction* is the assertion, not merely the absence of a panic.
    // A wrapping build does not produce a slightly-wrong bound, it produces a
    // tiny one -- and `slice_bytes_remaining` divides the budget by it, so an
    // undercharged atom yields an *oversized* slice exactly when the atom is
    // most expensive. Wrapping inverts the fence. So: the widest possible
    // atom must give the narrowest possible slice.
    assert_eq!(
        slice_bytes_remaining(huge, huge, huge, 0, 0),
        1,
        "an overflowing grid must clamp to the smallest slice; a wrapped \
         `max_atom_work` would hand back a generous one",
    );
    assert_eq!(
        slice_bytes_remaining(huge, huge, huge, 0, 0),
        1,
        "and the escape at the front of such a grid gets a single byte",
    );

    // The property behind those endpoints, and the stronger statement: a
    // grid that costs more may never buy a wider slice. Endpoints pin the
    // ends; only a sweep catches a non-monotone middle, and a wrap *is* a
    // non-monotone middle -- it makes the worst grid look cheap and hands it
    // the widest slice of all.
    // Every axis independently: a wrap on any one of the three products is a
    // non-monotone middle on that axis alone, and sweeping only scrollback
    // would miss a truncating `columns * lines`.
    for (axis, at) in [
        (
            "scrollback",
            (|n| slice_bytes_remaining(200, 50, n, 0, 0)) as fn(usize) -> usize,
        ),
        ("columns", |n| slice_bytes_remaining(n.max(1), 50, 0, 0, 0)),
        ("lines", |n| slice_bytes_remaining(200, n.max(1), 0, 0, 0)),
    ] {
        let mut previous = usize::MAX;
        for exponent in 0..60 {
            let width = at(1usize << exponent);
            assert!(
                width <= previous,
                "slice widened from {previous} to {width} at {axis} \
                 2^{exponent}: more expensive grid, more generous slice",
            );
            assert!(width >= 1);
            previous = width;
        }
    }

    // Just past 32 bits on one axis: large enough that a narrowing cast
    // shows (`1 << 32` truncates to 0 in `u32`, pricing an enormous grid at
    // nothing), small enough that the honest answer is exact rather than
    // saturated. Neither the extreme endpoints above nor the ordinary grids
    // below can see this -- the endpoints saturate either way and the
    // ordinary ones fit in 32 bits.
    assert_eq!(max_atom_work(1 << 32, 1, 0), 2 * (1u64 << 32));
    assert_eq!(max_atom_work(1, 1 << 32, 0), 2 * (1u64 << 32));
    assert_eq!(max_atom_work(1, 1, 1 << 32), 2 + (1u64 << 32));

    // Ordinary grids are untouched by the saturation: exact, not clamped.
    assert_eq!(max_atom_work(80, 24, 0), 2 * 80 * 24);
    assert_eq!(max_atom_work(80, 24, 100), 2 * 80 * 24 + 100 * 80);
}

/// An escape split across slices keeps its escape metering.
///
/// Kills: deciding "plain run or escape?" by looking only at the bytes ahead.
/// After a slice ending on a lone `ESC`, the next byte is `c` -- which looks
/// like ordinary text and is in fact a full grid reset. Meter it as text and
/// the oversized atom rides into a wide slice with whatever follows, which is
/// the post-atom overrun arriving through a different door. Found by the
/// oversized-atom fixture failing after I "optimised" the plain path, which
/// is the argument for keeping both.
#[test]
fn an_escape_split_across_slices_keeps_its_metering() {
    let size = Size {
        columns: 400,
        screen_lines: 100,
        scrollback: 10_000,
    };
    let ris_work =
        2 * (size.columns * size.screen_lines) as u64 + (size.scrollback * size.columns) as u64;

    // Deliver the escape one byte at a time, so the parser is left mid-
    // sequence with a tail that begins on the continuation byte.
    let (mut term, _a) = Terminal::new(size, Fences::ALL);
    term.feed(b"\x1b");
    assert_eq!(
        term.stats().completed_units,
        0,
        "ESC alone dispatches nothing"
    );

    let more = term.feed(b"cXmore");

    assert!(more, "the completed RIS must still yield with a tail");
    assert_eq!(
        term.stats().completed_units,
        1,
        "the continuation byte completed a grid reset; nothing may run after it",
    );
    assert_eq!(term.stats().completed_work, ris_work);

    while term.drain() {}
    assert_eq!(term.stats().completed_units, 1 + 5);
    assert_eq!(term.pending_bytes(), 0);
}
