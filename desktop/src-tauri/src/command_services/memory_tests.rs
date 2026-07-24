#[cfg(all(test, unix))]
mod tests {
    use super::super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    static TEST_SYNC_STATE: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct FakeCredentials {
        values: HashMap<String, String>,
    }

    impl CredentialSource for FakeCredentials {
        fn load(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self.values.get(key).cloned())
        }
    }

    fn protected_file(path: &Path, contents: &[u8], executable: bool) {
        fs::write(path, contents).expect("write fixture");
        let mut permissions = fs::metadata(path).expect("fixture metadata").permissions();
        permissions.set_mode(if executable { 0o700 } else { 0o600 });
        fs::set_permissions(path, permissions).expect("protect fixture");
    }

    fn tempdir() -> tempfile::TempDir {
        let base = std::env::current_dir().expect("current directory");
        tempfile::Builder::new()
            .tempdir_in(&base)
            .expect("temporary directory")
    }

    fn config_json(directory: &Path, local_port: u16) -> serde_json::Value {
        let fingerprint = crate::command_services::ssh::sha256_fingerprint(b"config fake key blob");
        json!({
            "schema_version": 1,
            "local_port": local_port,
            "home_host_alias": "memory-home",
            "home_user": "memory-sync",
            "pinned_host_fingerprint": fingerprint,
            "known_hosts_path": directory.join("known_hosts"),
            "identity_file": directory.join("identity"),
            "remote_loopback_port": 8006,
            "local_node_id": "node:macbook-command",
            "home_node_id": "node:home-command",
            "sync_interval_minutes": 30,
            "tool_allowlist": [
                "recall_for_entity",
                "search_events",
                "record_event"
            ],
            "replicate_cli_path": directory.join("memory-mcp-replicate"),
            "credential_keys": {
                "local_read": "memory.local.read",
                "local_replicate": "memory.local.replicate",
                "remote_read": "memory.remote.read",
                "remote_replicate": "memory.remote.replicate"
            }
        })
    }

    fn credentials() -> FakeCredentials {
        FakeCredentials {
            values: [
                ("memory.local.read", "local-read-token"),
                ("memory.local.replicate", "local-replicate-token"),
                ("memory.remote.read", "remote-read-token"),
                ("memory.remote.replicate", "remote-replicate-token"),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        }
    }

    fn write_config(directory: &Path, local_port: u16) -> (std::path::PathBuf, std::path::PathBuf) {
        let cli = directory.join("memory-mcp-replicate");
        protected_file(&cli, b"#!/bin/sh\nexit 0\n", true);
        let key = base64::engine::general_purpose::STANDARD.encode(b"config fake key blob");
        protected_file(
            &directory.join("known_hosts"),
            format!("memory-home ssh-ed25519 {key}\n").as_bytes(),
            false,
        );
        protected_file(&directory.join("identity"), b"placeholder\n", false);
        let path = directory.join("memory.json");
        protected_file(
            &path,
            serde_json::to_vec(&config_json(directory, local_port))
                .expect("serialize fixture config")
                .as_slice(),
            false,
        );
        (path, cli)
    }

    fn fake_readiness_server(
        response: serde_json::Value,
        expected_token: &'static str,
    ) -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake service");
        let port = listener.local_addr().expect("fake service address").port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept readiness request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 2048];
            loop {
                let count = stream.read(&mut chunk).expect("read request");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).expect("utf8 request");
            assert!(request.starts_with("GET /replication/readiness HTTP/1.1\r\n"));
            assert!(request
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {expected_token}")));
            let body = serde_json::to_vec(&response).expect("serialize response");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write response headers");
            stream.write_all(&body).expect("write response body");
        });
        (port, handle)
    }

    #[test]
    fn trusted_config_rejects_unknown_fields_collisions_and_invalid_tools() {
        let directory = tempdir();
        let (path, _) = write_config(directory.path(), 8006);
        let mut value = config_json(directory.path(), 8006);
        value
            .as_object_mut()
            .expect("config object")
            .insert("renderer_bearer".to_string(), json!("secret"));
        protected_file(
            &path,
            serde_json::to_vec(&value)
                .expect("serialize config")
                .as_slice(),
            false,
        );
        assert_eq!(
            load_trusted_config(&path, &credentials())
                .err()
                .expect("unknown renderer-controlled secret field must fail"),
            MemoryError::InvalidConfig
        );

        let mut value = config_json(directory.path(), 8006);
        value["local_node_id"] = value["home_node_id"].clone();
        protected_file(
            &path,
            serde_json::to_vec(&value)
                .expect("serialize config")
                .as_slice(),
            false,
        );
        assert_eq!(
            load_trusted_config(&path, &credentials())
                .err()
                .expect("node collision must fail"),
            MemoryError::InvalidConfig
        );

        let mut value = config_json(directory.path(), 8006);
        value["tool_allowlist"] = json!(["recall_for_entity", "recall_for_entity"]);
        protected_file(
            &path,
            serde_json::to_vec(&value)
                .expect("serialize config")
                .as_slice(),
            false,
        );
        assert_eq!(
            load_trusted_config(&path, &credentials())
                .err()
                .expect("duplicate tools must fail"),
            MemoryError::InvalidConfig
        );
    }

    #[test]
    fn readiness_is_authenticated_loopback_only_and_checks_stable_node_identity() {
        let response = json!({
            "status": "ready",
            "schema_version": 1,
            "node_id": "node:macbook-command",
            "revision_count": 41,
            "conflict_count": 2,
            "max_page_items": 200,
            "max_envelope_bytes": 2097152,
            "markdown_canonical": true,
            "sqlite_derived": true
        });
        let (port, server) = fake_readiness_server(response, "local-read-token");
        let directory = tempdir();
        let (path, _) = write_config(directory.path(), port);
        let trusted = load_trusted_config(&path, &credentials()).expect("load trusted config");

        let readiness = query_readiness(&trusted, Duration::from_secs(2))
            .expect("valid authenticated readiness");

        server.join().expect("join fake server");
        assert_eq!(readiness.node_id.as_deref(), Some("node:macbook-command"));
        assert_eq!(readiness.revision_count, 41);
        assert_eq!(readiness.conflict_count, 2);
        assert_eq!(readiness.endpoint, Some(format!("http://127.0.0.1:{port}")));
        assert_eq!(
            readiness.tool_allowlist,
            vec!["recall_for_entity", "search_events", "record_event"]
        );
    }

    #[test]
    fn readiness_rejects_an_unexpected_node_id() {
        let response = json!({
            "status": "ready",
            "schema_version": 1,
            "node_id": "node:attacker",
            "revision_count": 0,
            "conflict_count": 0,
            "max_page_items": 200,
            "max_envelope_bytes": 2097152,
            "markdown_canonical": true,
            "sqlite_derived": true
        });
        let (port, server) = fake_readiness_server(response, "local-read-token");
        let directory = tempdir();
        let (path, _) = write_config(directory.path(), port);
        let trusted = load_trusted_config(&path, &credentials()).expect("load trusted config");

        let error =
            query_readiness(&trusted, Duration::from_secs(2)).expect_err("node mismatch must fail");

        server.join().expect("join fake server");
        assert_eq!(error, MemoryError::NodeIdentityMismatch);
    }

    #[test]
    fn replication_cli_is_direct_bounded_redacted_and_exactly_parsed() {
        let _state = TEST_SYNC_STATE.lock().expect("lock sync test state");
        std::env::set_var("BUZZ_MEMORY_INHERITED_SENTINEL", "must-clear");
        let directory = tempdir();
        let log_path = directory.path().join("replicate-log");
        let fake_cli = directory.path().join("fake-replicate");
        let fixture = format!(
            r#"#!/bin/sh
set -eu
operation="$1"
shift
printf '%s\n' "$operation" "$@" > '{}'
test -z "${{BUZZ_MEMORY_INHERITED_SENTINEL+x}}"
test "$MEMORY_LOCAL_READ_TOKEN" = 'local-read-token'
test "$MEMORY_LOCAL_REPLICATE_TOKEN" = 'local-replicate-token'
test "$MEMORY_REMOTE_READ_TOKEN" = 'remote-read-token'
test "$MEMORY_REMOTE_REPLICATE_TOKEN" = 'remote-replicate-token'
if [ "$operation" = pull ]; then
  printf '%s\n' '{{"status":"ok","operation":"pull","source_node_id":"node:home-command","target_node_id":"node:macbook-command","from_cursor":2,"to_cursor":5,"accepted":3,"duplicates":0,"conflicts":1,"objects":3,"tombstones":1,"pages":1,"target_conflict_count":2,"last_success":"2026-07-24T00:00:00+00:00"}}'
else
  printf '%s\n' '{{"status":"ok","operation":"push","source_node_id":"node:macbook-command","target_node_id":"node:home-command","from_cursor":4,"to_cursor":6,"accepted":2,"duplicates":0,"conflicts":0,"objects":2,"tombstones":0,"pages":1,"target_conflict_count":1,"last_success":"2026-07-24T00:00:01+00:00"}}'
fi
"#,
            log_path.display()
        );
        protected_file(&fake_cli, fixture.as_bytes(), true);
        let trusted = TrustedMemoryConfig {
            config: serde_json::from_value(config_json(directory.path(), 8006))
                .expect("decode fixture config"),
            secrets: MemorySecrets::fixture(
                "local-read-token",
                "local-replicate-token",
                "remote-read-token",
                "remote-replicate-token",
            ),
        };

        let result =
            run_replication_cli(&fake_cli, "pull", &trusted, 49152, Duration::from_secs(2))
                .expect("valid fake replication");

        std::env::remove_var("BUZZ_MEMORY_INHERITED_SENTINEL");
        assert_eq!(result.source_node_id, "node:home-command");
        assert_eq!(result.target_node_id, "node:macbook-command");
        assert_eq!(result.to_cursor, 5);
        assert_eq!(result.conflicts, 1);
        let log = fs::read_to_string(log_path).expect("read replicate log");
        assert!(log.contains("--local-url"));
        assert!(log.contains("http://127.0.0.1:8006"));
        assert!(log.contains("--remote-url"));
        assert!(log.contains("http://127.0.0.1:49152"));
        assert!(!format!("{result:?}").contains("token"));
    }

    #[test]
    fn replication_timeout_kills_and_reaps_the_cli() {
        let _state = TEST_SYNC_STATE.lock().expect("lock sync test state");
        let directory = tempdir();
        let fake_cli = directory.path().join("hung-replicate");
        protected_file(&fake_cli, b"#!/bin/sh\nexec /bin/sleep 30\n", true);
        let trusted = TrustedMemoryConfig {
            config: serde_json::from_value(config_json(directory.path(), 8006))
                .expect("decode fixture config"),
            secrets: MemorySecrets::fixture("a", "b", "c", "d"),
        };
        let started = Instant::now();

        let error = run_replication_cli(
            &fake_cli,
            "pull",
            &trusted,
            49153,
            Duration::from_millis(100),
        )
        .expect_err("hung CLI must time out");

        assert_eq!(error, MemoryError::Timeout);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn shutdown_cancellation_kills_and_reaps_an_active_cli_without_waiting_for_deadline() {
        let _state = TEST_SYNC_STATE.lock().expect("lock sync test state");
        SYNC_CANCELLED.store(false, Ordering::SeqCst);
        let directory = tempdir();
        let fake_cli = directory.path().join("hung-replicate-cancel");
        protected_file(&fake_cli, b"#!/bin/sh\nexec /bin/sleep 30\n", true);
        let trusted = TrustedMemoryConfig {
            config: serde_json::from_value(config_json(directory.path(), 8006))
                .expect("decode fixture config"),
            secrets: MemorySecrets::fixture("a", "b", "c", "d"),
        };
        let canceller = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(75));
            cancel_active_memory_sync();
        });
        let started = Instant::now();
        let error =
            run_replication_cli(&fake_cli, "pull", &trusted, 49154, Duration::from_secs(30))
                .expect_err("shutdown cancellation must interrupt CLI");
        canceller.join().expect("join canceller");
        SYNC_CANCELLED.store(false, Ordering::SeqCst);

        assert_eq!(error, MemoryError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn fail_soft_response_never_exposes_credentials_or_private_paths() {
        let response = fail_soft_readiness(
            MemoryError::CredentialsUnavailable,
            Some("/Users/alice/.ssh/id_ed25519"),
        );
        let encoded = serde_json::to_string(&response).expect("serialize response");

        assert_eq!(response.status, MemoryServiceStatus::Unavailable);
        assert_eq!(response.error.as_deref(), Some("credentials_unavailable"));
        assert!(!encoded.contains("/Users/alice"));
        assert!(!encoded.contains("id_ed25519"));
        assert!(!encoded.contains("Bearer"));
    }

    #[test]
    fn production_scheduler_runs_at_the_configured_interval_and_stops_cleanly() {
        let gate = Arc::new(SyncGate::default());
        let executions = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&executions);
        let scheduler =
            MemorySyncScheduler::start_for_test(Duration::from_millis(20), gate, move || {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });

        let deadline = Instant::now() + Duration::from_secs(1);
        while executions.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(executions.load(Ordering::SeqCst) >= 1);

        scheduler
            .stop_and_join()
            .expect("scheduler stops and joins");
        let stopped_count = executions.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(executions.load(Ordering::SeqCst), stopped_count);
    }

    #[test]
    fn scheduled_and_explicit_syncs_use_the_same_real_gate() {
        let gate = Arc::new(SyncGate::default());
        let manual_guard = gate.try_enter().expect("manual sync owns gate");
        let executions = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&executions);
        let scheduler = MemorySyncScheduler::start_for_test(
            Duration::from_millis(20),
            Arc::clone(&gate),
            move || {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        );

        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        drop(manual_guard);

        let deadline = Instant::now() + Duration::from_secs(1);
        while executions.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        scheduler.stop_and_join().expect("scheduler joins");
    }

    #[test]
    fn remote_authentication_and_node_identity_precede_any_cli_credentials() {
        let cli_started = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&cli_started);

        let error = run_after_remote_preflight(
            || Err(MemoryError::NodeIdentityMismatch),
            move || {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .expect_err("remote node mismatch must block replication");

        assert_eq!(error, MemoryError::NodeIdentityMismatch);
        assert_eq!(cli_started.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn tunnel_close_failure_overrides_an_otherwise_successful_sync() {
        let error = finish_after_tunnel_close::<()>(
            Ok(()),
            Err(crate::command_services::ssh::SshError::Teardown),
        )
        .expect_err("unreaped tunnel cannot report sync success");

        assert_eq!(error, MemoryError::Teardown);
    }

    #[test]
    fn verified_cli_descriptor_defeats_after_validation_path_replacement() {
        let directory = tempdir();
        let cli_path = directory.path().join("memory-mcp-replicate");
        protected_file(&cli_path, b"#!/bin/sh\nprintf '%s\\n' verified\n", true);
        let executable =
            ProtectedExecutable::open(&cli_path).expect("open verified executable descriptor");
        fs::rename(&cli_path, directory.path().join("original")).expect("move verified inode away");
        protected_file(&cli_path, b"#!/bin/sh\nprintf '%s\\n' swapped\n", true);

        let output = executable
            .spawn_for_test()
            .expect("verified descriptor must execute original inode");

        assert_eq!(output, "verified\n");
    }
}
