use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use rusqlite::Connection;

use super::schedule::{
    acquire_due_claim, deterministic_run_id, due_local_date, idempotency_key,
    load_or_create_schedule, mark_claim_started, next_due_after, process_due_schedule,
    record_deferred, retry_deferred_on_transition, save_schedule_update, ClaimDecision,
    DeferredReason, ReadinessSnapshot, ScheduleRunOutcome, ScheduleTrigger, ScheduleUpdate,
    ScheduledRunPresence, ScheduledRunStarter, ScheduledStartError, DEFAULT_LOCAL_TIME,
    DEFAULT_SCHEDULE_ID, MAX_DEFERRED_RETRIES,
};
use super::store::migrate_command_brief_store;

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("valid time")
        .with_timezone(&Utc)
}

fn migrated_store() -> Connection {
    let conn = Connection::open_in_memory().expect("store");
    migrate_command_brief_store(&conn).expect("migrate");
    conn
}

#[test]
fn default_is_enabled_0600_current_timezone_capacity_one() {
    let conn = migrated_store();
    let schedule =
        load_or_create_schedule(&conn, "Australia/Sydney", 1_722_000_000).expect("schedule");
    assert_eq!(schedule.schedule_id(), DEFAULT_SCHEDULE_ID);
    assert!(schedule.enabled());
    assert_eq!(schedule.local_time(), DEFAULT_LOCAL_TIME);
    assert_eq!(schedule.timezone(), "Australia/Sydney");
    assert!(schedule.catch_up_same_day());
    assert_eq!(schedule.concurrency(), 1);
}

#[test]
fn before_at_after_0600_and_trigger_policy_are_deterministic() {
    let conn = migrated_store();
    let schedule =
        load_or_create_schedule(&conn, "Australia/Sydney", 1_722_000_000).expect("schedule");
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-24T19:59:59Z"),
            ScheduleTrigger::Timer
        ),
        None
    );
    let date = NaiveDate::from_ymd_opt(2026, 7, 25).expect("date");
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-24T20:00:00Z"),
            ScheduleTrigger::Timer
        ),
        Some(date)
    );
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-25T01:00:00Z"),
            ScheduleTrigger::Startup
        ),
        Some(date)
    );
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-25T01:00:00Z"),
            ScheduleTrigger::Wake
        ),
        Some(date)
    );
}

#[test]
fn disabled_and_no_catch_up_never_backfill_on_start_or_resume() {
    let conn = migrated_store();
    let _ = load_or_create_schedule(&conn, "Australia/Sydney", 1_722_000_000).expect("schedule");
    let mut schedule = save_schedule_update(
        &conn,
        ScheduleUpdate {
            enabled: false,
            local_time: "06:00".into(),
            timezone: "Australia/Sydney".into(),
            catch_up_same_day: true,
            concurrency: 1,
        },
        1_722_000_001,
    )
    .expect("disabled");
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-25T01:00:00Z"),
            ScheduleTrigger::Timer
        ),
        None
    );
    schedule = save_schedule_update(
        &conn,
        ScheduleUpdate {
            enabled: true,
            local_time: "06:00".into(),
            timezone: "Australia/Sydney".into(),
            catch_up_same_day: false,
            concurrency: 1,
        },
        1_722_000_002,
    )
    .expect("no catchup");
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-25T01:00:00Z"),
            ScheduleTrigger::Startup
        ),
        None
    );
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-25T01:00:00Z"),
            ScheduleTrigger::Timer
        ),
        None
    );
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-25T01:00:00Z"),
            ScheduleTrigger::Wake
        ),
        None
    );
}

