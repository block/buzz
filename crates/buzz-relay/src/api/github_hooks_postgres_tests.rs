// Drives GitHub deliveries through the production router against a real hook
// artifact. Run with `cargo test -p buzz-relay --lib github_hooks -- --ignored`.
use super::*;
use crate::api::bridge::postgres_tests::bridge_handler_test_state;
use nostr::{Event, Keys};
use tower::ServiceExt;

const SECRET: &str = "s3cret";
const PR_URL: &str = "https://github.com/o/r/pull/7";

struct Fixture {
    state: Arc<AppState>,
    pool: sqlx::PgPool,
    host: String,
    community: CommunityId,
    home: Uuid,
    owner: Keys,
    hook: Uuid,
}

impl Fixture {
    async fn new() -> Self {
        let state = bridge_handler_test_state()
            .await
            .expect("Postgres and Redis");
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let host = format!("github-hooks-{}.local", Uuid::new_v4());
        state.db.ensure_configured_community(&host).await.unwrap();
        state
            .db
            .maintain_partitions(
                1,
                buzz_db::partition::PartitionMaintenancePolicy {
                    create_enabled: true,
                    advance_enabled: false,
                },
            )
            .await
            .unwrap();
        let community = crate::tenant::bind_community(&state.db, &host)
            .await
            .unwrap()
            .community();
        let home = Uuid::new_v4();
        let owner = Keys::generate();
        sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) VALUES($1,$2,$2::text,'private'::channel_visibility,'stream',$3)")
            .bind(community.as_uuid()).bind(home).bind(owner.public_key().as_bytes().as_slice()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'owner')")
            .bind(community.as_uuid()).bind(home).bind(owner.public_key().as_bytes().as_slice()).execute(&pool).await.unwrap();
        let f = Self {
            state,
            pool,
            host,
            community,
            home,
            owner,
            hook: Uuid::new_v4(),
        };
        let secret = nostr::nips::nip44::encrypt(
            f.owner.secret_key(),
            &f.state.relay_keypair.public_key(),
            SECRET,
            nostr::nips::nip44::Version::V2,
        )
        .unwrap();
        let hook = f.hook_revision(&json!({"version": 1, "repo": "O/R", "secret": secret}));
        f.publish(&hook).await;
        f
    }

    fn hook_revision(&self, content: &Value) -> Event {
        EventBuilder::new(Kind::Custom(45010), content.to_string())
            .tags(
                [
                    ["ar", "1"],
                    ["d", &self.hook.to_string()],
                    ["h", &self.home.to_string()],
                    ["type", HOOK_TYPE],
                    ["title", "o/r"],
                    ["op", "create"],
                    ["repo", "o/r"],
                ]
                .map(|t| Tag::parse(t).unwrap()),
            )
            .sign_with_keys(&self.owner)
            .unwrap()
    }

    async fn publish(&self, event: &Event) {
        self.publish_as(&self.owner, event).await;
    }

    async fn publish_as(&self, keys: &Keys, event: &Event) {
        let response = crate::router::build_router(self.state.clone())
            .oneshot(
                axum::http::Request::post("/events")
                    .header("host", &self.host)
                    .header("content-type", "application/json")
                    .header("x-pubkey", keys.public_key().to_hex())
                    .body(axum::body::Body::from(json!(event).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            status.is_success(),
            "{status} {}",
            String::from_utf8_lossy(&body)
        );
    }

    async fn deliver_signed(&self, event: &str, payload: &Value, secret: &str) -> StatusCode {
        self.deliver_at(&self.host, event, payload, secret).await
    }

    async fn deliver_at(
        &self,
        host: &str,
        event: &str,
        payload: &Value,
        secret: &str,
    ) -> StatusCode {
        let body = payload.to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body.as_bytes());
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        crate::router::build_router(self.state.clone())
            .oneshot(
                axum::http::Request::post(format!("/hooks/github/{}", self.hook))
                    .header("host", host)
                    .header("content-type", "application/json")
                    .header("x-github-event", event)
                    .header("x-hub-signature-256", signature)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    async fn deliver(&self, event: &str, payload: &Value) -> StatusCode {
        self.deliver_signed(event, payload, SECRET).await
    }

    async fn card(&self) -> (StoredEvent, Pull) {
        let heads = self
            .state
            .db
            .relay_artifact_heads_tagged(
                self.community,
                self.home,
                PULL_TYPE,
                self.state.relay_keypair.public_key().as_bytes(),
                &[("r", PR_URL)],
                5,
            )
            .await
            .unwrap();
        assert_eq!(heads.len(), 1);
        let head = heads.into_iter().next().unwrap();
        assert_eq!(head.event.pubkey, self.state.relay_keypair.public_key());
        let pull = serde_json::from_str(&head.event.content).unwrap();
        (head, pull)
    }

    async fn add_channel(&self, member: &Keys) -> Uuid {
        let channel = Uuid::new_v4();
        sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) VALUES($1,$2,$2::text,'private'::channel_visibility,'stream',$3)")
            .bind(self.community.as_uuid()).bind(channel).bind(member.public_key().as_bytes().as_slice()).execute(&self.pool).await.unwrap();
        self.add_member(channel, member).await;
        channel
    }

    async fn add_member(&self, channel: Uuid, member: &Keys) {
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'member')")
            .bind(self.community.as_uuid()).bind(channel).bind(member.public_key().as_bytes().as_slice()).execute(&self.pool).await.unwrap();
    }

    /// A member's revision of the card at its current head.
    async fn tamper(&self, member: &Keys, op: &str, channel: Uuid, content: &str) {
        let (head, _) = self.card_any().await;
        let mut tags = vec![
            vec!["ar".to_owned(), "1".to_owned()],
            vec!["d".to_owned(), tag_value(&head, "d").unwrap().to_owned()],
            vec!["h".to_owned(), channel.to_string()],
            vec!["type".to_owned(), PULL_TYPE.to_owned()],
            vec!["op".to_owned(), op.to_owned()],
            vec!["prev".to_owned(), head.event.id.to_hex()],
        ];
        if op == "delete" {
            tags.push(vec![
                "root".to_owned(),
                tag_value(&head, "root").unwrap().to_owned(),
            ]);
        } else {
            tags.push(vec!["title".to_owned(), "forged".to_owned()]);
        }
        let event = EventBuilder::new(Kind::Custom(45010), content)
            .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
            .sign_with_keys(member)
            .unwrap();
        self.publish_as(member, &event).await;
    }

    /// The card's current head, whoever signed it.
    async fn card_any(&self) -> (StoredEvent, bool) {
        let id: Uuid = sqlx::query_scalar("SELECT a.artifact_id FROM artifact_heads a WHERE a.community_id=$1 AND a.artifact_type=$2")
            .bind(self.community.as_uuid())
            .bind(PULL_TYPE)
            .fetch_one(&self.pool)
            .await
            .unwrap();
        self.state
            .db
            .relay_artifact_head(self.community, id)
            .await
            .unwrap()
            .unwrap()
    }

    async fn messages(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT content FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=9 AND pubkey=$3 ORDER BY created_at, received_at")
            .bind(self.community.as_uuid())
            .bind(self.home)
            .bind(self.state.relay_keypair.public_key().as_bytes().as_slice())
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }
}

