//! Postgres-backed end-to-end tests for system channels: every read path
//! that does not name a system channel leaves it out, and naming it reads it
//! under the ordinary rules (member, or open channel).

use std::sync::Arc;

use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use uuid::Uuid;

use super::group_state_postgres_tests::{create_channel, edit_channel, http_auth, test_conn};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
    host: String,
}

async fn fixture() -> Fixture {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let host = format!("system-channels-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    let tenant = TenantContext::resolved(community, &host);
    Fixture {
        state,
        tenant,
        host,
    }
}

/// Ingest an event that `keys` signs and return its ID.
async fn publish(f: &Fixture, keys: &Keys, kind: u16, content: &str, tags: &[&[&str]]) -> String {
    let tags: Vec<Tag> = tags
        .iter()
        .map(|tag| Tag::parse(tag.iter().copied()).unwrap())
        .collect();
    let event = EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign");
    let id = event.id.to_hex();
    let result = ingest_event(&f.state, &f.tenant, event, http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("ingest kind:{kind}: {error:?}"));
    assert!(result.accepted, "kind:{kind} refused: {}", result.message);
    id
}

/// The current relay-signed group-state event ID of `kind` for `channel`.
async fn group_state_id(f: &Fixture, channel: Uuid, kind: i32) -> String {
    f.state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![kind]),
            d_tag: Some(channel.to_string()),
            limit: Some(1),
            ..EventQuery::for_community(f.tenant.community())
        })
        .await
        .expect("query group state")
        .first()
        .unwrap_or_else(|| panic!("no kind:{kind} for {channel}"))
        .event
        .id
        .to_hex()
}

/// Run one historical REQ and return the event IDs sent before EOSE. A
/// CLOSED reply counts as no events.
async fn req(f: &Fixture, reader: &Keys, filter: Value) -> Vec<String> {
    req_all(f, reader, json!([filter])).await
}

/// [`req`] with several filters in one REQ.
async fn req_all(f: &Fixture, reader: &Keys, filters: Value) -> Vec<String> {
    let (conn, mut rx) = test_conn(&f.tenant, reader);
    let filters: Vec<nostr::Filter> = serde_json::from_value(filters).expect("filters");
    let before = vec![None; filters.len()];
    crate::handlers::req::handle_req("s".into(), filters, before, conn, f.state.clone()).await;
    let mut ids = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("EVENT") => ids.push(message[2]["id"].as_str().expect("id").to_owned()),
            Some("EOSE") | Some("CLOSED") => return ids,
            other => panic!("unexpected frame {other:?}: {text}"),
        }
    }
    panic!("no EOSE; events so far: {ids:?}");
}

/// Run one WebSocket COUNT and return the count. A CLOSED reply counts as 0.
async fn count(f: &Fixture, reader: &Keys, filter: Value) -> u64 {
    count_all(f, reader, json!([filter])).await
}

/// [`count`] with several filters in one COUNT.
async fn count_all(f: &Fixture, reader: &Keys, filters: Value) -> u64 {
    let (conn, mut rx) = test_conn(&f.tenant, reader);
    let filters: Vec<nostr::Filter> = serde_json::from_value(filters).expect("filters");
    crate::handlers::count::handle_count("c".into(), filters, conn, f.state.clone()).await;
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("COUNT") => return message[2]["count"].as_u64().expect("count"),
            Some("CLOSED") => return 0,
            other => panic!("unexpected frame {other:?}: {text}"),
        }
    }
    panic!("no COUNT reply");
}

/// POST `filters` to an HTTP bridge route as `reader`. Returns the JSON body
/// of a 200, or `None` for a refusal.
async fn bridge(f: &Fixture, reader: &Keys, path: &str, filters: Value) -> Option<Value> {
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    let response = crate::router::build_router(f.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::HOST, &f.host)
                .header("x-pubkey", reader.public_key().to_hex())
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&filters).unwrap()))
                .expect("request"),
        )
        .await
        .expect("router");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    if status.is_success() {
        Some(serde_json::from_slice(&body).expect("json body"))
    } else {
        assert!(
            status.is_client_error(),
            "{path}: {status} {}",
            String::from_utf8_lossy(&body)
        );
        None
    }
}

fn bridge_ids(body: Option<Value>) -> Vec<String> {
    let Some(body) = body else {
        return Vec::new();
    };
    let events = body
        .as_array()
        .or_else(|| body["events"].as_array())
        .unwrap_or_else(|| panic!("bridge query body: {body}"));
    events
        .iter()
        .map(|event| event["id"].as_str().expect("id").to_owned())
        .collect()
}

fn bridge_count(body: Option<Value>) -> u64 {
    body.map_or(0, |body| {
        body["count"]
            .as_u64()
            .unwrap_or_else(|| panic!("bridge count body: {body}"))
    })
}

fn sorted<T: Ord>(mut ids: Vec<T>) -> Vec<T> {
    ids.sort();
    ids
}

