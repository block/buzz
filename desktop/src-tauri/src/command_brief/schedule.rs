//! Protected local-time schedule and durable per-day generation claims.

use std::str::FromStr;

use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::types::BriefSchedule;

/// Trusted schedule identity. Renderer input can never replace it.
pub const DEFAULT_SCHEDULE_ID: &str = "daily-command-brief";
/// Default local wall-clock generation time.
pub const DEFAULT_LOCAL_TIME: &str = "06:00";
/// Maximum readiness transitions allowed for one deferred claim.
pub const MAX_DEFERRED_RETRIES: u8 = 8;
const MAX_TRANSITION_TOKEN_BYTES: usize = 256;
const MAX_RUN_ID_BYTES: usize = 128;

/// Renderer-safe schedule settings. The schedule identity is intentionally absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleUpdate {
    /// Whether scheduled generation is enabled.
    pub enabled: bool,
    /// Local wall-clock time as exact `HH:MM`.
    pub local_time: String,
    /// IANA timezone name.
    pub timezone: String,
    /// Whether startup and wake may catch up once on the current local date.
    pub catch_up_same_day: bool,
    /// Local-model capacity, exactly one or two.
    pub concurrency: u8,
}

/// Source of one due check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleTrigger {
    /// Normal in-process timer or readiness poll.
    Timer,
    /// Application startup.
    Startup,
    /// Verified macOS workspace wake notification.
    Wake,
}

/// Closed reason a claimed run is visible but deferred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeferredReason {
    /// The owner identity is locked or in recovery.
    IdentityLocked,
    /// LM Studio or the selected local model is unavailable.
    ModelUnavailable,
    /// The protected orchestrator or mandatory local state is unavailable.
    LocalStateUnavailable,
}

impl DeferredReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::IdentityLocked => "identity_locked",
            Self::ModelUnavailable => "model_unavailable",
            Self::LocalStateUnavailable => "local_state_unavailable",
        }
    }
}

/// Durable unique claim acquired before model generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleClaim {
    idempotency_key: String,
    local_date: NaiveDate,
    run_id: String,
}

impl ScheduleClaim {
    /// Return the exact persisted idempotency key.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    /// Return the claimed local date.
    pub const fn local_date(&self) -> NaiveDate {
        self.local_date
    }

    /// Return the deterministic run identity stored by the claim transaction.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

/// Result of atomically claiming one local date.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimDecision {
    /// This process acquired the unique claim.
    Acquired(ScheduleClaim),
    /// The exact schedule/date has already been claimed.
    AlreadyClaimed,
}

/// One redacted readiness observation used to gate scheduled generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadinessSnapshot {
    deferred_reason: Option<DeferredReason>,
    transition_token: String,
}

impl ReadinessSnapshot {
    /// Construct an all-local-gates-ready observation.
    pub fn ready(transition_token: &str) -> Self {
        Self {
            deferred_reason: None,
            transition_token: transition_token.to_string(),
        }
    }

    /// Construct a visible fail-closed observation.
    pub fn deferred(reason: DeferredReason, transition_token: &str) -> Self {
        Self {
            deferred_reason: Some(reason),
            transition_token: transition_token.to_string(),
        }
    }
}

/// Durable/in-process reconciliation state for one exact scheduled run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledRunPresence {
    /// No current process or durable terminal knows the run.
    Absent,
    /// The current orchestrator has already accepted the exact run ID.
    Active,
    /// The encrypted audit spool contains a terminal for the exact run ID.
    Terminal,
}

/// Native-only boundary that starts one Task 5 orchestrator run.
pub trait ScheduledRunStarter {
    /// Idempotently start the exact deterministic run identity.
    fn start_scheduled(
        &self,
        run_id: &str,
        idempotency_key: &str,
        schedule_id: &str,
        observed_at: &str,
    ) -> Result<String, ScheduledStartError>;

    /// Reconcile the exact run against in-process and durable state.
    fn presence(&self, run_id: &str) -> ScheduledRunPresence;
}

/// Redacted native orchestrator start failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledStartError;

