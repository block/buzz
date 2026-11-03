//! Postgres-backed tests for guarded task archive and restore (NEXUS-4111).
//!
//! Every request goes through `ingest_event`, so validation, routing, the
//! atomic write and the reply mapping are the production path. Races are
//! ordered by locks that a second session holds, never by sleeps: a test
//! takes a lock, starts the contenders, waits until they queue on it, then
//! releases it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use buzz_core::channel::MemberRole;
use buzz_core::lifecycle::{lifecycle_digest, task_d, LifecycleScope, TokenEntry};
use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{community, create_channel, http_auth, state};
use crate::handlers::ingest::{ingest_event, IngestError, IngestResult};
use crate::state::AppState;

const GUARD_REQUIRED: &str = "invalid: lifecycle guard required";
const CHANGED: &str = "conflict: lifecycle changed";
const NOT_AUTHORIZED_S: &str = "invalid: actor not authorized for name/about/archived/visibility/ttl/label/join_role/unlisted changes";
const NOT_AUTHORIZED_ALL: &str = "invalid: actor not authorized for every affected channel";

/// One reply, as a client classifies it.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Applied,
    Duplicate,
    /// HTTP 409.
    Conflict(String),
    /// HTTP 400.
    Invalid(String),
    Other(String),
}

fn classify(result: Result<IngestResult, IngestError>) -> Reply {
    match result {
        Ok(r) if r.accepted && r.message.starts_with("duplicate:") => Reply::Duplicate,
        Ok(r) if r.accepted => Reply::Applied,
        Ok(r) => Reply::Other(r.message),
        Err(IngestError::CanvasConflict(m)) => Reply::Conflict(m),
        Err(IngestError::Rejected(m)) => Reply::Invalid(m),
        Err(e) => Reply::Other(format!("{e:?}")),
    }
}

fn conflict() -> Reply {
    Reply::Conflict(CHANGED.into())
}

fn invalid(message: &str) -> Reply {
    Reply::Invalid(message.into())
}

/// A strictly increasing `created_at`, so two requests with the same tags
/// are distinct events. Event ids are content hashes: the same tags in the
/// same second would be one event, a duplicate.
fn stamp() -> Timestamp {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    Timestamp::from(Timestamp::now().as_secs() + NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Lifecycle columns of one channel row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    archived: bool,
    deleted: bool,
    token: Option<Uuid>,
}

#[derive(Clone)]
struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
    owner: Keys,
}

impl Fixture {
    async fn new(label: &str, guard: bool) -> Self {
        let mut state = (*state().await).clone();
        Arc::make_mut(&mut state.config).lifecycle_guard_enabled = guard;
        let state = Arc::new(state);
        let tenant = community(&state, label).await;
        Self {
            state,
            tenant,
            owner: Keys::generate(),
        }
    }

    /// The same community and owner on a relay with the flag set to `guard`.
    fn with_guard(&self, guard: bool) -> Self {
        let mut state = (*self.state).clone();
        Arc::make_mut(&mut state.config).lifecycle_guard_enabled = guard;
        Self {
            state: Arc::new(state),
            ..self.clone()
        }
    }

    fn pool(&self) -> &PgPool {
        self.state.db.pool()
    }

    async fn send(&self, keys: &Keys, event: Event) -> Reply {
        classify(ingest_event(&self.state, &self.tenant, event, http_auth(keys)).await)
    }

    /// Send in the background, for a race.
    fn spawn(&self, keys: &Keys, event: Event) -> tokio::task::JoinHandle<Reply> {
        let (fixture, keys) = (self.clone(), keys.clone());
        tokio::spawn(async move { fixture.send(&keys, event).await })
    }

    async fn channel(&self) -> Uuid {
        create_channel(&self.state, &self.tenant, &self.owner, &[]).await
    }

    /// A new channel with an accepted task record.
    async fn task(&self, parent: Option<Uuid>) -> (Uuid, Event) {
        let channel = self.channel().await;
        let record = task_record(&self.owner, channel, parent, "create", None);
        assert_eq!(self.send(&self.owner, record.clone()).await, Reply::Applied);
        (channel, record)
    }

