const DEFAULT_DATABASE_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

/// Resolve the database URL shared by PostgreSQL-backed unit tests.
pub(crate) fn database_url() -> String {
    std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("TEST_DATABASE_URL"))
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_owned())
}

/// One-session fixture that bypasses ordinary write-fence triggers while
/// retaining application admission. Communities are active, quiescing, fenced.
pub(crate) async fn admission_fixture() -> (crate::Db, [buzz_core::CommunityId; 3]) {
    use buzz_core::CommunityId;
    use uuid::Uuid;

    let db = crate::Db::new(&crate::DbConfig {
        database_url: database_url(),
        max_connections: 1,
        min_connections: 1,
        ..crate::DbConfig::default()
    })
    .await
    .expect("connect test database");
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
    set_deletion_state(pool, *communities[1].as_uuid(), "quiescing").await;
    set_deletion_state(pool, *communities[2].as_uuid(), "fenced").await;
    // The one physical session serves every API call in this fixture.
    sqlx::query("SET session_replication_role = replica")
        .execute(pool)
        .await
        .expect("disable ordinary triggers");
    let role: String = sqlx::query_scalar("SHOW session_replication_role")
        .fetch_one(pool)
        .await
        .expect("read replication role");
    assert_eq!(role, "replica");
    // A raw write to the fenced tenant proves the DB backstop is bypassed.
    sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2)")
        .bind(communities[2].as_uuid())
        .bind(vec![0xfe_u8; 32])
        .execute(pool)
        .await
        .expect("raw fenced insert succeeds with triggers disabled");
    (db, communities)
}

/// In one per-test database, make any return to the retired SQL predicate
/// observable without changing the production function in the public schema.
pub(crate) async fn reject_legacy_sql_fence_predicate(pool: &sqlx::PgPool) -> String {
    let schema = format!("admission_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(pool)
        .await
        .expect("create test-only function schema");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {schema}.community_write_allowed(uuid) RETURNS boolean \
         LANGUAGE plpgsql VOLATILE AS $$ BEGIN \
           RAISE EXCEPTION 'legacy SQL fence predicate called'; END $$",
    )))
    .execute(pool)
    .await
    .expect("shadow legacy SQL predicate");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET search_path = {schema}, public"
    )))
    .execute(pool)
    .await
    .expect("resolve test-local predicate first");
    let shadowed: bool = sqlx::query_scalar(
        "SELECT to_regprocedure('community_write_allowed(uuid)') = to_regprocedure($1)",
    )
    .bind(format!("{schema}.community_write_allowed(uuid)"))
    .fetch_one(pool)
    .await
    .expect("resolve shadowed predicate");
    assert!(shadowed);
    schema
}

/// Hold the executor's exclusive lock until a sweep reaches shared admission,
/// then quiesce in that same transaction before releasing it. The ordinary
/// second pool keeps lifecycle triggers enabled; the sweep fixture bypasses them.
pub(crate) async fn fence_while_sweep_waits<F, T>(community: buzz_core::CommunityId, sweep: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("connect deletion executor pool");
    let mut tx = pool.begin().await.expect("begin deletion executor");
    sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
        .bind(community.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("hold exclusive deletion lock");
    let mut task = tokio::spawn(sweep);
    let waiting = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_locks \
                 WHERE locktype = 'advisory' AND NOT granted \
                   AND database = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                   AND pg_backend_pid() = ANY(pg_blocking_pids(pid)))",
            )
            .fetch_one(&mut *tx)
            .await
            .expect("inspect blocked advisory admission");
            if blocked {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    if waiting.is_err() {
        task.abort();
        panic!("sweep did not block in admission before deadline");
    }
    set_deletion_state_in_transaction(&mut tx, *community.as_uuid(), "quiescing").await;
    tx.commit().await.expect("commit deletion fence");
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), &mut task).await;
    if result.is_err() {
        task.abort();
        panic!("sweep did not finish after deletion fence committed");
    }
    pool.close().await;
    result.unwrap().expect("sweep task")
}

/// Move a community fixture into a deletion lifecycle state the way the
/// executor does: the tombstone trigger only accepts lifecycle changes from a
/// transaction carrying the executor GUCs for that community, including its
/// current fence generation.
pub(crate) async fn set_deletion_state(pool: &sqlx::PgPool, id: uuid::Uuid, state: &str) {
    let mut tx = pool.begin().await.expect("begin lifecycle fixture");
    set_deletion_state_in_transaction(&mut tx, id, state).await;
    tx.commit().await.expect("commit lifecycle fixture");
}

