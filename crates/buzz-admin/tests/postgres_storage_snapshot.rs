//! Run the real command through delayed/broken PostgreSQL connections.
//! The PostgreSQL nextest wrapper supplies a separate database per test.

use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{sleep, timeout};
use url::Url;

struct Server {
    address: std::net::SocketAddr,
    requests: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn postgres_proxy(database_url: &str, reject: usize, delay: Duration) -> Server {
    let target = Url::parse(database_url).expect("test database URL");
    let target = (
        target.host_str().expect("database host").to_owned(),
        target.port().unwrap_or(5432),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let address = listener.local_addr().expect("proxy address");
    let requests = Arc::new(AtomicUsize::new(0));
    let connections = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        let mut sessions = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut client, _) = accepted.expect("accept database connection");
                    let index = connections.fetch_add(1, Ordering::SeqCst);
                    let target = target.clone();
                    sessions.spawn(async move {
                        if index < reject {
                            // Wait for the client handshake before dropping the
                            // connection, so this is an I/O failure, not a refused
                            // connection SQLx would retry inside the same attempt.
                            let mut header = [0; 4];
                            let _ = client.read_exact(&mut header).await;
                            return;
                        }
                        sleep(delay).await;
                        let mut upstream = TcpStream::connect(target).await.expect("real Postgres");
                        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                    });
                }
                result = sessions.join_next(), if !sessions.is_empty() => {
                    result.expect("session").expect("proxy session completed");
                }
            }
        }
    });
    Server {
        address,
        requests,
        task,
    }
}

async fn s3_listing() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("S3 listener");
    let address = listener.local_addr().expect("S3 address");
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        let sha = "a".repeat(64);
        let contents: String = [
            (format!("{sha}.bin"), 100),
            (format!("{sha}.thumb.jpg"), 10),
            (
                format!("_meta/00000000-0000-0000-0000-000000000001/{sha}.json"),
                30,
            ),
        ]
        .into_iter()
        .map(|(key, size)| {
            format!(
                "<Contents><Key>{key}</Key><Size>{size}</Size>\
                 <LastModified>2026-09-20T00:00:00.000Z</LastModified>\
                 <ETag>\"test\"</ETag><StorageClass>STANDARD</StorageClass></Contents>"
            )
        })
        .collect();
        let body = format!(
            "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
             <Name>test-bucket</Name><Prefix></Prefix><MaxKeys>1000</MaxKeys>\
             <IsTruncated>false</IsTruncated>{contents}\
             </ListBucketResult>"
        );
        loop {
            let (mut stream, _) = listener.accept().await.expect("S3 connection");
            observed.fetch_add(1, Ordering::SeqCst);
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).await.expect("S3 request");
                assert!(read > 0 && request.len() + read <= 16 * 1024);
                request.extend_from_slice(&chunk[..read]);
            }
            assert!(String::from_utf8_lossy(&request).contains("list-type=2"));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("S3 response");
        }
    });
    Server {
        address,
        requests,
        task,
    }
}

async fn database() -> (String, PgPool) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .expect("run through scripts/postgres-test-run.sh for an isolated database");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("test DB");
    sqlx::query(
        "INSERT INTO storage_accounting_snapshots \
         (singleton, snapshot, completed_at, duration_ms, max_objects, code_sha) \
         VALUES (TRUE, $1, NOW(), 1, 100, 'before')",
    )
    .bind(json!({"previous": "complete"}))
    .execute(&pool)
    .await
    .expect("seed last-good snapshot");
    (url, pool)
}

