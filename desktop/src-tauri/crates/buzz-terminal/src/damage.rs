//! Turning grid changes into frames for the renderer.
//!
//! Two rules shape this module, both measured:
//!
//! 1. **Nothing but reading and copying happens under the `Term` lock.** The
//!    caller copies rows out; encoding, hashing and serializing run after the
//!    lock is released. Encoding inline costs ~75x in lock hold.
//! 2. **Damage over-reports.** `Term::damage()` marks the cursor line every
//!    call, so an idle terminal reports damage nearly every frame. Per-line
//!    content hashing suppresses those, so the transport never sees a no-op.
//!
//! # Why a frame is the whole viewport
//!
//! Nearly every frame is a full repaint: `Term::scroll_up_relative` calls
//! `mark_fully_damaged()` unconditionally, so any output reaching the bottom
//! row damages the whole grid. Partial damage is effectively the idle cursor.
//!
//! That is fine, and the reason is worth having here rather than in a review
//! thread. A full frame is O(viewport) *by construction* -- the grid is itself
//! the coalescing buffer -- so its cost does not depend on how fast the child
//! writes. Measured on a 200x50 grid, bytes per frame across four orders of
//! magnitude of output rate: 11,390 at an unthrottled flood (45,759 lines
//! scrolled per frame), 11,390 at ~1 MB/s, 11,390 at ~100 KB/s, 11,305 on a
//! slow build log. Constant to three digits.
//!
//! A scroll-aware diff inverts that: its cost is O(lines scrolled), unbounded,
//! and at 45,759 lines/frame it would ship ~915x more data than the full grid
//! it was optimising. It wins where nobody is watching and loses under `cat`.
//!
//! **Revisit if the viewport grows.** 80x24 costs 2.6 KB/frame (0.2 MB/s at
//! 60 Hz), 200x50 costs 11.4 KB (0.7 MB/s), 400x100 costs 42.8 KB (2.6 MB/s).
//! 400x100 is roughly 4x a typical maximised window and is where this decision
//! should be re-measured -- as a serialization/IPC question, not a damage one.
//!
//! Dedup earns its place in the interactive case rather than the streaming one:
//! typing is ~0.9 rows per keystroke, and an idle terminal ships 0 rows across
//! 60 frames instead of a cursor-line frame 60x/second. Idle is the load-bearing
//! one -- it is what the substrate does while sitting behind the GUI untouched.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::TermDamage;

/// A run of cells sharing one visual style **and one cell width**.
///
/// # Why the consumer can position every cluster without Unicode tables
///
/// The renderer must place each display cluster at its true column, and it
/// cannot derive that from the text: no single split rule over a concatenated
/// string is correct. A regional-indicator flag (`U+1F1FA U+1F1F8`) is two
/// ordinary one-column cells, so it must split *per codepoint*; a keycap
/// (`1 U+FE0F U+20E3`) is one cell holding three codepoints, so it must split
/// *per grapheme*. Those rules disagree, and the distinction lives in the grid,
/// not in the string.
///
/// So the run carries it instead. Within a span every cluster advances the same
/// [`width`](Self::width) columns, and [`cluster_count`](Self::cluster_count)
/// says how many clusters the text holds. The consumer's rule is arithmetic on
/// those two numbers, with no Unicode table anywhere:
///
/// ```text
/// cluster_count == 1  -> the whole text is one cluster, at `column`
/// otherwise           -> cluster i is the i-th char, at `column + i * width`
/// ```
///
/// The second case is exact because a cell carrying zerowidth marks is always
/// emitted alone, so every cell in a multi-cluster span contributes exactly one
/// `char`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// First column of the run.
    pub column: usize,
    /// The run's text. Grapheme clusters are kept whole: a cell's zerowidth
    /// combining marks follow its base character, so the renderer never sees
    /// a base and its accent as separate glyphs.
    pub text: String,
    /// Columns each cluster in this run occupies: 1, or 2 for wide glyphs.
    ///
    /// Uniform across the run by construction -- a width change ends the span.
    /// This is what lets the consumer position clusters by computed origin
    /// rather than by accumulated text advance.
    pub width: u8,
    /// How many display clusters [`text`](Self::text) holds.
    ///
    /// Without this the consumer cannot distinguish a one-cluster span carrying
    /// combining marks from an ordinary multi-character run, and would need a
    /// Unicode zerowidth table to guess. The grid already knows, so it says.
    pub cluster_count: u16,
    /// Packed style: fg, bg, and attribute flags.
    pub style: Style,
}