async fn set_deletion_state_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: uuid::Uuid,
    state: &str,
) {
    sqlx::query(
        "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                set_config('buzz.deletion_fence_generation', \
                    (SELECT deletion_fence_generation::text FROM communities WHERE id = $2), true)",
    )
    .bind(id.to_string())
    .bind(id)
    .execute(&mut **tx)
    .await
    .expect("authorize lifecycle fixture");
    sqlx::query(
        "UPDATE communities SET deletion_state = $2, \
                deleted_at = CASE WHEN $2 = 'tombstone' THEN now() END \
         WHERE id = $1",
    )
    .bind(id)
    .bind(state)
    .execute(&mut **tx)
    .await
    .expect("set lifecycle state");
}

/// Move a disposable test community into `quiescing`, the state in which
/// admission must reject new serving writes.
pub(crate) async fn quiesce_community_for_tests(
    pool: &sqlx::PgPool,
    community: buzz_core::CommunityId,
) {
    set_deletion_state(pool, *community.as_uuid(), "quiescing").await;
}

/// Whether `error` is the community-admission rejection taken at transaction
/// entry, as opposed to a later commit-time trigger rejection.
pub(crate) fn is_admission_rejection(error: &crate::DbError) -> bool {
    matches!(error, crate::DbError::AccessDenied(message) if message.contains("write-fenced (quiescing)"))
}

/// The desired-state schema as one SQL script.
///
/// `schema/schema.sql` is a manifest of `\i` includes resolved relative to the
/// including file (as `pgschema` does). This expands every include in place,
/// so text assertions and `sqlx::raw_sql` bootstraps see the same statements,
/// in the same order, that `pgschema apply` loads.
pub(crate) fn desired_state_schema_sql() -> String {
    fn expand(path: &std::path::Path, out: &mut String) {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let dir = path.parent().expect("schema file has a parent directory");
        for line in text.split_inclusive('\n') {
            match line.trim_end().strip_prefix("\\i ") {
                Some(include) => expand(&dir.join(include.trim()), out),
                None => out.push_str(line),
            }
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    let mut out = String::new();
    expand(&desired_state_schema_dir().join("schema.sql"), &mut out);
    out
}

/// `schema/` at the workspace root.
pub(crate) fn desired_state_schema_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema")
}

/// Rewrite every `ON DELETE CASCADE`, `SET NULL` and `SET DEFAULT` foreign key
/// in this test's database as `NO ACTION`: the schema the cascade retirement
/// targets. Every trigger stays live, including the foreign-key checks and the
/// community fence, so a delete path exercised afterwards must remove its
/// children itself, in an order the remaining constraints accept.
///
/// Each PostgreSQL test process owns its database (`crates/buzz-db/TESTING.md`),
/// so the rewrite does not reach other tests. It refuses any database whose
/// name does not match the per-test wrapper's (`buzz_nt_` plus 24 hex digits),
/// which excludes the run's `_desired` template. The name is the only check:
/// a manually created database with that shape would pass.
pub(crate) async fn retire_foreign_key_delete_actions(pool: &sqlx::PgPool) {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await
        .expect("read current database");
    assert!(
        database
            .strip_prefix("buzz_nt_")
            .is_some_and(|hash| hash.len() == 24
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
        "refusing to rewrite foreign keys in shared database {database}; \
         run this test through scripts/postgres-test-run.sh"
    );
    sqlx::raw_sql(
        r#"DO $$
        DECLARE c record;
        BEGIN
          FOR c IN SELECT conrelid::regclass AS tbl, conname, pg_get_constraintdef(oid) AS def
                   FROM pg_constraint
                   WHERE contype = 'f' AND confdeltype IN ('c', 'n', 'd') AND conparentid = 0
          LOOP
            EXECUTE format('ALTER TABLE %s DROP CONSTRAINT %I, ADD CONSTRAINT %I %s',
              c.tbl, c.conname, c.conname,
              regexp_replace(c.def, ' ON DELETE (CASCADE|SET NULL|SET DEFAULT)( \([^)]*\))?', ''));
          END LOOP;
        END $$"#,
    )
    .execute(pool)
    .await
    .expect("retire foreign-key delete actions");
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint WHERE contype = 'f' AND confdeltype IN ('c', 'n', 'd')",
    )
    .fetch_one(pool)
    .await
    .expect("count foreign-key delete actions");
    assert_eq!(remaining, 0, "every foreign-key delete action is retired");
}

/// A single-connection pool whose session carries `application_name`, so
/// [`wait_for_lock_wait`] can tell when its statement is blocked.
pub(crate) async fn named_pool(application_name: &str) -> sqlx::PgPool {
    use std::str::FromStr as _;
    let options = sqlx::postgres::PgConnectOptions::from_str(&database_url())
        .expect("parse test database URL")
        .application_name(application_name);
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect named test pool")
}

/// Wait until the session named `application_name` in this test's database is
/// blocked on a lock.
pub(crate) async fn wait_for_lock_wait(pool: &sqlx::PgPool, application_name: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                 WHERE datname = current_database() AND application_name = $1 \
                   AND wait_event_type = 'Lock')",
            )
            .bind(application_name)
            .fetch_one(pool)
            .await
            .expect("inspect lock wait");
            if waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("session reached a lock wait");
}

