//! Deciding whether a local clock is why Blossom media auth was refused.
//!
//! A Blossom upload token (`kind:24242`) is stamped with the client's clock and
//! checked against the relay's. A client whose system clock has drifted mints
//! tokens the relay reads as out of window and rejects — with a deliberately
//! opaque `401 authentication failed`, because the media auth errors are
//! collapsed to defeat enumeration. The failure is therefore indistinguishable
//! from a signing, scope, or authorization problem, even though it is neither a
//! Buzz bug nor anything the relay can fix.
//!
//! A relay response's `Date` header is a reference clock the client can compare
//! against. The comparison is not exact: `Date` is truncated to whole seconds,
//! and the client can only observe it before and after a round trip it does not
//! control. So this module never computes a point estimate. It brackets the
//! offset into the interval the observation actually proves, and speaks up only
//! when that whole interval lies outside what the relay accepts. Anything less
//! certain stays silent.
//!
//! Every function here is pure — the caller supplies the timestamps.
//!
//! ```
//! use buzz_core::clock_skew::ClockOffsetBounds;
//!
//! // The relay's `Date` says 09:14:38. The client read its own clock at
//! // 09:14:44.75 before sending and 09:14:44.95 after the reply arrived, so
//! // even the most charitable reading has it more than 5s fast.
//! let bounds = ClockOffsetBounds::measure(
//!     1_787_822_084_750,
//!     1_787_822_084_950,
//!     "Thu, 27 Aug 2026 09:14:38 GMT",
//! )
//! .expect("IMF-fixdate parses");
//!
//! assert_eq!(bounds.min_millis(), 5_750);
//! assert!(bounds.media_auth_advice(600).is_some());
//! ```

use chrono::{DateTime, NaiveDateTime};

/// Seconds a Blossom auth event's `created_at` may lead the verifier's clock
/// before it is rejected as `TimestampOutOfWindow`.
///
/// Mirrors the tolerance enforced by `buzz_media::auth`; a client that widened
/// this would only make itself quieter, never the relay more permissive.
pub const BLOSSOM_FUTURE_TOLERANCE_SECS: u64 = 5;

/// Resolution of an HTTP `Date` header, in milliseconds. The header names a
/// whole second, so the server's true clock at stamp time lies somewhere in the
/// second that follows it.
const DATE_RESOLUTION_MILLIS: i64 = 1_000;

/// Parse an RFC 9110 `Date` header into a Unix timestamp in seconds.
///
/// Accepts IMF-fixdate (`Thu, 27 Aug 2026 09:14:38 GMT`), which RFC 9110
/// requires senders to emit, and falls back to RFC 2822 for senders that use a
/// numeric offset. The obsolete RFC 850 and asctime forms are not accepted;
/// they yield `None`, which costs a diagnosis and nothing else.
pub fn parse_http_date(value: &str) -> Option<i64> {
    let value = value.trim();
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%a, %d %b %Y %H:%M:%S GMT") {
        return Some(naive.and_utc().timestamp());
    }
    DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|dt| dt.timestamp())
}

/// The interval of local-clock offsets consistent with one observed exchange.
///
/// Positive means the local clock is ahead of the server's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockOffsetBounds {
    min_millis: i64,
    max_millis: i64,
}

