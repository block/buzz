//! NIP-50 search through the `search.backend` switch: Postgres (0), shadow
//! (1) and OpenSearch (2), against real Postgres, Redis and OpenSearch.
//!
//! Index documents are written straight into each test's own index, not
//! through the outbox and indexer, so a document can disagree with Postgres
//! the way a lagging or stale index does: indexed under another word, under
//! another channel, or still present after its message was deleted. Every
//! test runs on a current-thread runtime so the shadow task's metrics and log
//! line land in the test's recorder and log capture.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use buzz_core::TenantContext;
use buzz_db::search_outbox::SearchDocument;
use buzz_feature_flags::{BooleanFlag, EvaluationContext, FlagEvaluator, IntegerFlag};
use buzz_search::opensearch::{OpenSearchConfig, SearchIndex};
use buzz_search::SearchService;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use uuid::Uuid;

use super::group_state_postgres_tests::{create_channel, edit_channel, http_auth, test_conn};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

fn opensearch_url() -> String {
    std::env::var("BUZZ_TEST_OPENSEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".into())
}

/// The relay's `search.backend` value, changeable while the relay runs, like
/// a LaunchDarkly flag. It answers only the process context: the switch must
/// not be evaluated for the searched community.
#[derive(Default)]
struct BackendFlags(AtomicI64);

impl BackendFlags {
    fn set(&self, value: i64) {
        self.0.store(value, Ordering::SeqCst);
    }
}

impl FlagEvaluator for BackendFlags {
    fn evaluate_bool(&self, flag: BooleanFlag, _context: &EvaluationContext) -> bool {
        flag.default()
    }

    fn evaluate_int(&self, flag: IntegerFlag, context: &EvaluationContext) -> i64 {
        assert_eq!(flag, buzz_search::SEARCH_BACKEND_FLAG);
        assert_eq!(
            *context,
            EvaluationContext::for_process(),
            "search.backend is one value for the whole relay"
        );
        self.0.load(Ordering::SeqCst)
    }
}

/// A relay whose search service switches on [`BackendFlags`] over `index`.
struct Fixture {
    state: Arc<AppState>,
    flags: Arc<BackendFlags>,
    index: SearchIndex,
}

/// One community on a [`Fixture`]'s relay, with a member-only channel its
/// owner writes in and an open channel anyone can read.
struct Community {
    tenant: TenantContext,
    owner: Keys,
    private: Uuid,
    open: Uuid,
}

impl Fixture {
    async fn new(index: SearchIndex, shadow_deadline: Duration) -> Self {
        let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
            .await
            .expect("local Postgres and Redis");
        let mut state = Arc::into_inner(state).expect("no other state handle yet");
        let flags = Arc::new(BackendFlags::default());
        state.search = Arc::new(
            SearchService::with_opensearch_switch(
                state.db.pool().clone(),
                index.clone(),
                Arc::clone(&flags) as Arc<dyn FlagEvaluator>,
            )
            .with_shadow_deadline(shadow_deadline),
        );
        Self {
            state: Arc::new(state),
            flags,
            index,
        }
    }

    /// A fixture on a fresh index of its own on the local cluster.
    async fn on_opensearch() -> Self {
        let index = SearchIndex::new(&OpenSearchConfig {
            url: opensearch_url(),
            basic_auth: None,
            aws_sigv4: None,
            index_prefix: format!("relay-test-{}-", Uuid::new_v4().simple()),
        })
        .expect("OpenSearch client");
        index.ensure().await.expect("create index");
        Self::new(index, buzz_search::backend::SHADOW_DEADLINE).await
    }

    async fn community(&self) -> Community {
        let host = format!("search-backend-{}.local", Uuid::new_v4().simple());
        let id = self
            .state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("ensure community")
            .id;
        let tenant = TenantContext::resolved(id, &host);
        let owner = Keys::generate();
        let private =
            create_channel(&self.state, &tenant, &owner, &[&["visibility", "private"]]).await;
        let open = create_channel(&self.state, &tenant, &owner, &[]).await;
        edit_channel(
            &self.state,
            &tenant,
            &owner,
            open,
            &[&["visibility", "open"]],
        )
        .await
        .expect("open the channel");
        Community {
            tenant,
            owner,
            private,
            open,
        }
    }