#[test]
fn dst_gap_uses_first_valid_instant_and_fold_uses_earliest() {
    let conn = migrated_store();
    let gap = save_schedule_update(
        &conn,
        ScheduleUpdate {
            enabled: true,
            local_time: "02:30".into(),
            timezone: "America/New_York".into(),
            catch_up_same_day: true,
            concurrency: 1,
        },
        1,
    )
    .expect("gap schedule");
    assert_eq!(
        next_due_after(&gap, utc("2026-03-08T00:00:00Z")).expect("next"),
        utc("2026-03-08T07:00:00Z")
    );

    let fold = save_schedule_update(
        &conn,
        ScheduleUpdate {
            enabled: true,
            local_time: "01:30".into(),
            timezone: "America/New_York".into(),
            catch_up_same_day: true,
            concurrency: 1,
        },
        2,
    )
    .expect("fold schedule");
    assert_eq!(
        next_due_after(&fold, utc("2026-11-01T00:00:00Z")).expect("next"),
        utc("2026-11-01T05:30:00Z")
    );
}

#[test]
fn timezone_change_recomputes_future_due_without_renderer_schedule_id() {
    let conn = migrated_store();
    let _ = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let changed = save_schedule_update(
        &conn,
        ScheduleUpdate {
            enabled: true,
            local_time: "06:00".into(),
            timezone: "Pacific/Auckland".into(),
            catch_up_same_day: true,
            concurrency: 2,
        },
        2,
    )
    .expect("changed");
    assert_eq!(changed.schedule_id(), DEFAULT_SCHEDULE_ID);
    assert_eq!(changed.concurrency(), 2);
    assert_eq!(
        next_due_after(&changed, utc("2026-07-24T00:00:00Z")).expect("next"),
        utc("2026-07-24T18:00:00Z")
    );
}

#[test]
fn invalid_timezone_time_and_concurrency_are_rejected() {
    let conn = migrated_store();
    for update in [
        ScheduleUpdate {
            enabled: true,
            local_time: "24:00".into(),
            timezone: "Australia/Sydney".into(),
            catch_up_same_day: true,
            concurrency: 1,
        },
        ScheduleUpdate {
            enabled: true,
            local_time: "06:00".into(),
            timezone: "Not/AZone".into(),
            catch_up_same_day: true,
            concurrency: 1,
        },
        ScheduleUpdate {
            enabled: true,
            local_time: "06:00".into(),
            timezone: "Australia/Sydney".into(),
            catch_up_same_day: true,
            concurrency: 3,
        },
    ] {
        assert!(save_schedule_update(&conn, update, 1).is_err());
    }
    assert!(Tz::from_str("Australia/Sydney").is_ok());
}

#[test]
fn claim_is_exact_and_survives_restart_duplicate_timer_and_overlap() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("brief.db");
    let conn = super::store::open_command_brief_store(&path).expect("store");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let date = NaiveDate::from_ymd_opt(2026, 7, 25).expect("date");
    assert_eq!(
        idempotency_key(&schedule, date),
        "daily-command-brief:2026-07-25"
    );
    let first = acquire_due_claim(&conn, &schedule, date, 100).expect("claim");
    assert!(matches!(first, ClaimDecision::Acquired(_)));
    assert_eq!(
        acquire_due_claim(&conn, &schedule, date, 101).expect("duplicate"),
        ClaimDecision::AlreadyClaimed
    );
    drop(conn);
    let reopened = super::store::open_command_brief_store(&path).expect("reopen");
    let loaded =
        load_or_create_schedule(&reopened, "Pacific/Auckland", 102).expect("persisted schedule");
    assert_eq!(loaded.timezone(), "Australia/Sydney");
    assert_eq!(
        acquire_due_claim(&reopened, &loaded, date, 103).expect("restart duplicate"),
        ClaimDecision::AlreadyClaimed
    );
}

#[test]
fn claim_persists_the_deterministic_run_identity_before_any_start_side_effect() {
    let conn = migrated_store();
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let date = NaiveDate::from_ymd_opt(2026, 7, 25).expect("date");
    let ClaimDecision::Acquired(claim) =
        acquire_due_claim(&conn, &schedule, date, 100).expect("claim")
    else {
        panic!("fresh claim");
    };
    let expected = deterministic_run_id(claim.idempotency_key());
    let stored: String = conn
        .query_row(
            "SELECT run_id FROM command_brief_schedule_claims
             WHERE idempotency_key=?1",
            [claim.idempotency_key()],
            |row| row.get(0),
        )
        .expect("stored run identity");
    assert_eq!(stored, expected);
}