/// The relay leaves system channels out of every read that does not name
/// them, and a read that names one follows the ordinary rules.
/// Mutation: drop `channel_type <> 'system'` from either half of
/// `get_accessible_channel_ids` → RED (implicit reads list them). Drop the
/// named-channel scope → RED (members cannot read their own system channel).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn system_channels_are_read_only_by_name() {
    let f = fixture().await;
    let owner = Keys::generate();
    let outsider = Keys::generate();

    let private = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    let open = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        open,
        &[&["visibility", "open"]],
    )
    .await
    .expect("open the system channel");
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        ordinary,
        &[&["visibility", "open"]],
    )
    .await
    .expect("open the ordinary channel");

    let word = format!("needle{}", Uuid::new_v4().simple());
    let mut message = std::collections::HashMap::new();
    for channel in [private, open, ordinary] {
        let id = publish(&f, &owner, 9, &word, &[&["h", &channel.to_string()]]).await;
        message.insert(channel, id);
    }
    let ordinary_message = vec![message[&ordinary].clone()];

    for reader in [&owner, &outsider] {
        let who = if std::ptr::eq(reader, &owner) {
            "member"
        } else {
            "outsider"
        };

        // Implicit scope: no channel named.
        assert_eq!(
            req(&f, reader, json!({"kinds": [9]})).await,
            ordinary_message,
            "{who}: REQ without #h"
        );
        assert_eq!(
            req(&f, reader, json!({"kinds": [9], "search": word})).await,
            ordinary_message,
            "{who}: NIP-50 search"
        );
        assert_eq!(
            count(&f, reader, json!({"kinds": [9]})).await,
            1,
            "{who}: COUNT"
        );
        assert_eq!(
            bridge_ids(bridge(&f, reader, "/query", json!([{"kinds": [9]}])).await),
            ordinary_message,
            "{who}: bridge /query"
        );
        assert_eq!(
            bridge_ids(
                bridge(
                    &f,
                    reader,
                    "/query",
                    json!([{"kinds": [9], "search": word}])
                )
                .await
            ),
            ordinary_message,
            "{who}: bridge search"
        );
        assert_eq!(
            bridge_count(bridge(&f, reader, "/count", json!([{"kinds": [9]}])).await),
            1,
            "{who}: bridge /count"
        );
        assert_eq!(
            req(&f, reader, json!({"kinds": [39000]})).await,
            vec![group_state_id(&f, ordinary, 39000).await],
            "{who}: channel list"
        );

        // Named scope: member, or the channel is open.
        for (channel, readable) in [(private, std::ptr::eq(reader, &owner)), (open, true)] {
            let h = channel.to_string();
            let expected_message = if readable {
                vec![message[&channel].clone()]
            } else {
                Vec::new()
            };
            let expected_count = u64::from(readable);
            assert_eq!(
                req(&f, reader, json!({"kinds": [9], "#h": [h]})).await,
                expected_message,
                "{who}: REQ #h {channel}"
            );
            assert_eq!(
                count(&f, reader, json!({"kinds": [9], "#h": [h]})).await,
                expected_count,
                "{who}: COUNT #h {channel}"
            );
            assert_eq!(
                bridge_ids(bridge(&f, reader, "/query", json!([{"kinds": [9], "#h": [h]}])).await),
                expected_message,
                "{who}: bridge /query #h {channel}"
            );
            assert_eq!(
                bridge_count(
                    bridge(&f, reader, "/count", json!([{"kinds": [9], "#h": [h]}])).await
                ),
                expected_count,
                "{who}: bridge /count #h {channel}"
            );
            assert_eq!(
                req(&f, reader, json!({"kinds": [9], "#h": [h], "search": word})).await,
                expected_message,
                "{who}: search #h {channel}"
            );
            let expected_metadata = if readable {
                vec![group_state_id(&f, channel, 39000).await]
            } else {
                Vec::new()
            };
            assert_eq!(
                req(&f, reader, json!({"kinds": [39000], "#d": [h]})).await,
                expected_metadata,
                "{who}: metadata #d {channel}"
            );
        }
    }

    // The member's own channel list (kind:39002 #p:[me]) leaves them out.
    assert_eq!(
        req(
            &f,
            &owner,
            json!({"kinds": [39002], "#p": [owner.public_key().to_hex()]})
        )
        .await,
        vec![group_state_id(&f, ordinary, 39002).await],
        "member list by #p"
    );
}

/// A reader can join a private system channel after the request loads its
/// readable system channels and before it re-checks a named channel. The
/// re-check then confirms membership. The channel must stay in the scope of
/// the filter that names it, not the request-wide implicit set, so a sibling
/// filter that never names it cannot read it.
/// Mutation: drop `keep_repaired_named_only_channels_named` from
/// `repair_requested_channel_access` → RED (the sibling filter's scope gets
/// the channel).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn a_system_channel_joined_mid_request_stays_in_its_named_filter() {
    let f = fixture().await;
    let owner = Keys::generate();
    let system = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[]).await;

    let filters: Vec<nostr::Filter> = serde_json::from_value(json!([
        {"kinds": [9], "#h": [system.to_string()]},
        {"kinds": [39000]},
    ]))
    .expect("filters");
    let mut implicit = f
        .state
        .db
        .get_accessible_channel_ids(f.tenant.community(), &owner.public_key().to_bytes())
        .await
        .expect("implicit channels");
    assert_eq!(implicit, [ordinary]);
    // The request's system-channel lookup ran before the join.
    let mut named_only = crate::handlers::system_channels::NamedOnlyChannels::default();

    crate::api::bridge::repair_requested_channel_access(
        &f.state,
        &f.tenant,
        &filters,
        &owner.public_key().to_bytes(),
        &mut implicit,
        &mut named_only,
    )
    .await
    .expect("repair");

    assert_eq!(implicit, [ordinary], "implicit set stays ordinary");
    assert_eq!(named_only.ids(), [system]);
    let scope = |filter| {
        crate::handlers::system_channels::filter_channel_scope(filter, &implicit, &named_only)
            .into_owned()
    };
    assert_eq!(scope(&filters[0]), [ordinary, system], "naming filter");
    assert_eq!(scope(&filters[1]), [ordinary], "sibling filter");
}