async fn capture(reader: impl AsyncRead + Unpin) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    reader.take(64 * 1024).read_to_end(&mut bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

struct WorkerOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

async fn run_worker(database_url: &str, proxy: &Server, s3: &Server) -> WorkerOutput {
    let mut url = Url::parse(database_url).expect("database URL");
    url.set_host(Some("127.0.0.1")).expect("proxy host");
    url.set_port(Some(proxy.address.port()))
        .expect("proxy port");
    // The proxy deliberately interrupts the PostgreSQL handshake. Keep TLS
    // out of this local transport test so it exercises sqlx::Error::Io.
    url.query_pairs_mut().append_pair("sslmode", "disable");
    // nextest relocates executables when extracting a test archive. Keep the
    // compile-time Cargo path only as the fallback for local cargo test runs.
    let worker_binary = std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into());
    let mut child = Command::new(worker_binary)
        .args(["storage-snapshot", "--max-objects", "100"])
        .env_clear()
        .env("DATABASE_URL", url.as_str())
        .env("BUZZ_S3_ENDPOINT", format!("http://{}", s3.address))
        .env("BUZZ_S3_BUCKET", "test-bucket")
        .env("BUZZ_S3_REGION", "us-east-1")
        .env("BUZZ_S3_ACCESS_KEY", "test-access-key")
        .env("BUZZ_S3_SECRET_KEY", "test-secret-key")
        .env("BUZZ_STORAGE_SNAPSHOT_CODE_SHA", "startup-test")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("start real worker");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let completed = timeout(Duration::from_secs(25), async {
        tokio::try_join!(child.wait(), capture(stdout), capture(stderr))
    })
    .await;
    let (status, stdout, stderr) = match completed {
        Ok(output) => output.expect("worker output"),
        Err(_) => {
            child.kill().await.expect("kill hung worker");
            panic!("worker did not finish within 25 seconds");
        }
    };
    WorkerOutput {
        status,
        stdout,
        stderr,
    }
}

fn events(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn storage_snapshot_retries_then_connects_after_the_old_three_second_budget() {
    let (url, pool) = database().await;
    let proxy = postgres_proxy(&url, 1, Duration::from_secs(5)).await;
    let s3 = s3_listing().await;
    let output = run_worker(&url, &proxy, &s3).await;
    assert!(
        output.status.success(),
        "{}\n{}",
        output.stdout,
        output.stderr
    );
    assert_eq!(
        proxy.requests.load(Ordering::SeqCst),
        2,
        "one rejected connection, one lock-owning session, no spare pool connections"
    );
    assert_eq!(s3.requests.load(Ordering::SeqCst), 1);
    let startup = events(&output.stderr);
    assert!(startup.iter().any(
        |event| event["event"] == "storage_snapshot_db_connect_failed"
            && event["attempt"] == 1
            && event["retry_in_ms"] == 2000
    ));
    assert!(startup.iter().any(
        |event| event["event"] == "storage_snapshot_db_connect_completed"
            && event["attempt"] == 2
            && event["attempt_elapsed_ms"].as_u64().unwrap() >= 5000
    ));
    let scan = events(&output.stdout);
    assert_eq!(scan.first().unwrap()["event"], "storage_snapshot_started");
    assert_eq!(scan.last().unwrap()["event"], "storage_snapshot_completed");
    let (snapshot, revision): (Value, String) = sqlx::query_as(
        "SELECT snapshot, code_sha FROM storage_accounting_snapshots WHERE singleton = TRUE",
    )
    .fetch_one(&pool)
    .await
    .expect("saved snapshot");
    assert_eq!(revision, "startup-test");
    assert_eq!(snapshot["physical_objects"], 3);
    assert_eq!(snapshot["physical_bytes"], 140);
    assert_eq!(snapshot["logical_bytes"], 110);
    assert_eq!(snapshot["logical_objects"], 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn storage_snapshot_exhaustion_never_lists_s3_or_replaces_the_last_good_snapshot() {
    let (url, pool) = database().await;
    let proxy = postgres_proxy(&url, usize::MAX, Duration::ZERO).await;
    let s3 = s3_listing().await;
    let output = run_worker(&url, &proxy, &s3).await;
    assert_eq!(output.status.code(), Some(5), "{}", output.stderr);
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 3);
    assert_eq!(s3.requests.load(Ordering::SeqCst), 0);
    assert!(!output.stdout.contains("storage_snapshot_started"));
    assert!(!output.stdout.contains("storage_snapshot_completed"));
    let failures: Vec<_> = events(&output.stderr)
        .into_iter()
        .filter(|event| event["event"] == "storage_snapshot_db_connect_failed")
        .collect();
    assert_eq!(failures.len(), 3);
    assert!(failures.last().unwrap()["retry_in_ms"].is_null());
    let (snapshot, revision): (Value, String) = sqlx::query_as(
        "SELECT snapshot, code_sha FROM storage_accounting_snapshots WHERE singleton = TRUE",
    )
    .fetch_one(&pool)
    .await
    .expect("last good snapshot");
    assert_eq!(snapshot, json!({"previous": "complete"}));
    assert_eq!(revision, "before");
}
