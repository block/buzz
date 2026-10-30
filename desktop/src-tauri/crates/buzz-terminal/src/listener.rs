//! The closed set of terminal events we act on.
//!
//! `EventListener` is how the emulator asks the embedder to do something. Most
//! of those requests write back to the PTY, and one of them — `ClipboardLoad` —
//! would let terminal output read the user's clipboard into the shell. We
//! answer a fixed set and **drop everything else by default**, so a new upstream
//! variant is inert until someone deliberately handles it.

use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::vte::ansi::Rgb;

/// Something the embedder must do on the terminal's behalf.
///
/// Two variants carry upstream's reply formatters rather than a finished
/// string: the answers depend on state this listener does not own (the color
/// palette, the cell metrics). Resolving them here would mean inventing
/// values, and a program that asked for its terminal's real background color
/// would silently be told black.
#[derive(Clone)]
pub enum Action {
    /// Write bytes back to the PTY.
    PtyWrite(String),
    /// Reply with palette entry `index`, formatted by `format`.
    ColorReply {
        index: usize,
        format: Arc<dyn Fn(Rgb) -> String + Send + Sync>,
    },
    /// Reply with the text area size, formatted by `format`.
    SizeReply {
        format: Arc<dyn Fn(WindowSize) -> String + Send + Sync>,
    },
    /// The program set the window title (already clamped).
    Title(String),
    /// The program reset the window title.
    ResetTitle,
    /// New content is available; the renderer should sample damage.
    Wakeup,
    /// The program rang the bell.
    Bell,
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PtyWrite(text) => write!(f, "PtyWrite({text:?})"),
            Self::ColorReply { index, .. } => write!(f, "ColorReply({index})"),
            Self::SizeReply { .. } => write!(f, "SizeReply"),
            Self::Title(title) => write!(f, "Title({title:?})"),
            Self::ResetTitle => write!(f, "ResetTitle"),
            Self::Wakeup => write!(f, "Wakeup"),
            Self::Bell => write!(f, "Bell"),
        }
    }
}

/// Longest title we will carry. A title is program-controlled text that ends up
/// in UI chrome; an unbounded one is a memory and layout problem.
pub const TITLE_LIMIT: usize = 512;

/// Clamp on a character boundary, never mid-UTF-8.
fn clamp_title(title: String) -> String {
    match title.char_indices().nth(TITLE_LIMIT) {
        None => title,
        Some((byte_idx, _)) => title[..byte_idx].to_string(),
    }
}

/// Translates upstream events into the closed [`Action`] set.
#[derive(Clone)]
pub struct Listener(Sender<Action>);

impl Listener {
    pub fn new() -> (Self, Receiver<Action>) {
        let (tx, rx) = mpsc::channel();
        (Self(tx), rx)
    }
}

/// Resolve an emulator action that can be answered without renderer state.
///
/// Color queries deliberately return `None`: named/indexed colors resolve
/// against the live theme, which this crate does not own.
pub fn reply(
    action: Action,
    columns: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
) -> Option<String> {
    match action {
        Action::PtyWrite(text) => Some(text),
        Action::SizeReply { format } => Some(format(WindowSize {
            num_lines: rows,
            num_cols: columns,
            cell_width,
            cell_height,
        })),
        // Palette values are renderer-owned. The transport must answer these
        // only after it has a renderer palette, never invent one here.
        Action::ColorReply { .. }
        | Action::Title(_)
        | Action::ResetTitle
        | Action::Wakeup
        | Action::Bell => None,
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let action = match event {
            // Replies the program is waiting on. These are the only routes by
            // which emulator state travels back into the shell.
            Event::PtyWrite(text) => Action::PtyWrite(text),
            // A program blocked on a color reply must get one, or it hangs.
            // The palette lives in `Term`, so the caller resolves the index;
            // the formatter is carried through untouched.
            Event::ColorRequest(index, format) => Action::ColorReply { index, format },
            Event::TextAreaSizeRequest(format) => Action::SizeReply { format },
            Event::Title(title) => Action::Title(clamp_title(title)),
            Event::ResetTitle => Action::ResetTitle,
            Event::Wakeup => Action::Wakeup,
            Event::Bell => Action::Bell,

            // Dropped on purpose, and enumerated so the reason survives:
            //
            // ClipboardLoad would let terminal output paste the user's
            // clipboard into the shell. Never handled.
            Event::ClipboardLoad(..) => return,
            // ClipboardStore is an OSC 52 write; OSC 52 is disabled in the
            // Term config, so this should be unreachable rather than merely
            // unhandled.
            Event::ClipboardStore(..) => return,
            // Presentation concerns the renderer polls for; no action here.
            Event::MouseCursorDirty | Event::CursorBlinkingChange => return,
            // Lifecycle is owned by the PTY layer, not the emulator.
            Event::Exit | Event::ChildExit(_) => return,
        };
        let _ = self.0.send(action);
    }
}
