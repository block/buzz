use chrono::TimeZone;

use super::*;
use crate::store::partition::{MonthCoverage, PartitionChildAudit, PARTITIONED_TABLES};

fn month(year: i32, month: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).unwrap()
}

fn child(
    name: &str,
    kind: PartitionChildKind,
    lower: PartitionBound,
    upper: PartitionBound,
    catch_all_nonempty: Option<bool>,
) -> PartitionChildAudit {
    PartitionChildAudit {
        schema: "public".to_string(),
        name: name.to_string(),
        relation_kind: "r".to_string(),
        lower: Some(lower),
        upper: Some(upper),
        kind,
        pending_detach: false,
        catch_all_nonempty,
        default_nonempty: None,
        missing_triggers: Vec::new(),
        extra_triggers: Vec::new(),
    }
}

/// The repaired production layout: a July/August legacy leaf, September through
/// December monthlies, and a catch-all from January 2027.
fn repaired_audit(catch_all_nonempty: bool) -> PartitionTableAudit {
    let mut children = vec![
        child(
            "events_p2026_07_08_legacy",
            PartitionChildKind::LegacyLeaf,
            PartitionBound::Finite(month(2026, 7)),
            PartitionBound::Finite(month(2026, 9)),
            None,
        ),
        child(
            "events_p_future_next",
            PartitionChildKind::CatchAll,
            PartitionBound::Finite(month(2027, 1)),
            PartitionBound::MaxValue,
            Some(catch_all_nonempty),
        ),
    ];
    for m in 9..=12 {
        children.push(child(
            &format!("events_p2026_{m:02}"),
            PartitionChildKind::CanonicalMonthly,
            PartitionBound::Finite(month(2026, m)),
            PartitionBound::Finite({
                let (year, next) = add_months(2026, m, 1).unwrap();
                month(year, next)
            }),
            None,
        ));
    }
    let months = (0..=PARTITION_MANAGER_MONTHS_AHEAD as i32)
        .map(|offset| {
            let (year, m) = add_months(2026, 9, offset).unwrap();
            let start = month(year, m);
            MonthCoverage {
                start,
                kind: if start < month(2027, 1) {
                    MonthCoverageKind::CoveredByMonthly
                } else {
                    MonthCoverageKind::CoveredByCatchAll
                },
            }
        })
        .collect();
    PartitionTableAudit {
        table: "events",
        partition_key: Some("RANGE (created_at)".to_string()),
        expected_partition_key: "RANGE (created_at)",
        partition_key_valid: true,
        children,
        coverage_leaves: Vec::new(),
        months,
        serving_safe: true,
    }
}

const ADVANCE: PartitionMaintenancePolicy = PartitionMaintenancePolicy {
    create_enabled: true,
    advance_enabled: true,
};

const CREATE_ONLY: PartitionMaintenancePolicy = PartitionMaintenancePolicy {
    create_enabled: true,
    advance_enabled: false,
};

#[test]
fn repaired_layout_plans_three_monthlies_and_an_april_catch_all() {
    assert_eq!(
        plan_maintenance(&repaired_audit(false), ADVANCE),
        MaintenancePlan::Apply(MaintenanceChange {
            create: vec![month(2027, 1), month(2027, 2), month(2027, 3)],
            catch_all: Some(CatchAllReplacement {
                schema: "public".to_string(),
                name: "events_p_future_next".to_string(),
                new_lower: month(2027, 4),
            }),
        })
    );
}

#[test]
fn advancement_requires_its_own_policy_flag() {
    assert_eq!(
        plan_maintenance(&repaired_audit(false), CREATE_ONLY),
        MaintenancePlan::Disabled
    );
    assert_eq!(
        plan_maintenance(
            &repaired_audit(false),
            PartitionMaintenancePolicy::default()
        ),
        MaintenancePlan::Disabled
    );
}

