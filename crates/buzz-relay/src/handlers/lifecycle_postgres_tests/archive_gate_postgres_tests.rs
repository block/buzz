//! Postgres-backed tests for the per-channel archive gate (NEXUS-4111,
//! slice 2).
//!
//! A write that an archived channel refuses holds the channel's gate
//! shared and reads the archive state under it. Every archive writer holds
//! the gate exclusive. Races are ordered by a second session that holds the
//! gate, never by sleeps: the test takes the gate, starts the contender,
//! waits until `pg_locks` shows it queued on that exact key, then commits.

use std::time::Duration;

use axum::http::StatusCode;
use buzz_core::channel::MemberRole;
use buzz_db::workflow::{ApprovalStatus, RunStatus};
use buzz_db::{ArchiveGate, DbError};
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{
    admin_event, advisory, conflict, edit, invalid, lock_row, stamp, task_record, Fixture, Reply,
    Subtree,
};

const GATE: &str = "buzz_channel_gate";
const MEMBERSHIP: &str = "buzz_channel_membership";
const ARCHIVED: &str = "invalid: channel is archived";
const DELETION_DENIED: &str = "invalid: deletion target not found or not deletable by you";
const WAIT: Duration = Duration::from_secs(10);

/// An event with exactly `tags`: the builder keeps a `p` tag of the signer.
fn event(keys: &Keys, kind: u16, content: &str, tags: &[&[&str]]) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()))
        .allow_self_tagging()
        .custom_created_at(stamp())
        .sign_with_keys(keys)
        .unwrap()
}

fn message(keys: &Keys, channel: Uuid, content: &str) -> Event {
    event(keys, 9, content, &[&["h", &channel.to_string()]])
}

/// A `buzz.task` artifact revision: an artifact that is not a task.
fn artifact(keys: &Keys, home: Uuid, id: Uuid, op: &str, prev: Option<&Event>) -> Event {
    let (id, home) = (id.to_string(), home.to_string());
    let prev = prev.map(|p| p.id.to_hex());
    let (d, h, op) = (["d", id.as_str()], ["h", home.as_str()], ["op", op]);
    let mut tags: Vec<&[&str]> = vec![
        &["ar", "1"],
        &d,
        &h,
        &["type", "buzz.task"],
        &["title", "Review"],
        &op,
    ];
    let prev = prev.as_deref().map(|p| ["prev", p]);
    if let Some(prev) = &prev {
        tags.push(prev);
    }
    event(keys, 45010, "review", &tags)
}

fn workflow(keys: &Keys, channel: Uuid, id: Uuid) -> Event {
    event(
        keys,
        30620,
        "name: Gate\ntrigger:\n  on: message_posted\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n",
        &[&["d", &id.to_string()], &["h", &channel.to_string()]],
    )
}

/// A store reply as a client would see it.
fn db_reply<T>(result: Result<T, buzz_db::DbError>) -> Reply {
    match result {
        Ok(_) => Reply::Applied,
        Err(e) if crate::handlers::ingest::is_archived_refusal(&e) => invalid(ARCHIVED),
        Err(e) => Reply::Other(e.to_string()),
    }
}

async fn post(f: &Fixture, channel: Uuid) -> Event {
    let posted = message(&f.owner, channel, "before the archive");
    assert_eq!(f.send(&f.owner, posted.clone()).await, Reply::Applied);
    posted
}

async fn add_member(f: &Fixture, channel: Uuid, keys: &Keys) {
    f.state
        .db
        .add_member(
            f.tenant.community(),
            channel,
            &keys.public_key().to_bytes(),
            MemberRole::Member,
            Some(&f.owner.public_key().to_bytes()),
        )
        .await
        .expect("add member");
}

async fn is_member(f: &Fixture, channel: Uuid, keys: &Keys) -> bool {
    f.state
        .db
        .is_member(f.tenant.community(), channel, &keys.public_key().to_bytes())
        .await
        .expect("read membership")
}

/// Store a NIP-33 `version` in `channel` without the gate.
async fn seed_version(f: &Fixture, version: &Event, channel: Uuid) {
    let d = buzz_db::event::extract_d_tag(version).expect("d tag");
    f.state
        .db
        .replace_parameterized_event(
            f.tenant.community(),
            version,
            &d,
            Some(channel),
            buzz_db::ArchiveGate::Exempt,
        )
        .await
        .expect("seed version");
}

/// Open (or re-open) the DM of the owner and `peer`.
async fn open_dm(f: &Fixture, peer: &str) -> Uuid {
    let open = event(&f.owner, 41010, "", &[&["p", peer]]);
    let opened = crate::handlers::ingest::ingest_event(
        &f.state,
        &f.tenant,
        open,
        crate::handlers::group_state_postgres_tests::http_auth(&f.owner),
    )
    .await
    .expect("open DM");
    let body = opened
        .message
        .strip_prefix("response:")
        .expect("DM open response");
    let body: serde_json::Value = serde_json::from_str(body).expect("DM open JSON");
    body["channel_id"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .expect("DM channel id")
}

/// An ephemeral channel whose TTL deadline has passed.
async fn expired_channel(f: &Fixture) -> Uuid {
    let channel = f.channel().await;
    assert_eq!(
        f.send(&f.owner, edit(&f.owner, channel, &[&["ttl", "3600"]]))
            .await,
        Reply::Applied
    );
    sqlx::query(
        "UPDATE channels SET ttl_deadline = NOW() - interval '1 second' \
         WHERE community_id = $1 AND id = $2",
    )
    .bind(f.tenant.community().as_uuid())
    .bind(channel)
    .execute(f.pool())
    .await
    .expect("expire the channel");
    channel
}

async fn archive(f: &Fixture, channel: Uuid) {
    sqlx::query("UPDATE channels SET archived_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(f.tenant.community().as_uuid())
        .bind(channel)
        .execute(f.pool())
        .await
        .expect("archive");
}

/// Archive `channel` in the holder's open transaction.
async fn archive_in(holder: &mut Transaction<'static, Postgres>, f: &Fixture, channel: Uuid) {
    sqlx::query("UPDATE channels SET archived_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(f.tenant.community().as_uuid())
        .bind(channel)
        .execute(&mut **holder)
        .await
        .expect("archive in the holder");
}

async fn advisory_shared(holder: &mut Transaction<'static, Postgres>, key: &str) {
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(key)
        .execute(&mut **holder)
        .await
        .expect("hold advisory lock shared");
}

/// The modes of the requests that wait for the advisory lock `key`.
async fn waiting(f: &Fixture, key: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT mode FROM pg_locks \
         WHERE locktype = 'advisory' AND NOT granted AND objsubid = 1 \
           AND ((classid::bigint << 32) | objid::bigint) = hashtextextended($1, 0) \
         ORDER BY mode",
    )
    .bind(key)
    .fetch_all(f.pool())
    .await
    .expect("inspect advisory waiters")
}

/// Wait until the requests that wait for `key` have exactly `modes`.
async fn queued(f: &Fixture, key: &str, modes: &[&str]) {
    let mut seen = Vec::new();
    let reached = tokio::time::timeout(WAIT, async {
        loop {
            seen = waiting(f, key).await;
            if seen == modes {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        reached.is_ok(),
        "waiters on {key}: {seen:?}, expected {modes:?}"
    );
}

async fn stored(f: &Fixture, id: &EventId) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM events WHERE community_id = $1 AND id = $2)")
        .bind(f.tenant.community().as_uuid())
        .bind(id.as_bytes().as_slice())
        .fetch_one(f.pool())
        .await
        .expect("read event")
}

/// The tables a channel write can change.
const WRITTEN: &[&str] = &[
    "events",
    "event_mentions",
    "thread_metadata",
    "reactions",
    "channel_members",
    "artifact_heads",
    "artifact_revisions",
    "artifact_parents",
    "workflows",
    "workflow_runs",
    "workflow_approvals",
    "parameterized_event_watermarks",
    "push_match_queue",
];

/// A digest of every row this community has in [`WRITTEN`] and in
/// `channels`, without the archive time that the test itself sets.
async fn snapshot(f: &Fixture) -> Vec<(String, Option<String>)> {
    let mut digests = Vec::new();
    let channels = "(to_jsonb(t) - 'archived_at')::text";
    for (table, row) in WRITTEN
        .iter()
        .map(|table| (*table, "t::text"))
        .chain([("channels", channels)])
    {
        let digest = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT md5(string_agg({row}, '|' ORDER BY {row})) FROM {table} AS t \
             WHERE community_id = $1"
        )))
        .bind(f.tenant.community().as_uuid())
        .fetch_one(f.pool())
        .await
        .unwrap_or_else(|e| panic!("snapshot {table}: {e}"));
        digests.push((table.to_string(), digest));
    }
    digests
}

