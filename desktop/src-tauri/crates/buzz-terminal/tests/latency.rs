//! G3: the renderer's wait for the terminal lock, under flood.
//!
//! The plan originally required "reader hold < 16.7 ms". That requirement was
//! struck: measured under a 180 MB/s flood, reader hold is p50 1 us while
//! renderer *acquire* is p50 4245 us. Hold time passes trivially while the
//! window is visibly stuck, because 0.389% of feeds carry 96.4% of the lock
//! time and the p50 hold never sees them. What a human feels is the wait, so
//! that is what is gated here.
//!
//! F1 is the fence being tested. It is a memory bound *and* a latency fence:
//! it turns one ~2 MiB parser release into ~64 KiB pieces, and the renderer's
//! wait falls with it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use buzz_terminal::damage::Encoder;
use buzz_terminal::fences::Fences;
use buzz_terminal::{SharedTerminal, Size, Terminal};

/// One frame at 60 Hz. No acquire may exceed this: a single wait this long is
/// a dropped frame regardless of how good the distribution looks.
///
/// Unlike the p95 below, this bound **cannot be protected by headroom**, and
/// that asymmetry is why this test is `#[ignore]`d and run only in release on
/// an idle host. A quantile discards its worst samples by construction, so it
/// degrades gracefully as a machine gets noisy; a maximum over `FRAMES` samples
/// is a single observation, and any one scheduler preemption exceeds it. There
/// is no budget that makes the max arm robust to contention -- the tail it
/// catches belongs to the scheduler, not to this code.
///
/// Measured on one 16-core host at `FRAMES = 200`: at load average ~6 the gate
/// passes; at ~31 it fails with p95 65535 us / max 164889 us. A run at ambient
/// load produced p95 1023 us -- 4x *inside* budget -- while max alone blew at
/// 38150 us.
///
/// So the repair for a flake here is to fix the host, never to raise this
/// number. Raising it is the one change that silently removes the only assert
/// that catches the user-visible failure: a hitch is a max-event, and a
/// p95-only gate passes a run containing a 38 ms stall.
const FRAME_MICROS: u64 = 16_667;

/// p95 budget. Measured at 127 us with F1 on -- 31x of headroom, which is the
/// margin that lets *this* arm tolerate a loaded machine without becoming a
/// coin flip. The reasoning covers the quantile only; see `FRAME_MICROS`.
const P95_MICROS: u64 = 4_000;

/// Frames sampled per arm. Counted rather than timed: sample count under a
/// wall-clock budget is a function of how slow the arm is, so a duration-based
/// loop gives the *unfenced* arm the fewest samples -- fewest exactly where the
/// tail being measured lives. Counting frames makes both arms the same
/// experiment.
const FRAMES: u32 = 200;

/// A ~2 MiB synchronized update, closed, replayed in PTY-sized reads.
///
/// The payload's *shape* is the load-bearing part, and it cost me a wrong
/// result to learn it. An earlier version poured 8 KiB blocks of `A` into an
/// update that was never closed. It floods just as many bytes per second, and
/// it does not discriminate F1 at all: measured p95 63 us fenced vs 63 us
/// unfenced. Plain `A` overwrites one line at a few ns per byte, so even a
/// 2 MiB release is a short lock hold.
///
/// What makes a release expensive is work per byte -- SGR state changes and
/// `\r\n` line feeds that push rows into scrollback. With that payload the same
/// experiment separates by 129x. So this gate is sensitive to input shape and
/// not merely to input rate, which is why the control below is not optional.
fn flood(shared: &SharedTerminal, stop: &AtomicBool) {
    let mut payload: Vec<u8> = b"\x1b[?2026h".to_vec();
    while payload.len() < (2 << 20) {
        payload.extend_from_slice(b"\x1b[1;32mbuzz\x1b[0m substrate line of output 0123456789\r\n");
    }
    payload.extend_from_slice(b"\x1b[?2026l");
    while !stop.load(Ordering::Relaxed) {
        for chunk in payload.chunks(8192) {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            shared.feed_fully(chunk);
        }
    }
}

/// Render at 60 Hz for the duration of the flood, and report the renderer
/// plane's acquisition latencies.
fn measure(fences: Fences) -> buzz_terminal::AcquireStats {
    let size = Size {
        columns: 200,
        screen_lines: 50,
        scrollback: 10_000,
    };
    let (term, _actions) = Terminal::new(size, fences);
    let shared = Arc::new(SharedTerminal::new(term));
    let stop = Arc::new(AtomicBool::new(false));

    let writer = {
        let (shared, stop) = (Arc::clone(&shared), Arc::clone(&stop));
        thread::spawn(move || flood(&shared, &stop))
    };

    // Don't measure the ramp: let the flood reach steady state, then clear.
    thread::sleep(Duration::from_millis(200));
    shared.renderer_acquire().reset();

    let mut encoder = Encoder::new();
    for _ in 0..FRAMES {
        shared.render(&mut encoder);
        thread::sleep(Duration::from_micros(FRAME_MICROS));
    }
    let stats = shared.renderer_acquire().snapshot();

    stop.store(true, Ordering::Relaxed);
    writer.join().expect("flood thread panicked");

    assert_eq!(stats.acquisitions, FRAMES as u64, "meter lost samples");
    stats
}

/// G3: with F1 on, the renderer's wait stays inside a frame -- and the
/// unfenced control shows the fence is what puts it there.
///
/// Both arms live in one `#[test]` on purpose. As separate tests they run
/// concurrently by default, each with its own flood thread, so each arm's
/// measurement includes the other arm's CPU load and the control's ratio
/// becomes a race between two floods rather than a statement about F1.
#[test]
#[ignore = "native performance gate; run release-mode on a known-idle host"]
fn g3_renderer_acquire_stays_within_frame_budget() {
    let fenced = measure(Fences::ALL);
    let p95 = fenced.percentile_micros(0.95);
    assert!(
        p95 <= P95_MICROS,
        "renderer acquire p95 {p95} us over the {P95_MICROS} us budget (max {} us, n={})",
        fenced.max_micros,
        fenced.acquisitions
    );
    assert!(
        fenced.max_micros <= FRAME_MICROS,
        "renderer waited {} us for the terminal lock -- a dropped frame (p95 {p95} us, n={})",
        fenced.max_micros,
        fenced.acquisitions
    );

    // The control. Without it this gate could pass because the fixture never
    // contended -- green over an experiment that did not run.
    let unfenced = measure(Fences::OSC_ONLY);
    assert!(
        unfenced.max_micros > fenced.max_micros.max(1) * 4,
        "unfenced renderer max {} us vs fenced {} us -- F1 is not what holds \
         renderer latency down, and this gate is measuring something else",
        unfenced.max_micros,
        fenced.max_micros
    );
}
