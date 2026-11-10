mod postgres_tests {
    use super::super::*;

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn busy_claim_processors_wait_for_the_polling_backstop() {
        use buzz_db::relay_invite::{ClaimCredential, ClaimRequest};
        use tracing_subscriber::{layer::Context, prelude::*, Layer};

        struct Attempts(tokio::sync::mpsc::UnboundedSender<std::time::Instant>);
        impl<S: tracing::Subscriber> Layer<S> for Attempts {
            fn on_new_span(
                &self,
                attrs: &tracing::span::Attributes<'_>,
                _: &tracing::Id,
                _: Context<'_, S>,
            ) {
                if attrs.metadata().name() == "try_admit_invite_claims" {
                    let _ = self.0.send(std::time::Instant::now());
                }
            }
        }

        let host = format!("invite-busy-{}.example", Uuid::new_v4().simple());
        let state = invite_test_state(&host).await.expect("isolated Postgres");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .unwrap()
            .unwrap()
            .id;
        let tenant = buzz_core::TenantContext::resolved(community, &host);
        let mut blocker = state.db.pool().begin().await.unwrap();
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('invite-admission:' || $1::text, 0))",
        )
        .bind(community.as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
        let (starts, mut observed) = tokio::sync::mpsc::unbounded_channel();
        let _capture =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(Attempts(starts)));
        let claimant = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .invite_claim_batches
                    .submit(
                        &state,
                        &tenant,
                        ClaimRequest {
                            pubkey: Keys::generate().public_key().to_hex(),
                            policy_version: None,
                            verified_owner: None,
                            credential: ClaimCredential::Legacy {
                                role: "member".into(),
                            },
                        },
                    )
                    .await
            }
        });
        let attempts = tokio::time::timeout(Duration::from_secs(5), async {
            let first = observed.recv().await.unwrap();
            observed.recv().await.unwrap();
            let third = observed.recv().await.unwrap();
            third.duration_since(first)
        })
        .await;
        blocker.commit().await.unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(5), claimant)
            .await
            .unwrap()
            .unwrap()
            .is_ok());
        // The submit wake may allow one immediate extra pass. Further attempts
        // must wait for the 50 ms backstop instead of self-waking on contention.
        assert!(
            attempts.expect("three processor attempts must be observed")
                >= Duration::from_millis(50)
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn relay_instances_share_batches_and_finish_a_lone_final_claim() {
        use crate::handlers::group_state_postgres_tests::{create_channel, edit_channel};
        let host = format!("invite-batches-{}.example", Uuid::new_v4().simple());
        let first = invite_test_state(&host).await.expect("isolated Postgres");
        let community = first
            .db
            .lookup_community_by_host(&host)
            .await
            .unwrap()
            .unwrap()
            .id;
        let tenant = buzz_core::TenantContext::resolved(community, &host);
        let owner = Keys::generate();
        let _ = first
            .db
            .add_relay_member(
                community,
                &owner.public_key().to_hex(),
                "owner",
                None,
                &first.relay_keypair,
            )
            .await
            .unwrap();
        let channel = create_channel(&first, &tenant, &owner, &[]).await;
        edit_channel(&first, &tenant, &owner, channel, &[&["auto_join", "true"]])
            .await
            .unwrap();
        let event_counts = || async {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT count(*) FILTER (WHERE kind=13534), count(*) FILTER (WHERE kind=39002 AND channel_id=$2) FROM events WHERE community_id=$1",
            ).bind(community.as_uuid()).bind(channel).fetch_one(first.db.pool()).await.unwrap()
        };
        let before = event_counts().await;
        let mut second = (*first).clone();
        second.invite_claim_batches = Arc::default();
        let second = Arc::new(second);
        let invite = first
            .db
            .mint_relay_invite_as_admin(community, &owner.public_key().to_hex(), 3600, None)
            .await
            .unwrap();
        let body = serde_json::json!({"code": invite.code}).to_string();
        let mut blocker = first.db.pool().begin().await.unwrap();
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('invite-admission:' || $1::text, 0))",
        )
        .bind(community.as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
        let mut claims = Vec::new();
        for index in 0..32 {
            let state = if index % 2 == 0 {
                first.clone()
            } else {
                second.clone()
            };
            let (host, body) = (host.clone(), body.clone());
            claims.push(tokio::spawn(async move {
                post_json(state, &host, "/api/invites/claim", &Keys::generate(), body).await
            }));
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let queued: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM relay_invite_claims WHERE community_id=$1",
                )
                .bind(community.as_uuid())
                .fetch_one(first.db.pool())
                .await
                .unwrap();
                if queued == 32 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("every request must enter the shared queue");
        blocker.commit().await.unwrap();
        for claim in claims {
            let response = claim.await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(read_json(response).await["status"], "joined");
        }
        let after = event_counts().await;
        assert_eq!(
            after.0 - before.0,
            1,
            "one community roster for the shared batch"
        );
        assert_eq!(
            after.1 - before.1,
            1,
            "one default-channel roster across both servers"
        );
        let seated: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND removed_at IS NULL",
        ).bind(community.as_uuid()).bind(channel).fetch_one(first.db.pool()).await.unwrap();
        assert_eq!(
            seated, 33,
            "all32 claimants and the existing owner joined the channel"
        );
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM relay_members WHERE community_id=$1 AND auto_join_pending IS NOT NULL")
            .bind(community.as_uuid()).fetch_one(first.db.pool()).await.unwrap();
        assert_eq!(
            pending, 0,
            "HTTP success includes completed default-channel work"
        );
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            post_json(second, &host, "/api/invites/claim", &Keys::generate(), body),
        )
        .await
        .expect("a lone claim must not wait for more arrivals");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(read_json(response).await["status"], "joined");
        let uses: i32 = sqlx::query_scalar(
            "SELECT use_count FROM relay_invites WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(invite.invite_id)
        .fetch_one(first.db.pool())
        .await
        .unwrap();
        assert_eq!(uses, 33);
        let tags: Value = sqlx::query_scalar(
            "SELECT tags FROM events WHERE community_id=$1 AND kind=13534 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .fetch_one(first.db.pool())
        .await
        .unwrap();
        assert_eq!(
            tags.as_array().unwrap().len(),
            35,
            "34members and the protected marker"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn default_channel_batches_publish_once_and_recover_an_interrupted_publication() {
        use crate::handlers::group_state_postgres_tests::{create_channel, edit_channel};
        use crate::handlers::side_effects::{
            auto_join_default_channels, auto_join_default_channels_batch,
        };

        let host = format!("invite-default-batch-{}.example", Uuid::new_v4().simple());
        let state = invite_test_state(&host).await.expect("isolated Postgres");
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
        let mut members = Vec::new();
        for _ in 0..8 {
            let member = Keys::generate().public_key().to_hex();
            let _ = state
                .db
                .add_relay_member_with_default_channels(
                    community,
                    &member,
                    "member",
                    &member,
                    &state.relay_keypair,
                )
                .await
                .unwrap();
            members.push(member);
        }
        let count = || async {
            sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind=39002 AND channel_id=$2",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
        };
        let before = count().await;
        let mut notice = crate::handlers::side_effects::notice_barrier::install(
            &hex::decode(&members[0]).unwrap(),
        );
        let batch = tokio::spawn({
            let (state, tenant, members) = (state.clone(), tenant.clone(), members.clone());
            async move { auto_join_default_channels_batch(&tenant, &state, &members).await }
        });
        let (noticed_channel, release) =
            tokio::time::timeout(Duration::from_secs(5), notice.next())
                .await
                .expect("batch must reach the new member's notification")
                .unwrap();
        let tags: Value = sqlx::query_scalar(
            "SELECT tags FROM events WHERE community_id=$1 AND kind=39002 AND channel_id=$2 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
        release.send(()).unwrap();
        batch.await.unwrap();
        assert_eq!(noticed_channel, channel);
        for member in &members {
            assert!(
                tags.as_array()
                    .unwrap()
                    .iter()
                    .any(|tag| tag[0] == "p" && tag[1] == *member),
                "the complete batched roster must be readable when a join notification arrives"
            );
        }
        assert_eq!(
            count().await - before,
            1,
            "one channel roster per group of joins"
        );
        for member in &members {
            assert!(state
                .db
                .pending_default_channel_join(community, member)
                .await
                .unwrap()
                .is_none());
            assert!(state
                .db
                .is_member(community, channel, &hex::decode(member).unwrap())
                .await
                .unwrap());
        }

        for scenario in ["cancel", "publication_failure", "manual_rejoin"] {
            let member = Keys::generate().public_key().to_hex();
            let _ = state
                .db
                .add_relay_member_with_default_channels(
                    community,
                    &member,
                    "member",
                    &member,
                    &state.relay_keypair,
                )
                .await
                .unwrap();
            let mut blocker = state.db.pool().begin().await.unwrap();
            sqlx::query("LOCK TABLE events IN SHARE MODE")
                .execute(&mut *blocker)
                .await
                .unwrap();
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker)
                .await
                .unwrap();
            let task = tokio::spawn({
                let (state, tenant, member) = (state.clone(), tenant.clone(), member.clone());
                async move { auto_join_default_channels_batch(&tenant, &state, &[member]).await }
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if state
                        .db
                        .is_member(community, channel, &hex::decode(&member).unwrap())
                        .await
                        .unwrap()
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("membership must commit before the held roster publication");
            if scenario == "publication_failure" {
                tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let cancelled: Vec<bool> = sqlx::query_scalar(
                        "SELECT pg_cancel_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid))",
                    ).bind(blocker_pid).fetch_all(state.db.pool()).await.unwrap();
                    if !cancelled.is_empty() {
                        assert_eq!(cancelled, [true]);
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }).await.expect("publication must reach the held events lock");
                task.await.unwrap();
            } else {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            assert!(state
                .db
                .pending_default_channel_join(community, &member)
                .await
                .unwrap()
                .is_some());
            blocker.rollback().await.unwrap();
            if scenario == "manual_rejoin" {
                let pubkey = hex::decode(&member).unwrap();
                state
                    .db
                    .remove_member(community, channel, &pubkey, &pubkey)
                    .await
                    .unwrap();
                state
                    .db
                    .put_member(community, channel, &pubkey, None, None)
                    .await
                    .unwrap();
            }
            auto_join_default_channels(&tenant, &state, &member, None).await;
            assert!(state
                .db
                .pending_default_channel_join(community, &member)
                .await
                .unwrap()
                .is_none());
            let tags: Value = sqlx::query_scalar(
        "SELECT tags FROM events WHERE community_id=$1 AND kind=39002 AND channel_id=$2 AND deleted_at IS NULL",
    ).bind(community.as_uuid()).bind(channel).fetch_one(state.db.pool()).await.unwrap();
            assert!(tags
                .as_array()
                .unwrap()
                .iter()
                .any(|tag| tag[0] == "p" && tag[1] == member));
            let expected = if scenario == "manual_rejoin" { 0 } else { 1 };
            assert_eq!(
                member_joined_notice_count(&state, community, channel, &member).await,
                expected,
                "{scenario}: recover only the admission's original default join"
            );
            assert_eq!(
                added_notification_count(&state, community, &member).await,
                i64::try_from(expected).unwrap()
            );
            auto_join_default_channels(&tenant, &state, &member, None).await;
            assert_eq!(
                member_joined_notice_count(&state, community, channel, &member).await,
                expected
            );
            assert_eq!(
                added_notification_count(&state, community, &member).await,
                i64::try_from(expected).unwrap()
            );
        }

        let member = Keys::generate().public_key().to_hex();
        let _ = state
            .db
            .add_relay_member_with_default_channels(
                community,
                &member,
                "member",
                &member,
                &state.relay_keypair,
            )
            .await
            .unwrap();
        sqlx::query("ALTER TABLE events ADD CONSTRAINT test_default_notice_failure CHECK (kind <> 44100) NOT VALID")
            .execute(state.db.pool()).await.unwrap();
        auto_join_default_channels_batch(&tenant, &state, std::slice::from_ref(&member)).await;
        let pending = state
            .db
            .pending_default_channel_join(community, &member)
            .await
            .unwrap();
        let messages = member_joined_notice_count(&state, community, channel, &member).await;
        let notices = added_notification_count(&state, community, &member).await;
        sqlx::query("ALTER TABLE events DROP CONSTRAINT test_default_notice_failure")
            .execute(state.db.pool())
            .await
            .unwrap();
        assert!(
            pending.is_some(),
            "failed notification must retain retry work"
        );
        assert_eq!(messages, 1, "the preceding system message committed");
        assert_eq!(notices, 0);
        auto_join_default_channels(&tenant, &state, &member, None).await;
        assert_eq!(
            member_joined_notice_count(&state, community, channel, &member).await,
            1,
            "recovery must reuse the stored system message's event ID"
        );
        assert_eq!(
            added_notification_count(&state, community, &member).await,
            1
        );
        assert!(state
            .db
            .pending_default_channel_join(community, &member)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn admission_progresses_while_an_earlier_default_channel_followup_is_held() {
        use crate::handlers::group_state_postgres_tests::{create_channel, edit_channel};
        use buzz_db::relay_invite::{ClaimCredential, ClaimRequest, EnqueuedClaim};

        let host = format!("invite-pipeline-{}.example", Uuid::new_v4().simple());
        let state = invite_test_state(&host).await.expect("isolated Postgres");
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
        let members = [Keys::generate(), Keys::generate()];
        let request = |member: &Keys| ClaimRequest {
            pubkey: member.public_key().to_hex(),
            policy_version: None,
            verified_owner: None,
            credential: ClaimCredential::Legacy {
                role: "member".into(),
            },
        };
        assert!(matches!(
            state
                .db
                .enqueue_invite_claim(community, &request(&members[0]))
                .await
                .unwrap(),
            EnqueuedClaim::Queued(_)
        ));
        let write = state
            .db
            .try_admit_invite_claims(community, &state.relay_keypair)
            .await
            .unwrap()
            .unwrap();
        crate::api::invites::claim_batches::announce_committed_admission(&state, &tenant, write)
            .await;
        let EnqueuedClaim::Queued(second_id) = state
            .db
            .enqueue_invite_claim(community, &request(&members[1]))
            .await
            .unwrap()
        else {
            panic!("second claim must enter the queue");
        };

        let mut admission_lock = state.db.pool().begin().await.unwrap();
        sqlx::query("LOCK TABLE relay_members IN SHARE MODE")
            .execute(&mut *admission_lock)
            .await
            .unwrap();
        let mut notice = crate::handlers::side_effects::notice_barrier::install(
            &members[0].public_key().to_bytes(),
        );
        let processor = tokio::spawn({
            let (state, tenant) = (state.clone(), tenant.clone());
            async move {
                crate::api::invites::claim_batches::process_pending_claims(&state, &tenant).await
            }
        });
        let (noticed_channel, release) =
            tokio::time::timeout(Duration::from_secs(5), notice.next())
                .await
                .expect("follow-up must progress while admission is locked")
                .unwrap();
        admission_lock.commit().await.unwrap();
        let admitted = tokio::time::timeout(Duration::from_secs(5), async {
            while !state
                .db
                .is_relay_member(community, &members[1].public_key().to_hex())
                .await
                .unwrap()
            {
                tokio::task::yield_now().await;
            }
        })
        .await;
        let unreadied = state
            .db
            .invite_claim_results(community, &[second_id])
            .await
            .unwrap();
        release.send(()).unwrap();
        processor.await.unwrap().unwrap();
        assert_eq!(noticed_channel, channel);
        admitted.expect("admission must progress while the earlier follow-up is held");
        assert!(
            unreadied.is_empty(),
            "admission alone must not publish HTTP readiness"
        );
        crate::api::invites::claim_batches::process_pending_claims(&state, &tenant)
            .await
            .unwrap();
        assert_eq!(
            state
                .db
                .invite_claim_results(community, &[second_id])
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn committed_owner_disconnect_precedes_timed_out_roster_delivery() {
        use crate::state::CommunityConnectionControl;
        use buzz_db::relay_invite::{
            ClaimCredential, ClaimRequest, ClaimResult, EnqueuedClaim, StoredClaimResult,
        };
        use tokio_util::sync::CancellationToken;

        let host = format!("invite-owner-order-{}.example", Uuid::new_v4().simple());
        let state = invite_test_state(&host).await.expect("isolated Postgres");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .unwrap()
            .unwrap()
            .id;
        let tenant = buzz_core::TenantContext::resolved(community, &host);
        let agent = Keys::generate().public_key();
        let owner = Keys::generate().public_key();
        let request = ClaimRequest {
            pubkey: agent.to_hex(),
            policy_version: None,
            verified_owner: Some(owner.to_hex()),
            credential: ClaimCredential::Legacy {
                role: "member".into(),
            },
        };
        assert!(matches!(
            state
                .db
                .enqueue_invite_claim(community, &request)
                .await
                .unwrap(),
            EnqueuedClaim::Queued(_)
        ));
        let write = state
            .db
            .try_admit_invite_claims(community, &state.relay_keypair)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(&write.outcome[0].result,
            StoredClaimResult::Success(ClaimResult::Legacy(result)) if result.owner_link_created));
        assert!(
            write.snapshot.is_some(),
            "the committed claim has a roster to deliver"
        );

        let ownerless_cancel = CancellationToken::new();
        let ownerless = CommunityConnectionControl::new(ownerless_cancel.clone());
        ownerless.bind_pubkey(agent.to_bytes());
        let _ownerless_guard =
            state
                .community_connections
                .register(Uuid::new_v4(), community, ownerless);
        let owned_cancel = CancellationToken::new();
        let owned = CommunityConnectionControl::new(owned_cancel.clone());
        owned.bind_pubkey(agent.to_bytes());
        owned.bind_owner(owner.to_bytes());
        let _owned_guard = state
            .community_connections
            .register(Uuid::new_v4(), community, owned);
        let cache_key = (community, agent.to_bytes().to_vec());
        state.seed_restriction(
            cache_key.clone(),
            buzz_db::moderation::RestrictionState::default(),
        );
        assert!(state.restriction_cache.entry(&cache_key).is_some());

        // Reserved capacity cannot be drained by the worker, so roster audit
        // delivery stays pending even if timeout polls its inner future once.
        let audit_tx = state.audit_tx.as_ref().expect("audit enabled");
        let mut audit_capacity = Vec::new();
        while let Ok(permit) = audit_tx.clone().try_reserve_owned() {
            audit_capacity.push(permit);
        }
        assert!(!audit_capacity.is_empty());
        assert_eq!(audit_tx.capacity(), 0);
        assert!(tokio::time::timeout_at(
            tokio::time::Instant::now() - Duration::from_secs(1),
            crate::api::invites::claim_batches::announce_committed_admission(
                &state, &tenant, write
            ),
        )
        .await
        .is_err());

        assert!(ownerless_cancel.is_cancelled(), "a committed owner link closes existing ownerless sockets even when roster delivery times out");
        assert!(
            !owned_cancel.is_cancelled(),
            "sockets already admitted with the owner stay connected"
        );
        assert!(
            state.restriction_cache.entry(&cache_key).is_none(),
            "the owner link invalidates restriction rows before async follow-up"
        );

        // A replacement processor must enforce the committed link too, even
        // when the previous admission stopped during snapshot fan-out.
        let recovered_cancel = CancellationToken::new();
        let recovered = CommunityConnectionControl::new(recovered_cancel.clone());
        recovered.bind_pubkey(agent.to_bytes());
        let _recovered_guard =
            state
                .community_connections
                .register(Uuid::new_v4(), community, recovered);
        let mut batch = state
            .db
            .try_finish_invite_claims(community)
            .await
            .unwrap()
            .unwrap();
        batch.deadline = std::time::Instant::now() - Duration::from_secs(1);
        crate::api::invites::claim_batches::finish_committed_batch(&state, &tenant, batch).await;
        assert!(recovered_cancel.is_cancelled());
        drop(audit_capacity);
    }
}
