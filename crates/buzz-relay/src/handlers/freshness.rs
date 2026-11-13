//! Shared `created_at` freshness (clock-skew) checks.
//!
//! Every skew gate compares in `u64` with [`u64::abs_diff`], so no
//! `created_at` value can overflow the comparison. The previous per-site
//! copies cast to `i64` and used `(event_ts - now).abs()`, which wraps for
//! `created_at = now + 2^63` and let an extreme future timestamp through.

use nostr::Timestamp;

/// Maximum `created_at` skew (seconds, either direction) accepted for signed
/// command events that are executed and not stored: moderation commands,
/// relay admin commands, NIP-IA archive requests and NIP-43 leave requests.
pub(crate) const MAX_COMMAND_SKEW_SECS: u64 = 120;

/// Current Unix time in whole seconds (0 if the system clock is before the
/// epoch, which then fails every freshness check closed).
pub(crate) fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A `created_at` that lies more than the allowed skew away from `now`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SkewExceeded {
    /// The event's `created_at`, in seconds.
    pub(crate) created_at: u64,
    /// The server time it was compared against, in seconds.
    pub(crate) now: u64,
}

impl SkewExceeded {
    /// Signed `created_at - now`, widened so it cannot overflow.
    pub(crate) fn delta(&self) -> i128 {
        i128::from(self.created_at) - i128::from(self.now)
    }
}

/// Accept `created_at` only if it is within `max_skew_secs` of `now`.
pub(crate) fn check_created_at_skew(
    created_at: Timestamp,
    now: u64,
    max_skew_secs: u64,
) -> Result<(), SkewExceeded> {
    let created_at = created_at.as_secs();
    if created_at.abs_diff(now) > max_skew_secs {
        return Err(SkewExceeded { created_at, now });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skew_table() {
        let now: u64 = 1_700_000_000;
        let max = MAX_COMMAND_SKEW_SECS;
        let cases: &[(u64, bool)] = &[
            (now, true),
            (now + max, true),
            (now - max, true),
            (now + max + 1, false),
            (now - max - 1, false),
            (0, false),
            (u64::MAX, false),
            // `as i64 - now` == i64::MIN: the old `.abs()` wrapped and passed.
            ((1u64 << 63) + now, false),
            ((1u64 << 63) + now - 1, false),
            ((1u64 << 63) + now + 1, false),
            (1u64 << 63, false),
        ];
        for &(created_at, ok) in cases {
            let got = check_created_at_skew(Timestamp::from_secs(created_at), now, max);
            assert_eq!(got.is_ok(), ok, "created_at={created_at}");
        }
    }

    #[test]
    fn delta_is_signed_and_does_not_overflow() {
        let e = SkewExceeded {
            created_at: u64::MAX,
            now: 0,
        };
        assert_eq!(e.delta(), i128::from(u64::MAX));
        let e = SkewExceeded {
            created_at: 0,
            now: 5,
        };
        assert_eq!(e.delta(), -5);
    }

    #[test]
    fn epoch_now_rejects_current_timestamps() {
        assert!(check_created_at_skew(Timestamp::from_secs(1_700_000_000), 0, 120).is_err());
    }
}