/// A [`Fixture`] on a migrated scratch database with a `future_type` value in
/// `channel_type`, as a later migration would add, so the shared test
/// database keeps its schema.
struct LaterTypeFixture {
    f: Fixture,
    pool: sqlx::PgPool,
    admin: sqlx::PgPool,
    name: String,
}

impl LaterTypeFixture {
    async fn new() -> Self {
        let url = crate::test_support::database_url();
        let admin = sqlx::PgPool::connect(&url).await.expect("connect admin");
        let name = format!("later_type_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch db");
        let scratch_url = format!("{}/{name}", &url[..url.rfind('/').expect("db url path")]);
        let pool = sqlx::PgPool::connect(&scratch_url)
            .await
            .expect("connect scratch db");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch db");
        sqlx::query("ALTER TYPE channel_type ADD VALUE 'future_type'")
            .execute(&pool)
            .await
            .expect("add future channel type");
        let state = crate::api::bridge::postgres_tests::bridge_handler_test_state_on(&scratch_url)
            .await
            .expect("scratch Postgres and local Redis");
        let host = format!("later-type-{}.local", Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("ensure community")
            .id;
        let tenant = TenantContext::resolved(community, &host);
        Self {
            f: Fixture {
                state,
                tenant,
                host,
            },
            pool,
            admin,
            name,
        }
    }

    /// Give `channel` the later type, and drop cached access sets built
    /// while it was a stream.
    async fn retype(&self, channel: Uuid) {
        sqlx::query("UPDATE channels SET channel_type = 'future_type' WHERE id = $1")
            .bind(channel)
            .execute(&self.pool)
            .await
            .expect("retype channel");
        self.f
            .state
            .invalidate_all_accessible_channels_local(self.f.tenant.community());
    }

    async fn drop_database(self) {
        drop(self.f);
        self.pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )))
        .execute(&self.admin)
        .await;
    }
}

/// A filter that names a channel of a type added later reads it under the
/// ordinary rules (member, or open), by `#h` on any kinds or by `#d` on
/// group-state kinds, on every read path. No sibling filter in the same
/// request gains it, including a group-state filter that opts in to system
/// channels with `#P` or `#t:["system", <label>]` (NIP-CL).
/// Mutation: load only `system` channels in
/// `get_readable_named_only_channels` → RED (an open channel's non-member,
/// and `#d`, read nothing). Add every named-only channel on opt-in in
/// `filter_channel_scope` → RED (the `#P` and `#t` siblings list it).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn channels_of_a_later_type_are_read_only_by_name() {
    let later = LaterTypeFixture::new().await;
    let f = &later.f;
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let label = "shared-label";

    let private = create_channel(
        &f.state,
        &f.tenant,
        &owner,
        &[&["visibility", "private"], &["t", label]],
    )
    .await;
    let open = create_channel(&f.state, &f.tenant, &owner, &[&["visibility", "open"]]).await;
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[&["t", label]]).await;
    let mut message = std::collections::HashMap::new();
    for channel in [private, open, ordinary] {
        let id = publish(f, &owner, 9, "hello", &[&["h", &channel.to_string()]]).await;
        message.insert(channel, id);
    }
    let ordinary_meta = group_state_id(f, ordinary, 39000).await;
    later.retype(private).await;
    later.retype(open).await;

    // A read that names no channel leaves both out.
    assert_eq!(
        req(f, &owner, json!({"kinds": [9]})).await,
        vec![message[&ordinary].clone()],
        "REQ without #h"
    );

    for reader in [&owner, &outsider] {
        let who = if std::ptr::eq(reader, &owner) {
            "member"
        } else {
            "outsider"
        };
        for (channel, readable) in [(private, std::ptr::eq(reader, &owner)), (open, true)] {
            let h = channel.to_string();
            let expected_message = if readable {
                vec![message[&channel].clone()]
            } else {
                Vec::new()
            };
            let expected_count = u64::from(readable);
            let by_h = json!({"kinds": [9], "#h": [h]});
            assert_eq!(
                req(f, reader, by_h.clone()).await,
                expected_message,
                "{who}: REQ #h {channel}"
            );
            assert_eq!(
                count(f, reader, by_h.clone()).await,
                expected_count,
                "{who}: COUNT #h {channel}"
            );
            assert_eq!(
                bridge_ids(bridge(f, reader, "/query", json!([by_h])).await),
                expected_message,
                "{who}: bridge /query #h {channel}"
            );
            assert_eq!(
                bridge_count(bridge(f, reader, "/count", json!([by_h])).await),
                expected_count,
                "{who}: bridge /count #h {channel}"
            );
            let expected_meta = if readable {
                vec![group_state_id(f, channel, 39000).await]
            } else {
                Vec::new()
            };
            let by_d = json!({"kinds": [39000], "#d": [h]});
            assert_eq!(
                req(f, reader, by_d.clone()).await,
                expected_meta,
                "{who}: REQ #d {channel}"
            );
            assert_eq!(
                bridge_ids(bridge(f, reader, "/query", json!([by_d])).await),
                expected_meta,
                "{who}: bridge /query #d {channel}"
            );
        }
    }

    // A sibling that opts in to system channels finds the creator's or the
    // label's ordinary channel, but not the later-type one that the first
    // filter names.
    for sibling in [
        json!({"kinds": [39000], "#P": [owner_hex]}),
        json!({"kinds": [39000], "#t": ["system", label]}),
    ] {
        let filters = json!([{"kinds": [9], "#h": [private.to_string()]}, sibling]);
        let expected = sorted(vec![message[&private].clone(), ordinary_meta.clone()]);
        assert_eq!(
            sorted(req_all(f, &owner, filters.clone()).await),
            expected,
            "REQ with {sibling}"
        );
        assert_eq!(
            count_all(f, &owner, filters.clone()).await,
            2,
            "COUNT with {sibling}"
        );
        assert_eq!(
            sorted(bridge_ids(
                bridge(f, &owner, "/query", filters.clone()).await
            )),
            expected,
            "bridge /query with {sibling}"
        );
        assert_eq!(
            bridge_count(bridge(f, &owner, "/count", filters).await),
            2,
            "bridge /count with {sibling}"
        );
    }

    later.drop_database().await;
}