    async fn row(&self, channel: Uuid) -> Row {
        let (archived, deleted, token) = sqlx::query_as::<_, (bool, bool, Option<Uuid>)>(
            "SELECT archived_at IS NOT NULL, deleted_at IS NOT NULL, lifecycle_token \
             FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(self.tenant.community().as_uuid())
        .bind(channel)
        .fetch_one(self.pool())
        .await
        .expect("channel row");
        Row {
            archived,
            deleted,
            token,
        }
    }

    async fn rows(&self, channels: &[Uuid]) -> Vec<Row> {
        let mut rows = Vec::new();
        for channel in channels {
            rows.push(self.row(*channel).await);
        }
        rows
    }

    /// The projected parent: `None` without a projection row.
    async fn projected(&self, channel: Uuid) -> Option<Option<Uuid>> {
        sqlx::query_scalar(
            "SELECT parent_channel_id FROM artifact_parents \
             WHERE community_id = $1 AND child_channel_id = $2",
        )
        .bind(self.tenant.community().as_uuid())
        .bind(channel)
        .fetch_optional(self.pool())
        .await
        .expect("projection row")
    }

    async fn count(&self, channel: Uuid, kind: i32) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = $3",
        )
        .bind(self.tenant.community().as_uuid())
        .bind(channel)
        .bind(kind)
        .fetch_one(self.pool())
        .await
        .expect("count events")
    }

    /// Kind:40099 notices of `kind` (`channel_archived`, …) in `channel`.
    async fn notices(&self, channel: Uuid, kind: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 40099 \
               AND content::jsonb ->> 'type' = $3",
        )
        .bind(self.tenant.community().as_uuid())
        .bind(channel)
        .bind(kind)
        .fetch_one(self.pool())
        .await
        .expect("count notices")
    }

    /// The digest a client previews, from the tokens it reads now. The
    /// test names `compared` and `affected` from the family it built.
    async fn expect(
        &self,
        archive: bool,
        scope: LifecycleScope,
        target: Uuid,
        compared: Option<Uuid>,
        affected: &[Uuid],
    ) -> String {
        let compared = match compared {
            Some(channel) => Some(TokenEntry {
                channel,
                token: self.row(channel).await.token,
            }),
            None => None,
        };
        let mut entries = Vec::new();
        for channel in affected {
            entries.push(TokenEntry {
                channel: *channel,
                token: self.row(*channel).await.token,
            });
        }
        hex::encode(lifecycle_digest(archive, scope, target, compared, &entries))
    }

    /// A guarded archive (`archive`) or restore, previewed now.
    async fn request(
        &self,
        archive: bool,
        scope: LifecycleScope,
        target: Uuid,
        compared: Option<Uuid>,
        affected: &[Uuid],
    ) -> Event {
        let expect = self
            .expect(archive, scope, target, compared, affected)
            .await;
        request_event(&self.owner, target, archive, scope.as_str(), &expect)
    }

    async fn add_admin(&self, channel: Uuid, keys: &Keys) {
        self.state
            .db
            .add_member(
                self.tenant.community(),
                channel,
                &keys.public_key().to_bytes(),
                MemberRole::Admin,
                Some(&self.owner.public_key().to_bytes()),
            )
            .await
            .expect("add admin");
    }

    /// A second session that holds locks until it commits.
    async fn holder(&self) -> Transaction<'static, Postgres> {
        self.pool().begin().await.expect("begin holder")
    }

    fn key(&self, namespace: &str, channel: Uuid) -> String {
        format!(
            "{namespace}:{}:{channel}",
            self.tenant.community().as_uuid()
        )
    }

    /// Wait until `n` sessions of this test's database wait on a lock.
    async fn waiters(&self, n: i64) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE datname = current_database() AND wait_event_type = 'Lock'",
                )
                .fetch_one(self.pool())
                .await
                .expect("inspect lock waits");
                if waiting >= n {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{n} sessions never queued on a lock"));
    }
}

async fn advisory(tx: &mut Transaction<'static, Postgres>, key: &str) {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(key)
        .execute(&mut **tx)
        .await
        .expect("hold advisory lock");
}

async fn lock_row(tx: &mut Transaction<'static, Postgres>, channel: Uuid) {
    sqlx::query("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel)
        .execute(&mut **tx)
        .await
        .expect("hold row lock");
}

fn task_record(
    keys: &Keys,
    channel: Uuid,
    parent: Option<Uuid>,
    op: &str,
    prev: Option<&Event>,
) -> Event {
    let mut tags = vec![
        vec!["ar".to_string(), "1".into()],
        vec!["d".into(), task_d(channel).to_string()],
        vec!["h".into(), channel.to_string()],
        vec!["type".into(), "nexus.task".into()],
        vec!["op".into(), op.into()],
    ];
    let mut content = String::new();
    if op != "delete" {
        tags.push(vec!["title".into(), "Task".into()]);
        let mut body = json!({"v": 0, "channelId": channel.to_string()});
        if let Some(parent) = parent {
            tags.push(vec!["parent".into(), parent.to_string()]);
            body["parentChannelId"] = json!(parent.to_string());
        }
        content = body.to_string();
    }
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    EventBuilder::new(Kind::Custom(45010), content)
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .custom_created_at(stamp())
        .sign_with_keys(keys)
        .unwrap()
}

fn admin_event(keys: &Keys, kind: u16, target: Uuid, tags: &[&[&str]]) -> Event {
    let mut built = vec![Tag::parse(["h", &target.to_string()]).unwrap()];
    built.extend(tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()));
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(built)
        .custom_created_at(stamp())
        .sign_with_keys(keys)
        .unwrap()
}

fn edit(keys: &Keys, target: Uuid, tags: &[&[&str]]) -> Event {
    admin_event(keys, 9002, target, tags)
}

fn request_event(keys: &Keys, target: Uuid, archive: bool, scope: &str, expect: &str) -> Event {
    let archived = if archive { "true" } else { "false" };
    edit(
        keys,
        target,
        &[
            &["archived", archived],
            &["scope", scope],
            &["expect", expect],
        ],
    )
}

