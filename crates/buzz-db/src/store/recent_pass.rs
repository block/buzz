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
//!
//! An oldest-first page (`ORDER BY created_at ASC … LIMIT n`) is the mirror
//! image ([`oldest_for`], [`Forward::advance`]), and it never reads more
//! partitions than it needs. It walks fixed two-month windows forward from a
//! literal floor, the start of the month of the read's earliest possible row,
//! and stops at the first window that completes the page. A regular-kind read
//! stops at the same ceiling; any other read ends with one open window. A
//! channel read starts at the month of [`channel_floor`] ([`oldest_first`]):
//! a page whose cursor is below that month first reads every row below it in
//! one pass ([`Range::Below`]), since a restore or backfill can store rows
//! older than the channel, then walks from it. A later cursor walks from its
//! own month. A page reads at most that pass and one window per two months of
//! the channel's life. Without the channel's row a page walks from its cursor,
//! and a read with neither plans every partition below the ceiling in one
//! window.
//!
//! The walk locks one more partition set per window it reads, and the locks
//! stay until the read's transaction ends, so a read that must reach the
//! newest rows (a channel with fewer rows than the page) still locks every
//! partition from its floor on. The floor follows the channel's creation
//! month and the cursor's month, so the cache keeps one statement per
//! window, shape and month read.

use buzz_core::kind::{is_parameterized_replaceable, is_replaceable};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};

use crate::replica_fence::CREATED_AT_FLOOR_SECS;

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

/// Months one window of an oldest-first walk covers.
const WINDOW_MONTHS: i32 = 2;

/// One window of an oldest-first page read: see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Forward {
    /// The window reads at or above it. `None` for a read with no known
    /// earliest row, which is one window.
    pub(crate) from: Option<DateTime<Utc>>,
    /// The window reads below it. `None` when the window is open ended.
    pub(crate) to: Option<DateTime<Utc>>,
    /// Where the walk of fixed windows ends: [`ceiling_at`] at the read's
    /// `now`.
    horizon: DateTime<Utc>,
    /// Whether no row of the read sits at or above the horizon (every kind
    /// is regular): the walk stops there. Otherwise one open window follows.
    closed: bool,
}

impl Forward {
    /// The window starting at `from`, which ends [`WINDOW_MONTHS`] later, at
    /// the horizon, or never, whichever comes first.
    fn from(from: Option<DateTime<Utc>>, horizon: DateTime<Utc>, closed: bool) -> Self {
        let to = match from {
            None => closed.then_some(horizon),
            Some(from) => {
                let to = month_start(from, WINDOW_MONTHS);
                if to < horizon {
                    Some(to)
                } else {
                    closed.then_some(horizon)
                }
            }
        };
        Forward {
            from,
            to,
            horizon,
            closed,
        }
    }

    /// The window that follows this one, or `None` when this one reached the
    /// end of the read: it is open ended, or it ended at the horizon of a
    /// closed read.
    pub(crate) fn advance(self) -> Option<Forward> {
        let to = self.to.filter(|to| *to < self.horizon)?;
        Some(Forward::from(Some(to), self.horizon, self.closed))
    }
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
    /// One window of an oldest-first walk.
    Forward(Forward),
    /// `created_at <` the window's start, read before the walk from that
    /// window.
    Below(Forward),
}

