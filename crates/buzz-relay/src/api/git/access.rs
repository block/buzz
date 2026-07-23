//! Repository visibility metadata and current-request access checks.
//!
//! Read privacy is explicit: only a kind:30617 announcement carrying exactly
//! one `["buzz-visibility", "private"]` tag and one valid `buzz-channel` UUID
//! is private. Existing `buzz-channel` tags without that opt-in continue to
//! affect push policy only and retain legacy public-read behavior.
//!
//! The current kind:30617 announcement is authoritative. Private discovery and
//! Git access allow the repository key, its verified managed-agent owner, or a
//! current member of the bound channel. Membership is read for every request so
//! removals take effect immediately. Malformed metadata and lookup errors fail
//! closed; Smart HTTP denials use the same 404 response as a missing repository.

use anyhow::Context;
use nostr::Event;
use uuid::Uuid;

use buzz_core::kind::{event_kind_u32, KIND_GIT_REPO_ANNOUNCEMENT, KIND_GIT_REPO_STATE};
use buzz_core::TenantContext;
use buzz_db::EventQuery;

use crate::state::AppState;

/// Return the channel that gates an explicitly private repository.
///
/// Private announcements must carry exactly one well-formed channel UUID.
/// Conflicting private metadata is rejected. An absent visibility tag, or a
/// visibility value other than `private`, preserves legacy public-read
/// behavior even when a `buzz-channel` push binding is present.
pub(crate) fn private_repository_channel(event: &Event) -> anyhow::Result<Option<Uuid>> {
    let visibility_tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.first().map(String::as_str) == Some("buzz-visibility"))
        .collect();
    if !visibility_tags.is_empty() {
        if visibility_tags.len() != 1 || visibility_tags[0].len() != 2 {
            anyhow::bail!("repository requires at most one two-part buzz-visibility tag");
        }
        if visibility_tags[0][1] != "private" {
            anyhow::bail!("unsupported buzz-visibility value");
        }
    }
    let private_visibility = !visibility_tags.is_empty();

    if !private_visibility {
        return Ok(None);
    }
    let channel_tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.first().map(String::as_str) == Some("buzz-channel"))
        .collect();
    if channel_tags.len() != 1 || channel_tags[0].len() != 2 {
        anyhow::bail!("private repository requires exactly one buzz-channel UUID");
    }

    Uuid::parse_str(&channel_tags[0][1])
        .map(Some)
        .map_err(|_| anyhow::anyhow!("private repository buzz-channel must be a valid UUID"))
}

/// Validate the private binding on a kind:30617 announcement before storage.
///
/// A private repository may only be bound to a channel its announcing owner
/// currently belongs to. Parsing and membership policy stay in this module so
/// ingest does not grow a second implementation of the access boundary.
pub(crate) async fn validate_private_repository_announcement(
    state: &AppState,
    tenant: &TenantContext,
    event: &Event,
) -> anyhow::Result<()> {
    let Some(channel_id) = private_repository_channel(event)? else {
        return Ok(());
    };

    let owner = event.pubkey.to_bytes();
    let role = state
        .db
        .get_member_role(tenant.community(), channel_id, &owner)
        .await
        .context("check private repository owner channel membership")?;
    if role.is_none() {
        anyhow::bail!("private repository owner must be a current member of buzz-channel");
    }

    Ok(())
}

/// Extract one exact two-element string tag.
///
/// Duplicate, missing, or extended forms are rejected so authorization never
/// depends on whichever conflicting value happened to be observed first.
fn exact_tag_value<'a>(event: &'a Event, name: &str) -> anyhow::Result<&'a str> {
    let tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.first().map(String::as_str) == Some(name))
        .collect();
    if tags.len() != 1 || tags[0].len() != 2 {
        anyhow::bail!("repository discovery event requires exactly one {name} tag");
    }
    Ok(tags[0][1].as_str())
}

