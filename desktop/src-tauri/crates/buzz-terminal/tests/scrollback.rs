//! Reaching the scrollback the engine has always been keeping.
//!
//! The grid retains 10k lines in production and, before this, nothing could
//! move the viewport off the live edge. Three things have to hold at once for
//! that to become usable, and each one fails silently on its own:
//!
//! 1. **Direction.** A flipped sign still scrolls, still clamps, and still
//!    repaints. Only a human notices. So the direction is asserted here, in
//!    test names, rather than left to the caller to get right.
//! 2. **Coordinates.** Capture reads screen rows out of a grid indexed from
//!    the live edge. Off-by-the-offset shows *some* plausible text.
//! 3. **Dedup.** The renderer's per-row hashes describe the screen it last
//!    saw. Scrolling changes every row without changing the grid, so a scroll
//!    that consumed the full-damage flag would leave those hashes describing
//!    a viewport that is no longer shown -- and they would then suppress a row
//!    that really did change.

use buzz_terminal::damage::{Encoder, Frame};
use buzz_terminal::fences::Fences;
use buzz_terminal::{Action, SharedTerminal, Size, Terminal};
use std::sync::mpsc::Receiver;

/// The receiver is returned rather than dropped: dropping it disconnects the
/// channel and every subsequent listener send silently fails.
fn terminal(
    columns: usize,
    screen_lines: usize,
    scrollback: usize,
) -> (SharedTerminal, Receiver<Action>) {
    let size = Size {
        columns,
        screen_lines,
        scrollback,
    };
    let (term, actions) = Terminal::new(size, Fences::ALL);
    (SharedTerminal::new(term), actions)
}

/// The text of every row the frame carries, indexed by screen row.
///
/// Blank rows are kept as empty strings rather than filtered out: this suite
/// is about *which row shows which line*, and dropping the blanks would
/// renumber every row after one.
fn rows_by_line(frame: &Frame) -> Vec<(usize, String)> {
    frame
        .rows
        .iter()
        .map(|row| {
            (
                row.line,
                row.spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_string(),
            )
        })
        .collect()
}