/// Loopback TCP proxy that can pause the upstream database, like
/// `docker pause` on a replica. While paused, every session (established or
/// new) holds the bytes it receives with both sockets open; on resume the
/// held bytes are delivered in order and the sessions carry on.
///
/// [`PausingProxy::stall_established`] instead holds only the sessions open
/// at that moment, for good, while later sessions relay normally: a failover
/// whose old sockets stop answering without closing.
/// [`PausingProxy::stall_from`] is the reverse: sessions from a given number
/// on hold for good, so dials past that point stall mid-handshake.
pub(crate) struct PausingProxy {
    pub(crate) port: u16,
    hold: tokio::sync::watch::Sender<ProxyHold>,
    next_session: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// Which proxy sessions hold their bytes.
#[derive(Clone, Copy, Default)]
struct ProxyHold {
    all: bool,
    /// Sessions numbered below this hold their bytes.
    below: u64,
    /// Sessions numbered at or above this hold their bytes.
    from: Option<u64>,
}

impl ProxyHold {
    fn holds(self, session: u64) -> bool {
        self.all || session < self.below || self.from.is_some_and(|from| session >= from)
    }
}

impl PausingProxy {
    pub(crate) async fn spawn(upstream_host: String, upstream_port: u16) -> Self {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        async fn pump(
            mut from: tokio::net::tcp::OwnedReadHalf,
            mut to: tokio::net::tcp::OwnedWriteHalf,
            mut hold: tokio::sync::watch::Receiver<ProxyHold>,
            session: u64,
        ) {
            let mut buf = [0u8; 8192];
            loop {
                let Ok(n) = from.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                if hold.wait_for(|hold| !hold.holds(session)).await.is_err() {
                    return;
                }
                if to.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy");
        let port = listener.local_addr().expect("proxy addr").port();
        let (hold, _) = tokio::sync::watch::channel(ProxyHold::default());
        let accept_hold = hold.subscribe();
        let next_session = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let accept_next = next_session.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let session = accept_next.fetch_add(1, Ordering::SeqCst);
                let Ok(server) =
                    tokio::net::TcpStream::connect((upstream_host.as_str(), upstream_port)).await
                else {
                    continue;
                };
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                tokio::spawn(pump(
                    client_read,
                    server_write,
                    accept_hold.clone(),
                    session,
                ));
                tokio::spawn(pump(
                    server_read,
                    client_write,
                    accept_hold.clone(),
                    session,
                ));
            }
        });
        Self {
            port,
            hold,
            next_session,
        }
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.hold.send_modify(|hold| hold.all = paused);
    }

    /// Hold every session open now, for good; later sessions relay normally.
    pub(crate) fn stall_established(&self) {
        let below = self.next_session.load(std::sync::atomic::Ordering::SeqCst);
        self.hold.send_modify(|hold| hold.below = below);
    }

    /// Relay the first `from` sessions normally and hold every later one,
    /// for good.
    pub(crate) fn stall_from(&self, from: u64) {
        self.hold.send_modify(|hold| hold.from = Some(from));
    }

    /// Sessions accepted so far.
    pub(crate) fn sessions(&self) -> u64 {
        self.next_session.load(std::sync::atomic::Ordering::SeqCst)
    }
}
