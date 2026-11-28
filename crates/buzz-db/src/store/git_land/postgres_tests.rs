//! The land's locks against the writers that revoke a push, on a real
//! Postgres: each writer waits for a held land, lands never wait on each
//! other, and the row swap is a compare-and-set that a second land misses.
use super::*;
use sqlx::PgPool;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

async fn setup_pool() -> PgPool {
    let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| TEST_DB_URL.to_owned());
    PgPool::connect(&database_url)
        .await
        .expect("connect to test DB")
}

fn pk() -> [u8; 32] {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(&Uuid::new_v4().as_u128().to_be_bytes());
    key[16..].copy_from_slice(&Uuid::new_v4().as_u128().to_be_bytes());
    key
}

struct Fixture {
    pool: PgPool,
    community: CommunityId,
    repo: RepoCoordinate,
    channel: Uuid,
    member: [u8; 32],
}

/// A community with a repository row bound to a channel, and a community
/// member who is a channel member.
async fn fixture() -> Fixture {
    let pool = setup_pool().await;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("git-land-test-{}.example", id.simple()))
        .execute(&pool)
        .await
        .expect("insert community");
    let community = CommunityId::from_uuid(id);
    let owner = pk();
    let member = pk();
    let channel = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
         VALUES ($1, $2, $3, 'stream', 'open', $4)",
    )
    .bind(channel)
    .bind(id)
    .bind(format!("git-land-{}", channel.simple()))
    .bind(owner.as_slice())
    .execute(&pool)
    .await
    .expect("insert channel");
    sqlx::query(
        "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
         VALUES ($1, $2, $3, 'member', $3)",
    )
    .bind(id)
    .bind(channel)
    .bind(member.as_slice())
    .execute(&pool)
    .await
    .expect("insert channel member");
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(id)
        .bind(hex::encode(member))
        .execute(&pool)
        .await
        .expect("insert community member");
    let repo = RepoCoordinate {
        owner: hex::encode(owner),
        name: format!("repo-{}", Uuid::new_v4().simple()),
    };
    sqlx::query(
        "INSERT INTO git_repo_names (community_id, repo_id, owner_pubkey, channel_id, manifest) \
         VALUES ($1, $2, $3, $4, 'm0')",
    )
    .bind(id)
    .bind(&repo.name)
    .bind(&repo.owner)
    .bind(channel)
    .execute(&pool)
    .await
    .expect("insert repository row");
    Fixture {
        pool,
        community,
        repo,
        channel,
        member,
    }
}

impl Fixture {
    async fn tx(&self) -> AdmittedTx {
        crate::begin_community_write_transaction(
            &self.pool,
            self.community,
            crate::observability::WriterOperation::Authorization,
        )
        .await
        .expect("begin")
    }

    /// A transaction that gives up on any lock after a short wait.
    async fn impatient_tx(&self) -> AdmittedTx {
        let mut tx = self.tx().await;
        sqlx::query("SET LOCAL lock_timeout = '200ms'")
            .execute(tx.conn())
            .await
            .expect("lock_timeout");
        tx
    }
}

fn is_lock_timeout(result: Result<impl std::fmt::Debug>) -> bool {
    match result {
        Err(crate::DbError::Sqlx(sqlx::Error::Database(error))) => {
            error.code().as_deref() == Some("55P03")
        }
        other => panic!("expected a lock timeout, got {other:?}"),
    }
}

