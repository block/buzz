//! Git repository name registry (NIP-34 kind:30617).
//!
//! The relay holds no persistent per-repo filesystem state: git reads and
//! writes hydrate an ephemeral bare repo from object storage per request, and
//! writer serialization is the object-store pointer CAS (see
//! `docs/git-on-object-storage.md`, `Inv_NoFork`). Repo-*name* uniqueness is
//! the one remaining shared-state need, and it lives here — in Postgres, not on
//! local disk — so the relay is stateless and can run multiple replicas without
//! a ReadWriteMany volume.
//!
//! Names are unique **within a community**: the primary key is
//! `(community_id, repo_id)`, matching the multi-tenant invariant that every
//! tenant-scoped key leads with `community_id`. The PK enforces uniqueness
//! atomically via `INSERT … ON CONFLICT DO NOTHING`, which replaces the old
//! filesystem `create_dir` race guard. `owner_pubkey` distinguishes an
//! idempotent re-announce (same owner) from a collision (different owner), and
//! backs the per-pubkey quota via `COUNT`.

use sqlx::{PgPool, Row as _};

use crate::error::Result;
use crate::CommunityId;

/// Outcome of a name-reservation attempt.
///
/// The caller (kind:30617 handler) uses this to decide whether to seed the
/// manifest pointer and, on seed failure, whether to release the reservation:
/// only a `Reserved` (freshly inserted) row should be rolled back — an
/// `AlreadyOwned` re-announce must leave the pre-existing reservation intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveOutcome {
    /// The name was newly claimed by this owner (a fresh row was inserted).
    Reserved,
    /// The name was already reserved by this same owner — idempotent
    /// re-announce, a no-op update. No row was inserted; the quota was not
    /// re-checked (re-announcing an already-owned name never grows the count).
    AlreadyOwned,
    /// The name is held by a *different* owner — a collision. No row was
    /// inserted.
    TakenByOther,
}

/// Return the current owner pubkey of `repo_id` in `community`, or `None` if
/// the name is unreserved. Used to classify an announce (same-owner
/// re-announce vs cross-owner collision) and to gate the quota check before a
/// fresh claim.
pub async fn repo_name_owner(
    pool: &PgPool,
    community: CommunityId,
    repo_id: &str,
) -> Result<Option<String>> {
    let row = sqlx::query(
        "SELECT owner_pubkey FROM git_repo_names \
         WHERE community_id = $1 AND repo_id = $2",
    )
    .bind(community.as_uuid())
    .bind(repo_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_get("owner_pubkey"))
        .transpose()
        .map_err(crate::error::DbError::from)
}

/// Reserve `repo_id` for `owner_pubkey` within `community`, enforcing a
/// per-pubkey quota of `max_repos_per_pubkey`.
///
/// Semantics (mirrors the previous filesystem registry exactly):
/// - already reserved by the same owner → [`ReserveOutcome::AlreadyOwned`]
///   (idempotent, no quota check);
/// - already reserved by another owner → [`ReserveOutcome::TakenByOther`];
/// - otherwise, if the owner is under quota, atomically claim it →
///   [`ReserveOutcome::Reserved`]; if a concurrent announce wins the insert
///   race, the `ON CONFLICT` re-read resolves it to `AlreadyOwned` (same owner
///   racing itself) or `TakenByOther`.
///
/// Returns `Err` only on backend/database failure — a full quota is *not* an
/// error here; the caller enforces the limit against `Reserved` outcomes using
/// [`count_repos_for_owner`]. (Kept as a separate call so the handler owns the
/// error message and the ordering, matching the old code.)
pub async fn reserve_repo_name(
    pool: &PgPool,
    community: CommunityId,
    repo_id: &str,
    owner_pubkey: &str,
) -> Result<ReserveOutcome> {
    // Atomic claim: insert only if the (community, repo) is free. RETURNING is
    // non-empty exactly when *this* statement inserted the row, so it cleanly
    // distinguishes "I claimed it" from "someone already holds it" without a
    // separate read (TOCTOU-free, the same guarantee `create_dir` gave).
    let inserted = sqlx::query(
        "INSERT INTO git_repo_names (community_id, repo_id, owner_pubkey) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (community_id, repo_id) DO NOTHING \
         RETURNING owner_pubkey",
    )
    .bind(community.as_uuid())
    .bind(repo_id)
    .bind(owner_pubkey)
    .fetch_optional(pool)
    .await?;

    if inserted.is_some() {
        return Ok(ReserveOutcome::Reserved);
    }

    // The row already existed — read the holder to classify same-owner
    // re-announce vs cross-owner collision.
    let existing = sqlx::query(
        "SELECT owner_pubkey FROM git_repo_names \
         WHERE community_id = $1 AND repo_id = $2",
    )
    .bind(community.as_uuid())
    .bind(repo_id)
    .fetch_optional(pool)
    .await?;

    match existing {
        Some(row) => {
            let holder: String = row
                .try_get("owner_pubkey")
                .map_err(crate::error::DbError::from)?;
            if holder == owner_pubkey {
                Ok(ReserveOutcome::AlreadyOwned)
            } else {
                Ok(ReserveOutcome::TakenByOther)
            }
        }
        // Extremely narrow: the conflicting row was deleted between our INSERT
        // and this SELECT (e.g. a concurrent seed-failure rollback). Treat as
        // taken-by-other rather than silently granting — the announcer can
        // retry, and we never hand out a name we didn't atomically claim.
        None => Ok(ReserveOutcome::TakenByOther),
    }
}

