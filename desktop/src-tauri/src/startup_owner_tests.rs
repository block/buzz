use super::*;

#[derive(Default)]
struct ScheduledEffects {
    sources: AtomicUsize,
    advisers: AtomicUsize,
    audits: AtomicUsize,
    publications: AtomicUsize,
}

struct CountingScheduledProvider {
    effects: Arc<ScheduledEffects>,
}

impl BriefSourceProvider for CountingScheduledProvider {
    fn freeze<'a>(
        &'a self,
        _run_id: &'a str,
        _co_request: &'a str,
        _observed_at: &'a str,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<FrozenSourceContext, SourceCollectionError>> {
        self.effects.sources.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            cancellation.cancelled().await;
            Err(SourceCollectionError::Cancelled)
        })
    }

    fn recheck<'a>(
        &'a self,
        _context: &'a FrozenSourceContext,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<(), SourceCollectionError>> {
        Box::pin(async { Err(SourceCollectionError::RagUnavailable) })
    }
}

impl BriefAdviserProvider for CountingScheduledProvider {
    fn run_specialist<'a>(
        &'a self,
        _run_id: &'a str,
        _adviser: AdviserId,
        _sources: Vec<ValidatedSource>,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<AdviserContribution, BriefAdviserError>> {
        self.effects.advisers.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(BriefAdviserError::Failed) })
    }

    fn run_chief_of_staff<'a>(
        &'a self,
        _run_id: &'a str,
        _contributions: Vec<AdviserContribution>,
        _source_ledger: Vec<ValidatedSource>,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<serde_json::Value, BriefAdviserError>> {
        self.effects.advisers.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(BriefAdviserError::Failed) })
    }
}

impl BriefPersistence for CountingScheduledProvider {
    fn persist_terminal<'a>(
        &'a self,
        _input: TerminalAuditInput,
        _cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<PersistedTerminal, BriefPersistenceError>> {
        self.effects.audits.fetch_add(1, Ordering::SeqCst);
        self.effects.publications.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(BriefPersistenceError::Failed) })
    }
}

fn owner_identity(
    owner: &str,
    model: &str,
    snapshot: &str,
    config: &str,
    capacity: u8,
) -> RuntimeConfigIdentity {
    RuntimeConfigIdentity::new_for_test(owner, model, snapshot, config, capacity, "policy-v1")
}

fn owner_runtime(
    owner: &str,
    generation: u64,
    effects: Arc<ScheduledEffects>,
) -> Arc<InstalledCommandBriefRuntime> {
    let provider = Arc::new(CountingScheduledProvider { effects });
    Arc::new(InstalledCommandBriefRuntime {
        owner_pubkey: owner.to_string(),
        config: owner_identity(owner, "qwen", "snapshot-a", "apple-a", 1),
        generation,
        orchestrator: CommandBriefOrchestrator::new(
            LocalModelScheduler::new(1).expect("scheduler"),
            provider.clone(),
            provider.clone(),
            provider,
        ),
    })
}