impl Span {
    /// The decoding invariant, stated once: a span is either a single cluster
    /// (which may hold several `char`s, as a keycap or an accented letter
    /// does) or one cluster per `char`.
    ///
    /// Exposed so consumers can assert it at a trust boundary rather than
    /// restate it. The encoder checks it in debug builds on every frame.
    pub fn counts_are_consistent(&self) -> bool {
        self.cluster_count == 1 || usize::from(self.cluster_count) == self.text.chars().count()
    }
}

/// Visual style of a span, as the renderer needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: u32,
    pub bg: u32,
    pub flags: u16,
}

/// One changed row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowFrame {
    pub line: usize,
    /// Whether this row continues onto the next screen row without a hard
    /// line break. Retained separately from visual style so copy serialization
    /// can reconstruct logical lines without exposing geometry flags to spans.
    pub wrapped: bool,
    pub spans: Vec<Span>,
}

/// The cursor, carried separately from row content.
///
/// Upstream damages the cursor's line on every `damage()` call. If the cursor
/// travelled inside the row payload, every frame would carry a row rewrite for
/// a caret that moved one column. As its own plane it costs a few bytes and
/// leaves row dedup free to suppress the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorFrame {
    pub line: usize,
    pub column: usize,
    pub visible: bool,
}

/// One update for the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub rows: Vec<RowFrame>,
    pub cursor: CursorFrame,
    /// Whether the cursor plane changed since this encoder's previous frame.
    /// Cursor movement can be the only visible effect of input (for example,
    /// echoing a space over an already blank cell), so it independently makes
    /// an incremental frame publishable.
    pub cursor_changed: bool,
    /// Whether the renderer should discard what it has and repaint.
    pub full: bool,
    /// The grid this frame describes. A change means the terminal was resized
    /// and row indices refer to a different geometry than the previous frame's.
    /// Carried so the consumer can detect that from the frame itself instead of
    /// trusting that no resize overtook it in flight -- across a transport, a
    /// frame captured before a resize can arrive after it.
    pub viewport: crate::Viewport,
}

impl Frame {
    /// True when there is nothing for the renderer to do.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && !self.cursor_changed && !self.full
    }
}

/// Raw rows copied out from under the lock, awaiting encode.
pub struct RawFrame {
    rows: Vec<(usize, Vec<Cell>)>,
    cursor: CursorFrame,
    full: bool,
    viewport: crate::Viewport,
}

/// The grid line a screen row reads from.
///
/// The grid indexes the active area from 0 and scrollback with *negative*
/// lines, so scrolling back `n` lines means every screen row reads `n` lines
/// higher. At the live edge the offset is zero and this is the identity, which
/// is why the unscrolled path is unchanged rather than merely equivalent.
fn row_of(screen_row: usize, display_offset: usize) -> Line {
    Line(screen_row as i32 - display_offset as i32)
}

/// Where the cursor sits on screen, given how far the viewport is scrolled
/// back.
///
/// The grid keeps the cursor in *active-area* coordinates, which do not move
/// when the user scrolls; the renderer paints *screen rows*, which do. The two
/// agree only at the live edge, so the conversion has to happen somewhere, and
/// it happens here rather than in the renderer -- the renderer is not told the
/// display offset, and giving it one would put this same arithmetic on the far
/// side of a transport.
///
/// Scrolling far enough pushes the cursor off the bottom of the viewport, and
/// then it is reported as not visible. Without that clamp a caret drawn at a
/// clamped row would sit on some unrelated line of history, which reads as
/// corruption rather than as scrollback.
fn cursor_frame(
    cursor_point: alacritty_terminal::index::Point,
    display_offset: usize,
    screen_lines: usize,
    shown: bool,
) -> CursorFrame {
    let line = cursor_point.line.0.max(0) as usize + display_offset;
    CursorFrame {
        line: line.min(screen_lines.saturating_sub(1)),
        column: cursor_point.column.0,
        visible: shown && line < screen_lines,
    }
}