/// One write that a concurrent archive must refuse.
#[derive(Clone, Copy, Debug)]
enum Row {
    /// kind:9, the default insert.
    Message,
    Reaction,
    /// kind:40100 with `expected-revision`.
    CanvasHead,
    /// NIP-16, straight to the store: no client kind is channel-scoped.
    Addressable,
    /// NIP-33, straight to the store.
    Parameterized,
    /// NIP-33 into an open channel, superseding a version in the archived one.
    ParameterizedMove,
    TaskRecord,
    Artifact,
    /// A move out of the archived channel.
    ArtifactMove,
    PutUser,
    DeleteGroup,
    /// A command's first transaction (a workflow definition).
    Command,
    /// Q3: adding a member to an archived DM.
    DmAddMember,
    /// A workflow `send_message` action.
    Sink,
    RemoveMember,
    Join,
    DeleteEvent,
    /// kind:5 with an `e` target.
    Deletion,
    /// E1: a settings-only kind:9002 takes no gate; the row lock orders it.
    SettingsEdit,
    /// E2: kind:5 with an `a` target, gated by the version's channel.
    CoordinateDeletion,
    /// E3: kind:5 of a workflow in the archived channel.
    WorkflowDeletion,
    /// The roster auto-join (invite claim, kind:9030): the shared self-join.
    AutoJoin,
    /// The admission auto-join of a community without a roster.
    FirstJoin,
}

const ROWS: [Row; 23] = [
    Row::Message,
    Row::Reaction,
    Row::CanvasHead,
    Row::Addressable,
    Row::Parameterized,
    Row::ParameterizedMove,
    Row::TaskRecord,
    Row::Artifact,
    Row::ArtifactMove,
    Row::PutUser,
    Row::DeleteGroup,
    Row::Command,
    Row::DmAddMember,
    Row::Sink,
    Row::RemoveMember,
    Row::Join,
    Row::DeleteEvent,
    Row::Deletion,
    Row::SettingsEdit,
    Row::CoordinateDeletion,
    Row::WorkflowDeletion,
    Row::AutoJoin,
    Row::FirstJoin,
];

enum Write {
    Client(Keys, Event),
    Addressable(Event, Uuid),
    Parameterized(Event, Uuid),
    Sink(Uuid),
    SelfJoin(Vec<u8>, Uuid),
    FirstJoin(Vec<u8>, Uuid),
}

struct Late {
    /// The channel the race archives.
    archived: Uuid,
    write: Write,
    expected: Reply,
    /// `false` for a write that waits on the channel row, not the gate.
    gated: bool,
}

/// Build `row`'s state, before the archive starts.
async fn prepare(f: &Fixture, row: Row) -> Late {
    let owner = f.owner.clone();
    let owner_hex = owner.public_key().to_hex();
    let channel = f.channel().await;
    let h = channel.to_string();
    let client = |event: Event| Write::Client(owner.clone(), event);
    let (archived, write, expected, gated) = match row {
        Row::Message => (
            channel,
            client(message(&owner, channel, "late")),
            invalid(ARCHIVED),
            true,
        ),
        Row::Reaction => {
            let target = post(f, channel).await.id.to_hex();
            let reaction = event(&owner, 7, "+", &[&["e", &target]]);
            let expected = invalid("invalid: reaction target event not found");
            (channel, client(reaction), expected, true)
        }
        Row::CanvasHead => {
            let canvas = event(
                &owner,
                40100,
                "canvas",
                &[&["h", &h], &["expected-revision", "none"]],
            );
            (channel, client(canvas), invalid(ARCHIVED), true)
        }
        Row::Addressable => {
            let replaceable = event(&owner, 10999, "", &[&["h", &h]]);
            let write = Write::Addressable(replaceable, channel);
            (channel, write, invalid(ARCHIVED), true)
        }
        Row::Parameterized => {
            let version = event(&owner, 30999, "", &[&["d", "late"], &["h", &h]]);
            let write = Write::Parameterized(version, channel);
            (channel, write, invalid(ARCHIVED), true)
        }
        Row::ParameterizedMove => {
            let open = f.channel().await;
            let v1 = event(&owner, 30998, "v1", &[&["d", "moved"], &["h", &h]]);
            seed_version(f, &v1, channel).await;
            let v2 = event(
                &owner,
                30998,
                "v2",
                &[&["d", "moved"], &["h", &open.to_string()]],
            );
            (
                channel,
                Write::Parameterized(v2, open),
                invalid(ARCHIVED),
                true,
            )
        }
        Row::TaskRecord => {
            let (task, record) = f.task(None).await;
            let update = task_record(&owner, task, None, "update", Some(&record));
            (task, client(update), invalid(ARCHIVED), true)
        }
        Row::Artifact => {
            let create = artifact(&owner, channel, Uuid::new_v4(), "create", None);
            (channel, client(create), invalid(ARCHIVED), true)
        }
        Row::ArtifactMove => {
            let id = Uuid::new_v4();
            let create = artifact(&owner, channel, id, "create", None);
            assert_eq!(f.send(&owner, create.clone()).await, Reply::Applied);
            let open = f.channel().await;
            let moved = artifact(&owner, open, id, "move", Some(&create));
            let expected =
                Reply::Conflict(format!("conflict: {}", buzz_db::artifact::HEAD_UNAVAILABLE));
            (channel, client(moved), expected, true)
        }
        Row::PutUser => {
            let newcomer = Keys::generate().public_key().to_hex();
            let put = admin_event(&owner, 9000, channel, &[&["p", &newcomer]]);
            (channel, client(put), invalid(ARCHIVED), true)
        }
        Row::DeleteGroup => {
            let delete = admin_event(&owner, 9008, channel, &[]);
            (channel, client(delete), invalid(ARCHIVED), true)
        }
        Row::Command => {
            let definition = workflow(&owner, channel, Uuid::new_v4());
            (channel, client(definition), invalid(ARCHIVED), true)
        }
        Row::DmAddMember => {
            let dm = open_dm(f, &Keys::generate().public_key().to_hex()).await;
            let added = Keys::generate().public_key().to_hex();
            let add = event(
                &owner,
                41011,
                "",
                &[&["h", &dm.to_string()], &["p", &added]],
            );
            (dm, client(add), invalid(ARCHIVED), true)
        }
        Row::Sink => (channel, Write::Sink(channel), invalid(ARCHIVED), true),
        Row::RemoveMember => {
            let member = Keys::generate();
            add_member(f, channel, &member).await;
            let removed = member.public_key().to_hex();
            let remove = admin_event(&owner, 9001, channel, &[&["p", &removed]]);
            (channel, client(remove), invalid(ARCHIVED), true)
        }
        Row::Join => {
            let joiner = Keys::generate();
            let join = admin_event(&joiner, 9021, channel, &[]);
            (
                channel,
                Write::Client(joiner, join),
                invalid(ARCHIVED),
                true,
            )
        }
        Row::DeleteEvent => {
            let target = post(f, channel).await.id.to_hex();
            let delete = admin_event(&owner, 9005, channel, &[&["e", &target]]);
            (channel, client(delete), invalid(ARCHIVED), true)
        }
        Row::Deletion => {
            let target = post(f, channel).await.id.to_hex();
            let delete = event(&owner, 5, "", &[&["e", &target]]);
            (channel, client(delete), invalid(DELETION_DENIED), true)
        }
        Row::SettingsEdit => {
            let rename = edit(&owner, channel, &[&["name", "renamed"]]);
            (channel, client(rename), invalid(ARCHIVED), false)
        }
        Row::CoordinateDeletion => {
            let version = event(&owner, 30997, "v", &[&["d", "gone"], &["h", &h]]);
            seed_version(f, &version, channel).await;
            let coordinate = format!("30997:{owner_hex}:gone");
            let delete = event(&owner, 5, "", &[&["a", &coordinate]]);
            (channel, client(delete), invalid(DELETION_DENIED), true)
        }
        Row::WorkflowDeletion => {
            let id = Uuid::new_v4();
            let definition = workflow(&owner, channel, id);
            assert_eq!(f.send(&owner, definition).await, Reply::Applied);
            let coordinate = format!("30620:{owner_hex}:{id}");
            let delete = event(&owner, 5, "", &[&["a", &coordinate]]);
            (channel, client(delete), invalid(DELETION_DENIED), true)
        }
        Row::AutoJoin => {
            let joiner = Keys::generate().public_key().to_bytes().to_vec();
            let write = Write::SelfJoin(joiner, channel);
            (channel, write, invalid(ARCHIVED), true)
        }
        Row::FirstJoin => {
            let joiner = Keys::generate().public_key().to_bytes().to_vec();
            let write = Write::FirstJoin(joiner, channel);
            (channel, write, invalid(ARCHIVED), true)
        }
    };
    Late {
        archived,
        write,
        expected,
        gated,
    }
}