    fn set_backend(&self, value: i64) {
        self.flags.set(value);
    }

    /// Ingest a kind:9 message from the owner in `channel`; return its ID.
    async fn post(&self, community: &Community, channel: Uuid, content: &str) -> nostr::Event {
        self.publish(
            community,
            EventBuilder::new(Kind::Custom(9), content)
                .tag(Tag::parse(["h", &channel.to_string()]).unwrap()),
        )
        .await
    }

    async fn publish(&self, community: &Community, builder: EventBuilder) -> nostr::Event {
        let event = builder.sign_with_keys(&community.owner).expect("sign");
        let result = ingest_event(
            &self.state,
            &community.tenant,
            event.clone(),
            http_auth(&community.owner),
        )
        .await
        .unwrap_or_else(|error| panic!("ingest: {error:?}"));
        assert!(result.accepted, "refused: {}", result.message);
        event
    }

    /// Index `event` under `content` and `channel`, whatever Postgres holds,
    /// and make it searchable.
    async fn index_as(
        &self,
        community: &Community,
        event: &nostr::Event,
        content: &str,
        channel: Uuid,
    ) {
        let document = SearchDocument {
            event_id: event.id.to_bytes(),
            kind: 9,
            pubkey: event.pubkey.to_bytes(),
            channel_id: Some(channel),
            created_at: chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
                .expect("timestamp"),
            content: content.into(),
            channel_type: Some("stream".into()),
            author_is_agent: false,
            thread_root: None,
            mentions: Vec::new(),
        };
        self.index
            .write(
                community.tenant.community(),
                &[document],
                &[],
                chrono::Utc::now().timestamp_micros(),
            )
            .await
            .expect("index document");
        self.index.refresh().await.expect("refresh");
    }

    /// Run a NIP-50 REQ as `reader` and return the IDs sent before EOSE, or
    /// `Err` with the CLOSED reason.
    async fn search(
        &self,
        community: &Community,
        reader: &Keys,
        word: &str,
    ) -> Result<Vec<String>, String> {
        let (conn, mut rx) = test_conn(&community.tenant, reader);
        let filters: Vec<nostr::Filter> =
            serde_json::from_value(json!([{"kinds": [9], "search": word}])).expect("filters");
        crate::handlers::req::handle_req("s".into(), filters, vec![None], conn, self.state.clone())
            .await;
        let mut ids = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            let axum::extract::ws::Message::Text(text) = frame else {
                continue;
            };
            let message: Value = serde_json::from_str(&text).expect("relay frame");
            match message[0].as_str() {
                Some("EVENT") => ids.push(message[2]["id"].as_str().expect("id").to_owned()),
                Some("EOSE") => return Ok(ids),
                Some("CLOSED") => return Err(message[2].as_str().unwrap_or_default().to_owned()),
                other => panic!("unexpected frame {other:?}: {text}"),
            }
        }
        panic!("no EOSE or CLOSED; events so far: {ids:?}");
    }

    async fn delete(&self) {
        let _ = self.index.delete().await;
    }
}

fn word(prefix: &str) -> String {
    format!("{prefix}{}", Uuid::new_v4().simple())
}

/// Metrics recorded on this thread, accumulated across snapshots (a
/// snapshot drains what it reads).
struct Recorded {
    snapshotter: Snapshotter,
    counters: HashMap<String, u64>,
    histograms: HashMap<String, Vec<f64>>,
    gauges: HashMap<String, f64>,
}

impl Recorded {
    fn new(recorder: &DebuggingRecorder) -> Self {
        Self {
            snapshotter: recorder.snapshotter(),
            counters: HashMap::new(),
            histograms: HashMap::new(),
            gauges: HashMap::new(),
        }
    }

    /// `name{label=value,...}` with labels sorted, as keys are stored.
    fn key(name: &str, labels: &[(&str, &str)]) -> String {
        let mut labels: Vec<String> = labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
        labels.sort();
        format!("{name}{{{}}}", labels.join(","))
    }