/// A reader can join a channel of a type added later after the request loads
/// its named-only channels and before it re-checks a named channel. The
/// re-check must keep it in the scope of the filter that names it: a sibling
/// that names no channel reads only the legacy types, and a sibling that opts
/// in to system channels (`#P`) does not gain it either (NIP-CL).
/// Mutation: classify the repaired tail as legacy in
/// `keep_repaired_named_only_channels_named` → RED (the plain sibling gets
/// it); count every repaired named-only channel as system → RED (the `#P`
/// sibling gets it).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn a_named_channel_of_a_later_type_stays_in_its_named_filter() {
    let later_type = LaterTypeFixture::new().await;
    let (state, tenant) = (&later_type.f.state, &later_type.f.tenant);
    let community = tenant.community();

    let owner = Keys::generate();
    let ordinary = create_channel(state, tenant, &owner, &[]).await;
    let later = create_channel(state, tenant, &owner, &[]).await;
    later_type.retype(later).await;

    let filters: Vec<nostr::Filter> = serde_json::from_value(json!([
        {"kinds": [9], "#h": [later.to_string()]},
        {"kinds": [39000]},
        {"kinds": [39000], "#P": [owner.public_key().to_hex()]},
    ]))
    .expect("filters");
    let mut implicit = state
        .db
        .get_accessible_channel_ids(community, &owner.public_key().to_bytes())
        .await
        .expect("implicit channels");
    assert_eq!(implicit, [ordinary]);
    let mut named_only = crate::handlers::system_channels::NamedOnlyChannels::default();

    crate::api::bridge::repair_requested_channel_access(
        state,
        tenant,
        &filters,
        &owner.public_key().to_bytes(),
        &mut implicit,
        &mut named_only,
    )
    .await
    .expect("repair");

    assert_eq!(implicit, [ordinary], "implicit set stays legacy");
    let scope = |filter| {
        crate::handlers::system_channels::filter_channel_scope(filter, &implicit, &named_only)
            .into_owned()
    };
    assert_eq!(scope(&filters[0]), [ordinary, later], "naming filter");
    assert_eq!(scope(&filters[1]), [ordinary], "sibling filter");
    assert_eq!(scope(&filters[2]), [ordinary], "system opt-in sibling");

    later_type.drop_database().await;
}

/// A request that names channels the cached access set lacks confirms them
/// all in one membership query. Members join the request-local set in request
/// order, each once; a channel the reader is not in stays out.
/// Mutation: push every unconfirmed channel in
/// `confirm_requested_channel_access` → RED (the stranger's channel gets in);
/// skip the push → RED (the reader's channels stay out).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn named_channels_missing_from_the_cache_are_confirmed_together() {
    let f = fixture().await;
    let reader = Keys::generate();
    let stranger = Keys::generate();
    let first = create_channel(&f.state, &f.tenant, &reader, &[]).await;
    let second = create_channel(&f.state, &f.tenant, &reader, &[]).await;
    let foreign = create_channel(&f.state, &f.tenant, &stranger, &[]).await;

    let filters: Vec<nostr::Filter> = serde_json::from_value(json!([
        {"kinds": [9], "#h": [second.to_string(), foreign.to_string()]},
        {"kinds": [9], "#h": [first.to_string(), second.to_string()]},
    ]))
    .expect("filters");
    // The request's cached lookup ran before the reader joined either channel.
    let mut implicit = Vec::new();
    let mut named_only = crate::handlers::system_channels::NamedOnlyChannels::default();

    crate::api::bridge::repair_requested_channel_access(
        &f.state,
        &f.tenant,
        &filters,
        &reader.public_key().to_bytes(),
        &mut implicit,
        &mut named_only,
    )
    .await
    .expect("repair");

    assert_eq!(implicit, [second, first]);
    assert!(named_only.is_empty());
}