async fn run(f: &Fixture, write: Write) -> Reply {
    let community = f.tenant.community();
    match write {
        Write::Client(keys, event) => f.send(&keys, event).await,
        Write::Addressable(event, channel) => db_reply(
            f.state
                .db
                .replace_addressable_event(
                    community,
                    &event,
                    Some(channel),
                    buzz_db::ArchiveGate::Refuse,
                )
                .await,
        ),
        Write::Parameterized(event, channel) => {
            let d = buzz_db::event::extract_d_tag(&event).expect("d tag");
            db_reply(
                f.state
                    .db
                    .replace_parameterized_event(
                        community,
                        &event,
                        &d,
                        Some(channel),
                        buzz_db::ArchiveGate::Refuse,
                    )
                    .await,
            )
        }
        Write::Sink(channel) => {
            use buzz_workflow::{ActionSink, ActionSinkError};
            let author = f.owner.public_key().to_hex();
            let sink = crate::workflow_sink::RelayActionSink::new(&f.state);
            match sink
                .send_message(
                    community,
                    &channel.to_string(),
                    "late",
                    "late",
                    &author,
                    None,
                )
                .await
            {
                Ok(_) => Reply::Applied,
                Err(ActionSinkError::ChannelArchived(_)) => invalid(ARCHIVED),
                Err(e) => Reply::Other(e.to_string()),
            }
        }
        Write::SelfJoin(pubkey, channel) => {
            let joined = crate::handlers::side_effects::join_channel_as_self(
                &f.tenant, &f.state, channel, &pubkey,
            )
            .await;
            match joined {
                Ok(_) => Reply::Applied,
                Err(e) => match e.downcast::<DbError>() {
                    Ok(e) => db_reply::<()>(Err(e)),
                    Err(e) => Reply::Other(e.to_string()),
                },
            }
        }
        Write::FirstJoin(pubkey, channel) => db_reply(
            f.state
                .db
                .put_first_member(community, channel, &pubkey)
                .await,
        ),
    }
}

/// A write that passed its unlocked checks before an archive committed is
/// refused under the gate, and stores nothing: every refused kind, the
/// store seams no client kind reaches, the command's first transaction, the
/// workflow sink and both auto-join membership writes. A refused write waits
/// for the gate shared. A settings-only kind:9002 (E1) takes no gate and is
/// refused under the row lock. Mutations: drop the gate from a seam, or its
/// archived re-read, and that row stores its write or waits on the row
/// instead of the gate.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn late_writes_are_refused_by_a_concurrent_archive() {
    let f = Fixture::new("archive-gate-late", true).await;
    for row in ROWS {
        let late = prepare(&f, row).await;
        let event_id = match &late.write {
            Write::Client(_, event)
            | Write::Addressable(event, _)
            | Write::Parameterized(event, _) => Some(event.id),
            Write::Sink(_) | Write::SelfJoin(..) | Write::FirstJoin(..) => None,
        };
        let before = snapshot(&f).await;
        let gate = f.key(GATE, late.archived);
        let mut holder = f.holder().await;
        advisory(&mut holder, &gate).await;
        archive_in(&mut holder, &f, late.archived).await;

        let attempt = {
            let f = f.clone();
            tokio::spawn(async move { run(&f, late.write).await })
        };
        if late.gated {
            queued(&f, &gate, &["ShareLock"]).await;
        } else {
            f.waiters(1).await;
            assert!(waiting(&f, &gate).await.is_empty(), "{row:?} takes no gate");
        }
        assert!(!attempt.is_finished(), "{row:?} did not wait");
        holder.commit().await.expect("commit the archive");

        let reply = tokio::time::timeout(WAIT, attempt)
            .await
            .unwrap_or_else(|_| panic!("{row:?} never returned"))
            .expect("the write must not panic");
        assert_eq!(reply, late.expected, "{row:?}");
        assert_eq!(snapshot(&f).await, before, "{row:?} stored nothing");
        if let Some(id) = event_id {
            assert!(!stored(&f, &id).await, "{row:?}: the event is not stored");
        }
        assert!(f.row(late.archived).await.archived, "{row:?}");
    }
}

#[derive(Clone, Copy, Debug)]
enum Writer {
    /// An unguarded kind:9002 archive.
    Archive,
    /// An unguarded kind:9002 restore.
    Restore,
    /// `archive_channel`, as the huddle auto-end calls it.
    ArchiveChannel,
    /// The TTL reaper.
    Reaper,
    /// A guarded archive of a task channel.
    Guarded,
}

