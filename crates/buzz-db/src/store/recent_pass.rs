//! Recent-first newest-first page reads over the partitioned `events` table.
//!
//! `events` is range-partitioned by month on `created_at`, and every partition
//! carries the full index set. A read with no `created_at` lower bound cannot
//! prune partitions, so Postgres plans and locks every partition and every
//! partition index: about 200 relations. Only 16 fit a backend's fast-path
//! lock slots; the rest go through the shared lock table, whose partition
//! locks every backend contends on (`LWLock:LockManager`).
//!
//! A newest-first page (`ORDER BY created_at DESC … LIMIT n`) usually lies in
//! the newest partitions, so it is split ([`split_for`], [`next`]) at a floor,
//! the start of the previous UTC month:
//!
//! 1. The *recent* pass reads `created_at >= floor`. When it returns `n`
//!    rows, they are the page: every row at or above the floor sorts before
//!    every row below it. A read of regular kinds only also stops at a
//!    ceiling ([`ceiling_at`]), so it does not plan the empty monthly
//!    partitions made ahead of time or the open-ended last one.
//! 2. Otherwise the *older* pass reads `created_at < floor` for the rows
//!    still missing, and they follow the recent rows. The two ranges are
//!    disjoint, so no row is read twice and the page is exact, provided both
//!    passes read one snapshot (the stores run them in one `REPEATABLE READ`
//!    transaction, `ReadTransaction`).
//!
//! A read whose own `since` is at or above the floor matches no row below
//! it, so its recent pass is the whole read and no older pass follows. The
//! floor literal is still worth adding: `since` is a bind parameter, which a
//! cached generic plan prunes only at execution, after it has locked every
//! partition. Live REQs with a recent `since` (agents reconnecting, `#p`
//! mention reads) are that read.
//!
//! The bounds are SQL literals, so Postgres prunes when it plans the
//! statement, cached generic plans included. They change about once a month,
//! so the prepared-statement cache keeps one statement per pass and shape.

use buzz_core::kind::{is_parameterized_replaceable, is_replaceable};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};

/// How far ahead of a relay's clock a regular-kind row's `created_at` can be,
/// with margin.
///
/// Ingest rejects events more than 900 s ahead of the accepting relay's clock
/// (`MAX_TIMESTAMP_DRIFT_SECS` in `buzz-relay`'s ingest handler), and the
/// regular-kind events the relay signs itself are stamped with its clock. The
/// margin over 900 s covers the clock difference between the relay that wrote
/// a row and the one reading it. It costs one extra planned partition in the
/// last hour of a month.
///
/// Replaceable and addressable rows have no such bound: the relay stamps its
/// own replacements one second past the live row when that is later than
/// `now`, so a coordinate replaced faster than once a second runs ahead.
pub(crate) const MAX_FUTURE: Duration = Duration::hours(1);

/// Where a split page read divides: see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Split {
    /// The recent pass reads at or above it; the older pass below it.
    pub(crate) floor: DateTime<Utc>,
    /// The recent pass reads below it, when set. No row the read can match
    /// sits at or above it.
    pub(crate) ceiling: Option<DateTime<Utc>>,
    /// Whether rows below the floor can match, so a short recent pass is
    /// followed by the older pass. False when the read's own `since` is at
    /// or above the floor: the recent pass is then the whole read.
    pub(crate) older: bool,
}

/// The `created_at` range one pass of a page read covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Range {
    /// Every row: the read is not split.
    All,
    /// `created_at >= floor`, and `created_at < ceiling` when set.
    Recent(Split),
    /// `created_at < floor`.
    Older(DateTime<Utc>),
}