/// Copy the damaged rows out of the terminal. **Runs under the lock; does no
/// encoding.** Keep this function boring — everything added here is lock hold.
pub fn capture(terminal: &mut crate::Terminal) -> RawFrame {
    let viewport = terminal.viewport();
    let display_offset = terminal.display_offset();
    let term = terminal.term_mut();
    let columns = term.columns();
    let screen_lines = term.screen_lines();
    let cursor_point = term.grid().cursor.point;
    let shown = term
        .mode()
        .contains(alacritty_terminal::term::TermMode::SHOW_CURSOR);

    // Upstream's partial iterator already reports **screen** rows: it offsets
    // each damaged active-area line by the display offset and drops the ones
    // that scrolling pushed off the bottom (`TermDamageIterator::new`). So both
    // arms below speak the same coordinate, and `row_of` converts once.
    let (lines, full) = match term.damage() {
        TermDamage::Full => ((0..screen_lines).collect::<Vec<_>>(), true),
        TermDamage::Partial(iter) => (
            iter.map(|bounds| bounds.line)
                .filter(|l| *l < screen_lines)
                .collect(),
            false,
        ),
    };

    let grid = term.grid();
    let mut rows = Vec::with_capacity(lines.len());
    for line in lines {
        let row = &grid[row_of(line, display_offset)];
        rows.push((line, row[..Column(columns)].to_vec()));
    }
    let cursor = cursor_frame(cursor_point, display_offset, screen_lines, shown);

    term.reset_damage();
    RawFrame {
        rows,
        cursor,
        full,
        viewport,
    }
}

/// Copy the **entire visible viewport**, leaving damage untouched.
///
/// This exists for subscribers that arrive mid-stream: attach, reattach, and
/// the successor side of a resize. Damage only describes what changed since
/// the last capture, so a newcomer that starts from [`capture`] sees whatever
/// happened to change next -- often just the cursor's line -- painted onto a
/// blank screen. Upstream's `mark_fully_damaged` is private, so an embedder
/// cannot ask for a full frame that way.
///
/// **It must not consume damage, and that is the load-bearing property.** The
/// incumbent subscriber's next [`capture`] has to still see its rows. If this
/// called `damage()`/`reset_damage()` it would steal them, and the incumbent
/// would freeze on stale content while a newcomer's full-frame test passed.
/// The absence of those two calls below is the mechanism; `snapshot_test.rs`
/// is the proof.
///
/// **Runs under the lock; does no encoding.** Costs a full grid copy rather
/// than a damaged-rows copy, so it belongs on attach, not in the frame loop.
pub fn capture_all(terminal: &mut crate::Terminal) -> RawFrame {
    let viewport = terminal.viewport();
    let display_offset = terminal.display_offset();
    let term = terminal.term_mut();
    let columns = term.columns();
    let screen_lines = term.screen_lines();
    let cursor_point = term.grid().cursor.point;
    let shown = term
        .mode()
        .contains(alacritty_terminal::term::TermMode::SHOW_CURSOR);

    let grid = term.grid();
    let mut rows = Vec::with_capacity(screen_lines);
    for line in 0..screen_lines {
        let row = &grid[row_of(line, display_offset)];
        rows.push((line, row[..Column(columns)].to_vec()));
    }
    let cursor = cursor_frame(cursor_point, display_offset, screen_lines, shown);

    // No `damage()` and no `reset_damage()`: see the note above.
    RawFrame {
        rows,
        cursor,
        // A snapshot *is* a repaint, and marking it full also resets the
        // consumer's `Encoder` hashes, so its dedup state describes the grid it
        // was actually given rather than a predecessor's.
        full: true,
        viewport,
    }
}

/// Suppresses rows whose content did not actually change.
#[derive(Default)]
pub struct Encoder {
    hashes: Vec<u64>,
    cursor: Option<CursorFrame>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode a captured frame. **Runs with the lock released.**
    pub fn encode(&mut self, raw: RawFrame) -> Frame {
        // A full frame invalidates the dedup cache. Both routes that produce
        // one matter: a `mark_fully_damaged` from scroll/alt-swap, and a resize,
        // where the cached hashes describe rows of a different width entirely.
        if raw.full {
            self.hashes.clear();
        }
        let mut rows = Vec::with_capacity(raw.rows.len());
        for (line, cells) in raw.rows {
            let hash = hash_cells(&cells);
            if self.hashes.len() <= line {
                self.hashes.resize(line + 1, 0);
            }
            if self.hashes[line] == hash {
                continue;
            }
            self.hashes[line] = hash;
            rows.push(RowFrame {
                line,
                wrapped: cells
                    .last()
                    .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE)),
                spans: spans(&cells),
            });
        }
        let cursor_changed = self.cursor != Some(raw.cursor);
        self.cursor = Some(raw.cursor);
        Frame {
            rows,
            cursor: raw.cursor,
            cursor_changed,
            full: raw.full,
            viewport: raw.viewport,
        }
    }
}

