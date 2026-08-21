use super::*;

/// Feed `chunks` through a capture in order.
fn capture_of(chunks: &[&[u8]]) -> Capture {
    let capture = Capture::new();
    for chunk in chunks {
        capture.push(chunk);
    }
    capture
}

/// Render what the UI would show for a stream of `chunks`.
fn ui(chunks: &[&[u8]]) -> String {
    capture_of(chunks).ui()
}

// ── bounded capture ──────────────────────────────────────────────────────────

/// Output within the cap is passed through byte-for-byte — no marker, no loss.
#[test]
fn test_capture_leaves_short_output_untouched() {
    let short = "a".repeat(1536);

    assert_eq!(ui(&[short.as_bytes()]), short);
}

/// Over the cap, both ends survive and the middle is replaced by a marker
/// naming the omitted byte count — the head keeps the command's opening
/// context and the tail keeps the error that usually trails.
#[test]
fn test_capture_over_cap_keeps_head_and_tail_with_marker() {
    let input = format!(
        "{}{}{}",
        "H".repeat(512),
        "M".repeat(4000),
        "T".repeat(1024)
    );

    let out = ui(&[input.as_bytes()]);

    assert!(out.starts_with(&"H".repeat(512)));
    assert!(out.ends_with(&"T".repeat(1024)));
    assert!(
        out.contains("... (4000 bytes omitted) ..."),
        "marker must name the omitted byte count, got: {out}"
    );
}

/// The rendered result depends only on the byte stream, not on how the reads
/// happened to split it — a real drain sees arbitrary chunk sizes.
#[test]
fn test_capture_is_independent_of_chunk_boundaries() {
    let input = "x".repeat(9000);
    let one_shot = ui(&[input.as_bytes()]);

    let chunked: Vec<&[u8]> = input.as_bytes().chunks(7).collect();

    assert_eq!(ui(&chunked), one_shot);
}

/// Truncation must not split a multi-byte character. Both cut points land
/// mid-codepoint here; the partial bytes are dropped rather than decoded into
/// replacement chars.
#[test]
fn test_capture_does_not_split_multibyte_characters() {
    // "é" is 2 bytes, so every candidate cut index lands mid-character.
    let input = "é".repeat(4000);

    let out = ui(&[input.as_bytes()]);

    assert!(out.contains("bytes omitted"), "input must exceed the cap");
    assert!(!out.contains('\u{fffd}'), "no replacement chars: {out}");
}

/// Memory stays flat regardless of how much the installer prints: the rendered
/// UI result of a 4MiB stream is no larger than that of a 6KiB one.
#[test]
fn test_capture_of_huge_output_stays_bounded() {
    let chunk = vec![b'z'; 8192];

    let capture = Capture::new();
    for _ in 0..512 {
        capture.push(&chunk);
    }

    let out = capture.ui();
    assert!(
        out.len() < 2048,
        "4MiB of output must render bounded, got {} bytes",
        out.len()
    );
    assert!(out.contains("bytes omitted"));
}

// ── the log view is separately bounded ───────────────────────────────────────

/// The log view holds output the UI view had to cut. A toast is capped for
/// readability; the log file's budget is disk, and "Full log: {path}" has to
/// point at more than the toast already showed.
#[test]
fn test_log_view_keeps_output_the_ui_view_truncates() {
    let input = format!("start{}end", "m".repeat(64 * 1024));

    let capture = capture_of(&[input.as_bytes()]);

    assert!(
        capture.ui().contains("bytes omitted"),
        "64KiB must exceed the UI cap"
    );
    assert_eq!(
        capture.log(),
        input,
        "the same output must be complete in the log view"
    );
}

/// Even the log view is bounded — a runaway installer cannot fill the disk —
/// and when it does cut, the record says so inline at the cap rather than
/// implying completeness.
#[test]
fn test_log_view_is_bounded_and_marks_its_cap() {
    let head = "H".repeat(128 * 1024);
    let middle = "M".repeat(5000);
    let tail = "T".repeat(128 * 1024);
    let input = format!("{head}{middle}{tail}");

    let capture = capture_of(&[input.as_bytes()]);

    let out = capture.log();
    assert!(
        out.len() < 300 * 1024,
        "output past the log cap must render bounded, got {} bytes",
        out.len()
    );
    assert!(out.starts_with(&head), "the log head must survive intact");
    assert!(out.ends_with(&tail), "the log tail must survive intact");
    assert!(
        out.contains("... [5000 bytes omitted at cap] ..."),
        "a cut log record must name the cap inline, got the middle: {}",
        &out[128 * 1024..(128 * 1024 + 64).min(out.len())]
    );
}