    fn collect(&mut self) {
        for (key, _, _, value) in self.snapshotter.snapshot().into_vec() {
            let key = key.key();
            let labels: Vec<(&str, &str)> = key.labels().map(|l| (l.key(), l.value())).collect();
            let name = Self::key(key.name(), &labels);
            match value {
                DebugValue::Counter(n) => *self.counters.entry(name).or_default() += n,
                DebugValue::Histogram(samples) => self
                    .histograms
                    .entry(name)
                    .or_default()
                    .extend(samples.into_iter().map(|s| s.into_inner())),
                DebugValue::Gauge(value) => {
                    self.gauges.insert(name, value.into_inner());
                }
            }
        }
    }

    fn counter(&self, name: &str, labels: &[(&str, &str)]) -> u64 {
        self.counters
            .get(&Self::key(name, labels))
            .copied()
            .unwrap_or(0)
    }

    fn samples(&self, name: &str, labels: &[(&str, &str)]) -> Vec<f64> {
        self.histograms
            .get(&Self::key(name, labels))
            .cloned()
            .unwrap_or_default()
    }

    fn gauge(&self, name: &str) -> Option<f64> {
        self.gauges.get(&Self::key(name, &[])).copied()
    }

    fn requests(&self, backend: &str, role: &str, outcome: &str) -> u64 {
        self.counter(
            "buzz_search_requests_total",
            &[("backend", backend), ("role", role), ("outcome", outcome)],
        )
    }

    fn shadow_searches(&self) -> u64 {
        ["hits", "zero_results", "empty_page", "error", "timeout"]
            .iter()
            .map(|outcome| self.requests("opensearch", "shadow", outcome))
            .sum()
    }

    /// Wait until `want` shadow searches have finished.
    async fn wait_for_shadows(&mut self, want: u64) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.collect();
            if self.shadow_searches() >= want {
                return;
            }
            assert!(Instant::now() < deadline, "shadow searches never finished");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// A stand-in for an OpenSearch cluster that accepts connections and either
/// never answers or answers every request with HTTP 500.
struct FakeCluster {
    index: SearchIndex,
    connections: Arc<AtomicUsize>,
}

impl FakeCluster {
    async fn start(answer_500: bool) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&connections);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut request = [0u8; 16 * 1024];
                    let _ = socket.read(&mut request).await;
                    if answer_500 {
                        let _ = socket
                            .write_all(
                                b"HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                            )
                            .await;
                    } else {
                        tokio::time::sleep(Duration::from_secs(120)).await;
                    }
                });
            }
        });
        let index = SearchIndex::new(&OpenSearchConfig {
            url: format!("http://{address}"),
            basic_auth: None,
            aws_sigv4: None,
            index_prefix: "fake-".into(),
        })
        .expect("client");
        Self { index, connections }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// At 0, Postgres serves and OpenSearch is never contacted; at 2, every