fn hash_cells(cells: &[Cell]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for cell in cells {
        cell.c.hash(&mut hasher);
        // Hash the *packed* colors, not the enum: this is the representation
        // the renderer receives, so the dedup key cannot disagree with the
        // wire encoding and suppress a row that actually changed on screen.
        pack_color(cell.fg).hash(&mut hasher);
        pack_color(cell.bg).hash(&mut hasher);
        cell.flags.bits().hash(&mut hasher);
        if let Some(zerowidth) = cell.zerowidth() {
            zerowidth.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Group a row's cells into runs of uniform style and width.
///
/// A run continues only while style *and* width match, and a cell carrying
/// zerowidth marks is always emitted alone. Both breaks exist so the consumer
/// can compute each cluster's column as `column + i * width`; see [`Span`].
///
/// The width comparison is the only thing keeping widths uniform within a run:
/// [`Style`] deliberately excludes [`GEOMETRY_FLAGS`], so a style key cannot
/// break a run on width behind this check's back.
fn spans(cells: &[Cell]) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    // Whether the run in progress may still be extended. Kept here rather than
    // on `Span` because it is grouping bookkeeping, not part of the wire shape.
    let mut open = false;
    for (column, cell) in cells.iter().enumerate() {
        // A wide glyph occupies two cells: the character, then a spacer. The
        // spacer carries no text of its own -- emitting its placeholder space
        // would insert a phantom column after every CJK character or emoji.
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let style = style_of(cell);
        let width = if cell.flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
        let zerowidth = cell.zerowidth();
        let mut text = String::new();
        text.push(cell.c);
        if let Some(marks) = zerowidth {
            text.extend(marks);
        }

        // A cluster with combining marks holds more `char`s than columns, so it
        // cannot share a run: it is the one case where "one char per cluster"
        // stops holding.
        let joinable = zerowidth.is_none();
        match spans.last_mut() {
            // `cluster_count` is refused rather than wrapped when it would
            // overflow: the run simply ends and a new span starts at this
            // column, which the consumer's rule already handles.
            Some(last)
                if open
                    && joinable
                    && last.style == style
                    && last.width == width
                    && last.cluster_count < u16::MAX =>
            {
                last.text.push_str(&text);
                last.cluster_count += 1;
            }
            _ => spans.push(Span {
                column,
                text,
                width,
                cluster_count: 1,
                style,
            }),
        }
        open = joinable;
    }
    // Enforced in release, not just in debug. This is a *wire* invariant: a
    // span that violates it is undecodable by the rule in [`Span`], and the
    // consumer's failure is silent misplacement of every cluster after it.
    // A `debug_assert` here would vanish in exactly the build where that
    // corruption ships. The cost is one pass over text already in cache --
    // the same order as building the spans -- and it buys a loud, local
    // failure instead of a renderer quietly drawing the wrong columns.
    assert!(
        spans.iter().all(Span::counts_are_consistent),
        "cluster_count must be 1 or the span's char count"
    );
    spans
}

/// Flags describing where a cell sits in the grid rather than how it looks.
///
/// `WRAPLINE` marks the last cell of a row that wrapped; the three wide-char
/// bits mark a two-column glyph and its spacer. Neither says anything about
/// appearance.
///
/// These are excluded from [`Style`] so the style key means one thing: visual
/// attributes. Geometry travels in [`Span::width`], which is compared on its
/// own when grouping -- if these bits stayed in the key they would break runs
/// as a side effect and leave the width comparison untestable.
///
/// Composite visual aliases (`BOLD_ITALIC`, `DIM_BOLD`, `ALL_UNDERLINES`) are
/// deliberately not masked: those are appearance.
const GEOMETRY_FLAGS: Flags = Flags::WRAPLINE
    .union(Flags::WIDE_CHAR)
    .union(Flags::WIDE_CHAR_SPACER)
    .union(Flags::LEADING_WIDE_CHAR_SPACER);

fn style_of(cell: &Cell) -> Style {
    Style {
        fg: pack_color(cell.fg),
        bg: pack_color(cell.bg),
        flags: cell.flags.difference(GEOMETRY_FLAGS).bits(),
    }
}

/// Pack a color into a tagged u32 the renderer resolves against the theme.
///
/// Named and indexed colors stay symbolic rather than being resolved here:
/// the substrate must follow the user's chosen theme, so the palette belongs
/// to the renderer, not to a snapshot taken at damage time.
fn pack_color(color: alacritty_terminal::vte::ansi::Color) -> u32 {
    use alacritty_terminal::vte::ansi::Color;
    match color {
        Color::Named(named) => 0x0100_0000 | named as u32,
        Color::Indexed(index) => 0x0200_0000 | index as u32,
        Color::Spec(rgb) => {
            0x0300_0000 | ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32
        }
    }
}
