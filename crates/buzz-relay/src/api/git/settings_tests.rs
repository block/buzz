//! Live route/store/clone regressions. Require explicit isolated service URLs;
//! never fall back to a developer's Desktop database.

mod external_infra {
    use super::super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use base64::Engine;
    use buzz_core::channel::MemberRole;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    struct Fixture {
        state: Arc<AppState>,
        pool: sqlx::PgPool,
        tenant: TenantContext,
        owner: Keys,
        member: Keys,
        maintainer: Keys,
        channel: uuid::Uuid,
        repo: String,
        scratch: tempfile::TempDir,
    }

    impl Fixture {
        async fn new() -> Self {
            let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
                .expect("explicit isolated BUZZ_TEST_DATABASE_URL");
            let redis_url = std::env::var("BUZZ_TEST_REDIS_URL")
                .expect("explicit isolated BUZZ_TEST_REDIS_URL");
            let endpoint = std::env::var("BUZZ_TEST_S3_ENDPOINT")
                .expect("explicit isolated BUZZ_TEST_S3_ENDPOINT");
            let scratch = tempfile::tempdir().unwrap();
            let mut config = crate::config::Config::from_env().unwrap();
            config.database_url = database_url;
            config.redis_url = redis_url;
            config.relay_url = "ws://127.0.0.1".into();
            config.require_relay_membership = false;
            config.git_repo_path = scratch.path().to_path_buf();
            config.git_pack_cache_path = scratch.path().join("cache");
            config.media.s3_endpoint = endpoint;
            config.media.s3_bucket =
                std::env::var("BUZZ_TEST_S3_BUCKET").unwrap_or_else(|_| "buzz-git".into());
            config.media.s3_access_key = "buzz_dev".into();
            config.media.s3_secret_key = "buzz_dev_secret".into();
            let pool = sqlx::PgPool::connect(&config.database_url).await.unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            // CI provisions schema/schema.sql with pgschema before this suite.
            // Only migration-backed local fixtures own the migration lifecycle.
            if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
                db.migrate().await.unwrap();
            }
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (state, _) = AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow,
                Keys::generate(),
                media,
            );
            let state = Arc::new(state);
            let host = format!("settings-{}.example", uuid::Uuid::new_v4().simple());
            let community = state
                .db
                .ensure_configured_community(&host)
                .await
                .unwrap()
                .id;
            let tenant = TenantContext::resolved(community, &host);
            let owner = Keys::generate();
            let member = Keys::generate();
            let maintainer = Keys::generate();
            let channel = uuid::Uuid::new_v4();
            state
                .db
                .ensure_user(community, owner.public_key().as_bytes())
                .await
                .unwrap();
            state
                .db
                .create_channel_with_id(
                    community,
                    channel,
                    &format!("settings-{channel}"),
                    buzz_db::channel::ChannelType::Stream,
                    buzz_db::channel::ChannelVisibility::Open,
                    None,
                    owner.public_key().as_bytes(),
                    None,
                )
                .await
                .unwrap();
            for (key, role) in [
                (&member, MemberRole::Admin),
                (&maintainer, MemberRole::Member),
                (&owner, MemberRole::Owner),
            ] {
                state
                    .db
                    .ensure_user(community, key.public_key().as_bytes())
                    .await
                    .unwrap();
                state
                    .db
                    .add_member(
                        community,
                        channel,
                        key.public_key().as_bytes(),
                        role,
                        Some(owner.public_key().as_bytes()),
                    )
                    .await
                    .unwrap();
            }
            let repo = format!("repo-{}", uuid::Uuid::new_v4().simple());
            let announcement = EventBuilder::new(Kind::Custom(30617), "")
                .tags([
                    Tag::parse(["d", &repo]).unwrap(),
                    Tag::parse(["buzz-channel", &channel.to_string()]).unwrap(),
                    Tag::parse(["maintainers", &maintainer.public_key().to_hex()]).unwrap(),
                ])
                .sign_with_keys(&owner)
                .unwrap();
            state
                .db
                .insert_event(community, &announcement, None)
                .await
                .unwrap();
            let f = Self {
                state,
                pool,
                tenant,
                owner,
                member,
                maintainer,
                channel,
                repo,
                scratch,
            };
            f.seed_git().await;
            f
        }

        fn path(&self) -> String {
            format!(
                "/git/{}/{}/default-branch",
                self.owner.public_key().to_hex(),
                self.repo
            )
        }

        async fn snapshot(&self) -> DefaultBranchSnapshot {
            DefaultBranchSnapshot::load(
                &self.state.git_store,
                &self.tenant,
                &self.owner.public_key().to_hex(),
                &self.repo,
            )
            .await
            .unwrap()
        }

        async fn seed_git(&self) {
            let source = self.scratch.path().join("source");
            std::fs::create_dir(&source).unwrap();
            git(&source, &["init", "--initial-branch=legacy"]).await;
            git(&source, &["config", "user.name", "Git settings test"]).await;
            git(
                &source,
                &["config", "user.email", "git-settings@example.invalid"],
            )
            .await;
            git(&source, &["commit", "--allow-empty", "-m", "legacy"]).await;
            git(&source, &["branch", "main"]).await;
            git(&source, &["checkout", "main"]).await;
            std::fs::write(source.join("main.txt"), b"selected branch\n").unwrap();
            git(&source, &["add", "main.txt"]).await;
            git(&source, &["commit", "-m", "main"]).await;
            git(&source, &["checkout", "legacy"]).await;
            super::super::super::cas_publish::cas_publish(
                &self.state.git_store,
                &self.tenant,
                &source,
                &self.owner.public_key().to_hex(),
                &self.repo,
                &super::super::super::cas_publish::ParentState::fresh(),
                limits(0),
            )
            .await
            .unwrap();
        }

        async fn call(
            &self,
            key: &Keys,
            body: Option<Value>,
            tag: Option<&str>,
        ) -> (StatusCode, Value) {
            let body = body.map(|value| value.to_string());
            let method = if body.is_some() { "POST" } else { "GET" };
            let path = self.path();
            let token = token(
                key,
                method,
                &format!("http://{}{path}", self.tenant.host()),
                body.as_deref(),
            );
            let mut request = Request::builder()
                .method(method)
                .uri(&path)
                .header("host", self.tenant.host())
                .header("authorization", token);
            if let Some(tag) = tag {
                request = request.header("x-auth-tag", tag);
            }
            let request = request.body(Body::from(body.unwrap_or_default())).unwrap();
            response(
                super::super::super::transport::git_router(self.state.clone())
                    .oneshot(request)
                    .await
                    .unwrap(),
            )
            .await
        }

        async fn set(&self, key: &Keys, branch: &str, tag: Option<&str>) -> (StatusCode, Value) {
            let digest = self.snapshot().await.digest;
            self.call(
                key,
                Some(json!({"branch": branch, "expected_manifest": digest})),
                tag,
            )
            .await
        }

        async fn add(&self, key: &Keys) {
            self.state
                .db
                .ensure_user(self.tenant.community(), key.public_key().as_bytes())
                .await
                .unwrap();
            self.state
                .db
                .add_member(
                    self.tenant.community(),
                    self.channel,
                    key.public_key().as_bytes(),
                    MemberRole::Bot,
                    Some(self.owner.public_key().as_bytes()),
                )
                .await
                .unwrap();
        }
    }

    fn limits(parent_hydrated_bytes: u64) -> super::super::super::cas_publish::PublishLimits {
        super::super::super::cas_publish::PublishLimits {
            parent_hydrated_bytes,
            max_pack_bytes: 1024 * 1024,
            max_repo_bytes: 2 * 1024 * 1024,
        }
    }

    async fn git(path: &std::path::Path, args: &[&str]) -> String {
        let mut command = tokio::process::Command::new("git");
        command.current_dir(path).args(args);
        super::super::super::transport::harden_git_env(&mut command);
        let result = command.output().await.unwrap();
        assert!(
            result.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }

    fn token(keys: &Keys, method: &str, url: &str, body: Option<&str>) -> String {
        token_with_payload(
            keys,
            method,
            url,
            body.map(|body| Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap()),
        )
    }

    fn token_with_payload(keys: &Keys, method: &str, url: &str, payload: Option<Tag>) -> String {
        let mut tags = vec![
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", method]).unwrap(),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ];
        if let Some(payload) = payload {
            tags.push(payload);
        }
        let event = EventBuilder::new(Kind::Custom(27235), "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap();
        format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event).unwrap())
        )
    }

    async fn response(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({"error": String::from_utf8_lossy(&bytes)})),
        )
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_route_permissions_and_protocol() {
        let f = Fixture::new().await;
        let before = f.snapshot().await;
        assert_eq!(
            f.call(&f.member, None, None).await.1["head"],
            "refs/heads/legacy"
        );
        assert_eq!(
            f.set(&f.member, "main", None).await.0,
            StatusCode::FORBIDDEN,
            "push-capable channel admin is not a repo manager"
        );
        assert_eq!(
            f.set(&Keys::generate(), "main", None).await.0,
            StatusCode::NOT_FOUND
        );
        for branch in [
            "",
            "absent",
            "../main",
            "refs/heads/main",
            "main.lock",
            "bad\nref",
            "main/",
            "-main",
            ".main",
        ] {
            assert_eq!(
                f.set(&f.owner, branch, None).await.0,
                StatusCode::BAD_REQUEST,
                "{branch:?}"
            );
        }
        assert_eq!(
            f.snapshot().await.digest,
            before.digest,
            "denials do not write"
        );
        let result = f.set(&f.maintainer, "main", None).await;
        assert_eq!(result.0, StatusCode::OK, "{result:?}");
        assert_eq!(result.1["changed"], true);
        let after = f.snapshot().await;
        assert_eq!(after.manifest.head, "refs/heads/main");
        assert_eq!(after.manifest.refs, before.manifest.refs);
        assert_eq!(after.manifest.packs, before.manifest.packs);
        assert_eq!(after.manifest.parent.as_ref(), Some(&before.digest));
        let result = f.set(&f.owner, "main", None).await;
        assert_eq!(result.0, StatusCode::OK);
        assert_eq!(result.1["changed"], false);
        assert_eq!(f.snapshot().await.digest, after.digest);
        assert_eq!(
            f.call(
                &f.owner,
                Some(json!({"branch":"legacy", "expected_manifest": before.digest})),
                None
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        let notification_query = buzz_db::EventQuery {
            kinds: Some(vec![30618]),
            d_tag: Some(f.repo.clone()),
            global_only: true,
            ..buzz_db::EventQuery::for_community(f.tenant.community())
        };
        let events = f.state.db.query_events(&notification_query).await.unwrap();
        let event_ids: Vec<_> = events.iter().map(|e| e.event.id).collect();
        assert!(
            events.iter().any(|e| e
                .event
                .tags
                .iter()
                .any(|t| t.as_slice() == ["HEAD", "ref: refs/heads/main"])),
            "committed default notification: {events:?}"
        );

        // Strict credentials: each mutated property must be rejected at the real route.
        let body = json!({"branch":"legacy", "expected_manifest": after.digest}).to_string();
        let path = f.path();
        let url = format!("http://{}{path}", f.tenant.host());
        let requests = [
            token_with_payload(
                &f.owner,
                "POST",
                &url,
                Some(Tag::parse(["payload"]).unwrap()),
            ),
            token_with_payload(
                &f.owner,
                "POST",
                &url,
                Some(Tag::parse(["payload", ""]).unwrap()),
            ),
            token(&f.owner, "GET", &url, Some(&body)),
            token(&f.owner, "POST", &url, None),
            token(&f.owner, "POST", &url, Some("{}")),
            token(
                &f.owner,
                "POST",
                &url.replace(f.tenant.host(), "other.example"),
                Some(&body),
            ),
            token(
                &f.owner,
                "GET",
                url.trim_end_matches("/default-branch"),
                None,
            ),
        ];
        for token in requests {
            let request = Request::builder()
                .method("POST")
                .uri(&path)
                .header("host", f.tenant.host())
                .header("authorization", token)
                .body(Body::from(body.clone()))
                .unwrap();
            let status = super::super::super::transport::git_router(f.state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status();
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(
                f.snapshot().await.digest,
                after.digest,
                "auth denial changed pointer"
            );
            let denied_events = f.state.db.query_events(&notification_query).await.unwrap();
            assert_eq!(
                denied_events.iter().map(|e| e.event.id).collect::<Vec<_>>(),
                event_ids,
                "auth denial published kind:30618"
            );
        }
        let reusable = token(&f.owner, "GET", &url, None);
        for expected in [StatusCode::OK, StatusCode::UNAUTHORIZED] {
            let request = Request::builder()
                .uri(&path)
                .header("host", f.tenant.host())
                .header("authorization", &reusable)
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                super::super::super::transport::git_router(f.state.clone())
                    .oneshot(request)
                    .await
                    .unwrap()
                    .status(),
                expected
            );
        }
        let other_host = format!("other-{}.example", uuid::Uuid::new_v4());
        f.state
            .db
            .ensure_configured_community(&other_host)
            .await
            .unwrap();
        let token = token(&f.owner, "GET", &format!("http://{other_host}{path}"), None);
        let request = Request::builder()
            .uri(&path)
            .header("host", &other_host)
            .header("authorization", token)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            super::super::super::transport::git_router(f.state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_delegation_and_revocation() {
        let f = Fixture::new().await;
        let agent = Keys::generate();
        f.add(&agent).await;
        let tag = buzz_sdk::nip_oa::compute_auth_tag(&f.owner, &agent.public_key(), "").unwrap();
        assert_eq!(f.set(&agent, "main", None).await.0, StatusCode::FORBIDDEN);
        let limited =
            buzz_sdk::nip_oa::compute_auth_tag(&f.owner, &agent.public_key(), "kind=1").unwrap();
        assert_eq!(
            f.set(&agent, "main", Some(&limited)).await.0,
            StatusCode::FORBIDDEN
        );
        let expired =
            buzz_sdk::nip_oa::compute_auth_tag(&f.owner, &agent.public_key(), "created_at<1")
                .unwrap();
        assert_eq!(
            f.set(&agent, "main", Some(&expired)).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(f.set(&agent, "main", Some(&tag)).await.0, StatusCode::OK);
        // Optional credential does not take direct authority away.
        let absent_owner = Keys::generate();
        let own_tag =
            buzz_sdk::nip_oa::compute_auth_tag(&absent_owner, &f.owner.public_key(), "").unwrap();
        assert_eq!(
            f.set(&f.owner, "legacy", Some(&own_tag)).await.0,
            StatusCode::OK
        );
        // A human can administer a repository announced by their managed agent.
        f.state
            .db
            .set_agent_owner(
                f.tenant.community(),
                f.owner.public_key().as_bytes(),
                f.member.public_key().as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(f.set(&f.member, "main", None).await.0, StatusCode::OK);
        f.state
            .db
            .add_member(
                f.tenant.community(),
                f.channel,
                f.maintainer.public_key().as_bytes(),
                MemberRole::Owner,
                Some(f.owner.public_key().as_bytes()),
            )
            .await
            .unwrap();
        f.state
            .db
            .remove_member(
                f.tenant.community(),
                f.channel,
                f.owner.public_key().as_bytes(),
                f.owner.public_key().as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(
            f.set(&agent, "legacy", Some(&tag)).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            f.set(&f.owner, "legacy", None).await.0,
            StatusCode::NOT_FOUND
        );
        // Durable ban cascades even when the signer has independent maintainer rights.
        let ban_tag =
            buzz_sdk::nip_oa::compute_auth_tag(&f.member, &f.maintainer.public_key(), "").unwrap();
        f.state
            .db
            .ban_community_member(
                f.tenant.community(),
                f.member.public_key().as_bytes(),
                f.member.public_key().as_bytes(),
                Some("test"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            f.set(&f.maintainer, "legacy", Some(&ban_tag)).await.0,
            StatusCode::FORBIDDEN
        );
        sqlx::query("UPDATE channels SET archived_at = NOW() WHERE community_id = $1 AND id = $2")
            .bind(f.tenant.community().as_uuid())
            .bind(f.channel)
            .execute(&f.pool)
            .await
            .unwrap();
        assert_eq!(
            f.set(&f.maintainer, "legacy", None).await.0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_push_races_and_fresh_clone() {
        let f = Fixture::new().await;
        let a = f.snapshot().await;
        let b = f.snapshot().await;
        let old_digest = a.digest.clone();
        let (_, changed) = a
            .set(
                &f.state.git_store,
                SetDefaultBranch {
                    branch: "main".into(),
                    expected_manifest: old_digest.clone(),
                },
            )
            .await
            .unwrap();
        assert!(changed);
        let loser = b
            .set(
                &f.state.git_store,
                SetDefaultBranch {
                    branch: "legacy".into(),
                    expected_manifest: old_digest,
                },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            loser.status(),
            StatusCode::CONFLICT,
            "stale no-op must CAS too"
        );
        // Snapshot a push before the metadata update; it must not restore stale HEAD.
        let options = || super::super::super::hydrate::HydrationOptions {
            pack_cache: &f.state.git_pack_cache,
            scratch_dir: f.scratch.path(),
            max_pack_bytes: 1024 * 1024,
            max_repo_bytes: 2 * 1024 * 1024,
        };
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &f.owner.public_key().to_hex(),
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        assert_eq!(f.set(&f.owner, "legacy", None).await.0, StatusCode::OK);
        let result = super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &f.owner.public_key().to_hex(),
            &f.repo,
            &parent,
            limits(push.hydrated_bytes()),
        )
        .await;
        assert!(matches!(
            result,
            Err(super::super::super::cas_publish::CasError::Conflict { .. })
        ));
        // Other direction: a push deletes the candidate after settings loaded it.
        let stale = f.snapshot().await;
        let digest = stale.digest.clone();
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &f.owner.public_key().to_hex(),
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        git(push.path(), &["update-ref", "-d", "refs/heads/main"]).await;
        super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &f.owner.public_key().to_hex(),
            &f.repo,
            &parent,
            limits(push.hydrated_bytes()),
        )
        .await
        .unwrap();
        assert_eq!(
            stale
                .set(
                    &f.state.git_store,
                    SetDefaultBranch {
                        branch: "main".into(),
                        expected_manifest: digest
                    }
                )
                .await
                .err()
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        assert!(!f
            .snapshot()
            .await
            .manifest
            .refs
            .contains_key("refs/heads/main"));
        // Restore main and add release/v1, then select the non-main branch so
        // Git's initial-branch default cannot mask a lost hydrated HEAD.
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &f.owner.public_key().to_hex(),
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        let main = git(&f.scratch.path().join("source"), &["rev-parse", "main"]).await;
        git(push.path(), &["update-ref", "refs/heads/main", main.trim()]).await;
        git(
            push.path(),
            &["update-ref", "refs/heads/release/v1", main.trim()],
        )
        .await;
        super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &f.owner.public_key().to_hex(),
            &f.repo,
            &parent,
            limits(push.hydrated_bytes()),
        )
        .await
        .unwrap();
        assert_eq!(f.set(&f.owner, "release/v1", None).await.0, StatusCode::OK);
        let (push, parent) = super::super::super::hydrate::hydrate_for_write(
            &f.state.git_store,
            &f.tenant,
            &f.owner.public_key().to_hex(),
            &f.repo,
            options(),
        )
        .await
        .unwrap();
        git(
            push.path(),
            &["update-ref", "refs/heads/later", main.trim()],
        )
        .await;
        super::super::super::cas_publish::cas_publish(
            &f.state.git_store,
            &f.tenant,
            push.path(),
            &f.owner.public_key().to_hex(),
            &f.repo,
            &parent,
            limits(push.hydrated_bytes()),
        )
        .await
        .unwrap();
        assert_eq!(f.snapshot().await.manifest.head, "refs/heads/release/v1");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Add a reachable host alias for the same tenant solely in this fixture.
        sqlx::query("UPDATE communities SET host = $1 WHERE id = $2")
            .bind(addr.to_string())
            .bind(f.tenant.community().as_uuid())
            .execute(&f.pool)
            .await
            .unwrap();
        let router = super::super::super::transport::git_router(f.state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let repo_url = format!(
            "http://{addr}/git/{}/{}",
            f.owner.public_key().to_hex(),
            f.repo
        );
        let auth = format!(
            "http.extraHeader=Authorization: {}",
            token(&f.owner, "GET", &repo_url, None)
        );
        let refs = git(
            f.scratch.path(),
            &["-c", &auth, "ls-remote", "--symref", &repo_url, "HEAD"],
        )
        .await;
        assert!(refs.contains("ref: refs/heads/release/v1\tHEAD"), "{refs}");
        git(
            f.scratch.path(),
            &["-c", &auth, "clone", &repo_url, "clone"],
        )
        .await;
        assert_eq!(
            git(&f.scratch.path().join("clone"), &["symbolic-ref", "HEAD"])
                .await
                .trim(),
            "refs/heads/release/v1"
        );
        assert_eq!(
            std::fs::read(f.scratch.path().join("clone/main.txt")).unwrap(),
            b"selected branch\n"
        );
        server.abort();
    }

    struct UnavailableReplayGuard;

    impl buzz_auth::Nip98ReplayGuard for UnavailableReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async {
                Err(buzz_auth::AuthError::Nip98Invalid(
                    "injected backend failure".into(),
                ))
            })
        }
    }

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and MinIO"]
    async fn default_branch_replay_outage_and_deletion_fail_closed() {
        let mut f = Fixture::new().await;
        let before = f.snapshot().await.digest;
        let original = f.state.clone();
        let mut state = (*original).clone();
        state.nip98_replay = Arc::new(UnavailableReplayGuard);
        f.state = Arc::new(state);
        for body in [
            None,
            Some(json!({"branch":"main", "expected_manifest":before})),
        ] {
            let (status, body) = f.call(&f.owner, body, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
            assert!(body.to_string().contains("replay check unavailable"));
        }
        assert_eq!(f.snapshot().await.digest, before);
        f.state = original;
        // Enter the deletion executor's transaction scope in this disposable
        // fixture; the DB correctly rejects unfenced ad-hoc state changes.
        let mut tx = f.pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('buzz.deletion_executor_community', $1, true), set_config('buzz.deletion_fence_generation', '0', true)")
            .bind(f.tenant.community().to_string())
            .execute(&mut *tx).await.unwrap();
        sqlx::query("UPDATE communities SET deletion_state = 'quiescing' WHERE id = $1")
            .bind(f.tenant.community().as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_ne!(f.set(&f.owner, "main", None).await.0, StatusCode::OK);
        assert_eq!(f.snapshot().await.digest, before);
    }
}
