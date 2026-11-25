//! Randomized consistency of the OpenSearch index under interleaving.
//!
//! Each seed writes a random mix of messages, edits, deletions and account
//! deletions while indexers drain and backfill, a proxy injects cluster
//! faults, and the driver crashes indexers mid-batch. After everything
//! drains, the index must hold exactly one document, with its current text,
//! for every searchable message Postgres holds, and nothing else.
//!
//! The seed fixes the plan, not the thread interleaving, so a failing seed
//! may need a few runs to reproduce. Every run clears and checks the whole
//! outbox, so it runs only through `scripts/postgres-test-run.sh`, which
//! gives it a database of its own. Settings:
//!
//! - `BUZZ_TEST_CONSISTENCY_SEEDS`: a seed (`7`) or an inclusive range
//!   (`1-300`); default `1-3`;
//! - `BUZZ_TEST_CONSISTENCY_OPS`: operations per writer; default 20;
//! - `BUZZ_TEST_CONSISTENCY_LATE_APPLY=1`: a request stalled when its
//!   indexer crashes reaches the cluster after a newer write;
//! - `BUZZ_TEST_CONSISTENCY_SEQUENCE_BACK=1`: the version sequence is set
//!   back now and then, as promoting a lagging replica would.

#[allow(unused_macros, unused_imports)]
mod search_suite;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use buzz_core::CommunityId;
use buzz_db::channel::{ChannelType, ChannelVisibility};
use buzz_db::event::ThreadMetadataParams;
use buzz_db::user_deletion::content_batches::ContentBatchOptions;
use buzz_db::user_deletion::ExecutionMode;
use buzz_db::{search_outbox, Db, DbConfig, DeletionTarget};
use buzz_search::indexer::Indexer;
use buzz_search::opensearch::{OpenSearchConfig, SearchIndex};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{json, Value};
use uuid::Uuid;

/// Searchable kinds written by the writers.
const KINDS: [u16; 2] = [9, 40002];
const EDIT: u16 = 40003;

fn opensearch_url() -> String {
    std::env::var("BUZZ_TEST_OPENSEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".into())
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.into())
}

fn seeds() -> Vec<u64> {
    let spec = env_or("BUZZ_TEST_CONSISTENCY_SEEDS", "1-3");
    match spec.split_once('-') {
        Some((from, to)) => {
            (from.trim().parse().expect("seed")..=to.trim().parse().expect("seed")).collect()
        }
        None => vec![spec.trim().parse().expect("seed")],
    }
}

// ---------------------------------------------------------------------------
// Fault proxy
// ---------------------------------------------------------------------------

/// Fault odds for one run, in percent per bulk request.
#[derive(Debug, Clone, Copy)]
struct Faults {
    throttle: u32,
    item_failures: u32,
    lost_response: u32,
    stall: u32,
    late_apply: bool,
    /// Whether the driver sets the version sequence back now and then.
    sequence_back: bool,
}

struct ProxyState {
    upstream: String,
    client: reqwest::Client,
    rng: Mutex<StdRng>,
    faults: Mutex<Option<Faults>>,
    outage_until: Mutex<Option<Instant>>,
    crash: tokio::sync::watch::Sender<u64>,
    /// Stalled requests not yet forwarded or dropped.
    pending: AtomicUsize,
    /// Requests forwarded after their client had gone.
    late_applied: AtomicUsize,
    /// Of those, the ones forwarded after a later request wrote one of
    /// their documents.
    late_after_newer: AtomicUsize,
    bulks: AtomicUsize,
    /// For each document id, the number of the last bulk request the
    /// cluster accepted a write of it from.
    written: Mutex<BTreeMap<String, u64>>,
    /// Bulk requests the cluster has answered, counting from 1.
    answered: AtomicU64,
    /// Wakes requests waiting in `written`.
    write_seen: tokio::sync::Notify,
}

struct Proxy {
    url: String,
    state: Arc<ProxyState>,
    server: tokio::task::JoinHandle<()>,
}