/// Observable result of one startup, wake, timer, or readiness check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleRunOutcome {
    /// No current-local-date run is due.
    NotDue,
    /// This date already has a claim or no readiness transition occurred.
    AlreadyClaimed,
    /// The claim exists but a named local gate remains unavailable.
    Deferred {
        /// Redacted gate preventing generation.
        reason: DeferredReason,
    },
    /// The Task 5 orchestrator accepted this claimed run.
    Started {
        /// Native orchestrator run ID.
        run_id: String,
    },
}

/// Return the current macOS/IANA timezone name.
pub fn current_macos_timezone() -> Result<String, String> {
    let timezone = iana_time_zone::get_timezone().map_err(|_| "timezone unavailable")?;
    Tz::from_str(&timezone).map_err(|_| "timezone unavailable")?;
    Ok(timezone)
}

/// Load the protected schedule or create its trusted default.
pub fn load_or_create_schedule(
    conn: &Connection,
    default_timezone: &str,
    now: i64,
) -> Result<BriefSchedule, String> {
    if let Some(row) = conn
        .query_row(
            "SELECT classification,schedule_id,enabled,local_time,timezone,
                    catch_up_same_day,concurrency
             FROM command_brief_schedule WHERE schedule_id=?1",
            [DEFAULT_SCHEDULE_ID],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| "command brief schedule unavailable")?
    {
        return parse_schedule_row(row);
    }
    let update = ScheduleUpdate {
        enabled: true,
        local_time: DEFAULT_LOCAL_TIME.to_string(),
        timezone: default_timezone.to_string(),
        catch_up_same_day: true,
        concurrency: 1,
    };
    save_schedule_update(conn, update, now)
}

/// Persist validated schedule settings under the fixed trusted identity.
pub fn save_schedule_update(
    conn: &Connection,
    update: ScheduleUpdate,
    now: i64,
) -> Result<BriefSchedule, String> {
    validate_update(&update)?;
    conn.execute(
        "INSERT INTO command_brief_schedule(
             schedule_id,classification,enabled,local_time,timezone,
             catch_up_same_day,concurrency,updated_at
         ) VALUES(?1,'OFFICIAL',?2,?3,?4,?5,?6,?7)
         ON CONFLICT(schedule_id) DO UPDATE SET
             enabled=excluded.enabled,
             local_time=excluded.local_time,
             timezone=excluded.timezone,
             catch_up_same_day=excluded.catch_up_same_day,
             concurrency=excluded.concurrency,
             updated_at=excluded.updated_at",
        params![
            DEFAULT_SCHEDULE_ID,
            i64::from(update.enabled),
            update.local_time,
            update.timezone,
            i64::from(update.catch_up_same_day),
            update.concurrency,
            now
        ],
    )
    .map_err(|_| "command brief schedule unavailable")?;
    schedule_from_update(update)
}

/// Return today's local date only when this trigger may generate it now.
pub fn due_local_date(
    schedule: &BriefSchedule,
    now: DateTime<Utc>,
    _trigger: ScheduleTrigger,
) -> Option<NaiveDate> {
    if !schedule.enabled() || !schedule.catch_up_same_day() {
        return None;
    }
    let timezone = Tz::from_str(schedule.timezone()).ok()?;
    let local_date = now.with_timezone(&timezone).date_naive();
    let due = scheduled_instant(schedule, local_date).ok()?;
    (now >= due).then_some(local_date)
}

