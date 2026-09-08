//! Mutation-sensitive byte fixtures for the two parser fences.
//!
//! These use the shipping `Terminal::feed` path. Arms that could be masked by
//! the other fence disable it explicitly; the switches are runtime values, not
//! cargo features, so the default test binary always contains every arm.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use buzz_terminal::fences::{Fences, OSC_BUDGET, SYNC_CAP};
use buzz_terminal::{Size, Terminal};

const CHUNK: usize = 8192;
const G1_BYTES: usize = 2 << 20;
const G2_FRAMES: usize = 40;
const G2_FRAME_BYTES: usize = 1_900 * 1024;

fn size() -> Size {
    Size {
        columns: 120,
        screen_lines: 40,
        scrollback: 2000,
    }
}

fn feed_synchronized(term: &mut Terminal, payload: &[u8], close: bool) {
    term.feed_fully(b"\x1b[?2026h");
    for chunk in payload.chunks(CHUNK) {
        term.feed_fully(chunk);
    }
    if close {
        term.feed_fully(b"\x1b[?2026l");
    }
}

fn repeated(pattern: &[u8], bytes: usize) -> Vec<u8> {
    pattern.iter().copied().cycle().take(bytes).collect()
}

fn count_markers(term: &Terminal, markers: usize) -> usize {
    let grid = term.term().grid();
    let mut text = String::new();
    let top = -(grid.history_size() as i32);
    for line in top..term.size().screen_lines as i32 {
        for column in 0..term.size().columns {
            text.push(grid[Point::new(Line(line), Column(column))].c);
        }
        text.push('\n');
    }
    (0..markers)
        .filter(|m| text.contains(&format!("MK{m:03}")))
        .count()
}

fn legitimate_frame(markers: usize, bytes: usize) -> Vec<u8> {
    let mut payload = Vec::with_capacity(bytes);
    for marker in 0..markers {
        payload.extend_from_slice(format!("MK{marker:03}\r\n").as_bytes());
        let target = bytes * (marker + 1) / markers;
        while payload.len() < target {
            payload.extend_from_slice(b"\x1b[1;32mx\x1b[0m");
        }
        payload.extend_from_slice(b"\r\n");
    }
    payload.truncate(bytes);
    payload
}

/// G1: every hostile content shape must remain below the deterministic byte
/// bound, and the same shape with F1 deleted must cross it. Keeping both arms
/// adjacent prevents a simplified fixture from becoming vacuously cheap.
#[test]
fn g1_sync_abort_bounds_all_hostile_shapes() {
    let shapes: [(&str, &[u8]); 5] = [
        ("sgr", b"\x1b[1;32mbuzz\x1b[0m\r\n"),
        ("ascii", b"buzz substrate output\r\n"),
        ("emoji", "🐝🚀✨\r\n".as_bytes()),
        ("zalgo", "z\u{0301}\u{0302}\u{0303}\u{0304}\r\n".as_bytes()),
        ("truecolor", b"\x1b[38;2;255;0;128mRGB\x1b[0m\r\n"),
    ];

    for (name, pattern) in shapes {
        let payload = repeated(pattern, G1_BYTES);
        let (mut fenced, _) = Terminal::new(size(), Fences::ALL);
        feed_synchronized(&mut fenced, &payload, false);
        let fenced_stats = fenced.stats();
        assert!(fenced_stats.sync_aborts > 0, "{name}: F1 never fired");
        assert!(
            fenced_stats.max_release <= 2 * SYNC_CAP,
            "{name}: fenced release {} exceeds 128 KiB",
            fenced_stats.max_release
        );

        let (mut unfenced, _) = Terminal::new(size(), Fences::NONE);
        feed_synchronized(&mut unfenced, &payload, false);
        let unfenced_stats = unfenced.stats();
        assert_eq!(unfenced_stats.sync_aborts, 0, "{name}: control enabled F1");
        assert!(
            unfenced_stats.max_release > 2 * SYNC_CAP,
            "{name}: unfenced release {} stayed inside the gate; fixture is vacuous",
            unfenced_stats.max_release
        );
    }
}

/// G2 arm 1: deletion oracle. F1 remains enabled because this arm proves F2
/// deletion under the combined production configuration.
#[test]
fn g2_hostile_unsynchronized_osc_resets_parser() {
    let (mut term, _) = Terminal::new(size(), Fences::ALL);
    term.feed_fully(b"\x1b]0;");
    for chunk in repeated(b"A", OSC_BUDGET * 4).chunks(CHUNK) {
        term.feed_fully(chunk);
    }
    assert!(term.stats().osc_resets > 0, "F2 never rebuilt the parser");
}

/// G2 arm 2: every synchronized release is attributed. F1 is disabled so its
/// small abort releases cannot mask an implementation that omits ESU flushes.
#[test]
fn g2_each_synchronized_flush_is_attributed() {
    let payload = repeated(b"A", G2_FRAME_BYTES);
    let (mut term, _) = Terminal::new(size(), Fences::OSC_ONLY);
    for _ in 0..G2_FRAMES {
        feed_synchronized(&mut term, &payload, true);
    }
    let stats = term.stats();
    assert_eq!(stats.sync_aborts, 0, "F1 must be disabled in this arm");
    assert_eq!(
        stats.osc_resets, G2_FRAMES as u64,
        "expected one reset for each atomic synchronized release"
    );
    assert!(
        stats.charged_bytes >= (G2_FRAMES * G2_FRAME_BYTES) as u64,
        "flush bytes were omitted from attribution: {} charged",
        stats.charged_bytes
    );
}

/// G2 arm 3: parser-visible attribution preserves a legitimate 1.5 MiB frame.
/// F1 is disabled; raw-input counting would reset mid-frame and lose markers.
#[test]
fn g2_legitimate_large_frame_preserves_all_markers() {
    let markers = 200;
    let payload = legitimate_frame(markers, 1_500 * 1024);
    let (mut term, _) = Terminal::new(size(), Fences::OSC_ONLY);
    feed_synchronized(&mut term, &payload, true);
    assert_eq!(
        count_markers(&term, markers),
        markers,
        "legitimate frame lost markers"
    );
}

/// Legitimacy control: neither fence alone nor the production combination may
/// corrupt a normal synchronized frame.
#[test]
fn g2_legitimate_frame_survives_each_fence_configuration() {
    let markers = 200;
    let payload = legitimate_frame(markers, 128 * 1024);
    for fences in [Fences::SYNC_ONLY, Fences::OSC_ONLY, Fences::ALL] {
        let (mut term, _) = Terminal::new(size(), fences);
        feed_synchronized(&mut term, &payload, true);
        assert_eq!(
            count_markers(&term, markers),
            markers,
            "{fences:?} lost markers"
        );
    }
}