impl Proxy {
    async fn start(seed: u64) -> Self {
        let (crash, _) = tokio::sync::watch::channel(0);
        let state = Arc::new(ProxyState {
            upstream: opensearch_url(),
            client: reqwest::Client::new(),
            rng: Mutex::new(StdRng::seed_from_u64(seed ^ 0x5e_ed0f_9a0c)),
            faults: Mutex::new(None),
            outage_until: Mutex::new(None),
            crash,
            pending: AtomicUsize::new(0),
            late_applied: AtomicUsize::new(0),
            late_after_newer: AtomicUsize::new(0),
            bulks: AtomicUsize::new(0),
            written: Mutex::new(BTreeMap::new()),
            answered: AtomicU64::new(0),
            write_seen: tokio::sync::Notify::new(),
        });
        let app = axum::Router::new()
            .fallback(proxy_request)
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy");
        let url = format!("http://{}", listener.local_addr().expect("proxy address"));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve proxy");
        });
        Self { url, state, server }
    }

    fn index(&self, prefix: &str) -> SearchIndex {
        SearchIndex::new(&OpenSearchConfig {
            url: self.url.clone(),
            basic_auth: None,
            aws_sigv4: None,
            index_prefix: prefix.into(),
        })
        .expect("proxied client")
    }

    fn set_faults(&self, faults: Option<Faults>) {
        *self.state.faults.lock().expect("faults") = faults;
        if faults.is_none() {
            *self.state.outage_until.lock().expect("outage") = None;
        }
    }

    fn outage(&self, length: Duration) {
        *self.state.outage_until.lock().expect("outage") = Some(Instant::now() + length);
    }

    /// The indexer whose requests are stalled has crashed.
    fn crash(&self) {
        self.state.crash.send_modify(|generation| *generation += 1);
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn reply(status: u16, body: Value) -> axum::response::Response {
    axum::response::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("response")
}

async fn forward(
    state: &ProxyState,
    method: axum::http::Method,
    path: &str,
    content_type: Option<axum::http::HeaderValue>,
    body: axum::body::Bytes,
) -> Result<(u16, Option<axum::http::HeaderValue>, axum::body::Bytes), String> {
    let mut request = state
        .client
        .request(method, format!("{}{path}", state.upstream))
        .body(body);
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let content_type = response.headers().get("content-type").cloned();
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    if status == 200
        && path
            .split('?')
            .next()
            .is_some_and(|path| path.ends_with("/_bulk"))
    {
        record_writes(state, &bytes);
    }
    Ok((status, content_type, bytes))
}

/// Note the documents a bulk response says the cluster wrote.
fn record_writes(state: &ProxyState, response: &[u8]) {
    let Ok(response) = serde_json::from_slice::<Value>(response) else {
        return;
    };
    let number = state.answered.fetch_add(1, Ordering::SeqCst) + 1;
    let mut written = state.written.lock().expect("written");
    for item in response["items"].as_array().into_iter().flatten() {
        let Some(result) = item.as_object().and_then(|item| item.values().next()) else {
            continue;
        };
        let accepted = result["status"].as_u64().is_some_and(|status| status < 300);
        if let (true, Some(id)) = (accepted, result["_id"].as_str()) {
            written.insert(id.to_string(), number);
        }
    }
    drop(written);
    state.write_seen.notify_waiters();
}

/// Wait until a bulk request answered after this call writes one of `ids`,
/// for at most `bound`. Whether one did.
async fn newer_write(state: &ProxyState, ids: &[String], bound: Duration) -> bool {
    let since = state.answered.load(Ordering::SeqCst);
    let deadline = tokio::time::Instant::now() + bound;
    loop {
        let seen = state.write_seen.notified();
        tokio::pin!(seen);
        seen.as_mut().enable();
        {
            let written = state.written.lock().expect("written");
            if ids
                .iter()
                .any(|id| written.get(id).is_some_and(|number| *number > since))
            {
                return true;
            }
        }
        if tokio::time::timeout_at(deadline, seen).await.is_err() {
            return false;
        }
    }
}

/// The item actions of an NDJSON bulk body: each action line and, for an
/// index action, its source line.
fn bulk_items(body: &[u8]) -> Vec<(Value, Option<String>)> {
    let text = std::str::from_utf8(body).expect("utf-8 bulk body");
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let mut items = Vec::new();
    while let Some(action) = lines.next() {
        let action: Value = serde_json::from_str(action).expect("bulk action");
        let source = if action.get("delete").is_some() {
            None
        } else {
            Some(lines.next().expect("bulk source").to_string())
        };
        items.push((action, source));
    }
    items
}

async fn proxy_request(
    axum::extract::State(state): axum::extract::State<Arc<ProxyState>>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("request body");
    let path = parts
        .uri
        .path_and_query()
        .map_or("/".to_string(), |path| path.as_str().to_string());
    let content_type = parts.headers.get("content-type").cloned();
    let faults = *state.faults.lock().expect("faults");
    let bulk = parts.uri.path().ends_with("/_bulk");
    let Some(faults) = faults.filter(|_| bulk) else {
        return match forward(&state, parts.method, &path, content_type, body).await {
            Ok((status, content_type, bytes)) => {
                let mut builder = axum::response::Response::builder().status(status);
                if let Some(content_type) = content_type {
                    builder = builder.header("content-type", content_type);
                }
                builder
                    .body(axum::body::Body::from(bytes))
                    .expect("response")
            }
            Err(error) => reply(502, json!({ "error": error })),
        };
    };
    state.bulks.fetch_add(1, Ordering::Relaxed);
    if state
        .outage_until
        .lock()
        .expect("outage")
        .is_some_and(|until| Instant::now() < until)
    {
        return reply(503, json!({ "error": "injected outage", "status": 503 }));
    }
    // One draw per decision, all from the run's seeded generator.
    let (roll, stall_ms, late_ms, failing) = {
        let mut rng = state.rng.lock().expect("rng");
        let roll = rng.random_range(0..100u32);
        let stall_ms = rng.random_range(5..400u64);
        let late_ms = rng.random_range(0..1500u64);
        let items = bulk_items(&body).len();
        let mut failing: Vec<(usize, u16)> = Vec::new();
        for i in 0..items {
            if rng.random_bool(0.3) {
                failing.push((i, [429, 500, 503, 400][rng.random_range(0..4)]));
            }
        }
        (roll, stall_ms, late_ms, failing)
    };
    let mut threshold = faults.throttle;
    if roll < threshold {
        return reply(
            429,
            json!({ "error": { "type": "too_many_requests" }, "status": 429 }),
        );
    }
    threshold += faults.item_failures;
    if roll < threshold && !failing.is_empty() {
        return partial_failure(&state, parts.method, &path, content_type, &body, &failing).await;
    }
    threshold += faults.lost_response;
    if roll < threshold {
        // The cluster applies the request; its response never arrives.
        let _ = forward(&state, parts.method, &path, content_type, body).await;
        return reply(502, json!({ "error": "injected lost response" }));
    }
    threshold += faults.stall;
    if roll < threshold {
        return stall(
            state,
            parts.method,
            path,
            content_type,
            body,
            stall_ms,
            late_ms,
            faults.late_apply,
        )
        .await;
    }
    match forward(&state, parts.method, &path, content_type, body).await {
        Ok((status, content_type, bytes)) => {
            let mut builder = axum::response::Response::builder().status(status);
            if let Some(content_type) = content_type {
                builder = builder.header("content-type", content_type);
            }
            builder
                .body(axum::body::Body::from(bytes))
                .expect("response")
        }
        Err(error) => reply(502, json!({ "error": error })),
    }
}

/// Forward all but the `failing` items and answer as the cluster would had
/// those items failed with their statuses.
async fn partial_failure(
    state: &ProxyState,
    method: axum::http::Method,
    path: &str,
    content_type: Option<axum::http::HeaderValue>,
    body: &[u8],
    failing: &[(usize, u16)],
) -> axum::response::Response {
    let items = bulk_items(body);
    let failed: BTreeMap<usize, u16> = failing.iter().copied().collect();
    let mut kept = String::new();
    for (i, (action, source)) in items.iter().enumerate() {
        if failed.contains_key(&i) {
            continue;
        }
        kept.push_str(&action.to_string());
        kept.push('\n');
        if let Some(source) = source {
            kept.push_str(source);
            kept.push('\n');
        }
    }
    let mut upstream_items = Vec::new();
    if !kept.is_empty() {
        match forward(state, method, path, content_type, kept.into()).await {
            Ok((200, _, bytes)) => {
                let body: Value = serde_json::from_slice(&bytes).expect("bulk response");
                upstream_items = body["items"].as_array().cloned().unwrap_or_default();
            }
            Ok((status, _, bytes)) => {
                return reply(
                    status,
                    serde_json::from_slice(&bytes).unwrap_or(json!({ "status": status })),
                )
            }
            Err(error) => return reply(502, json!({ "error": error })),
        }
    }
    let mut upstream_items = upstream_items.into_iter();
    let mut answer = Vec::with_capacity(items.len());
    for (i, (action, _)) in items.iter().enumerate() {
        let op = if action.get("delete").is_some() {
            "delete"
        } else {
            "index"
        };
        match failed.get(&i) {
            Some(status) => answer.push(json!({ op: {
                "_id": action[op]["_id"],
                "status": status,
                "error": { "type": "injected_item_failure", "reason": "injected" }
            }})),
            None => answer.push(upstream_items.next().expect("upstream item")),
        }
    }
    reply(200, json!({ "took": 1, "errors": true, "items": answer }))
}

/// Hold the request for `stall_ms`, then forward it. If its indexer
/// crashes meanwhile, the client gets an error and the request reaches the
/// cluster only with `late_apply`, otherwise never: once a later request
/// has written one of its documents, or after `late_ms`. A detached task
/// owns the request, so it outlives a client that hangs up.
#[allow(clippy::too_many_arguments)]
async fn stall(
    state: Arc<ProxyState>,
    method: axum::http::Method,
    path: String,
    content_type: Option<axum::http::HeaderValue>,
    body: axum::body::Bytes,
    stall_ms: u64,
    late_ms: u64,
    late_apply: bool,
) -> axum::response::Response {
    let (respond, response) = tokio::sync::oneshot::channel();
    let mut crashed = state.crash.subscribe();
    state.pending.fetch_add(1, Ordering::SeqCst);
    let owner = state.clone();
    tokio::spawn(async move {
        let state = owner;
        let orphaned = tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(stall_ms)) => false,
            _ = crashed.changed() => true,
        };
        if orphaned || respond.is_closed() {
            let _ = respond.send(reply(502, json!({ "error": "injected crash" })));
            if late_apply {
                let ids: Vec<String> = bulk_items(&body)
                    .iter()
                    .filter_map(|(action, _)| {
                        ["index", "delete"]
                            .iter()
                            .find_map(|op| action[*op]["_id"].as_str())
                            .map(str::to_string)
                    })
                    .collect();
                let after_newer = newer_write(&state, &ids, Duration::from_millis(late_ms)).await;
                let _ = forward(&state, method, &path, content_type, body).await;
                state.late_applied.fetch_add(1, Ordering::SeqCst);
                if after_newer {
                    state.late_after_newer.fetch_add(1, Ordering::SeqCst);
                }
            }
        } else {
            let response = match forward(&state, method, &path, content_type, body).await {
                Ok((status, _, bytes)) => reply(
                    status,
                    serde_json::from_slice(&bytes).unwrap_or(json!({ "status": status })),
                ),
                Err(error) => reply(502, json!({ "error": error })),
            };
            let _ = respond.send(response);
        }
        state.pending.fetch_sub(1, Ordering::SeqCst);
    });
    response
        .await
        .unwrap_or_else(|_| reply(502, json!({ "error": "stalled request dropped" })))
}