#[test]
fn concurrent_overlap_acquires_exactly_one_claim() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("brief.db");
    let conn = super::store::open_command_brief_store(&path).expect("store");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    drop(conn);
    let date = NaiveDate::from_ymd_opt(2026, 7, 25).expect("date");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let spawn = |now| {
        let path = path.clone();
        let schedule = schedule.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let conn = super::store::open_command_brief_store(&path).expect("worker");
            barrier.wait();
            acquire_due_claim(&conn, &schedule, date, now).expect("claim")
        })
    };
    let first = spawn(10);
    let second = spawn(11);
    barrier.wait();
    let outcomes = [first.join().expect("first"), second.join().expect("second")];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ClaimDecision::Acquired(_)))
            .count(),
        1
    );
}

#[test]
fn deferred_retries_only_on_distinct_readiness_transitions_and_are_bounded() {
    let conn = migrated_store();
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let date = NaiveDate::from_ymd_opt(2026, 7, 25).expect("date");
    let ClaimDecision::Acquired(claim) =
        acquire_due_claim(&conn, &schedule, date, 100).expect("claim")
    else {
        panic!("fresh claim");
    };
    record_deferred(
        &conn,
        &claim,
        DeferredReason::IdentityLocked,
        "identity:locked|model:ready|local:ready",
        101,
    )
    .expect("defer");
    assert!(!retry_deferred_on_transition(
        &conn,
        &claim,
        "identity:locked|model:ready|local:ready",
        102
    )
    .expect("same transition"));
    for attempt in 0..MAX_DEFERRED_RETRIES {
        assert!(retry_deferred_on_transition(
            &conn,
            &claim,
            &format!("transition-{attempt}"),
            103 + i64::from(attempt)
        )
        .expect("transition"));
        record_deferred(
            &conn,
            &claim,
            DeferredReason::ModelUnavailable,
            &format!("deferred-{attempt}"),
            200 + i64::from(attempt),
        )
        .expect("deferred again");
    }
    assert!(!retry_deferred_on_transition(&conn, &claim, "exhausted", 999).expect("bounded"));
}

#[test]
fn next_day_has_a_new_claim_but_prior_days_are_never_enumerated() {
    let conn = migrated_store();
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let first = NaiveDate::from_ymd_opt(2026, 7, 25).expect("date");
    let second = first.succ_opt().expect("next date");
    let ClaimDecision::Acquired(first_claim) =
        acquire_due_claim(&conn, &schedule, first, 1).expect("first")
    else {
        panic!("claim");
    };
    let first_run = first_claim.run_id().to_string();
    mark_claim_started(&conn, &first_claim, &first_run, 2).expect("started");
    assert!(matches!(
        acquire_due_claim(&conn, &schedule, second, 3).expect("second"),
        ClaimDecision::Acquired(_)
    ));
    assert_eq!(
        due_local_date(
            &schedule,
            utc("2026-07-26T01:00:00Z"),
            ScheduleTrigger::Wake
        ),
        Some(second)
    );
}

struct FakeStarter {
    starts: std::sync::Mutex<Vec<String>>,
    active: std::sync::Mutex<std::collections::BTreeSet<String>>,
    terminal: std::sync::Mutex<std::collections::BTreeSet<String>>,
    fail: bool,
}

impl Default for FakeStarter {
    fn default() -> Self {
        Self {
            starts: std::sync::Mutex::new(Vec::new()),
            active: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            terminal: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            fail: false,
        }
    }
}