fn fence_event(keys: &Keys, target: Uuid, token: Option<Uuid>) -> Event {
    let token = token.map_or("-".to_string(), |t| t.to_string());
    edit(
        keys,
        target,
        &[&["lifecycle", "fence"], &["expect-target", &token]],
    )
}

use LifecycleScope::{Subtree, Target};

/// Archive and restore of a family (A ← B ← C, A ← D): affected sets,
/// tokens, notices, duplicates, stale digests, state refusals, VD2-b,
/// the TTL exclusion and all-or-nothing permission.
/// Mutations: skipping the digest compare applies the stale request;
/// writing only the target leaves C active; dropping the per-channel
/// permission check archives B and C for an admin of A alone.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn guarded_archive_and_restore_journey() {
    let f = Fixture::new("lifecycle-journey", true).await;
    let (a, _) = f.task(None).await;
    let (b, _) = f.task(Some(a)).await;
    let (c, _) = f.task(Some(b)).await;
    let (d, _) = f.task(Some(a)).await;
    assert_eq!(f.projected(c).await, Some(Some(b)));
    assert_eq!(f.projected(a).await, Some(None));
    let before = f.rows(&[a, b, c, d]).await;
    assert!(
        before.iter().all(|r| r.token.is_some()),
        "create sets a token"
    );

    // A stale preview: B's token changes before the request arrives.
    let stale = f.request(true, Subtree, b, Some(a), &[b, c]).await;
    assert_eq!(
        f.send(&f.owner, fence_event(&f.owner, b, before[1].token))
            .await,
        Reply::Applied
    );
    let stored = f.count(b, 9002).await;
    assert_eq!(f.send(&f.owner, stale).await, conflict());
    assert_eq!(f.count(b, 9002).await, stored, "a refusal stores nothing");

    // Archive B: the subtree B, C; A and D are untouched.
    let archive_b = f.request(true, Subtree, b, Some(a), &[b, c]).await;
    assert_eq!(f.send(&f.owner, archive_b.clone()).await, Reply::Applied);
    let after = f.rows(&[a, b, c, d]).await;
    assert_eq!(
        after.iter().map(|r| r.archived).collect::<Vec<_>>(),
        [false, true, true, false]
    );
    assert_eq!(
        (after[0].token, after[3].token),
        (before[0].token, before[3].token)
    );
    assert_ne!(after[2].token, before[2].token);
    assert_eq!(
        (
            f.notices(b, "channel_archived").await,
            f.notices(c, "channel_archived").await
        ),
        (1, 1)
    );
    let notice: String = sqlx::query_scalar(
        "SELECT content FROM events \
         WHERE channel_id = $1 AND kind = 40099 AND content::jsonb ->> 'type' = 'channel_archived'",
    )
    .bind(c)
    .fetch_one(f.pool())
    .await
    .unwrap();
    let notice: serde_json::Value = serde_json::from_str(&notice).unwrap();
    assert_eq!(notice["type"], "channel_archived");
    assert_eq!(notice["cascade_root"], json!(b.to_string()));
    // A lost reply: the same event proves it applied.
    assert_eq!(f.send(&f.owner, archive_b).await, Reply::Duplicate);
    assert_eq!(f.rows(&[a, b, c, d]).await, after);

    // State refusals keep today's text.
    let again = f.request(true, Subtree, b, Some(a), &[b, c]).await;
    assert_eq!(
        f.send(&f.owner, again).await,
        invalid("invalid: channel is archived")
    );
    let active = f.request(false, Target, d, Some(a), &[d]).await;
    assert_eq!(
        f.send(&f.owner, active).await,
        invalid("invalid: channel is not archived")
    );

    // Restore C, target scope: C plus the archived ancestor B.
    let restore_c = f.request(false, Target, c, Some(a), &[b, c]).await;
    assert_eq!(f.send(&f.owner, restore_c).await, Reply::Applied);
    assert!(f.rows(&[a, b, c, d]).await.iter().all(|r| !r.archived));

    // Archive the whole family, then restore only A, then B's subtree.
    let all = f.request(true, Subtree, a, None, &[a, b, c, d]).await;
    assert_eq!(f.send(&f.owner, all).await, Reply::Applied);
    let restore_a = f.request(false, Target, a, None, &[a]).await;
    assert_eq!(f.send(&f.owner, restore_a).await, Reply::Applied);
    assert_eq!(
        f.rows(&[a, b, c, d])
            .await
            .iter()
            .map(|r| r.archived)
            .collect::<Vec<_>>(),
        [false, true, true, true]
    );
    let restore_b = f.request(false, Subtree, b, Some(a), &[b, c]).await;
    assert_eq!(f.send(&f.owner, restore_b).await, Reply::Applied);
    assert_eq!(
        f.rows(&[a, b, c, d])
            .await
            .iter()
            .map(|r| r.archived)
            .collect::<Vec<_>>(),
        [false, false, false, true]
    );

    // VD2-b and the TTL exclusion; other settings keep today's rules.
    assert_eq!(
        f.send(&f.owner, edit(&f.owner, b, &[&["archived", "true"]]))
            .await,
        invalid(GUARD_REQUIRED)
    );
    assert_eq!(
        f.send(&f.owner, edit(&f.owner, b, &[&["ttl", "3600"]]))
            .await,
        invalid("invalid: task channels cannot have a ttl")
    );
    assert_eq!(
        f.send(&f.owner, edit(&f.owner, b, &[&["topic", "still editable"]]))
            .await,
        Reply::Applied
    );
    assert!(!f.row(b).await.archived);

    // Permission: a stranger is refused on S before the transaction, and
    // an admin of A alone is refused for the whole subtree under the lock.
    let stranger = Keys::generate();
    let expect = f.expect(true, Subtree, a, None, &[a, b, c]).await;
    assert_eq!(
        f.send(
            &stranger,
            request_event(&stranger, a, true, "subtree", &expect)
        )
        .await,
        invalid(NOT_AUTHORIZED_S)
    );
    let admin = Keys::generate();
    f.add_admin(a, &admin).await;
    let snapshot = f.rows(&[a, b, c, d]).await;
    let stored = f.count(a, 9002).await;
    let expect = f.expect(true, Subtree, a, None, &[a, b, c]).await;
    assert_eq!(
        f.send(&admin, request_event(&admin, a, true, "subtree", &expect))
            .await,
        invalid(NOT_AUTHORIZED_ALL)
    );
    assert_eq!(f.rows(&[a, b, c, d]).await, snapshot, "nothing changed");
    assert_eq!(
        f.count(a, 9002).await,
        stored,
        "the refused request is not stored"
    );
}