fn claim_state(path: &std::path::Path) -> (String, Option<String>) {
    let conn = crate::command_brief::store::open_command_brief_store(path).expect("store");
    conn.query_row(
        "SELECT state,deferred_reason FROM command_brief_schedule_claims",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("claim state")
}

#[tokio::test]
async fn owner_switch_after_runtime_install_defers_without_start_or_effects_then_b_reconciles() {
    let state = Arc::new(crate::app_state::build_app_state());
    let owner_a = state.signing_keys().expect("owner A").public_key().to_hex();
    let owner_b_keys = nostr::Keys::generate();
    let owner_b = owner_b_keys.public_key().to_hex();
    let effects_a = Arc::new(ScheduledEffects::default());
    let runtime_a = owner_runtime(&owner_a, 1, Arc::clone(&effects_a));
    let installed = Arc::new(tokio::sync::Barrier::new(2));
    let resume = Arc::new(tokio::sync::Barrier::new(2));
    let directory = tempfile::tempdir().expect("temp");
    let store_path = directory.path().join("brief.db");
    let conn =
        crate::command_brief::store::open_command_brief_store(&store_path).expect("store");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let now = utc("2026-07-25T01:00:00Z");
    let runtime_future = {
        let state = Arc::clone(&state);
        let runtime_a = Arc::clone(&runtime_a);
        let installed = Arc::clone(&installed);
        let resume = Arc::clone(&resume);
        async move {
            state
                .command_brief_runtimes
                .write()
                .await
                .install(Arc::clone(&runtime_a));
            installed.wait().await;
            resume.wait().await;
            Ok(runtime_a)
        }
    };
    let switch_owner = {
        let state = Arc::clone(&state);
        let installed = Arc::clone(&installed);
        let resume = Arc::clone(&resume);
        async move {
            installed.wait().await;
            *state.keys.lock().expect("identity") = owner_b_keys;
            resume.wait().await;
        }
    };

    let (outcome, ()) = tokio::join!(
        super::super::process_scheduled_runtime(
            super::super::ScheduledRuntimeRequest {
                state: &state,
                expected_owner_pubkey: &owner_a,
                conn,
                schedule: &schedule,
                now,
                trigger: ScheduleTrigger::Startup,
                store_path: &store_path,
            },
            runtime_future,
            |_| {},
        ),
        switch_owner,
    );
    assert_eq!(
        outcome.expect("identity deferral"),
        ScheduleRunOutcome::Deferred {
            reason: DeferredReason::IdentityLocked,
        }
    );
    assert_eq!(
        claim_state(&store_path),
        (
            "deferred".to_string(),
            Some("identity_locked".to_string())
        )
    );
    let scheduled_run =
        crate::command_brief::schedule::deterministic_run_id("daily-command-brief:2026-07-25");
    assert!(runtime_a.orchestrator.status(&scheduled_run).is_none());
    tokio::task::yield_now().await;
    assert_eq!(effects_a.sources.load(Ordering::SeqCst), 0);
    assert_eq!(effects_a.advisers.load(Ordering::SeqCst), 0);
    assert_eq!(effects_a.audits.load(Ordering::SeqCst), 0);
    assert_eq!(effects_a.publications.load(Ordering::SeqCst), 0);

    let runtime_b = owner_runtime(
        &owner_b,
        2,
        Arc::new(ScheduledEffects::default()),
    );
    let runtime_b_for_future = Arc::clone(&runtime_b);
    let b_outcome = super::super::process_scheduled_runtime(
        super::super::ScheduledRuntimeRequest {
            state: &state,
            expected_owner_pubkey: &owner_b,
            conn: crate::command_brief::store::open_command_brief_store(&store_path)
                .expect("reconciliation store"),
            schedule: &schedule,
            now,
            trigger: ScheduleTrigger::Readiness,
            store_path: &store_path,
        },
        async {
            state
                .command_brief_runtimes
                .write()
                .await
                .install(Arc::clone(&runtime_b_for_future));
            Ok(runtime_b_for_future)
        },
        |_| {},
    )
    .await
    .expect("owner B reconciliation");
    assert_eq!(
        b_outcome,
        ScheduleRunOutcome::Started {
            run_id: scheduled_run.clone(),
        }
    );
    assert!(runtime_b.orchestrator.status(&scheduled_run).is_some());
    assert!(runtime_b.orchestrator.cancel(&scheduled_run));
}

#[tokio::test]
async fn owner_switch_while_awaiting_presence_and_admission_defers_before_start() {
    let state = Arc::new(crate::app_state::build_app_state());
    let owner_a = state.signing_keys().expect("owner A").public_key().to_hex();
    let runtime_a = owner_runtime(
        &owner_a,
        1,
        Arc::new(ScheduledEffects::default()),
    );
    let mut runtime_guard = state.command_brief_runtimes.write().await;
    runtime_guard.install(Arc::clone(&runtime_a));
    let directory = tempfile::tempdir().expect("temp");
    let store_path = directory.path().join("brief.db");
    let conn =
        crate::command_brief::store::open_command_brief_store(&store_path).expect("store");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let now = utc("2026-07-25T01:00:00Z");
    let switch_owner = {
        let state = Arc::clone(&state);
        async move {
            tokio::task::yield_now().await;
            *state.keys.lock().expect("identity") = nostr::Keys::generate();
            drop(runtime_guard);
        }
    };

    let (outcome, ()) = tokio::join!(
        super::super::process_scheduled_runtime(
            super::super::ScheduledRuntimeRequest {
                state: &state,
                expected_owner_pubkey: &owner_a,
                conn,
                schedule: &schedule,
                now,
                trigger: ScheduleTrigger::Startup,
                store_path: &store_path,
            },
            async { Ok(runtime_a.clone()) },
            |_| {},
        ),
        switch_owner,
    );
    assert_eq!(
        outcome.expect("identity deferral"),
        ScheduleRunOutcome::Deferred {
            reason: DeferredReason::IdentityLocked,
        }
    );
    assert_eq!(
        claim_state(&store_path),
        (
            "deferred".to_string(),
            Some("identity_locked".to_string())
        )
    );
}

#[tokio::test]
async fn timer_admission_token_uses_only_the_active_owners_runtime() {
    let state = crate::app_state::build_app_state();
    let owner_b = state.signing_keys().expect("owner B").public_key().to_hex();
    let runtime_b = owner_runtime(
        &owner_b,
        2,
        Arc::new(ScheduledEffects::default()),
    );
    let runtime_a = owner_runtime(
        "owner-a",
        1,
        Arc::new(ScheduledEffects::default()),
    );
    let request = || {
        CommandBriefRequest::new("daily-command-brief", "prepare", "2026-07-25T01:00:00Z")
            .expect("request")
    };
    runtime_b
        .orchestrator
        .start_exact("owner-b-existing", request())
        .expect("owner B active");
    runtime_a
        .orchestrator
        .start_exact("owner-a-existing", request())
        .expect("owner A active");
    {
        let mut runtimes = state.command_brief_runtimes.write().await;
        runtimes.install(Arc::clone(&runtime_b));
        runtimes.install(Arc::clone(&runtime_a));
    }
    let expected = RuntimeReadiness::ready(
        &runtime_b.config,
        runtime_b.generation,
        runtime_b.orchestrator.admission_state(),
    )
    .transition_token;
    let wrong_owner = RuntimeReadiness::ready(
        &runtime_a.config,
        runtime_a.generation,
        runtime_a.orchestrator.admission_state(),
    )
    .transition_token;

    let actual =
        super::super::current_runtime_admission_token(&state).expect("owner B admission token");
    assert_eq!(actual, expected);
    assert_ne!(actual, wrong_owner);
    assert!(runtime_a.orchestrator.cancel("owner-a-existing"));
    assert!(runtime_b.orchestrator.cancel("owner-b-existing"));
}

#[tokio::test]
async fn owner_transition_without_runtime_reopens_timer_once_then_stabilizes() {
    let state = crate::app_state::build_app_state();
    let owner_a = state.signing_keys().expect("owner A").public_key().to_hex();
    let token_a =
        super::super::current_runtime_admission_token(&state).expect("owner A no-runtime token");
    assert!(token_a.len() <= 256);
    assert_eq!(
        super::super::current_runtime_admission_token(&state).as_deref(),
        Some(token_a.as_str()),
        "an unchanged owner without a runtime must have a stable token"
    );

    let directory = tempfile::tempdir().expect("temp");
    let store_path = directory.path().join("brief.db");
    let conn =
        crate::command_brief::store::open_command_brief_store(&store_path).expect("store");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let now = utc("2026-07-25T01:00:00Z");
    assert_eq!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Startup,
            &ReadinessSnapshot::deferred(DeferredReason::AdmissionUnavailable, &token_a),
            &RecordingStarter::default(),
        )
        .expect("owner A deferral"),
        ScheduleRunOutcome::Deferred {
            reason: DeferredReason::AdmissionUnavailable,
        }
    );

    *state.keys.lock().expect("identity") = nostr::Keys::generate();
    let owner_b = state.signing_keys().expect("owner B").public_key().to_hex();
    assert_ne!(owner_a, owner_b);
    let token_b =
        super::super::current_runtime_admission_token(&state).expect("owner B no-runtime token");
    assert_ne!(
        token_b, token_a,
        "a new active owner must be a readiness transition even before runtime installation"
    );

    let preflight_attempts = AtomicUsize::new(0);
    let first_b_timer = timer_claim_fast_path(
        &conn,
        &schedule,
        now,
        ScheduleTrigger::Timer,
        || super::super::current_runtime_admission_token(&state),
    )
    .expect("owner B timer");
    if first_b_timer.is_none() {
        preflight_attempts.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            process_due_schedule(
                &conn,
                &schedule,
                now,
                ScheduleTrigger::Timer,
                &ReadinessSnapshot::deferred(DeferredReason::AdmissionUnavailable, &token_b),
                &RecordingStarter::default(),
            )
            .expect("owner B retry"),
            ScheduleRunOutcome::Deferred {
                reason: DeferredReason::AdmissionUnavailable,
            }
        );
    }
    assert_eq!(
        preflight_attempts.load(Ordering::SeqCst),
        1,
        "the owner transition must reach preflight"
    );

    for _ in 0..3 {
        let unchanged_b_timer = timer_claim_fast_path(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Timer,
            || super::super::current_runtime_admission_token(&state),
        )
        .expect("unchanged owner B timer");
        if unchanged_b_timer.is_none() {
            preflight_attempts.fetch_add(1, Ordering::SeqCst);
        }
        assert_eq!(
            unchanged_b_timer,
            Some(ScheduleRunOutcome::AlreadyClaimed)
        );
    }
    assert_eq!(
        preflight_attempts.load(Ordering::SeqCst),
        1,
        "an unchanged owner B no-runtime state must not hot-loop preflight"
    );
}

