use super::*;
use axum::body::{to_bytes, Body, Bytes};
use axum::http::{Method, Request};
use buzz_core::TenantContext;
use buzz_db::channel::{ChannelType, ChannelVisibility, MemberRole};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

type Objects = Arc<Mutex<HashMap<String, Bytes>>>;

struct ObjectServer {
    endpoint: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ObjectServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ObjectServer {
    async fn start() -> Self {
        async fn object(
            State((objects, requests)): State<(Objects, Arc<AtomicUsize>)>,
            Path(key): Path<String>,
            request: Request<Body>,
        ) -> Response {
            requests.fetch_add(1, Ordering::SeqCst);
            if request.method() == Method::PUT {
                let bytes = to_bytes(request.into_body(), 1024 * 1024)
                    .await
                    .expect("object body");
                objects.lock().await.insert(key, bytes);
                return StatusCode::OK.into_response();
            }
            let Some(bytes) = objects.lock().await.get(&key).cloned() else {
                return StatusCode::NOT_FOUND.into_response();
            };
            let mut status = StatusCode::OK;
            let mut data = bytes.clone();
            if let Some(range) = request.headers().get(header::RANGE) {
                let (start, end) = range
                    .to_str()
                    .expect("range")
                    .strip_prefix("bytes=")
                    .expect("bytes")
                    .split_once('-')
                    .expect("bounds");
                let start: usize = start.parse().expect("start");
                let end = end
                    .parse::<usize>()
                    .unwrap_or(bytes.len() - 1)
                    .min(bytes.len() - 1);
                data = bytes.slice(start..=end);
                status = StatusCode::PARTIAL_CONTENT;
            }
            Response::builder()
                .status(status)
                .header(header::CONTENT_LENGTH, data.len())
                .body(if request.method() == Method::HEAD {
                    Body::empty()
                } else {
                    Body::from(data)
                })
                .expect("object response")
        }
        let requests = Arc::new(AtomicUsize::new(0));
        let router = axum::Router::new()
            .route("/{*key}", axum::routing::any(object))
            .with_state((Objects::default(), requests.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind object server");
        let endpoint = format!("http://{}", listener.local_addr().expect("address"));
        let task =
            tokio::spawn(
                async move { axum::serve(listener, router).await.expect("object server") },
            );
        Self {
            endpoint,
            requests,
            task,
        }
    }
}

fn auth(keys: &Keys, host: &str, verb: &str, sha: &str) -> String {
    let expiry = (Timestamp::now().as_secs() + 300).to_string();
    let event = EventBuilder::new(Kind::Custom(24242), "media test")
        .tags([
            Tag::parse(["t", verb]).expect("t"),
            Tag::parse(["x", sha]).expect("x"),
            Tag::parse(["server", host]).expect("server"),
            Tag::parse(["expiration", &expiry]).expect("expiration"),
        ])
        .sign_with_keys(keys)
        .expect("auth event");
    format!(
        "Nostr {}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(event.as_json())
    )
}

fn request(
    keys: &Keys,
    host: &str,
    sha: &str,
    path: &str,
    method: &str,
    range: bool,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, host)
        .header(header::AUTHORIZATION, auth(keys, host, "get", sha));
    if range {
        request = request.header(header::RANGE, "bytes=0-3");
    }
    request.body(Body::empty()).expect("request")
}

async fn state(server: &ObjectServer) -> (Arc<AppState>, TenantContext, Keys, Keys, Uuid) {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("database");
    buzz_db::migration::run_migrations(&pool)
        .await
        .expect("migrate");
    let state = super::tests::test_state_with_media(Some(&server.endpoint)).await;
    let host = format!("media-http-{}.example", Uuid::new_v4());
    state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("community");
    let tenant = crate::tenant::bind_community(&state.db, &host)
        .await
        .expect("tenant");
    let owner = Keys::generate();
    let member = Keys::generate();
    for key in [&owner, &member] {
        state
            .db
            .ensure_user(tenant.community(), key.public_key().as_bytes())
            .await
            .expect("user");
    }
    let channel = state
        .db
        .create_channel(
            tenant.community(),
            "private",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .expect("channel")
        .id;
    state
        .db
        .add_member(
            tenant.community(),
            channel,
            member.public_key().as_bytes(),
            MemberRole::Member,
            Some(owner.public_key().as_bytes()),
        )
        .await
        .expect("add member");
    (state, tenant, owner, member, channel)
}

fn router(state: Arc<AppState>) -> axum::Router {
    axum::Router::new()
        .route(
            "/media/{sha256_ext}",
            axum::routing::get(get_blob).head(head_blob),
        )
        .route("/upload", axum::routing::put(upload_blob))
        .with_state(state)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn removal_denies_get_head_ranges_and_thumbnails_before_storage() {
    let server = ObjectServer::start().await;
    let (state, tenant, owner, member, channel) = state(&server).await;
    let bytes = Bytes::from_static(b"private attachment");
    let sha = hex::encode(Sha256::digest(&bytes));
    let upload = Request::builder()
        .method("PUT")
        .uri("/upload")
        .header(header::HOST, tenant.host())
        .header("X-SHA-256", &sha)
        .header(
            header::AUTHORIZATION,
            auth(&owner, tenant.host(), "upload", &sha),
        )
        .body(Body::from(bytes.clone()))
        .expect("upload request");
    let response = router(state.clone()).oneshot(upload).await.expect("upload");
    assert_eq!(response.status(), StatusCode::OK);
    let descriptor: buzz_media::BlobDescriptor = serde_json::from_slice(
        &to_bytes(response.into_body(), 16384)
            .await
            .expect("descriptor"),
    )
    .expect("json");
    assert!(state
        .db
        .can_reference_media(tenant.community(), &sha, owner.public_key().as_bytes())
        .await
        .expect("upload proof"));
    let path = url::Url::parse(&descriptor.url)
        .expect("url")
        .path()
        .to_string();
    state
        .media_storage
        .put(&format!("{sha}.thumb.jpg"), &bytes, "image/jpeg")
        .await
        .expect("thumbnail fixture");
    let event = EventBuilder::new(Kind::Custom(9), &descriptor.url)
        .sign_with_keys(&owner)
        .expect("event");
    state
        .db
        .insert_event(tenant.community(), &event, Some(channel))
        .await
        .expect("publish attachment");
    let paths = [
        path,
        format!("/media/{sha}"),
        format!("/media/{sha}.thumb.jpg"),
    ];
    for path in &paths {
        for (method, range, status) in [
            ("GET", false, StatusCode::OK),
            ("HEAD", false, StatusCode::OK),
            ("GET", true, StatusCode::PARTIAL_CONTENT),
        ] {
            let response = router(state.clone())
                .oneshot(request(&member, tenant.host(), &sha, path, method, range))
                .await
                .expect("member request");
            assert_eq!(response.status(), status, "{method} {path}");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "private, no-store"
            );
        }
    }
    state
        .db
        .remove_member(
            tenant.community(),
            channel,
            member.public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .expect("remove");
    // Poison the usual caches: media authorization must still consult live membership.
    state.membership_cache.insert(
        (
            tenant.community(),
            channel,
            member.public_key().to_bytes().to_vec(),
        ),
        true,
    );
    state.accessible_channels_cache.insert(
        (tenant.community(), member.public_key().to_bytes().to_vec()),
        vec![channel],
    );
    let before = server.requests.load(Ordering::SeqCst);
    for path in &paths {
        for (method, range) in [("GET", false), ("HEAD", false), ("GET", true)] {
            let response = router(state.clone())
                .oneshot(request(&member, tenant.host(), &sha, path, method, range))
                .await
                .expect("removed request");
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {path}");
        }
    }
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        before,
        "denied reads must not touch storage"
    );
    assert_eq!(
        router(state.clone())
            .oneshot(request(
                &owner,
                tenant.host(),
                &sha,
                &paths[0],
                "GET",
                false
            ))
            .await
            .expect("owner")
            .status(),
        StatusCode::OK
    );

    let open = state
        .db
        .create_channel(
            tenant.community(),
            "open",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .expect("open channel")
        .id;
    for (kind, content, tags) in [
        (
            0,
            serde_json::json!({"picture":descriptor.url}).to_string(),
            vec![],
        ),
        (
            9,
            descriptor.url.clone(),
            vec![Tag::parse(["h", &open.to_string()]).expect("channel tag")],
        ),
    ] {
        let forged = EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags)
            .sign_with_keys(&member)
            .expect("copied URL");
        let result = crate::handlers::ingest::ingest_event(
            &state,
            &tenant,
            forged,
            crate::handlers::ingest::IngestAuth::Nip42 {
                pubkey: member.public_key(),
                scopes: buzz_auth::Scope::all_known(),
                channel_ids: None,
                conn_id: Uuid::new_v4(),
            },
        )
        .await;
        assert!(
            matches!(result, Err(crate::handlers::ingest::IngestError::Rejected(ref reason)) if reason == "restricted: attachment is not accessible"),
            "copied URL must not create a read grant: {:?}",
            result.err()
        );
    }

    // Deduplication must still hash the supplied body before recording possession.
    for (body, status, can_publish) in [
        (
            Bytes::from_static(b"wrong bytes"),
            StatusCode::UNAUTHORIZED,
            false,
        ),
        (bytes, StatusCode::OK, true),
    ] {
        let upload = Request::builder()
            .method("PUT")
            .uri("/upload")
            .header(header::HOST, tenant.host())
            .header("X-SHA-256", &sha)
            .header(
                header::AUTHORIZATION,
                auth(&member, tenant.host(), "upload", &sha),
            )
            .body(Body::from(body))
            .expect("reupload request");
        assert_eq!(
            router(state.clone())
                .oneshot(upload)
                .await
                .expect("reupload")
                .status(),
            status
        );
        assert_eq!(
            state
                .db
                .can_reference_media(tenant.community(), &sha, member.public_key().as_bytes())
                .await
                .expect("publication access"),
            can_publish
        );
        assert!(!state
            .db
            .can_read_media(tenant.community(), &sha, member.public_key().as_bytes())
            .await
            .expect("read access"));
    }
}