/// F replaces only the target's token when it still matches, on any
/// live channel, with or without a projection row. A permission refusal
/// is never a 409, even with a stale token.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn fence_journey() {
    let f = Fixture::new("lifecycle-fence", true).await;
    let (task, _) = f.task(None).await;
    let plain = f.channel().await;

    // A row from before the token column: NULL, fenced with "-".
    sqlx::query("UPDATE channels SET lifecycle_token = NULL WHERE id = $1")
        .bind(plain)
        .execute(f.pool())
        .await
        .unwrap();
    let fence = fence_event(&f.owner, plain, None);
    assert_eq!(f.send(&f.owner, fence.clone()).await, Reply::Applied);
    let fenced = f.row(plain).await;
    assert!(fenced.token.is_some());
    assert_eq!(f.send(&f.owner, fence).await, Reply::Duplicate);
    assert_eq!(f.row(plain).await, fenced);
    assert_eq!(
        f.send(&f.owner, fence_event(&f.owner, plain, None)).await,
        conflict()
    );

    // On a task channel and on an archived channel; nothing else changes.
    let before = f.row(task).await;
    assert_eq!(
        f.send(&f.owner, fence_event(&f.owner, task, before.token))
            .await,
        Reply::Applied
    );
    let after = f.row(task).await;
    assert_ne!(after.token, before.token);
    assert_eq!((after.archived, after.deleted), (false, false));
    assert_eq!(
        f.notices(task, "channel_archived").await,
        0,
        "a fence posts no notice"
    );
    let archive = f.request(true, Subtree, task, None, &[task]).await;
    assert_eq!(f.send(&f.owner, archive).await, Reply::Applied);
    let archived = f.row(task).await;
    assert_eq!(
        f.send(&f.owner, fence_event(&f.owner, task, archived.token))
            .await,
        Reply::Applied
    );
    assert!(f.row(task).await.archived);

    // A member without the archive right gets 400 for a stale and for a
    // current token alike.
    let member = Keys::generate();
    f.state
        .db
        .add_member(
            f.tenant.community(),
            plain,
            &member.public_key().to_bytes(),
            MemberRole::Member,
            None,
        )
        .await
        .unwrap();
    for token in [None, f.row(plain).await.token] {
        assert_eq!(
            f.send(&member, fence_event(&member, plain, token)).await,
            invalid(NOT_AUTHORIZED_S)
        );
    }
}