#[tokio::test]
async fn owner_transition_timer_fallthrough_builds_b_runtime_and_retries_claim() {
    let state = crate::app_state::build_app_state();
    let token_a =
        super::super::current_runtime_admission_token(&state).expect("owner A no-runtime token");
    let directory = tempfile::tempdir().expect("temp");
    let store_path = directory.path().join("brief.db");
    let conn =
        crate::command_brief::store::open_command_brief_store(&store_path).expect("store");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
    let now = utc("2026-07-25T01:00:00Z");
    assert_eq!(
        process_due_schedule(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Startup,
            &ReadinessSnapshot::deferred(DeferredReason::AdmissionUnavailable, &token_a),
            &RecordingStarter::default(),
        )
        .expect("owner A deferral"),
        ScheduleRunOutcome::Deferred {
            reason: DeferredReason::AdmissionUnavailable,
        }
    );

    let owner_b_keys = nostr::Keys::generate();
    let owner_b = owner_b_keys.public_key().to_hex();
    *state.keys.lock().expect("identity") = owner_b_keys;
    assert_eq!(
        timer_claim_fast_path(
            &conn,
            &schedule,
            now,
            ScheduleTrigger::Timer,
            || super::super::current_runtime_admission_token(&state),
        )
        .expect("owner B timer"),
        None,
        "the owner transition must fall through to preflight"
    );

    let runtime_builds = AtomicUsize::new(0);
    let runtime_b = owner_runtime(
        &owner_b,
        2,
        Arc::new(ScheduledEffects::default()),
    );
    let runtime_for_build = Arc::clone(&runtime_b);
    let outcome = super::super::process_scheduled_runtime(
        super::super::ScheduledRuntimeRequest {
            state: &state,
            expected_owner_pubkey: &owner_b,
            conn: crate::command_brief::store::open_command_brief_store(&store_path)
                .expect("retry store"),
            schedule: &schedule,
            now,
            trigger: ScheduleTrigger::Timer,
            store_path: &store_path,
        },
        async {
            runtime_builds.fetch_add(1, Ordering::SeqCst);
            state
                .command_brief_runtimes
                .write()
                .await
                .install(Arc::clone(&runtime_for_build));
            Ok(runtime_for_build)
        },
        |_| {},
    )
    .await
    .expect("owner B retry");
    let scheduled_run =
        crate::command_brief::schedule::deterministic_run_id("daily-command-brief:2026-07-25");
    assert_eq!(
        outcome,
        ScheduleRunOutcome::Started {
            run_id: scheduled_run.clone(),
        }
    );
    assert_eq!(runtime_builds.load(Ordering::SeqCst), 1);
    assert!(runtime_b.orchestrator.status(&scheduled_run).is_some());
    assert!(runtime_b.orchestrator.cancel(&scheduled_run));
}