/// Resolve the kind:30617 repository key represented by a discovery event.
///
/// A kind:30617 is authored by the repository key directly. Relay-signed
/// kind:30618 events instead identify that repository key in their only `p`
/// tag and reuse its `d` tag. Any malformed state fails closed.
fn discovery_repository_key(event: &Event) -> anyhow::Result<Option<(Vec<u8>, &str)>> {
    let repo_id = match event_kind_u32(event) {
        KIND_GIT_REPO_ANNOUNCEMENT | KIND_GIT_REPO_STATE => exact_tag_value(event, "d")?,
        _ => return Ok(None),
    };

    let owner = if event_kind_u32(event) == KIND_GIT_REPO_ANNOUNCEMENT {
        event.pubkey.to_bytes().to_vec()
    } else {
        let owner_hex = exact_tag_value(event, "p")?;
        let owner = hex::decode(owner_hex).context("kind:30618 p tag is not hex")?;
        if owner.len() != 32 {
            anyhow::bail!("kind:30618 p tag must be a 32-byte public key");
        }
        owner
    };

    Ok(Some((owner, repo_id)))
}

/// Decide whether one kind:30617/30618 event is visible to `requester`.
///
/// Non-repository events are unchanged. Repository discovery events reuse the
/// same live authorization boundary as Smart HTTP. Errors and malformed
/// repository-state links fail closed, preventing a query/fan-out bypass.
pub(crate) async fn requester_can_discover_repository_event(
    state: &AppState,
    tenant: &TenantContext,
    event: &Event,
    requester: &[u8],
) -> bool {
    let key = match discovery_repository_key(event) {
        Ok(None) => return true,
        Ok(Some(key)) => key,
        Err(error) => {
            tracing::warn!(
                event_id = %event.id.to_hex(),
                error = %error,
                "repository discovery event authorization failed closed"
            );
            return false;
        }
    };

    match requester_can_access_repository(state, tenant, &key.0, key.1, requester).await {
        Ok(allowed) => allowed,
        Err(error) => {
            tracing::warn!(
                event_id = %event.id.to_hex(),
                error = %error,
                "repository discovery event authorization failed closed"
            );
            false
        }
    }
}

/// Return whether a filter might match kind:30617 or kind:30618.
///
/// COUNT must use per-event filtering whenever this is true; the fast SQL count
/// cannot subtract private repositories that the requester cannot discover.
pub(crate) fn filter_can_match_repository_discovery(filter: &nostr::Filter) -> bool {
    filter.kinds.as_ref().is_none_or(|kinds| {
        kinds.iter().any(|kind| {
            matches!(
                kind.as_u16() as u32,
                KIND_GIT_REPO_ANNOUNCEMENT | KIND_GIT_REPO_STATE
            )
        })
    })
}