/// Every membership notice is a stored kind:44100/44101 tagged with its
/// channel type (`["t","stream"]`, `["t","system"]`). A filter sees a notice
/// only when it knows the type: a filter without `#t` (every client today)
/// sees the legacy types, and a filter with `#t` sees the types it names. One
/// filter can name all types. This holds on every path: live fan-out, REQ
/// history, COUNT, and the HTTP bridge. An untagged notice stored before the
/// tag existed counts as a legacy type.
/// Mutation: drop the type tag in `emit_membership_notification` → RED;
/// drop the `notice_visible` check in `filters_match` → RED (live and
/// post-filtered reads show the system notice); drop the untagged clause in
/// `push_notice_visibility` → RED (the legacy notice disappears).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn membership_notices_reach_only_filters_that_know_their_type() {
    use buzz_core::kind::{KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION};
    let f = fixture().await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let member_hex = member.public_key().to_hex();
    let kinds = [
        KIND_MEMBER_ADDED_NOTIFICATION,
        KIND_MEMBER_REMOVED_NOTIFICATION,
    ];
    let old = json!({"kinds": kinds, "#p": [member_hex]});
    let asks = json!({"kinds": kinds, "#p": [member_hex], "#t": ["system"]});
    let all = json!({
        "kinds": kinds,
        "#p": [member_hex],
        "#t": ["stream", "forum", "dm", "workflow", "system"],
    });

    // The member's live global subscriptions: one as clients send today, one
    // that asks for system notices, and one that names every type. An outsider also holds a subscription
    // that asks. It is registered directly, as if some filter (for example a
    // kindless `ids` lookup) had been admitted, so only the fan-out gate can
    // keep the notices from it.
    let register = |reader: &Keys, subs: &[(&str, &Value)]| {
        let conn_id = Uuid::new_v4();
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        f.state.conn_manager.register(
            conn_id,
            tx,
            tokio::sync::mpsc::channel(4).0,
            tokio::sync::mpsc::channel(1).0,
            None,
            tokio_util::sync::CancellationToken::new(),
            f.tenant.community(),
            Arc::new(std::sync::atomic::AtomicU8::new(0)),
            Arc::new(tokio::sync::Mutex::new(Default::default())),
            3,
            crate::state::CommunityConnectionControl::new(
                tokio_util::sync::CancellationToken::new(),
            ),
        );
        f.state
            .conn_manager
            .set_authenticated_pubkey(conn_id, reader.public_key().to_bytes().to_vec());
        for (sub, filter) in subs {
            f.state.sub_registry.register_scoped(
                f.tenant.community(),
                conn_id,
                (*sub).into(),
                vec![serde_json::from_value((*filter).clone()).expect("filter")],
                None,
            );
        }
        rx
    };
    let mut rx = register(&member, &[("old", &old), ("asks", &asks), ("all", &all)]);
    let outsider = Keys::generate();
    let mut outsider_rx = register(&outsider, &[("outsider", &json!({"#t": ["system"]}))]);

    let system = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    for channel in [system, ordinary] {
        publish(
            &f,
            &owner,
            9000,
            "",
            &[&["h", &channel.to_string()], &["p", &member_hex]],
        )
        .await;
    }
    publish(
        &f,
        &owner,
        9001,
        "",
        &[&["h", &system.to_string()], &["p", &member_hex]],
    )
    .await;

    // Notifications are sent after the reply; collect the six frames.
    let mut live = Vec::new();
    let mut ids = std::collections::HashMap::new();
    while live.len() < 6 {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("notices so far: {live:?}"))
            .expect("connection open");
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        if message[0] != "EVENT" {
            continue;
        }
        let event: nostr::Event = serde_json::from_value(message[2].clone()).expect("event");
        assert_eq!(
            event.pubkey,
            f.state.relay_keypair.public_key(),
            "relay-signed"
        );
        let h = event
            .tags
            .iter()
            .find(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
            .and_then(|tag| tag.as_slice().get(1).cloned())
            .expect("h tag");
        let channel_type = event
            .tags
            .iter()
            .find(|tag| tag.as_slice().first().map(String::as_str) == Some("t"))
            .and_then(|tag| tag.as_slice().get(1).cloned());
        let expected_type = if h == system.to_string() {
            "system"
        } else {
            "stream"
        };
        assert_eq!(channel_type.as_deref(), Some(expected_type), "type tag");
        let kind = u32::from(event.kind.as_u16());
        ids.insert((kind, h.clone()), event.id.to_hex());
        live.push((message[1].as_str().expect("sub id").to_owned(), kind, h));
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "no seventh notice"
    );
    assert_eq!(
        sorted(live),
        sorted(vec![
            (
                "asks".into(),
                KIND_MEMBER_ADDED_NOTIFICATION,
                system.to_string()
            ),
            (
                "asks".into(),
                KIND_MEMBER_REMOVED_NOTIFICATION,
                system.to_string()
            ),
            (
                "old".into(),
                KIND_MEMBER_ADDED_NOTIFICATION,
                ordinary.to_string()
            ),
            (
                "all".into(),
                KIND_MEMBER_ADDED_NOTIFICATION,
                system.to_string()
            ),
            (
                "all".into(),
                KIND_MEMBER_REMOVED_NOTIFICATION,
                system.to_string()
            ),
            (
                "all".into(),
                KIND_MEMBER_ADDED_NOTIFICATION,
                ordinary.to_string()
            ),
        ])
    );
    let ordinary_added = ids[&(KIND_MEMBER_ADDED_NOTIFICATION, ordinary.to_string())].clone();
    let system_notices = sorted(vec![
        ids[&(KIND_MEMBER_ADDED_NOTIFICATION, system.to_string())].clone(),
        ids[&(KIND_MEMBER_REMOVED_NOTIFICATION, system.to_string())].clone(),
    ]);

    // History: the system notices are stored, and only the asking filter
    // reads them.
    assert_eq!(
        req(&f, &member, old.clone()).await,
        std::slice::from_ref(&ordinary_added)
    );
    assert_eq!(count(&f, &member, old.clone()).await, 1);
    assert_eq!(
        bridge_ids(bridge(&f, &member, "/query", json!([old])).await),
        std::slice::from_ref(&ordinary_added)
    );
    assert_eq!(
        bridge_count(bridge(&f, &member, "/count", json!([old])).await),
        1
    );
    assert_eq!(sorted(req(&f, &member, asks.clone()).await), system_notices);
    assert_eq!(count(&f, &member, asks.clone()).await, 2);
    assert_eq!(
        sorted(bridge_ids(
            bridge(&f, &member, "/query", json!([asks])).await
        )),
        system_notices
    );
    // Replay with `since` works for the asking filter.
    let mut since = asks.clone();
    since["since"] = json!(0);
    assert_eq!(sorted(req(&f, &member, since).await), system_notices);
    // One filter that names every type reads all three notices.
    let every_notice = {
        let mut ids = system_notices.clone();
        ids.push(ordinary_added.clone());
        sorted(ids)
    };
    assert_eq!(sorted(req(&f, &member, all.clone()).await), every_notice);
    assert_eq!(count(&f, &member, all.clone()).await, 3);
    assert_eq!(
        sorted(bridge_ids(
            bridge(&f, &member, "/query", json!([all])).await
        )),
        every_notice
    );

    // A notice stored before notices had a type tag counts as a legacy type:
    // filters without `#t`, and filters that name a legacy type, read it.
    let legacy_channel = Uuid::new_v4().to_string();
    let legacy = nostr::EventBuilder::new(
        nostr::Kind::Custom(KIND_MEMBER_ADDED_NOTIFICATION as u16),
        "",
    )
    .tags([
        nostr::Tag::parse(["p", &member_hex]).unwrap(),
        nostr::Tag::parse(["h", &legacy_channel]).unwrap(),
    ])
    .sign_with_keys(&f.state.relay_keypair)
    .expect("sign");
    f.state
        .db
        .insert_event(f.tenant.community(), &legacy, None)
        .await
        .expect("store legacy notice");
    let legacy_id = legacy.id.to_hex();
    let with_legacy = |ids: &[String]| {
        let mut ids = ids.to_vec();
        ids.push(legacy_id.clone());
        sorted(ids)
    };
    assert_eq!(
        sorted(req(&f, &member, old.clone()).await),
        with_legacy(std::slice::from_ref(&ordinary_added))
    );
    assert_eq!(count(&f, &member, old.clone()).await, 2);
    assert_eq!(
        sorted(req(&f, &member, all.clone()).await),
        with_legacy(&every_notice)
    );
    assert_eq!(count(&f, &member, all.clone()).await, 4);
    assert_eq!(sorted(req(&f, &member, asks.clone()).await), system_notices);

    // Knowing a notice's id is not authorization: only its target reads,
    // counts or receives it, with or without `kinds`.
    // Mutation: drop 44100/44101 from RESULT_GATED_KINDS → RED.
    assert!(
        outsider_rx.try_recv().is_err(),
        "live fan-out reached a non-target"
    );
    let by_id = |with_kinds: bool| {
        let mut filter = json!({"ids": system_notices, "#t": ["system"]});
        if with_kinds {
            filter["kinds"] = json!(kinds);
        }
        filter
    };
    for with_kinds in [true, false] {
        let filter = by_id(with_kinds);
        assert!(
            req(&f, &outsider, filter.clone()).await.is_empty(),
            "{filter}"
        );
        assert_eq!(count(&f, &outsider, filter.clone()).await, 0, "{filter}");
        assert!(
            bridge_ids(bridge(&f, &outsider, "/query", json!([filter])).await).is_empty(),
            "{filter}"
        );
        assert_eq!(
            bridge_count(bridge(&f, &outsider, "/count", json!([filter])).await),
            0,
            "{filter}"
        );
    }
    // The target's own kindless id lookup still works.
    assert_eq!(sorted(req(&f, &member, by_id(false)).await), system_notices);
}