// ---------------------------------------------------------------------------
// Model of what was written
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Edit {
    event: Event,
    token: String,
    deleted: bool,
}

#[derive(Debug, Clone)]
struct Message {
    community: usize,
    channel: Uuid,
    event: Event,
    author: usize,
    token: String,
    parent: Option<[u8; 32]>,
    root: Option<[u8; 32]>,
    mentions: Vec<String>,
    edits: Vec<Edit>,
    deleted: bool,
}

impl Message {
    fn id(&self) -> [u8; 32] {
        *self.event.id.as_bytes()
    }

    /// The token of the text search should match: the latest live edit's,
    /// or the message's own.
    fn current_token(&self) -> &str {
        self.edits
            .iter()
            .filter(|edit| !edit.deleted)
            .max_by_key(|edit| (edit.event.created_at, edit.event.id))
            .map_or(&self.token, |edit| &edit.token)
    }

    fn tokens(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.token.as_str())
            .chain(self.edits.iter().map(|edit| edit.token.as_str()))
    }
}

struct Author {
    keys: Keys,
    community: usize,
}

struct World {
    run: u64,
    communities: Vec<CommunityId>,
    channels: Vec<Vec<Uuid>>,
    authors: Vec<Author>,
    messages: Mutex<Vec<Message>>,
    /// Signing time of the next event, one second apart, so edits order.
    clock: AtomicI64,
    tokens: AtomicI64,
}

impl World {
    fn tick(&self) -> Timestamp {
        Timestamp::from(self.clock.fetch_add(1, Ordering::SeqCst) as u64)
    }

    fn token(&self) -> String {
        format!(
            "zq{}t{}",
            self.run,
            self.tokens.fetch_add(1, Ordering::SeqCst)
        )
    }
}

fn signed_at(event: &Event) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).expect("timestamp")
}

fn sign(keys: &Keys, kind: u16, content: &str, tags: Vec<Vec<String>>, at: Timestamp) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags.into_iter().map(|tag| Tag::parse(tag).expect("tag")))
        .custom_created_at(at)
        .sign_with_keys(keys)
        .expect("sign")
}