#[tokio::test]
async fn starter_rechecks_owner_immediately_before_presence_and_start_exact() {
    let state = crate::app_state::build_app_state();
    let owner_a = state.signing_keys().expect("owner A").public_key().to_hex();
    let runtime_a = owner_runtime(
        &owner_a,
        1,
        Arc::new(ScheduledEffects::default()),
    );
    *state.keys.lock().expect("identity") = nostr::Keys::generate();
    let directory = tempfile::tempdir().expect("temp");
    let store_path = directory.path().join("brief.db");
    let started_callbacks = AtomicUsize::new(0);
    let runtimes = vec![Arc::clone(&runtime_a)];
    let on_started = |_: &str| {
        started_callbacks.fetch_add(1, Ordering::SeqCst);
    };
    let starter = super::super::OrchestratorStarter {
        state: &state,
        owner_pubkey: &owner_a,
        current: &runtime_a.orchestrator,
        runtimes: &runtimes,
        store_path: &store_path,
        on_started: &on_started,
    };

    assert_eq!(
        starter.presence("scheduled-owner-a"),
        ScheduledRunPresence::IdentityUnavailable
    );
    assert_eq!(
        starter.start_scheduled(
            "scheduled-owner-a",
            "daily-command-brief:2026-07-25",
            "daily-command-brief",
            "2026-07-25T01:00:00Z",
        ),
        Err(ScheduledStartError::IdentityUnavailable)
    );
    assert!(runtime_a
        .orchestrator
        .status("scheduled-owner-a")
        .is_none());
    assert_eq!(started_callbacks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn scheduled_terminal_presence_is_global_but_reveals_no_owner_details() {
    let state = crate::app_state::build_app_state();
    let owner_b = state.signing_keys().expect("owner B").public_key().to_hex();
    let runtime_a = owner_runtime(
        "owner-a",
        1,
        Arc::new(ScheduledEffects::default()),
    );
    let runtime_b = owner_runtime(
        &owner_b,
        2,
        Arc::new(ScheduledEffects::default()),
    );
    let scheduled_run =
        crate::command_brief::schedule::deterministic_run_id("daily-command-brief:2026-07-25");
    runtime_a
        .orchestrator
        .start_exact(
            &scheduled_run,
            CommandBriefRequest::new(
                "daily-command-brief",
                "prepare",
                "2026-07-25T01:00:00Z",
            )
            .expect("request"),
        )
        .expect("owner A run");
    assert!(runtime_a.orchestrator.cancel(&scheduled_run));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let terminal = runtime_a
                .orchestrator
                .status(&scheduled_run)
                .is_some_and(|status| {
                    matches!(
                        status.state(),
                        crate::command_brief::types::BriefRunState::Cancelled
                            | crate::command_brief::types::BriefRunState::Failed
                    )
                });
            if terminal {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner A terminal runtime");
    let directory = tempfile::tempdir().expect("temp");
    let store_path = directory.path().join("brief.db");
    let conn =
        crate::command_brief::store::open_command_brief_store(&store_path).expect("store");
    let callbacks = AtomicUsize::new(0);
    let on_started = |_: &str| {
        callbacks.fetch_add(1, Ordering::SeqCst);
    };
    {
        let runtimes = vec![Arc::clone(&runtime_a), Arc::clone(&runtime_b)];
        let starter = super::super::OrchestratorStarter {
            state: &state,
            owner_pubkey: &owner_b,
            current: &runtime_b.orchestrator,
            runtimes: &runtimes,
            store_path: &store_path,
            on_started: &on_started,
        };
        assert_eq!(
            starter.presence(&scheduled_run),
            ScheduledRunPresence::Terminal,
            "another owner's terminal in-memory run must close the global date claim"
        );
    }
    conn.execute(
        "INSERT INTO command_brief_spool(
            owner_pubkey,run_id,event_id,status,previous_event_id,
            encrypted_payload,raw_event,publish_state,retry_count,next_retry_at,
            last_error_code,created_at,append_sequence,published_at
         ) VALUES (?1,?2,'event-a','completed',NULL,'ciphertext','{}','queued',0,0,NULL,1,0,NULL)",
        rusqlite::params!["owner-a", scheduled_run],
    )
    .expect("owner A terminal");

    {
        let runtimes = vec![Arc::clone(&runtime_b)];
        let starter = super::super::OrchestratorStarter {
            state: &state,
            owner_pubkey: &owner_b,
            current: &runtime_b.orchestrator,
            runtimes: &runtimes,
            store_path: &store_path,
            on_started: &on_started,
        };
        assert_eq!(
            starter.presence(&scheduled_run),
            ScheduledRunPresence::Terminal,
            "another owner's durable terminal must close the global date claim"
        );
        let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");
        assert_eq!(
            process_due_schedule(
                &conn,
                &schedule,
                utc("2026-07-25T01:00:00Z"),
                ScheduleTrigger::Startup,
                &ReadinessSnapshot::ready("global-terminal"),
                &starter,
            )
            .expect("terminal reconciliation"),
            ScheduleRunOutcome::AlreadyClaimed
        );
        assert_eq!(claim_state(&store_path), ("completed".to_string(), None));
    }
    assert_eq!(callbacks.load(Ordering::SeqCst), 0);
    assert!(runtime_b.orchestrator.status(&scheduled_run).is_none());
}

#[tokio::test]
async fn owner_a_started_daily_claim_blocks_owner_b_duplicate_without_leaking_status() {
    let state = crate::app_state::build_app_state();
    let owner_a = state.signing_keys().expect("owner A").public_key().to_hex();
    let owner_b_keys = nostr::Keys::generate();
    let owner_b = owner_b_keys.public_key().to_hex();
    let effects_a = Arc::new(ScheduledEffects::default());
    let effects_b = Arc::new(ScheduledEffects::default());
    let runtime_a = owner_runtime(
        &owner_a,
        1,
        Arc::clone(&effects_a),
    );
    let runtime_b = owner_runtime(
        &owner_b,
        2,
        Arc::clone(&effects_b),
    );
    let scheduled_run =
        crate::command_brief::schedule::deterministic_run_id("daily-command-brief:2026-07-25");
    let directory = tempfile::tempdir().expect("temp");
    let store_path = directory.path().join("brief.db");
    let conn = crate::command_brief::store::open_command_brief_store(&store_path)
        .expect("owner A store");
    let schedule = load_or_create_schedule(&conn, "Australia/Sydney", 1).expect("schedule");

    let owner_a_outcome = super::super::process_scheduled_runtime(
        super::super::ScheduledRuntimeRequest {
            state: &state,
            expected_owner_pubkey: &owner_a,
            conn,
            schedule: &schedule,
            now: utc("2026-07-25T01:00:00Z"),
            trigger: ScheduleTrigger::Startup,
            store_path: &store_path,
        },
        {
            let runtime_a = Arc::clone(&runtime_a);
            async {
                state
                    .command_brief_runtimes
                    .write()
                    .await
                    .install(Arc::clone(&runtime_a));
                Ok(runtime_a)
            }
        },
        |_| {},
    )
    .await
    .expect("owner A schedule");
    assert_eq!(
        owner_a_outcome,
        ScheduleRunOutcome::Started {
            run_id: scheduled_run.clone(),
        }
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while effects_a.sources.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner A source collection started");

    *state.keys.lock().expect("identity") = owner_b_keys;
    let owner_b_outcome = super::super::process_scheduled_runtime(
        super::super::ScheduledRuntimeRequest {
            state: &state,
            expected_owner_pubkey: &owner_b,
            conn: crate::command_brief::store::open_command_brief_store(&store_path)
                .expect("owner B store"),
            schedule: &schedule,
            now: utc("2026-07-25T01:00:00Z"),
            trigger: ScheduleTrigger::Readiness,
            store_path: &store_path,
        },
        {
            let runtime_b = Arc::clone(&runtime_b);
            async {
                state
                    .command_brief_runtimes
                    .write()
                    .await
                    .install(Arc::clone(&runtime_b));
                Ok(runtime_b)
            }
        },
        |_| {},
    )
    .await
    .expect("owner B reconciliation");
    assert_eq!(owner_b_outcome, ScheduleRunOutcome::AlreadyClaimed);
    assert_eq!(claim_state(&store_path), ("started".to_string(), None));
    assert_eq!(effects_a.sources.load(Ordering::SeqCst), 1);
    assert_eq!(effects_a.advisers.load(Ordering::SeqCst), 0);
    assert_eq!(effects_a.audits.load(Ordering::SeqCst), 0);
    assert_eq!(effects_a.publications.load(Ordering::SeqCst), 0);
    assert_eq!(effects_b.sources.load(Ordering::SeqCst), 0);
    assert_eq!(effects_b.advisers.load(Ordering::SeqCst), 0);
    assert_eq!(effects_b.audits.load(Ordering::SeqCst), 0);
    assert_eq!(effects_b.publications.load(Ordering::SeqCst), 0);
    assert!(runtime_b.orchestrator.status(&scheduled_run).is_none());
    {
        let runtimes = state.command_brief_runtimes.read().await;
        assert!(runtimes.status(&owner_b, &scheduled_run).is_none());
        assert!(runtimes.latest_status_and_history(&owner_b).is_none());
        assert!(runtimes
            .history_after(&owner_b, &scheduled_run, None)
            .is_empty());
        assert!(!runtimes.cancel(&owner_b, &scheduled_run));
    }
    assert!(runtime_a.orchestrator.cancel(&scheduled_run));
}

#[tokio::test]
async fn runtime_views_and_cancellation_never_signal_another_owner_run() {
    let make = |owner: &str, generation| {
        let config = owner_identity(owner, "qwen", "snapshot-a", "apple-a", 1);
        let scheduler = LocalModelScheduler::new(1).expect("scheduler");
        Arc::new(InstalledCommandBriefRuntime {
            owner_pubkey: owner.to_string(),
            config,
            generation,
            orchestrator: CommandBriefOrchestrator::new(
                scheduler,
                Arc::new(UnusedProvider),
                Arc::new(UnusedProvider),
                Arc::new(UnusedProvider),
            ),
        })
    };
    let request = || {
        CommandBriefRequest::new("daily-command-brief", "prepare", "2026-07-25T06:00:00Z")
            .expect("request")
    };
    let owner_a = "owner-a";
    let owner_b = "owner-b";
    let runtime_a = make(owner_a, 1);
    runtime_a
        .orchestrator
        .start_exact("owner-a-run", request())
        .expect("queued owner A run");
    let mut runtimes = CommandBriefRuntimeSet::default();
    runtimes.install(Arc::clone(&runtime_a));

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if runtimes
                .status(owner_a, "owner-a-run")
                .is_some_and(|status| {
                    status.state() == crate::command_brief::types::BriefRunState::CollectingSources
                })
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner A active");

    assert!(runtimes.status(owner_a, "owner-a-run").is_some());
    assert!(runtimes.status(owner_b, "owner-a-run").is_none());
    assert!(runtimes.latest_status_and_history(owner_b).is_none());
    assert!(runtimes
        .history_after(owner_b, "owner-a-run", None)
        .is_empty());
    assert!(!runtimes.cancel(owner_b, "owner-a-run"));
    assert!(
        runtimes.status(owner_a, "owner-a-run").is_some(),
        "a denied owner-B cancellation must not affect owner A"
    );

    assert!(runtimes.cancel(owner_a, "owner-a-run"));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let terminal = runtimes
                .status(owner_a, "owner-a-run")
                .is_some_and(|status| {
                    matches!(
                        status.state(),
                        crate::command_brief::types::BriefRunState::Cancelled
                            | crate::command_brief::types::BriefRunState::Failed
                    )
                });
            if terminal {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner A terminal");

    assert!(runtimes.status(owner_b, "owner-a-run").is_none());
    assert!(runtimes.latest_status_and_history(owner_b).is_none());
    assert!(runtimes
        .history_after(owner_b, "owner-a-run", None)
        .is_empty());
    assert!(!runtimes.cancel(owner_b, "owner-a-run"));

    let all = runtimes.history_after(owner_a, "owner-a-run", None);
    assert_eq!(
        all.iter()
            .map(|status| status.sequence())
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "fast queued, active, and terminal transitions must remain ordered"
    );
    assert_eq!(
        runtimes
            .history_after(owner_a, "owner-a-run", Some(0))
            .iter()
            .map(|status| status.sequence())
            .collect::<Vec<_>>(),
        vec![1, 2],
        "a cursor must return every unseen transition exactly once"
    );
}