/// Every archive writer takes the gate exclusive: it queues behind a write
/// that holds the gate shared and changes the channel after that write.
/// Mutation: take the gate shared, or not at all, in an archive writer and
/// its row never queues.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_writers_take_the_gate_exclusive() {
    let f = Fixture::new("archive-gate-writers", true).await;
    let writers = [
        Writer::Archive,
        Writer::Restore,
        Writer::ArchiveChannel,
        Writer::Reaper,
        Writer::Guarded,
    ];
    for writer in writers {
        let channel = match writer {
            Writer::Reaper => expired_channel(&f).await,
            Writer::Guarded => f.task(None).await.0,
            _ => f.channel().await,
        };
        if matches!(writer, Writer::Restore) {
            archive(&f, channel).await;
        }
        let guarded = match writer {
            Writer::Guarded => Some(f.request(true, Subtree, channel, None, &[channel]).await),
            _ => None,
        };
        let gate = f.key(GATE, channel);
        let mut holder = f.holder().await;
        advisory_shared(&mut holder, &gate).await;

        let attempt = {
            let f = f.clone();
            tokio::spawn(async move {
                let community = f.tenant.community();
                match (writer, guarded) {
                    (Writer::Archive | Writer::Restore, _) => {
                        let value = if matches!(writer, Writer::Archive) {
                            "true"
                        } else {
                            "false"
                        };
                        let change = edit(&f.owner, channel, &[&["archived", value]]);
                        f.send(&f.owner, change).await
                    }
                    (Writer::ArchiveChannel, _) => {
                        match f.state.db.archive_channel(community, channel).await {
                            Ok(true) => Reply::Applied,
                            other => Reply::Other(format!("{other:?}")),
                        }
                    }
                    (Writer::Reaper, _) => {
                        match f.state.db.reap_expired_ephemeral_channels(100).await {
                            Ok(reaped) if reaped.iter().any(|r| r.channel_id == channel) => {
                                Reply::Applied
                            }
                            other => Reply::Other(format!("{other:?}")),
                        }
                    }
                    (Writer::Guarded, Some(request)) => f.send(&f.owner, request).await,
                    (Writer::Guarded, None) => unreachable!("previewed above"),
                }
            })
        };
        queued(&f, &gate, &["ExclusiveLock"]).await;
        assert!(!attempt.is_finished(), "{writer:?} did not wait");
        holder.commit().await.expect("release the shared gate");

        let reply = tokio::time::timeout(WAIT, attempt)
            .await
            .unwrap_or_else(|_| panic!("{writer:?} never returned"))
            .expect("the writer must not panic");
        assert_eq!(reply, Reply::Applied, "{writer:?}");
        let archived = !matches!(writer, Writer::Restore);
        assert_eq!(f.row(channel).await.archived, archived, "{writer:?}");
    }
}

/// V15: two writes into an expired TTL channel race the reaper, in both
/// orders, and nothing deadlocks. Writes queued before the reaper commit
/// first and refresh the deadline, so the reaper skips the channel. A
/// reaper queued first archives the channel, and the writes are refused.
/// Mutation: drop the reaper's deadline re-check and the first order
/// archives a refreshed channel.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn ttl_writes_and_the_reaper_serialize_in_both_orders() {
    let f = Fixture::new("archive-gate-ttl", true).await;
    for reaper_first in [false, true] {
        let channel = expired_channel(&f).await;
        let gate = f.key(GATE, channel);
        let mut holder = f.holder().await;
        advisory(&mut holder, &gate).await;

        let reap = || {
            let f = f.clone();
            tokio::spawn(async move {
                f.state
                    .db
                    .reap_expired_ephemeral_channels(100)
                    .await
                    .expect("reap")
                    .iter()
                    .any(|r| r.channel_id == channel)
            })
        };
        let mut reaper = None;
        let mut expected: Vec<&str> = Vec::new();
        if reaper_first {
            reaper = Some(reap());
            expected.push("ExclusiveLock");
            queued(&f, &gate, &expected).await;
        }
        let mut writes = Vec::new();
        for n in 0..2 {
            writes.push(f.spawn(&f.owner, message(&f.owner, channel, &format!("w{n}"))));
            expected.push("ShareLock");
            expected.sort();
            queued(&f, &gate, &expected).await;
        }
        if !reaper_first {
            reaper = Some(reap());
            expected.push("ExclusiveLock");
            expected.sort();
            queued(&f, &gate, &expected).await;
        }
        holder.commit().await.expect("release the gate");

        let reaped = tokio::time::timeout(WAIT, reaper.expect("reaper started"))
            .await
            .expect("the reaper returns")
            .expect("the reaper must not panic");
        assert_eq!(reaped, reaper_first, "reaper first: {reaper_first}");
        for write in writes {
            let reply = tokio::time::timeout(WAIT, write)
                .await
                .expect("the write returns")
                .expect("the write must not panic");
            let expected = if reaper_first {
                invalid(ARCHIVED)
            } else {
                Reply::Applied
            };
            assert_eq!(reply, expected, "reaper first: {reaper_first}");
        }
        assert_eq!(f.row(channel).await.archived, reaper_first);
        let messages = if reaper_first { 0 } else { 2 };
        assert_eq!(f.count(channel, 9).await, messages);
        if !reaper_first {
            let refreshed: bool = sqlx::query_scalar(
                "SELECT ttl_deadline > NOW() FROM channels WHERE community_id = $1 AND id = $2",
            )
            .bind(f.tenant.community().as_uuid())
            .bind(channel)
            .fetch_one(f.pool())
            .await
            .expect("read the deadline");
            assert!(refreshed, "the writes refreshed the deadline");
        }
    }
}

/// One write that an archived channel does not refuse.
#[derive(Clone, Copy, Debug)]
enum Exempt {
    /// kind:9022 by a member.
    Leave,
    /// kind:9001 that names its own author.
    SelfRemoval,
    /// A relay notice (kind:40099).
    Notice,
    /// Relay discovery state (kind:39000-39002).
    Discovery,
    /// Q2: the caller's own DM view state.
    DmHide,
}