#[test]
fn full_monthly_runway_is_a_noop_even_when_disabled() {
    let mut audit = repaired_audit(false);
    for coverage in &mut audit.months {
        coverage.kind = MonthCoverageKind::CoveredByMonthly;
    }
    assert_eq!(plan_maintenance(&audit, ADVANCE), MaintenancePlan::Noop);
    assert_eq!(
        plan_maintenance(&audit, PartitionMaintenancePolicy::default()),
        MaintenancePlan::Noop
    );
}

#[test]
fn unsafe_catch_all_states_require_an_operator() {
    let refused = |audit: &PartitionTableAudit, needle: &str| match plan_maintenance(audit, ADVANCE)
    {
        MaintenancePlan::Refuse(reason) => assert!(reason.contains(needle), "{reason}"),
        plan => panic!("expected refusal containing {needle:?}, got {plan:?}"),
    };

    refused(&repaired_audit(true), "contains rows");

    let mut unknown = repaired_audit(false);
    unknown.children[1].catch_all_nonempty = None;
    refused(&unknown, "occupancy is unknown");

    let mut misaligned = repaired_audit(false);
    misaligned.children[1].lower = Some(PartitionBound::Finite(
        Utc.with_ymd_and_hms(2027, 1, 15, 0, 0, 0).unwrap(),
    ));
    refused(&misaligned, "not a UTC month start");

    let mut pending = repaired_audit(false);
    pending.children[1].pending_detach = true;
    refused(&pending, "not structurally safe");

    let mut child_triggers = repaired_audit(false);
    child_triggers.children[1].extra_triggers = vec!["audit_catch_all".to_string()];
    refused(&child_triggers, "child-only triggers (audit_catch_all)");
}

#[test]
fn advancement_crosses_the_year_boundary() {
    // December: the horizon runs through June and the catch-all moves to July.
    let mut audit = repaired_audit(false);
    audit.months = (0..=PARTITION_MANAGER_MONTHS_AHEAD as i32)
        .map(|offset| {
            let (year, m) = add_months(2026, 12, offset).unwrap();
            MonthCoverage {
                start: month(year, m),
                kind: if offset == 0 {
                    MonthCoverageKind::CoveredByMonthly
                } else {
                    MonthCoverageKind::CoveredByCatchAll
                },
            }
        })
        .collect();
    let MaintenancePlan::Apply(change) = plan_maintenance(&audit, ADVANCE) else {
        panic!("expected advancement");
    };
    assert_eq!(change.create.first(), Some(&month(2027, 1)));
    assert_eq!(change.create.last(), Some(&month(2027, 6)));
    assert_eq!(change.create.len(), 6);
    assert_eq!(change.catch_all.map(|c| c.new_lower), Some(month(2027, 7)));
}

#[test]
fn create_kill_switch_also_stops_advancement() {
    let advance_only = PartitionMaintenancePolicy {
        create_enabled: false,
        advance_enabled: true,
    };
    assert_eq!(
        plan_maintenance(&repaired_audit(false), advance_only),
        MaintenancePlan::Disabled
    );
}

#[test]
fn advancement_is_capped_at_the_supported_horizon() {
    // The horizon ends in March 2027, so a catch-all from April 2017 needs
    // exactly the maximum number of monthlies and one from March 2017 needs one more.
    let from = |lower| {
        let mut audit = repaired_audit(false);
        audit.children[1].lower = Some(PartitionBound::Finite(lower));
        plan_maintenance(&audit, ADVANCE)
    };
    let MaintenancePlan::Apply(change) = from(month(2017, 4)) else {
        panic!("expected advancement at the cap");
    };
    assert_eq!(change.create.len(), MAX_PARTITION_MONTHS_AHEAD as usize);
    match from(month(2017, 3)) {
        MaintenancePlan::Refuse(reason) => assert!(reason.contains("more than"), "{reason}"),
        plan => panic!("expected refusal past the cap, got {plan:?}"),
    }
}

