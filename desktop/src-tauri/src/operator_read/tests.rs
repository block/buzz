#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

    fn request(now: i64) -> ReadRequest {
        ReadRequest {
            schema_version: SCHEMA_VERSION,
            request_id: uuid::Uuid::new_v4().to_string(),
            operation: "messages".to_string(),
            issued_at: now,
            expires_at: now + MAX_REQUEST_LIFETIME_SECONDS,
            since: 1_787_872_400,
            until: 1_787_958_800,
            limit: 20,
            excerpt_chars: 280,
            expected_relay: None,
            expected_identity_pubkey: None,
            channel: None,
            search: None,
        }
    }

    fn fingerprint<'a>(
        workspace_generation: u64,
        identity_generation: u64,
        relay: &'a str,
        pubkey: &'a str,
    ) -> ScopeFingerprint<'a> {
        ScopeFingerprint {
            workspace_generation,
            identity_generation,
            relay,
            pubkey,
        }
    }

    #[test]
    fn request_allows_only_messages_and_denies_unknown_fields() {
        let now = 1_788_000_000;
        let mut candidate = request(now);
        candidate.operation = "send".to_string();
        assert_eq!(
            validate_request_at(&candidate, now).unwrap_err().code,
            "operation_rejected"
        );

        let mut value = serde_json::to_value(request(now)).unwrap();
        value["content"] = serde_json::json!("write this");
        assert!(serde_json::from_value::<ReadRequest>(value).is_err());
    }

    #[test]
    fn production_operator_rejects_nonproduction_and_nonkeyring_macos_owners() {
        assert!(!production_credential_owner_allowed(
            "xyz.contextdotbuild.buzz.client",
            IdentityStorage::SystemKeyring,
            true,
        ));
        assert!(!production_credential_owner_allowed(
            PRODUCTION_BUNDLE_IDENTIFIER,
            IdentityStorage::SystemKeyring,
            false,
        ));
        #[cfg(target_os = "macos")]
        {
            assert!(!production_credential_owner_allowed(
                PRODUCTION_BUNDLE_IDENTIFIER,
                IdentityStorage::Environment,
                true,
            ));
            assert!(!production_credential_owner_allowed(
                PRODUCTION_BUNDLE_IDENTIFIER,
                IdentityStorage::LocalFile,
                true,
            ));
            assert!(production_credential_owner_allowed(
                PRODUCTION_BUNDLE_IDENTIFIER,
                IdentityStorage::SystemKeyring,
                true,
            ));
            assert!(PRODUCTION_CODE_REQUIREMENT.contains("anchor apple generic"));
            assert!(PRODUCTION_CODE_REQUIREMENT.contains("EYF346PHUG"));
            use security_framework::os::macos::code_signing::SecRequirement;
            assert!(PRODUCTION_CODE_REQUIREMENT
                .parse::<SecRequirement>()
                .is_ok());
        }
    }

    #[test]
    fn same_key_storage_downgrade_cannot_race_the_owner_snapshot() {
        use std::sync::{mpsc, Barrier};

        let state = Arc::new(crate::app_state::build_app_state());
        state.set_identity_storage(IdentityStorage::SystemKeyring);
        let original_pubkey = state.keys.lock().unwrap().public_key();
        let writer_state = state.clone();
        let writer_ready = Arc::new(Barrier::new(2));
        let release_writer = Arc::new(Barrier::new(2));
        let writer_ready_thread = writer_ready.clone();
        let release_writer_thread = release_writer.clone();
        let writer = std::thread::spawn(move || {
            let keys = writer_state.keys.lock().unwrap();
            assert_eq!(keys.public_key(), original_pubkey);
            writer_state.set_identity_storage(IdentityStorage::LocalFile);
            writer_ready_thread.wait();
            release_writer_thread.wait();
        });
        writer_ready.wait();

        let reader_state = state.clone();
        let (result_tx, result_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            result_tx
                .send(capture_production_identity_with_signature(
                    &reader_state,
                    true,
                ))
                .unwrap();
        });
        assert!(result_rx.recv_timeout(Duration::from_millis(50)).is_err());
        release_writer.wait();

        assert_eq!(
            result_rx.recv().unwrap().unwrap_err().code,
            "identity_unavailable"
        );
        writer.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn identity_generation_rejects_keyring_backed_aba_transition() {
        let state = crate::app_state::build_app_state();
        state.set_identity_storage(IdentityStorage::SystemKeyring);
        let initial = capture_production_identity_with_signature(&state, true).unwrap();
        let initial_pubkey = initial.keys.public_key().to_hex();

        {
            let mut keys = state.keys.lock().unwrap();
            *keys = Keys::generate();
            state.set_identity_storage(IdentityStorage::SystemKeyring);
            *keys = initial.keys.clone();
            state.set_identity_storage(IdentityStorage::SystemKeyring);
        }
        let current = capture_production_identity_with_signature(&state, true).unwrap();
        assert_eq!(current.keys.public_key().to_hex(), initial_pubkey);
        assert_ne!(current.generation, initial.generation);
        assert_eq!(
            ensure_scope_values_unchanged(
                fingerprint(
                    7,
                    initial.generation,
                    "wss://buildcontext.communities.buzz.xyz",
                    &initial_pubkey,
                ),
                fingerprint(
                    7,
                    current.generation,
                    "wss://buildcontext.communities.buzz.xyz",
                    &current.keys.public_key().to_hex(),
                ),
            )
            .unwrap_err()
            .code,
            "active_scope_changed"
        );
    }

    #[test]
    fn request_freshness_fails_closed() {
        let now = 1_788_000_000;
        let mut stale = request(now);
        stale.issued_at = now - 60;
        stale.expires_at = now - 30;
        assert_eq!(
            validate_request_at(&stale, now).unwrap_err().code,
            "request_stale"
        );

        let mut future = request(now);
        future.issued_at = now + MAX_CLOCK_SKEW_SECONDS + 1;
        future.expires_at = future.issued_at + 1;
        assert_eq!(
            validate_request_at(&future, now).unwrap_err().code,
            "request_stale"
        );
        assert_eq!(
            ensure_request_not_expired_at(now, now).unwrap_err().code,
            "request_stale"
        );

        let mut overflow = request(now);
        overflow.issued_at = i64::MIN;
        overflow.expires_at = now + 1;
        assert_eq!(
            validate_request_at(&overflow, now).unwrap_err().code,
            "request_stale"
        );
    }

    #[test]
    fn replay_guard_consumes_each_request_once() {
        let guard = ReplayGuard::default();
        guard.consume("one", 120, 100).unwrap();
        assert_eq!(
            guard.consume("one", 120, 100).unwrap_err().code,
            "request_replayed"
        );
        guard.consume("two", 130, 121).unwrap();
        let consumed = guard.consumed.lock().unwrap();
        assert!(!consumed.contains_key("one"));
        assert!(consumed.contains_key("two"));
    }

    #[test]
    fn success_receipt_requires_unchanged_workspace_generation_relay_and_signer() {
        let relay = "wss://buildcontext.communities.buzz.xyz";
        let pubkey = "a".repeat(64);
        let initial = fingerprint(7, 11, relay, &pubkey);
        assert!(ensure_scope_values_unchanged(initial, initial).is_ok());
        let other_pubkey = "b".repeat(64);
        for result in [
            ensure_scope_values_unchanged(initial, fingerprint(8, 11, relay, &pubkey)),
            ensure_scope_values_unchanged(initial, fingerprint(7, 12, relay, &pubkey)),
            ensure_scope_values_unchanged(initial, fingerprint(7, 11, "wss://other", &pubkey)),
            ensure_scope_values_unchanged(initial, fingerprint(7, 11, relay, &other_pubkey)),
        ] {
            assert_eq!(result.unwrap_err().code, "active_scope_changed");
        }
    }

    #[test]
    fn client_requires_unchanged_socket_signed_peer_and_bound_receipt() {
        assert!(ensure_server_authentication(true, true).is_ok());
        for result in [
            ensure_server_authentication(false, true),
            ensure_server_authentication(true, false),
        ] {
            assert_eq!(result.unwrap_err().code, "server_rejected");
        }

        let mut receipt = error_receipt("request".to_string(), &OperatorError::new("x", "y"));
        receipt.desktop_pid = 42;
        assert!(validate_receipt_binding(&receipt, "request", 42).is_ok());
        assert_eq!(
            validate_receipt_binding(&receipt, "other", 42)
                .unwrap_err()
                .code,
            "receipt_invalid"
        );
        assert_eq!(
            validate_receipt_binding(&receipt, "request", 43)
                .unwrap_err()
                .code,
            "receipt_invalid"
        );
    }

    #[test]
    fn relay_is_assertion_only_and_canonical() {
        assert!(validate_relay("wss://buildcontext.communities.buzz.xyz").is_ok());
        assert!(validate_relay("https://buildcontext.communities.buzz.xyz/").is_ok());
        for rejected in [
            "wss://example.com",
            "http://buildcontext.communities.buzz.xyz",
            "wss://buildcontext.communities.buzz.xyz:8443",
            "wss://buildcontext.communities.buzz.xyz/query",
        ] {
            assert_eq!(validate_relay(rejected).unwrap_err().code, "relay_rejected");
        }
    }

    #[test]
    fn request_bounds_identity_range_and_search() {
        let now = 1_788_000_000;
        let mut candidate = request(now);
        candidate.limit = MAX_RESULTS + 1;
        assert_eq!(
            validate_request_at(&candidate, now).unwrap_err().code,
            "limit_rejected"
        );
        candidate.limit = 1;
        candidate.search = Some("x".repeat(MAX_SEARCH_CHARS + 1));
        assert_eq!(
            validate_request_at(&candidate, now).unwrap_err().code,
            "search_rejected"
        );
        candidate.search = None;
        candidate.expected_identity_pubkey = Some("not-a-pubkey".to_string());
        assert_eq!(
            validate_request_at(&candidate, now).unwrap_err().code,
            "identity_rejected"
        );
    }

    #[test]
    fn projection_bounds_redacts_and_filters_content() {
        let channel = "123e4567-e89b-12d3-a456-426614174000";
        let content = format!(
            "completed alpha BUZZ_PRIVATE_KEY={} Authorization=Bearer-abc secret=shh {}",
            "nsec1".to_string() + &"q".repeat(80),
            "x".repeat(800)
        );
        let event = EventBuilder::new(Kind::Custom(40002), content)
            .tags([Tag::parse(["h", channel]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let projected = project_event(&event, 160, &HashMap::new());
        let excerpt = projected.excerpt.unwrap();
        assert!(excerpt.chars().count() <= 160);
        assert!(!excerpt.contains("nsec1"));
        assert!(!excerpt.contains("Bearer-abc"));
        assert!(!excerpt.contains("secret=shh"));
        assert_eq!(bounded_excerpt("secret=shh", 4), "sec…");
        assert_eq!(bounded_excerpt("secret=shh", 1), "…");
        assert_eq!(bounded_excerpt("secret=shh", 0), "");

        let mut matching = request(1_788_000_000);
        matching.since = event.created_at.as_secs() as i64 - 1;
        matching.until = event.created_at.as_secs() as i64 + 1;
        matching.channel = Some(channel.to_string());
        matching.search = Some("ALPHA".to_string());
        assert!(event_matches_request(&event, &matching));
        matching.search = Some("missing".to_string());
        assert!(!event_matches_request(&event, &matching));

        let invalid_channel = EventBuilder::new(Kind::Custom(40002), "safe")
            .tags([Tag::parse(["h", "not-a-uuid\nsecret=bad"]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(project_event(&invalid_channel, 10, &HashMap::new())
            .channel
            .is_none());
    }

    #[test]
    fn event_signatures_are_verified() {
        let event = EventBuilder::text_note("verified")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(verify_event_set(std::slice::from_ref(&event)).is_ok());
        let mut value = serde_json::to_value(event).unwrap();
        value["content"] = serde_json::json!("tampered");
        let tampered: Event = serde_json::from_value(value).unwrap();
        assert_eq!(
            verify_event_set(&[tampered]).unwrap_err().code,
            "response_unverified"
        );
    }

    #[test]
    fn owner_only_directory_and_socket_replacement_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("operator-read");
        ensure_owner_only_dir(&directory).unwrap();
        assert_eq!(
            fs::symlink_metadata(&directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            ensure_owner_only_dir(&directory).unwrap_err().code,
            "control_dir_rejected"
        );

        let socket_path = temp.path().join("desktop.sock");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&socket_path)
            .unwrap();
        file.write_all(b"replacement").unwrap();
        assert_eq!(
            prepare_socket_path(&socket_path).unwrap_err().code,
            "socket_rejected"
        );
    }

    #[test]
    fn bundled_client_symlink_is_created_refreshed_and_never_clobbers_regular_files() {
        let temp = tempfile::tempdir().unwrap();
        let app_bin = temp.path().join("Buzz.app/Contents/MacOS");
        let local_bin = temp.path().join("local-bin");
        fs::create_dir_all(&app_bin).unwrap();
        fs::write(app_bin.join("buzz-read"), b"credentialless client").unwrap();

        ensure_client_symlink_at(&app_bin, &local_bin).unwrap();
        let link = local_bin.join("buzz-read");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), app_bin.join("buzz-read"));

        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(temp.path().join("wrong"), &link).unwrap();
        ensure_client_symlink_at(&app_bin, &local_bin).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), app_bin.join("buzz-read"));

        fs::remove_file(&link).unwrap();
        fs::write(&link, b"user-owned client").unwrap();
        ensure_client_symlink_at(&app_bin, &local_bin).unwrap();
        assert_eq!(fs::read(&link).unwrap(), b"user-owned client");
    }

    #[tokio::test]
    async fn framed_socket_io_is_bounded() {
        let (mut left, mut right) = UnixStream::pair().unwrap();
        let writer = tokio::spawn(async move {
            write_frame_async(&mut left, b"hello", 5).await.unwrap();
        });
        assert_eq!(read_frame_async(&mut right, 5).await.unwrap(), b"hello");
        writer.await.unwrap();

        let (mut left, mut right) = UnixStream::pair().unwrap();
        let writer = tokio::spawn(async move {
            left.write_all(&6_u32.to_be_bytes()).await.unwrap();
        });
        assert_eq!(
            read_frame_async(&mut right, 5).await.unwrap_err().code,
            "request_oversize"
        );
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn unix_socket_peer_is_the_current_owner() {
        let (left, _right) = UnixStream::pair().unwrap();
        validate_peer(&left).unwrap();
        assert_eq!(
            ensure_peer_uid(current_uid().saturating_add(1), current_uid())
                .unwrap_err()
                .code,
            "peer_rejected"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn whole_request_timeout_cancels_in_flight_execution_at_expiry() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct DropSignal(Arc<AtomicBool>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = DropSignal(cancelled.clone());
        let task = tokio::spawn(run_with_expiry_timeout(
            Duration::from_secs(5),
            async move {
                let _signal = signal;
                std::future::pending::<Result<(), OperatorError>>().await
            },
        ));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(5)).await;

        assert_eq!(task.await.unwrap().unwrap_err().code, "request_stale");
        assert!(cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn receipt_bound_is_enforced() {
        let mut receipt = error_receipt("id".to_string(), &OperatorError::new("x", "y"));
        receipt.events = (0..MAX_RESULTS)
            .map(|index| ReceiptEvent {
                id: format!("{index:064x}"),
                author_pubkey: "a".repeat(64),
                author_name: Some("name".to_string()),
                kind: 40002,
                created_at: index as i64,
                channel: Some("123e4567-e89b-12d3-a456-426614174000".to_string()),
                excerpt: Some("x".repeat(MAX_EXCERPT_CHARS as usize)),
            })
            .collect();
        assert!(ensure_receipt_bound(&receipt).is_ok());
        receipt.events.push(ReceiptEvent {
            id: "z".repeat(MAX_RECEIPT_BYTES),
            author_pubkey: String::new(),
            author_name: None,
            kind: 9,
            created_at: 0,
            channel: None,
            excerpt: None,
        });
        assert_eq!(
            ensure_receipt_bound(&receipt).unwrap_err().code,
            "receipt_oversize"
        );
    }
}