/// Writes that an archived channel does not refuse take no gate: each one
/// completes while an archive writer holds the channel's gate exclusive,
/// and stores its change. Mutation: refuse one of them and its row waits
/// for the gate.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn exempt_writes_take_no_gate() {
    let f = Fixture::new("archive-gate-exempt", true).await;
    let rows = [
        Exempt::Leave,
        Exempt::SelfRemoval,
        Exempt::Notice,
        Exempt::Discovery,
        Exempt::DmHide,
    ];
    for row in rows {
        let member = Keys::generate();
        let channel = match row {
            Exempt::DmHide => open_dm(&f, &Keys::generate().public_key().to_hex()).await,
            _ => f.channel().await,
        };
        add_member(&f, channel, &member).await;
        archive(&f, channel).await;
        let gate = f.key(GATE, channel);
        let mut holder = f.holder().await;
        advisory(&mut holder, &gate).await;

        let attempt = {
            let (f, member) = (f.clone(), member.clone());
            tokio::spawn(async move {
                let (tenant, state) = (&f.tenant, &f.state);
                let side_effects = |result: anyhow::Result<()>| match result {
                    Ok(()) => Reply::Applied,
                    Err(e) => Reply::Other(e.to_string()),
                };
                match row {
                    Exempt::Leave => {
                        f.send(&member, admin_event(&member, 9022, channel, &[]))
                            .await
                    }
                    Exempt::SelfRemoval => {
                        let (h, me) = (channel.to_string(), member.public_key().to_hex());
                        let remove = event(&member, 9001, "", &[&["h", &h], &["p", &me]]);
                        f.send(&member, remove).await
                    }
                    Exempt::Notice => side_effects(
                        crate::handlers::side_effects::emit_system_message(
                            tenant,
                            state,
                            channel,
                            serde_json::json!({"type": "gate_probe"}),
                            chrono::Utc::now(),
                        )
                        .await,
                    ),
                    Exempt::Discovery => side_effects(
                        crate::handlers::side_effects::emit_group_discovery_events(
                            tenant, state, channel,
                        )
                        .await,
                    ),
                    Exempt::DmHide => {
                        let hide = event(&f.owner, 41012, "", &[&["h", &channel.to_string()]]);
                        f.send(&f.owner, hide).await
                    }
                }
            })
        };
        let reply = tokio::time::timeout(WAIT, async {
            while !attempt.is_finished() {
                assert!(
                    waiting(&f, &gate).await.is_empty(),
                    "{row:?} waits for the gate"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            attempt.await.expect("the write must not panic")
        })
        .await
        .unwrap_or_else(|_| panic!("{row:?} never returned"));
        holder.commit().await.expect("release the gate");
        assert_eq!(reply, Reply::Applied, "{row:?}");

        match row {
            Exempt::Leave | Exempt::SelfRemoval => {
                assert!(!is_member(&f, channel, &member).await, "{row:?}");
            }
            Exempt::Notice => assert_eq!(f.notices(channel, "gate_probe").await, 1),
            Exempt::Discovery => {
                let metadata = crate::handlers::group_state_postgres_tests::group_state_tags(
                    &f.state, &f.tenant, channel, 39000,
                )
                .await;
                assert!(
                    metadata.contains(&vec!["archived".to_string(), "true".to_string()]),
                    "{metadata:?}"
                );
            }
            Exempt::DmHide => assert_eq!(f.count(channel, 41012).await, 1),
        }
        assert!(f.row(channel).await.archived, "{row:?}");
    }
}

/// Q1: a kind:9021 is stored and applied in one transaction, decided under
/// the membership lock. A channel made private while the join waits for
/// that lock refuses it, and nothing is stored: no event and no member.
/// Mutation: commit the event before the visibility check and the refused
/// join stays stored.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_join_refused_under_the_lock_stores_nothing() {
    let f = Fixture::new("archive-gate-q1", true).await;
    let channel = f.channel().await;
    let joiner = Keys::generate();
    let join = admin_event(&joiner, 9021, channel, &[]);
    let membership = f.key(MEMBERSHIP, channel);
    let mut holder = f.holder().await;
    advisory(&mut holder, &membership).await;

    let attempt = f.spawn(&joiner, join.clone());
    queued(&f, &membership, &["ExclusiveLock"]).await;
    sqlx::query("UPDATE channels SET visibility = 'private' WHERE community_id = $1 AND id = $2")
        .bind(f.tenant.community().as_uuid())
        .bind(channel)
        .execute(&mut *holder)
        .await
        .expect("make the channel private");
    holder.commit().await.expect("release the membership lock");

    let reply = tokio::time::timeout(WAIT, attempt)
        .await
        .expect("the join returns")
        .expect("the join must not panic");
    assert_eq!(reply, invalid("restricted: channel is private"));
    assert!(
        !stored(&f, &join.id).await,
        "the refused join is not stored"
    );
    assert!(!is_member(&f, channel, &joiner).await);
    assert_eq!(f.count(channel, 9021).await, 0);
}

// -- G1 to G3: DMs, approvals and workflow runs -------------------------------

/// Poll `probe` until it holds.
async fn until<F: std::future::Future<Output = bool>>(what: &str, mut probe: impl FnMut() -> F) {
    let reached = tokio::time::timeout(WAIT, async {
        while !probe().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(reached.is_ok(), "never reached: {what}");
}

async fn join<T>(attempt: tokio::task::JoinHandle<T>) -> T {
    tokio::time::timeout(WAIT, attempt)
        .await
        .expect("the attempt returns")
        .expect("the attempt must not panic")
}

async fn backend_pid(holder: &mut Transaction<'static, Postgres>) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut **holder)
        .await
        .expect("holder pid")
}

/// Wait until a session that holds the gate `gate` shared waits for the
/// transaction of the session `pid`.
async fn waits_on(f: &Fixture, gate: &str, pid: i32) {
    until(
        &format!("a holder of {gate} waits on {pid}"),
        || async move {
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_locks w JOIN pg_locks g ON g.pid = w.pid \
             WHERE w.locktype = 'transactionid' AND NOT w.granted \
               AND $1 = ANY (pg_blocking_pids(w.pid)) \
               AND g.locktype = 'advisory' AND g.granted AND g.mode = 'ShareLock' \
               AND g.objsubid = 1 \
               AND ((g.classid::bigint << 32) | g.objid::bigint) = hashtextextended($2, 0))",
            )
            .bind(pid)
            .bind(gate)
            .fetch_one(f.pool())
            .await
            .expect("inspect waits")
        },
    )
    .await;
}

/// The lock types that the sessions of this test's database wait for.
async fn pending(f: &Fixture) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT l.locktype FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
         WHERE NOT l.granted AND a.datname = current_database() ORDER BY 1",
    )
    .fetch_all(f.pool())
    .await
    .expect("inspect waits")
}

/// Hold an uncommitted copy of `event` in `channel`: a write of the same
/// event waits at its insert until the holder rolls back.
async fn hold_insert(
    holder: &mut Transaction<'static, Postgres>,
    f: &Fixture,
    event: &Event,
    channel: Option<Uuid>,
) {
    let created_at = chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0);
    sqlx::query(
        "INSERT INTO events \
         (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id, d_tag) \
         VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8, $9, $10)",
    )
    .bind(f.tenant.community().as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .bind(event.pubkey.to_bytes().as_slice())
    .bind(created_at.expect("timestamp"))
    .bind(i32::from(event.kind.as_u16()))
    .bind(serde_json::to_string(&event.tags).expect("tags"))
    .bind(&event.content)
    .bind(event.sig.serialize().as_slice())
    .bind(channel)
    .bind(buzz_db::event::extract_d_tag(event))
    .execute(&mut **holder)
    .await
    .expect("hold the event's insert");
}

async fn lock_workflow(holder: &mut Transaction<'static, Postgres>, f: &Fixture, id: Uuid) {
    sqlx::query("SELECT 1 FROM workflows WHERE community_id = $1 AND id = $2 FOR UPDATE")
        .bind(f.tenant.community().as_uuid())
        .bind(id)
        .execute(&mut **holder)
        .await
        .expect("hold the workflow row");
}

/// Rebind workflow `id` to `channel` in the holder's transaction, the only
/// way production can: delete the row, then create it again with the same id.
async fn rebind_in(
    holder: &mut Transaction<'static, Postgres>,
    f: &Fixture,
    id: Uuid,
    channel: Uuid,
) {
    let row: String = sqlx::query_scalar(
        "DELETE FROM workflows AS w WHERE community_id = $1 AND id = $2 RETURNING to_jsonb(w)::text",
    )
    .bind(f.tenant.community().as_uuid())
    .bind(id)
    .fetch_one(&mut **holder)
    .await
    .expect("delete the workflow");
    sqlx::query(
        "INSERT INTO workflows SELECT * FROM jsonb_populate_record(NULL::workflows, \
         $1::jsonb || jsonb_build_object('channel_id', $2::uuid))",
    )
    .bind(row)
    .bind(channel)
    .execute(&mut **holder)
    .await
    .expect("re-create the workflow");
}

