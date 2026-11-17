//! Storage-boundary journeys: CAS, durable retry, atomic failure and isolation.

use super::mention_group::GroupOutcome;
use crate::Db;
use buzz_core::mention_group::{parse_command, parse_policy_command};
use buzz_core::{CommunityId, StoredEvent};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::{PgPool, Row};
use uuid::Uuid;

struct Fixture {
    db: Db,
    community: CommunityId,
    actor: Keys,
    relay: Keys,
}

impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            crate::migration::run_migrations(&pool).await.unwrap();
        }
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let actor = Keys::generate();
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!(
                "mention-groups-{}.example.org",
                community.as_uuid()
            ))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'owner')")
            .bind(community.as_uuid())
            .bind(actor.public_key().to_hex())
            .execute(&pool)
            .await
            .unwrap();
        Self {
            db: Db::from_pool(pool),
            community,
            actor,
            relay: Keys::generate(),
        }
    }

    fn put(&self, id: Uuid, prev: i64, name: &str, members: &[String]) -> Event {
        let mut tags = vec![
            vec!["d".into(), id.to_string()],
            vec!["prev".into(), prev.to_string()],
            vec!["name".into(), name.into()],
        ];
        tags.extend(
            members
                .iter()
                .map(|member| vec!["p".into(), member.clone()]),
        );
        self.event(buzz_core::kind::RELAY_ADMIN_PUT_MENTION_GROUP, tags)
    }

    fn delete(&self, id: Uuid, prev: i64) -> Event {
        self.event(
            buzz_core::kind::RELAY_ADMIN_DELETE_MENTION_GROUP,
            vec![
                vec!["d".into(), id.to_string()],
                vec!["prev".into(), prev.to_string()],
            ],
        )
    }

    fn event(&self, kind: u32, tags: Vec<Vec<String>>) -> Event {
        EventBuilder::new(Kind::Custom(kind as u16), "")
            .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
            .allow_self_tagging()
            .sign_with_keys(&self.actor)
            .unwrap()
    }

    async fn apply(&self, event: &Event) -> crate::Result<super::mention_group::GroupWrite> {
        self.db
            .accept_mention_group(
                self.community,
                event,
                &parse_command(event).unwrap(),
                &self.relay,
                1_000_000,
                true,
            )
            .await
    }

    async fn state(&self, id: Uuid) -> (i64, bool, Vec<u8>) {
        let row = sqlx::query("SELECT version,deleted_at IS NOT NULL AS deleted,state_event_id FROM mention_groups WHERE community_id=$1 AND id=$2")
            .bind(self.community.as_uuid()).bind(id).fetch_one(self.db.pool()).await.unwrap();
        (
            row.get("version"),
            row.get("deleted"),
            row.get("state_event_id"),
        )
    }

    async fn live_heads(&self, id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND kind=39010 AND d_tag=$2 AND deleted_at IS NULL")
            .bind(self.community.as_uuid()).bind(id.to_string()).fetch_one(self.db.pool()).await.unwrap()
    }
}

