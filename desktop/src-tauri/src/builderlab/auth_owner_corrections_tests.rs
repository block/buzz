//! Regressions for the three checkpoint findings. Uses the native-auth fixture,
//! including its environment lock and preinitialized production workdir cache.
use super::*;
use crate::managed_agents::{save_personas, save_teams, AgentDefinition, TeamRecord};
use std::{collections::BTreeMap, path::Path};

fn disk(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn unmerged_persona(f: &Fixture) -> AgentDefinition {
    let persona: AgentDefinition = serde_json::from_value(serde_json::json!({
        "id":"custom:proof", "display_name":"Proof", "system_prompt":"Exact prompt",
        "source_team":"proof-team", "is_active":true,
        "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
    }))
    .unwrap();
    save_personas(f.app.handle(), std::slice::from_ref(&persona)).unwrap();
    persona
}

#[tokio::test]
async fn actual_team_delete_denies_signed_out_and_authenticated_inactive_remote_without_effects() {
    for authenticated in [false, true] {
        let f = Fixture::new(false).await;
        let persona = unmerged_persona(&f);
        let source = f._temp.path().join("proof-team");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("persona.md"), "keep cascade disk").unwrap();
        let team: TeamRecord = serde_json::from_value(serde_json::json!({
            "id":"proof-team", "name":"Proof Team", "persona_ids":[persona.id],
            "source_dir":source,
            "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
        }))
        .unwrap();
        save_teams(f.app.handle(), &[team]).unwrap();
        // The existing independent agent does not reference this team: the
        // baseline reference guard must not mask the destructive cascade.
        let state = f.app.state::<crate::AppState>();
        let scope =
            crate::managed_agents::retention::active_retention_scope(f.app.handle(), &state)
                .unwrap();
        seed(
            &scope.db_path,
            &f.api.state.keys.public_key().to_hex(),
            &f.agent.public_key().to_hex(),
        );
        if !authenticated {
            state.native_auth.clear().unwrap();
        }
        assert_eq!(
            serde_json::to_value(state.native_identity_status().unwrap()).unwrap()
                ["workspaceActive"],
            false
        );
        let before = disk(f._temp.path());
        let requests = f.api.state.requests.lock().unwrap().len();
        let result =
            crate::commands::delete_team("proof-team".into(), f.app.handle().clone()).await;
        assert!(
            source.exists(),
            "remote team deletion removed cascade directory"
        );
        assert!(
            result.is_err(),
            "remote team deletion must deny before cascade: {result:?}"
        );
        assert!(result.unwrap_err().contains("workspace"));
        assert_eq!(
            before,
            disk(f._temp.path()),
            "no disk/store/retention mutation"
        );
        assert_eq!(requests, f.api.state.requests.lock().unwrap().len());
        unchanged(
            &scope.db_path,
            &f.api.state.keys.public_key().to_hex(),
            &f.agent.public_key().to_hex(),
        );
    }
}

#[tokio::test]
async fn actual_create_preparation_failure_or_cancellation_does_not_merge_or_publish() {
    // The command's remote workspace gate intentionally stays closed. Invoke
    // its real inner proof/definition phase, not shared prepare_agent alone.
    for action in ["signature", "logout", "replace", "cancel", "relay"] {
        let f = Fixture::new(false).await;
        let persona = unmerged_persona(&f);
        let before = disk(f._temp.path());
        let records = f.records();
        let owner = OwnerAuthorizationScope::capture(&f.app.state::<crate::AppState>()).unwrap();
        f.api.hold(OA);
        let app = f.app.handle().clone();
        let task = tokio::spawn(async move {
            crate::commands::prepare_create_agent(&app, &app.state(), &owner, Some(&persona.id))
                .await
        });
        f.api.arrived().await;
        assert!(f
            .app
            .state::<crate::AppState>()
            .managed_agents_store_lock
            .try_lock()
            .is_ok());
        let during = disk(f._temp.path());
        match action {
            "signature" => {
                *f.api.state.oa_fault.lock().unwrap() = Some("signature");
                f.api.release();
            }
            "logout" => f
                .app
                .state::<crate::AppState>()
                .native_auth
                .clear()
                .unwrap(),
            "replace" => {
                f.api
                    .login(&f.app.state::<crate::AppState>().native_auth, "B")
                    .await
                    .unwrap();
            }
            "cancel" => task.abort(),
            "relay" => {
                *f.app
                    .state::<crate::AppState>()
                    .relay_url_override
                    .lock()
                    .unwrap() = Some("http://127.0.0.1:9".into());
                f.api.release();
            }
            _ => unreachable!(),
        }
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap();
        f.api.release();
        if action == "cancel" {
            assert!(matches!(result, Err(e) if e.is_cancelled()));
        } else {
            assert!(result.unwrap().is_err());
        }
        assert_eq!(
            before, during,
            "pre-proof phase wrote definitions: {action}"
        );
        assert_eq!(
            before,
            disk(f._temp.path()),
            "failed preparation changed disk: {action}"
        );
        assert_eq!(records, f.records());
        assert!(f.api.state.agent_publications.lock().unwrap().is_empty());
        assert!(f
            .api
            .state
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.0 != "events" && r.0 != "v1/buzz/identity/sign"));
    }
}

