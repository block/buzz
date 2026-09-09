//! Recognise provider usage-limit errors and work out when they lift.
//!
//! When the account behind the agent CLI exhausts a usage window, every
//! `session/prompt` fails with a JSON-RPC error whose message names the reset
//! time, e.g. (Claude Code via claude-agent-acp, code `-32603`):
//!
//! ```text
//! Internal error: You've hit your session limit · resets 3:10am (Asia/Seoul)
//! ```
//!
//! Such a failure is deterministic until the window resets: exponential
//! backoff cannot outlast a multi-hour window, so treating it like a transient
//! error burns the whole retry budget in ~20 minutes and dead-letters the batch
//! — the triggering event is discarded, and nothing resumes once the limit
//! lifts. This module classifies these errors and parses the reset time so the
//! queue can hold the batch until then instead
//! ([`EventQueue::requeue_held`](crate::queue::EventQueue::requeue_held)).
//!
//! # Timezone
//!
//! The reset time is interpreted in the harness process's local timezone. The
//! agent CLI is a child of this process on the same host, so the zone it
//! prints (`(Asia/Seoul)` above) is the same one `chrono::Local` resolves to;
//! the zone label itself is not parsed.

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, TimeZone};
use std::time::Duration;

/// Slack added after the parsed reset instant so the retry lands once the
/// provider has actually rolled the window over, not a second before.
pub(crate) const RESET_BUFFER_SECS: u64 = 90;

/// Hold applied when the error is a usage limit but no reset time could be
/// parsed from it. Long enough to avoid a tight retry loop, short enough that
/// a five-hour window is not overshot by much.
pub(crate) const FALLBACK_HOLD_SECS: u64 = 30 * 60;

/// Upper bound on a single hold. Weekly limits reset up to seven days out;
/// anything longer is a parse artefact and is clamped.
pub(crate) const MAX_HOLD_SECS: u64 = 8 * 24 * 60 * 60;

/// A usage-limit error extracted from a failed prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsageLimit {
    /// Reset instant parsed from the message, in local time. `None` when the
    /// message names no parseable time.
    pub resets_at: Option<DateTime<Local>>,
}

impl UsageLimit {
    /// How long to hold the batch before retrying, measured from `now`.
    ///
    /// With a parsed reset: time until the reset plus [`RESET_BUFFER_SECS`],
    /// clamped to [`MAX_HOLD_SECS`]. Without one: [`FALLBACK_HOLD_SECS`].
    pub(crate) fn hold_delay(&self, now: DateTime<Local>) -> Duration {
        match self.resets_at {
            Some(at) => {
                // A reset in the past (clock skew, parse edge) yields a zero
                // wait; the buffer alone then spaces the retry.
                let until = (at - now).to_std().unwrap_or_default();
                (until + Duration::from_secs(RESET_BUFFER_SECS))
                    .min(Duration::from_secs(MAX_HOLD_SECS))
            }
            None => Duration::from_secs(FALLBACK_HOLD_SECS),
        }
    }

    /// Human-readable reset time for notices and logs, e.g. `03:10 (+09:00)`.
    pub(crate) fn reset_label(&self) -> Option<String> {
        self.resets_at
            .map(|at| at.format("%H:%M (%:z)").to_string())
    }
}

/// Classify `message` (the `AgentError` text) as a usage-limit error and, if
/// it is one, parse its reset time relative to the current local time.
pub(crate) fn detect(message: &str) -> Option<UsageLimit> {
    detect_at(message, Local::now())
}

/// [`detect`] with an injectable `now` for deterministic tests.
pub(crate) fn detect_at(message: &str, now: DateTime<Local>) -> Option<UsageLimit> {
    if !is_usage_limit_message(message) {
        return None;
    }
    Some(UsageLimit {
        resets_at: parse_reset_time(message, now),
    })
}

/// Whether `message` describes an exhausted account usage window.
///
/// Matches the "You've hit your … limit" family (session / weekly / usage
/// limit, Claude Code) and the generic "usage limit" phrase. Rate-limit
/// (HTTP 429) messages from the API say "rate limit" without "hit your" and
/// are deliberately left to ordinary backoff. A false positive here only
/// delays a retry (the batch is held, never dropped), so the patterns lean
/// towards recall rather than the precision required of
/// [`is_auth_error`](crate::is_auth_error).
pub(crate) fn is_usage_limit_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    (lower.contains("hit your") && lower.contains("limit")) || lower.contains("usage limit")
}

