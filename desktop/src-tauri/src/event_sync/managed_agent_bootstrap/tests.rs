use super::*;
use nostr::{EventBuilder, Kind, Timestamp};

fn event(keys: &nostr::Keys, timestamp: u64, suffix: usize) -> nostr::Event {
    EventBuilder::new(Kind::Custom(KIND_DELETION as u16), suffix.to_string())
        .custom_created_at(Timestamp::from(timestamp))
        .sign_with_keys(keys)
        .unwrap()
}

#[test]
fn history_requires_exhaustion_and_advances_across_seconds() {
    let keys = nostr::Keys::generate();
    let mut history = History::default();
    let page: Vec<_> = (0..PAGE_LIMIT)
        .map(|i| event(&keys, 1000 - i as u64, i))
        .collect();
    assert!(!history.push(page, keys.public_key()).unwrap());
    assert!(history
        .push(vec![event(&keys, 500, PAGE_LIMIT)], keys.public_key())
        .unwrap());
    assert_eq!(history.events.len(), PAGE_LIMIT + 1);
}

#[test]
fn history_pages_dense_seconds_and_rejects_ignored_cursor() {
    let keys = nostr::Keys::generate();
    let mut events: Vec<_> = (0..PAGE_LIMIT * 2).map(|i| event(&keys, 1000, i)).collect();
    events.sort_by_key(|event| event.id);
    let mut history = History::default();
    for page in events.chunks(PAGE_LIMIT) {
        assert!(!history.push(page.to_vec(), keys.public_key()).unwrap());
    }
    assert!(history.push(vec![], keys.public_key()).unwrap());
    assert_eq!(history.events, events);

    let mut history = History::default();
    let page = events[..PAGE_LIMIT].to_vec();
    assert!(!history.push(page.clone(), keys.public_key()).unwrap());
    assert!(history.push(page, keys.public_key()).is_err());
}

#[test]
fn history_rejects_out_of_order_and_out_of_scope_data() {
    let keys = nostr::Keys::generate();
    assert!(History::default()
        .push(
            vec![event(&keys, 999, 0), event(&keys, 1000, 1)],
            keys.public_key()
        )
        .is_err());
    assert!(History::default()
        .push(
            vec![event(&keys, 1000, 0)],
            nostr::Keys::generate().public_key()
        )
        .is_err());
    let mut invalid = event(&keys, 1000, 0);
    invalid.content = "tampered".into();
    assert!(History::default()
        .push(vec![invalid], keys.public_key())
        .is_err());
}

#[test]
fn history_refuses_to_buffer_beyond_its_byte_budget() {
    let keys = nostr::Keys::generate();
    let event = event(&keys, 1000, 0);
    let mut history = History {
        bytes: MAX_HISTORY_BYTES,
        ..History::default()
    };
    assert!(history
        .push(vec![event], keys.public_key())
        .unwrap_err()
        .contains("byte limit"));
    assert!(history.events.is_empty());
}

#[test]
fn boot_publication_is_gated_after_native_history_not_before_it() {
    // AppHandle boot wiring guard, paired with real HTTP/inbound tests below.
    // Pure pagination success alone must never accidentally authorize the old
    // unconditional disk publication site.
    let source = include_str!("../../event_sync.rs");
    let sync = source
        .split("pub fn run_event_sync(")
        .nth(1)
        .unwrap()
        .split("/// Rebuild")
        .next()
        .unwrap();
    assert!(sync.contains("if agent_history_complete {\n        crate::managed_agents::reconcile::reconcile_agents_to_events"));
    let blocking = source
        .split("pub async fn run_event_sync_blocking(")
        .nth(1)
        .unwrap()
        .split("/// Reconcile `personas.json`")
        .next()
        .unwrap();
    assert!(
        blocking
            .find("managed_agent_bootstrap::bootstrap(")
            .unwrap()
            < blocking.find("run_event_sync(&app").unwrap()
    );
    assert!(
        blocking.contains("false\n"),
        "failure must withhold publication"
    );
}