/// Just the text, in screen order. Only meaningful for a full frame.
fn screen(frame: &Frame) -> Vec<String> {
    rows_by_line(frame)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// Fill history with numbered lines, then take a caught-up renderer.
///
/// Returns the terminal and an encoder that has already consumed the damage
/// from that output, so anything a later assertion sees is caused by the
/// thing under test rather than by the fixture.
fn scrolled_terminal(lines: usize) -> (SharedTerminal, Receiver<Action>, Encoder) {
    let (shared, actions) = terminal(20, 4, 100);
    let payload = (1..=lines)
        .map(|n| format!("L{n:02}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    shared.feed_fully(payload.as_bytes());
    let mut renderer = Encoder::new();
    let _ = shared.render(&mut renderer);
    (shared, actions, renderer)
}

#[test]
fn the_fixture_starts_at_the_live_edge_showing_the_newest_lines() {
    let (shared, _actions, _) = scrolled_terminal(10);
    let mut encoder = Encoder::new();
    assert_eq!(
        screen(&shared.snapshot(&mut encoder)),
        vec!["L07", "L08", "L09", "L10"]
    );
    assert_eq!(shared.lock().display_offset(), 0);
}

/// **The direction, at the engine boundary.** Positive goes *into* history.
///
/// This is upstream's convention and the reason the embedder negates the DOM
/// delta exactly once. If this assertion and `terminal_scroll`'s negation are
/// ever flipped together the pair still passes -- which is why the embedder's
/// own direction test asserts against the DOM sign rather than against this
/// one.
#[test]
fn positive_lines_scroll_backwards_into_history() {
    let (shared, _actions, _) = scrolled_terminal(10);

    assert!(shared.scroll(2), "two lines of history exist to move into");

    let mut encoder = Encoder::new();
    assert_eq!(
        screen(&shared.snapshot(&mut encoder)),
        vec!["L05", "L06", "L07", "L08"],
        "scrolling back two lines must show two older lines"
    );
    assert_eq!(shared.lock().display_offset(), 2);
}

#[test]
fn negative_lines_scroll_forwards_towards_the_live_edge() {
    let (shared, _actions, _) = scrolled_terminal(10);
    assert!(shared.scroll(3));

    assert!(shared.scroll(-1), "one line back towards the edge");

    let mut encoder = Encoder::new();
    assert_eq!(
        screen(&shared.snapshot(&mut encoder)),
        vec!["L05", "L06", "L07", "L08"]
    );
    assert_eq!(shared.lock().display_offset(), 2);
}

/// The momentum guard. A trackpad flick keeps delivering events for about a
/// second after the fingers lift; once history runs out every one of them
/// must be free.
#[test]
fn scrolling_past_the_oldest_line_clamps_and_reports_no_movement() {
    let (shared, _actions, _) = scrolled_terminal(10);
    // Six lines of history: ten written, four on screen.
    assert!(shared.scroll(6));
    assert_eq!(shared.lock().display_offset(), 6);

    assert!(
        !shared.scroll(1),
        "there is nothing older, so nothing moved"
    );
    assert!(
        !shared.scroll(1_000),
        "and a whole flick of it still moves nothing"
    );
    assert_eq!(shared.lock().display_offset(), 6);

    let mut encoder = Encoder::new();
    assert_eq!(
        screen(&shared.snapshot(&mut encoder)),
        vec!["L01", "L02", "L03", "L04"],
        "the top of history is the oldest line, not a blank grid"
    );
}

#[test]
fn scrolling_forwards_at_the_live_edge_reports_no_movement() {
    let (shared, _actions, _) = scrolled_terminal(10);
    assert!(!shared.scroll(-1));
    assert!(!shared.scroll(-1_000));
    assert_eq!(shared.lock().display_offset(), 0);
}

#[test]
fn snapping_to_the_bottom_moves_only_when_scrolled_back() {
    let (shared, _actions, _) = scrolled_terminal(10);
    assert!(
        !shared.scroll_to_bottom(),
        "already live: a keystroke must not cost a repaint"
    );

    assert!(shared.scroll(4));
    assert!(shared.scroll_to_bottom(), "scrolled back: this is the snap");
    assert_eq!(shared.lock().display_offset(), 0);

    let mut encoder = Encoder::new();
    assert_eq!(
        screen(&shared.snapshot(&mut encoder)),
        vec!["L07", "L08", "L09", "L10"]
    );
}

/// Why the snap has to exist at all: output does **not** bring the viewport
/// back. The grid pins a scrolled-back viewport and piles new lines above it
/// (`Grid::scroll_up` advances `display_offset` when it is non-zero), which is
/// the behaviour you want while reading -- and means the echo of a keystroke
/// would otherwise land on a screen the user cannot see.
#[test]
fn output_while_scrolled_back_leaves_the_viewport_where_the_reader_put_it() {
    let (shared, _actions, mut renderer) = scrolled_terminal(10);
    assert!(shared.scroll(3));

    shared.feed_fully(b"\r\nL11\r\nL12");

    let mut encoder = Encoder::new();
    assert_eq!(
        screen(&shared.snapshot(&mut encoder)),
        vec!["L04", "L05", "L06", "L07"],
        "the reader stays put while new output accumulates below"
    );

    // And the snap still returns to the *new* live edge, not the old one.
    assert!(shared.scroll_to_bottom());
    let after = shared.render(&mut renderer);
    assert!(after.full, "a viewport move is a repaint");
    assert_eq!(screen(&after), vec!["L09", "L10", "L11", "L12"]);
}

/// **The silent-corruption case.**
///
/// The renderer's `Encoder` holds one content hash per screen row. Scrolling
/// changes what every row shows without changing a single cell, so those
/// hashes are stale the instant the viewport moves. The engine's protection is
/// that `scroll_display` marks the grid fully damaged and the embedder
/// republishes via `snapshot`, which does not consume damage -- so the
/// full-damage flag survives for the renderer's own next `render()`, which is
/// what clears its hashes.
///
/// The discriminating part is the row content. Row 0 after the scroll holds
/// `L04`; if a stale hash for row 0 -- taken when it held `L07` -- survived,
/// the row would still ship, because the hashes differ. So the test scrolls to
/// a position where the *pre-scroll* text reappears at the *same screen row*:
/// scrolling back 4 puts `L03..L06` on screen, and then scrolling forward 4
/// restores exactly the rows the hashes describe. A renderer whose hashes were
/// never cleared suppresses the whole screen there, and the user is left
/// looking at history that has scrolled away.
#[test]
fn a_scroll_does_not_leave_the_renderer_deduping_against_a_viewport_it_no_longer_shows() {
    let (shared, _actions, mut renderer) = scrolled_terminal(10);

    // The embedder's scroll path: move, then republish by snapshot.
    assert!(shared.scroll(4));
    let mut scroll_encoder = Encoder::new();
    let republished = shared.snapshot(&mut scroll_encoder);
    assert_eq!(screen(&republished), vec!["L03", "L04", "L05", "L06"]);

    // The renderer thread's own next capture must still be told to repaint.
    let after_scroll = shared.render(&mut renderer);
    assert!(
        after_scroll.full,
        "the scroll's snapshot must not have eaten the full-damage flag"
    );
    assert_eq!(screen(&after_scroll), vec!["L03", "L04", "L05", "L06"]);

    // Now back to where the renderer's *original* hashes were taken. Every row
    // matches a hash it already holds, so only a cleared cache ships them.
    assert!(shared.scroll(-4));
    let mut back_encoder = Encoder::new();
    let _ = shared.snapshot(&mut back_encoder);
    let after_return = shared.render(&mut renderer);
    assert!(after_return.full);
    assert_eq!(
        screen(&after_return),
        vec!["L07", "L08", "L09", "L10"],
        "returning to a previously-hashed viewport must still repaint it"
    );
}

/// A row that genuinely changes while the viewport is scrolled back must
/// still reach the renderer. This is the same dedup hazard from the other
/// side: content changing under a stale hash rather than a stale hash under
/// unchanged content.
#[test]
fn a_row_that_changes_while_scrolled_back_still_ships() {
    let (shared, _actions, mut renderer) = scrolled_terminal(10);
    assert!(shared.scroll(2));
    let mut scroll_encoder = Encoder::new();
    let _ = shared.snapshot(&mut scroll_encoder);
    let _ = shared.render(&mut renderer);

    // Rewrite the top line of the active area, which is screen row 2 while
    // scrolled back two.
    shared.feed_fully(b"\x1b[1;1HCHANGED\x1b[K");

    let frame = shared.render(&mut renderer);
    let changed = rows_by_line(&frame)
        .into_iter()
        .find(|(_, text)| text == "CHANGED");
    assert_eq!(
        changed,
        Some((2, "CHANGED".to_string())),
        "the rewritten active row must ship, at its scrolled screen position; got {:?}",
        rows_by_line(&frame)
    );
}

/// The cursor plane travels with the viewport, because the renderer paints it
/// at a screen row and the grid stores it at an active-area row.
#[test]
fn the_cursor_moves_down_the_screen_as_the_viewport_scrolls_back() {
    let (shared, _actions, _) = scrolled_terminal(10);
    let mut encoder = Encoder::new();
    let live = shared.snapshot(&mut encoder);
    assert_eq!(live.cursor.line, 3, "cursor sits on the last active row");
    assert!(live.cursor.visible);

    assert!(shared.scroll(2));
    let mut scrolled_encoder = Encoder::new();
    let scrolled = shared.snapshot(&mut scrolled_encoder);
    assert_eq!(
        scrolled.cursor.line, 3,
        "row 3 + 2 is off a four-row screen, so it clamps to the last row"
    );
    assert!(
        !scrolled.cursor.visible,
        "scrolled off the bottom, so it must not be painted on an unrelated line"
    );
}

/// The clamp above is not the whole story: a cursor that is merely pushed
/// *down* -- still on screen -- must report its new row, not its old one. A
/// capture that ignored the offset entirely would pass the clamp test above
/// (row 3 is where the cursor already was) and fail this one.
///
/// Parking the cursor on the top row with `ESC[H` is what leaves it room to
/// move: at the live edge it is on row 0, and scrolling back two puts it on
/// row 2 of a four-row screen, still visible.
#[test]
fn a_cursor_still_on_screen_reports_its_scrolled_row() {
    let (shared, _actions, _) = scrolled_terminal(10);
    shared.feed_fully(b"\x1b[H");

    let mut live_encoder = Encoder::new();
    let live = shared.snapshot(&mut live_encoder);
    assert_eq!(live.cursor.line, 0, "parked on the top row");
    assert!(live.cursor.visible);

    assert!(shared.scroll(2));
    let mut encoder = Encoder::new();
    let frame = shared.snapshot(&mut encoder);
    assert_eq!(
        frame.cursor.line, 2,
        "the caret follows the row it is written on down the screen"
    );
    assert!(
        frame.cursor.visible,
        "still inside the viewport, so still painted"
    );
}

#[test]
fn the_cursor_becomes_visible_again_on_the_way_back() {
    let (shared, _actions, _) = scrolled_terminal(10);
    assert!(shared.scroll(3));
    assert!(shared.scroll_to_bottom());

    let mut encoder = Encoder::new();
    let frame = shared.snapshot(&mut encoder);
    assert_eq!(frame.cursor.line, 3);
    assert!(frame.cursor.visible);
}

/// The alternate screen has no scrollback by construction: `Term::new` builds
/// the inactive grid with a zero scroll limit. So scrolling inside `vim` or
/// `less` must be a clamped no-op, leaving the application's own scrolling to
/// the application. Asserted rather than assumed -- a viewport that drifted
/// here would show the primary screen's history behind a full-screen app.
#[test]
fn the_alternate_screen_has_no_scrollback_to_reach() {
    let (shared, _actions, _) = scrolled_terminal(10);

    shared.feed_fully(b"\x1b[?1049h");
    shared.feed_fully(b"ALT");

    assert!(!shared.scroll(1), "no history exists on the alt screen");
    assert!(!shared.scroll(1_000));
    assert_eq!(shared.lock().display_offset(), 0);

    // And the primary screen's position is undisturbed on the way back.
    shared.feed_fully(b"\x1b[?1049l");
    assert!(shared.scroll(2));
    assert_eq!(shared.lock().display_offset(), 2);
}

/// A terminal configured with no history cannot scroll at all. The guard is
/// upstream's clamp against `history_size()`, and this pins it: without it the
/// offset would advance and capture would index above the grid.
#[test]
fn a_terminal_without_scrollback_never_moves() {
    let (shared, _actions) = terminal(20, 4, 0);
    shared.feed_fully(b"a\r\nb\r\nc\r\nd\r\ne\r\nf");

    assert!(!shared.scroll(1));
    assert!(!shared.scroll(1_000));
    assert_eq!(shared.lock().display_offset(), 0);

    let mut encoder = Encoder::new();
    assert_eq!(
        screen(&shared.snapshot(&mut encoder)),
        vec!["c", "d", "e", "f"]
    );
}