/// Return the first scheduled UTC instant strictly after `now`.
pub fn next_due_after(
    schedule: &BriefSchedule,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, String> {
    let timezone = Tz::from_str(schedule.timezone()).map_err(|_| "invalid timezone")?;
    let mut date = now.with_timezone(&timezone).date_naive();
    for _ in 0..=370 {
        let candidate = scheduled_instant(schedule, date)?;
        if candidate > now {
            return Ok(candidate);
        }
        date = date
            .succ_opt()
            .ok_or_else(|| "schedule date unavailable".to_string())?;
    }
    Err("schedule date unavailable".to_string())
}

/// Construct the exact UTF-8 idempotency key for one schedule-local date.
pub fn idempotency_key(schedule: &BriefSchedule, date: NaiveDate) -> String {
    format!("{}:{}", schedule.schedule_id(), date.format("%Y-%m-%d"))
}

/// Derive the stable native run identity persisted before orchestration.
pub fn deterministic_run_id(idempotency_key: &str) -> String {
    format!(
        "scheduled-{}",
        hex::encode(Sha256::digest(idempotency_key.as_bytes()))
    )
}

/// Atomically acquire the one permitted claim for a schedule-local date.
pub fn acquire_due_claim(
    conn: &Connection,
    schedule: &BriefSchedule,
    date: NaiveDate,
    now: i64,
) -> Result<ClaimDecision, String> {
    let key = idempotency_key(schedule, date);
    let run_id = deterministic_run_id(&key);
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|_| "command brief schedule unavailable")?;
    let inserted = tx
        .execute(
            "INSERT OR IGNORE INTO command_brief_schedule_claims(
                 idempotency_key,schedule_id,local_date,timezone,state,
                 retry_count,claimed_at,updated_at,run_id
             ) VALUES(?1,?2,?3,?4,'claimed',0,?5,?5,?6)",
            params![
                key,
                schedule.schedule_id(),
                date.format("%Y-%m-%d").to_string(),
                schedule.timezone(),
                now,
                run_id,
            ],
        )
        .map_err(|_| "command brief schedule unavailable")?;
    tx.commit()
        .map_err(|_| "command brief schedule unavailable")?;
    if inserted == 1 {
        Ok(ClaimDecision::Acquired(ScheduleClaim {
            idempotency_key: key,
            local_date: date,
            run_id,
        }))
    } else {
        Ok(ClaimDecision::AlreadyClaimed)
    }
}

/// Record a visible deferred claim without consuming a retry transition.
pub fn record_deferred(
    conn: &Connection,
    claim: &ScheduleClaim,
    reason: DeferredReason,
    transition_token: &str,
    now: i64,
) -> Result<(), String> {
    if !valid_token(transition_token) {
        return Err("command brief schedule unavailable".to_string());
    }
    let changed = conn
        .execute(
            "UPDATE command_brief_schedule_claims
             SET state='deferred',deferred_reason=?2,transition_token=?3,updated_at=?4
             WHERE idempotency_key=?1 AND state IN ('claimed','deferred','started')",
            params![
                claim.idempotency_key,
                reason.as_str(),
                transition_token,
                now
            ],
        )
        .map_err(|_| "command brief schedule unavailable")?;
    if changed == 1 {
        Ok(())
    } else {
        Err("command brief schedule unavailable".to_string())
    }
}

/// Rearm a deferred claim only for a distinct readiness transition.
pub fn retry_deferred_on_transition(
    conn: &Connection,
    claim: &ScheduleClaim,
    transition_token: &str,
    now: i64,
) -> Result<bool, String> {
    if !valid_token(transition_token) {
        return Err("command brief schedule unavailable".to_string());
    }
    conn.execute(
        "UPDATE command_brief_schedule_claims
         SET state='claimed',deferred_reason=NULL,transition_token=?2,
             retry_count=retry_count+1,updated_at=?3
         WHERE idempotency_key=?1
           AND state='deferred'
           AND retry_count < ?4
           AND (transition_token IS NULL OR transition_token <> ?2)",
        params![
            claim.idempotency_key,
            transition_token,
            now,
            MAX_DEFERRED_RETRIES
        ],
    )
    .map(|changed| changed == 1)
    .map_err(|_| "command brief schedule unavailable".to_string())
}

/// Mark a claimed schedule date as started with the Task 5 run identity.
pub fn mark_claim_started(
    conn: &Connection,
    claim: &ScheduleClaim,
    run_id: &str,
    now: i64,
) -> Result<(), String> {
    if run_id.is_empty() || run_id.len() > MAX_RUN_ID_BYTES || run_id.chars().any(char::is_control)
    {
        return Err("command brief schedule unavailable".to_string());
    }
    let changed = conn
        .execute(
            "UPDATE command_brief_schedule_claims
             SET state='started',run_id=?2,deferred_reason=NULL,updated_at=?3
             WHERE idempotency_key=?1
               AND run_id=?2
               AND state IN ('claimed','started')",
            params![claim.idempotency_key, run_id, now],
        )
        .map_err(|_| "command brief schedule unavailable")?;
    if changed == 1 {
        Ok(())
    } else {
        Err("command brief schedule unavailable".to_string())
    }
}

