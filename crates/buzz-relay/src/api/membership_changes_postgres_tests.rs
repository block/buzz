// Exercises `POST /query` and `POST /count` for `{"membership":"removed","since":T}`.
use super::postgres_tests::bridge_handler_test_state;
use axum::http::StatusCode;
use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};
use nostr::Keys;
use serde_json::{json, Value};
use uuid::Uuid;

struct Fixture {
    state: std::sync::Arc<crate::state::AppState>,
    host: String,
    community: buzz_core::CommunityId,
    owner: Keys,
    reader: Keys,
}

impl Fixture {
    async fn new() -> Self {
        let state = bridge_handler_test_state()
            .await
            .expect("Postgres and Redis");
        let host = format!("membership-changes-{}.local", Uuid::new_v4());
        state.db.ensure_configured_community(&host).await.unwrap();
        let tenant = crate::tenant::bind_community(&state.db, &host)
            .await
            .unwrap();
        Self {
            state,
            host,
            community: tenant.community(),
            owner: Keys::generate(),
            reader: Keys::generate(),
        }
    }

    /// A private channel the reader has joined.
    async fn channel(&self) -> Uuid {
        let owner = self.owner.public_key().to_bytes();
        let channel = self
            .state
            .db
            .create_channel(
                self.community,
                "c",
                ChannelType::Stream,
                ChannelVisibility::Private,
                None,
                &owner,
                None,
            )
            .await
            .unwrap()
            .id;
        self.join(channel).await;
        channel
    }

    async fn join(&self, channel: Uuid) {
        self.state
            .db
            .add_member(
                self.community,
                channel,
                &self.reader.public_key().to_bytes(),
                MemberRole::Member,
                Some(&self.owner.public_key().to_bytes()),
            )
            .await
            .unwrap();
    }

    async fn remove(&self, channel: Uuid) {
        self.state
            .db
            .remove_member(
                self.community,
                channel,
                &self.reader.public_key().to_bytes(),
                &self.owner.public_key().to_bytes(),
            )
            .await
            .unwrap();
    }

    async fn request(&self, path: &str, key: &Keys, body: Value) -> (StatusCode, Value) {
        use tower::ServiceExt;
        let response = crate::router::build_router(self.state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("host", &self.host)
                    .header("content-type", "application/json")
                    .header("x-pubkey", key.public_key().to_hex())
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn removed_since(&self, since: i64) -> (StatusCode, Value) {
        self.request(
            "/query",
            &self.reader,
            json!([{"membership":"removed","since":since}]),
        )
        .await
    }
}

fn hour_ago() -> i64 {
    chrono::Utc::now().timestamp() - 3600
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn query_reports_removed_and_deleted_memberships_for_the_reader_only() {
    let f = Fixture::new().await;
    let removed = f.channel().await;
    let deleted = f.channel().await;
    let rejoined = f.channel().await;
    let active = f.channel().await;
    f.remove(removed).await;
    f.remove(rejoined).await;
    f.join(rejoined).await;
    assert!(f
        .state
        .db
        .soft_delete_channel(f.community, deleted)
        .await
        .unwrap());

    let before = chrono::Utc::now().timestamp();
    let (status, body) = f.removed_since(hour_ago()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["asOf"].as_i64().unwrap() >= before - 1, "{body}");
    let mut got: Vec<(String, String)> = body["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            assert!(r["at"].as_i64().unwrap() >= hour_ago(), "{body}");
            (
                r["channelId"].as_str().unwrap().to_owned(),
                r["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    got.sort();
    let mut want = vec![
        (removed.to_string(), "removed".to_owned()),
        (deleted.to_string(), "deleted".to_owned()),
    ];
    want.sort();
    assert_eq!(got, want, "{body} (active {active}, rejoined {rejoined})");

    // A reader who never joined sees none of it.
    let (status, body) = f
        .request(
            "/query",
            &Keys::generate(),
            json!([{"membership":"removed","since":hour_ago()}]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removed"], json!([]), "{body}");

    // A window after the changes is empty.
    let (status, body) = f.removed_since(chrono::Utc::now().timestamp() + 60).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removed"], json!([]), "{body}");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn count_and_malformed_membership_filters_are_rejected() {
    let f = Fixture::new().await;
    let (status, body) = f
        .request(
            "/count",
            &f.reader,
            json!([{"membership":"removed","since":hour_ago()}]),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    for filter in [
        json!({"membership":"removed"}),
        json!({"membership":"removed","since":"soon"}),
        json!({"membership":"removed","since":-5}),
        json!({"membership":"added","since":1}),
        json!({"membership":"removed","since":1,"kinds":[39002]}),
    ] {
        let (status, body) = f.request("/query", &f.reader, json!([filter])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{filter} {body}");
    }
    let one = json!({"membership":"removed","since":1});
    let (status, body) = f
        .request("/query", &f.reader, json!([one.clone(), one]))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