#[tokio::test]
async fn actual_create_preparation_local_success_still_merges_and_validates() {
    let f = Fixture::new(false).await;
    let persona = unmerged_persona(&f);
    let state = build_app_state_for_mode(SignerMode::Local);
    state.replace_local_identity_keys(Keys::generate()).unwrap();
    let owner = OwnerAuthorizationScope::capture(&state).unwrap();
    let before = disk(f._temp.path());
    let minted =
        crate::commands::prepare_create_agent(f.app.handle(), &state, &owner, Some(&persona.id))
            .await
            .unwrap();
    assert_ne!(
        before,
        disk(f._temp.path()),
        "fixture must trigger built-in merge/write"
    );
    assert_eq!(
        buzz_sdk_pkg::nip_oa::verify_auth_tag(&minted.auth_tag.unwrap(), &minted.keys.public_key())
            .unwrap(),
        owner.signer.public_key()
    );
    let after = disk(f._temp.path());
    let personas = crate::managed_agents::load_personas(f.app.handle()).unwrap();
    assert!(personas.iter().any(|p| p.is_builtin));
    assert_eq!(
        after,
        disk(f._temp.path()),
        "successful phase already normalized"
    );
    assert!(crate::commands::prepare_create_agent(
        f.app.handle(),
        &state,
        &owner,
        Some("missing-persona")
    )
    .await
    .is_err());
    assert!(f.api.state.agent_publications.lock().unwrap().is_empty());
}

#[tokio::test]
async fn standalone_commit_rechecks_session_after_sqlite_wait() {
    for action in ["logout", "replace", "valid", "local"] {
        let api = MockApi::new().await;
        let state = build_app_state_for_mode(SignerMode::Remote);
        api.login(&state.native_auth, "A").await.unwrap();
        let signer = if action == "local" {
            ActiveUserSigner::local(Keys::generate())
        } else {
            state.active_signer().unwrap()
        };
        let owner = signer.public_key().to_hex();
        let agent = Keys::generate().public_key().to_hex();
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("retention.db");
        seed(&db, &owner, &agent);
        let witness = prepare_agent_tombstone(&db, &signer, &agent)
            .unwrap()
            .sign(&signer)
            .await;
        let conn = open_retention_db(&db).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (started, waiting) = tokio::sync::oneshot::channel();
        let task = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            witness.commit()
        });
        waiting.await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!task.is_finished(), "writer must block standalone commit");
        match action {
            "logout" => state.native_auth.clear().unwrap(),
            "replace" => {
                api.login(&state.native_auth, "B").await.unwrap();
            }
            _ => {}
        }
        conn.execute_batch("ROLLBACK").unwrap();
        drop(conn);
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        if matches!(action, "logout" | "replace") {
            assert!(
                result.is_err(),
                "stale standalone commit succeeded: {action}"
            );
            unchanged(&db, &owner, &agent);
            if action == "replace" {
                state.active_signer().unwrap().check_valid().unwrap();
            }
        } else {
            result.unwrap();
            let conn = open_retention_db(&db).unwrap();
            assert!(get_retained_event(&conn, 30177, &owner, &agent)
                .unwrap()
                .is_none());
            assert_eq!(get_pending_sync(&conn).unwrap().len(), 2);
        }
    }
}
