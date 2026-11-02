//! Per-community NIP-FI issuer bindings (`community_nip_fi_issuers`).
//!
//! Operator-plane persistence only: creation paths, the backfill, and the
//! operator lookup route write and read these rows. The assertion audience of
//! a bound community is `https://<communities.host>`.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use sqlx::{Postgres, Row, Transaction};

use crate::{CommunityRecord, Db, Result};

/// Outcome of [`Db::unbind_nip_fi_issuer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NipFiUnbind {
    /// The issuer was removed.
    Removed,
    /// The issuer was not bound to the community.
    NotBound,
    /// The issuer is the community's last one and was kept.
    LastIssuer,
}

/// A community's stored NIP-FI binding, as returned to the operator route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NipFiBindingRecord {
    /// Canonical host stored on the community row.
    pub host: String,
    /// True when the community is not archived and its deletion has not
    /// started quiescing. A community whose deletion was requested but not yet
    /// quiesced still reports true.
    pub active: bool,
    /// Bound issuers in byte order. Empty means no binding.
    pub issuers: Vec<String>,
}

impl Db {
    /// Insert `issuers` for `community` inside the caller's transaction.
    /// Existing rows are left as they are. No-op for an empty list.
    pub(crate) async fn insert_nip_fi_issuers_in_transaction(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: CommunityId,
        issuers: &[String],
    ) -> Result<u64> {
        if issuers.is_empty() {
            return Ok(0);
        }
        self.deletion_store()
            .guard_transaction(tx, community)
            .await?;
        let inserted = sqlx::query(
            r#"
            INSERT INTO community_nip_fi_issuers (community_id, issuer)
            SELECT $1, issuer FROM unnest($2::text[]) AS issuer
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(community.as_uuid())
        .bind(issuers)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        Ok(inserted)
    }

    /// Bind `issuers` to an existing community. Returns the number of new rows.
    #[datastore_span(name = "bind_nip_fi_issuers", system = "postgresql")]
    pub async fn bind_nip_fi_issuers(
        &self,
        community: CommunityId,
        issuers: &[String],
    ) -> Result<u64> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = Transaction::begin(connection, None).await?;
        // Deletion lock before the row lock, the order deletion takes them.
        self.deletion_store()
            .guard_transaction(&mut tx, community)
            .await?;
        // Serializes with the backfill, which locks the same row.
        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR NO KEY UPDATE")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await?;
        let inserted = self
            .insert_nip_fi_issuers_in_transaction(&mut tx, community, issuers)
            .await?;
        tx.commit().await?;
        Ok(inserted)
    }

