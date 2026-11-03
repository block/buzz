//! Live writer-admission regressions. A one-connection pool runs with ordinary
//! PostgreSQL triggers disabled, so a missing application check is observable.

use crate::{Db, DbError};
use buzz_core::CommunityId;
use uuid::Uuid;

use crate::test_support::admission_fixture as fixture;

fn assert_fenced(error: DbError) {
    assert!(
        matches!(&error, DbError::AccessDenied(message) if message.contains("write-fenced")),
        "expected application admission rejection, got {error:?}"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn api_token_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_fenced(
            super::api_token::create_api_token(
                &db.pool,
                *community.as_uuid(),
                &[1; 32],
                &[2; 32],
                "test",
                &[],
                None,
                None,
            )
            .await
            .expect_err("token create must reject"),
        );
        assert_fenced(
            super::api_token::revoke_all_tokens(&db.pool, *community.as_uuid(), &[2; 32], &[2; 32])
                .await
                .expect_err("token revoke must reject"),
        );
    }
    super::api_token::create_api_token(
        &db.pool,
        *communities[0].as_uuid(),
        &[3; 32],
        &[2; 32],
        "active",
        &[],
        None,
        None,
    )
    .await
    .expect("active token create succeeds");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archived_identity_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    let pubkey = "a".repeat(64);
    let actor = "b".repeat(64);
    let event = "c".repeat(64);
    for community in &communities[1..] {
        assert_fenced(
            super::archived_identities::archive(
                &db.pool, *community, &pubkey, "self", &actor, None, None, &event,
            )
            .await
            .expect_err("archive must reject"),
        );
        assert_fenced(
            super::archived_identities::unarchive(&db.pool, *community, &pubkey)
                .await
                .expect_err("unarchive must reject"),
        );
    }
    assert!(super::archived_identities::archive(
        &db.pool,
        communities[0],
        &pubkey,
        "self",
        &actor,
        None,
        None,
        &event,
    )
    .await
    .expect("active archive succeeds"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn allowlist_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_fenced(
            db.add_to_allowlist(*community, &[1; 32], &[2; 32], None)
                .await
                .expect_err("allowlist add must reject"),
        );
        assert_fenced(
            db.remove_from_allowlist(*community, &[1; 32])
                .await
                .expect_err("allowlist remove must reject"),
        );
    }
    assert!(db
        .add_to_allowlist(communities[0], &[1; 32], &[2; 32], None)
        .await
        .expect("active allowlist add succeeds"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn git_name_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_fenced(
            super::git_repo::reserve_repo_name(&db.pool, *community, "repo", "owner")
                .await
                .expect_err("repo reserve must reject"),
        );
        assert_fenced(
            super::git_repo::release_repo_name(&db.pool, *community, "repo", "owner")
                .await
                .expect_err("repo release must reject"),
        );
    }
    assert_eq!(
        super::git_repo::reserve_repo_name(&db.pool, communities[0], "repo", "owner")
            .await
            .expect("active repo reserve succeeds"),
        super::git_repo::ReserveOutcome::Reserved
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn relay_member_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    let pubkey = "d".repeat(64);
    for community in &communities[1..] {
        assert_fenced(
            super::relay_members::add_relay_member(
                &db.pool,
                *community,
                &pubkey,
                "member",
                None,
                &nostr::Keys::generate(),
            )
            .await
            .map(|write| write.outcome)
            .expect_err("member add must reject"),
        );
        assert_fenced(
            super::relay_members::remove_relay_member(
                &db.pool,
                *community,
                &pubkey,
                &nostr::Keys::generate(),
            )
            .await
            .map(|write| write.outcome)
            .expect_err("member remove must reject"),
        );
    }
    assert!(super::relay_members::add_relay_member(
        &db.pool,
        communities[0],
        &pubkey,
        "member",
        None,
        &nostr::Keys::generate(),
    )
    .await
    .map(|write| write.outcome)
    .expect("active member add succeeds"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn relay_invite_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_fenced(
            super::relay_invite::mint_relay_invite(
                &db.pool,
                *community,
                &"e".repeat(64),
                3600,
                Some(1),
            )
            .await
            .expect_err("invite mint must reject"),
        );
        assert_fenced(
            super::relay_invite::claim_relay_invite(
                &db.pool,
                *community,
                &[3; 32],
                &"f".repeat(64),
                None,
                &nostr::Keys::generate(),
            )
            .await
            .map(|write| write.outcome)
            .expect_err("invite claim must reject before row lock"),
        );
    }
    super::relay_invite::mint_relay_invite(
        &db.pool,
        communities[0],
        &"e".repeat(64),
        3600,
        Some(1),
    )
    .await
    .expect("active invite mint succeeds");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn user_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_fenced(
            super::user::ensure_user(&db.pool, *community, &[1; 32])
                .await
                .expect_err("user ensure must reject"),
        );
        assert_fenced(
            super::user::update_user_profile(
                &db.pool,
                *community,
                &[1; 32],
                Some("name"),
                None,
                None,
                None,
            )
            .await
            .expect_err("profile update must reject"),
        );
    }
    assert!(super::user::ensure_user(&db.pool, communities[0], &[1; 32])
        .await
        .expect("active user ensure succeeds"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn moderation_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_fenced(
            super::moderation::insert_report(
                &db.pool,
                *community,
                super::moderation::NewReport {
                    report_event_id: &[3; 32],
                    reporter_pubkey: &[1; 32],
                    target: super::moderation::ReportTarget::Pubkey(vec![2; 32]),
                    channel_id: None,
                    report_type: "spam",
                    note: None,
                },
            )
            .await
            .expect_err("moderation report must reject"),
        );
        assert_fenced(
            super::moderation::insert_action(
                &db.pool,
                *community,
                super::moderation::NewAction {
                    actor_pubkey: &[1; 32],
                    action: "ban",
                    target_pubkey: Some(&[2; 32]),
                    target_event_id: None,
                    channel_id: None,
                    reason_code: None,
                    public_reason: None,
                    private_reason: None,
                    matched_principal: None,
                    actor_authority: None,
                },
            )
            .await
            .expect_err("moderation action must reject"),
        );
        assert_fenced(
            super::moderation::ban_member(&db.pool, *community, &[1; 32], &[2; 32], None, None)
                .await
                .expect_err("ban must reject"),
        );
        assert_fenced(
            super::moderation::unban_member(&db.pool, *community, &[1; 32], &[2; 32])
                .await
                .expect_err("unban must reject"),
        );
    }
    super::moderation::ban_member(&db.pool, communities[0], &[1; 32], &[2; 32], None, None)
        .await
        .expect("active ban succeeds");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_fenced(
            super::workflow::create_workflow(
                &db.pool, *community, None, &[1; 32], "workflow", "{}", &[2; 32],
            )
            .await
            .expect_err("workflow create must reject"),
        );
        assert_fenced(
            super::workflow::create_workflow_run(&db.pool, *community, Uuid::new_v4(), None, None)
                .await
                .expect_err("workflow run create must reject"),
        );
        assert_fenced(
            super::workflow::claim_scheduled_workflow_fire(
                &db.pool,
                *community,
                Uuid::new_v4(),
                chrono::Utc::now(),
            )
            .await
            .expect_err("schedule claim must reject"),
        );
        assert_fenced(
            super::workflow::attach_scheduled_workflow_run(
                &db.pool,
                *community,
                Uuid::new_v4(),
                chrono::Utc::now(),
                Uuid::new_v4(),
            )
            .await
            .expect_err("schedule attach must reject"),
        );
        assert_fenced(
            super::workflow::create_approval(
                &db.pool,
                super::workflow::CreateApprovalParams {
                    community_id: *community,
                    token: "fenced-approval",
                    workflow_id: Uuid::new_v4(),
                    run_id: Uuid::new_v4(),
                    step_id: "step",
                    step_index: 0,
                    approver_spec: "owner",
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                },
            )
            .await
            .expect_err("approval create must reject"),
        );
        assert_fenced(
            super::workflow::update_approval_by_stored_hash(
                &db.pool,
                *community,
                &[4; 32],
                super::workflow::ApprovalStatus::Granted,
                Some(&[1; 32]),
                None,
            )
            .await
            .expect_err("approval update must reject"),
        );
    }
    super::workflow::create_workflow(
        &db.pool,
        communities[0],
        None,
        &[1; 32],
        "active",
        "{}",
        &[2; 32],
    )
    .await
    .expect("active workflow create succeeds");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn push_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    let version = super::push::LeaseVersion {
        source_event_id: &[3; 32],
        source_created_at: chrono::Utc::now().timestamp(),
        generation: 1,
        expires_at: chrono::Utc::now().timestamp() + 3600,
    };
    for community in &communities[1..] {
        assert_fenced(
            super::push::revoke_lease(&db.pool, *community, &[1; 32], "install", version)
                .await
                .expect_err("lease revoke must reject"),
        );
        assert_fenced(
            super::push::complete_match_batch(&db.pool, *community, Uuid::new_v4(), &[vec![2; 32]])
                .await
                .expect_err("matcher complete must reject"),
        );
        assert_fenced(
            super::push::complete_wake(&db.pool, *community, Uuid::new_v4(), Uuid::new_v4())
                .await
                .expect_err("wake complete must reject"),
        );
    }
    super::push::revoke_lease(&db.pool, communities[0], &[1; 32], "install", version)
        .await
        .expect("active lease revoke succeeds");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_and_membership_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    let creator = [1_u8; 32];
    for community in &communities[1..] {
        assert_fenced(
            super::channel::create_channel(
                &db.pool,
                *community,
                "fenced",
                super::channel::ChannelType::Stream,
                super::channel::ChannelVisibility::Open,
                None,
                &creator,
                None,
                &[],
                false,
            )
            .await
            .expect_err("channel create must reject"),
        );
        assert_fenced(
            super::channel::set_canvas(&db.pool, *community, Uuid::new_v4(), Some("text"))
                .await
                .expect_err("canvas update must reject"),
        );
        assert_fenced(
            super::channel_members::add_member(
                &db.pool,
                *community,
                Uuid::new_v4(),
                &[2; 32],
                super::channel::MemberRole::Member,
                None,
            )
            .await
            .expect_err("membership add must reject"),
        );
    }
    let channel = super::channel::create_channel(
        &db.pool,
        communities[0],
        "active",
        super::channel::ChannelType::Stream,
        super::channel::ChannelVisibility::Open,
        None,
        &creator,
        None,
        &[],
        false,
    )
    .await
    .expect("active channel create succeeds");
    super::channel_members::add_member(
        &db.pool,
        communities[0],
        channel.id,
        &[2; 32],
        super::channel::MemberRole::Member,
        None,
    )
    .await
    .expect("active membership add succeeds");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dm_writers_require_application_admission() {
    let (db, communities) = fixture().await;
    let participants: [&[u8]; 2] = [&[1; 32], &[2; 32]];
    for community in &communities[1..] {
        assert_fenced(
            super::dm::create_dm(&db.pool, *community, &participants, participants[0])
                .await
                .expect_err("DM create must reject"),
        );
        assert_fenced(
            super::dm::hide_dm(&db.pool, *community, Uuid::new_v4(), participants[0])
                .await
                .expect_err("DM hide must reject"),
        );
    }
    super::dm::create_dm(&db.pool, communities[0], &participants, participants[0])
        .await
        .expect("active DM create succeeds");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_metadata_writer_requires_application_admission() {
    let (db, communities) = fixture().await;
    let timestamp = chrono::Utc::now();
    for community in &communities[1..] {
        assert_fenced(
            super::thread::insert_thread_metadata(
                &db.pool,
                *community,
                &[1; 32],
                timestamp,
                Uuid::new_v4(),
                None,
                None,
                None,
                None,
                0,
                false,
            )
            .await
            .expect_err("thread metadata must reject"),
        );
    }
    let channel = super::channel::create_channel(
        &db.pool,
        communities[0],
        "thread",
        super::channel::ChannelType::Stream,
        super::channel::ChannelVisibility::Open,
        None,
        &[1; 32],
        None,
        &[],
        false,
    )
    .await
    .expect("active channel");
    super::thread::insert_thread_metadata(
        &db.pool,
        communities[0],
        &[1; 32],
        timestamp,
        channel.id,
        None,
        None,
        None,
        None,
        0,
        false,
    )
    .await
    .expect("active thread metadata insert succeeds");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn relay_admin_domain_writer_requires_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_fenced(
            super::relay_admin_actions::deploy_kick_member(
                &db.pool,
                *community,
                Uuid::new_v4(),
                &[1; 32],
                &[2; 32],
            )
            .await
            .expect_err("deployment kick must reject"),
        );
    }
    let channel = super::channel::create_channel(
        &db.pool,
        communities[0],
        "kick",
        super::channel::ChannelType::Stream,
        super::channel::ChannelVisibility::Open,
        None,
        &[1; 32],
        None,
        &[],
        false,
    )
    .await
    .expect("active channel");
    assert!(matches!(
        super::relay_admin_actions::deploy_kick_member(
            &db.pool,
            communities[0],
            channel.id,
            &[1; 32],
            &[2; 32],
        )
        .await
        .expect("active kick succeeds"),
        super::relay_admin_actions::KickResult::Removed
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_writer_requires_application_admission() {
    use nostr::{EventBuilder, Keys, Kind};
    let (db, communities) = fixture().await;
    let actor = Keys::generate();
    let target = super::personal_read::ReadTarget {
        channel_id: Uuid::new_v4(),
        root_id: None,
    };
    let intent = super::personal_read::ReadIntent::MarkThrough {
        target,
        message_id: "a".repeat(64),
    };
    for community in &communities[1..] {
        assert_fenced(
            db.apply_personal_read_intent(*community, &actor.public_key(), &intent)
                .await
                .expect_err("personal read must reject before account lock"),
        );
    }
    let channel = super::channel::create_channel(
        &db.pool,
        communities[0],
        "reads",
        super::channel::ChannelType::Stream,
        super::channel::ChannelVisibility::Open,
        None,
        &actor.public_key().to_bytes(),
        None,
        &[],
        false,
    )
    .await
    .expect("active channel");
    let event = EventBuilder::new(Kind::Custom(9), "read")
        .sign_with_keys(&Keys::generate())
        .expect("sign event");
    db.insert_event(communities[0], &event, Some(channel.id))
        .await
        .expect("insert active event");
    let intent = super::personal_read::ReadIntent::MarkThrough {
        target: super::personal_read::ReadTarget {
            channel_id: channel.id,
            root_id: None,
        },
        message_id: event.id.to_hex(),
    };
    assert_eq!(
        db.apply_personal_read_intent(communities[0], &actor.public_key(), &intent)
            .await
            .expect("active personal read succeeds"),
        super::personal_read::IntentOutcome::Applied
    );
}

async fn count_relay_members(db: &Db, community: CommunityId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM relay_members WHERE community_id = $1")
        .bind(community.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("count relay members")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn allowlist_backfill_requires_application_admission() {
    let (db, communities) = fixture().await;
    for community in &communities {
        sqlx::query("INSERT INTO pubkey_allowlist (community_id, pubkey) VALUES ($1, $2)")
            .bind(community.as_uuid())
            .bind(vec![0x5a_u8; 32])
            .execute(&db.pool)
            .await
            .expect("seed legacy allowlist row");
    }
    for community in &communities[1..] {
        assert_fenced(
            super::relay_members::backfill_from_allowlist(
                &db.pool,
                *community,
                &nostr::Keys::generate(),
            )
            .await
            .map(|write| write.outcome)
            .expect_err("backfill into a write-fenced community must reject"),
        );
        assert_eq!(count_relay_members(&db, *community).await, 0);
    }
    assert_eq!(
        super::relay_members::backfill_from_allowlist(
            &db.pool,
            communities[0],
            &nostr::Keys::generate()
        )
        .await
        .map(|write| write.outcome)
        .expect("active backfill succeeds"),
        1
    );
    assert_eq!(count_relay_members(&db, communities[0]).await, 1);

    // A community that already has members needs no write, so startup does
    // not fail on admission even while the community is quiescing.
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(communities[1].as_uuid())
        .bind("e".repeat(64))
        .execute(&db.pool)
        .await
        .expect("seed existing member");
    assert_eq!(
        super::relay_members::backfill_from_allowlist(
            &db.pool,
            communities[1],
            &nostr::Keys::generate()
        )
        .await
        .map(|write| write.outcome)
        .expect("no-op backfill does not need admission"),
        0
    );
    assert_eq!(count_relay_members(&db, communities[1]).await, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn empty_allowlist_backfill_skips_admission() {
    // No members and no legacy allowlist rows: there is nothing to write, so
    // startup must not fail on admission while the community is fenced.
    let (db, communities) = fixture().await;
    for community in &communities[1..] {
        assert_eq!(
            super::relay_members::backfill_from_allowlist(
                &db.pool,
                *community,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
            .expect("empty-allowlist backfill does not need admission"),
            0
        );
        assert_eq!(count_relay_members(&db, *community).await, 0);
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn owner_mutations_report_pending_deletion_when_admission_refuses() {
    let (db, communities) = fixture().await;
    let quiescing = communities[1];
    sqlx::query(
        "INSERT INTO community_deletion_requests (community_id, community_host, requested_by) \
         SELECT id, host, 'admission-test' FROM communities WHERE id = $1",
    )
    .bind(quiescing.as_uuid())
    .execute(&db.pool)
    .await
    .expect("seed deletion request");
    let owner = "f".repeat(64);
    let next_owner = "1".repeat(64);

    assert_eq!(
        super::relay_members::transfer_ownership(
            &db.pool,
            quiescing,
            &next_owner,
            &owner,
            &nostr::Keys::generate()
        )
        .await
        .map(|write| write.outcome)
        .expect("transfer returns a result"),
        super::relay_members::TransferResult::DeletionPending
    );
    assert_eq!(
        db.provision_owner(quiescing, &owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("provision returns a result"),
        super::relay_members::ProvisionOwnerResult::DeletionPending
    );

    // Without a deletion request the refusal stays a lifecycle conflict.
    let fenced = communities[2];
    assert_eq!(
        super::relay_members::transfer_ownership(
            &db.pool,
            fenced,
            &next_owner,
            &owner,
            &nostr::Keys::generate()
        )
        .await
        .map(|write| write.outcome)
        .expect("transfer returns a result"),
        super::relay_members::TransferResult::LifecycleConflict
    );
    assert_eq!(
        db.provision_owner(fenced, &owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("provision returns a result"),
        super::relay_members::ProvisionOwnerResult::LifecycleConflict
    );
    assert_eq!(count_relay_members(&db, quiescing).await, 0);
    assert_eq!(count_relay_members(&db, fenced).await, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invite_sweep_admits_active_tenant_and_skips_quiescing_tenant() {
    let (db, communities) = fixture().await;
    crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    let cutoff = chrono::Utc::now();
    for (index, community) in communities[..2].iter().enumerate() {
        sqlx::query(
            "INSERT INTO relay_invites (community_id, token_hash, expires_at, created_by) \
             VALUES ($1, $2, $3, 'test')",
        )
        .bind(community.as_uuid())
        .bind(vec![index as u8; 32])
        .bind(cutoff - chrono::Duration::minutes(1))
        .execute(&db.pool)
        .await
        .expect("seed invite");
    }
    assert_eq!(
        super::relay_invite::reap_expired_relay_invites(&db.pool, cutoff)
            .await
            .unwrap(),
        1
    );
    for (community, expected) in [(communities[0], 0), (communities[1], 1)] {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM relay_invites WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(count, expected, "invite state for {community}");
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_prune_admits_active_tenant_and_skips_quiescing_tenant() {
    let (db, communities) = fixture().await;
    crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    let cutoff = chrono::Utc::now();
    for community in &communities[..2] {
        sqlx::query(
            "INSERT INTO scheduled_workflow_fires \
             (community_id, workflow_id, scheduled_for, claimed_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(community.as_uuid())
        .bind(Uuid::new_v4())
        .bind(cutoff - chrono::Duration::hours(2))
        .bind(cutoff - chrono::Duration::hours(1))
        .execute(&db.pool)
        .await
        .expect("seed fire");
    }
    assert_eq!(
        super::workflow::prune_scheduled_workflow_fires_before(&db.pool, cutoff)
            .await
            .unwrap(),
        1
    );
    for (community, expected) in [(communities[0], 0), (communities[1], 1)] {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM scheduled_workflow_fires WHERE community_id = $1",
        )
        .bind(community.as_uuid())
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(count, expected, "fire state for {community}");
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_reap_admits_active_tenant_and_skips_quiescing_tenant() {
    let (db, communities) = fixture().await;
    crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    for community in &communities[..2] {
        sqlx::query(
            "INSERT INTO channels (community_id, name, created_by, ttl_seconds, ttl_deadline) \
             VALUES ($1, 'expired', $2, 60, now() - interval '1 minute')",
        )
        .bind(community.as_uuid())
        .bind(vec![1_u8; 32])
        .execute(&db.pool)
        .await
        .expect("seed channel");
    }
    let reaped = super::channel::reap_expired_ephemeral_channels(&db.pool)
        .await
        .unwrap();
    assert_eq!(reaped.len(), 1);
    assert_eq!(reaped[0].community_id, communities[0]);
    for (community, expected) in [(communities[0], true), (communities[1], false)] {
        let archived: bool = sqlx::query_scalar(
            "SELECT archived_at IS NOT NULL FROM channels WHERE community_id = $1",
        )
        .bind(community.as_uuid())
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(archived, expected, "archive state for {community}");
    }
}

/// Keep every failure-injection object, including the ALWAYS trigger's table,
/// in this call's shadow schema so a shared-database run cannot leak it.
async fn fail_tenant_mutations(pool: &sqlx::PgPool, schema: &str, table: &str, id: Uuid) {
    // Dynamic identifiers come only from our UUID schema and literal table names.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {schema}.{table} (LIKE public.{table} INCLUDING ALL)"
    )))
    .execute(pool)
    .await
    .expect("create test-local serving table");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {schema}.fail_tenant (id uuid PRIMARY KEY)"
    )))
    .execute(pool)
    .await
    .expect("create failure target");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {schema}.fail_tenant (id) VALUES ($1)"
    )))
    .bind(id)
    .execute(pool)
    .await
    .expect("select failing tenant");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {schema}.fail_tenant_mutation() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
           IF EXISTS (SELECT 1 FROM {schema}.fail_tenant \
                      WHERE id = COALESCE(NEW.community_id, OLD.community_id)) \
           THEN RAISE EXCEPTION 'tenant mutation failed'; END IF; \
           IF TG_OP = 'DELETE' THEN RETURN OLD; END IF; RETURN NEW; END $$",
    )))
    .execute(pool)
    .await
    .expect("create failure trigger function");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER fail_tenant_mutation BEFORE UPDATE OR DELETE ON {schema}.{table} \
         FOR EACH ROW EXECUTE FUNCTION {schema}.fail_tenant_mutation()",
    )))
    .execute(pool)
    .await
    .expect("create test-local failure trigger");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE {schema}.{table} ENABLE ALWAYS TRIGGER fail_tenant_mutation"
    )))
    .execute(pool)
    .await
    .expect("enable trigger with replica role");
}

async fn two_active_tenants(db: &Db, first: CommunityId) -> [Uuid; 2] {
    let extra = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(extra)
        .bind(format!("admission-extra-{extra}.example"))
        .execute(&db.pool)
        .await
        .expect("seed second active tenant");
    let mut ids = [*first.as_uuid(), extra];
    ids.sort();
    ids
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_reap_returns_committed_rows_after_earlier_tenant_error() {
    let (db, communities) = fixture().await;
    let schema = crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    let ids = two_active_tenants(&db, communities[0]).await;
    fail_tenant_mutations(&db.pool, &schema, "channels", ids[0]).await;
    for id in ids {
        sqlx::query(
            "INSERT INTO channels (community_id, name, created_by, ttl_seconds, ttl_deadline) \
             VALUES ($1, 'expired', $2, 60, now() - interval '1 minute')",
        )
        .bind(id)
        .bind(vec![1_u8; 32])
        .execute(&db.pool)
        .await
        .expect("seed expired channel");
    }
    let reaped = super::channel::reap_expired_ephemeral_channels(&db.pool)
        .await
        .expect("tenant failure must not stop the fleet sweep");
    assert_eq!(reaped.len(), 1);
    assert_eq!(*reaped[0].community_id.as_uuid(), ids[1]);
    for (id, expected) in [(ids[0], false), (ids[1], true)] {
        let archived: bool = sqlx::query_scalar(
            "SELECT archived_at IS NOT NULL FROM channels WHERE community_id = $1",
        )
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("read archive state");
        assert_eq!(archived, expected, "archive state for {id}");
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM {schema}.fail_tenant"
    )))
    .execute(&db.pool)
    .await
    .unwrap();
    let retried = super::channel::reap_expired_ephemeral_channels(&db.pool)
        .await
        .unwrap();
    assert_eq!(retried.len(), 1);
    assert_eq!(*retried[0].community_id.as_uuid(), ids[0]);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn fleet_sweep_returns_committed_results_when_database_drops_mid_page() {
    let (db, communities) = fixture().await;
    let [first, second] = two_active_tenants(&db, communities[0]).await;
    let candidates = [first, second, Uuid::new_v4()];
    let pulled = std::cell::Cell::new(0);
    let pool = db.pool.clone();
    let committed = crate::sweep_admitted_communities(
        &db.pool,
        candidates
            .iter()
            .copied()
            .inspect(|_| pulled.set(pulled.get() + 1)),
        crate::observability::WriterOperation::Maintenance,
        "test_sweep",
        |id, tx| {
            let pool = pool.clone();
            async move {
                tx.commit().await?;
                pool.close().await;
                Ok(id)
            }
        },
    )
    .await;
    assert_eq!(committed, vec![first], "rows committed before the outage");
    assert_eq!(
        pulled.get(),
        2,
        "the sweep must stop at the unavailable tenant"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn match_claim_admits_active_tenant_and_skips_quiescing_tenant() {
    let (db, communities) = fixture().await;
    crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    for (index, community) in communities[..2].iter().enumerate() {
        sqlx::query(
            "INSERT INTO push_match_queue (community_id, event_id, next_attempt_at) \
             VALUES ($1, $2, now() - make_interval(mins => $3))",
        )
        .bind(community.as_uuid())
        .bind(vec![index as u8; 32])
        .bind(if index == 0 { 1 } else { 2 })
        .execute(&db.pool)
        .await
        .expect("seed match job");
    }
    // The production claim is exercised; absent source events are its normal
    // terminal cleanup case, so it returns None after claiming and pruning.
    assert!(super::push::claim_due_match_batch(
        &db.pool,
        8,
        chrono::Utc::now() + chrono::Duration::minutes(1)
    )
    .await
    .unwrap()
    .is_none());
    for (community, expected) in [(communities[0], 0), (communities[1], 1)] {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_match_queue WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(count, expected, "match job for {community}");
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn match_reap_admits_active_tenant_and_skips_quiescing_tenant() {
    let (db, communities) = fixture().await;
    crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    for (index, community) in communities[..2].iter().enumerate() {
        sqlx::query(
            "INSERT INTO push_match_queue (community_id, event_id, attempts) VALUES ($1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(vec![index as u8; 32])
        .bind(super::push::MAX_MATCH_ATTEMPTS)
        .execute(&db.pool)
        .await
        .expect("seed exhausted match job");
    }
    assert_eq!(
        super::push::reap_exhausted_matches(&db.pool).await.unwrap(),
        1
    );
    for (community, expected) in [(communities[0], 0), (communities[1], 1)] {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_match_queue WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(count, expected, "exhausted job for {community}");
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invite_sweep_rechecks_admission_after_candidate_read() {
    let (db, communities) = fixture().await;
    crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    let community = communities[0];
    let cutoff = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO relay_invites (community_id, token_hash, expires_at, created_by) \
         VALUES ($1, $2, $3, 'test')",
    )
    .bind(community.as_uuid())
    .bind(vec![0_u8; 32])
    .bind(cutoff - chrono::Duration::minutes(1))
    .execute(&db.pool)
    .await
    .unwrap();
    let pool = db.pool.clone();
    let reaped = crate::test_support::fence_while_sweep_waits(community, async move {
        super::relay_invite::reap_expired_relay_invites(&pool, cutoff).await
    })
    .await
    .unwrap();
    assert_eq!(reaped, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_invites WHERE community_id = $1")
            .bind(community.as_uuid())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "newly fenced tenant's invite must remain");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_reap_rechecks_admission_after_candidate_read() {
    let (db, communities) = fixture().await;
    crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    let community = communities[0];
    sqlx::query(
        "INSERT INTO channels (community_id, name, created_by, ttl_seconds, ttl_deadline) \
         VALUES ($1, 'expired', $2, 60, now() - interval '1 minute')",
    )
    .bind(community.as_uuid())
    .bind(vec![1_u8; 32])
    .execute(&db.pool)
    .await
    .unwrap();
    let pool = db.pool.clone();
    let reaped = crate::test_support::fence_while_sweep_waits(community, async move {
        super::channel::reap_expired_ephemeral_channels(&pool).await
    })
    .await
    .unwrap();
    assert!(reaped.is_empty());
    let archived: bool =
        sqlx::query_scalar("SELECT archived_at IS NOT NULL FROM channels WHERE community_id = $1")
            .bind(community.as_uuid())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(
        !archived,
        "newly fenced tenant's channel must remain unarchived"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn match_claim_rechecks_admission_and_continues_after_candidate_read() {
    let (db, communities) = fixture().await;
    crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
    let ids = two_active_tenants(&db, communities[0]).await;
    for (index, id) in ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO push_match_queue (community_id, event_id, next_attempt_at) \
             VALUES ($1, $2, now() - make_interval(mins => $3))",
        )
        .bind(id)
        .bind(vec![index as u8; 32])
        .bind(if index == 0 { 2 } else { 1 })
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let pool = db.pool.clone();
    // Missing source events take the production claim's terminal cleanup path.
    crate::test_support::fence_while_sweep_waits(CommunityId::from_uuid(ids[0]), async move {
        super::push::claim_due_match_batch(
            &pool,
            8,
            chrono::Utc::now() + chrono::Duration::minutes(1),
        )
        .await
    })
    .await
    .unwrap();
    for (id, expected) in [(ids[0], 1), (ids[1], 0)] {
        let state: Vec<(String, i32)> =
            sqlx::query_as("SELECT state, attempts FROM push_match_queue WHERE community_id = $1")
                .bind(id)
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(state.len(), expected, "queue state for {id}");
        if expected == 1 {
            assert_eq!(state[0], ("pending".to_owned(), 0));
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn periodic_reaps_continue_after_tenant_error_and_retry_retained_rows() {
    for table in [
        "relay_invites",
        "scheduled_workflow_fires",
        "push_match_queue",
    ] {
        let (db, communities) = fixture().await;
        let schema = crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
        let ids = two_active_tenants(&db, communities[0]).await;
        fail_tenant_mutations(&db.pool, &schema, table, ids[0]).await;
        let cutoff = chrono::Utc::now();
        for id in ids {
            match table {
                "relay_invites" => {
                    sqlx::query(
                        "INSERT INTO relay_invites (community_id, token_hash, expires_at, created_by) \
                         VALUES ($1, $2, $3, 'test')",
                    )
                    .bind(id).bind(vec![1_u8; 32])
                    .bind(cutoff - chrono::Duration::minutes(if id == ids[0] { 2 } else { 1 }))
                    .execute(&db.pool).await.unwrap();
                }
                "scheduled_workflow_fires" => {
                    sqlx::query(
                        "INSERT INTO scheduled_workflow_fires \
                         (community_id, workflow_id, scheduled_for, claimed_at) \
                         VALUES ($1, $2, $3, $3)",
                    )
                    .bind(id)
                    .bind(Uuid::new_v4())
                    .bind(cutoff - chrono::Duration::hours(1))
                    .execute(&db.pool)
                    .await
                    .unwrap();
                }
                "push_match_queue" => {
                    sqlx::query(
                        "INSERT INTO push_match_queue (community_id, event_id, attempts) \
                         VALUES ($1, $2, $3)",
                    )
                    .bind(id)
                    .bind(vec![1_u8; 32])
                    .bind(super::push::MAX_MATCH_ATTEMPTS)
                    .execute(&db.pool)
                    .await
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        for retry in [false, true] {
            if retry {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "DELETE FROM {schema}.fail_tenant"
                )))
                .execute(&db.pool)
                .await
                .unwrap();
            }
            let count = match table {
                "relay_invites" => {
                    super::relay_invite::reap_expired_relay_invites(&db.pool, cutoff).await
                }
                "scheduled_workflow_fires" => {
                    super::workflow::prune_scheduled_workflow_fires_before(&db.pool, cutoff).await
                }
                "push_match_queue" => super::push::reap_exhausted_matches(&db.pool).await,
                _ => unreachable!(),
            }
            .expect("per-tenant failures must not stop recurring sweeps");
            assert_eq!(count, 1, "{table}, retry={retry}");
            for (id, expected) in [(ids[0], i64::from(!retry)), (ids[1], 0)] {
                let remaining: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} WHERE community_id = $1",
                )))
                .bind(id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
                assert_eq!(remaining, expected, "{table} state for {id}, retry={retry}");
            }
        }
    }
}