fn pull_request(action: &str, updated_at: &str, extra: Value) -> Value {
    let mut pr = json!({
        "html_url": PR_URL,
        "number": 7,
        "title": "Ping @owner",
        "user": {"login": "bob"},
        "state": "open",
        "draft": false,
        "merged": false,
        "base": {"ref": "main"},
        "head": {"ref": "fix", "sha": "a1"},
        "updated_at": updated_at,
        "requested_reviewers": [{"login": "carol"}]
    });
    for (k, v) in extra.as_object().unwrap() {
        pr[k] = v.clone();
    }
    json!({
        "action": action,
        "pull_request": pr,
        "repository": {"full_name": "o/r", "html_url": "https://github.com/o/r"},
        "sender": {"login": "bob"}
    })
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_followed_repository_keeps_one_card_per_pull_request() {
    let f = Fixture::new().await;
    let opened = pull_request("opened", "2026-11-04T10:00:00Z", json!({}));

    assert_eq!(
        f.deliver_signed("pull_request", &opened, "wrong").await,
        StatusCode::NOT_FOUND
    );
    let mut elsewhere = opened.clone();
    elsewhere["repository"]["full_name"] = json!("o/other");
    assert_eq!(
        f.deliver("pull_request", &elsewhere).await,
        StatusCode::ACCEPTED
    );
    let mut fork = json!({"action": "completed", "check_run": {"head_sha": "a1", "name": "lint",
        "status": "completed", "conclusion": "success", "pull_requests": []},
        "repository": {"full_name": "o/r", "html_url": "https://github.com/o/r"}});
    assert_eq!(f.deliver("check_run", &fork).await, StatusCode::OK);
    assert_eq!(
        f.deliver("ping", &json!({"zen": "hi"})).await,
        StatusCode::OK
    );
    assert!(f.messages().await.is_empty());

    assert_eq!(f.deliver("pull_request", &opened).await, StatusCode::OK);
    assert_eq!(f.deliver("pull_request", &opened).await, StatusCode::OK);
    let (head, pull) = f.card().await;
    let messages = f.messages().await;
    assert_eq!(
        messages,
        vec![format!("#7 · Ping @\u{2060}owner\n{PR_URL}")]
    );
    assert_eq!(
        (pull.state.as_str(), pull.requested_reviewers.clone()),
        ("open", vec!["carol".to_owned()])
    );
    let root = tag_value(&head, "root").unwrap().to_owned();
    let announcement: String = sqlx::query_scalar(
        "SELECT encode(id,'hex') FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=9",
    )
    .bind(f.community.as_uuid())
    .bind(f.home)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(root, announcement);

    let review = json!({
        "action": "submitted",
        "review": {"user": {"login": "carol"}, "state": "APPROVED"},
        "pull_request": {"html_url": PR_URL},
        "repository": {"full_name": "o/r"}
    });
    assert_eq!(
        f.deliver("pull_request_review", &review).await,
        StatusCode::OK
    );
    assert_eq!(
        f.deliver("pull_request_review", &review).await,
        StatusCode::OK
    );
    let suite = json!({
        "action": "completed",
        "check_suite": {"head_sha": "a1", "status": "completed", "conclusion": "success",
            "app": {"slug": "buildkite"}, "pull_requests": [{"number": 7}]},
        "repository": {"full_name": "o/r", "html_url": "https://github.com/o/r"}
    });
    assert_eq!(f.deliver("check_suite", &suite).await, StatusCode::OK);
    let status = json!({"sha": "a1", "context": "kochiku", "state": "failure",
        "repository": {"full_name": "o/r"}});
    assert_eq!(f.deliver("status", &status).await, StatusCode::OK);
    let (_, pull) = f.card().await;
    assert_eq!(
        pull.reviews.get("carol").map(String::as_str),
        Some("approved")
    );
    assert!(pull.requested_reviewers.is_empty());
    assert_eq!(pull.checks.runs.len(), 2);
    assert_eq!(pull.checks.state, "failure");
    fork["check_run"]["conclusion"] = json!("failure");
    assert_eq!(f.deliver("check_run", &fork).await, StatusCode::OK);
    let (_, pull) = f.card().await;
    assert_eq!(
        pull.checks.runs.get("lint").map(String::as_str),
        Some("failure"),
        "a check without pull requests, as GitHub sends for forks, finds the card by SHA"
    );

    let merged = pull_request(
        "closed",
        "2026-11-04T12:00:00Z",
        json!({"state": "closed", "merged": true, "merged_by": {"login": "dan"}}),
    );
    assert_eq!(f.deliver("pull_request", &merged).await, StatusCode::OK);
    assert_eq!(f.deliver("pull_request", &merged).await, StatusCode::OK);
    let stale = pull_request("edited", "2026-11-04T11:00:00Z", json!({"title": "Stale"}));
    assert_eq!(f.deliver("pull_request", &stale).await, StatusCode::OK);
    let (head, pull) = f.card().await;
    assert_eq!(
        (pull.state.as_str(), pull.title.as_str()),
        ("merged", "Ping @owner")
    );
    assert_eq!(tag_value(&head, "state"), Some("merged"));
    assert_eq!(tag_value(&head, "status"), Some("merged"));
    assert_eq!(tag_value(&head, "source"), Some("github"));
    assert_eq!(tag_value(&head, "root"), Some(root.as_str()));
    assert_eq!(
        &f.messages().await[1..],
        ["carol approved", "Merged by dan"]
    );
    let replies: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=9 AND tags @> $3")
        .bind(f.community.as_uuid()).bind(f.home).bind(json!([["e", root, "", "reply"]]))
        .fetch_one(&f.pool).await.unwrap();
    assert_eq!(replies, 2);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_hook_stops_when_its_author_loses_the_channel_or_deletes_it() {
    let f = Fixture::new().await;
    let opened = pull_request("opened", "2026-11-04T10:00:00Z", json!({}));
    sqlx::query(
        "UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(f.home)
    .execute(&f.pool)
    .await
    .unwrap();
    let tenant = crate::tenant::bind_community(&f.state.db, &f.host)
        .await
        .unwrap();
    f.state
        .invalidate_membership(&tenant, f.home, &f.owner.public_key().to_bytes());
    assert_eq!(
        f.deliver("pull_request", &opened).await,
        StatusCode::FORBIDDEN
    );
    assert!(f.messages().await.is_empty());

    let other = Fixture::new().await;
    let head = other
        .state
        .db
        .relay_artifact_head(other.community, other.hook)
        .await
        .unwrap()
        .unwrap()
        .0;
    let delete = EventBuilder::new(Kind::Custom(45010), "")
        .tags(
            [
                ["ar", "1"],
                ["d", &other.hook.to_string()],
                ["h", &other.home.to_string()],
                ["type", HOOK_TYPE],
                ["op", "delete"],
                ["prev", &head.event.id.to_hex()],
            ]
            .map(|t| Tag::parse(t).unwrap()),
        )
        .sign_with_keys(&other.owner)
        .unwrap();
    other.publish(&delete).await;
    assert_eq!(
        other.deliver("pull_request", &opened).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn members_cannot_take_a_card_away_or_rewrite_it() {
    let f = Fixture::new().await;
    let mallory = Keys::generate();
    f.add_member(f.home, &mallory).await;
    let elsewhere = f.add_channel(&mallory).await;

    let opened = pull_request("opened", "2026-11-04T10:00:00Z", json!({}));
    let (a, b) = tokio::join!(
        f.deliver("pull_request", &opened),
        f.deliver("pull_request", &opened)
    );
    assert_eq!((a, b), (StatusCode::OK, StatusCode::OK));
    let (head, _) = f.card().await;
    let root = tag_value(&head, "root").unwrap().to_owned();
    assert_eq!(
        f.messages().await.len(),
        1,
        "concurrent opens announce once"
    );

    f.tamper(
        &mallory,
        "update",
        f.home,
        r#"{"version":1,"state":"merged"}"#,
    )
    .await;
    let review = json!({
        "action": "submitted",
        "review": {"user": {"login": "carol"}, "state": "approved"},
        "pull_request": {"html_url": PR_URL},
        "repository": {"full_name": "o/r"}
    });
    let (a, b) = tokio::join!(
        f.deliver("pull_request_review", &review),
        f.deliver("pull_request_review", &review)
    );
    assert_eq!((a, b), (StatusCode::OK, StatusCode::OK));
    let (head, pull) = f.card().await;
    assert_eq!(
        (pull.state.as_str(), tag_value(&head, "root")),
        ("open", Some(root.as_str()))
    );

    f.tamper(&mallory, "delete", f.home, "").await;
    let suite = json!({
        "action": "completed",
        "check_suite": {"head_sha": "a1", "status": "completed", "conclusion": "success",
            "app": {"slug": "buildkite"}, "pull_requests": [{"number": 7}]},
        "repository": {"full_name": "o/r", "html_url": "https://github.com/o/r"}
    });
    assert_eq!(f.deliver("check_suite", &suite).await, StatusCode::OK);
    let (_, pull) = f.card().await;
    assert_eq!(pull.checks.state, "success");

    let (_, pull) = f.card().await;
    f.tamper(
        &mallory,
        "move",
        elsewhere,
        &serde_json::to_string(&pull).unwrap(),
    )
    .await;
    let merged = pull_request(
        "closed",
        "2026-11-04T12:00:00Z",
        json!({"state": "closed", "merged": true, "merged_by": {"login": "dan"}}),
    );
    let (a, b) = tokio::join!(
        f.deliver("pull_request", &merged),
        f.deliver("pull_request", &merged)
    );
    assert_eq!((a, b), (StatusCode::OK, StatusCode::OK));
    let (head, pull) = f.card().await;
    assert_eq!(
        (pull.state.as_str(), tag_value(&head, "root")),
        ("merged", Some(root.as_str()))
    );
    assert_eq!(
        &f.messages().await[1..],
        ["carol approved", "Merged by dan"],
        "repairs and concurrent redeliveries post each reply once"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_timed_out_author_or_another_community_cannot_use_the_hook() {
    let f = Fixture::new().await;
    let opened = pull_request("opened", "2026-11-04T10:00:00Z", json!({}));
    let other = Fixture::new().await;
    let ping = json!({"zen": "hi"});
    assert_eq!(
        f.deliver_at(&other.host, "ping", &ping, SECRET).await,
        StatusCode::NOT_FOUND,
        "a hook answers only on its own community's host"
    );
    assert_eq!(f.deliver("ping", &ping).await, StatusCode::OK);

    let owner = f.owner.public_key().to_bytes();
    f.state
        .db
        .timeout_community_member(
            f.community,
            &owner,
            &owner,
            chrono::Utc::now() + chrono::Duration::hours(1),
            None,
        )
        .await
        .unwrap();
    f.state.invalidate_restrictions_local(f.community);
    assert_eq!(
        f.deliver("pull_request", &opened).await,
        StatusCode::FORBIDDEN
    );
    assert!(f.messages().await.is_empty());
}