/// Any guard tag makes the whole guard mandatory: each malformed shape is
/// refused, on a task channel and on a plain one, and never applies as an
/// unguarded edit. A well-formed request without a projection row is a
/// conflict, never a fall-through.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn malformed_guards_never_fall_through() {
    let f = Fixture::new("lifecycle-malformed", true).await;
    let (task, _) = f.task(None).await;
    let plain = f.channel().await;
    let digest = "0".repeat(64);
    let upper = "A".repeat(64);
    for target in [task, plain] {
        let h = target.to_string();
        let cases: Vec<(&str, Vec<Vec<&str>>)> = vec![
            (
                "no expect",
                vec![vec!["archived", "true"], vec!["scope", "subtree"]],
            ),
            (
                "archive target scope",
                vec![
                    vec!["archived", "true"],
                    vec!["scope", "target"],
                    vec!["expect", &digest],
                ],
            ),
            (
                "uppercase expect",
                vec![
                    vec!["archived", "true"],
                    vec!["scope", "subtree"],
                    vec!["expect", &upper],
                ],
            ),
            (
                "fence with archived",
                vec![
                    vec!["lifecycle", "fence"],
                    vec!["expect-target", "-"],
                    vec!["archived", "true"],
                ],
            ),
            (
                "unknown lifecycle",
                vec![vec!["lifecycle", "settle"], vec!["expect-target", "-"]],
            ),
            (
                "fence without expect-target",
                vec![vec!["lifecycle", "fence"]],
            ),
            (
                "expect-target alone",
                vec![vec!["expect-target", "-"], vec!["archived", "true"]],
            ),
            (
                "settings tag",
                vec![
                    vec!["archived", "true"],
                    vec!["scope", "subtree"],
                    vec!["expect", &digest],
                    vec!["name", "renamed"],
                ],
            ),
            (
                "second h",
                vec![
                    vec!["h", &h],
                    vec!["archived", "true"],
                    vec!["scope", "subtree"],
                    vec!["expect", &digest],
                ],
            ),
            (
                "archived arity",
                vec![
                    vec!["archived", "true", "x"],
                    vec!["scope", "subtree"],
                    vec!["expect", &digest],
                ],
            ),
            (
                "scope arity",
                vec![
                    vec!["archived", "true"],
                    vec!["scope", "subtree", "x"],
                    vec!["expect", &digest],
                ],
            ),
            (
                "repeated expect",
                vec![
                    vec!["archived", "true"],
                    vec!["scope", "subtree"],
                    vec!["expect", &digest],
                    vec!["expect", &digest],
                ],
            ),
            (
                "no archived",
                vec![vec!["scope", "subtree"], vec!["expect", &digest]],
            ),
        ];
        for (case, tags) in cases {
            let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
            let before = f.row(target).await;
            let reply = f.send(&f.owner, edit(&f.owner, target, &tags)).await;
            assert!(
                matches!(&reply, Reply::Invalid(m) if m.starts_with("invalid: ")),
                "{case} on {target}: {reply:?}"
            );
            assert_eq!(f.row(target).await, before, "{case}");
        }
        // An `h` with extra elements is malformed too.
        let event = EventBuilder::new(Kind::Custom(9002), "")
            .tags([
                Tag::parse(["h", &h, "x"]).unwrap(),
                Tag::parse(["archived", "true"]).unwrap(),
                Tag::parse(["scope", "subtree"]).unwrap(),
                Tag::parse(["expect", &digest]).unwrap(),
            ])
            .custom_created_at(stamp())
            .sign_with_keys(&f.owner)
            .unwrap();
        assert!(matches!(f.send(&f.owner, event).await, Reply::Invalid(_)));
        assert_eq!(
            f.count(target, 9002).await,
            0,
            "no malformed guard is stored"
        );
    }
    // Well formed, but the plain channel has no projection row.
    let request = f.request(true, Subtree, plain, None, &[plain]).await;
    assert_eq!(f.send(&f.owner, request).await, conflict());
    assert!(!f.row(plain).await.archived);
}

/// The flag gates only new parent edges (and the capability). With it
/// off, existing projection rows are kept and maintained, guards stay
/// enforced and a new root is not a task channel (no adoption bridge).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn flag_off_keeps_the_rollback_floor() {
    let on = Fixture::new("lifecycle-floor", true).await;
    let (a, a_record) = on.task(None).await;
    let (b, b_record) = on.task(Some(a)).await;
    let (other, _) = on.task(None).await;
    let off = on.with_guard(false);
    const GUARD_OFF: &str = "invalid: nexus.task parent requires the lifecycle guard";

    // A root written with the flag off is accepted but not projected, so
    // it keeps today's unguarded archive.
    let (solo, _) = off.task(None).await;
    assert_eq!(off.projected(solo).await, None);
    assert_eq!(
        off.send(&off.owner, edit(&off.owner, solo, &[&["archived", "true"]]))
            .await,
        Reply::Applied
    );
    // No new parent edge with the flag off.
    let fresh = off.channel().await;
    assert_eq!(
        off.send(
            &off.owner,
            task_record(&off.owner, fresh, Some(a), "create", None)
        )
        .await,
        invalid(GUARD_OFF)
    );
    assert_eq!(off.projected(fresh).await, None);
    // An existing edge survives a revision; a changed one is refused.
    let b_update = task_record(&off.owner, b, Some(a), "update", Some(&b_record));
    assert_eq!(off.send(&off.owner, b_update.clone()).await, Reply::Applied);
    assert_eq!(
        off.send(
            &off.owner,
            task_record(&off.owner, b, Some(other), "update", Some(&b_update))
        )
        .await,
        invalid(GUARD_OFF)
    );
    assert_eq!(off.projected(b).await, Some(Some(a)));
    // A root revision keeps its projection row.
    let a_update = task_record(&off.owner, a, None, "update", Some(&a_record));
    assert_eq!(off.send(&off.owner, a_update.clone()).await, Reply::Applied);
    assert_eq!(off.projected(a).await, Some(None));
    // Guards stay enforced: VD2-b, child delete, and R itself.
    assert_eq!(
        off.send(&off.owner, edit(&off.owner, b, &[&["archived", "true"]]))
            .await,
        invalid(GUARD_REQUIRED)
    );
    assert_eq!(
        off.send(
            &off.owner,
            task_record(&off.owner, a, None, "delete", Some(&a_update))
        )
        .await,
        invalid("invalid: a task with child tasks cannot be deleted")
    );
    let archive = off.request(true, Subtree, a, None, &[a, b]).await;
    assert_eq!(off.send(&off.owner, archive).await, Reply::Applied);
    assert!(off.rows(&[a, b]).await.iter().all(|r| r.archived));
}