struct TestHome {
    _temp: tempfile::TempDir,
    home: Option<std::ffi::OsString>,
    xdg: Option<std::ffi::OsString>,
}

impl TestHome {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = std::env::var_os("HOME");
        let xdg = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        Self {
            _temp: temp,
            home,
            xdg,
        }
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        for (name, value) in [("HOME", &self.home), ("XDG_DATA_HOME", &self.xdg)] {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn serve_history(
    body: String,
    status: &'static str,
    before_response: impl FnOnce() + Send + 'static,
) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    let mut before_response = Some(before_response);
    serve_pages(1, move |_, _| {
        before_response.take().unwrap()();
        (body.clone(), status)
    })
}

fn serve_pages(
    pages: usize,
    mut response: impl FnMut(usize, serde_json::Value) -> (String, &'static str) + Send + 'static,
) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        for index in 0..pages {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "bootstrap never queried relay"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            // Darwin may inherit O_NONBLOCK from the listening socket.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut buf).unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buf[..count]);
            }
            let header_end = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|v| v.parse().unwrap())
                })
                .unwrap();
            while request.len() < header_end + length {
                let count = stream.read(&mut buf).unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buf[..count]);
            }
            assert!(headers
                .to_ascii_lowercase()
                .contains("authorization: nostr "));
            let filters: serde_json::Value =
                serde_json::from_slice(&request[header_end..]).unwrap();
            assert!(filters[0]["kinds"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(30179)));
            let (body, status) = response(index, filters);
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (addr, server)
}

#[test]
fn authenticated_bootstrap_applies_old_device_delete_before_any_boot_mint() {
    use crate::managed_agents::{
        load_managed_agents,
        retention::{active_retention_scope, get_retained_event, open_retention_db},
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
    use nostr::Tag;
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    let keys = nostr::Keys::generate();
    let agent = nostr::Keys::generate().public_key().to_hex();
    let tombstone = EventBuilder::new(Kind::Custom(KIND_DELETION as u16), "")
        .tags([Tag::parse([
            "a",
            &format!(
                "{KIND_MANAGED_AGENT}:{}:{agent}",
                keys.public_key().to_hex()
            ),
        ])
        .unwrap()])
        .custom_created_at(Timestamp::from(20))
        .sign_with_keys(&keys)
        .unwrap();
    let body = serde_json::to_string(&vec![tombstone]).unwrap();
    let (addr, server) = serve_history(body, "200 OK", || {});
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let base = crate::managed_agents::managed_agents_base_dir(app.handle()).unwrap();
    // No key material needed: the old device still has the deleted lifecycle row.
    let record: crate::managed_agents::ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": agent, "name": "deleted elsewhere", "private_key_nsec": "", "relay_url": "",
        "acp_command": "buzz-acp", "agent_command": "goose", "agent_args": [], "mcp_command": "",
        "turn_timeout_seconds": 320, "system_prompt": "x", "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z", "last_started_at": null, "last_stopped_at": null,
        "last_exit_code": null, "last_error": null
    })).unwrap();
    std::fs::write(
        base.join("managed-agents.json"),
        serde_json::to_vec(&[record]).unwrap(),
    )
    .unwrap();
    let result = run_bootstrap(app.handle(), &keys);
    // An isolated HOME may have no OS keyring. That is a real cleanup failure,
    // not permission to forget the deletion; bootstrap must remain gated.
    if let Err(error) = &result {
        assert!(
            error.contains("keyring"),
            "unexpected bootstrap failure: {error}"
        );
    }
    server.join().unwrap();
    assert!(load_managed_agents(app.handle()).unwrap().is_empty());
    let scope = active_retention_scope(app.handle(), &app.state()).unwrap();
    let conn = open_retention_db(&scope.db_path).unwrap();
    assert_eq!(
        crate::managed_agents::retention::deletion_intent::pending(
            &conn,
            &keys.public_key().to_hex(),
            &agent,
        )
        .unwrap(),
        result.is_err(),
        "failed key cleanup keeps an exact boot-retry obligation",
    );
    assert!(get_retained_event(
        &conn,
        KIND_DELETION,
        &keys.public_key().to_hex(),
        &format!("30177:{agent}")
    )
    .unwrap()
    .is_some());
    assert!(get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &agent
    )
    .unwrap()
    .is_none());
}