impl ScheduledRunStarter for FakeStarter {
    fn start_scheduled(
        &self,
        run_id: &str,
        idempotency_key: &str,
        schedule_id: &str,
        _observed_at: &str,
    ) -> Result<String, ScheduledStartError> {
        if self.fail {
            Err(ScheduledStartError)
        } else {
            assert_eq!(run_id, deterministic_run_id(idempotency_key));
            assert_eq!(schedule_id, DEFAULT_SCHEDULE_ID);
            if self
                .active
                .lock()
                .expect("active")
                .insert(run_id.to_string())
            {
                self.starts.lock().expect("starts").push(run_id.to_string());
            }
            Ok(run_id.to_string())
        }
    }

    fn presence(&self, run_id: &str) -> ScheduledRunPresence {
        if self.terminal.lock().expect("terminal").contains(run_id) {
            ScheduledRunPresence::Terminal
        } else if self.active.lock().expect("active").contains(run_id) {
            ScheduledRunPresence::Active
        } else {
            ScheduledRunPresence::Absent
        }
    }
}

#[test]
fn process_claims_before_start_and_duplicate_timer_never_starts_twice() {
    let conn = migrated_store();
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let starter = FakeStarter::default();
    let now = utc("2026-07-25T01:00:00Z");
    let ready = ReadinessSnapshot::ready("ready-v1");
    assert_eq!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Timer,
            &ready,
            &starter
        )
        .expect("started"),
        ScheduleRunOutcome::Started {
            run_id: deterministic_run_id("daily-command-brief:2026-07-25")
        }
    );
    assert_eq!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Timer,
            &ready,
            &starter
        )
        .expect("duplicate"),
        ScheduleRunOutcome::AlreadyClaimed
    );
    assert_eq!(starter.starts.lock().expect("starts").len(), 1);
}

#[test]
fn crash_boundaries_reconcile_one_deterministic_run_without_loss_or_duplicate_effect() {
    let schedule_time = utc("2026-07-25T01:00:00Z");
    let ready = ReadinessSnapshot::ready("ready-v1");
    let expected_run = deterministic_run_id("daily-command-brief:2026-07-25");

    // Crash after the claim insert but before the starter call.
    let before_start = migrated_store();
    let schedule = load_or_create_schedule(&before_start, "Australia/Sydney", 1).expect("schedule");
    let date = NaiveDate::from_ymd_opt(2026, 7, 25).expect("date");
    assert!(matches!(
        acquire_due_claim(&before_start, &schedule, date, 1).expect("claim"),
        ClaimDecision::Acquired(_)
    ));
    let starter = FakeStarter::default();
    assert_eq!(
        process_due_schedule(
            &before_start,
            &schedule,
            schedule_time,
            ScheduleTrigger::Startup,
            &ready,
            &starter,
        )
        .expect("reconcile"),
        ScheduleRunOutcome::Started {
            run_id: expected_run.clone()
        }
    );
    assert_eq!(starter.starts.lock().expect("starts").len(), 1);

    // Crash after the starter side effect/return but before started is stored.
    let after_start = migrated_store();
    let schedule = load_or_create_schedule(&after_start, "Australia/Sydney", 1).expect("schedule");
    let ClaimDecision::Acquired(claim) =
        acquire_due_claim(&after_start, &schedule, date, 1).expect("claim")
    else {
        panic!("claim");
    };
    let starter = FakeStarter::default();
    starter
        .start_scheduled(
            &expected_run,
            claim.idempotency_key(),
            DEFAULT_SCHEDULE_ID,
            &schedule_time.to_rfc3339(),
        )
        .expect("first start side effect");
    assert!(matches!(
        process_due_schedule(
            &after_start,
            &schedule,
            schedule_time,
            ScheduleTrigger::Timer,
            &ready,
            &starter,
        )
        .expect("reconcile"),
        ScheduleRunOutcome::Started { .. }
    ));
    assert_eq!(starter.starts.lock().expect("starts").len(), 1);

    // Crash after the starter returned and the exact started state was stored.
    // A new process has no in-memory run, but resumes the same logical ID.
    let after_mark = migrated_store();
    let schedule = load_or_create_schedule(&after_mark, "Australia/Sydney", 1).expect("schedule");
    let ClaimDecision::Acquired(claim) =
        acquire_due_claim(&after_mark, &schedule, date, 1).expect("claim")
    else {
        panic!("claim");
    };
    mark_claim_started(&after_mark, &claim, &expected_run, 2).expect("mark started");
    let restarted = FakeStarter::default();
    assert_eq!(
        process_due_schedule(
            &after_mark,
            &schedule,
            schedule_time,
            ScheduleTrigger::Startup,
            &ready,
            &restarted,
        )
        .expect("restart exact run"),
        ScheduleRunOutcome::Started {
            run_id: expected_run.clone()
        }
    );
    assert_eq!(
        restarted.starts.lock().expect("starts").as_slice(),
        std::slice::from_ref(&expected_run)
    );

    // A durable terminal wins on restart and never invokes generation again.
    let terminal = migrated_store();
    let schedule = load_or_create_schedule(&terminal, "Australia/Sydney", 1).expect("schedule");
    let ClaimDecision::Acquired(_) =
        acquire_due_claim(&terminal, &schedule, date, 1).expect("claim")
    else {
        panic!("claim");
    };
    let starter = FakeStarter::default();
    starter
        .terminal
        .lock()
        .expect("terminal")
        .insert(expected_run);
    assert_eq!(
        process_due_schedule(
            &terminal,
            &schedule,
            schedule_time,
            ScheduleTrigger::Startup,
            &ready,
            &starter,
        )
        .expect("terminal reconcile"),
        ScheduleRunOutcome::AlreadyClaimed
    );
    assert!(starter.starts.lock().expect("starts").is_empty());
}

