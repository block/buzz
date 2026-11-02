const DEFAULT_DATABASE_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

/// Serialize lib tests whose assertions depend on which tracing dispatchers
/// are live: log-capture and callsite-interest tests, and tests that fire the
/// callsites they assert on. tracing-core caches each callsite's interest
/// process-wide from whichever dispatchers are live when it is first hit or
/// rebuilt, so a parallel test's dispatcher (or a thread with none) can enable
/// or disable another test's callsites. Not reentrant; take it once per test.
/// Two async sites install a dispatcher without it, because the guard would be
/// held across `.await`: `router.rs`'s otel-export test and the
/// `pause_after_aux_page` helper in
/// `api/bridge/thread_window/postgres_tests/failure_postgres_tests.rs`.
pub(crate) fn tracing_dispatch_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Resolve the database URL shared by PostgreSQL-backed relay tests.
pub(crate) fn database_url() -> String {
    std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("TEST_DATABASE_URL"))
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_owned())
}

#[cfg(test)]
const CHILD_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(test)]
const MAX_CAPTURE_BYTES: u64 = 1024 * 1024;

#[cfg(test)]
struct CapturedStream {
    retained: Vec<u8>,
    total_bytes: u64,
}

#[cfg(test)]
fn capture_stream(mut stream: impl std::io::Read) -> CapturedStream {
    let mut retained = Vec::new();
    let mut total_bytes = 0_u64;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = stream.read(&mut chunk).expect("read child output pipe");
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(u64::try_from(read).expect("read size fits u64"));
        let remaining = usize::try_from(MAX_CAPTURE_BYTES)
            .expect("capture ceiling fits usize")
            .saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    CapturedStream {
        retained,
        total_bytes,
    }
}

#[cfg(test)]
fn join_capture(capture: std::thread::JoinHandle<CapturedStream>, stream: &str) -> Vec<u8> {
    let capture = capture.join().expect("child capture thread must not panic");
    assert!(
        capture.total_bytes <= MAX_CAPTURE_BYTES,
        "child {stream} exceeded {MAX_CAPTURE_BYTES} bytes: {}",
        capture.total_bytes,
    );
    capture.retained
}

/// Run exactly one unit test in an isolated, deadline-bounded child process.
#[cfg(test)]
pub(crate) fn run_exact_test_child(test_name: &str, child_env: &str) {
    use std::{
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    let mut child = Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env(child_env, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn isolated test child");
    let stdout = child.stdout.take().expect("child stdout pipe");
    let stderr = child.stderr.take().expect("child stderr pipe");
    let stdout = thread::spawn(move || capture_stream(stdout));
    let stderr = thread::spawn(move || capture_stream(stderr));

    let deadline = Instant::now() + CHILD_TEST_TIMEOUT;
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().expect("poll isolated test child") {
            break (status, false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let status = child.wait().expect("reap timed-out test child");
            break (status, true);
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout = join_capture(stdout, "stdout");
    let stderr = join_capture(stderr, "stderr");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );

    assert!(
        !timed_out,
        "isolated test child exceeded {CHILD_TEST_TIMEOUT:?}:\n{output}"
    );
    assert!(status.success(), "isolated test child failed:\n{output}");
    assert!(
        output.contains("running 1 test") && output.contains(test_name),
        "exact selector did not run the intended test {test_name}:\n{output}"
    );
}

/// Loopback TCP proxy that can pause the upstream database. While paused,
/// every session holds the bytes it receives with both sockets open; on
/// resume the held bytes are delivered in order and the sessions carry on.
/// A trimmed copy of buzz-db's test-only `PausingProxy`, which is private to
/// that crate's unit tests.
#[cfg(test)]
pub(crate) struct PausingProxy {
    pub(crate) port: u16,
    paused: tokio::sync::watch::Sender<bool>,
}

#[cfg(test)]
impl PausingProxy {
    pub(crate) async fn spawn(upstream_host: String, upstream_port: u16) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        async fn pump(
            mut from: tokio::net::tcp::OwnedReadHalf,
            mut to: tokio::net::tcp::OwnedWriteHalf,
            mut paused: tokio::sync::watch::Receiver<bool>,
        ) {
            let mut buf = [0u8; 8192];
            loop {
                let Ok(n) = from.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                if paused.wait_for(|paused| !paused).await.is_err() {
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
        let (paused, accept_paused) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let Ok(server) =
                    tokio::net::TcpStream::connect((upstream_host.as_str(), upstream_port)).await
                else {
                    continue;
                };
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                tokio::spawn(pump(client_read, server_write, accept_paused.clone()));
                tokio::spawn(pump(server_read, client_write, accept_paused.clone()));
            }
        });
        Self { port, paused }
    }

    /// Hold or release every session's bytes. A pause applies to all bytes
    /// the proxy reads after this call returns, on any runtime thread.
    pub(crate) fn set_paused(&self, paused: bool) {
        self.paused.send_replace(paused);
    }
}

/// A database handle on which only restriction-state reads fail: its
/// `search_path` puts a schema first whose `community_bans` is a view that
/// errors when read, so every other table resolves to `public` as normal.
/// Returns the handle, an admin pool, and the schema to drop afterwards.
#[cfg(test)]
pub(crate) async fn restriction_lookup_failing_db() -> (buzz_db::Db, sqlx::PgPool, String) {
    use sqlx::postgres::PgConnectOptions;
    let url = database_url();
    let admin = sqlx::PgPool::connect(&url)
        .await
        .expect("PostgreSQL must be available");
    let schema = format!("ban_lookup_fails_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE SCHEMA {schema}; \
         CREATE VIEW {schema}.community_bans AS \
         SELECT * FROM public.community_bans WHERE 1 / (SELECT count(*) * 0 FROM public.communities) = 1;"
    )))
    .execute(&admin)
    .await
    .expect("create failing restriction schema");
    let options = url
        .parse::<PgConnectOptions>()
        .expect("database url")
        .options([("search_path", format!("{schema}, public"))]);
    let pool = sqlx::PgPool::connect_with(options)
        .await
        .expect("failing restriction pool");
    (buzz_db::Db::from_pool(pool), admin, schema)
}