/// A child attach and an archive of its parent's family serialize on the
/// family lock in both orders: the archive never leaves an active child
/// under an archived parent.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn child_attach_and_archive_serialize_in_both_orders() {
    let f = Fixture::new("lifecycle-attach", true).await;
    let (a, _) = f.task(None).await;
    let (b, _) = f.task(Some(a)).await;
    let (c, _) = f.task(Some(b)).await;

    // Attach first: the attach holds the family while it waits on B's row;
    // the archive, previewed without the new child, queues on the family.
    let e = f.channel().await;
    let archive = f.request(true, Subtree, b, Some(a), &[b, c]).await;
    let mut holder = f.holder().await;
    lock_row(&mut holder, b).await;
    let attach = f.spawn(&f.owner, task_record(&f.owner, e, Some(b), "create", None));
    f.waiters(1).await;
    let archived = f.spawn(&f.owner, archive);
    f.waiters(2).await;
    holder.commit().await.unwrap();
    assert_eq!(attach.await.unwrap(), Reply::Applied);
    assert_eq!(
        archived.await.unwrap(),
        conflict(),
        "the new child changes the digest"
    );
    assert!(f.rows(&[b, c, e]).await.iter().all(|r| !r.archived));
    let archive = f.request(true, Subtree, b, Some(a), &[b, c, e]).await;
    assert_eq!(f.send(&f.owner, archive).await, Reply::Applied);
    assert!(f.rows(&[b, c, e]).await.iter().all(|r| r.archived));

    // Archive first: the archive holds the family while it waits on D's
    // row; the attach queues on the family and then finds D archived.
    let (d, _) = f.task(Some(a)).await;
    let g = f.channel().await;
    let archive = f.request(true, Subtree, d, Some(a), &[d]).await;
    let mut holder = f.holder().await;
    lock_row(&mut holder, d).await;
    let archived = f.spawn(&f.owner, archive);
    f.waiters(1).await;
    let attach = f.spawn(&f.owner, task_record(&f.owner, g, Some(d), "create", None));
    f.waiters(2).await;
    holder.commit().await.unwrap();
    assert_eq!(archived.await.unwrap(), Reply::Applied);
    assert_eq!(
        attach.await.unwrap(),
        invalid("invalid: parent must be a live task")
    );
    assert_eq!(f.projected(g).await, None);
}

/// Overlapping requests and stale replays. The second of two overlapping
/// archives, previewed before the first applied, is a conflict. A stored
/// request replays as a duplicate; once its event is cleaned up it is a
/// conflict, never a second archive. A fence settles a request whose reply
/// was lost while it is still in flight.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn overlapping_and_stale_requests_never_apply_twice() {
    let f = Fixture::new("lifecycle-overlap", true).await;
    let (a, _) = f.task(None).await;
    let (b, _) = f.task(Some(a)).await;
    let (c, _) = f.task(Some(b)).await;
    let (d, _) = f.task(Some(a)).await;

    let inner = f.request(true, Subtree, b, Some(a), &[b, c]).await;
    let outer = f.request(true, Subtree, a, None, &[a, b, c, d]).await;
    let mut holder = f.holder().await;
    lock_row(&mut holder, c).await;
    let first = f.spawn(&f.owner, inner.clone());
    f.waiters(1).await;
    let second = f.spawn(&f.owner, outer);
    f.waiters(2).await;
    holder.commit().await.unwrap();
    assert_eq!(first.await.unwrap(), Reply::Applied);
    assert_eq!(second.await.unwrap(), conflict());
    assert_eq!(
        f.rows(&[a, b, c, d])
            .await
            .iter()
            .map(|r| r.archived)
            .collect::<Vec<_>>(),
        [false, true, true, false]
    );

    // Restore B's subtree, then replay the archive.
    let restore = f.request(false, Subtree, b, Some(a), &[b, c]).await;
    assert_eq!(f.send(&f.owner, restore).await, Reply::Applied);
    assert_eq!(f.send(&f.owner, inner.clone()).await, Reply::Duplicate);
    assert!(!f.row(b).await.archived);
    // Retention removed the stored event: the replay is a conflict.
    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(inner.id.as_bytes().as_slice())
        .execute(f.pool())
        .await
        .unwrap();
    assert_eq!(f.send(&f.owner, inner).await, conflict());
    assert!(f.rows(&[b, c]).await.iter().all(|r| !r.archived));

    // A request queued on B's gate; the client gave up, read B's token
    // and fenced it. The fence takes no gate, so it commits first and the
    // queued request can no longer apply.
    let pending = f.request(true, Subtree, b, Some(a), &[b, c]).await;
    let mut holder = f.holder().await;
    advisory(&mut holder, &f.key("buzz_channel_gate", b)).await;
    let queued = f.spawn(&f.owner, pending);
    f.waiters(1).await;
    let read = f
        .state
        .db
        .lifecycle_state(f.tenant.community(), &f.owner.public_key().to_bytes(), &[b])
        .await
        .unwrap();
    assert_eq!(read[0].lifecycle_token, f.row(b).await.token);
    assert_eq!(
        f.send(&f.owner, fence_event(&f.owner, b, read[0].lifecycle_token))
            .await,
        Reply::Applied
    );
    holder.commit().await.unwrap();
    assert_eq!(queued.await.unwrap(), conflict());
    assert!(f.rows(&[b, c]).await.iter().all(|r| !r.archived));
}