/// community's results come from OpenSearch, so the relay never mixes
/// backends; and changing the flag applies to the next search, with no
/// restart.
/// Mutation: route every setting to Postgres → RED (no OpenSearch-only hit
/// at 2). Evaluate the flag once at construction → RED (the switch back to 0
/// still serves OpenSearch). Evaluate it for the searched community → RED
/// (the evaluator answers only the process context).
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres, Redis and OpenSearch"]
async fn the_flag_picks_the_backend_for_the_whole_relay_while_it_runs() {
    let recorder = DebuggingRecorder::new();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let mut recorded = Recorded::new(&recorder);
    let f = Fixture::on_opensearch().await;
    let a = f.community().await;
    let b = f.community().await;

    // Postgres holds `pg_word`; the index holds the same messages under
    // `os_word`, so each word shows which backend answered.
    let pg_word = word("pgword");
    let os_word = word("osword");
    let mut expected = HashMap::new();
    for community in [&a, &b] {
        let message = f.post(community, community.private, &pg_word).await;
        f.index_as(community, &message, &os_word, community.private)
            .await;
        expected.insert(community.tenant.community(), vec![message.id.to_hex()]);
    }
    let expected = |community: &Community| Ok(expected[&community.tenant.community()].clone());

    // 0 everywhere: Postgres.
    assert_eq!(f.search(&a, &a.owner, &pg_word).await, expected(&a));
    assert_eq!(f.search(&a, &a.owner, &os_word).await, Ok(vec![]));
    recorded.collect();
    assert_eq!(recorded.requests("opensearch", "primary", "hits"), 0);
    assert_eq!(recorded.shadow_searches(), 0);
    assert_eq!(recorded.requests("postgres", "primary", "hits"), 1);
    assert_eq!(recorded.requests("postgres", "primary", "zero_results"), 1);

    // 2: both communities move together.
    f.set_backend(2);
    for community in [&a, &b] {
        assert_eq!(
            f.search(community, &community.owner, &os_word).await,
            expected(community)
        );
        assert_eq!(
            f.search(community, &community.owner, &pg_word).await,
            Ok(vec![])
        );
    }
    recorded.collect();
    assert_eq!(recorded.requests("opensearch", "primary", "hits"), 2);
    assert_eq!(
        recorded.requests("opensearch", "primary", "zero_results"),
        2
    );
    assert_eq!(
        recorded
            .samples(
                "buzz_search_duration_seconds",
                &[("backend", "opensearch"), ("role", "primary")]
            )
            .len(),
        4
    );

    // Back to 0, with no restart.
    f.set_backend(0);
    for community in [&a, &b] {
        assert_eq!(
            f.search(community, &community.owner, &pg_word).await,
            expected(community)
        );
        assert_eq!(
            f.search(community, &community.owner, &os_word).await,
            Ok(vec![])
        );
    }
    f.delete().await;
}