/// Every writer that can revoke a push waits while a land holds the repo,
/// and gets through once the land commits.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_held_land_blocks_every_revoking_writer() {
    let f = fixture().await;
    let mut land = f.tx().await;
    let held = hold(&mut land, &f.repo).await.expect("hold").expect("row");
    assert_eq!(held.channel, Some(f.channel));
    assert_eq!(held.manifest.as_deref(), Some("m0"));
    let facts = key_facts(&mut land, f.channel, &f.member)
        .await
        .expect("facts");
    assert!(facts.key.community_member && !facts.key.banned);
    assert_eq!(facts.key.channel_role.as_deref(), Some("member"));
    assert_eq!(facts.owner, None);

    let mut w = f.impatient_tx().await;
    assert!(is_lock_timeout(
        crate::governance::fence_repo_exclusive(&mut w, &f.repo).await
    ));
    let mut w = f.impatient_tx().await;
    assert!(is_lock_timeout(
        crate::governance::fence_community_exclusive(&mut w, f.community)
            .await
            .map(|()| 0)
    ));
    let mut w = f.impatient_tx().await;
    assert!(is_lock_timeout(
        crate::relay_invite::lock_admission(w.conn(), f.community)
            .await
            .map(|()| 0)
    ));
    let mut w = f.impatient_tx().await;
    assert!(is_lock_timeout(
        crate::channel_members::acquire_channel_membership_lock(w.conn(), f.community, f.channel)
            .await
            .map(|()| 0)
    ));
    let mut w = f.impatient_tx().await;
    assert!(is_lock_timeout(
        crate::channel_members::lock_channel_archived_at_in_transaction(&mut w, f.channel).await
    ));
    // The key a replacement of the repository's announcement takes.
    let announcement = crate::replaceable::event_replacement_lock_key(
        f.community,
        30617,
        &hex::decode(&f.repo.owner).expect("owner"),
        Some(f.repo.name.as_bytes()),
    );
    let mut w = f.impatient_tx().await;
    assert!(is_lock_timeout(
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(announcement)
            .execute(w.conn())
            .await
            .map(|_| 0)
            .map_err(crate::DbError::from)
    ));
    let mut w = f.impatient_tx().await;
    assert!(is_lock_timeout(
        sqlx::query("DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2")
            .bind(f.community.as_uuid())
            .bind(hex::encode(f.member))
            .execute(w.conn())
            .await
            .map(|_| 0)
            .map_err(crate::DbError::from)
    ));

    land.commit().await.expect("commit land");
    let mut w = f.impatient_tx().await;
    assert_eq!(
        crate::governance::fence_repo_exclusive(&mut w, &f.repo)
            .await
            .expect("writer after the land"),
        1
    );
    crate::governance::fence_community_exclusive(&mut w, f.community)
        .await
        .expect("community writer after the land");
    w.commit().await.expect("commit writer");
}

/// An agent's land holds its owner's community membership too.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_land_holds_its_owner() {
    let f = fixture().await;
    let owner = pk();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(f.community.as_uuid())
        .bind(hex::encode(owner))
        .execute(&f.pool)
        .await
        .expect("insert owner");
    sqlx::query("INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)")
        .bind(f.community.as_uuid())
        .bind(f.member.as_slice())
        .bind(owner.as_slice())
        .execute(&f.pool)
        .await
        .expect("record owner");
    let mut land = f.tx().await;
    hold(&mut land, &f.repo).await.expect("hold");
    let facts = key_facts(&mut land, f.channel, &f.member)
        .await
        .expect("facts");
    let (recorded, theirs) = facts.owner.expect("owner facts");
    assert_eq!(recorded, owner);
    assert!(theirs.community_member && !theirs.banned);
    assert_eq!(theirs.channel_role, None, "the owner is not in the channel");
    let mut w = f.impatient_tx().await;
    assert!(is_lock_timeout(
        sqlx::query("DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2")
            .bind(f.community.as_uuid())
            .bind(hex::encode(owner))
            .execute(w.conn())
            .await
            .map(|_| 0)
            .map_err(crate::DbError::from)
    ));
    land.commit().await.expect("commit");
}

/// Two lands hold the same repository at once; the second swap waits for the
/// first to commit, then misses because the row moved.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn lands_share_the_hold_and_the_second_swap_misses() {
    let f = fixture().await;
    let mut first = f.tx().await;
    let mut second = f.impatient_tx().await;
    let a = hold(&mut first, &f.repo).await.expect("first hold");
    let b = hold(&mut second, &f.repo).await.expect("second hold");
    assert_eq!(a, b, "lands never wait on each other to hold");
    key_facts(&mut second, f.channel, &f.member)
        .await
        .expect("second reads facts");
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(second.conn())
        .await
        .expect("lock_timeout");

    let won = swap(&mut first, &f.repo, Some("m0"), "m1", None)
        .await
        .expect("first swap")
        .expect("first wins");
    assert_eq!(won.version, 0);
    let repo = f.repo.clone();
    let racer = tokio::spawn(async move {
        let lost = swap(&mut second, &repo, Some("m0"), "m2", None).await;
        second.rollback().await.expect("rollback");
        lost
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!racer.is_finished(), "the second swap waits on the row");
    first.commit().await.expect("commit");
    assert_eq!(racer.await.expect("join").expect("second swap"), None);
    assert_eq!(
        f.pool_manifest().await.as_deref(),
        Some("m1"),
        "only the winner moved the row"
    );
}

/// A swap past the authority's expiry does not move the row.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_expired_authority_does_not_swap() {
    let f = fixture().await;
    let mut land = f.tx().await;
    hold(&mut land, &f.repo).await.expect("hold");
    let past = Utc::now() - chrono::Duration::seconds(1);
    assert_eq!(
        swap(&mut land, &f.repo, Some("m0"), "m1", Some(past))
            .await
            .expect("swap"),
        None
    );
    let future = Utc::now() + chrono::Duration::seconds(60);
    let won = swap(&mut land, &f.repo, Some("m0"), "m1", Some(future))
        .await
        .expect("swap")
        .expect("unexpired wins");
    assert!(won.swapped_at < future);
    land.commit().await.expect("commit");
    assert_eq!(f.pool_manifest().await.as_deref(), Some("m1"));
}