/// Count the repos currently reserved by `owner_pubkey` in `community`.
///
/// Backs the per-pubkey quota. Called *before* [`reserve_repo_name`] for a
/// not-yet-owned name, so the handler can reject over-quota announces with its
/// own error message.
pub async fn count_repos_for_owner(
    pool: &PgPool,
    community: CommunityId,
    owner_pubkey: &str,
) -> Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM git_repo_names \
         WHERE community_id = $1 AND owner_pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(owner_pubkey)
    .fetch_one(pool)
    .await?;
    row.try_get("n").map_err(crate::error::DbError::from)
}

/// Release a reservation held by `owner_pubkey` (rollback path).
///
/// Used only when seeding the manifest pointer fails *after* a fresh
/// [`ReserveOutcome::Reserved`], so the announce is all-or-nothing. Scoped to
/// `owner_pubkey` so a rollback can never delete a name a *different* owner
/// concurrently holds. Returns the number of rows removed (0 or 1).
pub async fn release_repo_name(
    pool: &PgPool,
    community: CommunityId,
    repo_id: &str,
    owner_pubkey: &str,
) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM git_repo_names \
         WHERE community_id = $1 AND repo_id = $2 AND owner_pubkey = $3",
    )
    .bind(community.as_uuid())
    .bind(repo_id)
    .bind(owner_pubkey)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn make_test_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("git-repo-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    fn pk() -> String {
        format!("{:064x}", Uuid::new_v4().as_u128())
    }

    /// A fresh name is `Reserved`; re-announcing it as the *same* owner is
    /// `AlreadyOwned` (idempotent) and never grows the owner's count; a
    /// *different* owner is `TakenByOther`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reserve_classifies_fresh_idempotent_and_collision() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let owner = pk();
        let other = pk();
        let repo = format!("repo-{}", Uuid::new_v4().simple());

        assert_eq!(
            reserve_repo_name(&pool, community, &repo, &owner)
                .await
                .expect("fresh reserve"),
            ReserveOutcome::Reserved,
            "first claim of a free name is Reserved"
        );
        assert_eq!(
            reserve_repo_name(&pool, community, &repo, &owner)
                .await
                .expect("re-reserve same owner"),
            ReserveOutcome::AlreadyOwned,
            "same-owner re-announce is idempotent AlreadyOwned"
        );
        assert_eq!(
            reserve_repo_name(&pool, community, &repo, &other)
                .await
                .expect("re-reserve other owner"),
            ReserveOutcome::TakenByOther,
            "a different owner claiming a held name is TakenByOther"
        );
        assert_eq!(
            count_repos_for_owner(&pool, community, &owner)
                .await
                .expect("count owner"),
            1,
            "re-announce must not double-count the owner's quota"
        );
        assert_eq!(
            count_repos_for_owner(&pool, community, &other)
                .await
                .expect("count other"),
            0,
            "a failed (TakenByOther) claim must not count toward the loser's quota"
        );
    }

    /// `repo_name_owner` returns the holder for a reserved name and `None` for a
    /// free one, so the handler can classify before claiming.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn repo_name_owner_reflects_reservation() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let owner = pk();
        let repo = format!("repo-{}", Uuid::new_v4().simple());

        assert!(
            repo_name_owner(&pool, community, &repo)
                .await
                .expect("owner of free name")
                .is_none(),
            "an unreserved name has no owner"
        );
        reserve_repo_name(&pool, community, &repo, &owner)
            .await
            .expect("reserve");
        assert_eq!(
            repo_name_owner(&pool, community, &repo)
                .await
                .expect("owner of reserved name"),
            Some(owner),
            "a reserved name resolves to its owner"
        );
    }

    /// Release is owner-scoped: it removes the reservation only for the holder,
    /// freeing the name for a subsequent claim; a release by a *non*-holder is a
    /// no-op that leaves the reservation intact.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn release_is_owner_scoped_and_frees_the_name() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let owner = pk();
        let stranger = pk();
        let repo = format!("repo-{}", Uuid::new_v4().simple());

        reserve_repo_name(&pool, community, &repo, &owner)
            .await
            .expect("reserve");

        // A non-holder cannot release the name.
        assert_eq!(
            release_repo_name(&pool, community, &repo, &stranger)
                .await
                .expect("stranger release"),
            0,
            "release by a non-holder removes nothing"
        );
        assert_eq!(
            repo_name_owner(&pool, community, &repo)
                .await
                .expect("still owned"),
            Some(owner.clone()),
            "the reservation survives a stranger's release attempt"
        );

        // The holder releases it, freeing the name.
        assert_eq!(
            release_repo_name(&pool, community, &repo, &owner)
                .await
                .expect("owner release"),
            1,
            "the holder's release removes exactly the one row"
        );
        assert_eq!(
            reserve_repo_name(&pool, community, &repo, &stranger)
                .await
                .expect("reclaim after release"),
            ReserveOutcome::Reserved,
            "once released, the name is free for a new owner"
        );
    }

    /// Names are unique *within* a community, not globally: the same repo name
    /// may be independently reserved by different owners in different
    /// communities without collision.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn names_are_scoped_per_community() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let owner_a = pk();
        let owner_b = pk();
        let repo = format!("repo-{}", Uuid::new_v4().simple());

        assert_eq!(
            reserve_repo_name(&pool, community_a, &repo, &owner_a)
                .await
                .expect("reserve in A"),
            ReserveOutcome::Reserved
        );
        assert_eq!(
            reserve_repo_name(&pool, community_b, &repo, &owner_b)
                .await
                .expect("reserve same name in B"),
            ReserveOutcome::Reserved,
            "the same name in a different community is a fresh, independent claim"
        );
        assert_eq!(
            repo_name_owner(&pool, community_a, &repo)
                .await
                .expect("owner in A"),
            Some(owner_a)
        );
        assert_eq!(
            repo_name_owner(&pool, community_b, &repo)
                .await
                .expect("owner in B"),
            Some(owner_b)
        );
    }
}