/// F and a TTL change on the same channel serialize in both orders
/// without a deadlock, and F's stored event refreshes the TTL deadline.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn fence_and_ttl_change_serialize_in_both_orders() {
    let f = Fixture::new("lifecycle-ttl", true).await;
    let channel = f.channel().await;
    let deadline = |f: Fixture| async move {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT ttl_deadline FROM channels WHERE id = $1",
        )
        .bind(channel)
        .fetch_one(f.pool())
        .await
        .unwrap()
    };
    for (ttl, fence_first) in [("3600", false), ("7200", true)] {
        let token = f.row(channel).await.token;
        let mut holder = f.holder().await;
        lock_row(&mut holder, channel).await;
        let change = edit(&f.owner, channel, &[&["ttl", ttl]]);
        let fence = fence_event(&f.owner, channel, token);
        let (first, second) = if fence_first {
            (fence, change)
        } else {
            (change, fence)
        };
        let first = f.spawn(&f.owner, first);
        f.waiters(1).await;
        let second = f.spawn(&f.owner, second);
        f.waiters(2).await;
        holder.commit().await.unwrap();
        assert_eq!(
            first.await.unwrap(),
            Reply::Applied,
            "fence_first={fence_first}"
        );
        assert_eq!(
            second.await.unwrap(),
            Reply::Applied,
            "fence_first={fence_first}"
        );
        let ttl_seconds: Option<i32> =
            sqlx::query_scalar("SELECT ttl_seconds FROM channels WHERE id = $1")
                .bind(channel)
                .fetch_one(f.pool())
                .await
                .unwrap();
        assert_eq!(ttl_seconds.map(|s| s.to_string()).as_deref(), Some(ttl));
        assert_ne!(f.row(channel).await.token, token);
    }
    let before = deadline(f.clone())
        .await
        .expect("TTL channel has a deadline");
    let token = f.row(channel).await.token;
    assert_eq!(
        f.send(&f.owner, fence_event(&f.owner, channel, token))
            .await,
        Reply::Applied
    );
    assert!(deadline(f.clone()).await.expect("deadline") > before);
}

/// Deleting a task with child tasks is refused, as a channel (9008) and
/// as a record; a leaf delete removes its projection row. A child attach
/// that commits while a 9008 waits on the channel's locks makes that 9008
/// refused, and nothing is stored: the 9008 and its delete share one commit.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn task_delete_with_children_is_refused() {
    let f = Fixture::new("lifecycle-delete", true).await;
    let (a, a_record) = f.task(None).await;
    let (b, b_record) = f.task(Some(a)).await;
    let delete_group = |target| admin_event(&f.owner, 9008, target, &[]);

    assert_eq!(
        f.send(&f.owner, delete_group(a)).await,
        invalid("invalid: a task with child tasks cannot be deleted")
    );
    assert_eq!(f.count(a, 9008).await, 0);
    assert_eq!(
        f.send(
            &f.owner,
            task_record(&f.owner, a, None, "delete", Some(&a_record))
        )
        .await,
        invalid("invalid: a task with child tasks cannot be deleted")
    );
    assert_eq!(
        f.send(
            &f.owner,
            task_record(&f.owner, b, None, "delete", Some(&b_record))
        )
        .await,
        Reply::Applied
    );
    assert_eq!(
        f.projected(b).await,
        None,
        "a record delete removes the row"
    );
    assert_eq!(f.send(&f.owner, delete_group(a)).await, Reply::Applied);
    assert!(f.row(a).await.deleted);
    assert_eq!(
        f.projected(a).await,
        None,
        "a channel delete removes the row"
    );

    // The attach commits while the delete waits on the channel's
    // membership lock.
    let (p, _) = f.task(None).await;
    let child = f.channel().await;
    let mut holder = f.holder().await;
    advisory(&mut holder, &f.key("buzz_channel_membership", p)).await;
    let deleting = f.spawn(&f.owner, delete_group(p));
    f.waiters(1).await;
    assert_eq!(
        f.send(
            &f.owner,
            task_record(&f.owner, child, Some(p), "create", None)
        )
        .await,
        Reply::Applied
    );
    holder.commit().await.unwrap();
    assert_eq!(
        deleting.await.unwrap(),
        invalid("invalid: a task with child tasks cannot be deleted")
    );
    assert_eq!(f.count(p, 9008).await, 0, "a refusal stores nothing");
    assert!(!f.row(p).await.deleted);
    assert_eq!(f.projected(child).await, Some(Some(p)));
}