/// The database returns only legacy-type notices unless the query names
/// other types, so a read path that builds its own `EventQuery` and forgets
/// the rule still hides system notices.
/// Mutation: drop `push_notice_visibility` → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn the_database_returns_only_legacy_type_notices_by_default() {
    use buzz_core::kind::KIND_MEMBER_ADDED_NOTIFICATION;
    let f = fixture().await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let member_hex = member.public_key().to_hex();
    let system = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    for channel in [system, ordinary] {
        publish(
            &f,
            &owner,
            9000,
            "",
            &[&["h", &channel.to_string()], &["p", &member_hex]],
        )
        .await;
    }
    // Kindless, but global only: the notices are the only global events
    // that mention the member.
    let base = EventQuery {
        p_tag_hex: Some(member_hex.clone()),
        global_only: true,
        ..EventQuery::for_community(f.tenant.community())
    };
    let channels_of = |events: Vec<buzz_core::StoredEvent>| {
        sorted(
            events
                .into_iter()
                .map(|stored| {
                    stored
                        .event
                        .tags
                        .iter()
                        .find(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
                        .and_then(|tag| tag.as_slice().get(1).cloned())
                        .expect("h tag")
                })
                .collect(),
        )
    };
    // Notifications are written after the reply; wait for both.
    let mut all = Vec::new();
    for _ in 0..50 {
        all = channels_of(
            f.state
                .db
                .query_events(&EventQuery {
                    notice_channel_types: Some(vec!["stream".into(), "system".into()]),
                    ..base.clone()
                })
                .await
                .expect("query"),
        );
        if all.len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(all, sorted(vec![system.to_string(), ordinary.to_string()]));

    for query in [
        base.clone(),
        EventQuery {
            kinds: Some(vec![KIND_MEMBER_ADDED_NOTIFICATION as i32]),
            ..base.clone()
        },
    ] {
        assert_eq!(
            channels_of(f.state.db.query_events(&query).await.expect("query")),
            [ordinary.to_string()]
        );
        assert_eq!(f.state.db.count_events(&query).await.expect("count"), 1);
    }
    let only_system = EventQuery {
        notice_channel_types: Some(vec!["system".into()]),
        ..base.clone()
    };
    assert_eq!(
        channels_of(f.state.db.query_events(&only_system).await.expect("query")),
        [system.to_string()]
    );
    assert_eq!(
        f.state.db.count_events(&only_system).await.expect("count"),
        1
    );
}

/// A group-state filter with `#t:["system"]` or any `#P` finds the system
/// channels the reader can read, and still matches as usual: a creator plus
/// a label finds that creator's one labeled channel.
/// Mutation: make `filter_opts_in` return false → RED (discovery finds
/// nothing). Drop the readability check from the opt-in → RED (an outsider
/// finds the private channel).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn system_channels_are_found_by_type_or_creator() {
    let f = fixture().await;
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let label = "agent-attention";

    let private = create_channel(
        &f.state,
        &f.tenant,
        &owner,
        &[&["channel_type", "system"], &["t", label]],
    )
    .await;
    let open = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        open,
        &[&["visibility", "open"]],
    )
    .await
    .expect("open the system channel");
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    // Another creator's channel with the same label: #P must exclude it.
    let lookalike = create_channel(&f.state, &f.tenant, &outsider, &[&["t", label]]).await;

    let meta = |channel| group_state_id(&f, channel, 39000);
    let (private_meta, open_meta, ordinary_meta, lookalike_meta) = (
        meta(private).await,
        meta(open).await,
        meta(ordinary).await,
        meta(lookalike).await,
    );

    // #t:["system"]: members find both; outsiders find only the open one.
    assert_eq!(
        sorted(req(&f, &owner, json!({"kinds": [39000], "#t": ["system"]})).await),
        sorted(vec![private_meta.clone(), open_meta.clone()]),
        "member #t:system"
    );
    assert_eq!(
        req(&f, &outsider, json!({"kinds": [39000], "#t": ["system"]})).await,
        vec![open_meta.clone()],
        "outsider #t:system"
    );
    // A member lists its own system channels from kind:39002.
    assert_eq!(
        sorted(
            req(
                &f,
                &owner,
                json!({"kinds": [39002], "#p": [owner_hex], "#t": ["system"]})
            )
            .await
        ),
        sorted(vec![
            group_state_id(&f, private, 39002).await,
            group_state_id(&f, open, 39002).await,
        ]),
        "member 39002 #p:[me] #t:system"
    );
    // #P opts in, so a creator's channels include its system channels.
    assert_eq!(
        sorted(req(&f, &owner, json!({"kinds": [39000], "#P": [owner_hex]})).await),
        sorted(vec![private_meta.clone(), open_meta.clone(), ordinary_meta]),
        "member #P"
    );
    // Creator plus label finds exactly the one channel, in SQL.
    let lookup = json!({"kinds": [39000], "#P": [owner_hex], "#t": [label], "limit": 1});
    assert_eq!(
        req(&f, &owner, lookup.clone()).await,
        vec![private_meta.clone()],
        "creator + label"
    );
    assert!(
        req(&f, &outsider, lookup.clone()).await.is_empty(),
        "an outsider cannot find a private system channel"
    );
    // A label alone is not an opt-in.
    assert_eq!(
        req(&f, &owner, json!({"kinds": [39000], "#t": [label]})).await,
        vec![lookalike_meta],
        "label alone"
    );
    // The same opt-in on COUNT and the HTTP bridge.
    assert_eq!(
        count(&f, &owner, json!({"kinds": [39000], "#t": ["system"]})).await,
        2,
        "COUNT #t:system"
    );
    assert_eq!(
        bridge_ids(bridge(&f, &owner, "/query", json!([lookup])).await),
        vec![private_meta],
        "bridge creator + label"
    );
    assert_eq!(
        bridge_count(
            bridge(
                &f,
                &outsider,
                "/count",
                json!([{"kinds": [39000], "#t": ["system"]}])
            )
            .await
        ),
        1,
        "bridge /count outsider #t:system"
    );
}

