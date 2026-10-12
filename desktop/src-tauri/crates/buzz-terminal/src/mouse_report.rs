//! Encode pointer / touch / wheel into PTY mouse reports when mouse mode is on.

use crate::Terminal;

/// Button codes for SGR / X10 mouse protocols (wheel uses 64/65).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left = 0,
    Middle = 1,
    Right = 2,
    WheelUp = 64,
    WheelDown = 65,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Move,
}

/// Encode one mouse event. Returns `None` when mouse tracking is off.
pub fn encode_mouse_event(
    terminal: &Terminal,
    button: MouseButton,
    action: MouseAction,
    column: usize,
    row: usize,
    mods: u8,
) -> Option<Vec<u8>> {
    if !terminal.mouse_tracking() {
        return None;
    }
    let col = column.max(1);
    let row = row.max(1);
    let mut cb = button as u8;
    cb |= mods & 0b1_1100;
    if matches!(action, MouseAction::Move) {
        cb |= 32;
    }

    let sgr = terminal.sgr_mouse();
    let mut out = Vec::with_capacity(24);
    if sgr {
        out.extend_from_slice(b"\x1b[<");
        out.extend_from_slice(cb.to_string().as_bytes());
        out.push(b';');
        out.extend_from_slice(col.to_string().as_bytes());
        out.push(b';');
        out.extend_from_slice(row.to_string().as_bytes());
        out.push(if matches!(action, MouseAction::Release) {
            b'm'
        } else {
            b'M'
        });
    } else {
        if matches!(action, MouseAction::Release) {
            return Some(Vec::new());
        }
        let encoded_btn = 32u8.saturating_add(cb);
        let cx = 32u8.saturating_add(col.min(223) as u8);
        let cy = 32u8.saturating_add(row.min(223) as u8);
        out.extend_from_slice(&[0x1b, b'[', b'M', encoded_btn, cx, cy]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fences, Size};

    fn term_with(feed: &[u8]) -> Terminal {
        let (mut term, actions) = Terminal::new(
            Size {
                columns: 40,
                screen_lines: 12,
                scrollback: 8,
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
    fn no_report_without_mouse_mode() {
        let term = term_with(b"");
        assert!(
            encode_mouse_event(&term, MouseButton::Left, MouseAction::Press, 2, 3, 0).is_none()
        );
    }

    #[test]
    fn sgr_press_and_release() {
        let term = term_with(b"\x1b[?1000h\x1b[?1006h");
        let press =
            encode_mouse_event(&term, MouseButton::Left, MouseAction::Press, 4, 5, 0).unwrap();
        assert_eq!(String::from_utf8_lossy(&press), "\u{1b}[<0;4;5M");
        let release =
            encode_mouse_event(&term, MouseButton::Left, MouseAction::Release, 4, 5, 0).unwrap();
        assert_eq!(String::from_utf8_lossy(&release), "\u{1b}[<0;4;5m");
    }
}