impl Range {
    /// The range as a SQL predicate on `column` (` AND …`), or `None` for
    /// [`Range::All`].
    ///
    /// The bounds are literals, not binds: Postgres prunes partitions on a
    /// literal while it plans, including in a cached generic plan, while a
    /// generic plan with a bind parameter prunes only at execution, after it
    /// has locked every partition. The text is chrono's fixed-format
    /// rendering of a UTC month start, digits and separators only. A forward
    /// window derives from the caller's cursor or channel, clamped to the
    /// ceiling at `now` (see [`oldest_for`]), so it is still a four-digit-year
    /// month.
    pub(crate) fn predicate(self, column: &str) -> Option<String> {
        let (from, to) = match self {
            Range::All => (None, None),
            Range::Recent(split) => (Some(split.floor), split.ceiling),
            Range::Older(floor) => (None, Some(floor)),
            Range::Forward(window) => (window.from, window.to),
            Range::Below(window) => (None, window.from),
        };
        let bound = |op: &str, at: Option<DateTime<Utc>>| {
            at.map(|at| {
                format!(
                    " AND {column} {op} '{}'::timestamptz",
                    at.format("%Y-%m-%dT%H:%M:%SZ")
                )
            })
            .unwrap_or_default()
        };
        let predicate = bound(">=", from) + &bound("<", to);
        (!predicate.is_empty()).then_some(predicate)
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
    let latest_state_only = kinds
        .filter(|ks| !ks.is_empty())
        .is_some_and(|ks| ks.iter().all(|&k| latest_state(k)));
    let regular_only = regular_only(kinds);
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

/// Replaceable and addressable rows are latest-state records, often months
/// old, and few per author.
fn latest_state(kind: i32) -> bool {
    u32::try_from(kind).is_ok_and(|k| is_replaceable(k) || is_parameterized_replaceable(k))
}

/// Whether every one of a read's kinds is regular, so its rows lie below
/// [`ceiling_at`].
fn regular_only(kinds: Option<&[i32]>) -> bool {
    kinds
        .filter(|ks| !ks.is_empty())
        .is_some_and(|ks| !ks.iter().any(|&k| latest_state(k)))
}

/// The channel's earliest possible row: the instant it was created, less the
/// floor the write path tolerates ([`CREATED_AT_FLOOR_SECS`]). `created_at` is
/// the database's `NOW()` when the channel row was inserted
/// (`schema/tables/public/channels.sql`), every write of a channel row checks
/// its `created_at` against the database clock, and the channel exists before
/// its first row, so no write the relay serves stores an older row. A restore
/// or backfill that bypasses that check (see [`crate::replica_fence`]) can,
/// so an oldest-first channel page reads below the floor's month in one pass
/// and walks from it ([`oldest_first`]).
pub(crate) fn channel_floor(channel_created_at: DateTime<Utc>) -> DateTime<Utc> {
    channel_created_at - Duration::seconds(CREATED_AT_FLOOR_SECS)
}

/// The first window of an oldest-first page read. `earliest` is a lower bound
/// on the read's rows: [`channel_floor`], or the page's cursor when that is
/// later or there is no floor (`None`: the read has neither, and its first
/// window is unbounded below).
/// `kinds` are the read's kinds (`None`: any).
pub(crate) fn oldest_for(
    now: DateTime<Utc>,
    kinds: Option<&[i32]>,
    earliest: Option<DateTime<Utc>>,
) -> Forward {
    let horizon = ceiling_at(now);
    // Clamped so a far-future bound keeps a literal Postgres accepts; any
    // floor at or below the bound is exact.
    let floor = earliest.map(|at| month_start(at.min(horizon), 0));
    Forward::from(floor, horizon, regular_only(kinds))
}

/// The first pass of an oldest-first channel page read. `floor` is the
/// channel's [`channel_floor`] (`None`: its row is not visible); `cursor`
/// the page's cursor (`None`: the head page). A page whose cursor is below
/// the floor's month reads every row below that month first, then walks from
/// it; a later cursor walks from its own month. Without a floor a page walks
/// from its cursor.
pub(crate) fn oldest_first(
    now: DateTime<Utc>,
    kinds: Option<&[i32]>,
    floor: Option<DateTime<Utc>>,
    cursor: Option<DateTime<Utc>>,
) -> Range {
    match floor.map(|floor| oldest_for(now, kinds, Some(floor))) {
        Some(walk)
            if walk
                .from
                .is_some_and(|from| cursor.is_none_or(|ts| ts < from)) =>
        {
            Range::Below(walk)
        }
        _ => Range::Forward(oldest_for(now, kinds, cursor)),
    }
}

/// The pass to run after `range` once the read holds `found` of its `limit`
/// rows, with that pass's limit: the older pass after a short recent pass,
/// the next window after a short window, or the walk after a short pass
/// below it, for the rows still missing; otherwise none. `limit` is the SQL
/// `LIMIT`.
pub(crate) fn next(range: Range, found: usize, limit: i64) -> Option<(Range, i64)> {
    let rest = match range {
        Range::Recent(split) if split.older => Range::Older(split.floor),
        Range::Forward(window) => Range::Forward(window.advance()?),
        Range::Below(window) => Range::Forward(window),
        _ => return None,
    };
    let found = i64::try_from(found).unwrap_or(i64::MAX);
    (found < limit).then(|| (rest, limit - found))
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
                                Range::Forward(_) | Range::Below(_) => unreachable!(),
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

    /// The first window of an oldest-first read starts in the month of its
    /// earliest possible row and covers two months, unless that reaches the
    /// horizon: a regular-kind read ends there, any other read stays open. A
    /// far-future bound's floor never passes the horizon, so it stays a
    /// literal Postgres parses. A read with no bound is one window.
    #[test]
    fn oldest_window_follows_the_earliest_month() {
        let now = at(2026, 11, 4);
        let horizon = ceiling_at(now);
        assert_eq!(horizon, midnight(2026, 12, 1));
        let far = DateTime::<Utc>::MAX_UTC;
        // (label, kinds, earliest, from, to)
        type Case<'a> = (
            &'a str,
            Option<&'a [i32]>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        );
        let cases: &[Case] = &[
            ("no bound", Some(&[9]), None, None, Some(horizon)),
            ("no bound, any kind", None, None, None, None),
            (
                "old bound",
                Some(&[9]),
                Some(at(2025, 6, 20)),
                Some(midnight(2025, 6, 1)),
                Some(midnight(2025, 8, 1)),
            ),
            (
                "bound last month",
                Some(&[9]),
                Some(at(2026, 10, 9)),
                Some(midnight(2026, 10, 1)),
                Some(horizon),
            ),
            (
                "bound last month, any kind",
                None,
                Some(at(2026, 10, 9)),
                Some(midnight(2026, 10, 1)),
                None,
            ),
            (
                "bound in the last window month",
                Some(&[9]),
                Some(at(2026, 11, 2)),
                Some(midnight(2026, 11, 1)),
                Some(horizon),
            ),
            (
                "far-future bound",
                Some(&[9]),
                Some(far),
                Some(horizon),
                Some(horizon),
            ),
            (
                "far-future bound, any kind",
                None,
                Some(far),
                Some(horizon),
                None,
            ),
            (
                "channel floor",
                Some(&[9]),
                Some(channel_floor(at(2026, 3, 1))),
                Some(midnight(2026, 3, 1)),
                Some(midnight(2026, 5, 1)),
            ),
        ];
        for &(label, kinds, earliest, from, to) in cases {
            let window = oldest_for(now, kinds, earliest);
            assert_eq!((window.from, window.to), (from, to), "{label}");
        }
        let far = oldest_for(now, None, Some(far));
        assert_eq!(
            Range::Forward(far).predicate("created_at").as_deref(),
            Some(" AND created_at >= '2026-12-01T00:00:00Z'::timestamptz")
        );
        let old = oldest_for(now, Some(&[9]), Some(at(2025, 6, 20)));
        assert_eq!(
            Range::Forward(old).predicate("created_at").as_deref(),
            Some(
                " AND created_at >= '2025-06-01T00:00:00Z'::timestamptz \
                 AND created_at < '2025-08-01T00:00:00Z'::timestamptz"
            )
        );
        assert_eq!(
            Range::Forward(oldest_for(now, None, None))
                .predicate("created_at")
                .as_deref(),
            None
        );
    }

    /// A channel's floor is its creation less the write path's tolerance, so
    /// a row stamped just before the channel existed stays in its first window.
    #[test]
    fn channel_floor_allows_the_write_tolerance() {
        let created = midnight(2026, 3, 1) + Duration::seconds(10);
        assert_eq!(
            channel_floor(created),
            midnight(2026, 3, 1) + Duration::seconds(10 - CREATED_AT_FLOOR_SECS)
        );
        let now = at(2026, 11, 4);
        let window = oldest_for(now, Some(&[9]), Some(channel_floor(created)));
        assert_eq!(window.from, Some(midnight(2026, 2, 1)));
    }

    /// A channel page whose cursor is below its floor's month, the head page
    /// included, reads every row below that month in one pass and then walks
    /// from it. A later cursor walks from its own month, and so does a page
    /// without a floor.
    #[test]
    fn oldest_first_reads_below_the_floor_month_in_one_pass() {
        let now = at(2026, 11, 4);
        let kinds = Some(&[9][..]);
        let floor = Some(channel_floor(midnight(2026, 3, 1) + Duration::seconds(10)));
        let walk = oldest_for(now, kinds, floor);
        assert_eq!(walk.from, Some(midnight(2026, 2, 1)));
        for cursor in [
            None,
            Some(DateTime::<Utc>::UNIX_EPOCH),
            Some(at(2026, 1, 31)),
        ] {
            let first = oldest_first(now, kinds, floor, cursor);
            assert_eq!(first, Range::Below(walk), "{cursor:?}");
            assert_eq!(
                first.predicate("created_at").as_deref(),
                Some(" AND created_at < '2026-02-01T00:00:00Z'::timestamptz")
            );
            assert_eq!(next(first, 2, 5), Some((Range::Forward(walk), 3)));
            assert_eq!(next(first, 5, 5), None);
        }
        for (floor, cursor) in [
            (floor, Some(midnight(2026, 2, 1))),
            (floor, Some(at(2026, 6, 20))),
            (None, Some(at(2026, 6, 20))),
            (None, None),
        ] {
            assert_eq!(
                oldest_first(now, kinds, floor, cursor),
                Range::Forward(oldest_for(now, kinds, cursor)),
                "{floor:?}, {cursor:?}"
            );
        }
    }

    /// The oldest-first windows against an in-memory table, driven the way
    /// the stores drive them: the oldest `limit` rows at or after the
    /// bound, in order, for every limit, every kind shape and several bounds,
    /// reading each row at most once and no window past the one that fills
    /// the page.
    #[test]
    fn oldest_windows_return_the_oldest_rows_and_stop_when_full() {
        let now = at(2026, 11, 4);
        let horizon = ceiling_at(now);
        let after = at(2025, 6, 20);
        // Oldest first: gaps of several windows, one row past the horizon
        // (only an any-kind read can match it).
        let table = [
            after,
            midnight(2025, 7, 31),
            midnight(2025, 8, 1),
            midnight(2026, 1, 9),
            at(2026, 11, 30),
            horizon + Duration::days(40),
        ];
        let regular = Some(&[9][..]);
        for (kinds, earliest) in [
            (regular, Some(after)),
            (None, Some(after)),
            (regular, Some(midnight(2025, 6, 1))),
            (regular, None),
            (None, None),
            (None, Some(DateTime::<Utc>::MAX_UTC)),
        ] {
            let first = oldest_for(now, kinds, earliest);
            let matching: Vec<_> = table
                .iter()
                .copied()
                .filter(|&t| first.from.is_none_or(|f| f <= t))
                .filter(|&t| kinds.is_none() || t < horizon)
                .collect();
            for limit in 0..=table.len() as i64 + 1 {
                let mut windows = Vec::new();
                let mut rows = Vec::new();
                let mut pass = Some((Range::Forward(first), limit));
                while let Some((range, n)) = pass {
                    let Range::Forward(window) = range else {
                        unreachable!("{range:?}")
                    };
                    windows.push(window);
                    rows.extend(
                        table
                            .iter()
                            .copied()
                            .filter(|&t| {
                                window.from.is_none_or(|f| f <= t)
                                    && window.to.is_none_or(|c| t < c)
                            })
                            .take(n as usize),
                    );
                    pass = next(range, rows.len(), limit);
                }
                let expected: Vec<_> = matching.iter().copied().take(limit as usize).collect();
                assert_eq!(
                    rows, expected,
                    "kinds {kinds:?}, from {earliest:?}, limit {limit}"
                );
                // The windows tile the walk: each starts where the last
                // ended, and the last one only exists because the page was
                // still short when the one before it ended.
                for pair in windows.windows(2) {
                    assert_eq!(pair[1].from, pair[0].to, "{kinds:?}, {earliest:?}");
                }
                let last = windows.last().expect("one window");
                if rows.len() as i64 >= limit {
                    assert!(
                        windows.len() == 1
                            || windows[windows.len() - 2].to.is_some_and(|to| matching
                                .iter()
                                .filter(|&&t| t < to)
                                .count()
                                < limit as usize),
                        "a window ran after the page was full: {windows:?}"
                    );
                } else {
                    assert!(
                        last.to.is_none_or(|to| to >= horizon),
                        "the walk ended short of the horizon: {last:?}"
                    );
                }
            }
        }
        // A read with no bound is one window, and so is an open one.
        let head = oldest_for(now, Some(&[9]), None);
        assert_eq!(next(Range::Forward(head), 0, 5), None);
        let open = oldest_for(now, None, Some(DateTime::<Utc>::MAX_UTC));
        assert_eq!(next(Range::Forward(open), 0, 5), None);
    }
}