/// At 0, a cluster that would answer is never contacted.
/// Mutation: start the shadow search at every setting → RED.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres and Redis"]
async fn at_zero_opensearch_is_never_queried() {
    let cluster = FakeCluster::start(true).await;
    let f = Fixture::new(cluster.index.clone(), Duration::from_secs(1)).await;
    let c = f.community().await;
    let needle = word("zero");
    let message = f.post(&c, c.private, &needle).await;

    assert_eq!(
        f.search(&c, &c.owner, &needle).await,
        Ok(vec![message.id.to_hex()])
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(cluster.connections(), 0, "OpenSearch was contacted at 0");
}

/// At 1, Postgres serves; OpenSearch runs the same query, and overlap,
/// latency and zero-result metrics are recorded for both, with one log line
/// per comparison that never holds the query text.
/// Mutation: serve the shadow result → RED (the OpenSearch-only word finds
/// a message). Skip the overlap histogram → RED.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres, Redis and OpenSearch"]
async fn shadow_serves_postgres_and_records_both_backends() {
    let recorder = DebuggingRecorder::new();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let (_logs, logs) = crate::access::tests::capture_scoped();
    let mut recorded = Recorded::new(&recorder);
    let f = Fixture::on_opensearch().await;
    let c = f.community().await;
    f.set_backend(1);

    let shared = word("shadowshared");
    let os_only = word("shadowos");
    let missing = word("shadownone");
    let both = f.post(&c, c.private, &shared).await;
    f.index_as(&c, &both, &shared, c.private).await;
    let indexed_only = f.post(&c, c.private, "unrelated words").await;
    f.index_as(&c, &indexed_only, &os_only, c.private).await;

    // Another test's thread may have registered the log callsites before
    // this capture existed, caching them as disabled.
    tracing::callsite::rebuild_interest_cache();
    assert_eq!(
        f.search(&c, &c.owner, &shared).await,
        Ok(vec![both.id.to_hex()])
    );
    assert_eq!(
        f.search(&c, &c.owner, &os_only).await,
        Ok(vec![]),
        "Postgres serves in shadow mode"
    );
    assert_eq!(f.search(&c, &c.owner, &missing).await, Ok(vec![]));
    recorded.wait_for_shadows(3).await;

    for backend in ["postgres", "opensearch"] {
        let role = if backend == "postgres" {
            "primary"
        } else {
            "shadow"
        };
        assert_eq!(
            recorded
                .samples(
                    "buzz_search_duration_seconds",
                    &[("backend", backend), ("role", role)]
                )
                .len(),
            3,
            "{backend} latency"
        );
    }
    assert_eq!(recorded.requests("postgres", "primary", "hits"), 1);
    assert_eq!(recorded.requests("postgres", "primary", "zero_results"), 2);
    assert_eq!(recorded.requests("opensearch", "shadow", "hits"), 2);
    assert_eq!(recorded.requests("opensearch", "shadow", "zero_results"), 1);
    let mut overlaps = recorded.samples("buzz_search_shadow_overlap_ratio", &[]);
    overlaps.sort_by(f64::total_cmp);
    assert_eq!(
        overlaps,
        vec![0.0, 1.0],
        "shared word, then OpenSearch-only"
    );

    let raw = logs.raw();
    let comparisons: Vec<Value> = raw
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON log line"))
        .filter(|line| line["message"] == "search shadow comparison")
        .collect();
    assert_eq!(comparisons.len(), 3, "one line per comparison: {raw}");
    assert!(comparisons
        .iter()
        .all(|line| line["community"] == c.tenant.community().to_string()));
    for text in [&shared, &os_only, &missing] {
        assert!(!raw.contains(text.as_str()), "query text logged: {raw}");
    }
    f.delete().await;
}

/// At 1, an OpenSearch that hangs or fails neither slows nor fails the
/// response; its searches count as a timeout or an error.
/// Mutation: await the shadow search before returning → RED (the hanging
/// cluster holds the REQ for the deadline).
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres and Redis"]
async fn a_slow_or_failing_shadow_does_not_touch_the_response() {
    const DEADLINE: Duration = Duration::from_secs(3);
    let recorder = DebuggingRecorder::new();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let mut recorded = Recorded::new(&recorder);
    for (answer_500, outcome) in [(false, "timeout"), (true, "error")] {
        let cluster = FakeCluster::start(answer_500).await;
        let f = Fixture::new(cluster.index.clone(), DEADLINE).await;
        let c = f.community().await;
        f.set_backend(1);
        let needle = word("slow");
        let message = f.post(&c, c.private, &needle).await;

        let started = Instant::now();
        assert_eq!(
            f.search(&c, &c.owner, &needle).await,
            Ok(vec![message.id.to_hex()]),
            "{outcome}"
        );
        assert!(
            started.elapsed() < DEADLINE / 2,
            "{outcome}: the response waited on OpenSearch ({:?})",
            started.elapsed()
        );
        recorded.wait_for_shadows(1).await;
        assert_eq!(recorded.requests("opensearch", "shadow", outcome), 1);
        assert_eq!(cluster.connections(), 1, "{outcome}");
        recorded.counters.clear();
    }
}

/// At 2, an OpenSearch failure fails the search through the usual CLOSED
/// error: no silent fallback to Postgres.
/// Mutation: answer from Postgres when OpenSearch fails → RED.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres and Redis"]
async fn at_two_an_opensearch_failure_fails_the_search() {
    let recorder = DebuggingRecorder::new();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let mut recorded = Recorded::new(&recorder);
    let cluster = FakeCluster::start(true).await;
    let f = Fixture::new(cluster.index.clone(), Duration::from_secs(1)).await;
    let c = f.community().await;
    f.set_backend(2);
    let needle = word("down");
    f.post(&c, c.private, &needle).await;

    assert_eq!(
        f.search(&c, &c.owner, &needle).await,
        Err(super::req::SEARCH_FAILED_CLOSED.to_owned())
    );
    recorded.collect();
    assert_eq!(recorded.requests("opensearch", "primary", "error"), 1);
    assert_eq!(recorded.requests("postgres", "primary", "hits"), 0);
}

/// At every value, each hit is re-read from Postgres and re-checked: a
/// message the searcher cannot read is never returned, even when the index
/// files it under a channel the searcher can read.
/// Mutation: skip the hydrated channel check in `handle_search_req` → RED at
/// 2.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres, Redis and OpenSearch"]
async fn every_value_rechecks_access_on_every_hit() {
    let f = Fixture::on_opensearch().await;
    let c = f.community().await;
    let outsider = Keys::generate();
    let secret = word("secret");
    let message = f.post(&c, c.private, &secret).await;
    // Stale index: it says the private message is in the open channel.
    f.index_as(&c, &message, &secret, c.open).await;

    for value in [0, 1, 2] {
        f.set_backend(value);
        assert_eq!(
            f.search(&c, &outsider, &secret).await,
            Ok(vec![]),
            "an outsider found a private message at {value}"
        );
        assert_eq!(
            f.search(&c, &c.owner, &secret).await,
            Ok(vec![message.id.to_hex()]),
            "the member lost their message at {value}"
        );
    }
    f.delete().await;
}

/// At 2, with the indexer paused, a deleted message still in the index is
/// never returned: the relay's re-read drops it.
/// Mutation: emit hits without the Postgres re-read → RED.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres, Redis and OpenSearch"]
async fn at_two_a_deleted_message_left_in_the_index_is_dropped() {
    let f = Fixture::on_opensearch().await;
    let c = f.community().await;
    f.set_backend(2);
    let gone = word("gone");
    let message = f.post(&c, c.private, &gone).await;
    f.index_as(&c, &message, &gone, c.private).await;
    assert_eq!(
        f.search(&c, &c.owner, &gone).await,
        Ok(vec![message.id.to_hex()])
    );

    f.publish(
        &c,
        EventBuilder::new(Kind::EventDeletion, "")
            .tag(Tag::event(message.id))
            .tag(Tag::parse(["h", &c.private.to_string()]).unwrap()),
    )
    .await;

    assert_eq!(f.search(&c, &c.owner, &gone).await, Ok(vec![]));
    f.delete().await;
}

/// At 2, a member removed from a private channel no longer finds its
/// messages, though the index still files them under that channel.
/// Mutation: keep the reader's cached channel access after the removal →
/// RED. Skipping only the per-hit channel check stays green: the query is
/// already scoped to the reader's channels.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres, Redis and OpenSearch"]
async fn at_two_a_removed_member_no_longer_finds_the_channel() {
    let f = Fixture::on_opensearch().await;
    let c = f.community().await;
    f.set_backend(2);
    let reader = Keys::generate();
    let membership = |kind: u16| {
        EventBuilder::new(Kind::Custom(kind), "")
            .tag(Tag::parse(["h", &c.private.to_string()]).unwrap())
            .tag(Tag::public_key(reader.public_key()))
    };
    f.publish(&c, membership(9000)).await;
    let members = word("members");
    let message = f.post(&c, c.private, &members).await;
    f.index_as(&c, &message, &members, c.private).await;
    assert_eq!(
        f.search(&c, &reader, &members).await,
        Ok(vec![message.id.to_hex()])
    );

    f.publish(&c, membership(9001)).await;
    assert_eq!(f.search(&c, &reader, &members).await, Ok(vec![]));
    f.delete().await;
}

/// At 2, a document filed under one community for a message of another is
/// never returned: the relay re-reads each hit in the searching community.
/// Channel ids are unique only within a community, so the other community
/// holds a channel with the same id as one the reader belongs to, and the
/// channel check alone would let the hit through.
/// Mutation: re-read hits without the community → RED.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres, Redis and OpenSearch"]
async fn at_two_a_document_of_another_community_is_dropped() {
    let f = Fixture::on_opensearch().await;
    let here = f.community().await;
    let there = f.community().await;
    f.set_backend(2);
    let twin = EventBuilder::new(Kind::Custom(9007), "")
        .tag(Tag::parse(["h", &here.open.to_string()]).unwrap())
        .tag(Tag::parse(["name", "twin"]).unwrap());
    f.publish(&there, twin).await;
    let elsewhere = word("elsewhere");
    let message = f.post(&there, here.open, &elsewhere).await;
    // Misfiled: the index says the other community's message is here, in
    // the same-id channel this community's owner can read.
    f.index_as(&here, &message, &elsewhere, here.open).await;
    assert_eq!(f.search(&here, &here.owner, &elsewhere).await, Ok(vec![]));
    f.delete().await;
}

/// The outbox gauges follow the queue: with the indexer paused, depth counts
/// the queued rows and lag ages with the oldest; once the indexer drains the
/// queue, both read zero. Runs on a scratch database so the counts are this
/// test's alone.
/// Mutation: take lag from the newest row → RED.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres and OpenSearch"]
async fn outbox_gauges_follow_a_paused_and_resumed_indexer() {
    let recorder = DebuggingRecorder::new();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let mut recorded = Recorded::new(&recorder);
    let url = crate::test_support::database_url();
    let admin = sqlx::PgPool::connect(&url).await.expect("connect admin");
    let name = format!("search_outbox_{}", Uuid::new_v4().simple());
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

    let read = |recorded: &mut Recorded| {
        recorded.collect();
        (
            recorded.gauge("buzz_search_outbox_depth"),
            recorded.gauge("buzz_search_index_lag_seconds"),
        )
    };
    buzz_search::outbox_metrics::record(&pool)
        .await
        .expect("sample");
    assert_eq!(read(&mut recorded), (Some(0.0), Some(0.0)));

    // Paused indexer: two queued changes, the older from 90 s ago.
    let db = buzz_db::Db::from_pool(pool.clone());
    let community = *db
        .ensure_configured_community("outbox-gauges.local")
        .await
        .expect("community")
        .id
        .as_uuid();
    for age in [90, 5] {
        sqlx::query(
            "INSERT INTO search_outbox (community_id, event_id, event_created_at, created_at) \
             VALUES ($1, $2, now(), now() - make_interval(secs => $3))",
        )
        .bind(community)
        .bind(Uuid::new_v4().as_bytes().repeat(2))
        .bind(f64::from(age))
        .execute(&pool)
        .await
        .expect("queue a change");
    }
    buzz_search::outbox_metrics::record(&pool)
        .await
        .expect("sample");
    let (depth, lag) = read(&mut recorded);
    assert_eq!(depth, Some(2.0));
    let lag = lag.expect("lag");
    assert!((90.0..120.0).contains(&lag), "lag {lag}");

    // Resumed: the indexer drains the queue (the events are gone, so it
    // removes their documents).
    let index = SearchIndex::new(&OpenSearchConfig {
        url: opensearch_url(),
        basic_auth: None,
        aws_sigv4: None,
        index_prefix: format!("relay-test-{}-", Uuid::new_v4().simple()),
    })
    .expect("OpenSearch client");
    index.ensure().await.expect("create index");
    buzz_search::indexer::Indexer::new(db, index.clone())
        .drain()
        .await
        .expect("drain");
    buzz_search::outbox_metrics::record(&pool)
        .await
        .expect("sample");
    assert_eq!(read(&mut recorded), (Some(0.0), Some(0.0)));

    let _ = index.delete().await;
    pool.close().await;
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
    )))
    .execute(&admin)
    .await;
}