#[test]
fn bootstrap_rejects_auth_failure_and_malformed_http_history() {
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    for (status, body) in [("401 Unauthorized", "[]"), ("200 OK", "not json")] {
        let (addr, server) = serve_history(body.into(), status, || {});
        let keys = nostr::Keys::generate();
        let state = crate::app_state::build_app_state();
        *state.keys.lock().unwrap() = keys.clone();
        *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        assert!(run_bootstrap(app.handle(), &keys).is_err());
        server.join().unwrap();
    }
}

#[test]
fn bootstrap_empty_history_is_not_success_for_a_changed_scope() {
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    let keys = nostr::Keys::generate();
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let handle = app.handle().clone();
    let (addr, server) = serve_history("[]".into(), "200 OK", move || {
        *handle
            .state::<crate::app_state::AppState>()
            .keys
            .lock()
            .unwrap() = nostr::Keys::generate();
    });
    *app.state::<crate::app_state::AppState>()
        .relay_url_override
        .lock()
        .unwrap() = Some(format!("http://{addr}"));
    assert!(run_bootstrap(app.handle(), &keys)
        .unwrap_err()
        .contains("scope changed"));
    server.join().unwrap();
}

#[test]
fn bootstrap_cannot_restore_a_head_covered_by_unflushed_local_deletion() {
    use crate::managed_agents::retention::{
        active_retention_scope, get_retained_event, open_retention_db, retain_event, RetainedEvent,
    };
    use nostr::{JsonUtil, ToBech32};
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    let keys = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let pubkey = agent.public_key().to_hex();
    let mut payload = crate::managed_agents::private_config_overlay::test_relay_payload(&pubkey);
    payload.owner_pubkey = keys.public_key().to_hex();
    payload.generation = 1;
    payload.identity.private_key_nsec = agent.secret_key().to_bech32().unwrap();
    let head = buzz_core_pkg::private_managed_agent::build_event(&keys, &payload, 20).unwrap();
    let (addr, server) =
        serve_history(serde_json::to_string(&vec![head]).unwrap(), "200 OK", || {});
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let scope = active_retention_scope(app.handle(), &app.state()).unwrap();
    let conn = open_retention_db(&scope.db_path).unwrap();
    let tombstone = EventBuilder::new(Kind::Custom(KIND_DELETION as u16), "")
        .tags([nostr::Tag::parse([
            "a",
            &format!("30177:{}:{pubkey}", keys.public_key().to_hex()),
        ])
        .unwrap()])
        .custom_created_at(Timestamp::from(30))
        .sign_with_keys(&keys)
        .unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_DELETION,
            pubkey: keys.public_key().to_hex(),
            d_tag: format!("30177:{pubkey}"),
            content: String::new(),
            created_at: 30,
            raw_event: tombstone.as_json(),
            pending_sync: true,
        },
    )
    .unwrap();
    run_bootstrap(app.handle(), &keys).unwrap();
    server.join().unwrap();
    assert!(get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &pubkey
    )
    .unwrap()
    .is_none());
    assert!(app
        .state::<crate::app_state::AppState>()
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .resolved_records(&[])
        .is_empty());
    assert!(
        get_retained_event(
            &conn,
            KIND_DELETION,
            &keys.public_key().to_hex(),
            &format!("30177:{pubkey}")
        )
        .unwrap()
        .unwrap()
        .pending_sync
    );
}

