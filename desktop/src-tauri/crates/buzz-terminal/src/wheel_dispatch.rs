//! Decide what a DOM wheel delta does, and encode PTY-bound mouse / alt-scroll.
//!
//! Three product behaviors, matching common terminal emulators:
//! 1. **Mouse tracking** (`MOUSE_MODE`): forward wheel as mouse button 64/65.
//! 2. **Alternate scroll** (alt screen + `ALTERNATE_SCROLL`, no mouse mode):
//!    forward as cursor up/down so TUIs like `less` / `vim` scroll.
//! 3. **Otherwise**: move the emulator scrollback (handled by the caller).

use crate::Terminal;

use crate::scroll_sign::DomLines;

/// What `terminal_scroll` should do with a DOM wheel delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WheelAction {
    /// Negate and scroll the emulator viewport (existing scrollback path).
    Scrollback(DomLines),
    /// Bytes to write to the PTY (mouse reports or cursor keys).
    Pty(Vec<u8>),
}

/// Plan the action for `dom_lines` whole cells against the terminal's modes.
///
/// `dom_lines` uses the DOM sign: negative is the gesture that scrolls a page
/// toward the top (wheel "up"). Zero is a no-op that yields empty Pty bytes
/// rather than a scrollback call, so the command can skip work.
pub fn plan(terminal: &Terminal, dom_lines: DomLines) -> WheelAction {
    let lines = dom_lines.0;
    if lines == 0 {
        return WheelAction::Pty(Vec::new());
    }

    if terminal.mouse_tracking() {
        return WheelAction::Pty(encode_mouse_wheel(
            lines,
            terminal.sgr_mouse(),
            report_cell(terminal),
        ));
    }

    if terminal.alt_screen() && terminal.alternate_scroll() {
        return WheelAction::Pty(encode_alternate_scroll(lines, terminal.app_cursor()));
    }

    WheelAction::Scrollback(dom_lines)
}

/// 1-based cell under which to report the wheel. Uses the cursor when it sits
/// on screen; otherwise the top-left cell. Apps that care about coordinates
/// usually track motion themselves; a stable fallback is enough for scroll.
fn report_cell(terminal: &Terminal) -> (usize, usize) {
    let display_offset = terminal.display_offset();
    let screen_lines = terminal.size().screen_lines;
    let point = terminal.term().grid().cursor.point;
    let line = point.line.0.max(0) as usize + display_offset;
    if line < screen_lines {
        (point.column.0 + 1, line + 1)
    } else {
        (1, 1)
    }
}

/// Encode one mouse-wheel report per cell of `dom_lines`.
///
/// Negative DOM → button 64 (wheel up); positive → 65 (wheel down). Cap the
/// burst so a large trackpad flick cannot flood the PTY.
fn encode_mouse_wheel(dom_lines: i32, sgr: bool, (col, row): (usize, usize)) -> Vec<u8> {
    const MAX_REPORTS: i32 = 32;
    let count = dom_lines.saturating_abs().min(MAX_REPORTS);
    let button: u8 = if dom_lines < 0 { 64 } else { 65 };
    let mut out = Vec::with_capacity((count as usize) * 16);
    for _ in 0..count {
        if sgr {
            // CSI < Cb ; Cx ; Cy M
            out.extend_from_slice(b"\x1b[<");
            out.extend_from_slice(button.to_string().as_bytes());
            out.push(b';');
            out.extend_from_slice(col.to_string().as_bytes());
            out.push(b';');
            out.extend_from_slice(row.to_string().as_bytes());
            out.push(b'M');
        } else {
            // X10: CSI M Cb Cx Cy with +32 offset; clamp to the protocol range.
            let cb = 32u8.saturating_add(button);
            let cx = 32u8.saturating_add(col.min(223) as u8);
            let cy = 32u8.saturating_add(row.min(223) as u8);
            out.extend_from_slice(&[0x1b, b'[', b'M', cb, cx, cy]);
        }
    }
    out
}

fn encode_alternate_scroll(dom_lines: i32, app_cursor: bool) -> Vec<u8> {
    const MAX_KEYS: i32 = 32;
    let count = dom_lines.saturating_abs().min(MAX_KEYS);
    // Negative DOM (page-upwards) → cursor up.
    let key: &[u8] = if dom_lines < 0 {
        if app_cursor {
            b"\x1bOA"
        } else {
            b"\x1b[A"
        }
    } else if app_cursor {
        b"\x1bOB"
    } else {
        b"\x1b[B"
    };
    let mut out = Vec::with_capacity((count as usize) * key.len());
    for _ in 0..count {
        out.extend_from_slice(key);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fences, Size};

    fn term_with(feed: &[u8]) -> Terminal {
        let (mut term, actions) = Terminal::new(
            Size {
                columns: 20,
                screen_lines: 6,
                scrollback: 16,
            },
            Fences::ALL,
        );
        std::mem::forget(actions);
        if !feed.is_empty() {
            term.feed_fully(feed);
        }
        term
    }

    #[test]
    fn plain_shell_scrolls_scrollback() {
        let term = term_with(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh");
        assert_eq!(
            plan(&term, DomLines(-2)),
            WheelAction::Scrollback(DomLines(-2))
        );
    }

    #[test]
    fn mouse_mode_sgr_encodes_wheel_up_and_down() {
        // DECSET 1000 (click) + 1006 (SGR).
        let term = term_with(b"\x1b[?1000h\x1b[?1006h");
        let up = plan(&term, DomLines(-2));
        match up {
            WheelAction::Pty(bytes) => {
                assert_eq!(
                    String::from_utf8_lossy(&bytes),
                    "\u{1b}[<64;1;1M\u{1b}[<64;1;1M"
                );
            }
            other => panic!("expected Pty, got {other:?}"),
        }
        let down = plan(&term, DomLines(1));
        match down {
            WheelAction::Pty(bytes) => {
                assert_eq!(String::from_utf8_lossy(&bytes), "\u{1b}[<65;1;1M");
            }
            other => panic!("expected Pty, got {other:?}"),
        }
    }

    #[test]
    fn alternate_scroll_sends_cursor_keys() {
        // Enter alt screen (1049) — ALTERNATE_SCROLL is on by default.
        let term = term_with(b"\x1b[?1049h");
        match plan(&term, DomLines(-3)) {
            WheelAction::Pty(bytes) => {
                assert_eq!(String::from_utf8_lossy(&bytes), "\u{1b}[A\u{1b}[A\u{1b}[A");
            }
            other => panic!("expected Pty, got {other:?}"),
        }
        // Application cursor keys (1).
        let term = term_with(b"\x1b[?1049h\x1b[?1h");
        match plan(&term, DomLines(2)) {
            WheelAction::Pty(bytes) => {
                assert_eq!(String::from_utf8_lossy(&bytes), "\u{1b}OB\u{1b}OB");
            }
            other => panic!("expected Pty, got {other:?}"),
        }
    }

    #[test]
    fn mouse_mode_wins_over_alternate_scroll() {
        let term = term_with(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h");
        match plan(&term, DomLines(-1)) {
            WheelAction::Pty(bytes) => {
                assert!(
                    String::from_utf8_lossy(&bytes).contains("<64;"),
                    "mouse reports must win over cursor keys"
                );
            }
            other => panic!("expected Pty, got {other:?}"),
        }
    }

    #[test]
    fn zero_delta_is_inert() {
        let term = term_with(b"");
        assert_eq!(plan(&term, DomLines(0)), WheelAction::Pty(Vec::new()));
    }
}
