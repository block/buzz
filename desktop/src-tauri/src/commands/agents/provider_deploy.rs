use std::sync::Arc;

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        discover_provider_candidates, load_managed_agents, provider_deploy,
        resolve_provider_binary, save_managed_agents, BackendKind, REPLAY_FLOOR_ENV_VAR,
    },
    util::now_iso,
};

use super::build_deploy_payload;

/// Deploy an agent to a provider backend. Resolves the binary, calls deploy via
/// spawn_blocking, and persists the result (backend_agent_id or last_error).
///
/// Idempotency: calling deploy on an already-deployed agent sends the same payload
/// again. Providers are expected to handle this as an update-in-place or no-op.
/// The protocol has no explicit `undeploy` operation or acknowledgement that an
/// existing process stopped, so a successful redeploy delegates access-policy
/// revocation semantics to the provider implementation (deferred to v2).
/// Returns Ok(()) on success, Err(message) on failure. Either way the record is
/// updated and saved before returning.
///
/// Callers with a captured tenant scope (Projects agent starts) pass
/// `expected_relay_url` / `expected_signer_pubkey`; they are asserted against
/// the payload REBUILT after the deploy lock — the exact value invoked — so a
/// workspace or identity switch landing while this call waited behind another
/// deployment fails closed instead of deploying a stale start into the new
/// tenant under the new tenant's owner identity. `None` preserves the
/// unscoped behavior for callers without a tenant boundary.
///
/// `replay_floor_unix`: optional unix-seconds replay floor from a
/// publish-first mention send. It is injected into the rebuilt payload's
/// `launch.policy_env` as `BUZZ_ACP_REPLAY_FLOOR`, so the remote harness's
/// startup watermark replays back past the already-published triggering
/// message exactly like a local spawn. Per-invocation only — never persisted
/// on the record, so later redeploys do not carry a stale floor.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn deploy_to_provider(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    _provider_id: &str,
    _config: &serde_json::Value,
    _agent_json: serde_json::Value,
    _cached_binary_path: Option<&str>,
    expected_relay_url: Option<&str>,
    expected_signer_pubkey: Option<&str>,
    replay_floor_unix: Option<u64>,
) -> Result<(), String> {
    let deploy_lock = {
        let mut locks = state
            .provider_deploy_locks
            .lock()
            .map_err(|error| error.to_string())?;
        Arc::clone(
            locks
                .entry(pubkey.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    };
    let _deploy_guard = deploy_lock.lock().await;
    // The payload may have waited behind another deployment. Rebuild it from
    // the current record so the final provider invocation always carries the
    // newest saved policy rather than the stale snapshot captured by its caller.
    let (provider_id, config, cached_binary_path, mut agent_json) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        let (provider_id, config) = match &record.backend {
            BackendKind::Provider { id, config } => (id.clone(), config.clone()),
            BackendKind::Local => return Err(format!("agent {pubkey} is not provider-backed")),
        };
        (
            provider_id,
            config,
            record.provider_binary_path.clone(),
            build_deploy_payload(app, state, record)?,
        )
    };
    // The rebuild above re-read the live workspace relay and owner identity.
    // Assert the caller's captured scope against THIS payload — the exact
    // value invoked below — not the pre-lock snapshot its caller validated.
    assert_payload_scope(&agent_json, expected_relay_url, expected_signer_pubkey)?;
    // The floor is invocation state, not record state, so the post-lock
    // rebuild cannot restore it — inject it into the payload actually invoked.
    apply_replay_floor(&mut agent_json, replay_floor_unix);
    // Resolve via discovered candidates only. Cached path must match BOTH
    // "is a discovered candidate" AND "belongs to this provider_id". A tampered
    // record cannot redirect deploys to a different provider's binary.
    let bin_path = cached_binary_path
        .as_deref()
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .map(|p| p.canonicalize().unwrap_or(p))
        .filter(|canonical| {
            discover_provider_candidates().iter().any(|(id, cp)| {
                id == &provider_id && cp.canonicalize().ok().as_ref() == Some(canonical)
            })
        })
        .map_or_else(|| resolve_provider_binary(&provider_id), Ok)?;

    let deployed_agent_json = agent_json.clone();
    let config_clone = config.clone();
    let deploy_result =
        tokio::task::spawn_blocking(move || provider_deploy(&bin_path, &agent_json, &config_clone))
            .await
            .map_err(|e| format!("spawn_blocking failed: {e}"))?;

    // Persist result under lock.
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(app)?;
    let rec = records
        .iter_mut()
        .find(|r| r.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;

    let result = apply_deploy_result(rec, deploy_result, &deployed_agent_json);
    save_managed_agents(app, &records)?;
    result
}

/// Assert a caller-captured tenant scope against the payload that will
/// actually be invoked. The relay lives at the payload's top-level
/// `relay_url`; the deploying identity lives at `launch.owner_pubkey` — both
/// were re-resolved from live workspace state by `build_deploy_payload`, so
/// this is the check tied to the use. When the caller carries an expectation
/// a missing payload field fails closed: an unverifiable payload must never
/// deploy on behalf of a scoped callback.
fn assert_payload_scope(
    agent_json: &serde_json::Value,
    expected_relay_url: Option<&str>,
    expected_signer_pubkey: Option<&str>,
) -> Result<(), String> {
    let has_expectation =
        |expected: Option<&str>| expected.map(str::trim).filter(|s| !s.is_empty()).is_some();
    match agent_json.get("relay_url").and_then(|v| v.as_str()) {
        Some(embedded_relay) => crate::relay::assert_expected_relay_scope(
            expected_relay_url,
            &crate::relay::relay_http_base_url(embedded_relay),
        )?,
        None if has_expectation(expected_relay_url) => {
            return Err("deploy payload carries no relay; not deployed".to_string());
        }
        None => {}
    }
    match agent_json
        .get("launch")
        .and_then(|launch| launch.get("owner_pubkey"))
        .and_then(|v| v.as_str())
    {
        Some(owner) => crate::relay::assert_expected_signer(expected_signer_pubkey, owner)?,
        None if has_expectation(expected_signer_pubkey) => {
            return Err("deploy payload carries no owner identity; not deployed".to_string());
        }
        None => {}
    }
    Ok(())
}

/// Inject a caller-supplied replay floor into the deploy payload so the
/// remote harness consumes it exactly like a local spawn: as the
/// [`REPLAY_FLOOR_ENV_VAR`] environment variable. The floor rides
/// `launch.policy_env` (tier 1); any same-named key in `launch.env` (tier 2)
/// is stripped because that tier later-wins and a persisted user value must
/// not shadow this send's floor — the remote mirror of
/// `apply_replay_floor_env`'s post-`descriptor.env` write on the local spawn.
/// With no caller floor the payload is left untouched — a user-supplied
/// `launch.env` value passes through, and plain redeploys never carry a stale
/// floor.
fn apply_replay_floor(agent_json: &mut serde_json::Value, replay_floor_unix: Option<u64>) {
    let Some(floor) = replay_floor_unix else {
        return;
    };
    let Some(launch) = agent_json
        .get_mut("launch")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    if let Some(env) = launch
        .get_mut("env")
        .and_then(serde_json::Value::as_object_mut)
    {
        let shadowed: Vec<String> = env
            .keys()
            .filter(|key| key.eq_ignore_ascii_case(REPLAY_FLOOR_ENV_VAR))
            .cloned()
            .collect();
        for key in shadowed {
            env.remove(&key);
        }
    }
    match launch
        .get_mut("policy_env")
        .and_then(serde_json::Value::as_object_mut)
    {
        Some(policy_env) => {
            policy_env.insert(
                REPLAY_FLOOR_ENV_VAR.to_string(),
                serde_json::Value::String(floor.to_string()),
            );
        }
        None => {
            launch.insert(
                "policy_env".to_string(),
                serde_json::json!({ (REPLAY_FLOOR_ENV_VAR): floor.to_string() }),
            );
        }
    }
}

fn policy_matches_payload(
    record: &crate::managed_agents::ManagedAgentRecord,
    deployed_agent_json: &serde_json::Value,
) -> bool {
    deployed_agent_json
        .get("respond_to")
        .and_then(serde_json::Value::as_str)
        == Some(record.respond_to.as_str())
        && deployed_agent_json.get("respond_to_allowlist")
            == Some(&serde_json::json!(record.respond_to_allowlist))
}

fn apply_deploy_result(
    record: &mut crate::managed_agents::ManagedAgentRecord,
    deploy_result: Result<String, String>,
    deployed_agent_json: &serde_json::Value,
) -> Result<(), String> {
    match deploy_result {
        Ok(backend_agent_id) => {
            record.backend_agent_id = Some(backend_agent_id);
            if policy_matches_payload(record, deployed_agent_json) {
                record.provider_policy_pending = false;
            }
            record.last_started_at = Some(now_iso());
            record.updated_at = now_iso();
            record.last_error = None;
            Ok(())
        }
        Err(error) => {
            record.last_error = Some(error.clone());
            record.updated_at = now_iso();
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> crate::managed_agents::ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": "agent", "name": "Agent", "relay_url": "", "acp_command": "",
            "agent_command": "", "agent_args": [], "mcp_command": "",
            "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
            "updated_at": "", "last_started_at": null, "last_stopped_at": null,
            "last_exit_code": null, "last_error": null,
            "provider_policy_pending": true
        }))
        .unwrap()
    }

    fn policy_payload(respond_to: &str) -> serde_json::Value {
        serde_json::json!({"respond_to": respond_to, "respond_to_allowlist": []})
    }

    fn scoped_payload(relay: &str, owner: &str) -> serde_json::Value {
        serde_json::json!({
            "relay_url": relay,
            "launch": { "owner_pubkey": owner },
        })
    }

    // ── assert_payload_scope: post-lock rebuilt-payload validation ──────────

    #[test]
    fn matching_scope_and_signer_pass_on_the_rebuilt_payload() {
        assert_payload_scope(
            &scoped_payload("wss://tenant-a.example", "aa11"),
            Some("wss://tenant-a.example"),
            Some("aa11"),
        )
        .unwrap();
    }

    #[test]
    fn relay_switch_during_the_lock_wait_fails_closed() {
        // Round-8 P1: a stale Projects-A start waited behind another deploy;
        // the rebuild resolved tenant B. The payload actually invoked must be
        // refused — the pre-lock snapshot its caller validated is irrelevant.
        let error = assert_payload_scope(
            &scoped_payload("wss://tenant-b.example", "aa11"),
            Some("wss://tenant-a.example"),
            Some("aa11"),
        )
        .unwrap_err();
        assert!(error.contains("active community changed"), "{error}");
    }

    #[test]
    fn same_relay_identity_switch_during_the_lock_wait_fails_closed() {
        // Same relay, different owner: an identity switch alone must also be
        // refused — the rebuilt launch.owner_pubkey belongs to a tenant the
        // caller never validated.
        let error = assert_payload_scope(
            &scoped_payload("wss://tenant-a.example", "bb22"),
            Some("wss://tenant-a.example"),
            Some("aa11"),
        )
        .unwrap_err();
        assert!(error.contains("active identity changed"), "{error}");
    }

    #[test]
    fn scoped_caller_with_an_unverifiable_payload_fails_closed() {
        let payload = serde_json::json!({});
        let relay_error =
            assert_payload_scope(&payload, Some("wss://tenant-a.example"), None).unwrap_err();
        assert!(relay_error.contains("no relay"), "{relay_error}");
        let signer_error = assert_payload_scope(&payload, None, Some("aa11")).unwrap_err();
        assert!(signer_error.contains("no owner identity"), "{signer_error}");
    }

    #[test]
    fn unscoped_callers_deploy_any_payload() {
        assert_payload_scope(
            &scoped_payload("wss://anywhere.example", "cc33"),
            None,
            None,
        )
        .unwrap();
        assert_payload_scope(&serde_json::json!({}), None, None).unwrap();
    }

    // ── apply_replay_floor: publish-first floor threading into the payload ──

    fn launch_payload() -> serde_json::Value {
        serde_json::json!({
            "launch": {
                "env": { "KEEP_ME": "yes" },
                "policy_env": { "BUZZ_ACP_LAZY_POOL": "true" },
            },
        })
    }

    #[test]
    fn caller_replay_floor_rides_launch_policy_env() {
        // A publish-first mention send's floor must reach the remote harness
        // as BUZZ_ACP_REPLAY_FLOOR, exactly like a local spawn's env.
        let mut payload = launch_payload();
        apply_replay_floor(&mut payload, Some(1_756_600_000));
        assert_eq!(
            payload["launch"]["policy_env"]["BUZZ_ACP_REPLAY_FLOOR"],
            "1756600000"
        );
        assert_eq!(payload["launch"]["env"]["KEEP_ME"], "yes");
        assert_eq!(
            payload["launch"]["policy_env"]["BUZZ_ACP_LAZY_POOL"],
            "true"
        );
    }

    #[test]
    fn caller_replay_floor_strips_user_env_shadow() {
        // launch.env later-wins over policy_env in the remote three-tier
        // model; a persisted user floor must not shadow this send's floor.
        let mut payload = launch_payload();
        payload["launch"]["env"]["BUZZ_ACP_REPLAY_FLOOR"] = "1".into();
        payload["launch"]["env"]["buzz_acp_replay_floor"] = "2".into();
        apply_replay_floor(&mut payload, Some(42));
        assert_eq!(
            payload["launch"]["policy_env"]["BUZZ_ACP_REPLAY_FLOOR"],
            "42"
        );
        assert!(payload["launch"]["env"]["BUZZ_ACP_REPLAY_FLOOR"].is_null());
        assert!(payload["launch"]["env"]["buzz_acp_replay_floor"].is_null());
        assert_eq!(payload["launch"]["env"]["KEEP_ME"], "yes");
    }

    #[test]
    fn no_caller_floor_leaves_payload_untouched() {
        // Create-flow deploys and plain redeploys carry no floor: user env
        // passthrough stands and no stale floor is invented.
        let mut payload = launch_payload();
        payload["launch"]["env"]["BUZZ_ACP_REPLAY_FLOOR"] = "1".into();
        let before = payload.clone();
        apply_replay_floor(&mut payload, None);
        assert_eq!(payload, before);
    }

    #[test]
    fn replay_floor_tolerates_payload_without_launch() {
        let mut payload = serde_json::json!({});
        apply_replay_floor(&mut payload, Some(42));
        assert_eq!(payload, serde_json::json!({}));
    }

    #[test]
    fn replay_floor_creates_missing_policy_env() {
        let mut payload = serde_json::json!({ "launch": {} });
        apply_replay_floor(&mut payload, Some(42));
        assert_eq!(
            payload["launch"]["policy_env"]["BUZZ_ACP_REPLAY_FLOOR"],
            "42"
        );
    }

    #[test]
    fn successful_deploy_acknowledges_pending_policy() {
        let mut record = record();

        apply_deploy_result(
            &mut record,
            Ok("provider-agent".into()),
            &policy_payload("owner-only"),
        )
        .unwrap();

        assert!(!record.provider_policy_pending);
        assert_eq!(record.backend_agent_id.as_deref(), Some("provider-agent"));
        assert_eq!(record.last_error, None);
    }

    #[test]
    fn successful_stale_deploy_preserves_newer_pending_policy() {
        let mut record = record();
        record.respond_to = crate::managed_agents::RespondTo::Anyone;

        apply_deploy_result(
            &mut record,
            Ok("provider-agent".into()),
            &policy_payload("owner-only"),
        )
        .unwrap();

        assert!(record.provider_policy_pending);
    }

    #[test]
    fn failed_deploy_preserves_pending_policy() {
        let mut record = record();

        let error = apply_deploy_result(
            &mut record,
            Err("provider unavailable".into()),
            &policy_payload("owner-only"),
        )
        .expect_err("deployment should fail");

        assert_eq!(error, "provider unavailable");
        assert!(record.provider_policy_pending);
        assert_eq!(record.last_error.as_deref(), Some("provider unavailable"));
    }
}
