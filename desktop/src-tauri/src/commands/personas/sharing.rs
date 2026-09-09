use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_personas,
        retention::{mark_synced, open_retention_db},
        storage::managed_agents_base_dir,
        AgentDefinition,
    },
};

use super::pending::{prepare_persona_publication, PreparedPersonaPublication};

/// Test-only observer called immediately after the `managed_agents_store_lock`
/// is acquired in `publish_and_refresh_teams_at`'s refresh section. Tests use
/// this to assert `try_lock()` fails — proving the lock is held during the
/// synchronous refresh. Moving the lock acquisition to AFTER the refresh call
/// (recreating the TOCTOU race) causes `try_lock()` to succeed, turning the
/// probe test RED.
#[cfg(test)]
type RefreshLockObserver = Box<dyn Fn(&AppState) + Send>;
#[cfg(test)]
pub(crate) static REFRESH_LOCK_OBSERVER: std::sync::Mutex<Option<RefreshLockObserver>> =
    std::sync::Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PersonaSharePublicationStatus {
    Published,
    Queued,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPersonaSharedResult {
    pub persona: AgentDefinition,
    pub publication_status: PersonaSharePublicationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_message: Option<String>,
}

#[tauri::command]
pub async fn set_persona_shared(
    id: String,
    shared: bool,
    app: AppHandle,
) -> Result<SetPersonaSharedResult, String> {
    let prepared = tokio::task::spawn_blocking({
        let app = app.clone();
        move || {
            let state = app.state::<AppState>();
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let personas = load_personas(&app)?;
            let persona = personas
                .iter()
                .find(|record| record.id == id)
                .ok_or_else(|| format!("agent {id} not found"))?;

            if persona.is_builtin {
                return Err("Built-in agents cannot be shared to the catalog.".to_string());
            }

            // Strict path: unlike ordinary definition saves, an enqueue failure
            // for this privacy-sensitive toggle must reach the command/UI.
            prepare_persona_publication(&app, &state, persona, Some(shared))
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    // Save persona id before `prepared` is consumed by publish_prepared_persona.
    let persona_id = prepared.persona.id.clone();
    let base_dir = managed_agents_base_dir(&app)?;
    let keys = prepared.scope.owner_keys.clone();
    let db_path = prepared.scope.db_path.clone();
    let state = app.state::<AppState>();
    publish_and_refresh_teams_at(&state, prepared, &base_dir, &keys, &db_path, &persona_id).await
}

/// Save a persona edit AND publish its catalog head, returning the same
/// `published | queued` outcome as [`set_persona_shared`].
///
/// The "save and publish" affordance in the edit dialog promises the change
/// reaches the catalog on save. Plain `update_persona` only enqueues
/// best-effort, so the UI could not report whether the relay accepted it. This
/// takes the identical input and reuses the strict preparation path, then awaits
/// the relay exactly like the share toggle does — a rejection or an unreachable
/// relay stays durably queued for the flush loop and is reported as `queued`.
#[tauri::command]
pub async fn update_persona_and_publish(
    input: crate::managed_agents::UpdatePersonaRequest,
    app: AppHandle,
) -> Result<SetPersonaSharedResult, String> {
    update_persona_and_publish_inner(input, app).await
}

/// Generic core of [`update_persona_and_publish`], testable with
/// `tauri::test::MockRuntime` as well as the production `Wry` runtime.
///
/// Extracted so tests can invoke the real command path — including the
/// `prepare_persona_publication` scope resolver — through a mock `AppHandle`
/// without the `#[tauri::command]` signature binding to `AppHandle<Wry>`.
pub(crate) async fn update_persona_and_publish_inner<R: tauri::Runtime>(
    input: crate::managed_agents::UpdatePersonaRequest,
    app: AppHandle<R>,
) -> Result<SetPersonaSharedResult, String> {
    let (_, prepared) =
        super::update::update_persona_with(input, app.clone(), |app, state, persona| {
            // Strict path: this command's contract is to report the publication
            // outcome, so an enqueue failure must reach the UI rather than being
            // logged and swallowed.
            let result = prepare_persona_publication(app, state, persona, None)?;
            // F2: refresh any shared 30178 heads that include this persona.
            crate::commands::refresh_team_catalog_heads_for_persona(app, state, &persona.id);
            Ok(result)
        })
        .await?;

    let state = app.state::<AppState>();
    publish_prepared_persona(&state, prepared).await
}

/// Publish a prepared persona head and refresh any shared 30178 team heads that
/// include this persona — the combined contract shared by the share toggle and
/// the publish-retry seam.
///
/// Extracted from [`set_persona_shared`] so this two-step sequence can be
/// tested directly through `publish_and_refresh_teams_at` without a
/// `tauri::AppHandle`. Deleting the [`refresh_for_persona_at`] call from this
/// function must cause the command-path regression to fail.
pub(crate) async fn publish_and_refresh_teams_at(
    state: &AppState,
    prepared: PreparedPersonaPublication,
    base_dir: &std::path::Path,
    keys: &nostr::Keys,
    db_path: &std::path::Path,
    persona_id: &str,
) -> Result<SetPersonaSharedResult, String> {
    let result = publish_prepared_persona(state, prepared).await?;
    // F2: refresh any shared 30178 heads that include this persona. The refresh
    // reads the current team/persona definitions and may retain a new head — it
    // must be serialized with team edit/unshare/delete operations that also hold
    // `managed_agents_store_lock` and may retain/retract the same head.
    //
    // Without the lock a concurrent `set_team_shared(false)` can:
    //   1. acquire the lock, retain unshared head T+1, release the lock;
    //   2. refresh (unlocked) reads old shared head T, rebuilds, retains T+1;
    //      `retain_event` accepts equal timestamps — the refresh wins, undoing
    //      the explicit unshare.
    //
    // Acquire AFTER the network await so the lock is never held across I/O;
    // the synchronous refresh completes entirely inside the critical section.
    {
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| format!("managed_agents_store_lock poisoned: {e}"))?;
        #[cfg(test)]
        {
            if let Ok(obs) = REFRESH_LOCK_OBSERVER.lock() {
                if let Some(ref f) = *obs {
                    f(state);
                }
            }
        }
        let _ = crate::commands::teams::refresh_for_persona_at(base_dir, keys, db_path, persona_id);
    }
    Ok(result)
}

async fn publish_prepared_persona(
    state: &AppState,
    prepared: PreparedPersonaPublication,
) -> Result<SetPersonaSharedResult, String> {
    let api_base_url = crate::relay::relay_http_base_url(&prepared.scope.relay_url);
    let publish_result = crate::relay::submit_signed_event_at_with_keys(
        &prepared.event,
        state,
        &api_base_url,
        &prepared.scope.owner_keys,
    )
    .await;

    match publish_result {
        Ok(_) => {
            let conn = open_retention_db(&prepared.scope.db_path)?;
            mark_synced(
                &conn,
                prepared.retained.kind,
                &prepared.retained.pubkey,
                &prepared.retained.d_tag,
                prepared.retained.created_at,
                &prepared.retained.content,
            )?;
            Ok(SetPersonaSharedResult {
                persona: prepared.persona,
                publication_status: PersonaSharePublicationStatus::Published,
                relay_message: None,
            })
        }
        Err(error) => Ok(SetPersonaSharedResult {
            persona: prepared.persona,
            publication_status: PersonaSharePublicationStatus::Queued,
            relay_message: Some(error),
        }),
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::*;
    use crate::{
        app_state::build_app_state,
        commands::personas::pending::prepare_persona_publication_at,
        managed_agents::{
            retention::{get_retained_event, open_retention_db, RetentionScope},
            save_managed_agents, save_personas, AgentDefinition, ManagedAgentRecord,
            UpdatePersonaRequest,
        },
    };
    use std::collections::BTreeMap;
    fn persona() -> AgentDefinition {
        AgentDefinition {
            description: None,
            id: "catalog-reviewer".to_string(),
            display_name: "Catalog Reviewer".to_string(),
            avatar_url: None,
            system_prompt: "Review the catalog.".to_string(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "2026-07-27T00:00:00Z".to_string(),
            updated_at: "2026-07-27T00:00:00Z".to_string(),
        }
    }

    async fn spawn_relay(accepted: bool) -> String {
        use axum::{routing::post, Router};

        let app = Router::new().route(
            "/events",
            post(move |body: String| async move {
                let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                serde_json::json!({
                    "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                    "accepted": accepted,
                    "message": if accepted { "" } else { "policy rejection" }
                })
                .to_string()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{addr}")
    }

    fn prepared(
        db_path: &std::path::Path,
        relay_url: String,
        keys: nostr::Keys,
        shared_override: Option<bool>,
    ) -> PreparedPersonaPublication {
        let (event, retained, persona) =
            prepare_persona_publication_at(db_path, &keys, &persona(), shared_override).unwrap();
        PreparedPersonaPublication {
            scope: RetentionScope {
                db_path: db_path.to_path_buf(),
                relay_url,
                owner_keys: keys,
            },
            event,
            retained,
            persona,
        }
    }

    #[tokio::test]
    async fn relay_rejection_stays_durably_queued() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let prepared = prepared(&db_path, spawn_relay(false).await, keys, Some(true));
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Queued
        );
        assert!(result
            .relay_message
            .as_deref()
            .is_some_and(|message| message.contains("relay rejected event")));
        assert!(
            get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync
        );
    }

    #[tokio::test]
    async fn unavailable_relay_stays_durably_queued() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let prepared = prepared(&db_path, relay_url, keys, Some(true));
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Queued
        );
        assert!(result
            .relay_message
            .as_deref()
            .is_some_and(|message| message.starts_with("relay unreachable:")));
        assert!(
            get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync
        );
    }

    #[tokio::test]
    async fn relay_acceptance_marks_the_scoped_head_synced() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let prepared = prepared(&db_path, spawn_relay(true).await, keys, Some(true));
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Published
        );
        assert!(
            !get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync
        );
    }

    /// `update_persona_and_publish` differs from the share toggle in one way:
    /// it passes no share override, so the edit must keep whatever the scoped
    /// head already says, and it reports the relay outcome to the caller.
    #[tokio::test]
    async fn test_update_and_publish_acceptance_publishes_the_edit_at_the_current_share_state() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        // The persona is already shared in this scope.
        prepare_persona_publication_at(&db_path, &keys, &persona(), Some(true)).unwrap();
        let prepared = prepared(&db_path, spawn_relay(true).await, keys, None);
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Published
        );
        assert!(
            result.persona.shared,
            "an ordinary edit must not silently unshare the persona"
        );
        assert!(
            !get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync
        );
    }

    #[tokio::test]
    async fn test_update_and_publish_relay_rejection_reports_queued_not_failure() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        prepare_persona_publication_at(&db_path, &keys, &persona(), Some(true)).unwrap();
        let prepared = prepared(&db_path, spawn_relay(false).await, keys, None);
        let state = build_app_state();

        let result = publish_prepared_persona(&state, prepared).await.unwrap();

        assert_eq!(
            result.publication_status,
            PersonaSharePublicationStatus::Queued
        );
        assert!(result
            .relay_message
            .as_deref()
            .is_some_and(|message| message.contains("relay rejected event")));
        assert!(
            get_retained_event(
                &open_retention_db(&db_path).unwrap(),
                buzz_core_pkg::kind::KIND_PERSONA,
                &owner,
                "catalog-reviewer"
            )
            .unwrap()
            .unwrap()
            .pending_sync,
            "the edit stays queued for the flush loop"
        );
    }

    /// The save path swallows enqueue failures (`retain_persona_pending` logs
    /// them). This command promises a publication outcome, so the strict
    /// preparation it uses must surface the failure instead.
    #[tokio::test]
    async fn test_update_and_publish_enqueue_failure_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();

        let error = prepare_persona_publication_at(dir.path(), &keys, &persona(), None)
            .expect_err("a directory cannot be opened as the retention database");

        assert!(error.contains("failed to open retention db"));
    }

    /// Build a headless mock app for tests that need a full `AppHandle`.
    ///
    /// Shares the same pattern used in `concurrent_edit_tests.rs`. Use
    /// `lock_path_mutex()` + `HOME`/`XDG_DATA_HOME` overrides around this in
    /// tests that touch file-backed stores.
    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        let state = build_app_state();
        tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds headless")
    }

    /// P2 relay-sync regression through the real `update_persona_and_publish`
    /// command path: when the active retention DB path is unwritable, the relay
    /// kind:0 profile sync for linked agents must still complete before the
    /// error propagates.
    ///
    /// Drives `update_persona_and_publish_inner` — the generic core of the
    /// exported `update_persona_and_publish` command — through a mock AppHandle,
    /// exercising the full wiring: `prepare_persona_publication` scope resolver,
    /// the strict-preparation `?`-propagation, and the phase-2 relay sync. The
    /// failure is induced by replacing the active retention DB path with a
    /// directory (EISDIR) after the relay override is set so the scope hash is
    /// stable.
    ///
    /// Mutation acceptance: restoring `retain_result?` inside
    /// `Ok((result, retain_result?, …))` at the blocking-phase return causes
    /// phase 2 to be skipped → counter receives 0 requests → RED.
    #[test]
    fn test_update_and_publish_relay_profile_syncs_despite_preparation_failure() {
        use crate::managed_agents::{
            load_managed_agents, load_personas, retention::active_retention_scope,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tauri::Manager;

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PartialPublishOutcomeContract {
            persona_id: String,
            before_display_name: String,
            after_display_name: String,
            command_error_contains: String,
            relay_profile_request_count: usize,
            retry_publication_status: String,
        }

        let contract: PartialPublishOutcomeContract = serde_json::from_str(include_str!(
            "../../../../../test-fixtures/update-persona-publish-partial-outcome.json"
        ))
        .expect("shared partial-publish contract must parse");
        assert_eq!(contract.retry_publication_status, "published");

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home_p2_seam");
        std::fs::create_dir_all(&home).unwrap();

        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        let _path_guard = crate::managed_agents::lock_path_mutex();
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &home);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime");

        rt.block_on(async {
            let app = mock_app();

            // Spawn a local HTTP server that counts POST /events requests.
            // sync_managed_agent_profile posts kind:0 profile events here.
            let post_count = Arc::new(AtomicUsize::new(0));
            let post_count_clone = post_count.clone();
            let relay_server = {
                use axum::{routing::post, Router};
                let app_router = Router::new().route(
                    "/events",
                    post(move |_body: String| {
                        let counter = post_count_clone.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            serde_json::json!({
                                "event_id": "test-event-id",
                                "accepted": true,
                                "message": ""
                            })
                            .to_string()
                        }
                    }),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                tokio::spawn(async move {
                    axum::serve(listener, app_router).await.ok();
                });
                format!("http://{addr}")
            };

            // Point the workspace relay override at the counter server.
            // Must be set BEFORE computing the active retention scope so the
            // scope hash is stable for the sabotage step.
            {
                let state = app.state::<AppState>();
                let mut override_slot = state
                    .relay_url_override
                    .lock()
                    .expect("relay_url_override must be lockable");
                *override_slot = Some(relay_server.clone());
            }

            // Seed persona "Alice" at a known revision.
            let r1 = "2026-01-01T00:00:00Z";
            save_personas(
                app.handle(),
                &[AgentDefinition {
                    id: contract.persona_id.clone(),
                    display_name: contract.before_display_name.clone(),
                    updated_at: r1.to_string(),
                    created_at: r1.to_string(),
                    ..persona()
                }],
            )
            .expect("seed must succeed");

            // Seed a linked agent with valid NSEC keys so sync_managed_agent_profile
            // can sign and submit the kind:0 profile event.
            let agent_keys = nostr::Keys::generate();
            let agent_record = ManagedAgentRecord {
                pubkey: agent_keys.public_key().to_hex(),
                name: contract.before_display_name.clone(),
                persona_id: Some(contract.persona_id.clone()),
                private_key_nsec: agent_keys.secret_key().to_secret_hex(),
                auth_tag: None,
                relay_url: String::new(),
                avatar_url: None,
                acp_command: String::new(),
                agent_command: String::new(),
                agent_command_override: None,
                agent_args: vec![],
                mcp_command: String::new(),
                turn_timeout_seconds: 0,
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
                parallelism: 1,
                system_prompt: None,
                model: None,
                provider: None,
                persona_source_version: None,
                env_vars: BTreeMap::new(),
                start_on_app_launch: false,
                auto_restart_on_config_change: false,
                runtime_pid: None,
                backend: Default::default(),
                backend_agent_id: None,
                provider_policy_pending: false,
                provider_binary_path: None,
                team_id: None,
                persona_team_dir: None,
                persona_name_in_team: None,
                created_at: String::new(),
                updated_at: String::new(),
                last_started_at: None,
                last_stopped_at: None,
                last_exit_code: None,
                last_error: None,
                last_error_code: None,
                respond_to: Default::default(),
                respond_to_allowlist: vec![],
                display_name: Some(contract.before_display_name.clone()),
                description: None,
                slug: None,
                runtime: None,
                name_pool: vec![],
                is_builtin: false,
                is_active: true,
                shared: false,
                source_team: None,
                source_team_persona_slug: None,
                catalog_source: None,
                team_catalog_source: None,
                definition_respond_to: None,
                definition_respond_to_allowlist: vec![],
                definition_parallelism: None,
                relay_mesh: None,
                effort_level: None,
            };
            save_managed_agents(app.handle(), &[agent_record])
                .expect("agent seed must succeed");

            // Resolve the active retention scope (relay + owner → DB path) and
            // sabotage it: replace the .db file path with a directory so that
            // `open_retention_db` inside `prepare_persona_publication` returns
            // "failed to open retention db". This exercises the production scope
            // resolver rather than injecting an arbitrary bad path.
            {
                let state = app.state::<AppState>();
                let scope = active_retention_scope(app.handle(), &state)
                    .expect("active_retention_scope must resolve with relay override set");
                // Remove the file if it was created by scope resolution, then
                // create a directory at the same path so SQLite cannot open it.
                std::fs::remove_file(&scope.db_path).ok();
                std::fs::create_dir_all(&scope.db_path)
                    .expect("must be able to create directory at db path for sabotage");
            }

            // Drive the REAL command path through a mock AppHandle. This exercises
            // prepare_persona_publication (scope resolver + ?-propagation) and the
            // phase-2 relay sync, verifying that wiring drift at the command
            // boundary — e.g. update_persona_and_publish stopping to call
            // update_persona_with — is caught.
            let result = update_persona_and_publish_inner(
                UpdatePersonaRequest {
                    id: contract.persona_id.clone(),
                    display_name: contract.after_display_name.clone(),
                    avatar_url: None,
                    description: None,
                    system_prompt: "Do the work.".to_string(),
                    runtime: None,
                    model: None,
                    provider: None,
                    name_pool: Vec::new(),
                    env_vars: None,
                    behavior: None,
                    expected_updated_at: Some(r1.to_string()),
                },
                app.handle().clone(),
            )
            .await;

            // The preparation failure must propagate (coordinator sees publishFailed).
            assert!(
                result.is_err(),
                "update_persona_and_publish_inner must return Err when prepare_persona_publication fails"
            );
            assert!(
                result
                    .as_ref()
                    .unwrap_err()
                    .contains(&contract.command_error_contains),
                "error must come from prepare_persona_publication scope resolver, got: {:?}",
                result
            );

            // Both durable stores must reflect the edit even though strict
            // publication preparation failed after the writes.
            let persisted_personas =
                load_personas(app.handle()).expect("persona store must reload after command error");
            let persisted_persona = persisted_personas
                .iter()
                .find(|persona| persona.id == contract.persona_id)
                .expect("renamed persona must remain in the store");
            assert_eq!(
                persisted_persona.display_name, contract.after_display_name,
                "persona rename must persist before the strict preparation error propagates"
            );

            let persisted_agents = load_managed_agents(app.handle())
                .expect("managed-agent store must reload after command error");
            let persisted_agent = persisted_agents
                .iter()
                .find(|agent| agent.persona_id.as_deref() == Some(contract.persona_id.as_str()))
                .expect("linked managed-agent record must remain in the store");
            assert_eq!(
                persisted_agent.name, contract.after_display_name,
                "linked record name must persist before the strict preparation error propagates"
            );
            assert_eq!(
                persisted_agent.display_name.as_deref(),
                Some(contract.after_display_name.as_str()),
                "linked record display_name must persist with the persona rename"
            );

            // Phase 2 must have run: relay sync fires despite the retain failure.
            // Before the fix (retain_result? inside the blocking Ok): count is 0 → RED.
            // After the fix (retain_result? after phase 2): count is 1 → GREEN.
            assert_eq!(
                post_count.load(Ordering::SeqCst),
                contract.relay_profile_request_count,
                "relay kind:0 profile sync must fire despite prepare_persona_publication failure; \
                 restoring `retain_result?` before phase 2 turns this RED"
            );
        }); // rt.block_on

        // Cleanup
        std::env::remove_var("HOME");
        std::env::remove_var("XDG_DATA_HOME");
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match old_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    /// Behavioral race regression for P1-2: an explicit team unshare that
    /// starts after refresh reads shared T must remain authoritative.
    ///
    /// The observer pauses refresh after its shared-T read. The competing
    /// thread then announces that it is about to acquire the same store lock.
    /// With the production lock in place, refresh must finish before unshare can
    /// acquire; unshare writes last and the final head is unshared. Under the
    /// mutation that moves refresh outside the lock, unshare acquires while
    /// refresh is paused, writes unshared T+1 completely, and only then releases
    /// refresh; refresh writes shared T+1 last and the final-state assertion
    /// turns RED. No lock-presence assertion is involved.
    #[tokio::test]
    async fn test_retry_refresh_holds_store_lock_during_refresh() {
        use crate::commands::teams::{prepare_team_publication_at, REFRESH_READ_OBSERVER};
        use crate::managed_agents::{
            retention::{get_retained_event, open_retention_db},
            TeamRecord,
        };
        use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
        use nostr::JsonUtil;
        use std::collections::BTreeMap;
        use std::sync::{mpsc, Arc};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let relay_url = spawn_relay(true).await;
        let state = Arc::new(build_app_state());

        let persona_id = "catalog-reviewer";
        let team = TeamRecord {
            id: "team-abc".to_string(),
            name: "Test Team".to_string(),
            description: None,
            instructions: None,
            persona_ids: vec![persona_id.to_string()],
            is_builtin: false,
            shared: true,
            catalog_source: None,
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let persona_member = AgentDefinition {
            id: persona_id.to_string(),
            display_name: "Catalog Reviewer".to_string(),
            description: None,
            avatar_url: None,
            system_prompt: "Review.".to_string(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        // Seed the authoritative shared kind:30178 head T.
        prepare_team_publication_at(
            &db_path,
            &keys,
            &team,
            std::slice::from_ref(&persona_member),
            Some(true),
        )
        .expect("seed shared team head must succeed");

        // Seed the real file seam used by refresh_for_persona_at. The changed
        // prompt guarantees the refresh rebuild is not idempotently skipped.
        let teams_json = serde_json::to_string(&[&team]).expect("serialize team");
        std::fs::write(dir.path().join("teams.json"), teams_json.as_bytes()).unwrap();
        let persona_updated = AgentDefinition {
            system_prompt: "Review catalog entries carefully.".to_string(),
            ..persona_member.clone()
        };
        let personas_json =
            serde_json::to_string(&[&persona_updated]).expect("serialize updated persona");
        std::fs::write(dir.path().join("personas.json"), personas_json.as_bytes())
            .expect("write personas.json must succeed");

        let initial_created_at = {
            let conn = open_retention_db(&db_path).unwrap();
            get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-abc")
                .unwrap()
                .expect("seed head must exist")
                .created_at
        };

        // Deterministic two-phase release. The observer first pauses refresh
        // after shared T was read. The unshare thread announces readiness but
        // cannot attempt the lock until the observer probes and releases it.
        let (refresh_read_tx, refresh_read_rx) = mpsc::channel::<()>();
        let (unshare_ready_tx, unshare_ready_rx) = mpsc::channel::<()>();
        let (unshare_go_tx, unshare_go_rx) = mpsc::channel::<()>();
        let (unshare_complete_tx, unshare_complete_rx) = mpsc::channel::<()>();
        let state_observer = state.clone();
        {
            let mut slot = REFRESH_READ_OBSERVER.lock().expect("observer slot");
            *slot = Some(Box::new(move || {
                refresh_read_tx
                    .send(())
                    .expect("unshare thread must receive refresh-read signal");
                unshare_ready_rx
                    .recv()
                    .expect("unshare thread must announce readiness");

                // This probe is ordering control, not the oracle. If the caller
                // holds the production lock, release the unshare to block on it
                // and let refresh finish. If the lock-moved mutation leaves the
                // lock free, release the unshare and require its write to finish
                // before allowing refresh to resume. Only final retained state
                // below decides whether the test passes.
                match state_observer.managed_agents_store_lock.try_lock() {
                    Ok(probe_guard) => {
                        drop(probe_guard);
                        unshare_go_tx
                            .send(())
                            .expect("unshare thread must receive release signal");
                        unshare_complete_rx
                            .recv()
                            .expect("unshare must complete before unlocked refresh resumes");
                    }
                    Err(_) => {
                        unshare_go_tx
                            .send(())
                            .expect("unshare thread must receive release signal");
                    }
                }
            }));
        }

        let state_unshare = state.clone();
        let db_path_unshare = db_path.clone();
        let keys_unshare = keys.clone();
        let team_unshare = team.clone();
        let member_unshare = persona_member.clone();
        let unshare_thread = std::thread::spawn(move || {
            refresh_read_rx
                .recv()
                .expect("refresh must announce its shared-T read");
            unshare_ready_tx
                .send(())
                .expect("observer must receive readiness signal");
            unshare_go_rx
                .recv()
                .expect("observer must release the unshare lock attempt");
            let _guard = state_unshare
                .managed_agents_store_lock
                .lock()
                .expect("unshare lock must not be poisoned");
            prepare_team_publication_at(
                &db_path_unshare,
                &keys_unshare,
                &team_unshare,
                &[member_unshare],
                Some(false),
            )
            .expect("unshare must retain its unshared head");
            // Under correct locking the observer has already returned and
            // drops this receiver; under the lock-moved mutation it is waiting
            // for this completion signal. Both outcomes are intentional.
            let _ = unshare_complete_tx.send(());
        });

        let prepared = prepared(&db_path, relay_url, keys.clone(), Some(true));
        let persona_id_str = prepared.persona.id.clone();
        let result = publish_and_refresh_teams_at(
            &state,
            prepared,
            dir.path(),
            &keys,
            &db_path,
            &persona_id_str,
        )
        .await;

        {
            let mut slot = REFRESH_READ_OBSERVER.lock().expect("observer slot");
            *slot = None;
        }
        unshare_thread
            .join()
            .expect("unshare thread must not panic");
        result.expect("publish_and_refresh_teams_at must succeed");

        let conn = open_retention_db(&db_path).unwrap();
        let retained = get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-abc")
            .expect("db query must not fail")
            .expect("retained team catalog head must exist after refresh + unshare");
        assert!(
            retained.created_at >= initial_created_at,
            "retained head must be at or after the seed timestamp"
        );
        let event =
            nostr::Event::from_json(&retained.raw_event).expect("must parse retained event");
        assert!(
            !event_is_shared(&event),
            "final retained kind:30178 head must be UNSHARED; moving refresh outside \
             managed_agents_store_lock lets the paused refresh overwrite the completed unshare"
        );
    }
}