#[test]
fn locked_model_and_local_state_defer_visibly_until_relevant_transition() {
    for (readiness, reason) in [
        (
            ReadinessSnapshot::deferred(
                DeferredReason::IdentityLocked,
                "identity:locked|model:ready|local:ready",
            ),
            DeferredReason::IdentityLocked,
        ),
        (
            ReadinessSnapshot::deferred(
                DeferredReason::ModelUnavailable,
                "identity:ready|model:down|local:ready",
            ),
            DeferredReason::ModelUnavailable,
        ),
        (
            ReadinessSnapshot::deferred(
                DeferredReason::LocalStateUnavailable,
                "identity:ready|model:ready|local:down",
            ),
            DeferredReason::LocalStateUnavailable,
        ),
    ] {
        let conn = migrated_store();
        let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
        let starter = FakeStarter::default();
        let now = utc("2026-07-25T01:00:00Z");
        assert_eq!(
            process_due_schedule(
                &conn,
                &schedule,
                now,
                ScheduleTrigger::Startup,
                &readiness,
                &starter
            )
            .expect("deferred"),
            ScheduleRunOutcome::Deferred { reason }
        );
        assert_eq!(
            process_due_schedule(
                &conn,
                &schedule,
                now,
                ScheduleTrigger::Timer,
                &readiness,
                &starter
            )
            .expect("same state"),
            ScheduleRunOutcome::AlreadyClaimed
        );
        let transitioned = ReadinessSnapshot::ready("identity:ready|model:ready|local:ready");
        assert!(matches!(
            process_due_schedule(
                &conn,
                &schedule,
                now,
                ScheduleTrigger::Timer,
                &transitioned,
                &starter
            )
            .expect("transition"),
            ScheduleRunOutcome::Started { .. }
        ));
    }
}

#[test]
fn failed_start_is_deferred_and_retries_only_after_transition() {
    let conn = migrated_store();
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let failing = FakeStarter {
        fail: true,
        ..FakeStarter::default()
    };
    let now = utc("2026-07-25T01:00:00Z");
    let ready = ReadinessSnapshot::ready("ready-v1");
    assert_eq!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Wake,
            &ready,
            &failing
        )
        .expect("deferred"),
        ScheduleRunOutcome::Deferred {
            reason: DeferredReason::LocalStateUnavailable
        }
    );
    assert_eq!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Timer,
            &ready,
            &failing
        )
        .expect("no hot loop"),
        ScheduleRunOutcome::AlreadyClaimed
    );
}