/// Parse the reset time named after the word `reset`/`resets` in `message`.
///
/// Understands `3:10am`, `3am`, `11pm`, `15:30`, an optional `Sep 8` /
/// `September 8` date, and `today` / `tomorrow`. A bare clock time earlier
/// than `now` means the next day. Returns `None` when no time is named.
pub(crate) fn parse_reset_time(message: &str, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let lower = message.to_ascii_lowercase();
    let idx = lower.find("reset")?;
    // Join "3:10 am" → "3:10am" so a detached meridiem still parses.
    let rest = lower[idx..].replace(" am", "am").replace(" pm", "pm");

    let mut time: Option<(u32, u32)> = None;
    let mut date: Option<NaiveDate> = None;
    let mut day_offset: Option<u64> = None;
    let mut pending_month: Option<u32> = None;

    let tokens = rest
        .split(|c: char| c.is_whitespace() || c == ',' || c == '·')
        .map(|t| t.trim_matches(|c| c == '(' || c == ')' || c == '.'))
        .filter(|t| !t.is_empty())
        .skip(1) // the reset word itself
        .take(8);
    for tok in tokens {
        if matches!(tok, "at" | "on" | "in" | "the") {
            continue;
        }
        if time.is_none() {
            if let Some(t) = parse_clock(tok) {
                time = Some(t);
                continue;
            }
        }
        match tok {
            "today" => day_offset = Some(0),
            "tomorrow" => day_offset = Some(1),
            _ => {
                if let Some(m) = month_number(tok) {
                    pending_month = Some(m);
                } else if let (Some(m), Ok(d)) = (pending_month, tok.parse::<u32>()) {
                    date = NaiveDate::from_ymd_opt(now.year(), m, d);
                    pending_month = None;
                }
            }
        }
    }

    let (hour, minute) = time?;
    let today = now.date_naive();
    let base = match (date, day_offset) {
        (Some(d), _) => d,
        (None, Some(off)) => today.checked_add_days(Days::new(off))?,
        (None, None) => today,
    };
    let candidate = local_at(base, hour, minute)?;
    if candidate > now {
        return Some(candidate);
    }
    match (date, day_offset) {
        // Explicit date already in the past: only possible across a year
        // boundary (e.g. "resets Jan 2" parsed in late December).
        (Some(d), _) => local_at(d.with_year(now.year() + 1)?, hour, minute),
        // "today"/"tomorrow" in the past: take it at face value.
        (None, Some(_)) => Some(candidate),
        // Bare clock time already passed today → next occurrence.
        (None, None) => local_at(base.checked_add_days(Days::new(1))?, hour, minute),
    }
}

fn local_at(date: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Local>> {
    let naive = date.and_hms_opt(hour, minute, 0)?;
    // `earliest` picks the first valid instant across a DST gap/overlap.
    Local.from_local_datetime(&naive).earliest()
}

/// Parse `3:10am`, `3am`, `12pm`, `15:30` into a 24h `(hour, minute)`.
/// A bare number without a meridiem is not a clock time.
fn parse_clock(tok: &str) -> Option<(u32, u32)> {
    let (body, pm) = if let Some(b) = tok.strip_suffix("am") {
        (b, Some(false))
    } else if let Some(b) = tok.strip_suffix("pm") {
        (b, Some(true))
    } else {
        (tok, None)
    };
    let (hour, minute) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok().filter(|_| pm.is_some())?, 0),
    };
    if minute > 59 {
        return None;
    }
    let hour = match pm {
        Some(pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            (hour % 12) + if pm { 12 } else { 0 }
        }
        None => {
            if hour > 23 {
                return None;
            }
            hour
        }
    };
    Some((hour, minute))
}

