use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

const MANILA_OFFSET_SECONDS: i64 = 8 * 60 * 60;

pub(crate) fn manila_date(now: DateTime<Utc>) -> NaiveDate {
    (now + Duration::seconds(MANILA_OFFSET_SECONDS)).date_naive()
}

pub(crate) fn boundary_for_date(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(
        &date
            .and_hms_opt(1, 0, 0)
            .expect("01:00 is a valid UTC boundary"),
    )
}

pub(crate) fn eligible_boundary(now: DateTime<Utc>) -> Option<(NaiveDate, DateTime<Utc>)> {
    let date = manila_date(now);
    let boundary = boundary_for_date(date);
    (now >= boundary).then_some((date, boundary))
}

pub(crate) fn next_boundary(now: DateTime<Utc>) -> DateTime<Utc> {
    let date = manila_date(now);
    let today = boundary_for_date(date);
    if now < today {
        today
    } else {
        boundary_for_date(date.succ_opt().expect("supported date range"))
    }
}

pub(crate) fn monday_for(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syn79_daily_boundary_is_fixed_to_manila_nine_am() {
        let before = DateTime::parse_from_rfc3339("2026-09-02T00:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(eligible_boundary(before).is_none());
        let at = DateTime::parse_from_rfc3339("2026-09-02T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (date, boundary) = eligible_boundary(at).unwrap();
        assert_eq!(date.to_string(), "2026-09-02");
        assert_eq!(boundary.to_rfc3339(), "2026-09-02T01:00:00+00:00");
    }
}