/// Prefix mode, which OpenSearch does not serve yet, stays on Postgres at
/// every value and is counted: shadow skips the comparison, and 2 serves it
/// from Postgres. Not a fallback: OpenSearch is never tried.
/// Mutation: route prefix searches like word searches → RED (the cluster is
/// contacted, and at 2 the search fails).
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres and Redis"]
async fn prefix_searches_stay_on_postgres_and_are_counted() {
    let recorder = DebuggingRecorder::new();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let mut recorded = Recorded::new(&recorder);
    let cluster = FakeCluster::start(true).await;
    let f = Fixture::new(cluster.index.clone(), Duration::from_secs(1)).await;
    let c = f.community().await;
    let needle = word("prefixed");
    let message = f.post(&c, c.private, &needle).await;
    let query = buzz_search::SearchQuery {
        community: c.tenant.community(),
        q: needle[..12].to_owned(),
        channel_scope: buzz_search::ChannelScope::Channels(vec![c.private]),
        kinds: Some(vec![9]),
        authors: None,
        since: None,
        until: None,
        page: 1,
        per_page: 10,
        mode: buzz_search::SearchMode::Prefix,
        sort: buzz_search::SearchSort::Relevance,
    };

    for (value, setting) in [(1, "shadow"), (2, "opensearch")] {
        f.set_backend(value);
        let hits = f.state.search.search(&query).await.expect("search");
        assert_eq!(
            hits.hits.iter().map(|hit| hit.event_id).collect::<Vec<_>>(),
            vec![message.id.to_bytes()],
            "{setting}"
        );
        recorded.collect();
        assert_eq!(
            recorded.counter(
                "buzz_search_opensearch_skipped_total",
                &[("setting", setting), ("reason", "unsupported_mode")]
            ),
            1,
            "{setting}"
        );
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(cluster.connections(), 0);
}

/// The index holds only message kinds, so a search that names another kind,
/// mixes one in, or names no kinds stays on Postgres at 1 and 2, is not
/// shadowed, and counts as skipped with `unindexed_kinds`. That rule comes
/// before the mode rule, so a people search (kind 0, in prefix mode as
/// clients send it) counts there too. A message-kind search at 2 still
/// reaches the cluster.
/// Mutation: drop the kind check from routing → RED (the cluster is
/// contacted and fails the word searches at 2).
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres and Redis"]
async fn searches_of_unindexed_kinds_stay_on_postgres() {
    let recorder = DebuggingRecorder::new();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let mut recorded = Recorded::new(&recorder);
    let cluster = FakeCluster::start(true).await;
    let f = Fixture::new(cluster.index.clone(), Duration::from_secs(1)).await;
    let c = f.community().await;
    let needle = word("unindexed")[..16].to_owned();
    let message = f.post(&c, c.private, &needle).await;
    let profile = EventBuilder::new(Kind::Metadata, json!({ "name": needle }).to_string())
        .sign_with_keys(&c.owner)
        .expect("sign");
    f.state
        .db
        .insert_event(c.tenant.community(), &profile, None)
        .await
        .expect("store the profile");
    let query = |kinds: Option<Vec<i32>>, mode: buzz_search::SearchMode| buzz_search::SearchQuery {
        community: c.tenant.community(),
        q: match mode {
            buzz_search::SearchMode::FullText => needle.clone(),
            buzz_search::SearchMode::Prefix => needle[..12].to_owned(),
        },
        channel_scope: buzz_search::ChannelScope::ChannelsOrChannelLess(vec![c.private]),
        kinds,
        authors: None,
        since: None,
        until: None,
        page: 1,
        per_page: 10,
        mode,
        sort: buzz_search::SearchSort::Relevance,
    };
    let found = |query: buzz_search::SearchQuery| {
        let search = Arc::clone(&f.state.search);
        async move {
            let mut ids: Vec<[u8; 32]> = search
                .search(&query)
                .await
                .expect("Postgres serves")
                .hits
                .iter()
                .map(|hit| hit.event_id)
                .collect();
            ids.sort();
            ids
        }
    };
    let mut both = vec![message.id.to_bytes(), profile.id.to_bytes()];
    both.sort();
    let words = buzz_search::SearchMode::FullText;
    let prefix = buzz_search::SearchMode::Prefix;

    f.set_backend(2);
    assert_eq!(
        found(query(Some(vec![0]), words)).await,
        vec![profile.id.to_bytes()]
    );
    assert_eq!(found(query(Some(vec![9, 0]), words)).await, both);
    assert_eq!(found(query(None, words)).await, both);
    assert_eq!(
        found(query(Some(vec![0]), prefix)).await,
        vec![profile.id.to_bytes()]
    );
    f.set_backend(1);
    assert_eq!(found(query(Some(vec![9, 0]), words)).await, both);

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(cluster.connections(), 0, "OpenSearch was contacted");
    recorded.collect();
    let skipped = |setting: &str, reason: &str| {
        recorded.counter(
            "buzz_search_opensearch_skipped_total",
            &[("setting", setting), ("reason", reason)],
        )
    };
    assert_eq!(
        (
            skipped("opensearch", "unindexed_kinds"),
            skipped("shadow", "unindexed_kinds"),
            skipped("opensearch", "unsupported_mode"),
        ),
        (4, 1, 0)
    );

    f.set_backend(2);
    f.state
        .search
        .search(&query(Some(vec![9]), words))
        .await
        .expect_err("a message search at 2 goes to the failing cluster");
}
