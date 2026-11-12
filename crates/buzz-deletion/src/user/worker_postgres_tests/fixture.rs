use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct Fixture {
    pub store: EntityDeletionStore,
    pub id: Uuid,
    pub pool: sqlx::PgPool,
    pub fail_listing: Arc<AtomicBool>,
    pub requests: Arc<AtomicUsize>,
    pub deleted: Arc<AtomicBool>,
    storage: Arc<buzz_media::MediaStorage>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    pub fn options(&self) -> RunOptions {
        RunOptions {
            db: Some(Db::from_pool(self.pool.clone())),
            media: Some(self.storage.clone()),
            delete: true,
            next_step: false,
            fail_stage: None,
            stage_delay: Duration::ZERO,
            no_agents_in: Vec::new(),
        }
    }
    pub async fn uploader_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM media_uploaders WHERE sha256 = $1")
            .bind("a".repeat(64))
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

pub(super) async fn fixture() -> Fixture {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let db = Db::from_pool(pool.clone());
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        db.migrate().await.unwrap();
    }
    let target = nostr::Keys::generate().public_key().to_hex();
    for id in [11, 12] {
        let community = Uuid::from_u128(id);
        sqlx::query("INSERT INTO communities(id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("{community}.workflow.example"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO relay_members(community_id, pubkey, role) VALUES ($1, $2, 'member')",
        )
        .bind(community)
        .bind(&target)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO media_uploaders(community_id, sha256, pubkey) VALUES ($1, $2, decode($3, 'hex'))")
        .bind(Uuid::from_u128(11)).bind("a".repeat(64)).bind(&target)
        .execute(&pool).await.unwrap();
    let store = db.entity_deletion_store();
    let request = db
        .user_deletion_store()
        .submit(&target, "submitter", None)
        .await
        .unwrap();
    store
        .approve(
            request.id,
            "approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let storage = Arc::new(
        buzz_media::MediaStorage::new(&buzz_media::MediaConfig {
            s3_endpoint: endpoint,
            s3_bucket: "test".into(),
            s3_access_key: "test".into(),
            s3_secret_key: "test".into(),
            s3_region: "us-east-1".into(),
            s3_addressing_style: buzz_media::S3AddressingStyle::Path,
            max_image_bytes: 1,
            max_gif_bytes: 1,
            max_video_bytes: 1,
            max_file_bytes: 1,
            public_base_url: "https://media.example".into(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        })
        .unwrap(),
    );
    let fail_listing = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let deleted = Arc::new(AtomicBool::new(false));
    let (failure, calls, removed) = (fail_listing.clone(), requests.clone(), deleted.clone());
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                let mut request = Vec::new();
                let headers_end = loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&chunk[..n]);
                    assert!(request.len() < 65536);
                    if let Some(end) = request.windows(4).position(|x| x == b"\r\n\r\n") { break end + 4; }
                };
                let headers = std::str::from_utf8(&request[..headers_end]).unwrap();
                let length = headers.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                }).unwrap_or(0);
                assert!(length < 65536);
                while request.len() < headers_end + length {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&chunk[..n]);
                }
                let request = String::from_utf8(request).unwrap();
                calls.fetch_add(1, Ordering::SeqCst);
                let hash = "a".repeat(64);
                let body = if request.starts_with("POST ") {
                    assert!(request.contains(&format!("<Key>{hash}.png</Key>")));
                    assert!(request.contains("<VersionId>v1</VersionId>"));
                    removed.store(true, Ordering::SeqCst);
                    format!("<DeleteResult><Deleted><Key>{hash}.png</Key><VersionId>v1</VersionId></Deleted></DeleteResult>")
                } else {
                    assert!(request.starts_with("GET "));
                    if failure.load(Ordering::SeqCst) {
                        "<ListVersionsResult><IsTruncated>false</IsTruncated>".into()
                    } else {
                        let entry = if removed.load(Ordering::SeqCst) || request.contains("prefix=_uploads") {
                            String::new()
                        } else {
                            format!("<Version><Key>{hash}.png</Key><VersionId>v1</VersionId><Size>1</Size></Version>")
                        };
                        format!("<ListVersionsResult><IsTruncated>false</IsTruncated>{entry}</ListVersionsResult>")
                    }
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }).await.unwrap();
        }
    });
    Fixture {
        store,
        id: request.id,
        pool,
        fail_listing,
        requests,
        deleted,
        storage,
        server,
    }
}
