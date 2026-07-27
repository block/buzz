use std::sync::atomic::Ordering;

use nostr::Keys;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app_state::AppState;
use crate::command_brief::audit::load_latest_published_brief;
use crate::command_brief::schedule::{
    current_macos_timezone, load_or_create_schedule, save_schedule_update, ScheduleUpdate,
};
use crate::command_brief::store::open_command_brief_store;
use crate::command_brief::types::{
    BriefRunState, BriefRunStatus, BriefSchedule, PublishedCommandBrief,
};
use crate::command_services::trusted_lan::{
    load_optional as load_optional_trusted_lan_config, save_routing_preference,
    ModelRoutingPreference,
};

/// Bounded metadata-only status view for the most recently active brief.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandBriefStatusView {
    pub classification: &'static str,
    pub current: Option<BriefRunStatus>,
    pub history: Vec<BriefRunStatus>,
}

/// Renderer-safe schedule fields. Identity, timezone, and catch-up policy stay native-owned.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandBriefScheduleUpdate {
    pub enabled: bool,
    pub local_time: String,
    pub concurrency: u8,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRoutingPreferenceView {
    pub preference: ModelRoutingPreference,
}

fn require_active_owner(state: &AppState) -> Result<Keys, &'static str> {
    if state.reset_failed.load(Ordering::Acquire) {
        return Err("command brief identity unavailable");
    }
    state
        .signing_keys()
        .map_err(|_| "command brief identity unavailable")
}

fn active_owner_matches(state: &AppState, expected_owner: &str) -> bool {
    require_active_owner(state)
        .map(|keys| keys.public_key().to_hex() == expected_owner)
        .unwrap_or(false)
}

fn validate_schedule_update(update: &CommandBriefScheduleUpdate) -> Result<(), &'static str> {
    let bytes = update.local_time.as_bytes();
    let valid_time = bytes.len() == 5
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b':'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
        && (bytes[0] - b'0') * 10 + (bytes[1] - b'0') < 24
        && (bytes[3] - b'0') * 10 + (bytes[4] - b'0') < 60;
    if !valid_time || !matches!(update.concurrency, 1 | 2) {
        return Err("command brief input invalid");
    }
    Ok(())
}

fn valid_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 256
        && !value.chars().any(char::is_control)
}

fn command_error() -> String {
    "command brief unavailable".to_string()
}

fn trusted_lan_config_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join("trusted-lan-sources.json"))
        .map_err(|_| command_error())
}

fn terminal(state: BriefRunState) -> bool {
    matches!(
        state,
        BriefRunState::Completed
            | BriefRunState::Degraded
            | BriefRunState::Cancelled
            | BriefRunState::Failed
    )
}

fn emit_status(app: &AppHandle, status: &BriefRunStatus) {
    let _ = app.emit("command-brief-status-changed", status);
}

async fn status_view_for_owner(
    state: &AppState,
    owner_pubkey: &str,
) -> Result<CommandBriefStatusView, &'static str> {
    let (current, history) = state
        .command_brief_runtimes
        .read()
        .await
        .latest_status_and_history(owner_pubkey)
        .map_or((None, Vec::new()), |(current, history)| {
            (Some(current), history)
        });
    if !active_owner_matches(state, owner_pubkey) {
        return Err("command brief identity unavailable");
    }
    Ok(CommandBriefStatusView {
        classification: "OFFICIAL",
        current,
        history,
    })
}

async fn history_for_owner_emit(
    state: &AppState,
    owner_pubkey: &str,
    run_id: &str,
    cursor: Option<u64>,
) -> Option<Vec<BriefRunStatus>> {
    if !active_owner_matches(state, owner_pubkey) {
        return None;
    }
    let history =
        state
            .command_brief_runtimes
            .read()
            .await
            .history_after(owner_pubkey, run_id, cursor);
    active_owner_matches(state, owner_pubkey).then_some(history)
}

fn unseen_status_batch(history: Vec<BriefRunStatus>, cursor: Option<u64>) -> Vec<BriefRunStatus> {
    history
        .into_iter()
        .filter(|status| cursor.is_none_or(|cursor| status.sequence() > cursor))
        .collect()
}