#[test]
fn boot_hydration_propagates_corruption_without_replacing_cached_authority() {
    use crate::managed_agents::retention::{
        active_retention_scope, open_retention_db, retain_event, RetainedEvent,
    };
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    let keys = nostr::Keys::generate();
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<crate::app_state::AppState>();
    let scope = active_retention_scope(app.handle(), &state).unwrap();
    let agent = nostr::Keys::generate().public_key().to_hex();
    state
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .insert(crate::managed_agents::private_config_overlay::test_relay_payload(&agent))
        .unwrap();
    let conn = open_retention_db(&scope.db_path).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: keys.public_key().to_hex(),
            d_tag: agent,
            content: "corrupt".into(),
            created_at: 20,
            raw_event: "not an event".into(),
            pending_sync: false,
        },
    )
    .unwrap();
    assert!(
        super::super::hydrate_private_config_overlay(app.handle(), &keys, &scope.db_path).is_err()
    );
    assert_eq!(state.private_managed_agent_overlay.lock().unwrap().len(), 1);
}

#[test]
fn lifecycle_resolution_requires_completed_hydration_not_just_an_inbound_patch() {
    use crate::managed_agents::private_config_overlay::{
        resolved_local_record, test_relay_payload,
    };
    use crate::managed_agents::retention::{active_retention_scope, open_retention_db};
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    let keys = nostr::Keys::generate();
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<crate::app_state::AppState>();
    let pubkey = nostr::Keys::generate().public_key().to_hex();
    let record = {
        let mut overlay = state.private_managed_agent_overlay.lock().unwrap();
        overlay.insert(test_relay_payload(&pubkey)).unwrap();
        overlay.resolved_records(&[]).remove(0)
    };
    assert!(resolved_local_record(&state, &record).is_err());
    let scope = active_retention_scope(app.handle(), &state).unwrap();
    let conn = open_retention_db(&scope.db_path).unwrap();
    super::super::hydrate_private_config_overlay(app.handle(), &keys, &scope.db_path).unwrap();
    assert!(resolved_local_record(&state, &record).is_ok());
    conn.execute_batch("DROP TABLE persona_events").unwrap();
    // Fail the SELECT, not open_retention_db's idempotent schema creation.
    conn.execute_batch("CREATE TABLE persona_events (wrong_column INTEGER)")
        .unwrap();
    assert!(
        super::super::hydrate_private_config_overlay(app.handle(), &keys, &scope.db_path).is_err()
    );
    assert!(resolved_local_record(&state, &record).is_err());
}

// Keep the process-global HOME/PATH guard on a synchronous test thread; the
// runtime drives the network future while no async task blocks on that guard.
fn run_bootstrap<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    keys: &nostr::Keys,
) -> Result<(), String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(bootstrap(app, keys))
}

#[test]
fn authenticated_bootstrap_pages_dense_second_until_empty_page() {
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    let keys = nostr::Keys::generate();
    let mut events: Vec<_> = (0..PAGE_LIMIT * 2).map(|i| event(&keys, 1000, i)).collect();
    events.sort_by_key(|event| event.id);
    let owner = keys.public_key().to_hex();
    let (addr, server) = serve_pages(3, move |index, filters| {
        let filter = &filters[0];
        assert_eq!(filter["authors"], serde_json::json!([owner]));
        assert_eq!(filter["limit"], PAGE_LIMIT);
        if index == 0 {
            assert!(filter.get("until").is_none());
            assert!(filter.get("before_id").is_none());
        } else {
            assert_eq!(filter["until"], 1000);
            assert_eq!(
                filter["before_id"],
                events[index * PAGE_LIMIT - 1].id.to_hex()
            );
        }
        // Relay ordering/predicate: created_at DESC, id ASC, strictly after
        // (until, before_id). The third, empty page proves exhaustion.
        let page: Vec<_> = events
            .iter()
            .filter(|event| {
                filter.get("until").is_none_or(|until| {
                    event.created_at.as_secs() < until.as_u64().unwrap()
                        || (event.created_at.as_secs() == until.as_u64().unwrap()
                            && event.id.to_hex().as_str() > filter["before_id"].as_str().unwrap())
                })
            })
            .take(PAGE_LIMIT)
            .cloned()
            .collect();
        assert_eq!(page.len(), if index < 2 { PAGE_LIMIT } else { 0 });
        (serde_json::to_string(&page).unwrap(), "200 OK")
    });
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let result = run_bootstrap(app.handle(), &keys);
    server.join().unwrap();
    result.unwrap();
}