/// Seeding fills only an empty row; pinning keeps the first channel.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn seed_and_pin_never_move_an_existing_value() {
    let f = fixture().await;
    let db = Db::from_pool(f.pool.clone());
    assert_eq!(
        db.git_repo_seed_manifest(f.community, &f.repo, "other")
            .await
            .expect("seed")
            .as_deref(),
        Some("m0")
    );
    sqlx::query(
        "UPDATE git_repo_names SET manifest = NULL, channel_id = NULL \
         WHERE community_id = $1 AND repo_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(&f.repo.name)
    .execute(&f.pool)
    .await
    .expect("clear");
    assert_eq!(
        db.git_repo_seed_manifest(f.community, &f.repo, "s3")
            .await
            .expect("seed")
            .as_deref(),
        Some("s3")
    );
    assert_eq!(
        db.git_repo_manifest(f.community, &f.repo)
            .await
            .expect("read")
            .as_deref(),
        Some("s3")
    );

    let mut tx = f.tx().await;
    assert_eq!(
        pin_channel(&mut tx, &f.repo, f.channel).await.expect("pin"),
        Some(f.channel)
    );
    assert_eq!(
        pin_channel(&mut tx, &f.repo, Uuid::new_v4())
            .await
            .expect("re-pin"),
        Some(f.channel),
        "the first pin wins"
    );
    tx.commit().await.expect("commit");
}

/// A repository with no row holds nothing and reports it.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_unknown_repository_holds_nothing() {
    let f = fixture().await;
    let mut tx = f.tx().await;
    let unknown = RepoCoordinate {
        owner: f.repo.owner.clone(),
        name: "missing".into(),
    };
    assert_eq!(hold(&mut tx, &unknown).await.expect("hold"), None);
    assert_eq!(
        swap(&mut tx, &unknown, None, "m1", None)
            .await
            .expect("swap"),
        None
    );
    assert_eq!(
        announcement_tags(&mut tx, &unknown).await.expect("tags"),
        None
    );
    assert!(channel_closed(&mut tx, Uuid::new_v4())
        .await
        .expect("closed"));
    assert!(!channel_closed(&mut tx, f.channel).await.expect("open"));
    tx.rollback().await.expect("rollback");
}

impl Fixture {
    async fn pool_manifest(&self) -> Option<String> {
        sqlx::query_scalar(
            "SELECT manifest FROM git_repo_names WHERE community_id = $1 AND repo_id = $2",
        )
        .bind(self.community.as_uuid())
        .bind(&self.repo.name)
        .fetch_one(&self.pool)
        .await
        .expect("read manifest")
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_delayed_seed_rollback_preserves_a_committed_land() {
    let f = fixture().await;
    let mut tx = f.tx().await;
    hold(&mut tx, &f.repo).await.unwrap();
    assert!(swap(&mut tx, &f.repo, Some("m0"), "m1", None)
        .await
        .unwrap()
        .is_some());
    tx.commit().await.unwrap();
    let removed =
        crate::git_repo::release_repo_name(&f.pool, f.community, &f.repo.name, &f.repo.owner)
            .await
            .unwrap();
    assert_eq!(
        removed, 0,
        "a late failed seed must not delete a row another request has landed"
    );
}

/// NIP-IA archival is visibility, not access (NIP-IA.md:436): an archive
/// committing while a land holds the repository neither waits on the land nor
/// changes the facts the land credits.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_archive_racing_a_land_changes_nothing() {
    let f = fixture().await;
    let mut land = f.tx().await;
    hold(&mut land, &f.repo).await.expect("hold").expect("row");
    let before = key_facts(&mut land, f.channel, &f.member)
        .await
        .expect("facts");
    assert!(before.admitted(false));
    let archived = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::archived_identities::archive(
            &f.pool,
            f.community,
            &hex::encode(f.member),
            "self",
            &hex::encode(f.member),
            None,
            None,
            &"c".repeat(64),
        ),
    )
    .await
    .expect("archiving does not wait on a land")
    .expect("archive");
    assert!(archived);
    let after = key_facts(&mut land, f.channel, &f.member)
        .await
        .expect("facts");
    assert_eq!(after, before, "archival changes no land fact");
    assert!(swap(&mut land, &f.repo, Some("m0"), "m1", None)
        .await
        .expect("swap")
        .is_some());
    land.commit().await.expect("commit");
    assert_eq!(f.pool_manifest().await.as_deref(), Some("m1"));
}