impl ClockOffsetBounds {
    /// Bracket the local clock's offset from one request/response pair.
    ///
    /// `before` and `after` are local Unix **milliseconds**, read immediately
    /// before the request was sent and immediately after the response arrived;
    /// the server stamped `Date` somewhere between them. Writing `Δ` for the
    /// true offset, `D` for the parsed header and `S` for the server's real
    /// clock at stamp time, `D ≤ S < D + 1s` and `before − Δ ≤ S ≤ after − Δ`,
    /// which rearranges to
    ///
    /// ```text
    /// before − D − 1s  ≤  Δ  ≤  after − D
    /// ```
    ///
    /// Both bounds hold whatever the network latency was, so no fudge factor is
    /// needed and none is applied. Milliseconds matter: only the *server's*
    /// stamp is truncated, so reading the local clock at full resolution keeps
    /// the interval a second narrower than whole-second readings would — enough
    /// to place a drift of six-and-change seconds decisively outside a five
    /// second tolerance.
    ///
    /// Returns `None` if the header is unparseable, or if the readings are out
    /// of order and the interval would be nonsense.
    pub fn measure(before: i64, after: i64, date_header: &str) -> Option<Self> {
        if after < before {
            return None;
        }
        let date_millis = parse_http_date(date_header)?.checked_mul(DATE_RESOLUTION_MILLIS)?;
        Some(Self {
            min_millis: before - date_millis - DATE_RESOLUTION_MILLIS,
            max_millis: after - date_millis,
        })
    }

    /// The least the local clock can be ahead by, in milliseconds.
    pub fn min_millis(self) -> i64 {
        self.min_millis
    }

    /// The most the local clock can be ahead by, in milliseconds.
    pub fn max_millis(self) -> i64 {
        self.max_millis
    }

    /// A message reporting the drift, or `None` when the observation does not
    /// prove this clock is outside what media upload auth allows.
    ///
    /// # What a verdict does and does not claim
    ///
    /// It claims a *measurement*: the local clock is provably outside the
    /// window, and a clock in that state will break media uploads. It does not
    /// claim to have diagnosed the 401 in hand, and the wording is careful not
    /// to. Certainty about *this* rejection is not available to a client: the
    /// relay compares whole seconds (`created > now + 5`), both sides are
    /// truncated, and the transit time that separates them is unknown — so a
    /// drift of, say, 5.6s is rejected or accepted depending on where the two
    /// clocks happen to fall inside their respective seconds. Asserting cause
    /// would be wrong in exactly the band this feature exists to serve.
    ///
    /// Callers therefore append this to the relay's own message rather than
    /// replacing it: the user gets a real lead without losing the evidence.
    ///
    /// # `token_lifetime_secs`
    ///
    /// The `expiration` the client stamps on its own upload tokens. A clock
    /// behind by more than a token's whole lifetime mints one already expired
    /// when it lands. This is one of the verifier's two backward bounds, and
    /// the only one a client knows exactly — the other, `max_age_secs`, is 600s
    /// or 3600s depending on a pipeline the relay picks by sniffing the body
    /// bytes rather than trusting `Content-Type`. Where that bound bites first
    /// this stays silent, which is the safe direction to be wrong in.
    pub fn media_auth_advice(self, token_lifetime_secs: u64) -> Option<String> {
        let tolerance_millis = BLOSSOM_FUTURE_TOLERANCE_SECS as i64 * DATE_RESOLUTION_MILLIS;
        let lifetime_millis = i64::try_from(token_lifetime_secs)
            .ok()?
            .checked_mul(DATE_RESOLUTION_MILLIS)?;

        if self.min_millis > tolerance_millis {
            return Some(Self::advice(
                self.min_millis,
                "ahead of",
                format!("the {BLOSSOM_FUTURE_TOLERANCE_SECS}s that media upload auth allows"),
            ));
        }
        // A token minted this far behind is already expired when it lands. The
        // extra second absorbs the truncation of the `created_at` the client
        // stamps, so the expiry is genuinely in the past and not merely level
        // with the relay's clock.
        if self.max_millis < -(lifetime_millis + DATE_RESOLUTION_MILLIS) {
            return Some(Self::advice(
                -self.max_millis,
                "behind",
                format!("the {token_lifetime_secs}s an upload token is valid for"),
            ));
        }
        None
    }