/// A channel without a projection row keeps today's behavior (Solo).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn plain_channels_keep_todays_behavior() {
    let f = Fixture::new("lifecycle-solo", true).await;
    let channel = f.channel().await;
    for archived in ["true", "false"] {
        assert_eq!(
            f.send(
                &f.owner,
                edit(&f.owner, channel, &[&["archived", archived]])
            )
            .await,
            Reply::Applied
        );
        assert_eq!(f.row(channel).await.archived, archived == "true");
    }
    assert_eq!(f.notices(channel, "channel_archived").await, 1);
    assert_eq!(f.notices(channel, "channel_unarchived").await, 1);
    assert_eq!(
        f.send(&f.owner, edit(&f.owner, channel, &[&["ttl", "3600"]]))
            .await,
        Reply::Applied
    );
    // A task record cannot attach to a TTL channel.
    assert_eq!(
        f.send(
            &f.owner,
            task_record(&f.owner, channel, None, "create", None)
        )
        .await,
        invalid("invalid: task channels cannot have a ttl")
    );
    assert_eq!(f.projected(channel).await, None);
    assert_eq!(
        f.send(&f.owner, admin_event(&f.owner, 9008, channel, &[]))
            .await,
        Reply::Applied
    );
    assert!(f.row(channel).await.deleted);
    // The huddle path keeps a task channel active and archives a plain one.
    let (task, _) = f.task(None).await;
    let other = f.channel().await;
    for (channel, archives) in [(task, false), (other, true)] {
        let archived = f
            .state
            .db
            .archive_channel(f.tenant.community(), channel)
            .await
            .unwrap();
        assert_eq!(archived, archives);
        assert_eq!(f.row(channel).await.archived, archives);
    }
}

/// Join `channel`'s huddle as the owner, leave as its last peer and wait
/// until the relay's teardown has returned.
async fn join_and_leave_huddle(f: &Fixture, channel: Uuid) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind huddle listener");
    let addr = listener.local_addr().expect("huddle listener address");
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let assertion = buzz_auth::VerifiedAssertion::for_test(
        Some(f.owner.public_key()),
        vec![chrono::Utc::now() + chrono::Duration::hours(1)],
    );
    let (state, tenant) = (Arc::clone(&f.state), f.tenant.clone());
    let app = axum::Router::new().route(
        "/",
        axum::routing::get(move |ws: axum::extract::ws::WebSocketUpgrade| {
            let (state, tenant) = (Arc::clone(&state), tenant.clone());
            let (assertion, done_tx) = (assertion.clone(), done_tx.clone());
            async move {
                ws.on_upgrade(move |socket| async move {
                    crate::audio::handler::handle_active_audio_connection(
                        socket,
                        state,
                        tenant,
                        channel,
                        crate::state::CommunityConnectionControl::new(
                            tokio_util::sync::CancellationToken::new(),
                        ),
                        Some(assertion),
                        chrono::Utc::now(),
                        None,
                    )
                    .await;
                    let _ = done_tx.send(());
                })
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
        .await
        .expect("connect huddle client");
    let next_json = |frame: Option<Result<Message, _>>| match frame {
        Some(Ok(Message::Text(text))) => {
            serde_json::from_str::<serde_json::Value>(&text).expect("huddle frame is JSON")
        }
        other => panic!("expected a text frame, got {other:?}"),
    };
    let wait = std::time::Duration::from_secs(10);
    let challenge = next_json(
        tokio::time::timeout(wait, client.next())
            .await
            .expect("challenge"),
    );
    let auth = EventBuilder::new(Kind::Authentication, "")
        .tags([
            Tag::parse(["relay", &format!("ws://{}", f.tenant.host())]).unwrap(),
            Tag::parse(["challenge", challenge["challenge"].as_str().unwrap()]).unwrap(),
        ])
        .sign_with_keys(&f.owner)
        .unwrap();
    let auth = json!({
        "type": "auth",
        "event": auth,
        "parent_channel_id": null,
        "protocol_version": 1,
    });
    client
        .send(Message::Text(auth.to_string().into()))
        .await
        .expect("send huddle auth");
    loop {
        let frame = tokio::time::timeout(wait, client.next())
            .await
            .expect("joined frame");
        if let Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) = frame {
            continue;
        }
        let frame = next_json(frame);
        if frame["type"] == "joined" {
            break;
        }
    }
    client.close(None).await.expect("leave the huddle");
    tokio::time::timeout(wait, done_rx.recv())
        .await
        .expect("huddle teardown finished");
    server.abort();
    let _ = server.await;
}

/// The last peer leaving a huddle that a channel hosts directly ends the
/// huddle (kind:48103, room removed) and archives that channel. A task
/// channel ends the huddle the same way but stays active.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn huddle_auto_end_leaves_a_task_channel_active() {
    let f = Fixture::new("lifecycle-huddle", true).await;
    let (task, _) = f.task(None).await;
    let plain = f.channel().await;
    for (channel, is_task) in [(plain, false), (task, true)] {
        join_and_leave_huddle(&f, channel).await;
        assert_eq!(f.count(channel, 48101).await, 1, "the peer joined");
        assert_eq!(f.count(channel, 48102).await, 1, "the peer left");
        assert_eq!(f.row(channel).await.archived, !is_task, "task={is_task}");
        assert_eq!(f.count(channel, 48103).await, 1, "task={is_task}");
        assert!(
            f.state
                .audio_rooms
                .get(f.tenant.community(), channel)
                .is_none(),
            "task={is_task}: the room is removed"
        );
    }
}
