use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::super::schedule::{
    deterministic_run_id, load_or_create_schedule, process_due_schedule, ReadinessSnapshot,
    ScheduleRunOutcome, ScheduleTrigger, ScheduledRunPresence, ScheduledRunStarter,
    ScheduledStartError,
};
use super::super::store::migrate_command_brief_store;
use super::{WakeEventHandler, WakeEventSource, WakeSubscription};

#[derive(Default)]
struct FakeWakeSource {
    handler: Mutex<Option<WakeEventHandler>>,
}

struct FakeSubscription;

impl WakeSubscription for FakeSubscription {}

impl WakeEventSource for FakeWakeSource {
    fn subscribe(
        &self,
        handler: WakeEventHandler,
    ) -> Result<Box<dyn WakeSubscription>, &'static str> {
        *self.handler.lock().expect("handler") = Some(handler);
        Ok(Box::new(FakeSubscription))
    }
}

impl FakeWakeSource {
    fn wake(&self) {
        self.handler
            .lock()
            .expect("handler")
            .as_ref()
            .expect("subscribed")();
    }
}

struct FakeStarter {
    starts: Mutex<Vec<String>>,
    active: Mutex<Option<String>>,
}

impl ScheduledRunStarter for FakeStarter {
    fn start_scheduled(
        &self,
        run_id: &str,
        _idempotency_key: &str,
        _schedule_id: &str,
        _observed_at: &str,
    ) -> Result<String, ScheduledStartError> {
        self.starts.lock().expect("starts").push(run_id.to_string());
        *self.active.lock().expect("active") = Some(run_id.to_string());
        Ok(run_id.to_string())
    }

    fn presence(&self, run_id: &str) -> ScheduledRunPresence {
        if self.active.lock().expect("active").as_deref() == Some(run_id) {
            ScheduledRunPresence::Active
        } else {
            ScheduledRunPresence::Absent
        }
    }
}

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("time")
        .with_timezone(&Utc)
}

#[test]
fn fake_system_wake_runs_one_catch_up_after_sleep_crosses_0600() {
    let source = FakeWakeSource::default();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().expect("store")));
    migrate_command_brief_store(&conn.lock().expect("store")).expect("migrate");
    let schedule = load_or_create_schedule(&conn.lock().expect("store"), "Australia/Sydney", 1)
        .expect("schedule");
    let clock = Arc::new(Mutex::new(utc("2026-07-24T19:59:59Z")));
    let starter = Arc::new(FakeStarter {
        starts: Mutex::new(Vec::new()),
        active: Mutex::new(None),
    });
    assert_eq!(
        process_due_schedule(
            &conn.lock().expect("store"),
            &schedule,
            *clock.lock().expect("clock"),
            ScheduleTrigger::Timer,
            &ReadinessSnapshot::ready("ready"),
            starter.as_ref(),
        )
        .expect("before due"),
        ScheduleRunOutcome::NotDue
    );
    let callback_store = Arc::clone(&conn);
    let callback_clock = Arc::clone(&clock);
    let callback_starter = Arc::clone(&starter);
    let callback_schedule = schedule.clone();
    let _subscription = source
        .subscribe(Arc::new(move || {
            process_due_schedule(
                &callback_store.lock().expect("store"),
                &callback_schedule,
                *callback_clock.lock().expect("clock"),
                ScheduleTrigger::Wake,
                &ReadinessSnapshot::ready("ready"),
                callback_starter.as_ref(),
            )
            .expect("wake catch-up");
        }))
        .expect("subscription");
    *clock.lock().expect("clock") = utc("2026-07-24T20:00:01Z");
    source.wake();
    source.wake();
    assert_eq!(
        starter.starts.lock().expect("starts").as_slice(),
        [deterministic_run_id("daily-command-brief:2026-07-25")]
    );
}