    fn advice(magnitude_millis: i64, direction: &str, limit: String) -> String {
        let magnitude = magnitude_millis as f64 / DATE_RESOLUTION_MILLIS as f64;
        format!(
            "note: this machine's clock is at least {magnitude:.1}s {direction} the relay's, which \
             is outside {limit}. A clock this far out makes media uploads fail exactly like this, \
             so it is worth ruling out before anything else. Sync the system clock and retry \
             (Windows: start the w32time service, then `w32tm /resync`; macOS and Linux: turn on \
             automatic network time)."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Thu, 27 Aug 2026 09:14:38 GMT`
    const SERVER_UNIX_MILLIS: i64 = 1_787_822_078_000;
    const SERVER_DATE: &str = "Thu, 27 Aug 2026 09:14:38 GMT";
    /// The `expiration` the CLI stamps on an image upload token.
    const IMAGE_TOKEN_LIFETIME: u64 = 600;

    #[test]
    fn parses_imf_fixdate() {
        assert_eq!(
            parse_http_date(SERVER_DATE),
            Some(SERVER_UNIX_MILLIS / 1_000)
        );
    }

    #[test]
    fn parses_rfc2822_with_a_non_zero_offset() {
        assert_eq!(
            parse_http_date("Thu, 27 Aug 2026 09:14:38 +0000"),
            Some(SERVER_UNIX_MILLIS / 1_000)
        );
        // 14:14:38 +0500 is the same instant.
        assert_eq!(
            parse_http_date("Thu, 27 Aug 2026 14:14:38 +0500"),
            Some(SERVER_UNIX_MILLIS / 1_000)
        );
    }

    #[test]
    fn rejects_unparseable_dates() {
        assert_eq!(parse_http_date(""), None);
        assert_eq!(parse_http_date("not a date"), None);
        assert_eq!(parse_http_date("2026-08-27T09:14:38Z"), None);
    }

    /// An exchange where the local clock is `before` ms fast relative to the
    /// server's stamp, and the reply came back `after - before` ms later.
    fn bounds(before_millis: i64, after_millis: i64) -> ClockOffsetBounds {
        ClockOffsetBounds::measure(
            SERVER_UNIX_MILLIS + before_millis,
            SERVER_UNIX_MILLIS + after_millis,
            SERVER_DATE,
        )
        .expect("fixture date parses")
    }

    #[test]
    fn brackets_the_offset_around_the_date_header() {
        let b = bounds(20_000, 21_000);
        assert_eq!(b.min_millis(), 19_000);
        assert_eq!(b.max_millis(), 21_000);
    }

    #[test]
    fn refuses_to_measure_readings_taken_out_of_order() {
        assert!(ClockOffsetBounds::measure(
            SERVER_UNIX_MILLIS + 5,
            SERVER_UNIX_MILLIS,
            SERVER_DATE
        )
        .is_none());
        assert!(
            ClockOffsetBounds::measure(SERVER_UNIX_MILLIS, SERVER_UNIX_MILLIS, "nonsense")
                .is_none()
        );
    }

    /// The regression this exists for: the ~6.75s-fast Windows clock reported
    /// on the issue. Whole-second local readings would leave this straddling
    /// the tolerance and silent; millisecond readings prove it.
    #[test]
    fn catches_the_reported_fast_clock() {
        let advice = bounds(6_750, 6_950)
            .media_auth_advice(IMAGE_TOKEN_LIFETIME)
            .expect("a 6.75s fast clock is outside the 5s tolerance");
        assert!(advice.contains("5.8s ahead of"), "{advice}");
        assert!(
            advice.contains("5s that media upload auth allows"),
            "{advice}"
        );
    }

    /// The message reports a measurement and a lead. It must not claim to have
    /// diagnosed the rejection in hand — the relay compares whole seconds
    /// across an unknown transit delay, so causation is not the client's to
    /// assert.
    #[test]
    fn the_message_does_not_claim_to_explain_the_rejection() {
        let advice = bounds(30_000, 30_100)
            .media_auth_advice(IMAGE_TOKEN_LIFETIME)
            .expect("30s is well outside the tolerance");
        assert!(
            !advice.contains("That is why"),
            "must not assert causation: {advice}"
        );
        assert!(advice.contains("worth ruling out"), "{advice}");
    }

    /// The bug both reviews caught in the first cut of this module: latency
    /// used to inflate the measured offset, so a correct clock on a slow link
    /// got blamed. The lower bound is taken *before* the request, so no amount
    /// of latency can manufacture an "ahead" verdict.
    #[test]
    fn latency_alone_never_produces_a_verdict() {
        for latency in [1_000, 5_000, 30_000, 600_000] {
            assert!(
                bounds(0, latency)
                    .media_auth_advice(IMAGE_TOKEN_LIFETIME)
                    .is_none(),
                "{latency}ms of latency on a synced clock must not be blamed"
            );
        }
    }

    /// A verdict must never outrun the interval: a clock the relay's tolerance
    /// covers must stay silent no matter where the server's stamp fell inside
    /// its second.
    ///
    /// This is the property the message actually asserts — that the drift is
    /// outside the window — and the truncation term in `measure` is what makes
    /// it hold. Drop the `- DATE_RESOLUTION_MILLIS` and a synced clock observed
    /// at a late stamp fraction starts getting reported.
    #[test]
    fn a_verdict_never_outruns_what_the_interval_proves() {
        let tolerance = BLOSSOM_FUTURE_TOLERANCE_SECS as i64 * 1_000;
        for true_drift in [0, 1_000, 4_999, tolerance] {
            for stamp_fraction in [0, 1, 500, 999] {
                // `before - date` reads as the drift plus however far into its
                // second the server's stamp fell.
                let observed = true_drift + stamp_fraction;
                assert!(
                    bounds(observed, observed + 50)
                        .media_auth_advice(IMAGE_TOKEN_LIFETIME)
                        .is_none(),
                    "a {true_drift}ms drift observed at stamp fraction {stamp_fraction} is within \
                     the {tolerance}ms tolerance and must stay silent"
                );
            }
        }

        // Just past the tolerance, at the least favourable stamp fraction, the
        // lower bound clears it and the drift is reported.
        assert!(bounds(tolerance + 1_001, tolerance + 1_051)
            .media_auth_advice(IMAGE_TOKEN_LIFETIME)
            .is_some());
    }

    /// Skew runs both ways. The backward bound is the token's own lifetime plus
    /// the second the client's own `created_at` loses to truncation, so a token
    /// minted this far behind is genuinely expired on arrival rather than level
    /// with the relay's clock.
    #[test]
    fn catches_a_clock_behind_by_more_than_the_token_lives() {
        let lifetime = IMAGE_TOKEN_LIFETIME as i64 * 1_000;
        for not_yet in [lifetime, lifetime + 500, lifetime + 1_000] {
            assert!(
                bounds(-not_yet, -not_yet)
                    .media_auth_advice(IMAGE_TOKEN_LIFETIME)
                    .is_none(),
                "{not_yet}ms behind is not yet proven to outlive a {lifetime}ms token"
            );
        }

        let advice = bounds(-lifetime - 60_000, -lifetime - 60_000)
            .media_auth_advice(IMAGE_TOKEN_LIFETIME)
            .expect("beyond the token lifetime");
        assert!(advice.contains("behind"), "{advice}");
        assert!(advice.contains("660.0s"), "{advice}");
        assert!(
            advice.contains("600s an upload token is valid for"),
            "{advice}"
        );
    }

    /// The backward bound tracks the lifetime the caller actually stamped, so a
    /// short-lived desktop image token is judged more tightly than a video one.
    #[test]
    fn the_backward_bound_follows_the_token_lifetime() {
        let behind = bounds(-700_000, -700_000);
        assert!(behind.media_auth_advice(3600).is_none());
        assert!(behind.media_auth_advice(300).is_some());
    }
}