/// Workflow `id` in `channel`, defined by the owner through the relay.
async fn define(f: &Fixture, channel: Uuid, id: Uuid) {
    let definition = workflow(&f.owner, channel, id);
    assert_eq!(f.send(&f.owner, definition).await, Reply::Applied);
}

/// The owner's kind:5 of workflow `id`: an E3 deletion.
fn delete_workflow(f: &Fixture, id: Uuid) -> Event {
    let coordinate = format!("30620:{}:{id}", f.owner.public_key().to_hex());
    event(&f.owner, 5, "", &[&["a", &coordinate]])
}

fn trigger(f: &Fixture, workflow: Uuid) -> Event {
    event(&f.owner, 46020, "", &[&["d", &workflow.to_string()]])
}

async fn runs(f: &Fixture, workflow: Uuid) -> usize {
    let community = f.tenant.community();
    let runs = f.state.db.list_workflow_runs(community, workflow, 10).await;
    runs.expect("list runs").len()
}

async fn create_run(f: &Fixture, workflow: Uuid) -> Result<Uuid, DbError> {
    let community = f.tenant.community();
    let gate = ArchiveGate::Refuse;
    f.state
        .db
        .create_workflow_run(community, workflow, None, None, gate)
        .await
}

const HOOK_SECRET: &str = "archive-gate-hook-secret";

/// A webhook workflow of the owner in `channel`, made without the relay.
async fn define_hook(f: &Fixture, channel: Option<Uuid>) -> Uuid {
    let (db, community) = (&f.state.db, f.tenant.community());
    let owner = f.owner.public_key().to_bytes();
    db.ensure_user(community, &owner).await.expect("owner row");
    let definition = serde_json::json!({
        "name": "hook",
        "trigger": {"on": "webhook"},
        "steps": [{"id": "s1", "action": "send_message", "text": "hi"}],
        "_webhook_secret": HOOK_SECRET,
    })
    .to_string();
    let created = db.create_workflow(community, channel, &owner, "hook", &definition, &[9; 32]);
    created.await.expect("webhook workflow")
}

/// Call the webhook of `workflow` with its secret: the status and the
/// error text, if any.
async fn webhook(f: &Fixture, workflow: Uuid) -> (StatusCode, Option<String>) {
    use axum::extract::{Path, Query, State};
    use axum::http::{header, HeaderMap, HeaderValue};
    let mut headers = HeaderMap::new();
    let host = HeaderValue::from_str(f.tenant.host()).expect("host header");
    headers.insert(header::HOST, host);
    headers.insert("x-webhook-secret", HeaderValue::from_static(HOOK_SECRET));
    let query = Query(crate::api::bridge::WebhookQuery { secret: None });
    let id = Path(workflow.to_string());
    let body = axum::body::Bytes::new();
    let reply =
        crate::api::bridge::workflow_webhook(State(f.state.clone()), id, query, headers, body);
    let (Ok((status, axum::Json(body))) | Err((status, axum::Json(body)))) = reply.await;
    (status, body["error"].as_str().map(str::to_owned))
}

fn spawn_archive(f: &Fixture, channel: Uuid) -> tokio::task::JoinHandle<bool> {
    let f = f.clone();
    tokio::spawn(async move {
        let community = f.tenant.community();
        f.state
            .db
            .archive_channel(community, channel)
            .await
            .expect("archive")
    })
}

/// A run of `workflow` that waits on an approval anyone may give. Returns the
/// run and the `d` tag of a command on the approval.
async fn pending_approval(f: &Fixture, workflow: Uuid) -> (Uuid, String) {
    let (db, community) = (&f.state.db, f.tenant.community());
    let run = db
        .create_workflow_run(community, workflow, None, None, ArchiveGate::Exempt)
        .await
        .expect("run");
    let waiting = RunStatus::WaitingApproval;
    db.update_workflow_run(community, run, waiting, 0, &serde_json::json!([]), None)
        .await
        .expect("the run waits");
    let token = Uuid::new_v4().to_string();
    db.create_approval(buzz_db::workflow::CreateApprovalParams {
        community_id: community,
        token: &token,
        workflow_id: workflow,
        run_id: run,
        step_id: "s1",
        step_index: 0,
        approver_spec: "any",
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
    })
    .await
    .expect("approval");
    (run, hex::encode(Sha256::digest(token.as_bytes())))
}

/// G1: in an archived DM, a re-open only clears the caller's own hidden
/// state (Q2), so it succeeds and shows the DM again. An open that names the
/// archived DM in its `h` tag is refused and stores nothing. Adding a member
/// makes a new DM from the archived one, so it is refused and stores nothing.
/// Mutations: gate the channel of a re-opened DM and the re-open is refused;
/// dm_open under Exempt and the h-tagged open is stored.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_archived_dm_reopens_but_takes_no_new_member() {
    let f = Fixture::new("archive-gate-dm", true).await;
    let peer = Keys::generate().public_key().to_hex();
    let dm = open_dm(&f, &peer).await;
    let hide = event(&f.owner, 41012, "", &[&["h", &dm.to_string()]]);
    assert_eq!(f.send(&f.owner, hide).await, Reply::Applied);
    archive(&f, dm).await;
    let tagged = event(
        &f.owner,
        41010,
        "",
        &[&["p", &peer], &["h", &dm.to_string()]],
    );
    assert_eq!(f.send(&f.owner, tagged.clone()).await, invalid(ARCHIVED));
    assert!(!stored(&f, &tagged.id).await);
    let hidden = || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT hidden_at IS NOT NULL FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(f.tenant.community().as_uuid())
        .bind(dm)
        .bind(f.owner.public_key().to_bytes().as_slice())
        .fetch_one(f.pool())
        .await
        .expect("read hidden state")
    };
    assert!(hidden().await);
    assert_eq!(open_dm(&f, &peer).await, dm, "the archived DM re-opens");
    assert!(!hidden().await, "and is shown again");

    let before = snapshot(&f).await;
    let added = Keys::generate().public_key().to_hex();
    let add = event(
        &f.owner,
        41011,
        "",
        &[&["h", &dm.to_string()], &["p", &added]],
    );
    assert_eq!(f.send(&f.owner, add.clone()).await, invalid(ARCHIVED));
    assert_eq!(snapshot(&f).await, before, "no new DM");
    assert!(!stored(&f, &add.id).await);
}

/// The state of an approval's workflow when its command runs.
#[derive(Clone, Copy, Debug)]
enum Approval {
    /// The workflow's channel is archived.
    Archived,
    /// The workflow has no channel.
    Unbound,
    /// The workflow is deleted while the command reads it.
    Missing,
    /// The workflow is deleted and created again in another channel while
    /// the command waits at its event insert.
    Rebound,
    /// The workflow is deleted (E3) and created again with the same id in
    /// another channel before the command.
    Recreated,
}