/// The two views mark their cuts differently on purpose: the toast reads as
/// prose, the log record reads as a machine-scannable annotation.
#[test]
fn test_ui_and_log_views_use_their_own_cap_markers() {
    let input = "x".repeat(300 * 1024);

    let capture = capture_of(&[input.as_bytes()]);

    assert!(
        capture.ui().contains("bytes omitted) ..."),
        "the UI marker reads as prose: {}",
        capture.ui()
    );
    assert!(capture.log().contains("bytes omitted at cap] ..."));
}

// ── line observation ─────────────────────────────────────────────────────────

/// Collect the lines a drain over `chunks` reports.
fn observed_lines(chunks: &[&[u8]]) -> Vec<String> {
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let observer: LineObserver = {
        let seen = Arc::clone(&seen);
        Arc::new(move |line: &str| seen.lock().unwrap().push(line.to_string()))
    };
    let bytes: Vec<u8> = chunks.concat();

    drain_into(bytes.as_slice(), &Capture::new(), Some(&observer));

    let observed = seen.lock().unwrap().clone();
    observed
}

/// The observer sees complete lines, reassembled across the read boundaries
/// that split them — a live output line must never show half a word.
#[test]
fn test_observer_reassembles_lines_split_across_reads() {
    let lines = observed_lines(&[b"downloa", b"ding 40%\nunpack", b"ing\n"]);

    assert_eq!(lines, vec!["downloading 40%", "unpacking"]);
}

/// A trailing line with no newline is never reported: it may still be growing,
/// and showing a half-line as if complete is worse than showing the previous
/// one.
#[test]
fn test_observer_withholds_a_line_that_has_no_newline_yet() {
    let lines = observed_lines(&[b"complete\n", b"still-writing"]);

    assert_eq!(lines, vec!["complete"]);
}

/// Blank lines carry nothing to display; progress output is full of them.
#[test]
fn test_observer_skips_blank_lines() {
    let lines = observed_lines(&[b"a\n\n  \nb\n"]);

    assert_eq!(lines, vec!["a", "b"]);
}

/// A pathological line with no newline must not grow the buffer without bound.
#[test]
fn test_observer_caps_a_pathologically_long_line() {
    let huge = "x".repeat(100_000);

    let lines = observed_lines(&[huge.as_bytes(), b"\n"]);

    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].len() <= LineSplitter::MAX_LINE,
        "line must be capped, got {} bytes",
        lines[0].len()
    );
}

/// A drain with no observer still captures — the log and UI views do not
/// depend on anyone watching.
#[test]
fn test_drain_captures_without_an_observer() {
    let capture = Capture::new();

    drain_into(b"hello\n".as_slice(), &capture, None);

    assert_eq!(capture.ui(), "hello\n");
}

// ── throttle ─────────────────────────────────────────────────────────────────

/// The first line goes out immediately, and one arriving inside the window is
/// *held* rather than dropped: it becomes the pending line, so the newest output
/// survives the rate limit instead of vanishing.
#[test]
fn test_throttle_emits_the_first_line_and_holds_the_next_in_window() {
    let throttle = Throttle::new(Duration::from_millis(250));
    let start = Instant::now();

    assert_eq!(throttle.offer("first", start), Some("first".to_string()));
    assert_eq!(
        throttle.offer("second", start + Duration::from_millis(100)),
        None
    );
    assert_eq!(
        throttle.take_pending(),
        Some("second".to_string()),
        "the line inside the window must be retained, not dropped"
    );
}

/// A burst inside one window collapses to its newest line: the display shows a
/// single line, so an older held line has no value once a newer one exists.
#[test]
fn test_throttle_keeps_only_the_newest_held_line() {
    let throttle = Throttle::new(Duration::from_millis(250));
    let start = Instant::now();
    throttle.offer("emitted", start);

    throttle.offer("held-then-superseded", start + Duration::from_millis(50));
    throttle.offer("newest", start + Duration::from_millis(100));

    assert_eq!(throttle.take_pending(), Some("newest".to_string()));
}

/// Once the window passes, emission resumes and nothing is left pending — the
/// emitted line *is* the newest, so holding it too would emit it twice.
#[test]
fn test_throttle_emits_again_after_the_window_and_clears_the_held_line() {
    let throttle = Throttle::new(Duration::from_millis(250));
    let start = Instant::now();
    throttle.offer("first", start);
    throttle.offer("held", start + Duration::from_millis(50));

    assert_eq!(
        throttle.offer("later", start + Duration::from_millis(300)),
        Some("later".to_string())
    );

    assert_eq!(
        throttle.take_pending(),
        None,
        "a line emitted after the window supersedes the held one"
    );
}

/// The window is measured from the last *emitted* line, not the last offer: a
/// stream of held lines must not extend the silence.
#[test]
fn test_throttle_window_runs_from_the_last_emission() {
    let throttle = Throttle::new(Duration::from_millis(250));
    let start = Instant::now();
    throttle.offer("first", start);

    assert_eq!(
        throttle.offer("held", start + Duration::from_millis(200)),
        None
    );

    assert_eq!(
        throttle.offer("next", start + Duration::from_millis(260)),
        Some("next".to_string()),
        "a held line must not restart the window"
    );
}