/// Claim and start at most one due run, retrying only on readiness transitions.
pub fn process_due_schedule(
    conn: &Connection,
    schedule: &BriefSchedule,
    now: DateTime<Utc>,
    trigger: ScheduleTrigger,
    readiness: &ReadinessSnapshot,
    starter: &dyn ScheduledRunStarter,
) -> Result<ScheduleRunOutcome, String> {
    if !valid_token(&readiness.transition_token) {
        return Err("command brief schedule unavailable".to_string());
    }
    let Some(date) = due_local_date(schedule, now, trigger) else {
        return Ok(ScheduleRunOutcome::NotDue);
    };
    let (claim, prior_state) = match acquire_due_claim(conn, schedule, date, now.timestamp())? {
        ClaimDecision::Acquired(claim) => (claim, ClaimState::Claimed),
        ClaimDecision::AlreadyClaimed => {
            let Some((existing, state)) = existing_claim(conn, schedule, date)? else {
                return Ok(ScheduleRunOutcome::AlreadyClaimed);
            };
            if state == ClaimState::Completed {
                return Ok(ScheduleRunOutcome::AlreadyClaimed);
            }
            if state == ClaimState::Deferred
                && !retry_deferred_on_transition(
                    conn,
                    &existing,
                    &readiness.transition_token,
                    now.timestamp(),
                )?
            {
                return Ok(ScheduleRunOutcome::AlreadyClaimed);
            }
            let state = if state == ClaimState::Deferred {
                ClaimState::Claimed
            } else {
                state
            };
            (existing, state)
        }
    };
    match starter.presence(claim.run_id()) {
        ScheduledRunPresence::Terminal => {
            mark_claim_completed(conn, &claim, now.timestamp())?;
            return Ok(ScheduleRunOutcome::AlreadyClaimed);
        }
        ScheduledRunPresence::Active => {
            mark_claim_started(conn, &claim, claim.run_id(), now.timestamp())?;
            if prior_state == ClaimState::Started {
                return Ok(ScheduleRunOutcome::AlreadyClaimed);
            }
            return Ok(ScheduleRunOutcome::Started {
                run_id: claim.run_id().to_string(),
            });
        }
        ScheduledRunPresence::Absent => {}
    }
    if let Some(reason) = readiness.deferred_reason {
        record_deferred(
            conn,
            &claim,
            reason,
            &readiness.transition_token,
            now.timestamp(),
        )?;
        return Ok(ScheduleRunOutcome::Deferred { reason });
    }
    let observed_at = now.to_rfc3339();
    match starter.start_scheduled(
        claim.run_id(),
        claim.idempotency_key(),
        schedule.schedule_id(),
        &observed_at,
    ) {
        Ok(run_id) if run_id == claim.run_id() => {
            mark_claim_started(conn, &claim, claim.run_id(), now.timestamp())?;
            Ok(ScheduleRunOutcome::Started { run_id })
        }
        Ok(_) | Err(ScheduledStartError) => {
            let reason = DeferredReason::LocalStateUnavailable;
            record_deferred(
                conn,
                &claim,
                reason,
                &readiness.transition_token,
                now.timestamp(),
            )?;
            Ok(ScheduleRunOutcome::Deferred { reason })
        }
    }
}

fn mark_claim_completed(conn: &Connection, claim: &ScheduleClaim, now: i64) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE command_brief_schedule_claims
             SET state='completed',deferred_reason=NULL,updated_at=?3
             WHERE idempotency_key=?1 AND run_id=?2
               AND state IN ('claimed','deferred','started','completed')",
            params![claim.idempotency_key, claim.run_id, now],
        )
        .map_err(|_| "command brief schedule unavailable")?;
    if changed == 1 {
        Ok(())
    } else {
        Err("command brief schedule unavailable".to_string())
    }
}