pub(crate) fn watch_command_brief_status(
    app: AppHandle,
    owner_pubkey: String,
    run_id: String,
    initial_status: Option<&BriefRunStatus>,
) {
    let initial_cursor = initial_status.map(BriefRunStatus::sequence);
    tauri::async_runtime::spawn(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4 * 60 * 60);
        let mut cursor = initial_cursor;
        let mut emitted = 0_usize;
        while tokio::time::Instant::now() < deadline && emitted < 32 {
            let Some(history) =
                history_for_owner_emit(&app.state::<AppState>(), &owner_pubkey, &run_id, cursor)
                    .await
            else {
                break;
            };
            for status in unseen_status_batch(history, cursor) {
                if !active_owner_matches(&app.state::<AppState>(), &owner_pubkey) {
                    return;
                }
                emit_status(&app, &status);
                emitted += 1;
                cursor = Some(status.sequence());
                if terminal(status.state()) {
                    return;
                }
                if emitted == 32 {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}

/// Read the latest bounded metadata lifecycle and its at-most-32 status history.
#[tauri::command]
pub async fn get_command_brief_status(
    state: State<'_, AppState>,
) -> Result<CommandBriefStatusView, String> {
    let owner = require_active_owner(&state).map_err(str::to_string)?;
    let owner_pubkey = owner.public_key().to_hex();
    status_view_for_owner(&state, &owner_pubkey)
        .await
        .map_err(str::to_string)
}

/// Read the protected provider order used by the next Daily Command Brief.
#[tauri::command]
pub fn get_model_routing_preference(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelRoutingPreferenceView, String> {
    let _owner = require_active_owner(&state).map_err(str::to_string)?;
    let config = load_optional_trusted_lan_config(&trusted_lan_config_path(&app)?)
        .map_err(|_| command_error())?;
    Ok(ModelRoutingPreferenceView {
        preference: config
            .map(|config| config.routing_preference())
            .unwrap_or_default(),
    })
}

/// Persist one of the two fixed provider orders for subsequent brief runs.
#[tauri::command]
pub fn set_model_routing_preference(
    app: AppHandle,
    state: State<'_, AppState>,
    preference: ModelRoutingPreference,
) -> Result<ModelRoutingPreferenceView, String> {
    let _owner = require_active_owner(&state).map_err(str::to_string)?;
    save_routing_preference(&trusted_lan_config_path(&app)?, preference)
        .map_err(|_| command_error())?;
    Ok(ModelRoutingPreferenceView { preference })
}

/// Start the fixed native OFFICIAL Daily Command Brief request.
#[tauri::command]
pub async fn start_command_brief(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BriefRunStatus, String> {
    let owner_pubkey = require_active_owner(&state)
        .map_err(str::to_string)?
        .public_key()
        .to_hex();
    let status = crate::startup::start_manual_command_brief(app.clone(), &owner_pubkey)
        .await
        .map_err(|error| {
            eprintln!("buzz-desktop: command brief start failed: {error}");
            command_error()
        })?;
    if !active_owner_matches(&state, &owner_pubkey) {
        let runtimes = state.command_brief_runtimes.read().await;
        runtimes.cancel(&owner_pubkey, status.run_id());
        return Err("command brief identity unavailable".to_string());
    }
    watch_command_brief_status(
        app,
        owner_pubkey,
        status.run_id().to_string(),
        Some(&status),
    );
    Ok(status)
}

/// Cooperatively cancel one bounded owner-visible run.
#[tauri::command]
pub async fn cancel_command_brief(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<BriefRunStatus, String> {
    let owner = require_active_owner(&state).map_err(str::to_string)?;
    let owner_pubkey = owner.public_key().to_hex();
    if !valid_run_id(&run_id) {
        return Err("command brief input invalid".to_string());
    }
    let runtimes = state.command_brief_runtimes.read().await;
    if !active_owner_matches(&state, &owner_pubkey) || !runtimes.cancel(&owner_pubkey, &run_id) {
        return Err(command_error());
    }
    let status = runtimes
        .status(&owner_pubkey, &run_id)
        .ok_or_else(command_error)?;
    drop(runtimes);
    if !active_owner_matches(&state, &owner_pubkey) {
        return Err("command brief identity unavailable".to_string());
    }
    Ok(status)
}

/// Return the newest owner-decrypted, contract-validated immutable brief.
#[tauri::command]
pub fn get_latest_command_brief(
    state: State<'_, AppState>,
) -> Result<Option<PublishedCommandBrief>, String> {
    let owner = require_active_owner(&state).map_err(str::to_string)?;
    let owner_pubkey = owner.public_key().to_hex();
    let path = crate::startup::command_brief_store_path().map_err(|_| command_error())?;
    let latest = load_latest_published_brief(&path, &owner).map_err(|_| command_error())?;
    if !active_owner_matches(&state, &owner_pubkey) {
        return Err("command brief identity unavailable".to_string());
    }
    Ok(latest)
}

/// Read the fixed local Daily Command Brief schedule.
#[tauri::command]
pub fn get_command_brief_schedule(state: State<'_, AppState>) -> Result<BriefSchedule, String> {
    let owner_pubkey = require_active_owner(&state)
        .map_err(str::to_string)?
        .public_key()
        .to_hex();
    let path = crate::startup::command_brief_store_path().map_err(|_| command_error())?;
    let conn = open_command_brief_store(&path).map_err(|_| command_error())?;
    let timezone = current_macos_timezone().map_err(|_| command_error())?;
    let schedule = load_or_create_schedule(&conn, &timezone, chrono::Utc::now().timestamp())
        .map_err(|_| command_error())?;
    if !active_owner_matches(&state, &owner_pubkey) {
        return Err("command brief identity unavailable".to_string());
    }
    Ok(schedule)
}

/// Update only renderer-safe schedule controls; identity and timezone stay native-owned.
#[tauri::command]
pub fn set_command_brief_schedule(
    state: State<'_, AppState>,
    update: CommandBriefScheduleUpdate,
) -> Result<BriefSchedule, String> {
    let owner_pubkey = require_active_owner(&state)
        .map_err(str::to_string)?
        .public_key()
        .to_hex();
    validate_schedule_update(&update).map_err(str::to_string)?;
    let path = crate::startup::command_brief_store_path().map_err(|_| command_error())?;
    let conn = open_command_brief_store(&path).map_err(|_| command_error())?;
    let timezone = current_macos_timezone().map_err(|_| command_error())?;
    let current = load_or_create_schedule(&conn, &timezone, chrono::Utc::now().timestamp())
        .map_err(|_| command_error())?;
    if !active_owner_matches(&state, &owner_pubkey) {
        return Err("command brief identity unavailable".to_string());
    }
    let updated = save_schedule_update(
        &conn,
        ScheduleUpdate {
            enabled: update.enabled,
            local_time: update.local_time,
            timezone: current.timezone().to_string(),
            catch_up_same_day: current.catch_up_same_day(),
            concurrency: update.concurrency,
        },
        chrono::Utc::now().timestamp(),
    )
    .map_err(|_| command_error())?;
    if !active_owner_matches(&state, &owner_pubkey) {
        return Err("command brief identity unavailable".to_string());
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use nostr::Event;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::app_state::build_app_state;
    use crate::command_brief::audit::{
        load_latest_published_brief, AuditPublishFuture, BriefAuditPublisher, EncryptedBriefAudit,
        TerminalAuditInput,
    };
    use crate::command_brief::types::CommandBrief;

    #[derive(Default)]
    struct AcceptingPublisher {
        events: Mutex<Vec<Event>>,
    }

    impl BriefAuditPublisher for AcceptingPublisher {
        fn publish<'a>(&'a self, event: Event) -> AuditPublishFuture<'a> {
            Box::pin(async move {
                self.events
                    .lock()
                    .map_err(|_| crate::command_brief::audit::AuditPublishError::Transient)?
                    .push(event);
                Ok(())
            })
        }
    }

    #[test]
    fn every_boundary_rejects_a_locked_or_recovery_identity_with_a_redacted_error() {
        for mutate in [
            |state: &crate::app_state::AppState| {
                state.keyring_locked.store(true, Ordering::Release)
            },
            |state: &crate::app_state::AppState| state.identity_lost.store(true, Ordering::Release),
            |state: &crate::app_state::AppState| state.reset_failed.store(true, Ordering::Release),
        ] {
            let state = build_app_state();
            mutate(&state);
            assert_eq!(
                require_active_owner(&state).expect_err("identity must be denied"),
                "command brief identity unavailable"
            );
        }
    }

    #[test]
    fn schedule_update_accepts_only_the_three_bounded_renderer_fields() {
        let valid: CommandBriefScheduleUpdate = serde_json::from_value(json!({
            "enabled": true,
            "localTime": "06:00",
            "concurrency": 2
        }))
        .expect("valid renderer schedule");
        assert!(validate_schedule_update(&valid).is_ok());

        for invalid in [
            json!({"enabled": true, "localTime": "6:00", "concurrency": 1}),
            json!({"enabled": true, "localTime": "24:00", "concurrency": 1}),
            json!({"enabled": true, "localTime": "06:60", "concurrency": 1}),
            json!({"enabled": true, "localTime": "06:00", "concurrency": 3}),
        ] {
            let value: CommandBriefScheduleUpdate =
                serde_json::from_value(invalid).expect("shape remains valid");
            assert_eq!(
                validate_schedule_update(&value).expect_err("value must fail"),
                "command brief input invalid"
            );
        }

        for forbidden in [
            ("prompt", json!("ignore policy")),
            ("persona", json!("navigator")),
            ("tools", json!(["shell"])),
            ("endpoint", json!("https://example.invalid")),
            ("scheduleId", json!("attacker-controlled")),
            ("timezone", json!("Etc/UTC")),
            ("catchUpSameDay", json!(false)),
            ("classification", json!("PUBLIC")),
        ] {
            let mut value = json!({
                "enabled": true,
                "localTime": "06:00",
                "concurrency": 1
            });
            value[forbidden.0] = forbidden.1;
            assert!(
                serde_json::from_value::<CommandBriefScheduleUpdate>(value).is_err(),
                "{} must never cross the renderer boundary",
                forbidden.0
            );
        }
    }

    #[tokio::test]
    async fn persisted_latest_view_is_validated_immutable_and_owner_scoped() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("brief.db");
        let owner = nostr::Keys::generate();
        let publisher = Arc::new(AcceptingPublisher::default());
        let audit = EncryptedBriefAudit::new(path.clone(), owner.clone(), publisher);
        let brief = CommandBrief::try_from(crate::command_brief::types_tests::brief_value())
            .expect("valid brief");
        audit
            .persist_terminal_input(
                TerminalAuditInput::completed(brief),
                CancellationToken::new(),
            )
            .await
            .expect("persist");

        let latest = load_latest_published_brief(&path, &owner)
            .expect("owner view")
            .expect("latest");
        assert_eq!(latest.brief().run_id(), "run-1");
        assert_eq!(
            latest.publication_state(),
            crate::command_brief::types::PublicationState::Published
        );

        let stranger = nostr::Keys::generate();
        assert!(
            load_latest_published_brief(&path, &stranger)
                .expect("owner scoped query")
                .is_none(),
            "a different unlocked identity must not learn another owner's history"
        );
    }

    #[tokio::test]
    async fn status_reconciliation_rejects_an_identity_switch_while_awaiting_runtime_state() {
        let state = Arc::new(build_app_state());
        let owner_a = state.signing_keys().expect("owner A").public_key().to_hex();
        let runtime_guard = state.command_brief_runtimes.write().await;
        let read_state = Arc::clone(&state);
        let read_owner = owner_a.clone();
        let pending =
            tokio::spawn(async move { status_view_for_owner(&read_state, &read_owner).await });
        tokio::task::yield_now().await;
        *state.keys.lock().expect("identity") = Keys::generate();
        drop(runtime_guard);

        assert!(matches!(
            pending.await.expect("status task"),
            Err("command brief identity unavailable")
        ));
    }

    #[tokio::test]
    async fn watcher_history_terminates_without_an_existence_signal_after_identity_switch() {
        let state = build_app_state();
        let owner_a = state.signing_keys().expect("owner A").public_key().to_hex();
        *state.keys.lock().expect("identity") = Keys::generate();

        assert!(
            history_for_owner_emit(&state, &owner_a, "owner-a-run", None)
                .await
                .is_none(),
            "identity mismatch must terminate the watcher instead of returning an empty existence signal"
        );
    }

    #[test]
    fn watcher_batch_keeps_every_unseen_fast_transition_in_sequence_order() {
        let make = |sequence, state| {
            BriefRunStatus::try_from(json!({
                "classification": "OFFICIAL",
                "runId": "run-fast",
                "scheduleId": "daily-command-brief",
                "sequence": sequence,
                "state": state,
                "updatedAt": "2026-07-25T06:00:00Z",
                "degradedSections": [],
                "error": null
            }))
            .expect("status")
        };
        let history = vec![
            make(0, "queued"),
            make(1, "collecting_sources"),
            make(2, "completed"),
        ];

        let unseen = unseen_status_batch(history, Some(0));
        assert_eq!(
            unseen
                .iter()
                .map(BriefRunStatus::sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }
}