/// A pending line is taken once. Taking it twice would re-emit a line the
/// display already shows.
#[test]
fn test_throttle_yields_a_held_line_only_once() {
    let throttle = Throttle::new(Duration::from_millis(250));
    let start = Instant::now();
    throttle.offer("first", start);
    throttle.offer("held", start + Duration::from_millis(50));

    assert_eq!(throttle.take_pending(), Some("held".to_string()));

    assert_eq!(throttle.take_pending(), None);
}

/// Restarting opens the window immediately, which is what lets a new attempt's
/// first line go out even when it arrives inside the previous attempt's window.
/// It also discards a held line: that line belongs to the attempt that just
/// ended, and the new attempt is about to clear the display.
#[test]
fn test_throttle_restart_opens_the_window_and_discards_the_held_line() {
    let throttle = Throttle::new(Duration::from_millis(250));
    let start = Instant::now();
    throttle.offer("previous attempt", start);
    throttle.offer("held", start + Duration::from_millis(10));

    throttle.restart();

    assert_eq!(throttle.take_pending(), None);
    assert_eq!(
        throttle.offer("new attempt", start + Duration::from_millis(20)),
        Some("new attempt".to_string())
    );
}

// ── cut-edge erosion ─────────────────────────────────────────────────────────

/// A secret cut in half by the head cap must not survive as a fragment.
/// Redaction matches whole tokens — a prefixed secret up to the next whitespace
/// — so `nsec1qqq…` cut mid-value would still be scrubbed, but the *tail* of
/// that same value, having lost its prefix, would not be. Both cut edges drop
/// their partial token for that reason.
#[test]
fn test_capture_drops_the_partial_token_at_each_cut_edge() {
    // Positioned so the head cap lands inside the first secret and the tail cap
    // inside the second.
    let head_secret = "nsec1headsecretvalue";
    let tail_secret = "nsec1tailsecretvalue";
    let input = format!(
        "{} {head_secret} {} {tail_secret} {}",
        "h".repeat(500),
        "m".repeat(4000),
        "t".repeat(1010)
    );

    let out = ui(&[input.as_bytes()]);

    assert!(out.contains("bytes omitted"), "input must exceed the cap");
    for fragment in ["nsec1head", "secretvalue"] {
        assert!(
            !out.contains(fragment),
            "a fragment of a cut token must not survive: {out}"
        );
    }
}

/// Erosion stops at the nearest whitespace, so it costs one partial token and
/// not the surrounding output — the head's earlier lines and the tail's later
/// ones are what make a truncated capture readable.
#[test]
fn test_capture_erosion_keeps_the_complete_tokens_around_the_cut() {
    let input = format!(
        "opening line
{}
cut-here-head{}cut-here-tail
{}
closing line
",
        "h".repeat(480),
        "m".repeat(4000),
        "t".repeat(980)
    );

    let out = ui(&[input.as_bytes()]);

    assert!(out.starts_with("opening line\n"), "got: {out}");
    assert!(out.ends_with("closing line\n"), "got: {out}");
}

/// A cut inside a whitespace-free run longer than the erosion window is left
/// intact. Erosion is bounded on purpose: erasing kilobytes of a single-token
/// stream — `npm` progress bars and base64 payloads both look like this — would
/// cost more diagnostics than a fragment of one could leak.
#[test]
fn test_capture_of_one_giant_token_keeps_its_cut_edges() {
    let input = "x".repeat(4000);

    let out = ui(&[input.as_bytes()]);

    assert!(out.starts_with(&"x".repeat(512)), "got: {out}");
    assert!(out.ends_with(&"x".repeat(1024)), "got: {out}");
}

/// The marker's byte count stays honest across erosion: what it names as omitted
/// must equal the input minus what is actually shown, or a reader cannot trust
/// the file to say how much is missing.
#[test]
fn test_capture_marker_counts_the_bytes_erosion_dropped() {
    let input = format!(
        "{} {} {}",
        "h".repeat(600),
        "m".repeat(4000),
        "t".repeat(1100)
    );

    let out = ui(&[input.as_bytes()]);

    let (head, rest) = out.split_once('\n').expect("a marker line");
    let (marker, tail) = rest.split_once('\n').expect("a marker line");
    let omitted: usize = marker
        .trim_start_matches("... (")
        .split_once(' ')
        .expect("a byte count")
        .0
        .parse()
        .expect("a byte count");
    assert_eq!(
        head.len() + omitted + tail.len(),
        input.len(),
        "shown + omitted must account for every input byte"
    );
}