    /// Remove one issuer from a community, refusing to remove its last one
    /// unless `allow_last`.
    ///
    /// The bound rows are locked first, so two concurrent unbinds of a
    /// community's two issuers cannot both succeed and leave it empty. An
    /// empty community looks never-provisioned and the next backfill would
    /// rebind it; archive or delete the community to stop it logging in.
    #[datastore_span(name = "unbind_nip_fi_issuer", system = "postgresql")]
    pub async fn unbind_nip_fi_issuer(
        &self,
        community: CommunityId,
        issuer: &str,
        allow_last: bool,
    ) -> Result<NipFiUnbind> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = Transaction::begin(connection, None).await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community)
            .await?;
        let bound: Vec<String> = sqlx::query_scalar(
            "SELECT issuer FROM community_nip_fi_issuers WHERE community_id = $1 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        let outcome = if !bound.iter().any(|bound| bound == issuer) {
            NipFiUnbind::NotBound
        } else if bound.len() == 1 && !allow_last {
            NipFiUnbind::LastIssuer
        } else {
            sqlx::query(
                "DELETE FROM community_nip_fi_issuers WHERE community_id = $1 AND issuer = $2",
            )
            .bind(community.as_uuid())
            .bind(issuer)
            .execute(&mut *tx)
            .await?;
            NipFiUnbind::Removed
        };
        tx.commit().await?;
        Ok(outcome)
    }

    /// Read a community's binding by host, regardless of lifecycle state.
    /// `None` when no community has this host.
    #[datastore_span(name = "lookup_nip_fi_binding_by_host", system = "postgresql")]
    pub async fn lookup_nip_fi_binding_by_host(
        &self,
        normalized_host: &str,
    ) -> Result<Option<NipFiBindingRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let row = sqlx::query(
            r#"
            SELECT c.host,
                   (c.archived_at IS NULL
                    AND c.deletion_state = 'active'
                    AND c.deleted_at IS NULL) AS active,
                   COALESCE(
                       array_agg(b.issuer ORDER BY b.issuer COLLATE "C")
                           FILTER (WHERE b.issuer IS NOT NULL),
                       '{}'
                   ) AS issuers
            FROM communities c
            LEFT JOIN community_nip_fi_issuers b ON b.community_id = c.id
            WHERE lower(c.host) = lower($1)
            GROUP BY c.id
            "#,
        )
        .bind(normalized_host)
        .fetch_optional(&mut *connection)
        .await?;
        row.map(|row| {
            Ok(NipFiBindingRecord {
                host: row.try_get("host")?,
                active: row.try_get("active")?,
                issuers: row.try_get("issuers")?,
            })
        })
        .transpose()
    }

    /// Up to `limit` active communities with no binding whose host ends in
    /// `.<host_suffix>` and sorts after `after`, in host order. Callers apply
    /// the exact one-label rule. Archived communities are included on purpose:
    /// unarchiving must not leave a community that cannot log in.
    #[datastore_span(name = "list_unbound_nip_fi_communities", system = "postgresql")]
    pub async fn list_unbound_nip_fi_communities(
        &self,
        host_suffix: &str,
        after: &str,
        limit: i64,
    ) -> Result<Vec<CommunityRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query(
            r#"
            SELECT c.id, c.host
            FROM communities c
            WHERE c.host COLLATE "C" > $2
              AND right(lower(c.host), length($1) + 1) = '.' || lower($1)
              AND c.deletion_state = 'active'
              AND c.deleted_at IS NULL
              AND NOT EXISTS (
                  SELECT 1 FROM community_nip_fi_issuers b WHERE b.community_id = c.id
              )
            ORDER BY c.host COLLATE "C"
            LIMIT $3
            "#,
        )
        .bind(host_suffix)
        .bind(after)
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(CommunityRecord {
                    id: CommunityId::from_uuid(row.try_get("id")?),
                    host: row.try_get("host")?,
                })
            })
            .collect()
    }

    /// Bind `issuers` to each of `communities` that is still active and still
    /// has no binding, in one statement. Takes each community's shared deletion
    /// lock first, so a community whose deletion has started is skipped rather
    /// than aborting the batch. The community rows are then locked, as
    /// [`Db::bind_nip_fi_issuers`] does, so an operator bind that commits after
    /// the caller listed the community wins and the default is not added
    /// beside it. Returns the number of new rows.
    #[datastore_span(name = "bind_nip_fi_issuers_to_active", system = "postgresql")]
    pub async fn bind_nip_fi_issuers_to_active(
        &self,
        communities: &[CommunityId],
        issuers: &[String],
    ) -> Result<u64> {
        if communities.is_empty() || issuers.is_empty() {
            return Ok(0);
        }
        let ids: Vec<uuid::Uuid> = communities.iter().map(|id| *id.as_uuid()).collect();
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = Transaction::begin(connection, None).await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock_shared(community_deletion_lock_key(id)) \
             FROM unnest($1::uuid[]) AS id ORDER BY id",
        )
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
        sqlx::query("SELECT 1 FROM communities WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE")
            .bind(&ids)
            .execute(&mut *tx)
            .await?;
        let inserted = sqlx::query(
            r#"
            INSERT INTO community_nip_fi_issuers (community_id, issuer)
            SELECT c.id, issuer
            FROM communities c CROSS JOIN unnest($2::text[]) AS issuer
            WHERE c.id = ANY($1)
              AND c.deletion_state = 'active'
              AND c.deleted_at IS NULL
              AND NOT EXISTS (
                  SELECT 1 FROM community_nip_fi_issuers b WHERE b.community_id = c.id
              )
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(&ids)
        .bind(issuers)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(inserted)
    }

    /// The subset of `hosts` whose community holds at least one binding.
    #[datastore_span(name = "nip_fi_bound_hosts", system = "postgresql")]
    pub async fn nip_fi_bound_hosts(&self, hosts: &[String]) -> Result<Vec<String>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Bootstrap,
        )
        .await?;
        let bound = sqlx::query_scalar(
            r#"
            SELECT c.host
            FROM communities c
            WHERE lower(c.host) = ANY(SELECT lower(h) FROM unnest($1::text[]) AS h)
              AND EXISTS (
                  SELECT 1 FROM community_nip_fi_issuers b WHERE b.community_id = c.id
              )
            ORDER BY c.host
            "#,
        )
        .bind(hosts)
        .fetch_all(&mut *connection)
        .await?;
        Ok(bound)
    }
}

