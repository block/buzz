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
//! the newest partitions, so it is split at a floor ([`floor_for`], [`next`]),
//! the start of the previous UTC month:
//!
//! 1. The *recent* pass reads `created_at >= floor`. When it returns `n` rows,
//!    they are the page: every row at or above the floor sorts before every
//!    row below it.
//! 2. Otherwise the *older* pass reads `created_at < floor` for the rows
//!    still missing, and they follow the recent rows. The two ranges are
//!    disjoint, so no row is read twice and the page is exact, provided both
//!    passes read one snapshot (the stores run them in one `REPEATABLE READ`
//!    transaction, `ReadTransaction`).
//!
//! The floor is a SQL literal, so Postgres prunes when it plans the statement,
//! cached generic plans included. It changes once a month, so the
//! prepared-statement cache keeps one statement per pass and shape.

use buzz_core::kind::{is_parameterized_replaceable, is_replaceable};
use chrono::{DateTime, Datelike, TimeZone, Utc};

/// The `created_at` range one pass of a page read covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Range {
    /// Every row: the read is not split.
    All,
    /// `created_at >= floor`.
    Recent(DateTime<Utc>),
    /// `created_at < floor`.
    Older(DateTime<Utc>),
}

impl Range {
    /// The range as a SQL predicate on `column` (` AND …`), or `None` for
    /// [`Range::All`].
    ///
    /// The floor is a literal, not a bind: Postgres prunes partitions on a
    /// literal while it plans, including in a cached generic plan, while a
    /// generic plan with a bind parameter prunes only at execution, after it
    /// has locked every partition. The text is chrono's fixed-format
    /// rendering of a UTC timestamp, digits and separators only, so it
    /// carries no caller input.
    pub(crate) fn predicate(self, column: &str) -> Option<String> {
        let (op, floor) = match self {
            Range::All => return None,
            Range::Recent(floor) => (">=", floor),
            Range::Older(floor) => ("<", floor),
        };
        Some(format!(
            " AND {column} {op} '{}'::timestamptz",
            floor.format("%Y-%m-%dT%H:%M:%SZ")
        ))
    }
}

/// The split point at `now`: 00:00 UTC on the first day of the previous
/// month, so the recent pass reads the current and previous monthly
/// partitions and the ones ahead of them.
pub(crate) fn floor_at(now: DateTime<Utc>) -> DateTime<Utc> {
    let (year, month) = match now.month() {
        1 => (now.year() - 1, 12),
        m => (now.year(), m - 1),
    };
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// The floor to split a page read at, or `None` to read it whole: its own
/// bounds already exclude everything older than the floor or everything
/// newer, or its rows are latest-state records that rarely fill a page above
/// the floor, so a split would only add a second statement.
///
/// `kinds` are the read's kinds (`None`: any); `since`/`until` its bounds.
pub(crate) fn floor_for(
    now: DateTime<Utc>,
    kinds: Option<&[i32]>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    let floor = floor_at(now);
    // Replaceable and addressable rows are latest-state records, often months
    // old, and few per author.
    let latest_state_only = kinds.is_some_and(|ks| {
        !ks.is_empty()
            && ks.iter().all(|&k| {
                u32::try_from(k).is_ok_and(|k| is_replaceable(k) || is_parameterized_replaceable(k))
            })
    });
    let since_prunes = since.is_some_and(|s| s >= floor);
    let until_before_floor = until.is_some_and(|u| u < floor);
    (!latest_state_only && !since_prunes && !until_before_floor).then_some(floor)
}

/// The pass to run after `range` once the read holds `found` of its `limit`
/// rows, with that pass's limit: the older pass for the rows still missing
/// after a short recent pass, otherwise none. `limit` is the SQL `LIMIT`.
pub(crate) fn next(range: Range, found: usize, limit: i64) -> Option<(Range, i64)> {
    let Range::Recent(floor) = range else {
        return None;
    };
    let found = i64::try_from(found).unwrap_or(i64::MAX);
    (found < limit).then(|| (Range::Older(floor), limit - found))
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

    #[test]
    fn predicates_are_fixed_utc_literals() {
        let floor = midnight(2026, 10, 1);
        assert_eq!(Range::All.predicate("e.created_at"), None);
        assert_eq!(
            Range::Recent(floor).predicate("e.created_at").as_deref(),
            Some(" AND e.created_at >= '2026-10-01T00:00:00Z'::timestamptz")
        );
        assert_eq!(
            Range::Older(floor).predicate("created_at").as_deref(),
            Some(" AND created_at < '2026-10-01T00:00:00Z'::timestamptz")
        );
    }

    /// Which reads are split: (kinds, since, until) → split?
    #[test]
    fn split_eligibility() {
        let now = at(2026, 11, 4);
        let floor = floor_at(now);
        let day = chrono::Duration::days(1);
        // (label, kinds, since, until, split)
        type Case<'a> = (
            &'a str,
            Option<&'a [i32]>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            bool,
        );
        let cases: &[Case] = &[
            ("unbounded messages", Some(&[9, 40002]), None, None, true),
            ("any kind", None, None, None, true),
            (
                "since older than the floor",
                Some(&[9]),
                Some(floor - day),
                None,
                true,
            ),
            (
                "since at the floor already prunes",
                Some(&[9]),
                Some(floor),
                None,
                false,
            ),
            (
                "recent since already prunes",
                Some(&[9]),
                Some(now - day),
                None,
                false,
            ),
            ("until at the floor", Some(&[9]), None, Some(floor), true),
            (
                "until before the floor",
                Some(&[9]),
                None,
                Some(floor - day),
                false,
            ),
            ("replaceable only", Some(&[0, 10002]), None, None, false),
            ("addressable only", Some(&[39002]), None, None, false),
            (
                "mixed with a message kind",
                Some(&[39002, 9]),
                None,
                None,
                true,
            ),
            (
                "negative kind is not replaceable",
                Some(&[-1]),
                None,
                None,
                true,
            ),
        ];
        for &(label, kinds, since, until, split) in cases {
            let expected = split.then_some(floor);
            assert_eq!(floor_for(now, kinds, since, until), expected, "{label}");
        }
    }

    /// The passes against an in-memory table of `created_at` values, newest
    /// first, driven the way the stores drive them: the newest `limit` rows,
    /// in order, for every limit, split or not, reading each row at most once.
    #[test]
    fn passes_return_the_newest_rows_and_read_each_row_once() {
        let day = chrono::Duration::days(1);
        let floor = midnight(2026, 10, 1);
        // Three at or above the floor (one exactly at it), two below.
        let table = [
            floor + day * 3,
            floor + day,
            floor,
            floor - day,
            floor - day * 9,
        ];
        for limit in 0..=table.len() as i64 + 1 {
            let expected: Vec<_> = table.iter().copied().take(limit as usize).collect();
            for split in [None, Some(floor)] {
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
                                Range::Recent(f) => t >= f,
                                Range::Older(f) => t < f,
                            })
                            .take(n as usize),
                    );
                    pass = next(range, rows.len(), limit);
                }
                assert_eq!(rows, expected, "split {split:?}, limit {limit}");
                let expected_passes = match split {
                    None => vec![(Range::All, limit)],
                    Some(f) if limit <= 3 => vec![(Range::Recent(f), limit)],
                    Some(f) => vec![(Range::Recent(f), limit), (Range::Older(f), limit - 3)],
                };
                assert_eq!(passes, expected_passes, "split {split:?}, limit {limit}");
            }
        }
    }
}