/// G2: a grant or deny carries no `h` tag. It gates its workflow's channel,
/// read before its first transaction, so an archived channel refuses it. A
/// workflow with no channel, or one deleted while the command reads it,
/// fails closed. A rebind while the command waits, or before it, deletes the
/// approval with the workflow, so the command fails before it commits. Every refusal
/// stores no command event and leaves the run as it was: not resumed, not
/// cancelled. Mutations: gate no channel; take an empty gate set for an
/// unbound or missing workflow; commit when the approval update matched
/// nothing.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn approval_commands_gate_the_workflow_channel() {
    let f = Fixture::new("archive-gate-approval", true).await;
    let (db, community) = (&f.state.db, f.tenant.community());
    let rows = [
        (Approval::Archived, 46030),
        (Approval::Archived, 46031),
        (Approval::Unbound, 46030),
        (Approval::Missing, 46031),
        (Approval::Rebound, 46030),
        (Approval::Rebound, 46031),
        (Approval::Recreated, 46030),
        (Approval::Recreated, 46031),
    ];
    for (row, kind) in rows {
        let channel = f.channel().await;
        let workflow = match row {
            Approval::Unbound => {
                let owner = f.owner.public_key().to_bytes();
                let created =
                    db.create_workflow(community, None, &owner, "unbound", "{}", &[7; 32]);
                created.await.expect("unbound workflow")
            }
            _ => {
                let id = Uuid::new_v4();
                define(&f, channel, id).await;
                id
            }
        };
        let (run, d) = pending_approval(&f, workflow).await;
        let command = event(&f.owner, kind, "", &[&["d", &d]]);
        let reply = match row {
            Approval::Archived => {
                archive(&f, channel).await;
                f.send(&f.owner, command.clone()).await
            }
            Approval::Unbound => f.send(&f.owner, command.clone()).await,
            Approval::Missing => {
                let mut holder = f.holder().await;
                sqlx::query("LOCK TABLE workflows IN ACCESS EXCLUSIVE MODE")
                    .execute(&mut *holder)
                    .await
                    .expect("pause workflow reads");
                let attempt = f.spawn(&f.owner, command.clone());
                f.waiters(1).await;
                sqlx::query("DELETE FROM workflows WHERE community_id = $1 AND id = $2")
                    .bind(community.as_uuid())
                    .bind(workflow)
                    .execute(&mut *holder)
                    .await
                    .expect("delete the workflow");
                holder.commit().await.expect("commit the delete");
                join(attempt).await
            }
            Approval::Rebound => {
                let mut holder = f.holder().await;
                hold_insert(&mut holder, &f, &command, None).await;
                let pid = backend_pid(&mut holder).await;
                let attempt = f.spawn(&f.owner, command.clone());
                waits_on(&f, &f.key(GATE, channel), pid).await;
                let deletion = f.send(&f.owner, delete_workflow(&f, workflow)).await;
                assert_eq!(deletion, Reply::Applied);
                define(&f, f.channel().await, workflow).await;
                holder.rollback().await.expect("release the insert");
                join(attempt).await
            }
            Approval::Recreated => {
                let deletion = f.send(&f.owner, delete_workflow(&f, workflow)).await;
                assert_eq!(deletion, Reply::Applied);
                define(&f, f.channel().await, workflow).await;
                f.send(&f.owner, command.clone()).await
            }
        };
        let expected = match row {
            Approval::Archived => invalid(ARCHIVED),
            Approval::Unbound => invalid("forbidden: workflow has no channel scope"),
            Approval::Missing | Approval::Recreated => invalid("invalid: approval not found"),
            Approval::Rebound => invalid("invalid: approval already acted on (race)"),
        };
        assert_eq!(reply, expected, "{row:?} {kind}");
        assert!(!stored(&f, &command.id).await, "{row:?} {kind}: no event");
        let hash = hex::decode(&d).expect("token hash");
        let approval = db.get_approval_by_stored_hash(community, &hash).await;
        let run = db.get_workflow_run(community, run).await;
        match row {
            Approval::Archived | Approval::Unbound => {
                assert_eq!(approval.expect("approval").status, ApprovalStatus::Pending);
                assert_eq!(run.expect("run").status, RunStatus::WaitingApproval);
            }
            // The delete took the approval and the run with the workflow.
            Approval::Missing | Approval::Rebound | Approval::Recreated => {
                assert!(approval.is_err() && run.is_err(), "{row:?} {kind}");
                assert_eq!(runs(&f, workflow).await, 0, "{row:?} {kind}");
            }
        }
    }
}

/// G2: an approval holds its workflow channel's gate shared until it
/// commits. An archive that queues on the gate meanwhile does not hold up
/// the approval's second write, which takes no gate. The archive applies
/// after the approval commits. Mutation: gate no channel and the archive
/// never queues.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_archive_waits_for_an_approval_in_flight() {
    let f = Fixture::new("archive-gate-approval-order", true).await;
    let (db, community) = (&f.state.db, f.tenant.community());
    let channel = f.channel().await;
    let workflow = Uuid::new_v4();
    define(&f, channel, workflow).await;
    let (_, d) = pending_approval(&f, workflow).await;
    let hash = hex::decode(&d).expect("token hash");
    let mut holder = f.holder().await;
    sqlx::query(
        "SELECT 1 FROM workflow_approvals WHERE community_id = $1 AND token = $2 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(&hash)
    .execute(&mut *holder)
    .await
    .expect("pause the approval update");
    let grant = f.spawn(&f.owner, event(&f.owner, 46030, "", &[&["d", &d]]));
    f.waiters(1).await;
    let archived = spawn_archive(&f, channel);
    queued(&f, &f.key(GATE, channel), &["ExclusiveLock"]).await;
    holder.commit().await.expect("release the approval");

    assert_eq!(join(grant).await, Reply::Applied);
    assert!(join(archived).await);
    let approval = db.get_approval_by_stored_hash(community, &hash).await;
    assert_eq!(approval.expect("approval").status, ApprovalStatus::Granted);
    assert!(f.row(channel).await.archived);
}

/// G3: a manual trigger reads its workflow's channel before its first
/// transaction, and checks the binding there under the gate. Here the
/// workflow is deleted (E3), created again in another channel, and that
/// channel is archived, while the trigger waits at its event insert. None of
/// them waits for the trigger. The trigger is refused with 409 and stores
/// nothing: no event, no run. Mutation: drop the binding check and the
/// workflow runs in the archived channel.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_manual_trigger_refuses_a_stale_binding() {
    let f = Fixture::new("archive-gate-trigger-stale", true).await;
    let (old, new) = (f.channel().await, f.channel().await);
    let workflow = Uuid::new_v4();
    define(&f, old, workflow).await;
    let fire = trigger(&f, workflow);
    let mut holder = f.holder().await;
    hold_insert(&mut holder, &f, &fire, Some(old)).await;
    let pid = backend_pid(&mut holder).await;
    let attempt = f.spawn(&f.owner, fire.clone());
    waits_on(&f, &f.key(GATE, old), pid).await;

    let rebind = async {
        assert_eq!(
            f.send(&f.owner, delete_workflow(&f, workflow)).await,
            Reply::Applied
        );
        define(&f, new, workflow).await;
        assert!(f
            .state
            .db
            .archive_channel(f.tenant.community(), new)
            .await
            .expect("archive"));
    };
    let done = tokio::time::timeout(WAIT, rebind).await;
    assert!(
        done.is_ok(),
        "the rebind and the archive wait for the trigger"
    );
    assert!(!attempt.is_finished(), "the trigger still waits");
    holder.rollback().await.expect("release the insert");

    assert_eq!(join(attempt).await, conflict());
    assert!(!stored(&f, &fire.id).await, "no trigger event");
    assert_eq!(runs(&f, workflow).await, 0, "no run");
}