fn month_number(tok: &str) -> Option<u32> {
    if tok.len() < 3 || !tok.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    MONTHS
        .iter()
        .position(|m| tok.starts_with(m))
        .map(|i| i as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    const INCIDENT_MSG: &str =
        "Internal error: You've hit your session limit · resets 3:10am (Asia/Seoul)";

    /// 2026-09-03 00:47:46 local — the first failed turn of the incident.
    fn incident_now() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 9, 3, 0, 47, 46)
            .single()
            .expect("unambiguous local time")
    }

    #[test]
    fn detects_the_incident_message_and_parses_its_reset() {
        let limit = detect_at(INCIDENT_MSG, incident_now()).expect("usage limit");
        let at = limit.resets_at.expect("reset time");
        assert_eq!(
            at.date_naive(),
            NaiveDate::from_ymd_opt(2026, 9, 3).unwrap()
        );
        assert_eq!((at.hour(), at.minute()), (3, 10));
        let delay = limit.hold_delay(incident_now());
        // 2h22m14s until reset, plus the buffer.
        assert_eq!(
            delay,
            Duration::from_secs(2 * 3600 + 22 * 60 + 14 + RESET_BUFFER_SECS)
        );
    }

    #[test]
    fn clock_time_already_passed_today_rolls_to_tomorrow() {
        let now = incident_now(); // 00:47
        let at = parse_reset_time("hit your limit · resets 12:30am", now).unwrap();
        assert_eq!(
            at.date_naive(),
            NaiveDate::from_ymd_opt(2026, 9, 4).unwrap()
        );
        assert_eq!((at.hour(), at.minute()), (0, 30));
    }

    #[test]
    fn parses_hour_only_meridiem_and_24h_forms() {
        let now = incident_now();
        let pm = parse_reset_time("resets 11pm (America/Los_Angeles)", now).unwrap();
        assert_eq!((pm.hour(), pm.minute()), (23, 0));
        let noon = parse_reset_time("resets 12pm", now).unwrap();
        assert_eq!(noon.hour(), 12);
        let midnight = parse_reset_time("resets 12am", now).unwrap();
        assert_eq!(midnight.hour(), 0);
        let h24 = parse_reset_time("resets at 15:30", now).unwrap();
        assert_eq!((h24.hour(), h24.minute()), (15, 30));
        let spaced = parse_reset_time("resets 3:10 am", now).unwrap();
        assert_eq!((spaced.hour(), spaced.minute()), (3, 10));
    }

    #[test]
    fn parses_an_explicit_date_for_weekly_limits() {
        let now = incident_now();
        let at =
            parse_reset_time("You've hit your weekly limit · resets Sep 8 at 3pm", now).unwrap();
        assert_eq!(
            at.date_naive(),
            NaiveDate::from_ymd_opt(2026, 9, 8).unwrap()
        );
        assert_eq!(at.hour(), 15);
        let tomorrow = parse_reset_time("resets tomorrow at 9am", now).unwrap();
        assert_eq!(
            tomorrow.date_naive(),
            NaiveDate::from_ymd_opt(2026, 9, 4).unwrap()
        );
    }

    #[test]
    fn unparseable_reset_falls_back_to_the_default_hold() {
        let limit = detect_at("You've hit your usage limit.", incident_now()).unwrap();
        assert_eq!(limit.resets_at, None);
        assert_eq!(
            limit.hold_delay(incident_now()),
            Duration::from_secs(FALLBACK_HOLD_SECS)
        );
        assert!(parse_reset_time("resets soon", incident_now()).is_none());
        // A clock time without the `reset` keyword is not a reset time.
        assert!(parse_reset_time("try again after 3:10am", incident_now()).is_none());
    }

    #[test]
    fn hold_is_clamped_and_never_negative() {
        let now = incident_now();
        let far = UsageLimit {
            resets_at: Some(now + chrono::Duration::days(30)),
        };
        assert_eq!(far.hold_delay(now), Duration::from_secs(MAX_HOLD_SECS));
        let past = UsageLimit {
            resets_at: Some(now - chrono::Duration::hours(1)),
        };
        assert_eq!(past.hold_delay(now), Duration::from_secs(RESET_BUFFER_SECS));
    }

    #[test]
    fn ignores_errors_that_are_not_usage_limits() {
        for msg in [
            "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
            "Usage credits required for 1M context",
            "rate_limit_error: This request would exceed your organization's rate limit",
            "Internal error: something else",
        ] {
            assert!(detect_at(msg, incident_now()).is_none(), "{msg}");
        }
    }
}