#[test]
fn bootstrap_warning_survives_reads_is_scoped_and_clears_after_retry() {
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    let keys = nostr::Keys::generate();
    let (addr, server) = serve_pages(2, |index, _| {
        if index == 0 {
            ("[]".into(), "401 Unauthorized")
        } else {
            ("[]".into(), "200 OK")
        }
    });
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let error = run_bootstrap(app.handle(), &keys).unwrap_err();
    for _ in 0..2 {
        assert_eq!(
            crate::commands::managed_agent_sync_error(app.handle()).unwrap(),
            Some(error.clone())
        );
    }
    let state = app.state::<crate::app_state::AppState>();
    *state.keys.lock().unwrap() = nostr::Keys::generate();
    assert_eq!(
        crate::commands::managed_agent_sync_error(app.handle()).unwrap(),
        None
    );
    *state.keys.lock().unwrap() = keys.clone();
    run_bootstrap(app.handle(), &keys).unwrap();
    assert_eq!(
        crate::commands::managed_agent_sync_error(app.handle()).unwrap(),
        None
    );
    server.join().unwrap();
}

#[test]
fn public_only_recreation_survives_bootstrap_without_consuming_live_policy_update() {
    use crate::managed_agents::{agent_events::build_agent_event, retention::*};
    use nostr::{JsonUtil, ToBech32};
    let _paths = crate::managed_agents::lock_path_mutex();
    let _home = TestHome::new();
    let keys = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let pubkey = agent.public_key().to_hex();
    let record: crate::managed_agents::ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": pubkey, "name": "surviving identity", "private_key_nsec": agent.secret_key().to_bech32().unwrap(), "relay_url": "",
        "acp_command": "buzz-acp", "agent_command": "goose", "agent_args": [], "mcp_command": "",
        "turn_timeout_seconds": 320, "system_prompt": "x", "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z", "last_started_at": null, "last_stopped_at": null,
        "last_exit_code": null, "last_error": null
    })).unwrap();
    let public = build_agent_event(&record)
        .unwrap()
        .custom_created_at(Timestamp::from(30))
        .sign_with_keys(&keys)
        .unwrap();
    let tombstone = crate::managed_agents::agent_events::build_agent_delete(
        &pubkey,
        &keys.public_key().to_hex(),
    )
    .unwrap()
    .custom_created_at(Timestamp::from(20))
    .sign_with_keys(&keys)
    .unwrap();
    let (addr, server) = serve_pages(1, move |_, filters| {
        let kinds = filters[0]["kinds"].as_array().unwrap();
        let page: Vec<_> = [&public, &tombstone]
            .into_iter()
            .filter(|event| kinds.contains(&serde_json::json!(event.kind.as_u16())))
            .collect();
        (serde_json::to_string(&page).unwrap(), "200 OK")
    });
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(format!("http://{addr}"));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let base = crate::managed_agents::managed_agents_base_dir(app.handle()).unwrap();
    let path = base.join("managed-agents.json");
    let bytes = serde_json::to_vec(std::slice::from_ref(&record)).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let scope = active_retention_scope(app.handle(), &app.state()).unwrap();
    let conn = open_retention_db(&scope.db_path).unwrap();
    let mut payload = crate::managed_agents::private_config_overlay::test_relay_payload(&pubkey);
    payload.owner_pubkey = keys.public_key().to_hex();
    payload.generation = 1;
    payload.identity.private_key_nsec = agent.secret_key().to_bech32().unwrap();
    let old_private =
        buzz_core_pkg::private_managed_agent::build_event(&keys, &payload, 10).unwrap();
    retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: keys.public_key().to_hex(),
            d_tag: pubkey.clone(),
            content: old_private.content.clone(),
            created_at: 10,
            raw_event: old_private.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
    app.state::<crate::app_state::AppState>()
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .insert(payload)
        .unwrap();
    app.state::<crate::app_state::AppState>()
        .managed_agent_authority_ready
        .store(true, std::sync::atomic::Ordering::Release);
    let result = run_bootstrap(app.handle(), &keys);
    server.join().unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "history must not erase a recreated identity's local lifecycle/key"
    );
    result.unwrap();
    let scope = active_retention_scope(app.handle(), &app.state()).unwrap();
    let conn = open_retention_db(&scope.db_path).unwrap();
    assert!(get_retained_event(
        &conn,
        KIND_DELETION,
        &keys.public_key().to_hex(),
        &format!("30177:{pubkey}")
    )
    .unwrap()
    .is_some());
    assert!(
        get_retained_event(&conn, 30177, &keys.public_key().to_hex(), &pubkey)
            .unwrap()
            .is_none(),
        "survival witness must not mark live public policy as already applied"
    );
    assert!(!deletion_intent::pending(&conn, &keys.public_key().to_hex(), &pubkey).unwrap());
    assert!(get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &pubkey
    )
    .unwrap()
    .is_none());
    assert_eq!(
        app.state::<crate::app_state::AppState>()
            .private_managed_agent_overlay
            .lock()
            .unwrap()
            .len(),
        0
    );
    let state = app.state::<crate::app_state::AppState>();
    assert!(
        crate::managed_agents::private_config_overlay::resolved_local_record(&state, &record)
            .is_err(),
        "live bootstrap must deny deleted disk config before hydration too"
    );
    // Exercise the production read/Start resolver after the same hydration
    // step used by run_event_sync, not only the absence of a cached patch.
    super::super::hydrate_private_config_overlay(app.handle(), &keys, &scope.db_path).unwrap();
    let state = app.state::<crate::app_state::AppState>();
    assert!(
        crate::managed_agents::private_config_overlay::resolved_local_record(&state, &record)
            .is_err(),
        "preserving identity must not authorize Start to execute deleted disk config"
    );
    assert!(
        crate::managed_agents::private_config_overlay::resolved_record_for_read(
            &state,
            std::slice::from_ref(&record),
            &pubkey
        )
        .is_err()
    );

    let mut restored = crate::managed_agents::private_config_overlay::test_relay_payload(&pubkey);
    restored.owner_pubkey = keys.public_key().to_hex();
    restored.generation = 1;
    restored.identity.private_key_nsec = agent.secret_key().to_bech32().unwrap();
    restored.config.system_prompt = Some("explicitly restored private config".into());
    let newer = buzz_core_pkg::private_managed_agent::build_event(&keys, &restored, 40).unwrap();
    crate::commands::reconcile_managed_agent_bootstrap_event(
        &newer,
        &scope.relay_url,
        app.handle(),
    )
    .unwrap();
    for hydrate in [false, true] {
        if hydrate {
            super::super::hydrate_private_config_overlay(app.handle(), &keys, &scope.db_path)
                .unwrap();
        }
        let resolved =
            crate::managed_agents::private_config_overlay::resolved_local_record(&state, &record)
                .unwrap();
        assert_eq!(
            resolved.system_prompt.as_deref(),
            Some("explicitly restored private config")
        );
    }
}
