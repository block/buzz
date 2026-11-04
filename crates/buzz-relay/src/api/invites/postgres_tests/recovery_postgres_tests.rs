mod postgres_tests {
    use super::super::*;
    use crate::handlers::group_state_postgres_tests::{create_channel, edit_channel, http_auth};
    use crate::handlers::ingest::ingest_event;
    use crate::handlers::side_effects::auto_join_default_channels;

    // Cancel the real listing query only after PostgreSQL reports that it is
    // blocked by our table lock. No timer or production-only failure hook.
    async fn cancel_blocked_query(pool: &sqlx::PgPool, pattern: &str) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let cancelled: Vec<bool> = sqlx::query_scalar(
                    "SELECT pg_cancel_backend(pid) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' \
                 AND query LIKE $1",
                )
                .bind(pattern)
                .fetch_all(pool)
                .await
                .expect("cancel blocked default-channel listing");
                if !cancelled.is_empty() {
                    assert!(cancelled.into_iter().all(|cancelled| cancelled));
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the admission must reach the actual default-channel listing");
    }

    async fn recovery_fixture(
        label: &str,
        channel_count: usize,
    ) -> (
        Arc<AppState>,
        buzz_core::TenantContext,
        Keys,
        Keys,
        Vec<Uuid>,
    ) {
        let host = format!("join-{label}-{}.example", Uuid::new_v4().simple());
        let state = invite_test_state(&host).await.expect("isolated Postgres");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .unwrap()
            .unwrap()
            .id;
        let tenant = buzz_core::TenantContext::resolved(community, &host);
        let (owner, member) = (Keys::generate(), Keys::generate());
        let _ = state
            .db
            .add_relay_member(
                community,
                &owner.public_key().to_hex(),
                "owner",
                None,
                &state.relay_keypair,
            )
            .await
            .unwrap();
        let mut channels = Vec::new();
        for _ in 0..channel_count {
            let channel = create_channel(&state, &tenant, &owner, &[]).await;
            edit_channel(&state, &tenant, &owner, channel, &[&["auto_join", "true"]])
                .await
                .unwrap();
            channels.push(channel);
        }
        assert_eq!(
            state
                .db
                .list_auto_join_channels(community)
                .await
                .unwrap()
                .iter()
                .map(|channel| channel.id)
                .collect::<Vec<_>>(),
            channels,
            "the failing channel must precede the healthy one",
        );
        let _ = state
            .db
            .add_relay_member_with_default_channels(
                community,
                &member.public_key().to_hex(),
                "member",
                &member.public_key().to_hex(),
                &state.relay_keypair,
            )
            .await
            .unwrap();
        (state, tenant, owner, member, channels)
    }

    // Target only the connection blocked by this test's transaction, even
    // when another recovery test is running against the same database.
    async fn cancel_query_blocked_by(pool: &sqlx::PgPool, blocker: i32) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let cancelled: Vec<bool> = sqlx::query_scalar(
                    "SELECT pg_cancel_backend(pid) FROM pg_stat_activity \
                     WHERE datname = current_database() AND wait_event_type = 'Lock' \
                     AND $1 = ANY(pg_blocking_pids(pid))",
                )
                .bind(blocker)
                .fetch_all(pool)
                .await
                .unwrap();
                if !cancelled.is_empty() {
                    assert_eq!(cancelled, [true], "exactly our recovery query was blocked");
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("recovery must reach the held PostgreSQL lock");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn channel_local_failures_continue_later_defaults_and_retain_retry_work() {
        let (state, tenant, _, member, channels) = recovery_fixture("membership", 2).await;
        let community = tenant.community();
        let member_hex = member.public_key().to_hex();
        let pending = state
            .db
            .pending_default_channel_join(community, &member_hex)
            .await
            .unwrap()
            .unwrap();
        let mut blocker = state.db.pool().begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM channels WHERE community_id = $1 AND id = $2 FOR UPDATE")
            .bind(community.as_uuid())
            .bind(channels[0])
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let retry = tokio::spawn({
            let (state, tenant, member_hex) = (state.clone(), tenant.clone(), member_hex.clone());
            async move { auto_join_default_channels(&tenant, &state, &member_hex, None).await }
        });
        cancel_query_blocked_by(state.db.pool(), blocker_pid).await;
        // Only the first channel fails. Later memberships can write.
        blocker.rollback().await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), retry)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            channel_roles(&state, community, &channels, &member).await,
            [None, Some("member".into())],
            "a local failure must not strand the later channel",
        );
        assert_eq!(
            state
                .db
                .pending_default_channel_join(community, &member_hex)
                .await
                .unwrap()
                .unwrap()
                .token,
            pending.token
        );
        auto_join_default_channels(&tenant, &state, &member_hex, None).await;
        assert_eq!(
            channel_roles(&state, community, &channels, &member).await,
            [Some("member".into()), Some("member".into())]
        );
        assert!(state
            .db
            .pending_default_channel_join(community, &member_hex)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            added_notification_count(&state, community, &member_hex).await,
            2
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn temporary_listing_failure_is_recovered_for_each_admission_path() {
        let host = format!("join-recovery-{}.example", Uuid::new_v4().simple());
        let state = invite_test_state(&host).await.expect("test state");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .unwrap()
            .unwrap()
            .id;
        let tenant = buzz_core::TenantContext::resolved(community, &host);
        let owner = Keys::generate();
        let _ = state
            .db
            .add_relay_member(
                community,
                &owner.public_key().to_hex(),
                "owner",
                None,
                &state.relay_keypair,
            )
            .await
            .unwrap();
        let channel = create_channel(&state, &tenant, &owner, &[]).await;
        edit_channel(&state, &tenant, &owner, channel, &[&["auto_join", "true"]])
            .await
            .unwrap();

        for path in ["v1", "v2", "admin"] {
            let member = Keys::generate();
            let member_hex = member.public_key().to_hex();
            let body = match path {
            "v1" => serde_json::json!({"code": forge_v1_invite_code(&state, community, unix_now() + 3600)}).to_string(),
            "v2" => serde_json::json!({"code": mint_code(state.clone(), &host, &owner,
                serde_json::json!({"max_uses": 1})).await}).to_string(),
            _ => String::new(),
        };
            let add = EventBuilder::new(Kind::Custom(9030), "")
                .tags([Tag::public_key(member.public_key())])
                .sign_with_keys(&owner)
                .unwrap();
            let admit = || {
                let (state, host, member, body, tenant, add, owner) = (
                    state.clone(),
                    host.clone(),
                    member.clone(),
                    body.clone(),
                    tenant.clone(),
                    add.clone(),
                    owner.clone(),
                );
                async move {
                    if path == "admin" {
                        let mut auth = http_auth(&owner);
                        if let crate::handlers::ingest::IngestAuth::Http { scopes, .. } = &mut auth
                        {
                            scopes.push(buzz_auth::Scope::AdminUsers);
                        }
                        let result = ingest_event(&state, &tenant, add, auth).await.unwrap();
                        assert!(result.accepted, "{:?}", result.message);
                    } else {
                        let response =
                            post_json(state, &host, "/api/invites/claim", &member, body).await;
                        assert_eq!(response.status(), StatusCode::OK);
                    }
                }
            };
            let mut blocker = state.db.pool().begin().await.unwrap();
            sqlx::query("LOCK TABLE channels IN ACCESS EXCLUSIVE MODE")
                .execute(&mut *blocker)
                .await
                .unwrap();
            let request = tokio::spawn(admit());
            cancel_blocked_query(
                state.db.pool(),
                "%FROM channels%WHERE community_id = $1 AND auto_join%",
            )
            .await;
            tokio::time::timeout(Duration::from_secs(10), request)
                .await
                .unwrap()
                .unwrap();
            blocker.rollback().await.unwrap();
            assert!(state
                .db
                .is_relay_member(community, &member_hex)
                .await
                .unwrap());
            let pending = state
                .db
                .pending_default_channel_join(community, &member_hex)
                .await
                .unwrap();
            assert!(
                pending.is_some(),
                "{path}: committed admission lost its retry record"
            );
            assert_eq!(
                channel_roles(&state, community, &[channel], &member).await,
                [None]
            );

            // A retry is already_member (and a single-use v2 invite is exhausted
            // for new claimants), so recovery cannot depend on a fresh insertion.
            admit().await;
            assert_eq!(
                channel_roles(&state, community, &[channel], &member).await,
                [Some("member".to_string())],
                "{path}: retry must finish the join"
            );
            assert!(state
                .db
                .pending_default_channel_join(community, &member_hex)
                .await
                .unwrap()
                .is_none());
            assert_eq!(
                added_notification_count(&state, community, &member_hex).await,
                1
            );
            let leave = EventBuilder::new(Kind::Custom(9022), "")
                .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
                .sign_with_keys(&member)
                .unwrap();
            assert!(
                ingest_event(&state, &tenant, leave, http_auth(&member))
                    .await
                    .unwrap()
                    .accepted
            );
            admit().await;
            assert_eq!(
                channel_roles(&state, community, &[channel], &member).await,
                [None]
            );
        }
    }

    /// Recovery preserves a leave made during partial completion, rejects stale
    /// generations after removal/re-addition, and still enforces the original actor.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn pending_join_preserves_leaves_and_fences_old_admissions() {
        let host = format!("join-fence-{}.example", Uuid::new_v4().simple());
        let state = invite_test_state(&host).await.unwrap();
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .unwrap()
            .unwrap()
            .id;
        let tenant = buzz_core::TenantContext::resolved(community, &host);
        let (owner, member) = (Keys::generate(), Keys::generate());
        let member_hex = member.public_key().to_hex();
        for (keys, role) in [(&owner, "owner"), (&member, "member")] {
            let _ = state
                .db
                .add_relay_member_with_default_channels(
                    community,
                    &keys.public_key().to_hex(),
                    role,
                    &owner.public_key().to_hex(),
                    &state.relay_keypair,
                )
                .await
                .unwrap();
        }
        let a = create_channel(&state, &tenant, &owner, &[]).await;
        let b = create_channel(&state, &tenant, &owner, &[]).await;
        for channel in [a, b] {
            edit_channel(&state, &tenant, &owner, channel, &[&["auto_join", "true"]])
                .await
                .unwrap();
        }
        let old = state
            .db
            .pending_default_channel_join(community, &member_hex)
            .await
            .unwrap()
            .unwrap()
            .token;
        assert!(state
            .db
            .put_pending_default_member(community, a, &member.public_key().to_bytes(), old)
            .await
            .unwrap()
            .is_some());
        state
            .db
            .remove_member(
                community,
                a,
                &member.public_key().to_bytes(),
                &member.public_key().to_bytes(),
            )
            .await
            .unwrap();
        auto_join_default_channels(&tenant, &state, &member_hex, None).await;
        assert_eq!(
            channel_roles(&state, community, &[a, b], &member).await,
            [None, Some("member".to_string())]
        );

        let _ = state
            .db
            .remove_relay_member(community, &member_hex, &state.relay_keypair)
            .await
            .unwrap();
        let _ = state
            .db
            .add_relay_member_with_default_channels(
                community,
                &member_hex,
                "member",
                &owner.public_key().to_hex(),
                &state.relay_keypair,
            )
            .await
            .unwrap();
        let new = state
            .db
            .pending_default_channel_join(community, &member_hex)
            .await
            .unwrap()
            .unwrap()
            .token;
        assert_ne!(old, new);
        let c = create_channel(&state, &tenant, &owner, &[]).await;
        edit_channel(&state, &tenant, &owner, c, &[&["auto_join", "true"]])
            .await
            .unwrap();
        assert!(state
            .db
            .put_pending_default_member(community, c, &member.public_key().to_bytes(), old)
            .await
            .unwrap()
            .is_none());
        state
            .db
            .finish_default_channel_join(community, &member_hex, old)
            .await
            .unwrap();
        assert_eq!(
            state
                .db
                .pending_default_channel_join(community, &member_hex)
                .await
                .unwrap()
                .unwrap()
                .token,
            new
        );
        assert!(
            state
                .db
                .put_pending_default_member(community, a, &member.public_key().to_bytes(), new,)
                .await
                .unwrap()
                .is_some(),
            "a fresh roster admission restores a leave from the previous admission"
        );
        // If the failed admin-add is retried by the member, it must still ask the
        // original admin's permission rather than treating the retry as self-join.
        state
            .db
            .ensure_user(community, &member.public_key().to_bytes())
            .await
            .unwrap();
        sqlx::query(
        "UPDATE users SET channel_add_policy = 'nobody' WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(member.public_key().to_bytes().as_slice())
    .execute(state.db.pool())
    .await
    .unwrap();
        auto_join_default_channels(&tenant, &state, &member_hex, None).await;
        assert_eq!(
            channel_roles(&state, community, &[c], &member).await,
            [None]
        );
        assert!(state
            .db
            .pending_default_channel_join(community, &member_hex)
            .await
            .unwrap()
            .is_none());
    }
}