/// Write a new message or reply by `author`; returns it once stored, or
/// `None` when a reply is refused because its parent was erased meanwhile.
async fn write_message(
    db: &Db,
    world: &World,
    rng: &mut StdRng,
    author: usize,
    parent: Option<Message>,
) -> Option<Message> {
    let community = world.authors[author].community;
    let keys = &world.authors[author].keys;
    let channel = match &parent {
        Some(parent) => parent.channel,
        None => world.channels[community][rng.random_range(0..world.channels[community].len())],
    };
    let token = world.token();
    let kind = KINDS[rng.random_range(0..KINDS.len())];
    let mut tags = vec![vec!["h".to_string(), channel.to_string()]];
    let mut mentions = Vec::new();
    if rng.random_bool(0.3) {
        let others: Vec<&Author> = world
            .authors
            .iter()
            .enumerate()
            .filter(|(i, other)| other.community == community && *i != author)
            .map(|(_, other)| other)
            .collect();
        let mentioned = others[rng.random_range(0..others.len())]
            .keys
            .public_key()
            .to_hex();
        tags.push(vec!["p".into(), mentioned.clone()]);
        mentions.push(mentioned);
    }
    let root = parent
        .as_ref()
        .map(|parent| parent.root.unwrap_or(parent.id()));
    if let Some(parent) = &parent {
        let root_hex = hex::encode(root.expect("root"));
        tags.push(vec!["e".into(), root_hex, String::new(), "root".into()]);
        tags.push(vec![
            "e".into(),
            parent.event.id.to_hex(),
            String::new(),
            "reply".into(),
        ]);
    }
    let event = sign(keys, kind, &format!("{token} chatter"), tags, world.tick());
    let community_id = world.communities[community];
    match &parent {
        None => {
            db.insert_event(community_id, &event, Some(channel))
                .await
                .expect("insert message");
        }
        Some(parent) => {
            let messages = world.messages.lock().expect("model").clone();
            let created = |id: [u8; 32]| {
                messages
                    .iter()
                    .find(|message| message.id() == id)
                    .map(|message| signed_at(&message.event))
                    .expect("known message")
            };
            let root = root.expect("root");
            let stored = db
                .insert_event_with_thread_metadata(
                    community_id,
                    &event,
                    Some(channel),
                    Some(ThreadMetadataParams {
                        event_id: event.id.as_bytes(),
                        event_created_at: signed_at(&event),
                        channel_id: channel,
                        parent_event_id: Some(parent.event.id.as_bytes()),
                        parent_event_created_at: Some(signed_at(&parent.event)),
                        root_event_id: Some(&root),
                        root_event_created_at: Some(created(root)),
                        depth: if parent.root.is_some() { 2 } else { 1 },
                        broadcast: false,
                    }),
                )
                .await;
            match stored {
                Ok(_) => {}
                Err(buzz_db::DbError::InvalidData(reason))
                    if reason.contains("parent disappeared") =>
                {
                    return None
                }
                Err(error) => panic!("insert reply: {error}"),
            }
        }
    }
    mentions.sort();
    Some(Message {
        community,
        channel,
        event,
        author,
        token,
        parent: parent.as_ref().map(Message::id),
        root,
        mentions,
        edits: Vec::new(),
        deleted: false,
    })
}

#[allow(clippy::too_many_arguments)]
async fn delete_event(
    db: &Db,
    world: &World,
    author: usize,
    community: usize,
    channel: Uuid,
    target: &Event,
    parent: Option<[u8; 32]>,
    root: Option<[u8; 32]>,
) {
    let keys = &world.authors[author].keys;
    let deletion = sign(
        keys,
        5,
        "",
        vec![
            vec!["h".into(), channel.to_string()],
            vec!["e".into(), target.id.to_hex()],
        ],
        world.tick(),
    );
    db.insert_deletion_event(
        world.communities[community],
        &deletion,
        Some(channel),
        None,
        &[DeletionTarget {
            event_id: target.id.as_bytes(),
            created_at: signed_at(target),
            parent_event_id: parent.as_ref().map(|id| id.as_slice()),
            root_event_id: root.as_ref().map(|id| id.as_slice()),
        }],
        &Keys::generate(),
    )
    .await
    .expect("insert deletion");
}