/// An unlisted open system channel is read only by naming it, for a
/// non-member: `#t:["system"]` and `#P` discovery leave it out, also when a
/// sibling filter in the same request names it, and naming it by `#h` or
/// `#d` reads it. Its members still find it by the opt-in (NIP-CL).
/// Mutation: make `opt_in` always true in `get_readable_named_only_channels`,
/// or ignore it in `NamedOnlyChannels::from_readable` → RED (the sibling of a
/// filter that names it finds it). The branch that loads it only when named
/// is covered in buzz-db (`an_unlisted_system_channel_is_read_only_by_name`).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn unlisted_system_channels_are_found_only_by_name() {
    let f = fixture().await;
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let owner_hex = owner.public_key().to_hex();

    let listed = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    let unlisted =
        create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        listed,
        &[&["visibility", "open"]],
    )
    .await
    .expect("open the listed system channel");
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        unlisted,
        &[&["visibility", "open"], &["unlisted", "true"]],
    )
    .await
    .expect("open the system channel unlisted");
    let h = unlisted.to_string();
    let message = publish(&f, &owner, 9, "hello", &[&["h", &h]]).await;
    let listed_meta = group_state_id(&f, listed, 39000).await;
    let unlisted_meta = group_state_id(&f, unlisted, 39000).await;

    // Opt-in discovery: members find both, outsiders only the listed one.
    for opt_in in [
        json!({"kinds": [39000], "#t": ["system"]}),
        json!({"kinds": [39000], "#P": [owner_hex]}),
    ] {
        assert_eq!(
            sorted(req(&f, &owner, opt_in.clone()).await),
            sorted(vec![listed_meta.clone(), unlisted_meta.clone()]),
            "member {opt_in}"
        );
        assert_eq!(
            req(&f, &outsider, opt_in.clone()).await,
            vec![listed_meta.clone()],
            "outsider {opt_in}"
        );
        assert_eq!(
            count(&f, &outsider, opt_in.clone()).await,
            1,
            "outsider COUNT {opt_in}"
        );
        assert_eq!(
            bridge_ids(bridge(&f, &outsider, "/query", json!([opt_in])).await),
            vec![listed_meta.clone()],
            "outsider bridge /query {opt_in}"
        );
    }

    // Naming it reads it.
    assert_eq!(
        req(&f, &outsider, json!({"kinds": [9], "#h": [h]})).await,
        vec![message.clone()],
        "outsider #h"
    );
    assert_eq!(
        req(&f, &outsider, json!({"kinds": [39000], "#d": [h]})).await,
        vec![unlisted_meta.clone()],
        "outsider #d"
    );

    // A sibling that opts in does not gain the channel the first filter names.
    let filters = json!([
        {"kinds": [9], "#h": [h]},
        {"kinds": [39000], "#t": ["system"]},
    ]);
    let expected = sorted(vec![message, listed_meta]);
    assert_eq!(
        sorted(req_all(&f, &outsider, filters.clone()).await),
        expected,
        "outsider REQ with a naming sibling"
    );
    assert_eq!(
        count_all(&f, &outsider, filters.clone()).await,
        2,
        "outsider COUNT with a naming sibling"
    );
    assert_eq!(
        sorted(bridge_ids(bridge(&f, &outsider, "/query", filters).await)),
        expected,
        "outsider bridge /query with a naming sibling"
    );
}