impl Range {
    /// The range as a SQL predicate on `column` (` AND …`), or `None` for
    /// [`Range::All`].
    ///
    /// The bounds are literals, not binds: Postgres prunes partitions on a
    /// literal while it plans, including in a cached generic plan, while a
    /// generic plan with a bind parameter prunes only at execution, after it
    /// has locked every partition. The text is chrono's fixed-format
    /// rendering of a UTC timestamp, digits and separators only, so it
    /// carries no caller input.
    pub(crate) fn predicate(self, column: &str) -> Option<String> {
        let bound = |op: &str, at: DateTime<Utc>| {
            format!(
                " AND {column} {op} '{}'::timestamptz",
                at.format("%Y-%m-%dT%H:%M:%SZ")
            )
        };
        match self {
            Range::All => None,
            Range::Recent(split) => {
                let ceiling = split.ceiling.map(|c| bound("<", c));
                Some(bound(">=", split.floor) + ceiling.as_deref().unwrap_or(""))
            }
            Range::Older(floor) => Some(bound("<", floor)),
        }
    }
}

/// 00:00 UTC on the first day of the month `offset` months from `at`'s.
fn month_start(at: DateTime<Utc>, offset: i32) -> DateTime<Utc> {
    let months = at.year() * 12 + at.month0() as i32 + offset;
    Utc.with_ymd_and_hms(
        months.div_euclid(12),
        months.rem_euclid(12) as u32 + 1,
        1,
        0,
        0,
        0,
    )
    .single()
    .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// The floor at `now`: 00:00 UTC on the first day of the previous month, so
/// the recent pass reads the current and previous monthly partitions.
pub(crate) fn floor_at(now: DateTime<Utc>) -> DateTime<Utc> {
    month_start(now, -1)
}

/// The ceiling at `now`: 00:00 UTC on the first day of the month after
/// `now` + [`MAX_FUTURE`], so the recent pass of a regular-kind read plans
/// only the previous and current monthly partitions (and the next one in the
/// last hour of a month).
pub(crate) fn ceiling_at(now: DateTime<Utc>) -> DateTime<Utc> {
    month_start(now + MAX_FUTURE, 1)
}

/// Where to split a page read, or `None` to read it whole: its own bounds
/// already exclude everything newer than the floor, or its rows are
/// latest-state records that rarely fill a page above the floor, so a split
/// would only add a second statement. A read whose `since` is at or above the
/// floor gets a recent pass with no older pass after it (see the module
/// docs), whatever its kinds. The recent pass has a ceiling only when every
/// kind is regular (see [`MAX_FUTURE`]).
///
/// `kinds` are the read's kinds (`None`: any); `since`/`until` its bounds.
pub(crate) fn split_for(
    now: DateTime<Utc>,
    kinds: Option<&[i32]>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> Option<Split> {
    let floor = floor_at(now);
    // Replaceable and addressable rows are latest-state records, often months
    // old, and few per author.
    let latest_state = |k: i32| {
        u32::try_from(k).is_ok_and(|k| is_replaceable(k) || is_parameterized_replaceable(k))
    };
    let kinds = kinds.filter(|ks| !ks.is_empty());
    let latest_state_only = kinds.is_some_and(|ks| ks.iter().all(|&k| latest_state(k)));
    let regular_only = kinds.is_some_and(|ks| !ks.iter().any(|&k| latest_state(k)));
    let since_prunes = since.is_some_and(|s| s >= floor);
    let until_before_floor = until.is_some_and(|u| u < floor);
    let split = |older| Split {
        floor,
        ceiling: regular_only.then(|| ceiling_at(now)),
        older,
    };
    if since_prunes {
        return (!until_before_floor).then(|| split(false));
    }
    (!latest_state_only && !until_before_floor).then(|| split(true))
}

/// The pass to run after `range` once the read holds `found` of its `limit`
/// rows, with that pass's limit: the older pass for the rows still missing
/// after a short recent pass, otherwise none. `limit` is the SQL `LIMIT`.
pub(crate) fn next(range: Range, found: usize, limit: i64) -> Option<(Range, i64)> {
    let Range::Recent(split) = range else {
        return None;
    };
    let found = i64::try_from(found).unwrap_or(i64::MAX);
    (split.older && found < limit).then(|| (Range::Older(split.floor), limit - found))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 34, 56).single().unwrap()
    }

    fn midnight(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).single().unwrap()
    }

    #[test]
    fn floor_is_the_start_of_the_previous_month() {
        assert_eq!(floor_at(at(2026, 11, 4)), midnight(2026, 10, 1));
        assert_eq!(floor_at(at(2026, 11, 30)), midnight(2026, 10, 1));
        assert_eq!(floor_at(at(2027, 1, 1)), midnight(2026, 12, 1));
        assert_eq!(floor_at(at(2026, 3, 31)), midnight(2026, 2, 1));
    }

    /// The ceiling is the start of the next month, or of the one after once
    /// `now` + [`MAX_FUTURE`] reaches the next month: a row the relay can
    /// still accept is always below it.
    #[test]
    fn ceiling_is_the_start_of_the_next_month_after_max_future() {
        let ceiling = ceiling_at;
        assert_eq!(ceiling(at(2026, 11, 4)), midnight(2026, 12, 1));
        assert_eq!(ceiling(midnight(2026, 11, 1)), midnight(2026, 12, 1));
        assert_eq!(ceiling(at(2026, 12, 31)), midnight(2027, 1, 1));
        let month_end = midnight(2026, 12, 1);
        let second = Duration::seconds(1);
        assert_eq!(
            ceiling(month_end - MAX_FUTURE - second),
            month_end,
            "before the margin"
        );
        assert_eq!(
            ceiling(month_end - MAX_FUTURE),
            midnight(2027, 1, 1),
            "inside the margin"
        );
        assert_eq!(
            ceiling(midnight(2026, 12, 31) + Duration::hours(23)),
            midnight(2027, 2, 1)
        );
        for now in [at(2026, 11, 4), month_end - second, midnight(2026, 12, 31)] {
            assert!(ceiling(now) > now + MAX_FUTURE, "{now}");
        }
    }

    #[test]
    fn predicates_are_fixed_utc_literals() {
        let floor = midnight(2026, 10, 1);
        let split = Split {
            floor,
            ceiling: Some(midnight(2026, 12, 1)),
            older: true,
        };
        assert_eq!(Range::All.predicate("e.created_at"), None);
        assert_eq!(
            Range::Recent(split).predicate("e.created_at").as_deref(),
            Some(
                " AND e.created_at >= '2026-10-01T00:00:00Z'::timestamptz \
                 AND e.created_at < '2026-12-01T00:00:00Z'::timestamptz"
            )
        );
        let open = Split {
            ceiling: None,
            ..split
        };
        assert_eq!(
            Range::Recent(open).predicate("e.created_at").as_deref(),
            Some(" AND e.created_at >= '2026-10-01T00:00:00Z'::timestamptz")
        );
        assert_eq!(
            Range::Older(floor).predicate("created_at").as_deref(),
            Some(" AND created_at < '2026-10-01T00:00:00Z'::timestamptz")
        );
    }

    /// Which reads are split, which recent passes stop at the ceiling, and
    /// which are the whole read: (kinds, since, until) → `WHOLE` (not split),
    /// `CEILED` or `OPEN` (recent pass, then the older pass if short), or
    /// `CEILED_ONLY` or `OPEN_ONLY` (recent pass, no older pass).
    #[test]
    fn split_eligibility() {
        // (ceiled, older)
        const WHOLE: Option<(bool, bool)> = None;
        const CEILED: Option<(bool, bool)> = Some((true, true));
        const OPEN: Option<(bool, bool)> = Some((false, true));
        const CEILED_ONLY: Option<(bool, bool)> = Some((true, false));
        const OPEN_ONLY: Option<(bool, bool)> = Some((false, false));
        let now = at(2026, 11, 4);
        let floor = floor_at(now);
        let day = Duration::days(1);
        // (label, kinds, since, until, outcome)
        type Case<'a> = (
            &'a str,
            Option<&'a [i32]>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<(bool, bool)>,
        );
        let cases: &[Case] = &[
            ("unbounded messages", Some(&[9, 40002]), None, None, CEILED),
            ("any kind", None, None, None, OPEN),
            ("no kinds listed", Some(&[]), None, None, OPEN),
            (
                "since older than the floor",
                Some(&[9]),
                Some(floor - day),
                None,
                CEILED,
            ),
            (
                "since at the floor",
                Some(&[9]),
                Some(floor),
                None,
                CEILED_ONLY,
            ),
            (
                "recent since",
                Some(&[9]),
                Some(now - day),
                None,
                CEILED_ONLY,
            ),
            (
                "recent since, any kind",
                None,
                Some(now - day),
                None,
                OPEN_ONLY,
            ),
            (
                "recent since, addressable only",
                Some(&[39002]),
                Some(now - day),
                None,
                OPEN_ONLY,
            ),
            (
                "recent since, until before the floor",
                Some(&[9]),
                Some(now - day),
                Some(floor - day),
                WHOLE,
            ),
            ("until at the floor", Some(&[9]), None, Some(floor), CEILED),
            (
                "until before the floor",
                Some(&[9]),
                None,
                Some(floor - day),
                WHOLE,
            ),
            ("replaceable only", Some(&[0, 10002]), None, None, WHOLE),
            ("addressable only", Some(&[39002]), None, None, WHOLE),
            (
                "mixed with an addressable kind",
                Some(&[39002, 9]),
                None,
                None,
                OPEN,
            ),
            (
                "mixed with a replaceable kind",
                Some(&[9, 13534]),
                None,
                None,
                OPEN,
            ),
            (
                "negative kind is not replaceable",
                Some(&[-1]),
                None,
                None,
                CEILED,
            ),
        ];
        for &(label, kinds, since, until, outcome) in cases {
            let expected = outcome.map(|(ceiled, older)| Split {
                floor,
                ceiling: ceiled.then(|| ceiling_at(now)),
                older,
            });
            assert_eq!(split_for(now, kinds, since, until), expected, "{label}");
        }
    }

    /// The passes against an in-memory table of `created_at` values, newest
    /// first, driven the way the stores drive them: the newest `limit` rows,
    /// in order, for every limit, split or not, reading each row at most once.
    #[test]
    fn passes_return_the_newest_rows_and_read_each_row_once() {
        let day = Duration::days(1);
        let floor = midnight(2026, 10, 1);
        let ceiling = midnight(2026, 12, 1);
        // Four at or above the floor (one exactly at it, one a second below
        // the ceiling), two below. No row sits at or above the ceiling.
        let table = [
            ceiling - Duration::seconds(1),
            floor + day * 3,
            floor + day,
            floor,
            floor - day,
            floor - day * 9,
        ];
        let recent = 4;
        for limit in 0..=table.len() as i64 + 1 {
            let expected: Vec<_> = table.iter().copied().take(limit as usize).collect();
            let ceiled = Split {
                floor,
                ceiling: Some(ceiling),
                older: true,
            };
            let open = Split {
                ceiling: None,
                ..ceiled
            };
            // A read whose `since` is at or above the floor: only the rows
            // at or above it match.
            let since_floor = Split {
                older: false,
                ..ceiled
            };
            for split in [None, Some(ceiled), Some(open), Some(since_floor)] {
                let expected: Vec<_> = match split {
                    Some(s) if !s.older => {
                        expected.iter().copied().filter(|&t| t >= floor).collect()
                    }
                    _ => expected.clone(),
                };
                let mut passes = Vec::new();
                let mut rows = Vec::new();
                let mut pass = Some((split.map_or(Range::All, Range::Recent), limit));
                while let Some((range, n)) = pass {
                    passes.push((range, n));
                    rows.extend(
                        table
                            .iter()
                            .copied()
                            .filter(|&t| match range {
                                Range::All => true,
                                Range::Recent(s) => s.floor <= t && s.ceiling.is_none_or(|c| t < c),
                                Range::Older(f) => t < f,
                            })
                            .take(n as usize),
                    );
                    pass = next(range, rows.len(), limit);
                }
                assert_eq!(rows, expected, "split {split:?}, limit {limit}");
                let expected_passes = match split {
                    None => vec![(Range::All, limit)],
                    Some(s) if limit <= recent || !s.older => vec![(Range::Recent(s), limit)],
                    Some(s) => vec![
                        (Range::Recent(s), limit),
                        (Range::Older(s.floor), limit - recent),
                    ],
                };
                assert_eq!(passes, expected_passes, "split {split:?}, limit {limit}");
            }
        }
    }
}