fn validate_update(update: &ScheduleUpdate) -> Result<(), String> {
    parse_local_time(&update.local_time)?;
    Tz::from_str(&update.timezone).map_err(|_| "invalid timezone")?;
    if !matches!(update.concurrency, 1 | 2) {
        return Err("invalid concurrency".to_string());
    }
    Ok(())
}

fn parse_local_time(value: &str) -> Result<NaiveTime, String> {
    if value.len() != 5 {
        return Err("invalid local time".to_string());
    }
    NaiveTime::parse_from_str(value, "%H:%M").map_err(|_| "invalid local time".to_string())
}

fn scheduled_instant(schedule: &BriefSchedule, date: NaiveDate) -> Result<DateTime<Utc>, String> {
    let timezone = Tz::from_str(schedule.timezone()).map_err(|_| "invalid timezone")?;
    let time = parse_local_time(schedule.local_time())?;
    let requested = NaiveDateTime::new(date, time);
    for minute in 0..=1_440 {
        let candidate = requested
            .checked_add_signed(Duration::minutes(minute))
            .ok_or_else(|| "schedule date unavailable".to_string())?;
        match timezone.from_local_datetime(&candidate) {
            LocalResult::Single(value) => return Ok(value.with_timezone(&Utc)),
            LocalResult::Ambiguous(first, second) => {
                return Ok(first.min(second).with_timezone(&Utc));
            }
            LocalResult::None => {}
        }
    }
    Err("schedule date unavailable".to_string())
}

fn schedule_from_update(update: ScheduleUpdate) -> Result<BriefSchedule, String> {
    BriefSchedule::try_from(json!({
        "classification": "OFFICIAL",
        "scheduleId": DEFAULT_SCHEDULE_ID,
        "enabled": update.enabled,
        "localTime": update.local_time,
        "timezone": update.timezone,
        "catchUpSameDay": update.catch_up_same_day,
        "concurrency": update.concurrency,
    }))
    .map_err(|_| "command brief schedule unavailable".to_string())
}

fn parse_schedule_row(
    row: (String, String, i64, String, String, i64, i64),
) -> Result<BriefSchedule, String> {
    let (classification, schedule_id, enabled, local_time, timezone, catch_up, concurrency) = row;
    let concurrency =
        u8::try_from(concurrency).map_err(|_| "command brief schedule unavailable")?;
    let update = ScheduleUpdate {
        enabled: enabled == 1,
        local_time,
        timezone,
        catch_up_same_day: catch_up == 1,
        concurrency,
    };
    validate_update(&update)?;
    if classification != "OFFICIAL"
        || schedule_id != DEFAULT_SCHEDULE_ID
        || !matches!(enabled, 0 | 1)
        || !matches!(catch_up, 0 | 1)
    {
        return Err("command brief schedule unavailable".to_string());
    }
    schedule_from_update(update)
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TRANSITION_TOKEN_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaimState {
    Claimed,
    Deferred,
    Started,
    Completed,
}

fn existing_claim(
    conn: &Connection,
    schedule: &BriefSchedule,
    date: NaiveDate,
) -> Result<Option<(ScheduleClaim, ClaimState)>, String> {
    let key = idempotency_key(schedule, date);
    conn.query_row(
        "SELECT local_date,run_id,state FROM command_brief_schedule_claims
         WHERE idempotency_key=?1",
        [&key],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )
    .optional()
    .map_err(|_| "command brief schedule unavailable".to_string())?
    .map(|(stored_date, run_id, state)| {
        let local_date = NaiveDate::parse_from_str(&stored_date, "%Y-%m-%d")
            .map_err(|_| "command brief schedule unavailable".to_string())?;
        let state = match state.as_str() {
            "claimed" => ClaimState::Claimed,
            "deferred" => ClaimState::Deferred,
            "started" => ClaimState::Started,
            "completed" => ClaimState::Completed,
            _ => return Err("command brief schedule unavailable".to_string()),
        };
        if run_id != deterministic_run_id(&key) {
            return Err("command brief schedule unavailable".to_string());
        }
        Ok((
            ScheduleClaim {
                idempotency_key: key,
                local_date,
                run_id,
            },
            state,
        ))
    })
    .transpose()
}