/// GET a workflow's runs as `reader`, returning the response status.
async fn workflow_runs_status(f: &Fixture, reader: &Keys, workflow: Uuid) -> u16 {
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    crate::router::build_router(f.state.clone())
        .oneshot(
            Request::get(format!("/workflows/{workflow}/runs"))
                .header(header::HOST, &f.host)
                .header("x-pubkey", reader.public_key().to_hex())
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router")
        .status()
        .as_u16()
}

/// A workflow names its channel, so the workflows API reads a workflow in a
/// system channel under the ordinary rules: members read it, outsiders read
/// it only when the channel is open, and a private one answers 404 like a
/// missing workflow.
/// Mutation: drop the `get_readable_named_only_channels` check from
/// `authorize_workflow_read` → RED (members get 404 for their own system
/// channel's workflow).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn workflows_in_system_channels_follow_channel_access() {
    let f = fixture().await;
    let owner = Keys::generate();
    let outsider = Keys::generate();

    let private = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    let open = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        open,
        &[&["visibility", "open"]],
    )
    .await
    .expect("open the system channel");
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[&["visibility", "private"]]).await;

    let owner_bytes = owner.public_key().to_bytes();
    f.state
        .db
        .ensure_user(f.tenant.community(), &owner_bytes)
        .await
        .expect("workflow owner user");
    let workflow = |channel: Uuid| {
        let state = f.state.clone();
        let community = f.tenant.community();
        async move {
            state
                .db
                .create_workflow(
                    community,
                    Some(channel),
                    &owner_bytes,
                    &format!("wf-{}", channel.simple()),
                    "{}",
                    &[0; 32],
                )
                .await
                .expect("create workflow")
        }
    };
    let (private_wf, open_wf, ordinary_wf) = (
        workflow(private).await,
        workflow(open).await,
        workflow(ordinary).await,
    );

    let cases = [
        (&owner, private_wf, 200, "member, private system channel"),
        (&owner, open_wf, 200, "member, open system channel"),
        (&owner, ordinary_wf, 200, "member, ordinary channel"),
        (
            &outsider,
            private_wf,
            404,
            "outsider, private system channel",
        ),
        (&outsider, open_wf, 200, "outsider, open system channel"),
        (
            &outsider,
            ordinary_wf,
            404,
            "outsider, private ordinary channel",
        ),
    ];
    for (reader, workflow, expected, case) in cases {
        assert_eq!(
            workflow_runs_status(&f, reader, workflow).await,
            expected,
            "{case}"
        );
    }
}