/// A snapshot load produced more rows than its cap allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("row cap of {0} exceeded")]
pub struct RowCapExceeded(pub usize);

impl Db {
    /// Read every binding of an active community to an issuer in `registry`,
    /// on the primary, as `(host, issuer)` rows, one per community-issuer
    /// pair. It is one `SELECT`, so every row comes from the same statement
    /// snapshot. The read stops at the `max_rows + 1`st row and fails.
    #[datastore_span(name = "load_nip_fi_binding_snapshot", system = "postgresql")]
    pub async fn load_nip_fi_binding_snapshot(
        &self,
        registry: &[String],
        max_rows: usize,
    ) -> Result<std::result::Result<Vec<(String, String)>, RowCapExceeded>> {
        use futures_util::StreamExt;
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut stream = sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT c.host, b.issuer
            FROM community_nip_fi_issuers b
            JOIN communities c ON c.id = b.community_id
            WHERE b.issuer = ANY($1)
              AND c.archived_at IS NULL
              AND c.deletion_state = 'active'
              AND c.deleted_at IS NULL
            "#,
        )
        .bind(registry)
        .fetch(&mut *connection);
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            if rows.len() == max_rows {
                return Ok(Err(RowCapExceeded(max_rows)));
            }
            rows.push(row?);
        }
        Ok(Ok(rows))
    }

    /// The registry-filtered issuers bound to the active community at
    /// `normalized_host`, on the primary. Empty when the host is unknown,
    /// inactive, or bound only to unregistered issuers. The primary key
    /// `(community_id, issuer)` bounds the result at `registry.len()` rows.
    #[datastore_span(name = "lookup_nip_fi_admission_binding", system = "postgresql")]
    pub async fn lookup_nip_fi_admission_binding(
        &self,
        normalized_host: &str,
        registry: &[String],
    ) -> Result<Vec<String>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let issuers = sqlx::query_scalar::<_, String>(
            r#"
            SELECT b.issuer
            FROM community_nip_fi_issuers b
            JOIN communities c ON c.id = b.community_id
            WHERE lower(c.host) = lower($1)
              AND b.issuer = ANY($2)
              AND c.archived_at IS NULL
              AND c.deletion_state = 'active'
              AND c.deleted_at IS NULL
            "#,
        )
        .bind(normalized_host)
        .bind(registry)
        .fetch_all(&mut *connection)
        .await?;
        Ok(issuers)
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::CreateCommunityWithOwnerResult;
    use sqlx::PgPool;
    use uuid::Uuid;

    const ISS: &str = "https://issuer.example/api/goose";
    const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    async fn setup_db() -> Db {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        Db::from_pool(pool)
    }

    fn unique_host(prefix: &str) -> String {
        format!("{prefix}-{}.bind.example", Uuid::new_v4().simple())
    }

    fn issuers() -> Vec<String> {
        vec![ISS.to_string()]
    }

    async fn bound(db: &Db, host: &str) -> Option<Vec<String>> {
        db.lookup_nip_fi_binding_by_host(host)
            .await
            .expect("lookup binding")
            .map(|record| record.issuers)
    }

    async fn community_exists(db: &Db, host: &str) -> bool {
        db.lookup_community_by_host_for_management(host)
            .await
            .expect("lookup community")
            .is_some()
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_to_active_skips_a_community_whose_deletion_started() {
        let db = setup_db().await;
        let deleting = db
            .ensure_configured_community(&unique_host("deleting"))
            .await
            .expect("seed")
            .id;
        let control_host = unique_host("control");
        let control = db
            .ensure_configured_community(&control_host)
            .await
            .expect("seed")
            .id;
        crate::test_support::quiesce_community_for_tests(&db.pool, deleting).await;
        assert_eq!(
            db.bind_nip_fi_issuers_to_active(&[deleting, control], &issuers())
                .await
                .expect("a deleting community is skipped, not an error"),
            1
        );
        assert_eq!(bound(&db, &control_host).await, Some(issuers()));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unbind_refuses_the_last_issuer() {
        let db = setup_db().await;
        let host = unique_host("unbind");
        let other = "https://other.example".to_string();
        let community = db
            .ensure_configured_community_with_nip_fi_issuers(
                &host,
                &[ISS.to_string(), other.clone()],
            )
            .await
            .expect("seed")
            .id;
        assert_eq!(
            db.unbind_nip_fi_issuer(community, "https://unbound.example", false)
                .await
                .expect("unbind unknown"),
            NipFiUnbind::NotBound
        );
        assert_eq!(
            db.unbind_nip_fi_issuer(community, &other, false)
                .await
                .expect("unbind"),
            NipFiUnbind::Removed
        );
        assert_eq!(
            db.unbind_nip_fi_issuer(community, ISS, false)
                .await
                .expect("unbind last"),
            NipFiUnbind::LastIssuer
        );
        assert_eq!(bound(&db, &host).await, Some(issuers()));
        assert_eq!(
            db.unbind_nip_fi_issuer(community, ISS, true)
                .await
                .expect("unbind last when allowed"),
            NipFiUnbind::Removed
        );
        assert_eq!(bound(&db, &host).await, Some(Vec::new()));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn create_with_owner_binds_in_the_insert_transaction() {
        let db = setup_db().await;
        let host = unique_host("create");
        let created = db
            .create_community_with_owner_and_nip_fi_issuers(
                &host,
                OWNER,
                &issuers(),
                &nostr::Keys::generate(),
            )
            .await
            .map(|write| write.outcome)
            .expect("create");
        assert!(matches!(
            created,
            CreateCommunityWithOwnerResult::Created(_)
        ));
        assert_eq!(bound(&db, &host).await, Some(issuers()));

        // An identical retry returns the record without new rows.
        db.create_community_with_owner_and_nip_fi_issuers(
            &host,
            OWNER,
            &["https://other.example".to_string()],
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("retry");
        assert_eq!(bound(&db, &host).await, Some(issuers()));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn ensure_binds_only_when_it_creates_the_row() {
        let db = setup_db().await;
        let host = unique_host("ensure");
        let first = db
            .ensure_configured_community_with_nip_fi_issuers(&host, &issuers())
            .await
            .expect("ensure");
        assert!(first.created);
        assert_eq!(bound(&db, &host).await, Some(issuers()));

        let again = db
            .ensure_configured_community_with_nip_fi_issuers(
                &host,
                &["https://other.example".to_string()],
            )
            .await
            .expect("ensure existing");
        assert!(!again.created);
        assert_eq!(bound(&db, &host).await, Some(issuers()));

        let bootstrap = unique_host("bootstrap");
        db.ensure_configured_community_for_bootstrap(&bootstrap, &issuers())
            .await
            .expect("bootstrap");
        assert_eq!(bound(&db, &bootstrap).await, Some(issuers()));
        let unbound = unique_host("bootstrap-unbound");
        db.ensure_configured_community_for_bootstrap(&unbound, &[])
            .await
            .expect("bootstrap unbound");
        assert_eq!(bound(&db, &unbound).await, Some(Vec::new()));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn binding_insert_failure_rolls_back_the_community_on_every_path() {
        let db = setup_db().await;
        // An empty issuer violates the table's CHECK, failing the binding insert.
        let invalid = vec![String::new()];

        let create = unique_host("fail-create");
        assert!(db
            .create_community_with_owner_and_nip_fi_issuers(
                &create,
                OWNER,
                &invalid,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
            .is_err());
        assert!(!community_exists(&db, &create).await);

        let ensure = unique_host("fail-ensure");
        assert!(db
            .ensure_configured_community_with_nip_fi_issuers(&ensure, &invalid)
            .await
            .is_err());
        assert!(!community_exists(&db, &ensure).await);

        let bootstrap = unique_host("fail-bootstrap");
        assert!(db
            .ensure_configured_community_for_bootstrap(&bootstrap, &invalid)
            .await
            .is_err());
        assert!(!community_exists(&db, &bootstrap).await);
    }

    // Deletion takes its exclusive lock, then the community row. A bind that
    // held the row while waiting for that lock would deadlock with it.
    // Mutation: locking the row before `guard_transaction` in
    // `bind_nip_fi_issuers` aborts one side with a deadlock.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn operator_bind_takes_locks_in_deletion_order() {
        let db = setup_db().await;
        let id = db
            .ensure_configured_community(&unique_host("lock-order"))
            .await
            .expect("seed")
            .id;
        let mut deletion = db.pool.begin().await.expect("begin deletion");
        let deletion_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *deletion)
            .await
            .expect("pid");
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(id.as_uuid())
            .execute(&mut *deletion)
            .await
            .expect("exclusive deletion lock");

        let bind = tokio::spawn({
            let db = db.clone();
            async move { db.bind_nip_fi_issuers(id, &issuers()).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(deletion_pid)
            .fetch_one(&db.pool)
            .await
            .expect("observe bind blocked")
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("bind must wait for the deletion lock");

        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE")
            .bind(id.as_uuid())
            .execute(&mut *deletion)
            .await
            .expect("deletion takes the community row");
        deletion.commit().await.expect("commit deletion");
        assert_eq!(bind.await.expect("join").expect("bind"), 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn backfill_keeps_only_an_operator_bind_made_after_it_listed() {
        let db = setup_db().await;
        let suffix = format!("{}.suffix.example", Uuid::new_v4().simple());
        let host = format!("acme.{suffix}");
        db.ensure_configured_community(&host).await.expect("seed");
        let listed = db
            .list_unbound_nip_fi_communities(&suffix, "", 1000)
            .await
            .expect("list unbound");
        assert_eq!(listed.len(), 1);

        let operator = "https://operator.example".to_string();
        db.bind_nip_fi_issuers(listed[0].id, std::slice::from_ref(&operator))
            .await
            .expect("operator bind between list and insert");
        assert_eq!(
            db.bind_nip_fi_issuers_to_active(&[listed[0].id], &issuers())
                .await
                .expect("backfill insert"),
            0
        );
        assert_eq!(bound(&db, &host).await, Some(vec![operator]));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn backfill_queries_find_and_bind_unbound_suffix_hosts() {
        let db = setup_db().await;
        let suffix = format!("{}.suffix.example", Uuid::new_v4().simple());
        let unbound = format!("acme.{suffix}");
        let already = format!("bound.{suffix}");
        let elsewhere = unique_host("elsewhere");
        for (host, issuers) in [
            (&unbound, Vec::new()),
            (&already, issuers()),
            (&elsewhere, Vec::new()),
        ] {
            db.ensure_configured_community_with_nip_fi_issuers(host, &issuers)
                .await
                .expect("seed");
        }

        let found = db
            .list_unbound_nip_fi_communities(&suffix, "", 1000)
            .await
            .expect("list unbound");
        assert_eq!(
            found.iter().map(|c| c.host.as_str()).collect::<Vec<_>>(),
            [unbound.as_str()]
        );
        assert!(db
            .list_unbound_nip_fi_communities(&suffix, &unbound, 1000)
            .await
            .expect("page after the last host")
            .is_empty());
        let ids = [found[0].id];
        assert_eq!(
            db.bind_nip_fi_issuers_to_active(&ids, &issuers())
                .await
                .expect("bind"),
            1
        );
        assert_eq!(
            db.bind_nip_fi_issuers_to_active(&ids, &issuers())
                .await
                .expect("rebind"),
            0
        );
        assert!(db
            .list_unbound_nip_fi_communities(&suffix, "", 1000)
            .await
            .expect("relist")
            .is_empty());

        assert_eq!(
            db.nip_fi_bound_hosts(&[unbound.to_uppercase(), elsewhere.clone()])
                .await
                .expect("bound hosts"),
            std::slice::from_ref(&unbound)
        );
        assert_eq!(bound(&db, &unique_host("missing")).await, None);
    }

    /// Move `host`'s community out of `active` the way the deletion executor
    /// does: the tombstone trigger admits the state change only under the
    /// executor's community and fence-generation settings.
    async fn start_deleting(db: &Db, host: &str) {
        let mut tx = db.pool.begin().await.expect("begin");
        let id: Uuid = sqlx::query_scalar("SELECT id FROM communities WHERE host = $1")
            .bind(host)
            .fetch_one(&mut *tx)
            .await
            .expect("community id");
        sqlx::query("SELECT set_config('buzz.deletion_executor_community', $1, true), set_config('buzz.deletion_fence_generation', '1', true)")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .expect("executor settings");
        sqlx::query("UPDATE communities SET deletion_state = 'quiescing', deletion_fence_generation = 1 WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .expect("start deletion");
        tx.commit().await.expect("commit");
    }

    async fn snapshot_hosts(db: &Db, registry: &[String]) -> Vec<String> {
        db.load_nip_fi_binding_snapshot(registry, 100_000)
            .await
            .expect("snapshot")
            .expect("under cap")
            .into_iter()
            .map(|(host, _)| host)
            .collect()
    }

    async fn admission(db: &Db, host: &str, registry: &[String]) -> Vec<String> {
        db.lookup_nip_fi_admission_binding(host, registry)
            .await
            .expect("admission lookup")
    }

    // Criterion 14(a): archived and deleting communities drop out of both the
    // snapshot load and the admission miss lookup; an active control stays.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn snapshot_and_admission_lookup_skip_archived_and_deleting_communities() {
        let db = setup_db().await;
        let registry = issuers();
        let [archived, deleting, control] = ["archived", "deleting", "control"].map(unique_host);
        for host in [&archived, &deleting, &control] {
            db.ensure_configured_community_with_nip_fi_issuers(host, &registry)
                .await
                .expect("seed");
            assert!(snapshot_hosts(&db, &registry).await.contains(host));
            assert_eq!(admission(&db, host, &registry).await, registry);
        }

        sqlx::query("UPDATE communities SET archived_at = now() WHERE host = $1")
            .bind(&archived)
            .execute(&db.pool)
            .await
            .expect("archive");
        start_deleting(&db, &deleting).await;

        let hosts = snapshot_hosts(&db, &registry).await;
        for gone in [&archived, &deleting] {
            assert!(!hosts.contains(gone), "{gone} must leave the snapshot");
            assert!(admission(&db, gone, &registry).await.is_empty(), "{gone}");
        }
        assert!(hosts.contains(&control));
        assert_eq!(admission(&db, &control, &registry).await, registry);
    }

    // Criterion 17(a)(1): only registered issuers come back, and a host bound
    // only to unregistered issuers yields zero rows from the real SQL.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn admission_lookup_returns_only_registered_issuers() {
        let db = setup_db().await;
        let other = "https://unregistered.example".to_string();
        let mixed = unique_host("mixed");
        let foreign = unique_host("foreign");
        db.ensure_configured_community_with_nip_fi_issuers(
            &mixed,
            &[ISS.to_string(), other.clone()],
        )
        .await
        .expect("seed mixed");
        db.ensure_configured_community_with_nip_fi_issuers(&foreign, std::slice::from_ref(&other))
            .await
            .expect("seed foreign");

        assert_eq!(
            admission(&db, &mixed.to_uppercase(), &issuers()).await,
            issuers()
        );
        assert!(admission(&db, &foreign, &issuers()).await.is_empty());
        assert!(!snapshot_hosts(&db, &issuers()).await.contains(&foreign));
    }

    // The load fails at the `max_rows + 1`st row instead of truncating.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn snapshot_load_over_its_row_cap_fails() {
        let db = setup_db().await;
        let registry = vec![format!("https://issuer-{}.example", uuid::Uuid::new_v4())];
        for prefix in ["cap-a", "cap-b"] {
            db.ensure_configured_community_with_nip_fi_issuers(&unique_host(prefix), &registry)
                .await
                .expect("seed");
        }
        assert_eq!(
            db.load_nip_fi_binding_snapshot(&registry, 1)
                .await
                .expect("load"),
            Err(RowCapExceeded(1))
        );
        assert_eq!(
            db.load_nip_fi_binding_snapshot(&registry, 2)
                .await
                .expect("load")
                .expect("at cap")
                .len(),
            2
        );
    }
}
