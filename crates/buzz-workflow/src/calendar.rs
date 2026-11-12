//! Anchored calendar recurrence. The relay's build pins the timezone data.

use chrono::{DateTime, Days, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::{error::WorkflowError, schema::CalendarSchedule};

impl CalendarSchedule {
    pub(crate) fn parse(&self) -> Result<(NaiveDate, NaiveTime, Tz), WorkflowError> {
        let invalid = |message: &str| WorkflowError::InvalidDefinition(message.into());
        if self.every_weeks != 2 {
            return Err(invalid("calendar.every_weeks must be 2"));
        }
        let date = NaiveDate::parse_from_str(&self.start_date, "%Y-%m-%d")
            .map_err(|_| invalid("calendar.start_date must be a real YYYY-MM-DD date"))?;
        if self.start_date.len() != 10 || date.format("%Y-%m-%d").to_string() != self.start_date {
            return Err(invalid("calendar.start_date must use YYYY-MM-DD"));
        }
        let time = NaiveTime::parse_from_str(&self.time, "%H:%M")
            .map_err(|_| invalid("calendar.time must use HH:MM"))?;
        if time.format("%H:%M").to_string() != self.time {
            return Err(invalid("calendar.time must use HH:MM"));
        }
        let zone = self.timezone.parse::<Tz>().map_err(|_| {
            invalid("calendar.timezone must be a supported case-sensitive IANA name")
        })?;
        Ok((date, time, zone))
    }

    /// Match the existing open-lower-bound 60-second admission window, not a backlog.
    pub(crate) fn fire_instant(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, WorkflowError> {
        let (anchor, time, zone) = self.parse()?;
        let Some(start) = now.checked_sub_signed(Duration::seconds(60)) else {
            return Ok(None);
        };
        // IANA offsets are less than one day. UTC date bounds padded by a day
        // cover midnight and offset transitions without assuming local dates
        // move monotonically. At most four dates are considered in this window.
        let first = start.date_naive().pred_opt().unwrap_or(start.date_naive());
        let last = now.date_naive().succ_opt().unwrap_or(now.date_naive());
        let elapsed = first.signed_duration_since(anchor).num_days().max(0);
        let weeks = (elapsed + 13) / 14;
        let Some(mut date) = anchor.checked_add_days(Days::new((weeks * 14) as u64)) else {
            return Ok(None);
        };
        while date <= last {
            // earliest() skips gaps and chooses just the first UTC instant of
            // a fold. Neither case changes the next eligible calendar date.
            if let Some(local) = zone.from_local_datetime(&date.and_time(time)).earliest() {
                let instant = local.with_timezone(&Utc);
                if instant > start && instant <= now {
                    return Ok(Some(instant));
                }
            }
            let Some(next) = date.checked_add_days(Days::new(14)) else {
                break;
            };
            date = next;
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn anchored_dates_and_local_wall_times_survive_calendar_boundaries() {
        for (anchor, time, zone, due) in [
            ("2026-12-22", "18:00", "UTC", "2027-01-05T18:00:00Z"),
            ("2020-12-22", "18:00", "UTC", "2021-01-05T18:00:00Z"),
            ("2024-02-29", "09:05", "UTC", "2024-03-14T09:05:00Z"),
            (
                "2026-11-18",
                "13:35",
                "America/New_York",
                "2027-03-10T18:35:00Z",
            ),
            (
                "2026-11-18",
                "13:35",
                "America/New_York",
                "2027-03-24T17:35:00Z",
            ),
            (
                "2026-11-18",
                "13:35",
                "America/New_York",
                "2027-11-03T17:35:00Z",
            ),
            (
                "2026-11-18",
                "13:35",
                "America/New_York",
                "2027-11-17T18:35:00Z",
            ),
            (
                "2026-11-24",
                "09:05",
                "Australia/Melbourne",
                "2027-03-29T22:05:00Z",
            ),
            (
                "2026-11-24",
                "09:05",
                "Australia/Melbourne",
                "2027-04-12T23:05:00Z",
            ),
            (
                "2026-11-24",
                "09:05",
                "Australia/Melbourne",
                "2027-09-27T23:05:00Z",
            ),
            (
                "2026-11-24",
                "09:05",
                "Australia/Melbourne",
                "2027-10-11T22:05:00Z",
            ),
            ("2000-01-01", "00:00", "Etc/GMT-12", "2026-12-25T12:00:00Z"),
        ] {
            let calendar = CalendarSchedule {
                every_weeks: 2,
                start_date: anchor.into(),
                time: time.into(),
                timezone: zone.into(),
            };
            let due = utc(due);
            for seconds in [0, 1, 59] {
                assert_eq!(
                    calendar
                        .fire_instant(due + Duration::seconds(seconds))
                        .unwrap(),
                    Some(due),
                    "{calendar:?}"
                );
            }
            for seconds in [-1, 60, 61, 7 * 86400] {
                assert_eq!(
                    calendar
                        .fire_instant(due + Duration::seconds(seconds))
                        .unwrap(),
                    None,
                    "{calendar:?}"
                );
            }
            let first = calendar.parse().unwrap();
            if let Some(instant) = first
                .2
                .from_local_datetime(&first.0.and_time(first.1))
                .earliest()
            {
                assert_eq!(
                    calendar
                        .fire_instant(instant.with_timezone(&Utc) - Duration::days(14))
                        .unwrap(),
                    None
                );
            }
        }
    }

    #[test]
    fn gaps_skip_without_rephasing_and_folds_fire_only_at_the_earlier_instant() {
        for (zone, anchor, time, absent, next) in [
            (
                "America/New_York",
                "2027-03-14",
                "02:30",
                "2027-03-14T07:30:00Z",
                "2027-03-28T06:30:00Z",
            ),
            (
                "Australia/Melbourne",
                "2027-10-03",
                "02:30",
                "2027-10-02T16:30:00Z",
                "2027-10-16T15:30:00Z",
            ),
        ] {
            let calendar = CalendarSchedule {
                every_weeks: 2,
                start_date: anchor.into(),
                time: time.into(),
                timezone: zone.into(),
            };
            assert_eq!(calendar.fire_instant(utc(absent)).unwrap(), None);
            assert_eq!(calendar.fire_instant(utc(next)).unwrap(), Some(utc(next)));
        }
        for (zone, anchor, earlier, later) in [
            (
                "America/New_York",
                "2027-11-07",
                "2027-11-07T05:30:00Z",
                "2027-11-07T06:30:00Z",
            ),
            (
                "Australia/Melbourne",
                "2027-04-04",
                "2027-04-03T15:30:00Z",
                "2027-04-03T16:30:00Z",
            ),
        ] {
            let calendar = CalendarSchedule {
                every_weeks: 2,
                start_date: anchor.into(),
                time: if zone == "America/New_York" {
                    "01:30"
                } else {
                    "02:30"
                }
                .into(),
                timezone: zone.into(),
            };
            assert_eq!(
                calendar.fire_instant(utc(earlier)).unwrap(),
                Some(utc(earlier))
            );
            assert_eq!(calendar.fire_instant(utc(later)).unwrap(), None);
        }
    }
}