/// Decide whether `requester` may discover or use the current repository.
///
/// The current kind:30617 event is authoritative. Public repositories retain
/// existing relay-member access. Private repositories allow the repository
/// key, its verified managed-agent owner, or a current member of the bound
/// channel. Membership is queried on every request so removals take effect on
/// the next request. Missing or malformed announcements fail closed.
pub(crate) async fn requester_can_access_repository(
    state: &AppState,
    tenant: &TenantContext,
    owner: &[u8],
    repo_id: &str,
    requester: &[u8],
) -> anyhow::Result<bool> {
    let query = EventQuery {
        kinds: Some(vec![KIND_GIT_REPO_ANNOUNCEMENT as i32]),
        pubkey: Some(owner.to_vec()),
        d_tag: Some(repo_id.to_owned()),
        global_only: true,
        limit: Some(1),
        ..EventQuery::for_community(tenant.community())
    };
    let Some(repo_event) = state
        .db
        .query_events(&query)
        .await
        .context("query current repository announcement")?
        .pop()
    else {
        return Ok(false);
    };

    let channel_id = match private_repository_channel(&repo_event.event) {
        Ok(None) => return Ok(true),
        Ok(Some(channel_id)) => channel_id,
        Err(_) => return Ok(false),
    };

    if requester == owner {
        return Ok(true);
    }
    if state
        .db
        .is_agent_owner(tenant.community(), owner, requester)
        .await
        .context("check managed-agent repository ownership")?
    {
        return Ok(true);
    }

    state
        .db
        .get_member_role(tenant.community(), channel_id, requester)
        .await
        .map(|role| role.is_some())
        .context("check private repository channel membership")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn repo_announcement_with_keys(keys: &Keys, repo_id: &str, tags: Vec<Tag>) -> Event {
        let mut all_tags = vec![Tag::parse(["d", repo_id]).expect("d tag")];
        all_tags.extend(tags);
        EventBuilder::new(Kind::Custom(KIND_GIT_REPO_ANNOUNCEMENT as u16), "")
            .tags(all_tags)
            .sign_with_keys(keys)
            .expect("sign repo announcement")
    }

    fn repo_announcement(tags: Vec<Tag>) -> Event {
        repo_announcement_with_keys(&Keys::generate(), "test-repo", tags)
    }

    #[test]
    fn buzz_channel_without_private_visibility_remains_public() {
        let channel_id = Uuid::new_v4().to_string();
        let event = repo_announcement(vec![
            Tag::parse(["buzz-channel", channel_id.as_str()]).expect("channel tag")
        ]);

        assert_eq!(
            private_repository_channel(&event).expect("parse visibility"),
            None
        );
    }

    #[test]
    fn private_visibility_requires_exactly_one_valid_channel() {
        let missing_channel = repo_announcement(vec![
            Tag::parse(["buzz-visibility", "private"]).expect("visibility tag")
        ]);
        assert!(private_repository_channel(&missing_channel)
            .expect_err("missing channel must fail")
            .to_string()
            .contains("exactly one buzz-channel UUID"));

        let invalid_channel = repo_announcement(vec![
            Tag::parse(["buzz-visibility", "private"]).expect("visibility tag"),
            Tag::parse(["buzz-channel", "not-a-uuid"]).expect("channel tag"),
        ]);
        assert!(private_repository_channel(&invalid_channel)
            .expect_err("invalid channel must fail")
            .to_string()
            .contains("valid UUID"));

        let channel_a = Uuid::new_v4().to_string();
        let channel_b = Uuid::new_v4().to_string();
        let conflicting_channels = repo_announcement(vec![
            Tag::parse(["buzz-visibility", "private"]).expect("visibility tag"),
            Tag::parse(["buzz-channel", channel_a.as_str()]).expect("channel tag"),
            Tag::parse(["buzz-channel", channel_b.as_str()]).expect("channel tag"),
        ]);
        assert!(private_repository_channel(&conflicting_channels)
            .expect_err("conflicting channels must fail")
            .to_string()
            .contains("exactly one buzz-channel UUID"));
    }

    #[test]
    fn malformed_or_unsupported_visibility_fails_closed() {
        let channel = Uuid::new_v4().to_string();
        for tags in [
            vec![Tag::parse(["buzz-visibility", "public"]).expect("visibility")],
            vec![
                Tag::parse(["buzz-visibility", "private"]).expect("visibility"),
                Tag::parse(["buzz-visibility", "public"]).expect("visibility"),
                Tag::parse(["buzz-channel", channel.as_str()]).expect("channel"),
            ],
            vec![
                Tag::parse(["buzz-visibility", "private", "extra"]).expect("visibility"),
                Tag::parse(["buzz-channel", channel.as_str()]).expect("channel"),
            ],
        ] {
            assert!(private_repository_channel(&repo_announcement(tags)).is_err());
        }
    }

    #[test]
    fn private_visibility_resolves_channel() {
        let channel_id = Uuid::new_v4();
        let channel_id_string = channel_id.to_string();
        let event = repo_announcement(vec![
            Tag::parse(["buzz-visibility", "private"]).expect("visibility tag"),
            Tag::parse(["buzz-channel", channel_id_string.as_str()]).expect("channel tag"),
        ]);

        assert_eq!(
            private_repository_channel(&event).expect("parse private binding"),
            Some(channel_id)
        );
    }
    #[test]
    fn discovery_key_uses_announcement_author_and_state_p_tag() {
        let owner = Keys::generate();
        let relay = Keys::generate();
        let announcement = repo_announcement_with_keys(&owner, "test-repo", Vec::new());
        let (announcement_owner, repo_id) = discovery_repository_key(&announcement)
            .expect("announcement key")
            .expect("repository discovery event");
        assert_eq!(announcement_owner, owner.public_key().to_bytes());
        assert_eq!(repo_id, "test-repo");

        let owner_hex = owner.public_key().to_hex();
        let state = EventBuilder::new(Kind::Custom(KIND_GIT_REPO_STATE as u16), "")
            .tags([
                Tag::parse(["d", "test-repo"]).expect("d tag"),
                Tag::parse(["p", owner_hex.as_str()]).expect("p tag"),
            ])
            .sign_with_keys(&relay)
            .expect("sign state event");
        let (state_owner, repo_id) = discovery_repository_key(&state)
            .expect("state key")
            .expect("repository discovery event");
        assert_eq!(state_owner, owner.public_key().to_bytes());
        assert_eq!(repo_id, "test-repo");
    }

    #[test]
    fn malformed_state_link_fails_closed() {
        let relay = Keys::generate();
        let duplicate_owner_a = Keys::generate().public_key().to_hex();
        let duplicate_owner_b = Keys::generate().public_key().to_hex();
        for tags in [
            vec![Tag::parse(["d", "test-repo"]).expect("d tag")],
            vec![
                Tag::parse(["d", "test-repo"]).expect("d tag"),
                Tag::parse(["p", "not-a-pubkey"]).expect("p tag"),
            ],
            vec![
                Tag::parse(["d", "test-repo"]).expect("d tag"),
                Tag::parse(["p", duplicate_owner_a.as_str()]).expect("p tag"),
                Tag::parse(["p", duplicate_owner_b.as_str()]).expect("p tag"),
            ],
        ] {
            let event = EventBuilder::new(Kind::Custom(KIND_GIT_REPO_STATE as u16), "")
                .tags(tags)
                .sign_with_keys(&relay)
                .expect("sign state event");
            assert!(discovery_repository_key(&event).is_err());
        }
    }

    #[test]
    fn repository_discovery_filter_detection_covers_wildcard_and_both_kinds() {
        assert!(filter_can_match_repository_discovery(&nostr::Filter::new()));
        assert!(filter_can_match_repository_discovery(
            &nostr::Filter::new().kind(Kind::Custom(KIND_GIT_REPO_ANNOUNCEMENT as u16))
        ));
        assert!(filter_can_match_repository_discovery(
            &nostr::Filter::new().kind(Kind::Custom(KIND_GIT_REPO_STATE as u16))
        ));
        assert!(!filter_can_match_repository_discovery(
            &nostr::Filter::new().kind(Kind::TextNote)
        ));
    }

    #[derive(Clone)]
    pub(crate) struct RepositoryAccessFixture {
        pub state: Arc<AppState>,
        pub pool: sqlx::PgPool,
        pub tenant: TenantContext,
        pub owner: Keys,
        pub member: Keys,
        pub outsider: Keys,
        pub private_announcement: Event,
        pub private_state: Event,
        pub public_announcement: Event,
    }

    pub(crate) async fn repository_access_fixture(
        redis_url: &str,
    ) -> Option<RepositoryAccessFixture> {
        use buzz_core::channel::MemberRole;
        use buzz_db::channel::{ChannelType, ChannelVisibility};

        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned()); // sadscan:disable np.postgres.1
        let pool = sqlx::PgPool::connect(&database_url).await.ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        db.migrate().await.ok()?;
        let host = format!("git-discovery-test-{}.example", Uuid::new_v4().simple());
        let record = db.ensure_configured_community(&host).await.ok()?;
        let tenant = TenantContext::resolved(record.id, host);

        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = database_url;
        config.redis_url = redis_url.to_owned();
        config.require_relay_membership = false;
        let redis_pool = deadpool_redis::Config::from_url(redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
        let relay_keys = Keys::generate();
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            relay_keys.clone(),
            media_storage,
        );
        let state = Arc::new(state);

        let owner = Keys::generate();
        let member = Keys::generate();
        let outsider = Keys::generate();
        let channel = state
            .db
            .create_channel(
                tenant.community(),
                "repository-discovery-test",
                ChannelType::Stream,
                ChannelVisibility::Private,
                None,
                owner.public_key().as_bytes(),
                None,
            )
            .await
            .ok()?;
        state
            .db
            .add_member(
                tenant.community(),
                channel.id,
                member.public_key().as_bytes(),
                MemberRole::Member,
                Some(owner.public_key().as_bytes()),
            )
            .await
            .ok()?;

        let private_announcement = repo_announcement_with_keys(
            &owner,
            "private-repo",
            vec![
                Tag::parse(["buzz-visibility", "private"]).ok()?,
                Tag::parse(["buzz-channel", channel.id.to_string().as_str()]).ok()?,
            ],
        );
        state
            .db
            .insert_event(tenant.community(), &private_announcement, None)
            .await
            .ok()?;
        let owner_hex = owner.public_key().to_hex();
        let private_state = EventBuilder::new(Kind::Custom(KIND_GIT_REPO_STATE as u16), "")
            .tags([
                Tag::parse(["d", "private-repo"]).ok()?,
                Tag::parse(["p", owner_hex.as_str()]).ok()?,
            ])
            .sign_with_keys(&relay_keys)
            .ok()?;
        state
            .db
            .insert_event(tenant.community(), &private_state, None)
            .await
            .ok()?;

        let public_announcement = repo_announcement_with_keys(
            &owner,
            "public-repo",
            vec![Tag::parse(["buzz-channel", channel.id.to_string().as_str()]).ok()?],
        );
        state
            .db
            .insert_event(tenant.community(), &public_announcement, None)
            .await
            .ok()?;

        Some(RepositoryAccessFixture {
            state,
            pool,
            tenant,
            owner,
            member,
            outsider,
            private_announcement,
            private_state,
            public_announcement,
        })
    }

    #[tokio::test]
    #[ignore = "requires Postgres and Redis"]
    async fn private_announcement_requires_current_owner_membership() {
        let Some(fixture) = repository_access_fixture("redis://127.0.0.1:6379").await else {
            return;
        };

        validate_private_repository_announcement(
            &fixture.state,
            &fixture.tenant,
            &fixture.private_announcement,
        )
        .await
        .expect("channel owner may publish private announcement");

        let channel_id = private_repository_channel(&fixture.private_announcement)
            .expect("parse private binding")
            .expect("private channel");
        let outsider_announcement = repo_announcement_with_keys(
            &fixture.outsider,
            "outsider-private-repo",
            vec![
                Tag::parse(["buzz-visibility", "private"]).expect("visibility"),
                Tag::parse(["buzz-channel", channel_id.to_string().as_str()]).expect("channel"),
            ],
        );
        let error = validate_private_repository_announcement(
            &fixture.state,
            &fixture.tenant,
            &outsider_announcement,
        )
        .await
        .expect_err("non-member owner must be rejected");
        assert!(error
            .to_string()
            .contains("private repository owner must be a current member of buzz-channel"));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn repository_access_tracks_membership_and_legacy_visibility() {
        use buzz_core::channel::MemberRole;
        use buzz_core::CommunityId;
        use buzz_db::channel::{ChannelType, ChannelVisibility};

        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned()); // sadscan:disable np.postgres.1
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect to test Postgres");
        let db = buzz_db::Db::from_pool(pool.clone());
        db.migrate().await.expect("migrate test Postgres");
        let mut config = crate::config::Config::from_env().expect("default config");
        config.database_url = database_url;
        config.redis_url = "redis://127.0.0.1:1".to_owned();
        config.require_relay_membership = false;
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        let state = Arc::new(state);

        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        let host = format!("git-access-test-{}.example", community_uuid.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("insert community");
        let tenant = TenantContext::resolved(community, host);

        let owner = Keys::generate();
        let member = Keys::generate();
        let outsider = Keys::generate();
        let managed_agent = Keys::generate();
        let managed_agent_owner = Keys::generate();
        state
            .db
            .ensure_user(community, managed_agent.public_key().as_bytes())
            .await
            .expect("ensure managed agent");
        state
            .db
            .ensure_user(community, managed_agent_owner.public_key().as_bytes())
            .await
            .expect("ensure managed-agent owner");
        assert!(state
            .db
            .set_agent_owner(
                community,
                managed_agent.public_key().as_bytes(),
                managed_agent_owner.public_key().as_bytes(),
            )
            .await
            .expect("set managed-agent owner"));
        let channel = state
            .db
            .create_channel(
                community,
                "private-repo-test",
                ChannelType::Stream,
                ChannelVisibility::Private,
                None,
                owner.public_key().as_bytes(),
                None,
            )
            .await
            .expect("create channel");
        state
            .db
            .add_member(
                community,
                channel.id,
                member.public_key().as_bytes(),
                MemberRole::Member,
                Some(owner.public_key().as_bytes()),
            )
            .await
            .expect("add member");

        let private_event = repo_announcement_with_keys(
            &owner,
            "private-repo",
            vec![
                Tag::parse(["buzz-visibility", "private"]).expect("visibility"),
                Tag::parse(["buzz-channel", channel.id.to_string().as_str()]).expect("channel"),
            ],
        );
        state
            .db
            .insert_event(community, &private_event, None)
            .await
            .expect("insert private announcement");

        for requester in [owner.public_key(), member.public_key()] {
            assert!(requester_can_access_repository(
                &state,
                &tenant,
                owner.public_key().as_bytes(),
                "private-repo",
                requester.as_bytes(),
            )
            .await
            .expect("check allowed requester"));
        }
        assert!(!requester_can_access_repository(
            &state,
            &tenant,
            owner.public_key().as_bytes(),
            "private-repo",
            outsider.public_key().as_bytes(),
        )
        .await
        .expect("check outsider"));

        let managed_private_event = repo_announcement_with_keys(
            &managed_agent,
            "managed-private-repo",
            vec![
                Tag::parse(["buzz-visibility", "private"]).expect("visibility"),
                Tag::parse(["buzz-channel", channel.id.to_string().as_str()]).expect("channel"),
            ],
        );
        state
            .db
            .insert_event(community, &managed_private_event, None)
            .await
            .expect("insert managed private announcement");
        assert!(requester_can_access_repository(
            &state,
            &tenant,
            managed_agent.public_key().as_bytes(),
            "managed-private-repo",
            managed_agent_owner.public_key().as_bytes(),
        )
        .await
        .expect("managed-agent owner access"));

        let private_owner_hex = owner.public_key().to_hex();
        let private_state = EventBuilder::new(Kind::Custom(KIND_GIT_REPO_STATE as u16), "")
            .tags([
                Tag::parse(["d", "private-repo"]).expect("d tag"),
                Tag::parse(["p", private_owner_hex.as_str()]).expect("p tag"),
            ])
            .sign_with_keys(&state.relay_keypair)
            .expect("sign private state event");
        assert!(
            requester_can_discover_repository_event(
                &state,
                &tenant,
                &private_state,
                member.public_key().as_bytes(),
            )
            .await
        );
        assert!(
            !requester_can_discover_repository_event(
                &state,
                &tenant,
                &private_state,
                outsider.public_key().as_bytes(),
            )
            .await
        );

        state
            .db
            .remove_member(
                community,
                channel.id,
                member.public_key().as_bytes(),
                owner.public_key().as_bytes(),
            )
            .await
            .expect("remove member");
        assert!(!requester_can_access_repository(
            &state,
            &tenant,
            owner.public_key().as_bytes(),
            "private-repo",
            member.public_key().as_bytes(),
        )
        .await
        .expect("check revoked member"));

        let legacy_event = repo_announcement_with_keys(
            &owner,
            "legacy-repo",
            vec![Tag::parse(["buzz-channel", channel.id.to_string().as_str()]).expect("channel")],
        );
        state
            .db
            .insert_event(community, &legacy_event, None)
            .await
            .expect("insert legacy announcement");
        assert!(requester_can_access_repository(
            &state,
            &tenant,
            owner.public_key().as_bytes(),
            "legacy-repo",
            outsider.public_key().as_bytes(),
        )
        .await
        .expect("check legacy public access"));

        // The test community uses a random host/UUID and remains isolated in
        // the disposable integration-test database. Deleting it directly would
        // violate the channel foreign key and production has no cascade here.
    }
}