fn applied(write: super::mention_group::GroupWrite) -> StoredEvent {
    assert_eq!(write.outcome, GroupOutcome::Applied);
    let snapshot = write.snapshot.unwrap();
    snapshot.event.verify().unwrap();
    assert!(snapshot.channel_id.is_none());
    snapshot
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mention_group_lifecycle_keeps_receipts_and_live_tombstones() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4();
    let mut members: Vec<String> = (0..19)
        .map(|_| Keys::generate().public_key().to_hex())
        .collect();
    members.push(f.relay.public_key().to_hex());
    members.push(members[0].clone());
    let create = f.put(id, 0, "  On call  ", &members);
    let first = applied(f.apply(&create).await.unwrap());
    let row =
        sqlx::query("SELECT name,members FROM mention_groups WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(id)
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(row.get::<String, _>("name"), "On call");
    let stored: Vec<String> = row.get("members");
    members.sort();
    members.dedup();
    assert_eq!(stored, members);
    let published: Vec<String> = first
        .event
        .tags
        .iter()
        .filter_map(|tag| {
            let fields = tag.as_slice();
            (fields.first().map(String::as_str) == Some("p")).then(|| fields[1].clone())
        })
        .collect();
    assert_eq!(
        published, members,
        "relay-key membership must survive signing"
    );
    let edit = f.put(id, 1, "Night shift", &[]);
    let second = applied(f.apply(&edit).await.unwrap());
    assert!(second.event.created_at > first.event.created_at);
    assert_eq!(
        f.apply(&create).await.unwrap().outcome,
        GroupOutcome::Duplicate
    );
    assert_eq!(
        f.apply(&f.delete(id, 1)).await.unwrap().outcome,
        GroupOutcome::Conflict
    );
    assert_eq!(f.state(id).await.0, 2);
    let tombstone = applied(f.apply(&f.delete(id, 2)).await.unwrap());
    assert!(tombstone.event.created_at > second.event.created_at);
    assert!(tombstone
        .event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["deleted", "true"]));
    assert!(!tombstone
        .event
        .tags
        .iter()
        .any(|tag| matches!(tag.as_slice()[0].as_str(), "name" | "p")));
    assert_eq!(
        f.state(id).await,
        (3, true, tombstone.event.id.to_bytes().to_vec())
    );
    assert_eq!(f.live_heads(id).await, 1);
    assert_eq!(
        f.apply(&f.put(id, 3, "Revive", &[])).await.unwrap().outcome,
        GroupOutcome::Conflict
    );
    assert_eq!(
        f.apply(&edit).await.unwrap().outcome,
        GroupOutcome::Duplicate
    );
    applied(
        f.apply(&f.put(Uuid::new_v4(), 0, "NIGHT SHIFT", &[]))
            .await
            .unwrap(),
    );
    assert!(f
        .db
        .mention_group_command_accepted(f.community, create.id.as_bytes())
        .await
        .unwrap());
    let old_mentions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_mentions WHERE community_id=$1 AND event_id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(first.event.id.as_bytes().as_slice())
    .fetch_one(f.db.pool())
    .await
    .unwrap();
    assert_eq!(
        old_mentions, 0,
        "retirement removes stale mention index entries"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mention_group_concurrent_cas_names_and_identical_retries_commit_once() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4();
    let a = f.put(id, 0, "A", &[]);
    let b = f.put(id, 0, "B", &[]);
    let (a, b) = tokio::join!(f.apply(&a), f.apply(&b));
    let outcomes = [a.unwrap().outcome, b.unwrap().outcome];
    assert_eq!(
        outcomes
            .iter()
            .filter(|v| **v == GroupOutcome::Applied)
            .count(),
        1
    );
    assert!(outcomes.contains(&GroupOutcome::Conflict));
    let a = f.put(id, 1, "C", &[]);
    let b = f.put(id, 1, "D", &[]);
    let (a, b) = tokio::join!(f.apply(&a), f.apply(&b));
    let outcomes = [a.unwrap().outcome, b.unwrap().outcome];
    assert_eq!(
        outcomes
            .iter()
            .filter(|v| **v == GroupOutcome::Applied)
            .count(),
        1
    );
    assert!(outcomes.contains(&GroupOutcome::Conflict));
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let a = f.put(first_id, 0, "Shared", &[]);
    let b = f.put(second_id, 0, "SHARED", &[]);
    let (a, b) = tokio::join!(f.apply(&a), f.apply(&b));
    let outcomes = [a.unwrap().outcome, b.unwrap().outcome];
    assert_eq!(
        outcomes
            .iter()
            .filter(|v| **v == GroupOutcome::Applied)
            .count(),
        1
    );
    assert!(outcomes.contains(&GroupOutcome::NameTaken));
    let before = f.state(id).await;
    assert_eq!(
        f.apply(&f.put(id, 2, "shared", &[])).await.unwrap().outcome,
        GroupOutcome::NameTaken
    );
    assert_eq!(f.state(id).await, before, "rename collision must roll back");
    let retry = f.put(id, 2, "Final", &[]);
    let (a, b) = tokio::join!(f.apply(&retry), f.apply(&retry));
    let outcomes = [a.unwrap().outcome, b.unwrap().outcome];
    assert!(outcomes.contains(&GroupOutcome::Applied));
    assert!(outcomes.contains(&GroupOutcome::Duplicate));
    assert_eq!(f.state(id).await.0, 3);
    assert_eq!(f.live_heads(id).await, 1);
}

// A settings writer already holding a channel lock must be able to finish
// its community invalidation while a group put waits for that channel.
async fn assert_channel_writer_precedes_group_put(updates: crate::channel::ChannelUpdate) {
    use buzz_core::channel::{ChannelType, ChannelVisibility};
    use std::time::Duration;

    let f = Fixture::new().await;
    let channel =
        f.db.create_channel(
            f.community,
            "Lock order",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &f.actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let id = Uuid::new_v4();
    let event = f.event(
        buzz_core::kind::RELAY_ADMIN_PUT_MENTION_GROUP,
        vec![
            vec!["d".into(), id.to_string()],
            vec!["prev".into(), "0".into()],
            vec!["name".into(), "Ordered".into()],
            vec!["p".into(), f.actor.public_key().to_hex()],
            vec!["channel".into(), channel.to_string()],
        ],
    );
    // One connection identifies the real acceptance backend without a
    // test-only production seam or matching unrelated cluster sessions.
    let put_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let put_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&put_pool)
        .await
        .unwrap();
    let mut holder =
        f.db.begin_event_write_transaction(f.community)
            .await
            .unwrap();
    sqlx::query("SET LOCAL statement_timeout='2s'")
        .execute(holder.conn())
        .await
        .unwrap();
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(holder.conn())
        .await
        .unwrap();
    let mut writer =
        crate::channel::lock_channel_for_settings_write(&mut holder, channel, &updates)
            .await
            .unwrap();
    let community = f.community;
    let relay = f.relay.clone();
    let put = tokio::spawn(async move {
        Db::from_pool(put_pool)
            .accept_mention_group(
                community,
                &event,
                &parse_command(&event).unwrap(),
                &relay,
                1_000_000,
                true,
            )
            .await
    });
    // Observe PostgreSQL's actual lock dependency, with a bounded deadline.
    // No scheduling delay can stand in for proof that acceptance is blocked.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid=$1 \
                 AND wait_event_type='Lock' AND $2=ANY(pg_blocking_pids(pid)))",
            )
            .bind(put_pid)
            .bind(holder_pid)
            .fetch_one(f.db.pool())
            .await
            .unwrap();
            if blocked {
                break;
            }
            assert!(
                !put.is_finished(),
                "put finished without waiting for channel writer"
            );
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("put never waited for the production channel lock");
    let invalidated: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE communities SET artifact_policy_modified_at=clock_timestamp() \
         WHERE id=$1 RETURNING artifact_policy_modified_at",
    )
    .bind(community.as_uuid())
    .fetch_one(writer.tx().conn())
    .await
    .expect("channel writer must update community while group put waits");
    holder.commit().await.unwrap();
    let accepted = tokio::time::timeout(Duration::from_secs(6), put)
        .await
        .unwrap()
        .unwrap()
        .expect("group put must also commit; neither writer may abort");
    assert_eq!(accepted.outcome, GroupOutcome::Applied);
    assert_eq!(accepted.joins.skipped.get("already_member"), Some(&1));
    assert_eq!(f.state(id).await.0, 1);
    assert_eq!(f.live_heads(id).await, 1);
    let actual: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT artifact_policy_modified_at FROM communities WHERE id=$1")
            .bind(community.as_uuid())
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(actual, invalidated);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cluster_global_mention_group_put_waits_before_community_for_membership_writer() {
    assert_channel_writer_precedes_group_put(crate::channel::ChannelUpdate {
        visibility: Some("private".into()),
        ..Default::default()
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cluster_global_mention_group_put_waits_before_community_for_exclusive_ttl_writer() {
    assert_channel_writer_precedes_group_put(crate::channel::ChannelUpdate {
        ttl_seconds: Some(Some(3600)),
        ..Default::default()
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cluster_global_mention_group_put_waits_before_community_for_row_writer() {
    assert_channel_writer_precedes_group_put(crate::channel::ChannelUpdate {
        unlisted: Some(true),
        ..Default::default()
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mention_group_failures_roll_back_rows_receipts_and_retirement() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4();
    let create = f.put(id, 0, "Stable", &[]);
    let command = parse_command(&create).unwrap();
    assert!(f
        .db
        .accept_mention_group(f.community, &create, &command, &f.relay, 1, true)
        .await
        .is_err());
    assert_eq!(f.live_heads(id).await, 0);
    assert!(!f
        .db
        .mention_group_command_accepted(f.community, create.id.as_bytes())
        .await
        .unwrap());
    applied(f.apply(&create).await.unwrap());
    let before = f.state(id).await;
    // A real database boundary failure after group/head writes must roll back
    // canonical data, the previous-head retirement and the acceptance receipt.
    sqlx::query("ALTER TABLE mention_group_commands ADD CONSTRAINT reject_second_revision CHECK (version < 2)")
        .execute(f.db.pool()).await.unwrap();
    let update = f.put(id, 1, "Uncommitted", &[]);
    assert!(f.apply(&update).await.is_err());
    assert_eq!(f.state(id).await, before);
    assert_eq!(f.live_heads(id).await, 1);
    assert!(!f
        .db
        .mention_group_command_accepted(f.community, update.id.as_bytes())
        .await
        .unwrap());
    sqlx::query("ALTER TABLE mention_group_commands DROP CONSTRAINT reject_second_revision")
        .execute(f.db.pool())
        .await
        .unwrap();
    applied(f.apply(&update).await.unwrap());
    // Deferred uniqueness reaches COMMIT rather than the earlier INSERT.
    sqlx::query("ALTER TABLE mention_group_commands ADD CONSTRAINT commit_failure UNIQUE (community_id,operation,version) DEFERRABLE INITIALLY DEFERRED")
        .execute(f.db.pool()).await.unwrap();
    let other = Uuid::new_v4();
    let other_create = f.put(other, 0, "Commit failure", &[]);
    assert!(f.apply(&other_create).await.is_err());
    assert_eq!(f.live_heads(other).await, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mention_groups WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(other)
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(count, 0);
    assert!(!f
        .db
        .mention_group_command_accepted(f.community, other_create.id.as_bytes())
        .await
        .unwrap());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mention_group_policy_cas_uses_settings_coordinate_and_shared_receipts() {
    let f = Fixture::new().await;
    let defaults: (String,i64,Option<Vec<u8>>) = sqlx::query_as("SELECT mention_groups_manage,mention_groups_version,mention_groups_state_event_id FROM communities WHERE id=$1")
        .bind(f.community.as_uuid()).fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(defaults, ("everyone".into(), 0, None));
    let event = f.event(
        buzz_core::kind::RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
        vec![
            vec!["manage".into(), "admins".into()],
            vec!["prev".into(), "0".into()],
        ],
    );
    let command = parse_policy_command(&event).unwrap();
    let state = applied(
        f.db.accept_mention_group_policy(f.community, &event, &command, &f.relay, 1_000_000, true)
            .await
            .unwrap(),
    );
    assert!(state
        .event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["d", "settings"]));
    assert_eq!(
        f.db.accept_mention_group_policy(f.community, &event, &command, &f.relay, 1_000_000, true)
            .await
            .unwrap()
            .outcome,
        GroupOutcome::Duplicate
    );
    let stale = f.event(
        buzz_core::kind::RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
        vec![
            vec!["manage".into(), "everyone".into()],
            vec!["prev".into(), "0".into()],
        ],
    );
    assert_eq!(
        f.db.accept_mention_group_policy(
            f.community,
            &stale,
            &parse_policy_command(&stale).unwrap(),
            &f.relay,
            1_000_000,
            true,
        )
        .await
        .unwrap()
        .outcome,
        GroupOutcome::Conflict
    );
    let groups: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mention_groups WHERE community_id=$1")
            .bind(f.community.as_uuid())
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(groups, 0, "settings must not create a fake group");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mention_group_default_joins_are_atomic_report_skips_and_never_replay() {
    use buzz_core::channel::{ChannelType, ChannelVisibility};
    let f = Fixture::new().await;
    let public =
        f.db.create_channel(
            f.community,
            "Public",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &f.actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let archived =
        f.db.create_channel(
            f.community,
            "Archived",
            ChannelType::Forum,
            ChannelVisibility::Open,
            None,
            &f.actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    f.db.archive_channel(f.community, archived).await.unwrap();
    let private =
        f.db.create_channel(
            f.community,
            "Private",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            &f.actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let gone = Uuid::new_v4();
    let target = Keys::generate();
    let blocked = Keys::generate();
    let removed = Keys::generate();
    let agent = Keys::generate();
    for key in [&target, &blocked, &removed] {
        let _ =
            f.db.add_relay_member(
                f.community,
                &key.public_key().to_hex(),
                "member",
                None,
                &f.relay,
            )
            .await
            .unwrap();
    }
    let _ =
        f.db.remove_relay_member(f.community, &removed.public_key().to_hex(), &f.relay)
            .await
            .unwrap();
    f.db.ensure_user(f.community, &f.actor.public_key().to_bytes())
        .await
        .unwrap();
    f.db.ensure_user(f.community, &agent.public_key().to_bytes())
        .await
        .unwrap();
    f.db.set_agent_owner(
        f.community,
        &agent.public_key().to_bytes(),
        &f.actor.public_key().to_bytes(),
    )
    .await
    .unwrap();
    f.db.ensure_user(f.community, &blocked.public_key().to_bytes())
        .await
        .unwrap();
    f.db.set_channel_add_policy(f.community, &blocked.public_key().to_bytes(), "nobody")
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let event = |prev: i64, name: &str, members: &[String], channels: &[Uuid]| {
        let mut tags = vec![
            vec!["d".into(), id.to_string()],
            vec!["prev".into(), prev.to_string()],
            vec!["name".into(), name.into()],
        ];
        tags.extend(members.iter().map(|key| vec!["p".into(), key.clone()]));
        tags.extend(
            channels
                .iter()
                .map(|channel| vec!["channel".into(), channel.to_string()]),
        );
        f.event(buzz_core::kind::RELAY_ADMIN_PUT_MENTION_GROUP, tags)
    };
    let members = vec![
        target.public_key().to_hex(),
        blocked.public_key().to_hex(),
        f.actor.public_key().to_hex(),
        removed.public_key().to_hex(),
        agent.public_key().to_hex(),
    ];
    // A present private channel invalidates the configuration and commits no
    // earlier join, even when a valid channel appears in the same command.
    let invalid = event(0, "Invalid", &members, &[public, private]);
    assert!(f.apply(&invalid).await.is_err());
    assert!(f
        .db
        .get_member_role(f.community, public, &target.public_key().to_bytes())
        .await
        .unwrap()
        .is_none());
    assert!(!f
        .db
        .mention_group_command_accepted(f.community, invalid.id.as_bytes())
        .await
        .unwrap());
    let create = event(0, "Configured", &members, &[public, archived, gone]);
    let accepted = f.apply(&create).await.unwrap();
    assert_eq!(accepted.outcome, GroupOutcome::Applied);
    assert_eq!(accepted.joins.joined, 2);
    assert!(
        accepted
            .joined_members
            .contains(&(public, agent.public_key().to_bytes().to_vec())),
        "an agent is admitted through its current owner without its own roster row"
    );
    assert!(f
        .db
        .get_member_role(f.community, public, &removed.public_key().to_bytes())
        .await
        .unwrap()
        .is_none());
    assert_eq!(accepted.joins.skipped.get("target_not_admitted"), Some(&1));
    assert_eq!(accepted.joined_channels, vec![public]);
    assert_eq!(accepted.joins.skipped.get("target_policy_nobody"), Some(&1));
    assert_eq!(accepted.joins.skipped.get("already_member"), Some(&1));
    assert_eq!(accepted.joins.skipped.get("channel_closed"), Some(&5));
    assert_eq!(accepted.joins.skipped.get("channel_unavailable"), Some(&5));
    assert_eq!(
        f.db.get_member_role(f.community, public, &f.actor.public_key().to_bytes())
            .await
            .unwrap()
            .as_deref(),
        Some("owner")
    );
    f.db.remove_member(
        f.community,
        public,
        &target.public_key().to_bytes(),
        &f.actor.public_key().to_bytes(),
    )
    .await
    .unwrap();
    let replay = f.apply(&create).await.unwrap();
    assert_eq!(replay.outcome, GroupOutcome::Duplicate);
    assert_eq!(
        replay.joins, accepted.joins,
        "retry retains original skip report"
    );
    assert!(replay.joined_channels.is_empty());
    assert!(
        f.db.get_member_role(f.community, public, &target.public_key().to_bytes())
            .await
            .unwrap()
            .is_none(),
        "retry cannot undo a later leave"
    );
    // Changing only group metadata/defaults does not re-add unchanged members.
    let edit = event(1, "Renamed", &members, &[public, gone]);
    let edited = f.apply(&edit).await.unwrap();
    assert_eq!(edited.outcome, GroupOutcome::Applied);
    assert_eq!(edited.joins.joined, 0);
    assert!(f
        .db
        .get_member_role(f.community, public, &target.public_key().to_bytes())
        .await
        .unwrap()
        .is_none());
    let retained: Vec<Uuid> =
        sqlx::query_scalar("SELECT channels FROM mention_groups WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(id)
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert!(
        retained.contains(&gone),
        "gone defaults remain inert saved references"
    );
    let outsider = Keys::generate();
    let mut open_members = members.clone();
    open_members.push(outsider.public_key().to_hex());
    let open = event(2, "Open mode", &open_members, &[public]);
    let open_result =
        f.db.accept_mention_group(
            f.community,
            &open,
            &parse_command(&open).unwrap(),
            &f.relay,
            1_000_000,
            false,
        )
        .await
        .unwrap();
    assert_eq!(open_result.outcome, GroupOutcome::Applied);
    assert_eq!(
        open_result.joins.joined, 1,
        "open mode permits authenticated outside recipients"
    );
    assert!(f
        .db
        .get_member_role(f.community, public, &outsider.public_key().to_bytes())
        .await
        .unwrap()
        .is_some());

    // A saved default can become nonpublic. It must not prevent a rename or
    // member removal, and re-adding a group member must not grant access.
    let without_target: Vec<_> = open_members
        .iter()
        .filter(|key| **key != target.public_key().to_hex())
        .cloned()
        .collect();
    let mut version = 3;
    for (visibility, unlisted) in [("open", true), ("private", false)] {
        f.db.update_channel(
            f.community,
            public,
            crate::channel::ChannelUpdate {
                visibility: Some(visibility.into()),
                unlisted: Some(unlisted),
                ..Default::default()
            },
            &f.actor.public_key().to_bytes(),
        )
        .await
        .unwrap();
        let replay = f.apply(&create).await.unwrap();
        assert_eq!(replay.outcome, GroupOutcome::Duplicate);
        assert_eq!(replay.joins, accepted.joins);
        let rename = event(version, "Still editable", &without_target, &[public]);
        let renamed = f.apply(&rename).await.unwrap();
        assert_eq!(renamed.outcome, GroupOutcome::Applied);
        assert_eq!(renamed.joins.joined, 0);
        version += 1;
        let add = event(version, "Still editable", &open_members, &[public]);
        let added = f.apply(&add).await.unwrap();
        assert_eq!(added.outcome, GroupOutcome::Applied);
        assert_eq!(added.joins.skipped.get("channel_closed"), Some(&1));
        assert_eq!(added.joins.joined, 0);
        assert!(f
            .db
            .get_member_role(f.community, public, &target.public_key().to_bytes())
            .await
            .unwrap()
            .is_none());
        version += 1;
    }
    f.db.soft_delete_channel(f.community, public).await.unwrap();
    let replay = f.apply(&create).await.unwrap();
    assert_eq!(replay.outcome, GroupOutcome::Duplicate);
    assert_eq!(replay.joins, accepted.joins);
    // Even a physically absent default must not turn an accepted receipt
    // into a new eligibility decision in the prelock phase.
    sqlx::query("DELETE FROM channels WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(public)
        .execute(f.db.pool())
        .await
        .unwrap();
    let replay = f.apply(&create).await.unwrap();
    assert_eq!(replay.outcome, GroupOutcome::Duplicate);
    assert_eq!(replay.joins, accepted.joins);
    assert_eq!(f.state(id).await.0, version);
    assert_eq!(f.live_heads(id).await, 1);
}