/// G3: a manual trigger's run (its second write) takes no gate, so it
/// completes while an E3 deletion waits for the trigger's row lock and an
/// archive waits for its gate. A gated run would queue behind the archive,
/// which waits for the trigger: a cycle across two connections that Postgres
/// cannot see. One session holds the workflow row, so the trigger stops at
/// its binding check, after its gate. Another holds `workflow_runs`, so the
/// run stops at its insert. Mutation: create the run under Refuse and it
/// waits on the gate, so the wait never forms.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_manual_trigger_completes_behind_a_queued_archive() {
    let f = Fixture::new("archive-gate-trigger-run", true).await;
    let channel = f.channel().await;
    let workflow = Uuid::new_v4();
    define(&f, channel, workflow).await;
    let gate = f.key(GATE, channel);
    let mut runs_holder = f.holder().await;
    sqlx::query("LOCK TABLE workflow_runs IN SHARE MODE")
        .execute(&mut *runs_holder)
        .await
        .expect("pause run inserts");
    let mut row_holder = f.holder().await;
    lock_workflow(&mut row_holder, &f, workflow).await;
    let pid = backend_pid(&mut row_holder).await;

    let fire = f.spawn(&f.owner, trigger(&f, workflow));
    waits_on(&f, &gate, pid).await;
    let deletion = f.spawn(&f.owner, delete_workflow(&f, workflow));
    f.waiters(2).await;
    let archived = spawn_archive(&f, channel);
    queued(&f, &gate, &["ExclusiveLock"]).await;
    row_holder
        .rollback()
        .await
        .expect("release the workflow row");
    // The run waits at its insert, the deletion for the trigger's
    // transaction, and the archive for the gate.
    until("the run, the deletion and the archive wait", || async {
        pending(&f).await == ["advisory", "relation", "transactionid"]
    })
    .await;
    runs_holder.commit().await.expect("release run inserts");

    assert_eq!(join(fire).await, Reply::Applied);
    assert_eq!(join(deletion).await, Reply::Applied);
    assert!(join(archived).await);
    assert!(f.row(channel).await.archived);
}

/// G3: run creation and an archive serialize on the workflow channel's gate,
/// in both orders. Archive first: the creator queues on the gate and is
/// refused. Creator first (stopped at its binding check, after its gate): the
/// archive queues on the gate and applies after the run commits. A webhook
/// into an archived channel answers 404, like a missing workflow, and creates
/// no run. Mutations: create runs without the gate and the creator never
/// queues; create webhook runs under Exempt and the webhook runs.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn run_creation_and_archive_serialize_in_both_orders() {
    let f = Fixture::new("archive-gate-run-order", true).await;
    for archive_first in [true, false] {
        let channel = f.channel().await;
        let workflow = Uuid::new_v4();
        define(&f, channel, workflow).await;
        let gate = f.key(GATE, channel);
        let create = || {
            let f = f.clone();
            tokio::spawn(async move { create_run(&f, workflow).await })
        };
        let mut holder = f.holder().await;
        let (run, archived) = if archive_first {
            lock_row(&mut holder, channel).await;
            let archived = spawn_archive(&f, channel);
            f.waiters(1).await;
            let run = create();
            queued(&f, &gate, &["ShareLock"]).await;
            (run, archived)
        } else {
            lock_workflow(&mut holder, &f, workflow).await;
            let run = create();
            f.waiters(1).await;
            let archived = spawn_archive(&f, channel);
            queued(&f, &gate, &["ExclusiveLock"]).await;
            (run, archived)
        };
        holder.rollback().await.expect("release the holder");

        let run = join(run).await;
        assert!(join(archived).await, "archive first: {archive_first}");
        if archive_first {
            assert_eq!(db_reply(run), invalid(ARCHIVED));
            assert_eq!(runs(&f, workflow).await, 0);
        } else {
            assert!(run.is_ok(), "{run:?}");
            assert_eq!(runs(&f, workflow).await, 1);
        }
        assert!(f.row(channel).await.archived);
    }

    let channel = f.channel().await;
    let hook = define_hook(&f, Some(channel)).await;
    assert_eq!(webhook(&f, hook).await.0, StatusCode::ACCEPTED);
    archive(&f, channel).await;
    let not_found = (StatusCode::NOT_FOUND, Some("workflow not found".into()));
    assert_eq!(webhook(&f, hook).await, not_found);
    assert_eq!(runs(&f, hook).await, 1, "the archived webhook adds no run");
}

/// G3: under Refuse, a run or a schedule claim of a missing workflow, or of
/// one with no channel, fails closed before any gate and writes nothing. A
/// rebind (a delete and re-create of the same id) that commits while the
/// creator's re-read waits on the row gives `LifecycleConflict`: READ
/// COMMITTED skips the deleted row and does not see the new one. A webhook
/// answers 404 for a workflow with no channel (its own check answers first)
/// and 409 for the rebind, and creates no run. Mutations: take an empty gate
/// set for an unbound workflow; map no row at the re-read to unbound; create
/// webhook runs under Exempt.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn unbound_and_rebound_workflows_admit_nothing() {
    let f = Fixture::new("archive-gate-unbound", true).await;
    let (db, community) = (&f.state.db, f.tenant.community());
    let owner = f.owner.public_key().to_bytes();
    db.ensure_user(community, &owner).await.expect("owner row");
    let unbound = db.create_workflow(community, None, &owner, "unbound", "{}", &[7; 32]);
    let unbound = unbound.await.expect("unbound workflow");
    for workflow in [Uuid::new_v4(), unbound] {
        let now = chrono::Utc::now();
        let run = create_run(&f, workflow).await.map(drop);
        let claim = db.claim_scheduled_workflow_fire(community, workflow, now, ArchiveGate::Refuse);
        for result in [run, claim.await.map(drop)] {
            assert!(
                matches!(&result, Err(DbError::AccessDenied(reason))
                    if reason == buzz_db::workflow::WORKFLOW_UNBOUND),
                "{workflow}: {result:?}"
            );
        }
    }
    assert_eq!(runs(&f, unbound).await, 0);
    let anchor = db.latest_scheduled_workflow_fire(community, unbound).await;
    assert_eq!(anchor.expect("anchor"), None, "no claim");

    let (old, new) = (f.channel().await, f.channel().await);
    let workflow = Uuid::new_v4();
    define(&f, old, workflow).await;
    let mut holder = f.holder().await;
    rebind_in(&mut holder, &f, workflow, new).await;
    let pid = backend_pid(&mut holder).await;
    let run = {
        let f = f.clone();
        tokio::spawn(async move { create_run(&f, workflow).await })
    };
    waits_on(&f, &f.key(GATE, old), pid).await;
    holder.commit().await.expect("commit the rebind");
    let run = join(run).await;
    assert!(matches!(run, Err(DbError::LifecycleConflict)), "{run:?}");
    assert_eq!(runs(&f, workflow).await, 0);

    let unbound = define_hook(&f, None).await;
    let not_found = (StatusCode::NOT_FOUND, Some("workflow not found".into()));
    assert_eq!(webhook(&f, unbound).await, not_found);
    assert_eq!(runs(&f, unbound).await, 0);
    let (old, new) = (f.channel().await, f.channel().await);
    let hook = define_hook(&f, Some(old)).await;
    let mut holder = f.holder().await;
    rebind_in(&mut holder, &f, hook, new).await;
    let pid = backend_pid(&mut holder).await;
    let posted = {
        let f = f.clone();
        tokio::spawn(async move { webhook(&f, hook).await })
    };
    waits_on(&f, &f.key(GATE, old), pid).await;
    holder.commit().await.expect("commit the rebind");
    let conflict = (
        StatusCode::CONFLICT,
        Some("conflict: lifecycle changed".into()),
    );
    assert_eq!(join(posted).await, conflict);
    assert_eq!(runs(&f, hook).await, 0);
}