/// `buzz_partition_create_attempts_total` counts keyed by their `outcome` label.
fn create_attempts(result: &TableMaintenance) -> std::collections::BTreeMap<String, u64> {
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    metrics::with_local_recorder(&recorder, || {
        emit_maintenance_metrics(&repaired_audit(false), result)
    });
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| key.key().name() == "buzz_partition_create_attempts_total")
        .map(|(key, _, _, value)| {
            let outcome = key
                .key()
                .labels()
                .find(|label| label.key() == "outcome")
                .map(|label| label.value().to_string())
                .expect("outcome label");
            let metrics_util::debugging::DebugValue::Counter(count) = value else {
                panic!("create attempts must be a counter");
            };
            (outcome, count)
        })
        .collect()
}

#[test]
fn only_ddl_failures_count_as_create_errors() {
    let MaintenancePlan::Apply(change) = plan_maintenance(&repaired_audit(false), ADVANCE) else {
        panic!("expected advancement");
    };
    let failed = |outcome| TableMaintenance::failed(outcome, Some(change.clone()), String::new());
    let covered = ("skipped_covered".to_string(), 4);

    for outcome in [
        PartitionMaintenanceOutcome::LockTimeout,
        PartitionMaintenanceOutcome::Deadline,
    ] {
        assert_eq!(
            create_attempts(&failed(outcome)),
            [covered.clone()].into(),
            "{outcome:?}"
        );
    }
    for outcome in [
        PartitionMaintenanceOutcome::Error,
        PartitionMaintenanceOutcome::OperatorRequired,
    ] {
        assert_eq!(
            create_attempts(&failed(outcome)),
            [covered.clone(), ("error".to_string(), 3)].into(),
            "{outcome:?}"
        );
    }
    let advanced = TableMaintenance {
        outcome: PartitionMaintenanceOutcome::Advanced,
        change: Some(change.clone()),
        error: None,
    };
    assert_eq!(
        create_attempts(&advanced),
        [covered, ("created".to_string(), 3)].into()
    );
}

#[test]
fn outcome_labels_are_stable() {
    use PartitionMaintenanceOutcome::*;
    assert_eq!(
        [
            Noop,
            SkippedDisabled,
            Created,
            Advanced,
            SkippedLocked,
            OperatorRequired,
            LockTimeout,
            Deadline,
            Error
        ]
        .map(PartitionMaintenanceOutcome::as_str),
        [
            "noop",
            "skipped_disabled",
            "created",
            "advanced",
            "skipped_locked",
            "operator_required",
            "lock_timeout",
            "deadline",
            "error"
        ]
    );
}

mod postgres_tests {
    use std::time::Instant;

    use sqlx::PgPool;

    use super::*;
    use crate::store::partition::tests::postgres_tests::{
        catalog_snapshot, create_child, drop_schema, fixed_now, scratch_pool, seed_parents,
    };

