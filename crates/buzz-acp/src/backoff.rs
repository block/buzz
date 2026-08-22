//! Jitter helpers for retry, backoff, and rate-limit gate delays.
//!
//! Jitter uses the nanosecond sub-second component of the system clock as a
//! cheap entropy source (no `rand` dependency). The factor computation is a
//! pure function of that value so the reachable range can be tested directly —
//! a wall-clock helper cannot be driven to its domain boundary, which is how
//! the divisor defect below survived every existing bound assertion.
//!
//! Two shapes, and the distinction is load-bearing:
//!
//! * [`jittered_duration`] is **symmetric** (±20%). Correct for self-chosen
//!   backoff ladders, where waking early only costs an extra attempt.
//! * [`extend_with`] is **one-sided** (+0–20%, never shorter). Required
//!   wherever the base duration is an authoritative deadline supplied by the
//!   relay: shortening a `retry in {N}s` hint wakes into a window the relay has
//!   already told us is closed, which earns a fresh denial and burns a counter
//!   increment for no chance of success.

use std::time::Duration;

/// Nanoseconds in a second — the true domain bound of `Duration::subsec_nanos`.
///
/// The previous implementations divided by `u32::MAX` (4,294,967,295), which is
/// 4.295x larger than the largest value `subsec_nanos()` can return. That
/// capped the symmetric factor at 0.893 instead of approaching 1.2, making
/// "±20% jitter" unconditionally negative and averaging −15%.
const NANOS_PER_SEC: f64 = 1_000_000_000.0;

/// Map a sub-second nanosecond count onto `[0.0, 1.0]`.
///
/// Clamped so the function is total for any `u32`; inputs above one second are
/// unreachable from `subsec_nanos()` but must not push the factor out of range.
fn nanos_fraction(nanos: u32) -> f64 {
    (f64::from(nanos) / NANOS_PER_SEC).min(1.0)
}

/// Symmetric jitter factor in `[0.8, 1.2)` over the domain `[0, 1s)`.
pub(crate) fn symmetric_jitter_factor(nanos: u32) -> f64 {
    0.8 + nanos_fraction(nanos) * 0.4
}

/// One-sided jitter factor in `[1.0, 1.2)` over the domain `[0, 1s)`.
///
/// Never returns less than 1.0, so a delay built from it can never fall below
/// its base. This is a structural guarantee, not a policy applied by callers.
pub(crate) fn nonnegative_jitter_factor(nanos: u32) -> f64 {
    1.0 + nanos_fraction(nanos) * 0.2
}

/// Sub-second component of the current wall clock, used as the jitter source.
pub(crate) fn clock_nanos() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
}

/// Apply symmetric ±20% jitter to a self-chosen backoff duration.
pub(crate) fn jittered_duration(base: Duration) -> Duration {
    base.mul_f64(symmetric_jitter_factor(clock_nanos()))
}

/// Extend an authoritative deadline by 0–20%, never shortening it.
///
/// The entropy sample is an explicit parameter so tests can drive the whole
/// domain deterministically. With a wall-clock-only helper the "never
/// shortens" property is only probabilistically testable: a symmetric-jitter
/// regression satisfies it on roughly half of all draws, which reads as a
/// surviving mutant rather than a flaky assertion. Callers pass
/// [`clock_nanos`].
pub(crate) fn extend_with(base: Duration, nanos: u32) -> Duration {
    base.mul_f64(nonnegative_jitter_factor(nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The largest value `Duration::subsec_nanos()` can return. Asserting the
    /// ceiling at `u32::MAX` instead would re-encode the very divisor defect
    /// these tests exist to catch, so the domain bound is pinned explicitly.
    const MAX_SUBSEC_NANOS: u32 = 999_999_999;

    /// Symmetric factor reaches its documented floor and (near) ceiling.
    ///
    /// The ceiling assertion is the discriminating one: the `u32::MAX` divisor
    /// caps this at 0.893, while every range assertion of the form
    /// `0.8 <= f < 1.2` passes in both the broken and the fixed implementation.
    #[test]
    fn symmetric_factor_spans_its_documented_range() {
        assert_eq!(symmetric_jitter_factor(0), 0.8, "floor must be exactly 0.8");
        let ceiling = symmetric_jitter_factor(MAX_SUBSEC_NANOS);
        assert!(
            ceiling > 1.199 && ceiling < 1.2,
            "ceiling {ceiling} must approach 1.2 from below — a ceiling near 0.893 \
             means the factor is divided by u32::MAX instead of 1e9"
        );
    }

    /// One-sided factor never shortens its base and reaches +20%.
    #[test]
    fn nonnegative_factor_spans_its_documented_range() {
        assert_eq!(
            nonnegative_jitter_factor(0),
            1.0,
            "floor must be exactly 1.0 — a one-sided factor may never shorten"
        );
        let ceiling = nonnegative_jitter_factor(MAX_SUBSEC_NANOS);
        assert!(
            ceiling > 1.199 && ceiling < 1.2,
            "ceiling {ceiling} must approach 1.2 from below"
        );
    }

    /// No input can make the one-sided factor shorten a deadline. Swept across
    /// the whole `u32` range, not just the reachable sub-second domain.
    #[test]
    fn nonnegative_factor_is_never_below_one() {
        for step in 0..=1_000u32 {
            let nanos = (u32::MAX / 1_000).saturating_mul(step);
            let factor = nonnegative_jitter_factor(nanos);
            assert!(
                (1.0..=1.2 + 1e-9).contains(&factor),
                "factor {factor} out of [1.0, 1.2] at nanos={nanos}"
            );
        }
    }

    /// Both factors stay in range for out-of-domain inputs (clamped).
    ///
    /// `subsec_nanos()` can never return these values; the clamp exists so the
    /// factor functions are total. Compared with a tolerance because
    /// `0.8 + 1.0 * 0.4` is not exactly 1.2 in binary floating point.
    #[test]
    fn factors_are_clamped_above_one_second() {
        assert!((symmetric_jitter_factor(u32::MAX) - 1.2).abs() < 1e-9);
        assert!((nonnegative_jitter_factor(u32::MAX) - 1.2).abs() < 1e-9);
    }

    /// The wrappers apply their factor to the base duration.
    #[test]
    fn wrappers_stay_within_their_factor_ranges() {
        let base = Duration::from_secs(5);
        for _ in 0..64 {
            let symmetric = jittered_duration(base);
            assert!(
                symmetric >= base.mul_f64(0.8) && symmetric <= base.mul_f64(1.2),
                "symmetric {symmetric:?} out of range"
            );
            let extended = extend_with(base, clock_nanos());
            assert!(
                extended >= base && extended <= base.mul_f64(1.2),
                "extended {extended:?} shortened the base"
            );
        }
    }
}