/// Delete every message of `author`'s account, rerunning while concurrent
/// replies make a batch roll back, as the product asks.
async fn delete_account(db: &Db, world: &World, author: usize) {
    let community = world.authors[author].community;
    let options = ContentBatchOptions {
        community: *world.communities[community].as_uuid(),
        pubkey: world.authors[author].keys.public_key().to_hex(),
        relay_pubkey: None,
        batch_size: 7,
        max_items: 100_000,
    };
    let mut last = None;
    for _ in 0..100 {
        match db
            .user_deletion_store()
            .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
            .await
        {
            Ok(_) => return,
            Err(error) => {
                last = Some(error.to_string());
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
    panic!("account deletion never completed: {last:?}");
}

/// One writer: owns `author`, so the model of its messages is exact.
async fn writer(db: Db, world: Arc<World>, author: usize, seed: u64, ops: usize) {
    let mut rng = StdRng::seed_from_u64(seed);
    let community = world.authors[author].community;
    for _ in 0..ops {
        tokio::time::sleep(Duration::from_millis(rng.random_range(0..8))).await;
        let roll = rng.random_range(0..100u32);
        let snapshot = world.messages.lock().expect("model").clone();
        let own_live: Vec<usize> = snapshot
            .iter()
            .enumerate()
            .filter(|(_, message)| message.author == author && !message.deleted)
            .map(|(i, _)| i)
            .collect();
        let peers: Vec<&Message> = snapshot
            .iter()
            .filter(|message| message.community == community && !message.deleted)
            .collect();
        if roll < 40 || own_live.is_empty() {
            let message = write_message(&db, &world, &mut rng, author, None).await;
            world.messages.lock().expect("model").extend(message);
        } else if roll < 55 && !peers.is_empty() {
            let parent = peers[rng.random_range(0..peers.len())].clone();
            let message = write_message(&db, &world, &mut rng, author, Some(parent)).await;
            world.messages.lock().expect("model").extend(message);
        } else if roll < 78 {
            let target = own_live[rng.random_range(0..own_live.len())];
            let message = &snapshot[target];
            let token = world.token();
            let edit = sign(
                &world.authors[author].keys,
                EDIT,
                &format!("{token} edited"),
                vec![
                    vec!["h".into(), message.channel.to_string()],
                    vec!["e".into(), message.event.id.to_hex()],
                ],
                world.tick(),
            );
            db.insert_event(world.communities[community], &edit, Some(message.channel))
                .await
                .expect("insert edit");
            world.messages.lock().expect("model")[target]
                .edits
                .push(Edit {
                    event: edit,
                    token,
                    deleted: false,
                });
        } else if roll < 90 {
            let target = own_live[rng.random_range(0..own_live.len())];
            let message = &snapshot[target];
            delete_event(
                &db,
                &world,
                author,
                community,
                message.channel,
                &message.event,
                message.parent,
                message.root,
            )
            .await;
            world.messages.lock().expect("model")[target].deleted = true;
        } else if roll < 96 {
            let edits: Vec<(usize, usize)> = own_live
                .iter()
                .flat_map(|&m| {
                    snapshot[m]
                        .edits
                        .iter()
                        .enumerate()
                        .filter(|(_, edit)| !edit.deleted)
                        .map(move |(e, _)| (m, e))
                })
                .collect();
            if let Some(&(m, e)) = edits.get(rng.random_range(0..edits.len().max(1))) {
                let message = &snapshot[m];
                delete_event(
                    &db,
                    &world,
                    author,
                    community,
                    message.channel,
                    &message.edits[e].event,
                    None,
                    None,
                )
                .await;
                world.messages.lock().expect("model")[m].edits[e].deleted = true;
            }
        } else {
            delete_account(&db, &world, author).await;
            let mut messages = world.messages.lock().expect("model");
            for message in messages
                .iter_mut()
                .filter(|message| message.author == author)
            {
                message.deleted = true;
                for edit in &mut message.edits {
                    edit.deleted = true;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

async fn connect() -> Db {
    // A run clears the whole outbox and completes every backfill, so it never
    // falls back to the development database.
    let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .expect("Postgres wrapper must provide an isolated database URL");
    assert_eq!(
        std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref(),
        Ok("desired"),
        "run through scripts/postgres-test-run.sh, which gives each test a database of its own"
    );
    let (pool, _) = search_suite::setup().await;
    pool.close().await;
    let db = Db::new(&DbConfig {
        database_url,
        max_connections: 48,
        ..DbConfig::default()
    })
    .await
    .expect("connect database layer");
    search_outbox::enable(db.pool())
        .await
        .expect("enable outbox");
    db
}

/// Leave nothing of an earlier, failed run for this one to index.
async fn reset(db: &Db) {
    sqlx::query("DELETE FROM search_outbox")
        .execute(db.pool())
        .await
        .expect("clear outbox");
    sqlx::query(
        "INSERT INTO search_backfill (community_id, enqueued, completed_at) \
         SELECT id, 0, now() FROM communities \
         ON CONFLICT (community_id) DO UPDATE SET completed_at = now()",
    )
    .execute(db.pool())
    .await
    .expect("complete earlier backfills");
}

async fn setup_world(db: &Db, rng: &mut StdRng, seed: u64) -> World {
    let mut communities = Vec::new();
    let mut channels = Vec::new();
    let mut authors = Vec::new();
    let community_count = rng.random_range(2..4usize);
    for c in 0..community_count {
        let community = search_suite::mk_community(
            db.pool(),
            &format!("c{c}-{}.consistency.example", Uuid::new_v4().simple()),
        )
        .await;
        let author_count = rng.random_range(2..4usize);
        let mut community_authors = Vec::new();
        for _ in 0..author_count {
            let keys = Keys::generate();
            db.ensure_user(community, keys.public_key().as_bytes())
                .await
                .expect("ensure user");
            community_authors.push(keys);
        }
        // One author is an agent, so author_is_agent is checked both ways.
        // Its owner never writes: deleting the owner's account deletes the
        // agent's messages too, concurrently with the agent's own writer.
        let owner = Keys::generate();
        db.ensure_user(community, owner.public_key().as_bytes())
            .await
            .expect("ensure owner");
        db.set_agent_owner(
            community,
            community_authors[0].public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .expect("agent owner");
        let mut community_channels = Vec::new();
        for _ in 0..rng.random_range(1..3usize) {
            community_channels.push(
                db.create_channel(
                    community,
                    &format!("ch-{}", Uuid::new_v4().simple()),
                    ChannelType::Stream,
                    ChannelVisibility::Open,
                    None,
                    community_authors[0].public_key().as_bytes(),
                    None,
                )
                .await
                .expect("create channel")
                .id,
            );
        }
        communities.push(community);
        channels.push(community_channels);
        authors.extend(
            community_authors
                .into_iter()
                .map(|keys| Author { keys, community: c }),
        );
    }
    World {
        run: seed,
        communities,
        channels,
        authors,
        messages: Mutex::new(Vec::new()),
        clock: AtomicI64::new(Timestamp::now().as_secs() as i64 - 300),
        tokens: AtomicI64::new(0),
    }
}

/// Messages stored before the index existed: written, edited and deleted,
/// then their outbox rows dropped, so only the backfill can index them.
async fn stored_history(db: &Db, world: &World, rng: &mut StdRng) {
    for author in 0..world.authors.len() {
        for _ in 0..rng.random_range(0..12usize) {
            let message = write_message(db, world, rng, author, None).await;
            world.messages.lock().expect("model").extend(message);
        }
    }
    let snapshot = world.messages.lock().expect("model").clone();
    for (i, message) in snapshot.iter().enumerate() {
        if rng.random_bool(0.2) {
            let token = world.token();
            let edit = sign(
                &world.authors[message.author].keys,
                EDIT,
                &format!("{token} edited"),
                vec![
                    vec!["h".into(), message.channel.to_string()],
                    vec!["e".into(), message.event.id.to_hex()],
                ],
                world.tick(),
            );
            db.insert_event(
                world.communities[message.community],
                &edit,
                Some(message.channel),
            )
            .await
            .expect("insert edit");
            world.messages.lock().expect("model")[i].edits.push(Edit {
                event: edit,
                token,
                deleted: false,
            });
        } else if rng.random_bool(0.1) {
            delete_event(
                db,
                world,
                message.author,
                message.community,
                message.channel,
                &message.event,
                None,
                None,
            )
            .await;
            world.messages.lock().expect("model")[i].deleted = true;
        }
    }
    sqlx::query("DELETE FROM search_outbox")
        .execute(db.pool())
        .await
        .expect("forget the history's rows");
}

#[derive(Debug, Clone, Copy)]
struct Plan {
    workers: usize,
    other_workers: usize,
    batch: i64,
    faults: Faults,
}

fn plan(seed: u64, rng: &mut StdRng) -> Plan {
    let workers = [1, 4, 8][(seed % 3) as usize];
    Plan {
        workers,
        other_workers: [1, 4, 8][rng.random_range(0..3)],
        batch: rng.random_range(1..12),
        faults: Faults {
            throttle: rng.random_range(0..10),
            item_failures: rng.random_range(0..25),
            lost_response: rng.random_range(0..8),
            stall: rng.random_range(0..20),
            late_apply: env_or("BUZZ_TEST_CONSISTENCY_LATE_APPLY", "0") == "1",
            sequence_back: env_or("BUZZ_TEST_CONSISTENCY_SEQUENCE_BACK", "0") == "1",
        },
    }
}

/// Run `indexer`'s drain, giving up after a bound so nothing hangs.
async fn bounded_drain(indexer: &Indexer) {
    let _ = tokio::time::timeout(Duration::from_secs(40), indexer.drain()).await;
}

/// Index concurrently with the writers until `done`, injecting crashes,
/// overlapping indexers, outages and, with `sequence_back`, sequence
/// rewinds.
async fn driver(
    db: Db,
    first: Indexer,
    second: Indexer,
    proxy: Arc<Proxy>,
    done: Arc<AtomicBool>,
    seed: u64,
    sequence_back: bool,
) -> (usize, usize, usize, usize) {
    let mut rng = StdRng::seed_from_u64(seed ^ 0xd21_7e5);
    let (mut crashes, mut overlaps, mut outages, mut rewinds) = (0, 0, 0, 0);
    while !done.load(Ordering::SeqCst) {
        if sequence_back && rng.random_bool(0.15) {
            // A failover to a replica that had not seen the last values:
            // the sequence hands some of them out again.
            sqlx::query(
                "SELECT setval('search_index_version', \
                 greatest(last_value - $1, 1)) FROM search_index_version",
            )
            .bind(rng.random_range(1..=50i64))
            .execute(db.pool())
            .await
            .expect("set the version sequence back");
            rewinds += 1;
        }
        match rng.random_range(0..100u32) {
            0..=44 => bounded_drain(&first).await,
            45..=69 => {
                // Crash mid-batch: the claimed rows roll back.
                let indexer = first.clone();
                let task = tokio::spawn(async move { indexer.drain().await });
                tokio::time::sleep(Duration::from_millis(rng.random_range(0..60))).await;
                task.abort();
                let _ = task.await;
                proxy.crash();
                crashes += 1;
            }
            70..=89 => {
                // A lease handover: the old holder's pass is still running
                // when the new one starts.
                let (a, b) = (first.clone(), second.clone());
                let backfill = second.clone();
                let _ = tokio::join!(bounded_drain(&a), bounded_drain(&b), async {
                    let _ = backfill.backfill().await;
                });
                overlaps += 1;
            }
            _ => {
                proxy.outage(Duration::from_millis(rng.random_range(50..600)));
                outages += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(rng.random_range(0..20))).await;
    }
    (crashes, overlaps, outages, rewinds)
}

/// What one community's document should hold, read from Postgres.
#[derive(Debug, Clone, PartialEq)]
struct Expected {
    kind: i64,
    author: String,
    channel: Option<String>,
    channel_type: Option<String>,
    author_is_agent: bool,
    thread_root: Option<String>,
    mentions: Vec<String>,
    created_at: i64,
    content: String,
}

/// id, kind, pubkey, channel, created_at, channel type, agent, thread root,
/// mentions, content.
type ExpectedRow = (
    Vec<u8>,
    i32,
    Vec<u8>,
    Option<Uuid>,
    i64,
    Option<String>,
    bool,
    Option<Vec<u8>>,
    Vec<String>,
    String,
);
/// id, created_at, channel, tags, content of a live edit.
type EditRow = (Vec<u8>, i64, Option<Uuid>, Value, String);
/// created_at, id, content of the latest live edit of a message.
type LatestEdit = (i64, Vec<u8>, String);

async fn expected_from_postgres(db: &Db, community: CommunityId) -> BTreeMap<String, Expected> {
    let rows: Vec<ExpectedRow> = sqlx::query_as(
        "SELECT e.id, e.kind, e.pubkey, e.channel_id, extract(epoch FROM e.created_at)::bigint, \
                    c.channel_type::text, (u.agent_owner_pubkey IS NOT NULL), t.root_event_id, \
                    ARRAY(SELECT m.pubkey_hex::text FROM event_mentions m \
                          WHERE m.community_id = e.community_id AND m.event_id = e.id \
                          ORDER BY 1), \
                    e.content \
             FROM events e \
             LEFT JOIN channels c ON c.community_id = e.community_id AND c.id = e.channel_id \
             LEFT JOIN users u ON u.community_id = e.community_id AND u.pubkey = e.pubkey \
             LEFT JOIN thread_metadata t ON t.community_id = e.community_id \
                  AND t.event_created_at = e.created_at AND t.event_id = e.id \
             WHERE e.community_id = $1 AND e.deleted_at IS NULL AND e.kind = ANY($2)",
    )
    .bind(community.as_uuid())
    .bind(search_outbox::SEARCHABLE_KINDS.as_slice())
    .fetch_all(db.pool())
    .await
    .expect("expected documents");
    // Live edits, applied in Rust from their first `e` tag.
    let edits: Vec<EditRow> = sqlx::query_as(
        "SELECT id, extract(epoch FROM created_at)::bigint, channel_id, tags, content FROM events \
         WHERE community_id = $1 AND kind = 40003 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .fetch_all(db.pool())
    .await
    .expect("edits");
    let mut latest: BTreeMap<(String, Option<Uuid>), LatestEdit> = BTreeMap::new();
    for (id, at, channel, tags, content) in edits {
        let Some(target) = tags.as_array().and_then(|tags| {
            tags.iter()
                .find(|tag| tag[0] == "e")
                .and_then(|tag| tag[1].as_str().map(str::to_lowercase))
        }) else {
            continue;
        };
        let entry =
            latest
                .entry((target, channel))
                .or_insert((i64::MIN, Vec::new(), String::new()));
        if (at, &id) > (entry.0, &entry.1) {
            *entry = (at, id, content);
        }
    }
    rows.into_iter()
        .map(
            |(
                id,
                kind,
                pubkey,
                channel,
                created_at,
                channel_type,
                agent,
                root,
                mentions,
                content,
            )| {
                let id = hex::encode(id);
                let content = latest
                    .get(&(id.clone(), channel))
                    .map_or(content, |(_, _, content)| content.clone());
                (
                    id,
                    Expected {
                        kind: kind.into(),
                        author: hex::encode(pubkey),
                        channel: channel.map(|channel| channel.to_string()),
                        channel_type,
                        author_is_agent: agent,
                        thread_root: root.map(hex::encode),
                        mentions,
                        created_at,
                        content,
                    },
                )
            },
        )
        .collect()
}

async fn upstream(method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
    let client = reqwest::Client::new();
    let mut request = client.request(method, format!("{}/{path}", opensearch_url()));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("OpenSearch request");
    response.json().await.expect("OpenSearch body")
}

/// Check the index against Postgres and the model; returns the violations.
async fn verify(db: &Db, world: &World, prefix: &str) -> Vec<String> {
    let alias = format!("{prefix}messages");
    let mut violations = Vec::new();
    let depth = search_outbox::depth(db.pool()).await.expect("depth");
    if depth != 0 {
        violations.push(format!("outbox holds {depth} rows"));
    }
    upstream(reqwest::Method::POST, &format!("{alias}/_refresh"), None).await;
    let messages = world.messages.lock().expect("model").clone();
    let mut total_expected = 0;
    for (c, community) in world.communities.iter().enumerate() {
        let expected = expected_from_postgres(db, *community).await;
        total_expected += expected.len();
        let model: Vec<&Message> = messages.iter().filter(|m| m.community == c).collect();
        // The model and Postgres agree on what is live and its text.
        for message in &model {
            let id = message.event.id.to_hex();
            match (expected.get(&id), message.deleted) {
                (Some(_), true) => violations.push(format!("{id}: deleted in the model, live in Postgres")),
                (None, false) => violations.push(format!("{id}: live in the model, gone from Postgres; author {} {}, parent {:?}", message.author, world.authors[message.author].keys.public_key().to_hex(), message.parent.map(hex::encode))),
                (Some(row), false) if !row.content.starts_with(&format!("{} ", message.current_token())) => {
                    violations.push(format!(
                        "{id}: Postgres text {:?}, model token {}",
                        row.content,
                        message.current_token()
                    ))
                }
                (Some(row), false)
                    if row.mentions != message.mentions
                        || row.thread_root != message.root.map(hex::encode) =>
                {
                    violations.push(format!(
                        "{id}: Postgres fields differ from the model: {row:?}, model mentions {:?} root {:?}",
                        message.mentions,
                        message.root.map(hex::encode)
                    ))
                }
                _ => {}
            }
        }
        // Every token ever written in the community, as a named query, so
        // each hit says which texts it matches.
        let tokens: Vec<&str> = model.iter().flat_map(|m| m.tokens()).collect();
        let mut hits: BTreeMap<String, (Value, BTreeSet<String>)> = BTreeMap::new();
        let mut chunks: Vec<&[&str]> = tokens.chunks(500).collect();
        if chunks.is_empty() {
            chunks.push(&[]);
        }
        for chunk in chunks {
            let should: Vec<Value> = chunk
                .iter()
                .map(|token| json!({ "term": { "content": { "value": token, "_name": token } } }))
                .collect();
            let body = upstream(
                reqwest::Method::POST,
                &format!("{alias}/_search"),
                Some(json!({
                    "size": 10000,
                    "query": { "bool": {
                        "filter": [{ "term": { "community": community.as_uuid().to_string() } }],
                        "should": should
                    }}
                })),
            )
            .await;
            for hit in body["hits"]["hits"].as_array().expect("hits") {
                let id = hit["_id"].as_str().expect("_id").to_string();
                let entry = hits
                    .entry(id)
                    .or_insert_with(|| (hit["_source"].clone(), BTreeSet::new()));
                for name in hit["matched_queries"].as_array().into_iter().flatten() {
                    entry.1.insert(name.as_str().expect("name").to_string());
                }
            }
        }
        let prefix = format!("{}:", community.as_uuid());
        for (doc_id, (source, matched)) in &hits {
            let Some(event) = doc_id.strip_prefix(&prefix) else {
                violations.push(format!("{doc_id}: document of another community"));
                continue;
            };
            let Some(row) = expected.get(event) else {
                violations.push(format!(
                    "{event}: document for a message that is not live; matches {matched:?}"
                ));
                continue;
            };
            let want_token = row
                .content
                .split(' ')
                .next()
                .unwrap_or_default()
                .to_string();
            if *matched != BTreeSet::from([want_token.clone()]) {
                violations.push(format!(
                    "{event}: text matches {matched:?}, want {want_token}"
                ));
            }
            let fields = json!({
                "community": community.as_uuid().to_string(),
                "event_id": event,
                "kind": row.kind,
                "author": row.author,
                "channel": row.channel,
                "channel_type": row.channel_type,
                "author_is_agent": row.author_is_agent,
                "thread_root": row.thread_root,
                "mentions": row.mentions,
                "created_at": row.created_at,
            });
            if *source != fields {
                violations.push(format!("{event}: fields {source} want {fields}"));
            }
        }
        for event in expected.keys() {
            if !hits.contains_key(&format!("{prefix}{event}")) {
                violations.push(format!("{event}: live message has no document"));
            }
        }
    }
    let count = upstream(reqwest::Method::GET, &format!("{alias}/_count"), None).await;
    if count["count"].as_u64() != Some(total_expected as u64) {
        violations.push(format!(
            "index holds {} documents, want {total_expected}",
            count["count"]
        ));
    }
    violations
}

struct RunReport {
    messages: usize,
    bulks: usize,
    crashes: usize,
    overlaps: usize,
    outages: usize,
    late_applied: usize,
    late_after_newer: usize,
    rewinds: usize,
}

async fn run(db: &Db, seed: u64, ops: usize) -> Result<RunReport, Vec<String>> {
    reset(db).await;
    let mut rng = StdRng::seed_from_u64(seed);
    let plan = plan(seed, &mut rng);
    let world = Arc::new(setup_world(db, &mut rng, seed).await);
    stored_history(db, &world, &mut rng).await;

    let prefix = format!("cons-{}-", Uuid::new_v4().simple());
    let direct = SearchIndex::new(&OpenSearchConfig {
        url: opensearch_url(),
        basic_auth: None,
        aws_sigv4: None,
        index_prefix: prefix.clone(),
    })
    .expect("client");
    direct.ensure().await.expect("create index");
    let proxy = Arc::new(Proxy::start(seed).await);
    let indexer = |workers: usize| {
        Indexer::new(db.clone(), proxy.index(&prefix))
            .with_workers(workers)
            .with_batch_size(plan.batch)
            .with_backfill_bound(plan.batch * 2)
    };
    let (first, second) = (indexer(plan.workers), indexer(plan.other_workers));
    proxy.set_faults(Some(plan.faults));

    let done = Arc::new(AtomicBool::new(false));
    let backfill = {
        let (indexer, done) = (first.clone(), done.clone());
        let mut rng = StdRng::seed_from_u64(seed ^ 0xbac_f111);
        tokio::spawn(async move {
            while !done.load(Ordering::SeqCst) {
                let _ = tokio::time::timeout(Duration::from_secs(20), indexer.backfill()).await;
                tokio::time::sleep(Duration::from_millis(rng.random_range(0..30))).await;
            }
        })
    };
    let driving = tokio::spawn(driver(
        db.clone(),
        first.clone(),
        second,
        proxy.clone(),
        done.clone(),
        seed,
        plan.faults.sequence_back,
    ));
    let writers: Vec<_> = (0..world.authors.len())
        .map(|author| {
            tokio::spawn(writer(
                db.clone(),
                world.clone(),
                author,
                seed.wrapping_mul(1000).wrapping_add(author as u64),
                ops,
            ))
        })
        .collect();
    for writer in writers {
        writer.await.expect("writer");
    }
    done.store(true, Ordering::SeqCst);
    backfill.await.expect("backfill task");
    let (crashes, overlaps, outages, rewinds) = driving.await.expect("driver");

    // Let everything drain with the faults off. A pass can still fail
    // while documents hold versions the sequence handed out again; their
    // rows stay queued until the sequence passes them.
    proxy.set_faults(None);
    let mut last_error = None;
    let drained = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let queued = first.backfill().await.expect("final backfill");
            if let Err(error) = first.drain().await {
                last_error = Some(error.to_string());
            }
            let incomplete: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM communities c LEFT JOIN search_backfill b ON b.community_id = c.id \
                 WHERE b.completed_at IS NULL",
            )
            .fetch_one(db.pool())
            .await
            .expect("backfill state");
            if queued == 0
                && incomplete == 0
                && search_outbox::depth(db.pool()).await.expect("depth") == 0
                && proxy.state.pending.load(Ordering::SeqCst) == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let mut violations = Vec::new();
    if drained.is_err() {
        violations.push(format!(
            "the outbox did not drain within 120 s; last drain error: {last_error:?}"
        ));
    }
    violations.extend(verify(db, &world, &prefix).await);
    let report = RunReport {
        messages: world.messages.lock().expect("model").len(),
        bulks: proxy.state.bulks.load(Ordering::Relaxed),
        crashes,
        overlaps,
        outages,
        late_applied: proxy.state.late_applied.load(Ordering::SeqCst),
        late_after_newer: proxy.state.late_after_newer.load(Ordering::SeqCst),
        rewinds,
    };
    if violations.is_empty() {
        direct.delete().await.expect("delete index");
        Ok(report)
    } else {
        violations.insert(
            0,
            format!(
                "seed {seed}: {plan:?}, index prefix {prefix} kept, {} messages, {} bulks, {crashes} crashes, {overlaps} overlaps, {outages} outages, {} late ({} after a newer write), {rewinds} sequence rewinds",
                report.messages, report.bulks, report.late_applied, report.late_after_newer
            ),
        );
        Err(violations)
    }
}

/// From each seed, random writes, deletions, edits, account deletions and a
/// backfill interleave with crashing, overlapping and throttled indexers;
/// afterwards the index matches Postgres exactly.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires Postgres and OpenSearch, and a database of its own"]
async fn the_index_converges_on_postgres_under_interleaving() {
    let db = connect().await;
    let ops: usize = env_or("BUZZ_TEST_CONSISTENCY_OPS", "20")
        .parse()
        .expect("ops");
    let mut failures = Vec::new();
    for seed in seeds() {
        let started = Instant::now();
        match tokio::time::timeout(Duration::from_secs(300), run(&db, seed, ops)).await {
            Ok(Ok(report)) => eprintln!(
                "seed {seed}: ok in {:?}: {} messages, {} bulks, {} crashes, {} overlaps, {} outages, {} late ({} after a newer write), {} sequence rewinds",
                started.elapsed(),
                report.messages,
                report.bulks,
                report.crashes,
                report.overlaps,
                report.outages,
                report.late_applied,
                report.late_after_newer,
                report.rewinds
            ),
            Ok(Err(violations)) => {
                eprintln!("seed {seed}: FAILED\n  {}", violations.join("\n  "));
                failures.push(seed);
            }
            Err(_) => {
                eprintln!("seed {seed}: FAILED: timed out after 300 s");
                failures.push(seed);
            }
        }
    }
    assert!(failures.is_empty(), "failing seeds: {failures:?}");
}