    /// The repaired incident layout for both managed parents, optionally with
    /// outgoing foreign keys to a `communities` counterpart.
    async fn seed_repaired_layout(pool: &PgPool, with_foreign_keys: bool) {
        seed_parents(pool).await;
        if with_foreign_keys {
            sqlx::query("CREATE TABLE communities (id UUID PRIMARY KEY)")
                .execute(pool)
                .await
                .expect("create communities");
            for table in PARTITIONED_TABLES {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "ALTER TABLE {table} ADD COLUMN community_id UUID REFERENCES communities(id)"
                )))
                .execute(pool)
                .await
                .expect("add counterpart foreign key");
            }
        }
        for table in PARTITIONED_TABLES {
            create_child(
                pool,
                table,
                &format!("{table}_p_past"),
                "MINVALUE",
                "'2026-07-01'",
            )
            .await;
            create_child(
                pool,
                table,
                &format!("{table}_p2026_07_08_legacy"),
                "'2026-07-01'",
                "'2026-09-01'",
            )
            .await;
            for m in 9..=12 {
                let upper = if m == 12 {
                    "'2027-01-01'".to_string()
                } else {
                    format!("'2026-{:02}-01'", m + 1)
                };
                create_child(
                    pool,
                    table,
                    &format!("{table}_p2026_{m:02}"),
                    &format!("'2026-{m:02}-01'"),
                    &upper,
                )
                .await;
            }
            create_child(
                pool,
                table,
                &format!("{table}_p_future_next"),
                "'2027-01-01'",
                "MAXVALUE",
            )
            .await;
        }
    }

    async fn maintain(pool: &PgPool) -> Result<PartitionAudit> {
        maintain_partitions_at(pool, PARTITION_MANAGER_MONTHS_AHEAD, ADVANCE, fixed_now()).await
    }

    fn assert_advanced(audit: &PartitionAudit) {
        for table in &audit.tables {
            assert!(
                table
                    .months
                    .iter()
                    .all(|month| month.kind == MonthCoverageKind::CoveredByMonthly),
                "{}: {:?}",
                table.table,
                table.months
            );
            let catch_all = table
                .children
                .iter()
                .find(|child| child.kind == PartitionChildKind::CatchAll)
                .expect("catch-all");
            assert_eq!(catch_all.name, format!("{}_p_future", table.table));
            assert_eq!(
                catch_all.lower,
                Some(PartitionBound::Finite(month(2027, 4)))
            );
            assert_eq!(catch_all.catch_all_nonempty, Some(false));
            assert!(!table
                .children
                .iter()
                .any(|child| child.name.ends_with("_p_future_next")));
            assert_eq!(table.missing_trigger_count(), 0);
            assert_eq!(table.extra_trigger_count(), 0);
        }
    }

    /// Open a transaction on a separate session that holds `sql`'s lock until dropped.
    async fn hold_lock(pool: &PgPool, sql: &str) -> sqlx::Transaction<'static, sqlx::Postgres> {
        let mut holder = pool.begin().await.expect("begin lock holder");
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
            .execute(&mut *holder)
            .await
            .expect("take conflicting lock");
        holder
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn empty_catch_all_advances_to_canonical_layout_and_is_idempotent() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, true).await;

        let audit = maintain(&pool).await.expect("advance");
        assert_advanced(&audit);
        let fresh = audit_partition_catalog_at(&pool, PARTITION_MANAGER_MONTHS_AHEAD, fixed_now())
            .await
            .expect("fresh audit");
        assert_advanced(&fresh);

        // Rows route to the new leaves, including past the new horizon.
        for (created_at, leaf) in [
            ("2027-02-10", "events_p2027_02"),
            ("2031-01-01", "events_p_future"),
        ] {
            let routed: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "WITH inserted AS (INSERT INTO events (created_at, alternate_at) \
                 VALUES ('{created_at}', now()) RETURNING tableoid) \
                 SELECT tableoid::regclass::text FROM inserted"
            )))
            .fetch_one(&pool)
            .await
            .expect("route row");
            assert_eq!(routed, leaf);
        }
        sqlx::query("DELETE FROM events WHERE created_at >= '2027-01-01'")
            .execute(&pool)
            .await
            .expect("clear routed rows");

        let before = catalog_snapshot(&pool).await;
        maintain(&pool).await.expect("idempotent rerun");
        assert_eq!(catalog_snapshot(&pool).await, before);
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn canonical_catch_all_name_is_replaced_in_one_transaction() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_parents(&pool).await;
        for table in PARTITIONED_TABLES {
            create_child(
                &pool,
                table,
                &format!("{table}_p_past"),
                "MINVALUE",
                "'2026-09-01'",
            )
            .await;
            create_child(
                &pool,
                table,
                &format!("{table}_p_future"),
                "'2026-09-01'",
                "MAXVALUE",
            )
            .await;
        }
        let audit = maintain(&pool).await.expect("advance fresh layout");
        assert_advanced(&audit);
        assert!(audit.tables.iter().all(|table| table
            .children
            .iter()
            .any(|child| child.name == format!("{}_p2026_09", table.table))));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn populated_catch_all_is_refused_before_any_lock() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        sqlx::query("INSERT INTO events (created_at, alternate_at) VALUES ('2027-02-01', now())")
            .execute(&pool)
            .await
            .expect("populate catch-all");
        let before = catalog_snapshot(&pool).await;
        // A lock-taking attempt would wait out the 2s parent lock timeout.
        let holder = hold_lock(&pool, "LOCK TABLE ONLY events IN ACCESS SHARE MODE").await;

        let started = Instant::now();
        let result = maintain(&pool).await;
        let elapsed = started.elapsed();
        drop(holder);

        assert!(
            matches!(result, Err(DbError::InvalidData(ref message))
                if message.contains("events operator_required")
                    && message.contains("events_p_future_next contains rows")
                    && !message.contains("delivery_log")),
            "{result:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
        let after = catalog_snapshot(&pool).await;
        let events_only = |snapshot: &[(String, String, String, i64)]| {
            snapshot
                .iter()
                .filter(|(name, ..)| name.starts_with("events_"))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(events_only(&after), events_only(&before));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn counterpart_contention_fails_fast_and_rolls_back() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, true).await;
        let before = catalog_snapshot(&pool).await;
        let holder = hold_lock(
            &pool,
            "INSERT INTO communities(id) VALUES (gen_random_uuid())",
        )
        .await;

        let started = Instant::now();
        let result = maintain(&pool).await;
        let elapsed = started.elapsed();
        drop(holder);

        assert!(
            matches!(result, Err(DbError::InvalidData(ref message))
                if message.contains("events lock_timeout")
                    && message.contains("delivery_log lock_timeout")),
            "{result:?}"
        );
        // NOWAIT: no 2s wait per table while a counterpart writer is active.
        assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
        assert_eq!(catalog_snapshot(&pool).await, before);

        assert_advanced(&maintain(&pool).await.expect("advance after release"));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn counterpart_reader_does_not_block_advancement() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, true).await;
        let holder = hold_lock(&pool, "SELECT count(*) FROM communities").await;

        let result = maintain(&pool).await;
        drop(holder);

        assert_advanced(&result.expect("advance beside a counterpart reader"));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn writer_checking_a_foreign_key_behind_maintenance_does_not_deadlock() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, true).await;
        // An in-flight insert: the parent first, then the foreign-key check.
        let mut writer = hold_lock(&pool, "LOCK TABLE ONLY events IN ROW EXCLUSIVE MODE").await;
        let writer_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *writer)
            .await
            .expect("writer backend pid");

        let maintenance = tokio::spawn({
            let pool = pool.clone();
            async move { maintain(&pool).await }
        });
        let mut queued = false;
        for _ in 0..50 {
            queued = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(writer_pid)
            .fetch_one(&admin)
            .await
            .expect("check queued maintenance");
            if queued {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(queued, "maintenance never queued behind the writer");

        // Maintenance waits on the parent holding no counterpart lock, so the
        // writer's key check proceeds instead of entering a lock cycle.
        let started = Instant::now();
        sqlx::query("SELECT id FROM communities FOR KEY SHARE")
            .fetch_all(&mut *writer)
            .await
            .expect("foreign-key check behind queued maintenance");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
        writer.commit().await.expect("commit writer");

        assert_advanced(
            &maintenance
                .await
                .expect("maintenance task")
                .expect("advance after the writer commits"),
        );
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deadlock_victim_is_classified_as_a_lock_timeout() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_parents(&pool).await;
        let mut first = hold_lock(&pool, "LOCK TABLE ONLY events IN ACCESS EXCLUSIVE MODE").await;
        let mut second = hold_lock(
            &pool,
            "LOCK TABLE ONLY delivery_log IN ACCESS EXCLUSIVE MODE",
        )
        .await;
        let first_waits = sqlx::query("LOCK TABLE ONLY delivery_log IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *first);
        let second_waits = async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            sqlx::query("LOCK TABLE ONLY events IN ACCESS EXCLUSIVE MODE")
                .execute(&mut *second)
                .await
        };
        let (first_result, second_result) = tokio::join!(first_waits, second_waits);
        let victim = first_result
            .err()
            .or(second_result.err())
            .expect("one session is the deadlock victim");
        assert_eq!(
            victim
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("40P01"),
            "{victim:?}"
        );
        assert_eq!(
            classify_error(&DbError::from(victim)),
            PartitionMaintenanceOutcome::LockTimeout
        );
        drop((first, second));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn parent_contention_times_out_without_blocking_the_other_table() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, true).await;
        let holder = hold_lock(&pool, "LOCK TABLE ONLY events IN ACCESS SHARE MODE").await;

        let result = maintain(&pool).await;
        drop(holder);

        assert!(
            matches!(result, Err(DbError::InvalidData(ref message))
                if message.contains("events lock_timeout") && !message.contains("delivery_log")),
            "{result:?}"
        );
        let audit = audit_partition_catalog_at(&pool, PARTITION_MANAGER_MONTHS_AHEAD, fixed_now())
            .await
            .expect("audit");
        let events = audit.tables.iter().find(|t| t.table == "events").unwrap();
        assert!(events
            .children
            .iter()
            .any(|child| child.name == "events_p_future_next"));
        let delivery_log = audit
            .tables
            .iter()
            .filter(|t| t.table == "delivery_log")
            .cloned()
            .collect::<Vec<_>>();
        assert_advanced(&PartitionAudit {
            audited_at: audit.audited_at,
            tables: delivery_log,
        });
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn held_maintenance_lock_skips_without_ddl() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        let before = catalog_snapshot(&pool).await;
        let holder = hold_lock(
            &pool,
            "SELECT pg_advisory_xact_lock(hashtextextended(\
             'buzz.partition_maintenance:' || current_schema()::text, 0))",
        )
        .await;

        maintain(&pool)
            .await
            .expect("skipped_locked is not a failure");
        assert_eq!(catalog_snapshot(&pool).await, before);
        drop(holder);
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_runners_converge_on_one_layout() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, true).await;

        let (first, second) = tokio::join!(maintain(&pool), maintain(&pool));
        // A runner's catch-all NOWAIT fails fast if the other runner's audit is
        // probing that catch-all; nothing else may go wrong.
        for (runner, result) in [("first", first), ("second", second)] {
            let Err(error) = result else { continue };
            let message = error.to_string();
            assert!(
                message.contains("lock_timeout"),
                "{runner} runner: {message}"
            );
            for table in PARTITIONED_TABLES {
                for outcome in ["operator_required", "deadline", "error"] {
                    assert!(
                        !message.contains(&format!("{table} {outcome}:")),
                        "{runner} runner: {message}"
                    );
                }
            }
        }
        // Whatever a fail-fast loser left undone, the next pass converges.
        assert_advanced(&maintain(&pool).await.expect("follow-up pass"));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn client_deadline_rolls_back_and_discards_the_session() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        let before = catalog_snapshot(&pool).await;
        // The parent lock wait (2s) outlasts the 300ms client deadline.
        let mut holder = hold_lock(&pool, "LOCK TABLE ONLY events IN ACCESS SHARE MODE").await;
        let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *holder)
            .await
            .expect("holder backend pid");

        let result = maintain_partitions_with_deadline(
            &pool,
            PARTITION_MANAGER_MONTHS_AHEAD,
            ADVANCE,
            fixed_now(),
            std::time::Duration::from_millis(300),
        )
        .await;
        assert!(
            matches!(result, Err(DbError::InvalidData(ref message))
                if message.contains("events deadline")),
            "{result:?}"
        );

        // The abandoned session is closed, not pooled: the backend that was
        // queued behind the holder disappears and releases every lock before
        // the holder commits. Only sessions blocked by this test's holder
        // count, so maintenance elsewhere on the cluster cannot interfere.
        let blocked: Vec<i32> = sqlx::query_scalar(
            "SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
        )
        .bind(holder_pid)
        .fetch_all(&admin)
        .await
        .expect("sessions blocked by the holder");
        let mut gone = blocked.is_empty();
        for _ in 0..50 {
            if gone {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let alive: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = ANY($1))",
            )
            .bind(&blocked)
            .fetch_one(&admin)
            .await
            .expect("check abandoned backend");
            gone = !alive;
        }
        drop(holder);
        assert!(
            blocked.len() <= 1,
            "unexpected blocked sessions {blocked:?}"
        );
        assert!(
            gone,
            "abandoned maintenance session {blocked:?} was not closed"
        );
        let after = catalog_snapshot(&pool).await;
        let events_only = |snapshot: &[(String, String, String, i64)]| {
            snapshot
                .iter()
                .filter(|(name, ..)| name.starts_with("events_"))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(events_only(&after), events_only(&before));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn catch_all_with_child_only_trigger_is_refused() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        sqlx::query(
            "CREATE TRIGGER catch_all_only BEFORE INSERT ON events_p_future_next \
             FOR EACH ROW EXECUTE FUNCTION partition_test_trigger()",
        )
        .execute(&pool)
        .await
        .expect("create child-only trigger");
        let before = catalog_snapshot(&pool).await;

        let result = maintain(&pool).await;
        assert!(
            matches!(result, Err(DbError::InvalidData(ref message))
                if message.contains("events operator_required")
                    && message.contains("child-only triggers (catch_all_only)")
                    && !message.contains("delivery_log")),
            "{result:?}"
        );
        let after = catalog_snapshot(&pool).await;
        let events_only = |snapshot: &[(String, String, String, i64)]| {
            snapshot
                .iter()
                .filter(|(name, ..)| name.starts_with("events_"))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(events_only(&after), events_only(&before));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_audit_after_a_concurrent_advance_is_a_noop() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        // A runner audits, then loses the race to a runner that commits first.
        let stale = audit_partition_catalog_at(&pool, PARTITION_MANAGER_MONTHS_AHEAD, fixed_now())
            .await
            .expect("stale audit");
        assert_advanced(&maintain(&pool).await.expect("winning runner"));
        let advanced = catalog_snapshot(&pool).await;

        for table_audit in &stale.tables {
            let result = maintain_table(
                &pool,
                table_audit,
                plan_maintenance(table_audit, ADVANCE),
                PARTITION_MANAGER_MONTHS_AHEAD as i32,
                ADVANCE,
                fixed_now(),
                std::time::Duration::from_secs(10),
            )
            .await;
            assert_eq!(
                (result.outcome, result.error.as_deref()),
                (PartitionMaintenanceOutcome::Noop, None),
                "{}",
                table_audit.table
            );
        }
        assert_eq!(catalog_snapshot(&pool).await, advanced);

        // The losing runner reports the layout the winner committed.
        let reported = maintain_audited(
            &pool,
            stale,
            PARTITION_MANAGER_MONTHS_AHEAD,
            ADVANCE,
            fixed_now(),
            std::time::Duration::from_secs(10),
        )
        .await
        .expect("losing runner");
        assert_advanced(&reported);
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replan_that_finds_only_disabled_work_is_skipped_disabled() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        sqlx::query("DROP TABLE events_p_future_next")
            .execute(&pool)
            .await
            .expect("drop catch-all");
        // Creation is planned for the uncovered months, but a catch-all appears
        // before the lock and advancing it is disabled.
        let stale = audit_partition_catalog_at(&pool, PARTITION_MANAGER_MONTHS_AHEAD, fixed_now())
            .await
            .expect("stale audit");
        let events = stale.tables.iter().find(|t| t.table == "events").unwrap();
        create_child(
            &pool,
            "events",
            "events_p_future_next",
            "'2027-01-01'",
            "MAXVALUE",
        )
        .await;
        let before = catalog_snapshot(&pool).await;

        let result = maintain_table(
            &pool,
            events,
            plan_maintenance(events, CREATE_ONLY),
            PARTITION_MANAGER_MONTHS_AHEAD as i32,
            CREATE_ONLY,
            fixed_now(),
            std::time::Duration::from_secs(10),
        )
        .await;
        assert_eq!(
            (result.outcome, result.error.as_deref()),
            (PartitionMaintenanceOutcome::SkippedDisabled, None)
        );
        assert_eq!(catalog_snapshot(&pool).await, before);
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn audit_retries_when_a_listed_partition_is_dropped() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        let mut dropper = pool.begin().await.expect("begin dropper");
        let dropper_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *dropper)
            .await
            .expect("dropper backend pid");
        sqlx::query("DROP TABLE events_p_future_next")
            .execute(&mut *dropper)
            .await
            .expect("drop catch-all");

        // The audit lists the catch-all, then queues behind the drop's lock.
        let audit = tokio::spawn({
            let pool = pool.clone();
            async move {
                audit_partition_catalog_at(&pool, PARTITION_MANAGER_MONTHS_AHEAD, fixed_now()).await
            }
        });
        let mut queued = false;
        for _ in 0..50 {
            queued = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(dropper_pid)
            .fetch_one(&admin)
            .await
            .expect("check audit queue");
            if queued {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(queued, "audit never queued behind the drop");
        dropper.commit().await.expect("commit drop");

        let audit = audit.await.expect("audit task").expect("audit retried");
        let events = audit
            .tables
            .iter()
            .find(|table| table.table == "events")
            .expect("events audit");
        assert!(events
            .children
            .iter()
            .all(|child| child.name != "events_p_future_next"));
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unrelated_catch_all_name_is_an_operator_error() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        sqlx::query("CREATE TABLE events_p_future (unrelated BOOLEAN)")
            .execute(&pool)
            .await
            .expect("create colliding relation");
        let before = catalog_snapshot(&pool).await;

        let result = maintain(&pool).await;
        assert!(
            matches!(result, Err(DbError::InvalidData(ref message))
                if message.contains("canonical name events_p_future already exists")),
            "{result:?}"
        );
        let after = catalog_snapshot(&pool).await;
        assert!(after
            .iter()
            .any(|(name, ..)| name == "events_p_future_next"));
        assert_eq!(
            after
                .iter()
                .filter(|(name, ..)| name.starts_with("events_"))
                .count(),
            before
                .iter()
                .filter(|(name, ..)| name.starts_with("events_"))
                .count()
        );
        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn incoming_foreign_key_is_refused_under_lock() {
        let (pool, admin, schema) = scratch_pool().await;
        seed_repaired_layout(&pool, false).await;
        sqlx::query(
            "CREATE TABLE event_refs (created_at TIMESTAMPTZ, alternate_at TIMESTAMPTZ, id BIGINT, \
             FOREIGN KEY (created_at, alternate_at, id) REFERENCES events)",
        )
        .execute(&pool)
        .await
        .expect("create referencing table");
        let before = catalog_snapshot(&pool).await;

        let result = maintain(&pool).await;
        assert!(
            matches!(result, Err(DbError::InvalidData(ref message))
                if message.contains("events is referenced by a foreign key")),
            "{result:?}"
        );
        let after = catalog_snapshot(&pool).await;
        assert!(after
            .iter()
            .any(|(name, ..)| name == "events_p_future_next"));
        assert_ne!(after, before, "delivery_log still advances");
        drop_schema(&admin, &schema).await;
    }
}
