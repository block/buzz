//! Live writer-admission regressions. A one-connection pool runs with ordinary
//! PostgreSQL triggers disabled, so a missing application check is observable.

use crate::{Db, DbConfig, DbError};
use buzz_core::CommunityId;
use uuid::Uuid;

async fn fixture() -> (Db, [CommunityId; 3]) {
    let config = DbConfig {
        database_url: crate::test_support::database_url(),
        max_connections: 1,
        min_connections: 1,
        ..DbConfig::default()
    };
    let db = Db::new(&config).await.expect("connect test database");
    let pool = &db.pool;
    let mut communities = [CommunityId::from_uuid(Uuid::nil()); 3];
    for community in &mut communities {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("admission-{id}.example"))
            .execute(pool)
            .await
            .expect("create community");
        *community = CommunityId::from_uuid(id);
    }
    crate::test_support::set_deletion_state(pool, *communities[1].as_uuid(), "quiescing").await;
    crate::test_support::set_deletion_state(pool, *communities[2].as_uuid(), "fenced").await;

    // One physical session serves every test API call. PostgreSQL's replica
    // role skips the 32 community-write triggers as well as other ordinary
    // triggers; the application precheck remains fully active.
    sqlx::query("SET session_replication_role = replica")
        .execute(pool)
        .await
        .expect("disable ordinary triggers on the test session");
    let role: String = sqlx::query_scalar("SHOW session_replication_role")
        .fetch_one(pool)
        .await
        .expect("read replication role");
    assert_eq!(role, "replica");
    // Witness that the database backstop is truly bypassed: this raw fenced
    // insert would be rejected with the community-write trigger enabled.
    sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2)")
        .bind(communities[2].as_uuid())
        .bind(vec![0xfe_u8; 32])
        .execute(pool)
        .await
        .expect("raw fenced insert succeeds with triggers disabled");
    (db, communities)
}

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
            super::relay_members::add_relay_member(&db.pool, *community, &pubkey, "member", None)
                .await
                .expect_err("member add must reject"),
        );
        assert_fenced(
            super::relay_members::remove_relay_member(&db.pool, *community, &pubkey)
                .await
                .expect_err("member remove must reject"),
        );
    }
    assert!(super::relay_members::add_relay_member(
        &db.pool,
        communities[0],
        &pubkey,
        "member",
        None,
    )
    .await
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
            )
            .await
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
