//! The one place the DOM's wheel sign meets the engine's scroll sign.
//!
//! Its own module because the crossing is the whole subject: the conversion is
//! two characters of code and roughly a hundred lines of tests and reasoning
//! about which way is back, and inlining that into the command file buries it
//! among unrelated session plumbing.

use buzz_terminal::SharedTerminal;
use serde::Deserialize;

/// A wheel delta in whole cells, carrying the **DOM's** sign.
///
/// A newtype rather than a bare `i32` so the engine's opposite convention
/// cannot be reached by accident: `SharedTerminal::scroll` takes an `i32`, so
/// handing it this value straight from the wire is a type error rather than a
/// silently reversed terminal. Deleting the conversion is caught by the tests
/// below; *bypassing* it is caught by the compiler.
///
/// Serde already reads a one-field tuple struct as its inner value, so no
/// `transparent` attribute is needed -- verified by the wire test below, which
/// deserializes from a bare number.
#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct DomLines(i32);

/// Scroll `terminal` by a DOM wheel delta in cells.
///
/// This is the only place the two sign conventions meet, so it is a named
/// function rather than a `-` in an argument list: inlined, the conversion is
/// executed by every scroll test and asserted by none of them, and deleting it
/// leaves both the Rust and the frontend suites green while reversing the
/// terminal on a real trackpad. It takes the terminal rather than returning a
/// number so the tests below drive the same call `terminal_scroll` makes,
/// instead of a conversion that agrees with itself.
///
/// The DOM sign is the spec because the OS-reported delta is the only stable
/// thing here. macOS "natural scrolling" flips what a given finger motion
/// reports, so a rule stated as "swipe up goes back" is correct for one
/// preference setting and backwards for the other. Stated against `deltaY` it
/// is correct for both, and matches whatever the page around the terminal
/// does: **negative DOM lines -- the direction that scrolls a web page toward
/// the top of the document -- go back into history**, which is positive in the
/// engine's `Scroll::Delta` convention.
///
/// `saturating_neg` because this arrives over IPC: plain unary negation on
/// `i32::MIN` panics in debug and wraps to `i32::MIN` in release, so a crafted
/// command could scroll the wrong way. Saturating leaves it going back, which
/// the engine then clamps at the oldest line.
///
/// Returns whether the viewport actually moved.
pub(super) fn scroll_by_dom_lines(terminal: &SharedTerminal, dom_lines: DomLines) -> bool {
    terminal.scroll(dom_lines.0.saturating_neg())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal with four lines of history and a two-row viewport, used by
    /// the sign tests below.
    fn scrollable() -> SharedTerminal {
        let size = buzz_terminal::Size {
            columns: 8,
            screen_lines: 2,
            scrollback: 16,
        };
        let (term, actions) =
            buzz_terminal::Terminal::new(size, buzz_terminal::fences::Fences::ALL);
        // Leaked deliberately: dropping the receiver disconnects the channel
        // and every later listener send fails silently.
        std::mem::forget(actions);
        let shared = SharedTerminal::new(term);
        shared.feed_fully(b"L1\r\nL2\r\nL3\r\nL4");
        shared
    }

    fn screen(terminal: &SharedTerminal) -> Vec<String> {
        let mut encoder = buzz_terminal::damage::Encoder::new();
        terminal
            .snapshot(&mut encoder)
            .rows
            .iter()
            .map(|row| {
                row.spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// The newtype is on the wire, so it has to deserialize from the bare
    /// number the frontend sends. The frontend test mocks `invoke` and cannot
    /// see this; if the wrapper did not read transparently, every scroll would
    /// fail to deserialize and the terminal would silently stop scrolling.
    #[test]
    fn a_wheel_delta_deserializes_from_the_bare_number_the_frontend_sends() {
        #[derive(Deserialize)]
        struct Args {
            lines: DomLines,
        }
        let args: Args = serde_json::from_str(r#"{"lines":-2}"#).unwrap();
        assert_eq!(args.lines.0, -2);

        let terminal = scrollable();
        assert!(scroll_by_dom_lines(&terminal, args.lines));
        assert_eq!(
            screen(&terminal),
            vec!["L1", "L2"],
            "the value off the wire must scroll back, not forwards"
        );
    }

    /// **The DOM->engine crossing, asserted rather than executed in passing.**
    ///
    /// The direction under test: *negative DOM lines go back into history.*
    /// Negative is what a wheel reports for the gesture that scrolls a web
    /// page toward the top of the document, and the engine's `Scroll::Delta`
    /// takes positive for backwards, so the boundary negates.
    ///
    /// This asserts on the text that lands on screen rather than on the
    /// converted number. A test of the arithmetic alone passes just as well
    /// against an engine that reads the sign the other way -- two one-sided
    /// tests are not a test of the crossing.
    #[test]
    fn a_negative_dom_delta_scrolls_backwards_into_history() {
        let terminal = scrollable();
        assert_eq!(
            screen(&terminal),
            vec!["L3", "L4"],
            "starts at the live edge"
        );

        assert!(scroll_by_dom_lines(&terminal, DomLines(-2)));
        assert_eq!(
            screen(&terminal),
            vec!["L1", "L2"],
            "a page-upwards gesture must reach older lines, not newer ones"
        );

        assert!(scroll_by_dom_lines(&terminal, DomLines(2)));
        assert_eq!(
            screen(&terminal),
            vec!["L3", "L4"],
            "and a page-downwards gesture returns to the live edge"
        );
    }

    /// The momentum tail: once history runs out the engine clamps, and the
    /// boundary must report that nothing moved so the command skips the
    /// republish rather than shipping an identical frame per event.
    #[test]
    fn a_dom_delta_past_the_oldest_line_reports_no_movement() {
        let terminal = scrollable();
        assert!(scroll_by_dom_lines(&terminal, DomLines(-2)));
        assert!(
            !scroll_by_dom_lines(&terminal, DomLines(-1_000)),
            "there is nothing older, so nothing moved"
        );
        assert_eq!(screen(&terminal), vec!["L1", "L2"]);
    }

    /// `lines` arrives over IPC, so it can be any `i32`. Plain unary negation
    /// panics on `i32::MIN` in debug and wraps back to `i32::MIN` in release --
    /// the release case being the dangerous one, since it silently reverses
    /// the direction. Saturating keeps the direction the caller asked for and
    /// lets the engine clamp.
    #[test]
    fn the_most_negative_dom_delta_saturates_backwards_instead_of_wrapping_forwards() {
        let terminal = scrollable();
        assert!(scroll_by_dom_lines(&terminal, DomLines(i32::MIN)));
        assert_eq!(
            screen(&terminal),
            vec!["L1", "L2"],
            "a hostile delta must clamp at the oldest line, not flip direction"
        );
    }
}
