//! Drives the simulated population through the scenario's ramp.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use buzz_core::kind::{
    KIND_AGENT_OBSERVER_FRAME, KIND_DELETION, KIND_GIT_REPO_ANNOUNCEMENT,
    KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION, KIND_NIP29_GROUP_METADATA,
    KIND_PRESENCE_UPDATE, KIND_PROFILE, KIND_PROJECT, KIND_REACTION, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_EDIT, KIND_STREAM_MESSAGE_V2, KIND_STREAM_REMINDER, KIND_TYPING_INDICATOR,
    KIND_WORKFLOW_APPROVAL_REQUESTED,
};
use buzz_core::observer::{
    encrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY,
};
use buzz_ws_client::connection::PUBLISH_OK_TIMEOUT_SECS;
use buzz_ws_client::{NostrWsConnection, OkResponse, RelayMessage, WsClientError};
use nostr::{
    Alphabet, Event, EventBuilder, Filter, Keys, Kind, PublicKey, SingleLetterTag, Tag, Timestamp,
};
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex};

use super::desktop;
use super::identity::{self, Role};
use super::net::{http_reason, Net};
use super::scenario::{AgentTurn, Community, HumanProfile, Members, Scenario, WideSplit};
use super::seed::{
    channel_creator, find_roots, hot_root_keys, reply_tags, HOT_ROOT_PREFIX, ROOT_PREFIX,
};
use super::stats::{abort_reason, ops_json, Op, Pending, Stats};

/// Content prefix of timed messages: `lg:<send unix micros>`.
const TIMED_PREFIX: &str = "lg:";
/// Longest an idle client blocks on its socket before rechecking the stop flag.
pub(super) const MAX_IDLE: Duration = Duration::from_secs(1);
/// How long stopped clients get to settle what they have in flight and
/// disconnect before they are cancelled.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);
/// Live subscriptions every human opens after connecting.
const HUMAN_SUBS: [&str; 4] = ["live", "mentions", "typing", "presence"];
/// Prefix of the finite history subscriptions a human opens and closes.
const HISTORY_PREFIX: &str = "history-";

/// `i/n`: this process runs every global client index `g` with `g % n == i`.
#[derive(Clone, Copy, Debug)]
pub struct Shard {
    pub index: usize,
    pub count: usize,
}

impl Shard {
    pub fn parse(text: &str) -> Result<Self, String> {
        let (i, n) = text.split_once('/').ok_or("shard must be i/n")?;
        let index: usize = i.parse().map_err(|_| "shard index must be a number")?;
        let count: usize = n.parse().map_err(|_| "shard count must be a number")?;
        if count == 0 || index >= count {
            return Err("shard needs 0 <= i < n".into());
        }
        Ok(Self { index, count })
    }

    fn owns(&self, global: usize) -> bool {
        global % self.count == self.index
    }
}

pub(super) struct Shared {
    pub(super) scenario: Scenario,
    pub(super) communities: Vec<Community>,
    pub(super) net: Net,
    pub(super) stats: Arc<Stats>,
    pub(super) stop: watch::Receiver<bool>,
    /// Seeded thread roots per channel id, resolved lazily by whichever client needs them first.
    roots: Mutex<HashMap<String, Arc<Vec<String>>>>,
    /// Index within its community of every channel id, to name the
    /// [`channel_creator`] whose roots a lookup searches for.
    channel_indices: HashMap<String, usize>,
    /// Channel members and human pubkeys, built once by the first desktop
    /// client that needs them.
    pub(super) directory: tokio::sync::OnceCell<Arc<Directory>>,
}

/// Who is in each channel, for clients that read their co-members' profiles.
pub(super) struct Directory {
    pub(super) members: Members,
    /// Hex pubkey of every human up to the peak population, by global index.
    pub(super) humans: Vec<String>,
}

impl Shared {
    pub(super) fn new(
        scenario: Scenario,
        net: Net,
        stats: Arc<Stats>,
        stop: watch::Receiver<bool>,
    ) -> Self {
        let channel_indices = (0..scenario.shape.communities)
            .flat_map(|community| {
                scenario
                    .channel_ids(community)
                    .into_iter()
                    .enumerate()
                    .map(|(index, id)| (id, index))
            })
            .collect();
        Self {
            communities: scenario.communities(),
            scenario,
            net,
            stats,
            stop,
            roots: Mutex::new(HashMap::new()),
            channel_indices,
            directory: tokio::sync::OnceCell::new(),
        }
    }

    /// The [`Directory`], derived off the async workers on first use: it
    /// derives a key for every human up to the peak population.
    pub(super) async fn directory(&self) -> Arc<Directory> {
        self.directory
            .get_or_init(|| async {
                let scenario = self.scenario.clone();
                tokio::task::spawn_blocking(move || {
                    let humans = (0..scenario.peak().humans)
                        .map(|i| {
                            identity::keys(&scenario.run_id, Role::Human, i)
                                .public_key()
                                .to_hex()
                        })
                        .collect();
                    Arc::new(Directory {
                        members: scenario.channel_members(),
                        humans,
                    })
                })
                .await
                .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
            })
            .await
            .clone()
    }
}

/// Runs every ramp step and returns the JSON report; `Err` when a step aborted.
pub async fn run(scenario: Scenario, net: Net, shard: Shard) -> Result<Value, Value> {
    run_with(scenario, net, shard, SHUTDOWN_GRACE).await
}

/// [`run`] with the shutdown grace period given, so tests need not wait out
/// [`SHUTDOWN_GRACE`] for a client that never settles.
async fn run_with(
    scenario: Scenario,
    net: Net,
    shard: Shard,
    grace: Duration,
) -> Result<Value, Value> {
    let stats = Arc::new(Stats::default());
    let (stop_tx, stop) = watch::channel(false);
    let shared = Arc::new(Shared::new(scenario, net, stats.clone(), stop));
    let scenario = &shared.scenario;
    let pop = &scenario.population;
    let connect_gap = Duration::from_secs_f64(1.0 / pop.connect_rate_per_sec);

    let mut tasks = Vec::new();
    let (mut humans, mut agents, mut authors) = (0usize, 0usize, 0usize);
    let mut steps = Vec::new();
    let mut aborted = false;
    // End of the last reported window and start of the last judged one.
    let mut reported = stats.snapshot();
    let mut last_window = reported.clone();

    for (n, step) in scenario.ramp.iter().enumerate() {
        let want_humans = (pop.humans as f64 * step.users).ceil() as usize;
        let want_agents = (pop.agents as f64 * step.users).ceil() as usize;
        let want_authors = (scenario.hot_thread.authors as f64 * step.users).ceil() as usize;
        // The window covers the ramp too, so connect/auth and history
        // failures from new clients count toward the abort check.
        let before = stats.snapshot();
        last_window = before.clone();
        let ramp_start = Instant::now();
        while humans < want_humans || agents < want_agents {
            // Interleave so agents and humans arrive in proportion.
            let spawn_agent = agents < want_agents
                && (humans >= want_humans || agents * want_humans <= humans * want_agents);
            let global = if spawn_agent { agents } else { humans };
            if shard.owns(global) {
                let shared = shared.clone();
                tasks.push(tokio::spawn(async move {
                    if spawn_agent {
                        agent(shared, global).await
                    } else if shared.scenario.is_desktop(global) {
                        desktop::desktop(shared, global).await
                    } else {
                        human(shared, global).await
                    }
                }));
                tokio::time::sleep(connect_gap).await;
            }
            if spawn_agent {
                agents += 1;
            } else {
                humans += 1;
            }
        }
        // Hot-thread authors hold no socket, so they start without pacing.
        while authors < want_authors {
            if shard.owns(authors) {
                let (shared, index) = (shared.clone(), authors);
                tasks.push(tokio::spawn(hot_author(shared, index)));
            }
            authors += 1;
        }
        let ramp_secs = ramp_start.elapsed().as_secs_f64();

        let hold_start = Instant::now();
        tokio::time::sleep(Duration::from_secs(step.hold_secs)).await;
        let hold_secs = hold_start.elapsed().as_secs_f64();
        // Clients only return once stopped, so a finished one panicked.
        let lost = tasks.iter().filter(|t| t.is_finished()).count();
        // Stop before the last step's snapshot, so the shutdown settles only
        // work already in flight when that step's window closed.
        if n + 1 == scenario.ramp.len() {
            let _ = stop_tx.send(true);
        }
        let after = stats.snapshot();
        let summaries = after.since(&before);
        let reason = if lost > 0 {
            Some(format!("{lost} clients exited unexpectedly"))
        } else {
            abort_reason(
                &summaries,
                scenario.abort.max_error_rate,
                scenario.abort.max_p99_ms,
            )
        };
        let report = json!({
            "step": n,
            "users": step.users,
            "humans": want_humans,
            "agents": want_agents,
            "hot_authors": want_authors,
            "shard": format!("{}/{}", shard.index, shard.count),
            "ramp_secs": (ramp_secs * 10.0).round() / 10.0,
            "hold_secs": (hold_secs * 10.0).round() / 10.0,
            "ops": ops_json(&summaries, ramp_secs + hold_secs),
            "failures": after
                .reasons_since(&before, 10)
                .into_iter()
                .map(|(reason, count)| json!({"reason": reason, "count": count}))
                .collect::<Vec<_>>(),
            "aborted": reason.is_some(),
            "abort_reason": reason,
        });
        eprintln!("{report}");
        steps.push(report);
        reported = after;
        if reason.is_some() {
            aborted = true;
            break;
        }
    }

    let _ = stop_tx.send(true);
    let shutdown_start = Instant::now();
    let (panicked, cancelled) = shutdown(tasks, grace).await;
    // Operations still in flight at the stop settle now, after the last step
    // was judged: a stalled connect as a connect timeout, a cancelled client
    // as `unfinished at stop`. They are judged together with the last step's
    // window, not alone: work in flight at an instant is skewed toward slow
    // operations, so on its own it would fail healthy runs on p99.
    let closing = stats.snapshot();
    let closed = closing.since(&reported);
    let shutdown_reason = abort_reason(
        &closing.since(&last_window),
        shared.scenario.abort.max_error_rate,
        shared.scenario.abort.max_p99_ms,
    );
    let report = json!({
        "run_id": shared.scenario.run_id,
        "steps": steps,
        "shutdown": {
            "secs": (shutdown_start.elapsed().as_secs_f64() * 10.0).round() / 10.0,
            "ops": ops_json(&closed, shutdown_start.elapsed().as_secs_f64()),
            "failures": closing
                .reasons_since(&reported, 10)
                .into_iter()
                .map(|(reason, count)| json!({"reason": reason, "count": count}))
                .collect::<Vec<_>>(),
            "abort_reason": shutdown_reason,
        },
        "clients_panicked": panicked,
        "clients_cancelled": cancelled,
    });
    if failed(aborted, panicked, cancelled, shutdown_reason.as_deref()) {
        Err(report)
    } else {
        Ok(report)
    }
}

/// A run fails when a step aborted, a client panicked, shutdown had to cancel
/// a client, or the operations that settled during shutdown crossed the abort
/// limits.
fn failed(aborted: bool, panicked: usize, cancelled: usize, shutdown: Option<&str>) -> bool {
    aborted || panicked > 0 || cancelled > 0 || shutdown.is_some()
}

/// Waits up to `grace` for stopped clients, then cancels the rest and waits
/// for their futures to drop. Returns how many panicked and how many were
/// cancelled.
async fn shutdown(tasks: Vec<tokio::task::JoinHandle<()>>, grace: Duration) -> (usize, usize) {
    let deadline = tokio::time::Instant::now() + grace;
    let (mut panicked, mut cancelled) = (0, 0);
    for mut task in tasks {
        match tokio::time::timeout_at(deadline, &mut task).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => panicked += usize::from(e.is_panic()),
            Err(_) => {
                task.abort();
                // Dropping the future records what the client left in flight.
                let _ = task.await;
                cancelled += 1;
            }
        }
    }
    (panicked, cancelled)
}

/// Small deterministic PRNG so every client draws an independent schedule.
pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub(super) fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// Next arrival of a Poisson process with `per_hour` events per hour.
    pub(super) fn after(&mut self, per_hour: f64) -> Option<Instant> {
        if per_hour <= 0.0 {
            return None;
        }
        let secs = -(1.0 - self.unit()).ln() * 3600.0 / per_hour;
        Some(Instant::now() + Duration::from_secs_f64(secs))
    }
}

pub(super) fn now_micros() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
}

pub(super) fn tag<const N: usize>(parts: [&str; N]) -> Tag {
    Tag::parse(parts).expect("static tag shape")
}

pub(super) fn kind(k: u32) -> Kind {
    Kind::Custom(k as u16)
}

pub(super) fn stopped(shared: &Shared) -> bool {
    *shared.stop.borrow()
}

/// Resolves once the run is stopping.
pub(super) async fn stopping(shared: &Shared) {
    let mut stop = shared.stop.clone();
    let _ = stop.wait_for(|s| *s).await;
}

/// Jittered exponential backoff between failed connects or failed sessions.
pub(super) struct Backoff(f64);

impl Backoff {
    const MAX_SECS: f64 = 30.0;

    pub(super) fn new() -> Self {
        Self(1.0)
    }

    /// Sleeps for the next interval, returning early when the run stops.
    pub(super) async fn wait(&mut self, shared: &Shared, rng: &mut Rng) {
        let wait = Duration::from_secs_f64(self.0 * (0.5 + rng.unit()));
        self.0 = (self.0 * 2.0).min(Self::MAX_SECS);
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stopping(shared) => {}
        }
    }
}

/// A stream message a client can react to: its id and channel.
#[derive(Clone)]
pub(super) struct Seen {
    id: String,
    channel: String,
}

fn seen(message: &RelayMessage) -> Option<Seen> {
    let RelayMessage::Event { event, .. } = message else {
        return None;
    };
    if event.kind != kind(KIND_STREAM_MESSAGE) {
        return None;
    }
    let channel = event
        .tags
        .iter()
        .find(|t| t.kind().as_str() == "h")
        .and_then(|t| t.content())?;
    Some(Seen {
        id: event.id.to_hex(),
        channel: channel.to_string(),
    })
}

/// Handles a message for a live subscription. `EOSE` completes a subscribe
/// and `CLOSED` is a rejection that ends the session. Events record delivery
/// latency and become reaction targets. Late messages for a history
/// subscription, including the relay's `CLOSED` reply to our `CLOSE`, are
/// ignored.
fn live(
    stats: &Stats,
    message: &RelayMessage,
    subscribed: Instant,
    last_message: &mut Option<Seen>,
) -> Result<(), String> {
    if subscription_of(message).is_some_and(|id| id.starts_with(HISTORY_PREFIX)) {
        return Ok(());
    }
    match message {
        RelayMessage::Eose { .. } => stats.ok(Op::Subscribe, subscribed.elapsed()),
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            let reason = format!("CLOSED {subscription_id}: {message}");
            stats.err(Op::Subscribe, &reason);
            return Err(reason);
        }
        _ => {
            if let Some(target) = observe(stats, message) {
                *last_message = Some(target);
            }
        }
    }
    Ok(())
}

/// Records end-to-end delivery latency for a live timed message and returns
/// it as a reaction target. Only live subscriptions call this: history
/// replays old messages, whose age is not delivery latency.
pub(super) fn observe(stats: &Stats, message: &RelayMessage) -> Option<Seen> {
    if let RelayMessage::Event { event, .. } = message {
        if let Some(sent) = event
            .content
            .strip_prefix(TIMED_PREFIX)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|micros| micros.parse::<u128>().ok())
        {
            let micros = now_micros().saturating_sub(sent).min(u64::MAX as u128) as u64;
            stats.ok(Op::Deliver, Duration::from_micros(micros));
        }
    }
    seen(message)
}

/// Publishes over the socket and records send→OK latency. `true` when the
/// relay answered, accepting or not.
async fn publish(
    stats: &Stats,
    conn: &mut NostrWsConnection,
    op: Op,
    event: Event,
) -> Result<bool, WsClientError> {
    let pending = stats.begin(op);
    match conn.send_event(event).await {
        Ok(ok) if ok.accepted => pending.ok(),
        Ok(ok) => pending.err(&ok.message),
        Err(WsClientError::Timeout) => {
            pending.err("OK timeout");
            return Ok(false);
        }
        Err(e) => {
            // The caller records the broken socket.
            pending.forget();
            return Err(e);
        }
    }
    Ok(true)
}

/// Connects with retry until connected or stopped. Each attempt is bounded
/// by the connect deadline, which should be shorter than [`SHUTDOWN_GRACE`]
/// so an attempt under way at the stop settles before clients are cancelled.
pub(super) async fn connect(
    shared: &Shared,
    community: &Community,
    keys: &Keys,
    auth: Option<&Tag>,
    rng: &mut Rng,
    backoff: &mut Backoff,
) -> Option<NostrWsConnection> {
    while !stopped(shared) {
        // An attempt already under way runs to its connect deadline even when
        // the run stops, so it settles as a success or an error rather than
        // vanishing; the loop then ends instead of starting another.
        let pending = shared.stats.begin(Op::WsConnect);
        match shared.net.connect(community, keys, auth).await {
            Ok(conn) => {
                pending.ok();
                // Shutdown drains work already in flight; it does not start
                // a session for a connect that finished after the stop.
                if stopped(shared) {
                    let _ = conn.disconnect().await;
                    return None;
                }
                return Some(conn);
            }
            Err(e) => pending.err(&e.to_string()),
        }
        backoff.wait(shared, rng).await;
    }
    None
}

/// `read` under `limit`, abandoned once the run stops: `None` when it was,
/// as EOSE waits are abandoned at stop. A read still running then would
/// outlive the shutdown grace and fail a healthy run.
pub(super) async fn bounded<T>(
    shared: &Shared,
    limit: Duration,
    read: impl std::future::Future<Output = T>,
) -> Option<Result<T, tokio::time::error::Elapsed>> {
    if stopped(shared) {
        return None;
    }
    tokio::select! {
        result = tokio::time::timeout(limit, read) => Some(result),
        () = stopping(shared) => None,
    }
}

/// Thread roots of `channel`, cached once found. Concurrent first callers may
/// each query; a failed query is not cached, so a later reply retries it.
/// With `bound`, the lookup is [`bounded`] by it; one abandoned at stop is
/// not recorded.
pub(super) async fn roots(
    shared: &Shared,
    community: &Community,
    reader: &Keys,
    channel: &str,
    bound: Option<Duration>,
) -> Arc<Vec<String>> {
    if let Some(found) = shared.roots.lock().await.get(channel) {
        return found.clone();
    }
    let index = shared.channel_indices.get(channel).copied().unwrap_or(0);
    let author = channel_creator(&shared.scenario, community.index, index);
    let pending = shared.stats.begin(Op::HttpQuery);
    let lookup = find_roots(
        &shared.net,
        community,
        reader,
        &author,
        channel,
        ROOT_PREFIX,
    );
    let result = match bound {
        None => lookup.await,
        Some(limit) => match bounded(shared, limit, lookup).await {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(anyhow::anyhow!("read timeout")),
            None => {
                pending.forget();
                return Arc::new(Vec::new());
            }
        },
    };
    let found = match result {
        Ok(found) => {
            pending.ok();
            found
        }
        Err(e) => {
            pending.err(&e.to_string());
            return Arc::new(Vec::new());
        }
    };
    let found = Arc::new(found);
    shared
        .roots
        .lock()
        .await
        .insert(channel.to_string(), found.clone());
    found
}

/// A human client: live subscriptions, history on connect, presence
/// heartbeats, typing, occasional messages, thread replies and reactions.
async fn human(shared: Arc<Shared>, global: usize) {
    let scenario = &shared.scenario;
    let profile = &scenario.human;
    let community = &shared.communities[global % shared.communities.len()];
    let keys = identity::keys(&scenario.run_id, Role::Human, global);
    let channels = scenario.member_channels(community.index, Role::Human, global);
    let split = scenario.wide_split(Role::Human, global);
    let peers: Vec<PublicKey> = (1..=10)
        .map(|k| {
            let peer = (global + k * shared.communities.len()) % scenario.population.humans.max(1);
            identity::keys(&scenario.run_id, Role::Human, peer).public_key()
        })
        .collect();
    let mut rng = Rng::new(global as u64);
    let mut backoff = Backoff::new();
    let mut last_message: Option<Seen> = None;

    while let Some(mut conn) =
        connect(&shared, community, &keys, None, &mut rng, &mut backoff).await
    {
        let since = Timestamp::now();
        let subscribed = Instant::now();
        let subs = [
            (
                HUMAN_SUBS[0],
                Filter::new()
                    .kinds([
                        kind(KIND_STREAM_MESSAGE),
                        kind(KIND_REACTION),
                        Kind::Custom(5),
                        Kind::Custom(40003),
                    ])
                    .custom_tags(
                        nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                        channels.clone(),
                    )
                    .since(since),
            ),
            (
                HUMAN_SUBS[1],
                human_mentions_filter(keys.public_key(), since),
            ),
            (
                HUMAN_SUBS[2],
                Filter::new().kind(kind(KIND_TYPING_INDICATOR)).custom_tags(
                    nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                    channels.clone(),
                ),
            ),
            (
                HUMAN_SUBS[3],
                Filter::new()
                    .kind(kind(KIND_PRESENCE_UPDATE))
                    .authors(peers.clone()),
            ),
        ];
        let mut ok = true;
        for (id, filter) in subs {
            if let Err(e) = send_req(&mut conn, id, filter).await {
                shared
                    .stats
                    .err(Op::WsConnect, &format!("disconnected: {e}"));
                ok = false;
                break;
            }
        }
        if ok && profile.history_limit > 0 {
            for (n, channel) in channels.iter().enumerate() {
                if stopped(&shared) {
                    break;
                }
                let id = format!("{HISTORY_PREFIX}{n}");
                let filter = Filter::new()
                    .kind(kind(KIND_STREAM_MESSAGE))
                    .custom_tag(
                        nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                        channel.clone(),
                    )
                    .limit(profile.history_limit);
                let fetched = history(
                    &shared.stats,
                    &mut conn,
                    &id,
                    filter,
                    subscribed,
                    &mut last_message,
                )
                .await;
                if let Err(e) = fetched {
                    if let HistoryError::Socket(e) = e {
                        shared
                            .stats
                            .err(Op::WsConnect, &format!("disconnected: {e}"));
                    }
                    ok = false;
                    break;
                }
            }
        }

        let mut activity = Activity::new(profile, &mut rng, split);
        while ok && !stopped(&shared) {
            let wait = activity
                .due()
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(MAX_IDLE)
                .min(MAX_IDLE);
            // Fall through to the schedule after every message so a busy
            // subscription cannot starve this client's own sends.
            match conn.next_event(wait).await {
                Ok(message) => {
                    if live(&shared.stats, &message, subscribed, &mut last_message).is_err() {
                        ok = false;
                        break;
                    }
                }
                Err(WsClientError::Timeout) => {}
                Err(e) => {
                    shared
                        .stats
                        .err(Op::WsConnect, &format!("disconnected: {e}"));
                    ok = false;
                    break;
                }
            }
            let outcome = activity
                .step(
                    &shared,
                    community,
                    &keys,
                    &channels,
                    &mut conn,
                    &mut rng,
                    &mut last_message,
                )
                .await;
            match outcome {
                Ok(Step::Continue | Step::Answered) => {}
                Ok(Step::SessionEnd) => break,
                Err(e) => {
                    shared
                        .stats
                        .err(Op::WsConnect, &format!("disconnected: {e}"));
                    ok = false;
                    break;
                }
            }
        }
        let _ = conn.disconnect().await;
        if stopped(&shared) {
            return;
        }
        // A failed session backs off like a failed connect, so a relay that
        // rejects subscriptions cannot drive a tight reconnect loop.
        if ok {
            backoff = Backoff::new();
        } else {
            backoff.wait(&shared, &mut rng).await;
        }
    }
}

/// What [`Activity::step`] leaves the session to do.
pub(super) enum Step {
    Continue,
    /// The relay answered a publish: a frame arrived.
    Answered,
    /// The session reached its scheduled end; reconnect.
    SessionEnd,
}

/// A human's own traffic on its socket: presence heartbeats, typing,
/// messages (some as replies into seeded threads), reactions and the end of
/// its session, each on its own schedule.
pub(super) struct Activity {
    interval: f64,
    next_presence: Option<Instant>,
    next_typing: Option<Instant>,
    next_message: Option<Instant>,
    next_reaction: Option<Instant>,
    session_end: Option<Instant>,
    /// Set when `population.wide_activity_share` applies to this member.
    split: Option<WideSplit>,
    /// With a split, the latest message seen in a wide (0) and a regular (1)
    /// channel, so reactions follow the split too.
    recent: [Option<Seen>; 2],
    /// The bound of a reply's thread root lookup ([`roots`]); `None` for the
    /// basic human.
    pub(super) root_bound: Option<Duration>,
}

/// Index into `channels` of the channel a member's next typing indicator,
/// message or reaction goes to. Without a split every channel is equally
/// likely; with one, a wide channel gets `share` of the picks unless the
/// member has no regular channels.
fn pick_channel(split: Option<WideSplit>, channels: &[String], rng: &mut Rng) -> usize {
    match split {
        Some(WideSplit { wide, share }) if wide > 0 && wide < channels.len() => {
            if rng.unit() < share {
                rng.below(wide)
            } else {
                wide + rng.below(channels.len() - wide)
            }
        }
        _ => rng.below(channels.len()),
    }
}

impl Activity {
    /// Schedules from now, the first heartbeat at a random point within one
    /// interval so clients connecting together do not beat together.
    pub(super) fn new(profile: &HumanProfile, rng: &mut Rng, split: Option<WideSplit>) -> Self {
        let interval = profile.presence_interval_secs;
        Self {
            interval,
            next_presence: Some(Instant::now() + Duration::from_secs_f64(interval * rng.unit())),
            next_typing: rng.after(profile.typing_per_hour),
            next_message: rng.after(profile.messages_per_hour),
            next_reaction: rng.after(profile.reactions_per_hour),
            session_end: (profile.mean_session_secs > 0.0)
                .then(|| rng.after(3600.0 / profile.mean_session_secs))
                .flatten(),
            split,
            recent: [None, None],
            root_bound: None,
        }
    }

    /// When the next item is due.
    pub(super) fn due(&self) -> Option<Instant> {
        [
            self.next_presence,
            self.next_typing,
            self.next_message,
            self.next_reaction,
            self.session_end,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Runs the first due item, if any. An error means the socket broke.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn step(
        &mut self,
        shared: &Shared,
        community: &Community,
        keys: &Keys,
        channels: &[String],
        conn: &mut NostrWsConnection,
        rng: &mut Rng,
        last_message: &mut Option<Seen>,
    ) -> Result<Step, WsClientError> {
        let profile = &shared.scenario.human;
        if let Some(split) = self.split {
            if let Some(seen) = last_message.take() {
                let wide = channels[..split.wide.min(channels.len())].contains(&seen.channel);
                self.recent[usize::from(!wide)] = Some(seen);
            }
        }
        let now = Instant::now();
        let mut answered = false;
        if self.next_presence.is_some_and(|t| t <= now) {
            self.next_presence = Some(now + Duration::from_secs_f64(self.interval));
            let event =
                EventBuilder::new(kind(KIND_PRESENCE_UPDATE), "online").sign_with_keys(keys);
            answered = send(&shared.stats, conn, Op::PublishEphemeral, event).await?;
        } else if self.next_typing.is_some_and(|t| t <= now) {
            self.next_typing = rng.after(profile.typing_per_hour);
            let channel = &channels[pick_channel(self.split, channels, rng)];
            let event = EventBuilder::new(kind(KIND_TYPING_INDICATOR), "")
                .tags([tag(["h", channel])])
                .sign_with_keys(keys);
            answered = send(&shared.stats, conn, Op::PublishEphemeral, event).await?;
        } else if self.next_message.is_some_and(|t| t <= now) {
            self.next_message = rng.after(profile.messages_per_hour);
            let channel = &channels[pick_channel(self.split, channels, rng)];
            let mut tags = vec![tag(["h", channel])];
            if rng.unit() < profile.thread_reply_fraction {
                let roots = roots(shared, community, keys, channel, self.root_bound).await;
                // A lookup the stop cut short leaves nothing to publish.
                if stopped(shared) {
                    return Ok(Step::Continue);
                }
                if !roots.is_empty() {
                    tags.push(tag(["e", &roots[rng.below(roots.len())], "", "reply"]));
                }
            }
            let me = keys.public_key().to_hex();
            let event = EventBuilder::new(
                kind(KIND_STREAM_MESSAGE),
                format!("{TIMED_PREFIX}{} {me}", now_micros()),
            )
            .tags(tags)
            .sign_with_keys(keys);
            answered = send(&shared.stats, conn, Op::PublishMessage, event).await?;
        } else if self.next_reaction.is_some_and(|t| t <= now) {
            self.next_reaction = rng.after(profile.reactions_per_hour);
            // React to each seen message at most once; the relay rejects duplicates.
            let target = match self.split {
                None => last_message.take(),
                Some(split) => {
                    let i = pick_channel(self.split, channels, rng);
                    self.recent[usize::from(i >= split.wide)].take()
                }
            };
            if let Some(target) = target {
                let event = EventBuilder::new(kind(KIND_REACTION), "+")
                    .tags([tag(["h", &target.channel]), tag(["e", &target.id])])
                    .sign_with_keys(keys);
                answered = send(&shared.stats, conn, Op::PublishMessage, event).await?;
            }
        } else if self.session_end.is_some_and(|t| t <= now) {
            return Ok(Step::SessionEnd);
        }
        Ok(if answered {
            Step::Answered
        } else {
            Step::Continue
        })
    }
}

pub(super) async fn send(
    stats: &Stats,
    conn: &mut NostrWsConnection,
    op: Op,
    event: Result<Event, nostr::event::builder::Error>,
) -> Result<bool, WsClientError> {
    match event {
        Ok(event) => publish(stats, conn, op, event).await,
        Err(e) => {
            stats.err(op, &format!("sign: {e}"));
            Ok(false)
        }
    }
}

async fn send_req(
    conn: &mut NostrWsConnection,
    id: &str,
    filter: Filter,
) -> Result<(), WsClientError> {
    conn.send_raw(&json!(["REQ", id, filter])).await
}

enum HistoryError {
    /// The socket failed.
    Socket(WsClientError),
    /// The relay closed a live subscription (already recorded).
    LiveClosed,
}

impl From<WsClientError> for HistoryError {
    fn from(e: WsClientError) -> Self {
        Self::Socket(e)
    }
}

/// Fetches history until EOSE, records the latency and closes the
/// subscription. Messages for the live subscriptions opened at `subscribed`
/// go through [`live`].
async fn history(
    stats: &Stats,
    conn: &mut NostrWsConnection,
    id: &str,
    filter: Filter,
    subscribed: Instant,
    last_message: &mut Option<Seen>,
) -> Result<(), HistoryError> {
    let pending = stats.begin(Op::History);
    if let Err(e) = send_req(conn, id, filter).await {
        pending.forget();
        return Err(e.into());
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match conn.next_event(remaining).await {
            Ok(RelayMessage::Eose { subscription_id }) if subscription_id == id => {
                pending.ok();
                break;
            }
            Ok(RelayMessage::Closed {
                subscription_id,
                message,
            }) if subscription_id == id => {
                pending.err(&message);
                return Ok(());
            }
            Ok(message) if subscription_of(&message) == Some(id) => {
                if let Some(target) = seen(&message) {
                    *last_message = Some(target);
                }
            }
            Ok(message) => {
                if live(stats, &message, subscribed, last_message).is_err() {
                    pending.forget();
                    return Err(HistoryError::LiveClosed);
                }
            }
            Err(WsClientError::Timeout) => {
                pending.err("EOSE timeout");
                break;
            }
            Err(e) => {
                pending.forget();
                return Err(e.into());
            }
        }
    }
    Ok(conn.send_raw(&json!(["CLOSE", id])).await?)
}

/// Global `#p` events addressed to `me`. Real clients name kinds here; a
/// kindless global filter would be matched against every global event.
fn mentions_filter(me: PublicKey, since: Timestamp) -> Filter {
    Filter::new()
        .kinds([
            kind(KIND_MEMBER_ADDED_NOTIFICATION),
            kind(KIND_MEMBER_REMOVED_NOTIFICATION),
        ])
        .pubkey(me)
        .since(since)
}

/// Humans own the simulated agents, so their mentions also carry the
/// owner-addressed observer frames, as desktop subscribes to them.
fn human_mentions_filter(me: PublicKey, since: Timestamp) -> Filter {
    mentions_filter(me, since).kind(kind(KIND_AGENT_OBSERVER_FRAME))
}

fn subscription_of(message: &RelayMessage) -> Option<&str> {
    match message {
        RelayMessage::Event {
            subscription_id, ..
        }
        | RelayMessage::Eose { subscription_id }
        | RelayMessage::Closed {
            subscription_id, ..
        } => Some(subscription_id),
        _ => None,
    }
}

/// Kinds an agent harness subscribes to for its mentions in each channel.
const AGENT_MENTION_KINDS: [u32; 4] = [
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_EDIT,
    KIND_WORKFLOW_APPROVAL_REQUESTED,
    KIND_STREAM_REMINDER,
];
/// Kinds a turn reads as channel context and tool reads.
const AGENT_READ_KINDS: [u32; 2] = [KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2];
/// Reactions a turn adds to the event it answers and removes at its end:
/// "seen" and "working".
const TURN_REACTIONS: [&str; 2] = ["👀", "💬"];
/// Replies a harness puts in a thread's context, as its default context
/// message limit. It fetches one more, and counts the thread when it gets it.
const THREAD_CONTEXT_REPLIES: usize = 12;

/// An agent's turn under way: the channel it works in, the event it
/// answers, and when its next typing indicator, observer frame and tool
/// reads are due.
struct Turn {
    channel: String,
    target: Option<Event>,
    ends: Instant,
    next_typing: Option<Instant>,
    next_frame: Option<Instant>,
    /// Due times of the tool reads still to make, latest first.
    reads: Vec<Instant>,
}

/// What a turn does next.
enum TurnStep {
    Typing(String),
    Frame,
    Read(String),
    End,
}

impl Turn {
    /// When the turn next has something to do. Without `http`, only its
    /// socket steps count: typing and frames.
    fn due(&self, http: bool) -> Option<Instant> {
        let http_due = http.then(|| self.reads.last().copied().unwrap_or(self.ends));
        [self.next_typing, self.next_frame, http_due]
            .into_iter()
            .flatten()
            .min()
    }

    /// The step due at `now`, if any, with its next one scheduled. Typing,
    /// frames and reads run before the end, so none is lost to a late wakeup.
    /// Without `http`, reads and the end wait.
    fn step(&mut self, now: Instant, profile: &AgentTurn, http: bool) -> Option<TurnStep> {
        let due = |t: Option<Instant>| t.is_some_and(|t| t <= now);
        if due(self.next_typing) {
            self.next_typing = every(profile.typing_secs, now).filter(|t| *t < self.ends);
            Some(TurnStep::Typing(self.channel.clone()))
        } else if due(self.next_frame) {
            self.next_frame = every(profile.frame_secs, now).filter(|t| *t < self.ends);
            Some(TurnStep::Frame)
        } else if !http {
            None
        } else if due(self.reads.last().copied()) {
            self.reads.pop();
            Some(TurnStep::Read(self.channel.clone()))
        } else {
            (self.ends <= now).then_some(TurnStep::End)
        }
    }
}

/// A turn's random draws, made before its requests start.
struct TurnPlan {
    channel: String,
    length: Duration,
    /// Each tool read's offset into the turn, as a fraction of its length.
    reads: Vec<f64>,
}

impl TurnPlan {
    fn draw(channels: &[String], profile: &AgentTurn, rng: &mut Rng) -> Self {
        let channel = channels[rng.below(channels.len())].clone();
        let length = Duration::from_secs_f64(profile.mean_secs * (0.5 + rng.unit()));
        let reads = (0..profile.reads).map(|_| rng.unit()).collect();
        Self {
            channel,
            length,
            reads,
        }
    }
}

/// How an agent's HTTP action finished.
enum Outcome {
    Started(Box<Turn>),
    Ended,
    Done,
}

/// An agent's HTTP action in flight. One runs at a time, beside the socket.
type Busy<'a> = Option<Pin<Box<dyn Future<Output = Outcome> + Send + 'a>>>;

/// The in-flight action's outcome; never resolves when there is none.
async fn settle(busy: &mut Busy<'_>) -> Outcome {
    match busy {
        Some(action) => action.await,
        None => std::future::pending().await,
    }
}

/// Applies a finished HTTP action to the agent's turn state.
fn land(
    outcome: Outcome,
    turn: &mut Option<Turn>,
    next_turn: &mut Option<Instant>,
    profile: &AgentTurn,
    rng: &mut Rng,
) {
    match outcome {
        Outcome::Started(started) => *turn = Some(*started),
        Outcome::Ended => *next_turn = rng.after(profile.per_hour),
        Outcome::Done => {}
    }
}

/// `Some(now + secs)`, or `None` for an interval of 0 (off).
fn every(secs: f64, from: Instant) -> Option<Instant> {
    (secs > 0.0).then(|| from + Duration::from_secs_f64(secs))
}

/// The most socket publishes an agent leaves awaiting their `OK`; past it, a
/// publish is an error instead of sent. A relay that answers within the
/// publish timeout keeps an agent far below it.
const MAX_UNACKED: usize = 128;

/// An agent's socket publishes awaiting their `OK`, oldest first. An agent
/// sends a frame and moves on, as a harness does, so a slow `OK` holds up
/// neither its HTTP action nor its next frame. Each `OK` is matched as it
/// arrives. A publish gets `timeout` from the start of its send: one not
/// written by then fails and drops the socket, and one not answered by then
/// is an error.
struct Unacked<'a> {
    stats: &'a Stats,
    timeout: Duration,
    /// Event id, the publish's timing and its deadline. Identical frames sent
    /// within a second share an id; each `OK` settles the oldest.
    waiting: VecDeque<(String, Pending<'a>, Instant)>,
}

impl<'a> Unacked<'a> {
    fn new(stats: &'a Stats, timeout: Duration) -> Self {
        Self {
            stats,
            timeout,
            waiting: VecDeque::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    /// Sends `event` as `op` without waiting for its `OK`.
    async fn send(
        &mut self,
        conn: &mut NostrWsConnection,
        op: Op,
        event: Result<Event, String>,
    ) -> Result<(), WsClientError> {
        let event = match event {
            Ok(event) => event,
            Err(e) => {
                self.stats.err(op, &e);
                return Ok(());
            }
        };
        if self.waiting.len() >= MAX_UNACKED {
            self.stats.err(op, "too many publishes awaiting OK");
            return Ok(());
        }
        let pending = self.stats.begin(op);
        let deadline = Instant::now() + self.timeout;
        let frame = json!(["EVENT", event]);
        match tokio::time::timeout(self.timeout, conn.send_raw(&frame)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                // The caller records the broken socket.
                pending.forget();
                return Err(e);
            }
            Err(_) => {
                // The relay stopped reading; the caller drops the socket.
                pending.err("send timeout");
                return Err(WsClientError::Timeout);
            }
        }
        self.waiting
            .push_back((event.id.to_hex(), pending, deadline));
        Ok(())
    }

    /// Settles the oldest publish `ok` answers, if any.
    fn answer(&mut self, ok: &OkResponse) {
        let Some(at) = self.waiting.iter().position(|(id, ..)| *id == ok.event_id) else {
            return;
        };
        if let Some((_, pending, _)) = self.waiting.remove(at) {
            if ok.accepted {
                pending.ok();
            } else {
                pending.err(&ok.message);
            }
        }
    }

    /// When the oldest publish times out.
    fn deadline(&self) -> Option<Instant> {
        self.waiting.front().map(|(.., deadline)| *deadline)
    }

    /// Fails each publish whose `OK` is overdue at `now`.
    fn expire(&mut self, now: Instant) {
        while let Some((.., deadline)) = self.waiting.front() {
            if *deadline > now {
                break;
            }
            if let Some((_, pending, _)) = self.waiting.pop_front() {
                pending.err("OK timeout");
            }
        }
    }

    /// Fails every publish still awaiting its `OK` with `reason`.
    fn fail(&mut self, reason: &str) {
        for (_, pending, _) in self.waiting.drain(..) {
            pending.err(reason);
        }
    }

    /// Leaves every publish unjudged once its socket is gone, which is
    /// recorded as such.
    fn forget(&mut self) {
        for (_, pending, _) in self.waiting.drain(..) {
            pending.forget();
        }
    }
}

/// An agent client: NIP-OA authenticated socket with its mentions, then
/// either HTTP polling, posts and observer frames on Poisson schedules, or
/// turns of work (see [`AgentTurn`]), or both. One HTTP action at a time
/// runs beside the socket, and socket publishes do not wait for their `OK`
/// (see [`Unacked`]), so presence, typing, frames and requests keep their
/// schedule while a request or an `OK` is slow, as a harness's do.
async fn agent(shared: Arc<Shared>, global: usize) {
    let scenario = &shared.scenario;
    let profile = &scenario.agent;
    let community = &shared.communities[global % shared.communities.len()];
    let keys = identity::keys(&scenario.run_id, Role::Agent, global);
    let owner = identity::keys(
        &scenario.run_id,
        Role::Human,
        global % scenario.population.humans.max(1),
    );
    let auth = identity::nip_oa_tag(&owner, &keys);
    let channels = scenario.member_channels(community.index, Role::Agent, global);
    let me = keys.public_key().to_hex();
    // What HTTP actions borrow; each owns everything else it needs.
    let (sh, k, a): (&Shared, &Keys, &Tag) = (&shared, &keys, &auth);
    let mut rng = Rng::new(u64::MAX - global as u64);
    let mut backoff = Backoff::new();
    let mut last_message = None;
    let query_per_hour = if profile.query_interval_secs > 0.0 {
        3600.0 / profile.query_interval_secs
    } else {
        0.0
    };
    let mut next_query = rng.after(query_per_hour);
    let mut next_post = rng.after(profile.events_per_hour);
    let mut next_frame = rng.after(profile.observer_frames_per_hour);
    let mut next_turn = rng.after(profile.turn.per_hour);
    // A turn outlives a dropped socket, as a harness keeps working while it
    // reconnects.
    let mut turn: Option<Turn> = None;
    let mut busy: Busy<'_> = None;

    while let Some(mut conn) = connect(
        &shared,
        community,
        &keys,
        Some(&auth),
        &mut rng,
        &mut backoff,
    )
    .await
    {
        let since = Timestamp::now();
        let subscribed = Instant::now();
        let mut reqs = vec![(
            "mentions".to_string(),
            mentions_filter(keys.public_key(), since),
        )];
        if profile.channel_subscriptions {
            reqs.extend(channels.iter().enumerate().map(|(i, channel)| {
                let filter = Filter::new()
                    .kinds(AGENT_MENTION_KINDS.map(kind))
                    .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()])
                    .pubkey(keys.public_key())
                    .since(since);
                (format!("ch-{i}"), filter)
            }));
        }
        let mut ok = true;
        for (id, filter) in reqs {
            if let Err(e) = send_req(&mut conn, &id, filter).await {
                shared
                    .stats
                    .err(Op::WsConnect, &format!("disconnected: {e}"));
                ok = false;
                break;
            }
        }
        // Presence starts at a random point of its interval, so a fleet that
        // connects together does not heartbeat together.
        let mut next_presence = every(profile.presence_interval_secs * rng.unit(), Instant::now())
            .or_else(|| every(profile.presence_interval_secs, Instant::now()));
        let mut unacked = Unacked::new(&shared.stats, Duration::from_secs(PUBLISH_OK_TIMEOUT_SECS));

        while ok && !stopped(&shared) {
            // While an HTTP action runs, only socket work is waited for.
            let idle = busy.is_none();
            let if_idle = |t: Option<Instant>| t.filter(|_| idle);
            let due = [
                next_presence,
                next_frame,
                turn.as_ref().map_or(if_idle(next_turn), |t| t.due(idle)),
                if_idle(next_query),
                if_idle(next_post),
                unacked.deadline(),
            ]
            .into_iter()
            .flatten()
            .min();
            let wait = due
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(MAX_IDLE)
                .min(MAX_IDLE);
            let received = tokio::select! {
                received = conn.next_event(wait) => Some(received),
                outcome = settle(&mut busy), if busy.is_some() => {
                    busy = None;
                    land(outcome, &mut turn, &mut next_turn, &profile.turn, &mut rng);
                    None
                }
            };
            match received {
                Some(Ok(RelayMessage::Ok(answer))) => unacked.answer(&answer),
                Some(Ok(message)) => {
                    if live(&shared.stats, &message, subscribed, &mut last_message).is_err() {
                        ok = false;
                        break;
                    }
                }
                Some(Err(WsClientError::Timeout)) | None => {}
                Some(Err(e)) => {
                    shared
                        .stats
                        .err(Op::WsConnect, &format!("disconnected: {e}"));
                    ok = false;
                    break;
                }
            }
            let now = Instant::now();
            unacked.expire(now);
            let idle = busy.is_none();
            // One action per wakeup; anything else due runs on the next pass.
            // An HTTP action only starts with none in flight.
            let sent = if next_presence.is_some_and(|t| t <= now) {
                next_presence = every(profile.presence_interval_secs, now);
                let event =
                    EventBuilder::new(kind(KIND_PRESENCE_UPDATE), "online").sign_with_keys(&keys);
                let event = event.map_err(|e| format!("sign: {e}"));
                unacked.send(&mut conn, Op::PublishEphemeral, event).await
            } else if let Some(step) = turn.as_mut().and_then(|t| t.step(now, &profile.turn, idle))
            {
                match step {
                    TurnStep::Typing(channel) => {
                        let event = EventBuilder::new(kind(KIND_TYPING_INDICATOR), "")
                            .tags([tag(["h", &channel])])
                            .sign_with_keys(&keys)
                            .map_err(|e| format!("sign: {e}"));
                        unacked.send(&mut conn, Op::PublishEphemeral, event).await
                    }
                    TurnStep::Frame => {
                        let frame = observer_frame(&keys, &owner, profile.turn.frame_bytes);
                        let frame = frame.map_err(|e| format!("observer: {e}"));
                        unacked.send(&mut conn, Op::PublishEphemeral, frame).await
                    }
                    TurnStep::Read(channel) => {
                        let filters = json!([{
                            "kinds": AGENT_READ_KINDS,
                            "#h": [channel],
                            "limit": 50,
                        }]);
                        busy = Some(Box::pin(async move {
                            query(sh, Op::HttpQuery, k, community, &filters, a).await;
                            Outcome::Done
                        }));
                        Ok(())
                    }
                    TurnStep::End => {
                        if let Some(ended) = turn.take() {
                            busy = Some(Box::pin(async move {
                                end_turn(sh, community, k, a, ended).await;
                                Outcome::Ended
                            }));
                        }
                        Ok(())
                    }
                }
            } else if idle && turn.is_none() && next_turn.is_some_and(|t| t <= now) {
                next_turn = None;
                let plan = TurnPlan::draw(&channels, &profile.turn, &mut rng);
                busy = Some(Box::pin(async move {
                    Outcome::Started(Box::new(start_turn(sh, community, k, a, plan).await))
                }));
                Ok(())
            } else if idle && next_query.is_some_and(|t| t <= now) {
                next_query = rng.after(query_per_hour);
                let channel = &channels[rng.below(channels.len())];
                let window = Timestamp::now().as_secs().saturating_sub(3600);
                let filters = json!([
                    { "kinds": [KIND_STREAM_MESSAGE], "#h": [channel], "since": window, "limit": 20 },
                    { "#p": [me], "since": window, "limit": 20 },
                ]);
                busy = Some(Box::pin(async move {
                    query(sh, Op::HttpQuery, k, community, &filters, a).await;
                    Outcome::Done
                }));
                Ok(())
            } else if idle && next_post.is_some_and(|t| t <= now) {
                next_post = rng.after(profile.events_per_hour);
                let channel = &channels[rng.below(channels.len())];
                let event = EventBuilder::new(
                    kind(KIND_STREAM_MESSAGE),
                    format!("{TIMED_PREFIX}{} {me}", now_micros()),
                )
                .tags([tag(["h", channel])]);
                busy = Some(Box::pin(async move {
                    submit(sh, community, k, a, event, Op::HttpEvents).await;
                    Outcome::Done
                }));
                Ok(())
            } else if next_frame.is_some_and(|t| t <= now) {
                next_frame = rng.after(profile.observer_frames_per_hour);
                let frame = observer_frame(&keys, &owner, 0);
                let frame = frame.map_err(|e| format!("observer: {e}"));
                unacked.send(&mut conn, Op::PublishEphemeral, frame).await
            } else {
                Ok(())
            };
            if let Err(e) = sent {
                shared
                    .stats
                    .err(Op::WsConnect, &format!("disconnected: {e}"));
                ok = false;
            }
        }
        // At the stop, publishes still awaiting their `OK` settle within
        // their deadlines while the HTTP action finishes, as a human's
        // publish in flight does. One never answered outlasts the shutdown
        // grace and fails the run, and a socket lost before they all settle
        // fails the rest.
        while ok && stopped(&shared) && !unacked.is_empty() {
            let wait = unacked
                .deadline()
                .map_or(MAX_IDLE, |d| d.saturating_duration_since(Instant::now()))
                .min(MAX_IDLE);
            let received = tokio::select! {
                received = conn.next_event(wait) => Some(received),
                outcome = settle(&mut busy), if busy.is_some() => {
                    busy = None;
                    land(outcome, &mut turn, &mut next_turn, &profile.turn, &mut rng);
                    None
                }
            };
            match received {
                Some(Ok(RelayMessage::Ok(answer))) => unacked.answer(&answer),
                Some(Ok(_)) | Some(Err(WsClientError::Timeout)) | None => {}
                Some(Err(e)) => {
                    unacked.fail(&format!("disconnected before OK: {e}"));
                    break;
                }
            }
            unacked.expire(Instant::now());
        }
        unacked.forget();
        let _ = conn.disconnect().await;
        // The action in flight finishes before the agent reconnects or
        // stops; each checks the stop between its requests.
        if let Some(action) = busy.take() {
            land(
                action.await,
                &mut turn,
                &mut next_turn,
                &profile.turn,
                &mut rng,
            );
        }
        if stopped(&shared) {
            return;
        }
        if ok {
            backoff = Backoff::new();
        } else {
            backoff.wait(&shared, &mut rng).await;
        }
    }
}

/// Starts a turn the way an agent harness starts a prompt. The loadgen first
/// finds the event to answer, the channel's newest message, where a harness
/// is handed it by its subscription. Then, as `turn_context`: a threaded
/// target's thread context (see [`thread_context`]), the channel's metadata,
/// its project home (two reads) and the target author's profile, and then
/// two reactions on the target.
async fn start_turn(
    shared: &Shared,
    community: &Community,
    keys: &Keys,
    auth: &Tag,
    plan: TurnPlan,
) -> Turn {
    let profile = &shared.scenario.agent.turn;
    let channel = plan.channel;
    let newest = json!([{ "kinds": AGENT_READ_KINDS, "#h": [channel], "limit": 1 }]);
    let target: Option<Event> = query(shared, Op::HttpQuery, keys, community, &newest, auth)
        .await
        .and_then(|events| events.into_iter().max_by_key(|e| e.created_at));
    let root = target
        .as_ref()
        .and_then(|t| buzz_core::nip10::parse_thread_markers(&t.tags).resolve())
        .map(|(root, _)| root);
    if let Some(root) = root {
        thread_context(shared, community, keys, auth, &channel, &root).await;
    }
    let author = target.as_ref().map(|e| e.pubkey.to_hex());
    let reads = [
        json!([{ "kinds": [KIND_NIP29_GROUP_METADATA], "#d": [channel], "limit": 1 }]),
        json!([{ "kinds": [KIND_PROJECT], "#buzz-channel": [channel] }]),
        json!([{ "kinds": [KIND_GIT_REPO_ANNOUNCEMENT], "#buzz-channel": [channel] }]),
        json!([{ "kinds": [KIND_PROFILE], "authors": [author.unwrap_or_else(|| keys.public_key().to_hex())] }]),
    ];
    for filters in reads {
        query(shared, Op::TurnContext, keys, community, &filters, auth).await;
    }
    if let Some(target) = &target {
        for emoji in TURN_REACTIONS {
            let event = EventBuilder::new(kind(KIND_REACTION), emoji)
                .tags([tag(["h", &channel]), tag(["e", &target.id.to_hex()])]);
            submit(shared, community, keys, auth, event, Op::HttpEvents).await;
        }
    }
    let now = Instant::now();
    let mut reads: Vec<Instant> = plan
        .reads
        .iter()
        .map(|at| now + plan.length.mul_f64(*at))
        .collect();
    reads.sort_unstable_by(|a, b| b.cmp(a));
    Turn {
        channel,
        target,
        ends: now + plan.length,
        next_typing: every(profile.typing_secs, now).map(|_| now),
        next_frame: every(profile.frame_secs, now).map(|_| now),
        reads,
    }
}

/// A harness's thread context read, as `turn_context`: one `/query` for the
/// root, the thread's newest replies (one past the context limit) and the
/// agent's own newest reply, then a `/count` of the replies when the window
/// overflowed.
async fn thread_context(
    shared: &Shared,
    community: &Community,
    keys: &Keys,
    auth: &Tag,
    channel: &str,
    root: &str,
) {
    let replies = json!({
        "kinds": AGENT_READ_KINDS,
        "#e": [root],
        "#h": [channel],
        "limit": THREAD_CONTEXT_REPLIES + 1,
    });
    let mut mine = replies.clone();
    mine["authors"] = json!([keys.public_key().to_hex()]);
    mine["limit"] = json!(1);
    let filters = json!([{ "ids": [root] }, replies, mine]);
    let Some(events) = query(shared, Op::TurnContext, keys, community, &filters, auth).await else {
        return;
    };
    let fetched: HashSet<_> = events
        .iter()
        .map(|e| e.id.to_hex())
        .filter(|id| id != root)
        .collect();
    if fetched.len() > THREAD_CONTEXT_REPLIES && !stopped(shared) {
        let mut count = replies;
        count["limit"] = json!(0);
        http::<CountReply>(
            shared,
            Op::TurnContext,
            keys,
            community,
            "/count",
            &json!([count]),
            Some(auth),
        )
        .await;
    }
}

/// A `/count` response.
#[derive(serde::Deserialize)]
struct CountReply {
    #[serde(rename = "count")]
    _count: u64,
}

/// Ends a turn: a timed reply in the answered message's thread, then each
/// reaction's removal, which reads the agent's reactions on the target and
/// deletes them, as the harness does.
async fn end_turn(shared: &Shared, community: &Community, keys: &Keys, auth: &Tag, turn: Turn) {
    let me = keys.public_key().to_hex();
    let mut tags = vec![tag(["h", &turn.channel])];
    if let Some(target) = &turn.target {
        // As a harness answers a person: a threaded target is answered at
        // its thread's first level, under the root, and a top-level one
        // becomes the root of a new thread. Replies never nest deeper, so
        // agents answering agents can't climb to the relay's depth limit.
        let root = buzz_core::nip10::parse_thread_markers(&target.tags)
            .resolve()
            .map_or_else(|| target.id.to_hex(), |(root, _)| root);
        tags.extend(reply_tags(&root, &root));
    }
    let reply = EventBuilder::new(
        kind(KIND_STREAM_MESSAGE),
        format!("{TIMED_PREFIX}{} {me}", now_micros()),
    )
    .tags(tags);
    submit(shared, community, keys, auth, reply, Op::HttpEvents).await;
    let Some(target) = turn.target else {
        return;
    };
    let mine = json!([{
        "kinds": [KIND_REACTION],
        "authors": [me],
        "#e": [target.id.to_hex()],
    }]);
    for _ in TURN_REACTIONS {
        let found = query(shared, Op::HttpQuery, keys, community, &mine, auth)
            .await
            .unwrap_or_default();
        // The first pass deletes every reaction found, so the second only
        // reads, as the harness's second removal finds nothing left.
        for reaction in found {
            let event = EventBuilder::new(kind(KIND_DELETION), "")
                .tags([tag(["e", &reaction.id.to_hex()])]);
            submit(shared, community, keys, auth, event, Op::HttpEvents).await;
        }
    }
}

/// An encrypted kind:24200 telemetry frame to `owner`, its plaintext padded
/// by `pad` bytes.
fn observer_frame(keys: &Keys, owner: &Keys, pad: usize) -> Result<Event, String> {
    let payload = json!({
        "type": "loadgen",
        "sent_micros": now_micros().to_string(),
        "pad": "x".repeat(pad),
    });
    let content =
        encrypt_observer_payload(keys, &owner.public_key(), &payload).map_err(|e| e.to_string())?;
    EventBuilder::new(kind(KIND_AGENT_OBSERVER_FRAME), content)
        .tags([
            tag(["p", &owner.public_key().to_hex()]),
            tag([OBSERVER_AGENT_TAG, &keys.public_key().to_hex()]),
            tag([OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]),
        ])
        .sign_with_keys(keys)
        .map_err(|e| e.to_string())
}

/// Signs and submits `event` through HTTP `/events` as `op`; as with
/// [`query`], none starts after the stop. A reaction the relay already holds
/// counts as answered: an earlier turn cut short by the stop left it, and
/// the next removal deletes it.
async fn submit(
    shared: &Shared,
    community: &Community,
    keys: &Keys,
    auth: &Tag,
    event: EventBuilder,
    op: Op,
) {
    if stopped(shared) {
        return;
    }
    let event = match event.sign_with_keys(keys) {
        Ok(event) => event,
        Err(e) => return shared.stats.err(op, &format!("sign: {e}")),
    };
    let pending = shared.stats.begin(op);
    match shared.net.submit(keys, community, &event, Some(auth)).await {
        Ok(()) => pending.ok(),
        Err(e) if e.to_string().contains("reaction already exists") => pending.ok(),
        Err(e) => pending.err(&e.to_string()),
    }
}

/// A hot-thread author: replies to the hot root through HTTP `/events` and
/// reads the thread's newest replies through HTTP `/query`. Replies carry
/// their send time, so humans in the hot channel record their delivery.
async fn hot_author(shared: Arc<Shared>, index: usize) {
    let scenario = &shared.scenario;
    let profile = &scenario.hot_thread;
    let community = &shared.communities[0];
    let keys = identity::keys(&scenario.run_id, Role::HotAuthor, index);
    let root_author = hot_root_keys(&scenario.run_id);
    let channel = identity::channel_id(&scenario.run_id, 0, 0);
    let me = keys.public_key().to_hex();
    let mut rng = Rng::new(u64::MAX / 2 - index as u64);
    let mut backoff = Backoff::new();
    // A missing root is an error every attempt, so a run without the seeded
    // hot thread fails its steps instead of measuring nothing.
    let root = loop {
        if stopped(&shared) {
            return;
        }
        let pending = shared.stats.begin(Op::ThreadFetch);
        let newest = find_roots(
            &shared.net,
            community,
            &keys,
            &root_author,
            &channel,
            HOT_ROOT_PREFIX,
        )
        .await
        .map(|roots| roots.last().cloned());
        match newest {
            Ok(Some(root)) => {
                pending.ok();
                break root;
            }
            Ok(None) => pending.err("hot thread root not seeded"),
            Err(e) => pending.err(&e.to_string()),
        }
        backoff.wait(&shared, &mut rng).await;
    };
    let read = json!([{
        "kinds": [KIND_STREAM_MESSAGE],
        "#h": [channel],
        "#e": [root],
        "limit": 100,
    }]);
    let mut next_reply = rng.after(profile.replies_per_hour);
    let mut next_fetch = rng.after(profile.fetches_per_hour);
    loop {
        let Some(due) = [next_reply, next_fetch].into_iter().flatten().min() else {
            return stopping(&shared).await;
        };
        tokio::select! {
            _ = tokio::time::sleep_until(due.into()) => {}
            _ = stopping(&shared) => return,
        }
        // An overdue arrival and the stop can be ready together, and `select!`
        // picks either; start nothing new once the run is stopping.
        if stopped(&shared) {
            return;
        }
        if next_reply == Some(due) {
            next_reply = rng.after(profile.replies_per_hour);
            let event = EventBuilder::new(
                kind(KIND_STREAM_MESSAGE),
                format!("{TIMED_PREFIX}{} {me}", now_micros()),
            )
            .tags(
                [tag(["h", &channel])]
                    .into_iter()
                    .chain(reply_tags(&root, &root)),
            )
            .sign_with_keys(&keys);
            let pending = shared.stats.begin(Op::HotReply);
            match event {
                Ok(event) => match shared.net.submit(&keys, community, &event, None).await {
                    Ok(()) => pending.ok(),
                    Err(e) => pending.err(&e.to_string()),
                },
                Err(e) => pending.err(&format!("sign: {e}")),
            }
        } else {
            next_fetch = rng.after(profile.fetches_per_hour);
            http::<Vec<Event>>(
                &shared,
                Op::ThreadFetch,
                &keys,
                community,
                "/query",
                &read,
                None,
            )
            .await;
        }
    }
}

/// An HTTP POST of `body` to `path` as `op`; its response decoded as `T`.
/// The op succeeds only when the relay answered 2xx with a body that
/// decodes, so a malformed answer is an error rather than an empty result.
async fn http<T: serde::de::DeserializeOwned>(
    shared: &Shared,
    op: Op,
    keys: &Keys,
    community: &Community,
    path: &str,
    body: &Value,
    auth: Option<&Tag>,
) -> Option<T> {
    let pending = shared.stats.begin(op);
    let body = serde_json::to_vec(body).expect("serialize JSON body");
    match shared
        .net
        .post(keys, &community.http_url, path, body, auth)
        .await
    {
        Ok((status, text)) if status.is_success() => match serde_json::from_str(&text) {
            Ok(decoded) => {
                pending.ok();
                Some(decoded)
            }
            Err(e) => {
                pending.err(&format!("decode {path}: {e}"));
                None
            }
        },
        Ok((status, text)) => {
            pending.err(&http_reason(status, &text));
            None
        }
        Err(e) => {
            pending.err(&e.to_string());
            None
        }
    }
}

/// An agent's HTTP `/query` as `op`; the events it returned. None starts
/// after the stop, so an action under way at the stop sends nothing more
/// than the request already in flight.
async fn query(
    shared: &Shared,
    op: Op,
    keys: &Keys,
    community: &Community,
    filters: &Value,
    auth: &Tag,
) -> Option<Vec<Event>> {
    if stopped(shared) {
        return None;
    }
    http(shared, op, keys, community, "/query", filters, Some(auth)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn shards_partition_clients() {
        let shards: Vec<Shard> = (0..3)
            .map(|i| Shard::parse(&format!("{i}/3")).unwrap())
            .collect();
        for g in 0..30 {
            assert_eq!(shards.iter().filter(|s| s.owns(g)).count(), 1);
        }
        assert!(Shard::parse("3/3").is_err());
        assert!(Shard::parse("x").is_err());
    }

    #[test]
    fn poisson_schedule_matches_rate() {
        let mut rng = Rng::new(1);
        let start = Instant::now();
        let n = 20_000;
        let total: f64 = (0..n)
            .map(|_| {
                rng.after(3600.0)
                    .unwrap()
                    .duration_since(start)
                    .as_secs_f64()
            })
            .sum();
        let mean = total / n as f64;
        assert!((mean - 1.0).abs() < 0.05, "mean interval {mean}s for 1/s");
        assert!(rng.after(0.0).is_none());
    }

    #[test]
    fn timed_messages_record_delivery() {
        let stats = Stats::default();
        let keys = Keys::generate();
        let event = EventBuilder::new(
            kind(KIND_STREAM_MESSAGE),
            format!("{TIMED_PREFIX}{} x", now_micros()),
        )
        .tags([tag(["h", "c"])])
        .sign_with_keys(&keys)
        .unwrap();
        let target = observe(
            &stats,
            &RelayMessage::Event {
                subscription_id: "live".into(),
                event: Box::new(event.clone()),
            },
        )
        .expect("stream message with an h tag is a reaction target");
        assert_eq!(
            (target.id, target.channel),
            (event.id.to_hex(), "c".to_string())
        );
        let summary = stats.snapshot().since(&Default::default());
        assert_eq!(
            summary
                .iter()
                .find(|(op, _)| *op == Op::Deliver)
                .unwrap()
                .1
                .count,
            1
        );
    }
    #[test]
    fn human_mentions_receive_owner_observer_frames() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let frame_to = |to: &Keys| {
            EventBuilder::new(kind(KIND_AGENT_OBSERVER_FRAME), "x")
                .tags([tag(["p", &to.public_key().to_hex()])])
                .sign_with_keys(&agent)
                .unwrap()
        };
        let opts = nostr::filter::MatchEventOptions::default();
        let since = Timestamp::from(0);
        let human = human_mentions_filter(owner.public_key(), since);
        assert!(human.match_event(&frame_to(&owner), opts));
        assert!(!human.match_event(&frame_to(&agent), opts));
        let agent_filter = mentions_filter(owner.public_key(), since);
        assert!(!agent_filter.match_event(&frame_to(&owner), opts));
    }

    #[test]
    fn closed_live_subscriptions_end_the_session_and_count() {
        let stats = Stats::default();
        let mut last = None;
        let subscribed = Instant::now();
        let eose = RelayMessage::Eose {
            subscription_id: "live".into(),
        };
        assert!(live(&stats, &eose, subscribed, &mut last).is_ok());
        // The relay acknowledges our own CLOSE of a history subscription.
        let closed_history = RelayMessage::Closed {
            subscription_id: format!("{HISTORY_PREFIX}0"),
            message: String::new(),
        };
        assert!(live(&stats, &closed_history, subscribed, &mut last).is_ok());
        let closed = RelayMessage::Closed {
            subscription_id: "live".into(),
            message: "error: overloaded".into(),
        };
        assert!(live(&stats, &closed, subscribed, &mut last).is_err());
        let summary = stats.snapshot().since(&Default::default());
        let sub = summary
            .iter()
            .find(|(op, _)| *op == Op::Subscribe)
            .unwrap()
            .1;
        assert_eq!((sub.count, sub.errors), (1, 1));
        let reason = abort_reason(&summary, 0.05, 5_000.0).expect("rejected subscriptions abort");
        assert!(reason.starts_with("subscribe_eose"), "{reason}");
    }

    #[tokio::test]
    async fn stalled_connects_count_and_the_run_still_returns() {
        // A peer that accepts TCP but never answers the WebSocket upgrade.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let mut open = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                open.push(socket);
            }
        });
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
connect_timeout_secs = 0.2
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 3
agents = 1
channels_per_member = 1
connect_rate_per_sec = 100.0
[[ramp]]
users = 1.0
hold_secs = 1
[[ramp]]
users = 2.0
hold_secs = 1
"#
        ))
        .unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let started = Instant::now();
        let report = tokio::time::timeout(
            Duration::from_secs(20),
            run(scenario, net, Shard { index: 0, count: 1 }),
        )
        .await
        .expect("run returns while every connect stalls")
        .expect_err("a step whose connects all fail aborts");
        assert!(started.elapsed() < Duration::from_secs(10));
        let steps = report["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1, "the ramp stops at the failing step");
        assert_eq!(steps[0]["aborted"], true);
        let errors = steps[0]["ops"]["ws_connect_auth"]["errors"]
            .as_u64()
            .unwrap();
        assert!(errors >= 4, "{report}");
        assert_eq!(report["clients_cancelled"], 0);
        held.abort();
    }

    #[tokio::test]
    async fn stalled_connects_still_under_way_at_the_stop_settle_and_fail_the_run() {
        // The 2s connect deadline outlasts the 1s hold, so every attempt is
        // still in flight when the step is judged: the step passes, the stop
        // lets the attempts run to their deadline, and they fail the run.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let mut open = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                open.push(socket);
            }
        });
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
connect_timeout_secs = 2.0
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 3
agents = 1
channels_per_member = 1
connect_rate_per_sec = 100.0
[[ramp]]
users = 1.0
hold_secs = 1
"#
        ))
        .unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let started = Instant::now();
        let report = tokio::time::timeout(
            Duration::from_secs(10),
            run(scenario, net, Shard { index: 0, count: 1 }),
        )
        .await
        .expect("the stalled connects settle at their deadline")
        .expect_err("clients that never connected fail the run");
        assert!(started.elapsed() >= Duration::from_secs(2), "{report}");
        let step = &report["steps"][0];
        assert_eq!(step["aborted"], false, "{report}");
        assert_eq!(step["ops"]["ws_connect_auth"]["pending"], 4, "{report}");
        assert_eq!(step["ops"]["ws_connect_auth"]["errors"], 0, "{report}");
        // The attempts finish as connect timeouts, not as abandoned ones.
        let closing = &report["shutdown"];
        assert_eq!(closing["ops"]["ws_connect_auth"]["errors"], 4, "{report}");
        let reasons = closing["failures"].to_string();
        assert!(reasons.contains("connect timeout"), "{report}");
        assert!(!reasons.contains("unfinished at stop"), "{report}");
        assert!(
            closing["abort_reason"]
                .as_str()
                .is_some_and(|r| r.starts_with("ws_connect_auth error rate 1.000")),
            "{report}"
        );
        assert_eq!(report["clients_cancelled"], 0, "{report}");
        held.abort();
    }

    #[test]
    fn a_run_fails_unless_every_step_and_the_shutdown_were_clean() {
        let reason = Some("publish_ephemeral_ok error rate 1.000 > 0.05");
        // (aborted, panicked, cancelled, shutdown) -> failed
        let cases = [
            ((false, 0, 0, None), false),
            ((true, 0, 0, None), true),
            ((false, 1, 0, None), true),
            ((false, 0, 1, None), true),
            ((false, 0, 0, reason), true),
        ];
        for ((aborted, panicked, cancelled, shutdown), expected) in cases {
            assert_eq!(
                failed(aborted, panicked, cancelled, shutdown),
                expected,
                "{aborted} {panicked} {cancelled} {shutdown:?}"
            );
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_stragglers_and_counts_what_they_left_in_flight() {
        let stats = Arc::new(Stats::default());
        let before = stats.snapshot();
        let stuck = {
            let stats = stats.clone();
            tokio::spawn(async move {
                // A publish that never gets its OK.
                let _pending = stats.begin(Op::PublishEphemeral);
                std::future::pending::<()>().await;
            })
        };
        let finished = tokio::spawn(async {});
        // Let the straggler register as in flight before shutting down.
        while stats
            .snapshot()
            .since(&before)
            .iter()
            .all(|(_, s)| s.pending == 0)
        {
            tokio::task::yield_now().await;
        }
        let (panicked, cancelled) =
            shutdown(vec![stuck, finished], Duration::from_millis(100)).await;
        assert_eq!((panicked, cancelled), (0, 1));
        let closed = stats.snapshot().since(&before);
        let publish = closed.iter().find(|(op, _)| *op == Op::PublishEphemeral);
        let publish = publish.unwrap().1;
        assert_eq!((publish.errors, publish.pending), (1, 0));
        let reason = abort_reason(&closed, 0.05, 5_000.0).expect("the straggler fails the run");
        assert!(reason.starts_with("publish_ephemeral_ok"), "{reason}");
    }

    /// How [`fake_ws_relay`] answers a client.
    #[derive(Clone, Copy)]
    struct FakeRelay {
        /// Delay before the AUTH challenge.
        challenge_delay: Duration,
        /// Delay before each EVENT's `OK`; never answered when `None`.
        ok_delay: Option<Duration>,
        /// Delay before the EOSE of each history REQ.
        history_delay: Duration,
        /// From this long after the handshake, each `OK` takes this long.
        slow_ok: Option<(Duration, Duration)>,
        /// Each EVENT's `OK` follows this long after `ok_delay`, without
        /// holding up the frames after it.
        ok_lag: Duration,
        /// Drops the socket this long after the handshake.
        close_after: Option<Duration>,
    }

    impl FakeRelay {
        const PROMPT: Self = Self {
            challenge_delay: Duration::ZERO,
            ok_delay: Some(Duration::ZERO),
            history_delay: Duration::ZERO,
            slow_ok: None,
            ok_lag: Duration::ZERO,
            close_after: None,
        };
    }

    /// A loopback relay that speaks just enough NIP-42 and NIP-01 for a
    /// client: it sends the AUTH challenge, accepts the AUTH event, answers
    /// every REQ with EOSE and every EVENT with an `OK`, each as `relay`
    /// says. Returns its address and what it has seen.
    async fn fake_ws_relay(relay: FakeRelay) -> (String, Arc<Seen>) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let seen = Arc::new(Seen::default());
        let counted = seen.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seen = counted.clone();
                tokio::spawn(async move {
                    // Counted on every exit, so frames read before it are final.
                    let _closed = Closed(seen.clone());
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    let opened = Instant::now();
                    tokio::time::sleep(relay.challenge_delay).await;
                    let challenge = json!(["AUTH", "challenge"]).to_string();
                    if ws.send(Message::Text(challenge.into())).await.is_err() {
                        return;
                    }
                    let (mut sink, mut ws) = ws.split();
                    let (out, mut replies) = tokio::sync::mpsc::unbounded_channel::<Value>();
                    tokio::spawn(async move {
                        while let Some(reply) = replies.recv().await {
                            let text = Message::Text(reply.to_string().into());
                            if sink.send(text).await.is_err() {
                                return;
                            }
                        }
                    });
                    let serve = async move {
                        while let Some(Ok(Message::Text(text))) = ws.next().await {
                            let frame: Value = serde_json::from_str(&text).unwrap();
                            let reply = match frame[0].as_str() {
                                Some("AUTH") => json!(["OK", frame[1]["id"], true, ""]),
                                Some("REQ") => {
                                    seen.reqs.fetch_add(1, Ordering::SeqCst);
                                    let id = frame[1].as_str().unwrap_or_default();
                                    if id.starts_with(HISTORY_PREFIX) {
                                        tokio::time::sleep(relay.history_delay).await;
                                    }
                                    json!(["EOSE", frame[1]])
                                }
                                Some("EVENT") => {
                                    if let Some(k) = frame[1]["kind"].as_u64() {
                                        seen.kinds.lock().unwrap().push((k, Instant::now()));
                                    }
                                    let tags =
                                        frame[1]["tags"].as_array().cloned().unwrap_or_default();
                                    if let Some(h) = tags.iter().find(|t| t[0] == "h") {
                                        let h = h[1].as_str().unwrap_or_default().to_string();
                                        seen.channels.lock().unwrap().push(h);
                                    }
                                    let Some(mut delay) = relay.ok_delay else {
                                        continue;
                                    };
                                    if let Some((after, slow)) = relay.slow_ok {
                                        if opened.elapsed() >= after {
                                            delay = slow;
                                        }
                                    }
                                    tokio::time::sleep(delay).await;
                                    let ok = json!(["OK", frame[1]["id"], true, ""]);
                                    if !relay.ok_lag.is_zero() {
                                        let out = out.clone();
                                        tokio::spawn(async move {
                                            tokio::time::sleep(relay.ok_lag).await;
                                            let _ = out.send(ok);
                                        });
                                        continue;
                                    }
                                    ok
                                }
                                _ => continue,
                            };
                            if out.send(reply).is_err() {
                                return;
                            }
                        }
                    };
                    // Ending `serve` drops both halves of the socket.
                    match relay.close_after {
                        Some(after) => {
                            let _ = tokio::time::timeout(after, serve).await;
                        }
                        None => serve.await,
                    }
                });
            }
        });
        (addr, seen)
    }

    /// What [`fake_ws_relay`] has received.
    #[derive(Default)]
    struct Seen {
        reqs: AtomicUsize,
        closed: AtomicUsize,
        /// The `h` tag of every EVENT that had one.
        channels: std::sync::Mutex<Vec<String>>,
        /// The kind of every EVENT, with when it arrived.
        kinds: std::sync::Mutex<Vec<(u64, Instant)>>,
    }

    impl Seen {
        /// REQs received over `sessions` connections, once all have closed.
        /// A client sends its REQs before it closes, so the count is final.
        async fn reqs_after(&self, sessions: usize) -> usize {
            tokio::time::timeout(Duration::from_secs(5), async {
                while self.closed.load(Ordering::SeqCst) < sessions {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the client closes its connection");
            self.reqs.load(Ordering::SeqCst)
        }
    }

    struct Closed(Arc<Seen>);

    impl Drop for Closed {
        fn drop(&mut self) {
            self.0.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn wide_activity_share_splits_picks_and_unset_keeps_the_uniform_pick() {
        let channels: Vec<String> = (0..6).map(|i| format!("c{i}")).collect();
        let draws = 100_000;
        let wide_share = |split: Option<WideSplit>| {
            let mut rng = Rng::new(7);
            let wide = (0..draws)
                .filter(|_| pick_channel(split, &channels, &mut rng) < 3)
                .count();
            wide as f64 / draws as f64
        };
        let split = |share| Some(WideSplit { wide: 3, share });
        // Unset: the same draws as before the knob, so every channel is as likely.
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..1_000 {
            assert_eq!(
                pick_channel(None, &channels, &mut a),
                b.below(channels.len())
            );
        }
        assert!((wide_share(None) - 0.5).abs() < 0.01);
        assert!((wide_share(split(0.1)) - 0.1).abs() < 0.01);
        assert!((wide_share(split(0.8)) - 0.8).abs() < 0.01);
        assert_eq!(wide_share(split(1.0)), 1.0);
        assert_eq!(wide_share(split(0.0)), 0.0);
        // Regular picks stay uniform over the regular channels.
        let mut rng = Rng::new(7);
        let mut per = [0usize; 6];
        for _ in 0..draws {
            per[pick_channel(split(0.1), &channels, &mut rng)] += 1;
        }
        for n in &per[3..] {
            assert!((*n as f64 / draws as f64 - 0.3).abs() < 0.01, "{per:?}");
        }
        // A member in wide channels only sends everything to them.
        let mut rng = Rng::new(7);
        let only_wide = Some(WideSplit {
            wide: 6,
            share: 0.1,
        });
        let mut seen = [false; 6];
        for _ in 0..1_000 {
            seen[pick_channel(only_wide, &channels, &mut rng)] = true;
        }
        assert_eq!(seen, [true; 6]);
    }

    /// One human in three wide and three regular channels that only types,
    /// ten times a second.
    fn typing_wide_member(addr: &str, share: &str) -> Scenario {
        Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 6
wide_channels = 3
[population]
humans = 1
agents = 0
channels_per_member = 3
wide_member_fraction = 1.0
{share}
[human]
presence_interval_secs = 3600.0
typing_per_hour = 36000.0
messages_per_hour = 0.0
reactions_per_hour = 0.0
history_limit = 1
[[ramp]]
users = 1.0
hold_secs = 2
"#
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn typing_follows_the_wide_activity_share() {
        let mut wide_counts = Vec::new();
        for share in ["", "wide_activity_share = 0.0", "wide_activity_share = 1.0"] {
            let (addr, seen) = fake_ws_relay(FakeRelay::PROMPT).await;
            let scenario = typing_wide_member(&addr, share);
            let wide: Vec<String> = scenario.channel_ids(0)[..3].to_vec();
            run_lone_human(scenario, Duration::from_millis(300))
                .await
                .expect("the run passes");
            let channels = seen.channels.lock().unwrap().clone();
            assert!(channels.len() >= 5, "{share}: {channels:?}");
            let in_wide = channels.iter().filter(|c| wide.contains(c)).count();
            wide_counts.push((in_wide, channels.len()));
        }
        let [(unset, n), (none, _), (all, m)] = wide_counts[..] else {
            unreachable!()
        };
        assert!(
            unset > 0 && unset < n,
            "unset picks both kinds: {wide_counts:?}"
        );
        assert_eq!(none, 0, "{wide_counts:?}");
        assert_eq!(all, m, "{wide_counts:?}");
    }

    /// One human, no agents, one `hold_secs` step, and a presence heartbeat
    /// every second; other traffic is off so the heartbeat is the only publish.
    fn lone_human(addr: &str, hold_secs: u64, max_error_rate: f64) -> Scenario {
        Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 1
agents = 0
channels_per_member = 1
[human]
presence_interval_secs = 1.0
typing_per_hour = 0.0
messages_per_hour = 0.0
reactions_per_hour = 0.0
history_limit = 1
[abort]
max_error_rate = {max_error_rate}
[[ramp]]
users = 1.0
hold_secs = {hold_secs}
"#
        ))
        .unwrap()
    }

    async fn run_lone_human(scenario: Scenario, grace: Duration) -> Result<Value, Value> {
        let net = Net::new(&scenario).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(20),
            run_with(scenario, net, Shard { index: 0, count: 1 }, grace),
        )
        .await
        .expect("the run returns")
    }

    #[tokio::test]
    async fn a_publish_that_never_gets_its_ok_is_cancelled_and_fails_the_run() {
        let (addr, _) = fake_ws_relay(FakeRelay {
            ok_delay: None,
            ..FakeRelay::PROMPT
        })
        .await;
        let report = run_lone_human(lone_human(&addr, 2, 0.05), Duration::from_millis(300))
            .await
            .expect_err("a relay that never acknowledges a publish fails the run");
        // The heartbeat is still waiting for its OK, which is not judged yet,
        // but the client cannot settle it within the grace period.
        let step = &report["steps"][0];
        assert_eq!(step["aborted"], false, "{report}");
        assert_eq!(
            step["ops"]["publish_ephemeral_ok"]["pending"], 1,
            "{report}"
        );
        assert_eq!(report["clients_cancelled"], 1, "{report}");
        let closing = &report["shutdown"];
        assert_eq!(
            closing["ops"]["publish_ephemeral_ok"]["errors"], 1,
            "{report}"
        );
        assert!(
            closing["failures"]
                .to_string()
                .contains("unfinished at stop"),
            "{report}"
        );
    }

    #[tokio::test]
    async fn a_cancelled_client_fails_the_run_even_within_the_error_limit() {
        // With the error limit lifted, only the cancellation can fail the run.
        let (addr, _) = fake_ws_relay(FakeRelay {
            ok_delay: None,
            ..FakeRelay::PROMPT
        })
        .await;
        let report = run_lone_human(lone_human(&addr, 2, 1.0), Duration::from_millis(300))
            .await
            .expect_err("cancelling a client fails the run");
        assert_eq!(report["clients_cancelled"], 1, "{report}");
        assert!(report["shutdown"]["abort_reason"].is_null(), "{report}");
    }

    #[tokio::test]
    async fn a_publish_in_flight_at_the_snapshot_settles_and_the_run_passes() {
        // The OK takes 1.5s, so a heartbeat is awaiting it when the 2s hold
        // ends. The relay is healthy: the publish settles during shutdown.
        let (addr, _) = fake_ws_relay(FakeRelay {
            ok_delay: Some(Duration::from_millis(1500)),
            ..FakeRelay::PROMPT
        })
        .await;
        let report = run_lone_human(lone_human(&addr, 2, 0.05), Duration::from_secs(5))
            .await
            .expect("a healthy publish straddling the snapshot does not fail the run");
        assert_eq!(
            report["steps"][0]["ops"]["publish_ephemeral_ok"]["pending"], 1,
            "{report}"
        );
        let closing = &report["shutdown"]["ops"]["publish_ephemeral_ok"];
        assert!(closing["count"].as_u64().unwrap() >= 1, "{report}");
        assert_eq!(closing["errors"], 0, "{report}");
        assert_eq!(report["clients_cancelled"], 0, "{report}");
    }

    #[tokio::test]
    async fn a_connect_under_way_at_the_stop_settles_and_the_run_passes() {
        // The relay takes 1.5s to challenge, so the first connect is still in
        // flight when the 1s hold ends and the run stops. It is not abandoned,
        // and it does not start a session: the client subscribes to nothing.
        let (addr, seen) = fake_ws_relay(FakeRelay {
            challenge_delay: Duration::from_millis(1500),
            ..FakeRelay::PROMPT
        })
        .await;
        let report = run_lone_human(lone_human(&addr, 1, 0.05), Duration::from_secs(5))
            .await
            .expect("a healthy connect straddling the stop does not fail the run");
        assert_eq!(
            report["steps"][0]["ops"]["ws_connect_auth"]["pending"], 1,
            "{report}"
        );
        let closing = &report["shutdown"]["ops"]["ws_connect_auth"];
        assert_eq!(
            (closing["count"].clone(), closing["errors"].clone()),
            (json!(1), json!(0)),
            "{report}"
        );
        assert_eq!(seen.reqs_after(1).await, 0, "{report}");
    }

    #[tokio::test]
    async fn a_stop_during_history_ends_the_session_after_the_fetch_under_way() {
        // Ten channels whose history takes 400ms each: the 1s hold ends
        // mid-history. Fetching the rest would outlast the 2s grace and get
        // the client cancelled; the fetch under way settles and the rest are
        // skipped.
        let (addr, seen) = fake_ws_relay(FakeRelay {
            history_delay: Duration::from_millis(400),
            ..FakeRelay::PROMPT
        })
        .await;
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 10
[population]
humans = 1
agents = 0
channels_per_member = 10
[human]
typing_per_hour = 0.0
messages_per_hour = 0.0
reactions_per_hour = 0.0
history_limit = 1
[[ramp]]
users = 1.0
hold_secs = 1
"#
        ))
        .unwrap();
        let report = run_lone_human(scenario, Duration::from_secs(2))
            .await
            .expect("a stop mid-history does not fail a healthy run");
        assert_eq!(report["clients_cancelled"], 0, "{report}");
        // Four live subscriptions plus the history fetched before the stop.
        let sent = seen.reqs_after(1).await;
        assert!((5..4 + 10).contains(&sent), "{sent} REQs: {report}");
    }

    #[tokio::test]
    async fn one_slow_publish_in_flight_at_the_stop_is_judged_with_the_last_step() {
        // About 200 typing indicators a second are acknowledged at once until
        // 1.7s in; from then each OK takes 1.5s, so exactly one is in flight
        // when the 2s hold ends. On its own, the settlement's p99 would be
        // that one slow publish; among the step's fast ones it is not.
        let (addr, _) = fake_ws_relay(FakeRelay {
            slow_ok: Some((Duration::from_millis(1700), Duration::from_millis(1500))),
            ..FakeRelay::PROMPT
        })
        .await;
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 1
agents = 0
channels_per_member = 1
[human]
presence_interval_secs = 3600.0
typing_per_hour = 720000.0
messages_per_hour = 0.0
reactions_per_hour = 0.0
history_limit = 1
[abort]
max_p99_ms = 1000.0
[[ramp]]
users = 1.0
hold_secs = 2
"#
        ))
        .unwrap();
        let report = run_lone_human(scenario, Duration::from_secs(5))
            .await
            .expect("one slow publish among many fast ones does not fail the run");
        let step = &report["steps"][0]["ops"]["publish_ephemeral_ok"];
        assert!(step["count"].as_u64().unwrap() >= 100, "{report}");
        assert_eq!(step["pending"], 1, "{report}");
        let closing = &report["shutdown"]["ops"]["publish_ephemeral_ok"];
        assert_eq!(
            (closing["count"].clone(), closing["errors"].clone()),
            (json!(1), json!(0)),
            "{report}"
        );
        assert!(closing["p99_ms"].as_f64().unwrap() > 1000.0, "{report}");
    }

    /// What a hot-thread run sent: its outcome, the events and `/query`
    /// filters it posted, and the hot root's id and channel.
    struct HotRun {
        outcome: Result<Value, Value>,
        posted: Vec<Event>,
        queries: Vec<Value>,
        root_id: String,
        channel: String,
    }

    type Log<T> = Arc<std::sync::Mutex<Vec<T>>>;

    /// A fake relay for the hot thread: the events it received, the `/query`
    /// filters it answered, and the hot root's id and channel.
    struct HotRelay {
        addr: String,
        posted: Log<Event>,
        queries: Log<Value>,
        root_id: String,
        channel: String,
    }

    /// `[agent]` turns for one agent in one channel over `ws` and `http`,
    /// held for 3 s, with `extra` TOML appended.
    fn agent_turn_scenario(ws: &str, http: &str, extra: &str) -> Scenario {
        Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{ws}"
http_url = "http://{http}"
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 0
agents = 1
channels_per_member = 1
[agent]
query_interval_secs = 0.0
events_per_hour = 0.0
observer_frames_per_hour = 0.0
presence_interval_secs = 1.0
channel_subscriptions = true
[agent.turn]
per_hour = 36000.0
mean_secs = 0.6
frame_secs = 0.1
typing_secs = 0.1
reads = 2
[[ramp]]
users = 1.0
hold_secs = 3
{extra}
"#
        ))
        .unwrap()
    }

    /// How long the fake relay holds each tool read.
    const TOOL_READ_HOLD: Duration = Duration::from_millis(250);

    /// One agent running turns over a fake socket and a fake HTTP relay
    /// that holds one channel message, a full thread window and the agent's
    /// live reactions, and is slow to answer tool reads.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agent_turns_read_react_stream_reply_and_remove_their_reactions() {
        agent_turns_against(FakeRelay::PROMPT).await;
    }

    /// The same turns when every socket `OK` is slower than the agent's
    /// heartbeat, though well inside the abort ceiling: nothing waits for an
    /// `OK`, so tool reads, typing and frames all keep their schedule.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agent_turns_keep_moving_while_socket_oks_lag() {
        let lag = Duration::from_millis(1500);
        let report = agent_turns_against(FakeRelay {
            ok_lag: lag,
            ..FakeRelay::PROMPT
        })
        .await;
        let acked = &report["steps"][0]["ops"]["publish_ephemeral_ok"];
        assert!(acked["count"].as_u64() > Some(0), "{report}");
        assert_eq!(acked["errors"], 0, "{report}");
        let p50 = acked["p50_ms"].as_f64().unwrap();
        assert!(p50 >= lag.as_millis() as f64, "{report}");
        // The OKs still due at the stop settle during the shutdown.
        let closing = &report["shutdown"]["ops"]["publish_ephemeral_ok"];
        assert!(closing["count"].as_u64() > Some(0), "{report}");
        assert_eq!(closing["errors"], 0, "{report}");
    }

    #[tokio::test]
    async fn an_agent_publish_that_never_gets_its_ok_fails_the_run() {
        let (addr, _) = fake_ws_relay(FakeRelay {
            ok_delay: None,
            ..FakeRelay::PROMPT
        })
        .await;
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 0
agents = 1
channels_per_member = 1
[agent]
query_interval_secs = 0.0
events_per_hour = 0.0
observer_frames_per_hour = 0.0
presence_interval_secs = 1.0
[[ramp]]
users = 1.0
hold_secs = 2
"#
        ))
        .unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let report = tokio::time::timeout(
            Duration::from_secs(20),
            run_with(
                scenario,
                net,
                Shard { index: 0, count: 1 },
                Duration::from_millis(300),
            ),
        )
        .await
        .expect("the run returns")
        .expect_err("a relay that never acknowledges a publish fails the run");
        assert_eq!(report["clients_cancelled"], 1, "{report}");
        let closing = &report["shutdown"];
        assert!(
            closing["ops"]["publish_ephemeral_ok"]["errors"].as_u64() > Some(0),
            "{report}"
        );
        assert!(
            closing["failures"]
                .to_string()
                .contains("unfinished at stop"),
            "{report}"
        );
    }

    /// A relay that drops the socket while the agent drains its publishes at
    /// the stop: the ones still awaiting their `OK` fail, so the drop cannot
    /// pass the run. The agent's loop wakes at least every second, so it is
    /// draining well before the drop, 2 s after the 2 s hold ends.
    #[tokio::test]
    async fn a_socket_lost_with_publishes_awaiting_their_ok_at_the_stop_fails_the_run() {
        let (addr, _) = fake_ws_relay(FakeRelay {
            ok_delay: None,
            close_after: Some(Duration::from_secs(4)),
            ..FakeRelay::PROMPT
        })
        .await;
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 0
agents = 1
channels_per_member = 1
[agent]
query_interval_secs = 0.0
events_per_hour = 0.0
observer_frames_per_hour = 0.0
presence_interval_secs = 1.0
[[ramp]]
users = 1.0
hold_secs = 2
"#
        ))
        .unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let report = tokio::time::timeout(
            Duration::from_secs(20),
            run_with(
                scenario,
                net,
                Shard { index: 0, count: 1 },
                Duration::from_secs(10),
            ),
        )
        .await
        .expect("the run returns")
        .expect_err("publishes lost with their socket fail the run");
        assert_eq!(report["clients_cancelled"], 0, "{report}");
        let closing = &report["shutdown"];
        assert!(
            closing["ops"]["publish_ephemeral_ok"]["errors"].as_u64() > Some(0),
            "{report}"
        );
        assert!(
            closing["failures"]
                .to_string()
                .contains("disconnected before OK"),
            "{report}"
        );
        assert!(!closing["abort_reason"].is_null(), "{report}");
    }

    /// A relay that completes the handshake and then never reads: a publish
    /// that cannot be written within its timeout fails and drops the socket.
    #[tokio::test]
    async fn a_publish_the_relay_never_reads_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            std::future::pending::<()>().await;
            drop(ws);
        });
        let mut conn = NostrWsConnection::connect(&format!("ws://{addr}"))
            .await
            .unwrap();
        let stats = Stats::default();
        let before = stats.snapshot();
        let mut unacked = Unacked::new(&stats, Duration::from_millis(200));
        let keys = Keys::generate();
        let big = "x".repeat(1 << 20);
        let mut result = Ok(());
        for _ in 0..MAX_UNACKED {
            let event = EventBuilder::new(kind(KIND_TYPING_INDICATOR), big.as_str())
                .sign_with_keys(&keys)
                .map_err(|e| e.to_string());
            result = unacked.send(&mut conn, Op::PublishEphemeral, event).await;
            if result.is_err() {
                break;
            }
        }
        assert!(
            matches!(result, Err(WsClientError::Timeout)),
            "every write went through: {result:?}"
        );
        let reasons = stats.snapshot().reasons_since(&before, 10);
        assert!(
            reasons.iter().any(|(r, _)| r.contains("send timeout")),
            "{reasons:?}"
        );
        unacked.forget();
        held.abort();
    }

    /// Runs one agent's turns against `relay` and checks what they sent.
    async fn agent_turns_against(relay: FakeRelay) -> Value {
        let (ws, seen) = fake_ws_relay(relay).await;
        let channel = identity::channel_id("t", 0, 0);
        let me = identity::keys("t", Role::Agent, 0).public_key().to_hex();
        // A human's thread reply: only a `reply` marker, naming the root.
        let author = Keys::generate();
        let root_id = "a".repeat(64);
        let target = EventBuilder::new(kind(KIND_STREAM_MESSAGE), "question")
            .tags([tag(["h", &channel]), tag(["e", &root_id, "", "reply"])])
            .sign_with_keys(&author)
            .unwrap();
        // One reply past the context window, so each turn counts the thread.
        let window: Vec<Event> = (0..=THREAD_CONTEXT_REPLIES)
            .map(|i| {
                EventBuilder::new(kind(KIND_STREAM_MESSAGE), format!("reply {i}"))
                    .tags([tag(["h", &channel]), tag(["e", &root_id, "", "reply"])])
                    .sign_with_keys(&author)
                    .unwrap()
            })
            .collect();
        let posted: Log<Event> = Default::default();
        let queries: Log<Value> = Default::default();
        let counts: Log<Value> = Default::default();
        let held: Log<(Instant, Instant)> = Default::default();
        let (post_log, query_log, count_log, hold_log, newest) = (
            posted.clone(),
            queries.clone(),
            counts.clone(),
            held.clone(),
            target.clone(),
        );
        let http = super::super::seed::tests::fake_http(move |_, path, body| {
            if path == "/events" {
                let event: Event = serde_json::from_slice(body).unwrap();
                post_log.lock().unwrap().push(event);
                return (200, json!({"accepted": true, "message": ""}).to_string());
            }
            let filters: Value = serde_json::from_slice(body).unwrap();
            if path == "/count" {
                count_log.lock().unwrap().push(filters);
                return (200, json!({"count": 40}).to_string());
            }
            query_log.lock().unwrap().push(filters.clone());
            if filters[0]["limit"] == json!(50) {
                let start = Instant::now();
                tokio::task::block_in_place(|| std::thread::sleep(TOOL_READ_HOLD));
                hold_log.lock().unwrap().push((start, Instant::now()));
                return (200, "[]".to_string());
            }
            if filters[0].get("ids").is_some() {
                return (200, json!(window).to_string());
            }
            let found = match filters[0]["kinds"][0].as_u64() {
                Some(k) if k == u64::from(KIND_STREAM_MESSAGE) => json!([newest]),
                // Reactions not deleted since they were posted. Two turns in
                // one second post the same reaction id again.
                Some(k) if k == u64::from(KIND_REACTION) => {
                    let mut live: Vec<Event> = Vec::new();
                    for e in post_log.lock().unwrap().iter() {
                        if e.kind == kind(KIND_REACTION) {
                            live.push(e.clone());
                        } else if e.kind == kind(KIND_DELETION) {
                            let gone: Vec<&str> =
                                e.tags.iter().filter_map(|t| t.content()).collect();
                            live.retain(|r| !gone.contains(&r.id.to_hex().as_str()));
                        }
                    }
                    json!(live)
                }
                _ => json!([]),
            };
            (200, found.to_string())
        })
        .await;
        let scenario = agent_turn_scenario(&ws, &http, "");
        let net = Net::new(&scenario).await.unwrap();
        let shard = Shard::parse("0/1").unwrap();
        let report = run_with(scenario, net, shard, Duration::from_secs(5))
            .await
            .expect("agent turn run passes");
        let ops = &report["steps"][0]["ops"];
        assert!(ops["turn_context"]["count"].as_u64() > Some(0), "{report}");

        // Mentions plus one channel subscription on the one session.
        assert_eq!(seen.reqs_after(1).await, 2);
        let kinds = seen.kinds.lock().unwrap().clone();
        for k in [
            KIND_TYPING_INDICATOR,
            KIND_AGENT_OBSERVER_FRAME,
            KIND_PRESENCE_UPDATE,
        ] {
            assert!(
                kinds.iter().any(|(seen, _)| *seen == u64::from(k)),
                "no kind {k} over the socket: {kinds:?}"
            );
        }
        // Typing and frames keep streaming while a tool read is held.
        let held = held.lock().unwrap().clone();
        assert!(!held.is_empty(), "no tool read made");
        let streaming = [KIND_TYPING_INDICATOR, KIND_AGENT_OBSERVER_FRAME].map(u64::from);
        let during = kinds.iter().filter(|(k, at)| {
            streaming.contains(k)
                && held
                    .iter()
                    .any(|(start, end)| *at > *start + TOOL_READ_HOLD / 5 && at < end)
        });
        assert!(
            during.count() > 0,
            "nothing streamed during {held:?}: {kinds:?}"
        );

        let posted = posted.lock().unwrap().clone();
        let of = |k: u32| posted.iter().filter(move |e| e.kind == kind(k));
        let target_id = target.id.to_hex();
        let reactions: Vec<&Event> = of(KIND_REACTION).collect();
        assert!(reactions.len() >= 4, "two reactions a turn: {posted:?}");
        for reaction in &reactions {
            let e = reaction.tags.iter().find(|t| t.kind().as_str() == "e");
            assert_eq!(e.and_then(|t| t.content()), Some(target_id.as_str()));
        }
        // Every finished turn replied at the first level of the target's
        // thread, under its root, and deleted both of its reactions; only a
        // turn cut by the stop leaves any.
        let replies: Vec<&Event> = of(KIND_STREAM_MESSAGE).collect();
        assert!(!replies.is_empty(), "{posted:?}");
        for reply in &replies {
            let markers = buzz_core::nip10::parse_thread_markers(&reply.tags);
            assert_eq!(markers.root.as_deref(), Some(root_id.as_str()));
            assert_eq!(markers.reply.as_deref(), Some(root_id.as_str()));
        }
        let deleted: Vec<String> = of(KIND_DELETION)
            .flat_map(|e| {
                e.tags
                    .iter()
                    .filter_map(|t| t.content())
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(deleted.len(), 2 * replies.len(), "{posted:?}");
        let ids: Vec<String> = reactions.iter().map(|e| e.id.to_hex()).collect();
        assert!(deleted.iter().all(|id| ids.contains(id)), "{deleted:?}");

        // Each turn reads its thread's context as the harness does, and
        // counts the thread when the window overflows.
        let queries = queries.lock().unwrap().clone();
        let replies_filter = json!({
            "kinds": AGENT_READ_KINDS,
            "#e": [root_id],
            "#h": [channel],
            "limit": THREAD_CONTEXT_REPLIES + 1,
        });
        let mut mine = replies_filter.clone();
        mine["authors"] = json!([me]);
        mine["limit"] = json!(1);
        let context = json!([{ "ids": [root_id] }, replies_filter.clone(), mine]);
        assert!(queries.contains(&context), "{queries:?}");
        let mut count = replies_filter;
        count["limit"] = json!(0);
        let counts = counts.lock().unwrap().clone();
        assert!(!counts.is_empty(), "no thread count");
        assert!(counts.iter().all(|c| *c == json!([count])), "{counts:?}");
        // Then the channel's metadata, project home and the author's profile.
        let has = |k: u32| queries.iter().any(|q| q[0]["kinds"][0] == json!(k));
        for k in [
            KIND_NIP29_GROUP_METADATA,
            KIND_PROJECT,
            KIND_GIT_REPO_ANNOUNCEMENT,
        ] {
            assert!(has(k), "no kind {k} read: {queries:?}");
        }
        let profile =
            json!([{ "kinds": [KIND_PROFILE], "authors": [author.public_key().to_hex()] }]);
        assert!(queries.contains(&profile), "{queries:?}");
        // Tool reads fetch the channel's newest 50 messages.
        assert!(
            queries.iter().any(|q| q[0]["limit"] == json!(50)),
            "{queries:?}"
        );
        report
    }

    /// A relay that answers every `/query` with 200 and a body that is not
    /// events: each read is an error, and a turn has nothing to answer.
    #[tokio::test]
    async fn agent_turn_reads_that_do_not_decode_are_errors() {
        let (ws, _) = fake_ws_relay(FakeRelay::PROMPT).await;
        let posted: Log<Event> = Default::default();
        let post_log = posted.clone();
        let http = super::super::seed::tests::fake_http(move |_, path, body| {
            if path == "/events" {
                post_log
                    .lock()
                    .unwrap()
                    .push(serde_json::from_slice(body).unwrap());
                return (200, json!({"accepted": true, "message": ""}).to_string());
            }
            (200, json!({"events": "none"}).to_string())
        })
        .await;
        let scenario = agent_turn_scenario(&ws, &http, "[abort]\nmax_error_rate = 1.0");
        let net = Net::new(&scenario).await.unwrap();
        let shard = Shard::parse("0/1").unwrap();
        let report = run_with(scenario, net, shard, Duration::from_secs(5))
            .await
            .expect("the run tolerates every error");
        let ops = &report["steps"][0]["ops"];
        for op in ["turn_context", "http_query"] {
            assert!(ops[op]["errors"].as_u64() > Some(0), "{op}: {report}");
            assert_eq!(ops[op]["count"], 0, "{op}: {report}");
        }
        // No target was found, so no turn reacted.
        let posted = posted.lock().unwrap();
        assert!(
            posted.iter().all(|e| e.kind != kind(KIND_REACTION)),
            "{posted:?}"
        );
    }

    /// Runs one agent's turns against a relay that holds one top-level
    /// channel message and the agent's live reactions, and holds the first
    /// request `held` picks past the stop. Returns every request's path and
    /// body, in order, and where the held one is among them.
    async fn turns_stopped_during(held: fn(&str, &Value) -> bool) -> (Vec<(String, Value)>, usize) {
        let (ws, _) = fake_ws_relay(FakeRelay::PROMPT).await;
        let channel = identity::channel_id("t", 0, 0);
        let target = EventBuilder::new(kind(KIND_STREAM_MESSAGE), "question")
            .tags([tag(["h", &channel])])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let log: Log<(String, Value)> = Default::default();
        let at: Arc<std::sync::Mutex<Option<usize>>> = Default::default();
        let (requests, held_at) = (log.clone(), at.clone());
        // Released after the 3 s hold has stopped the run, well inside the
        // shutdown grace.
        let release = Instant::now() + Duration::from_millis(4500);
        let http = super::super::seed::tests::fake_http(move |_, path, body| {
            let body: Value = serde_json::from_slice(body).unwrap();
            let hold = {
                let mut requests = requests.lock().unwrap();
                requests.push((path.to_string(), body.clone()));
                let mut held_at = held_at.lock().unwrap();
                let hold = held_at.is_none() && held(path, &body);
                if hold {
                    *held_at = Some(requests.len() - 1);
                }
                hold
            };
            if hold {
                tokio::task::block_in_place(|| {
                    std::thread::sleep(release.saturating_duration_since(Instant::now()));
                });
            }
            if path == "/events" {
                return (200, json!({"accepted": true, "message": ""}).to_string());
            }
            let found = match body[0]["kinds"][0].as_u64() {
                Some(k) if k == u64::from(KIND_STREAM_MESSAGE) => json!([target]),
                Some(k) if k == u64::from(KIND_REACTION) => {
                    let posted = requests.lock().unwrap();
                    let reactions = posted
                        .iter()
                        .filter(|(path, e)| path == "/events" && e["kind"] == json!(KIND_REACTION));
                    json!(reactions.map(|(_, e)| e).collect::<Vec<_>>())
                }
                _ => json!([]),
            };
            (200, found.to_string())
        })
        .await;
        let scenario = agent_turn_scenario(&ws, &http, "");
        let net = Net::new(&scenario).await.unwrap();
        let shard = Shard::parse("0/1").unwrap();
        let report = run_with(scenario, net, shard, Duration::from_secs(5))
            .await
            .expect("a turn cut by the stop does not fail the run");
        assert_eq!(report["clients_cancelled"], 0, "{report}");
        let log = log.lock().unwrap().clone();
        let at = at.lock().unwrap().expect("the request was made and held");
        (log, at)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_turn_stopped_while_adding_a_reaction_sends_nothing_more() {
        let (log, at) = turns_stopped_during(|path, body| {
            path == "/events" && body["kind"] == json!(KIND_REACTION)
        })
        .await;
        assert_eq!(at + 1, log.len(), "requests after the stop: {log:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_turn_stopped_while_reading_its_reactions_deletes_none() {
        let (log, at) = turns_stopped_during(|path, body| {
            path == "/query" && body[0]["kinds"][0] == json!(KIND_REACTION)
        })
        .await;
        assert_eq!(at + 1, log.len(), "requests after the stop: {log:?}");
        let deleted = log.iter().any(|(_, e)| e["kind"] == json!(KIND_DELETION));
        assert!(!deleted, "{log:?}");
    }

    /// Holds `/events` responses until opened, counting the requests held.
    #[derive(Default)]
    struct Gate {
        arrived: AtomicUsize,
        open: std::sync::Mutex<bool>,
        opened: std::sync::Condvar,
    }

    impl Gate {
        fn release(&self) {
            *self.open.lock().unwrap() = true;
            self.opened.notify_all();
        }
    }

    /// Serves a relay whose root lookup returns a newer reply-shaped decoy by
    /// the root's author, plus the hot root itself when `seeded`. With a
    /// `gate`, every event waits for it to open before it is accepted.
    async fn hot_relay(seeded: bool, gate: Option<Arc<Gate>>) -> HotRelay {
        let author = hot_root_keys("t");
        let channel = identity::channel_id("t", 0, 0);
        let root = EventBuilder::new(kind(KIND_STREAM_MESSAGE), HOT_ROOT_PREFIX)
            .tags([tag(["h", &channel])])
            .sign_with_keys(&author)
            .unwrap();
        let root_id = root.id.to_hex();
        let decoy = EventBuilder::new(kind(KIND_STREAM_MESSAGE), "lg-hot-reply 0")
            .tags(
                [tag(["h", &channel])]
                    .into_iter()
                    .chain(reply_tags(&"0".repeat(64), &"0".repeat(64))),
            )
            .custom_created_at(Timestamp::from(root.created_at.as_secs() + 1))
            .sign_with_keys(&author)
            .unwrap();
        let posted: Log<Event> = Default::default();
        let queries: Log<Value> = Default::default();
        let (post_log, query_log) = (posted.clone(), queries.clone());
        let addr = super::super::seed::tests::fake_http(move |_, target, body| {
            if target == "/events" {
                let event: Event = serde_json::from_slice(body).unwrap();
                post_log.lock().unwrap().push(event);
                if let Some(gate) = &gate {
                    gate.arrived.fetch_add(1, Ordering::SeqCst);
                    tokio::task::block_in_place(|| {
                        let open = gate.open.lock().unwrap();
                        drop(gate.opened.wait_while(open, |open| !*open).unwrap());
                    });
                }
                return (200, json!({"accepted": true, "message": ""}).to_string());
            }
            let filter: Value = serde_json::from_slice(body).unwrap();
            let lookup = filter[0].get("authors").is_some();
            query_log.lock().unwrap().push(filter[0].clone());
            let found = match (lookup, seeded) {
                (true, true) => json!([root, decoy]),
                (true, false) => json!([decoy]),
                (false, _) => json!([]),
            };
            (200, found.to_string())
        })
        .await;
        HotRelay {
            addr,
            posted,
            queries,
            root_id,
            channel,
        }
    }

    /// A one-second scenario with `[hot_thread]` section `hot` against `addr`.
    fn hot_scenario(addr: &str, hot: &str) -> Scenario {
        Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 1
[population]
humans = 0
agents = 0
channels_per_member = 1
[hot_thread]
{hot}
[[ramp]]
users = 1.0
hold_secs = 1
"#
        ))
        .unwrap()
    }

    /// Runs `[hot_thread]` section `hot` for a second against `hot_relay`.
    async fn hot_thread_run(seeded: bool, hot: &str) -> HotRun {
        let relay = hot_relay(seeded, None).await;
        let scenario = hot_scenario(&relay.addr, hot);
        let net = Net::new(&scenario).await.unwrap();
        let shard = Shard::parse("0/1").unwrap();
        let outcome = run_with(scenario, net, shard, Duration::from_secs(5)).await;
        let posted = relay.posted.lock().unwrap().clone();
        let queries = relay.queries.lock().unwrap().clone();
        HotRun {
            outcome,
            posted,
            queries,
            root_id: relay.root_id,
            channel: relay.channel,
        }
    }

    const TWO_AUTHORS: &str = "authors = 2\nreplies_per_hour = 36000.0\nfetches_per_hour = 36000.0";

    #[tokio::test]
    async fn hot_thread_authors_reply_to_the_hot_root_and_fail_without_it() {
        let run = hot_thread_run(true, TWO_AUTHORS).await;
        let report = run.outcome.expect("hot-thread run passes");
        assert_eq!(report["steps"][0]["hot_authors"], 2, "{report}");
        assert!(!run.posted.is_empty(), "{report}");
        for event in &run.posted {
            let markers = buzz_core::nip10::parse_thread_markers(&event.tags);
            assert_eq!(markers.root.as_deref(), Some(run.root_id.as_str()));
            assert_eq!(markers.reply.as_deref(), Some(run.root_id.as_str()));
            let h = event
                .tags
                .iter()
                .find(|t| t.kind().as_str() == "h")
                .unwrap();
            assert_eq!(h.content(), Some(run.channel.as_str()));
        }
        // The root is looked up among its own author's posts, so other roots
        // in the channel can't push it out of the window.
        let author = hot_root_keys("t").public_key().to_hex();
        let (lookups, reads): (Vec<&Value>, Vec<&Value>) =
            run.queries.iter().partition(|q| q.get("authors").is_some());
        assert!(!lookups.is_empty());
        for lookup in lookups {
            assert_eq!(lookup["authors"], json!([author]), "{lookup}");
        }
        // Scheduled reads fetch the hot thread itself.
        let read = json!({
            "kinds": [KIND_STREAM_MESSAGE],
            "#h": [run.channel],
            "#e": [run.root_id],
            "limit": 100,
        });
        assert!(!reads.is_empty(), "no scheduled thread reads: {report}");
        assert!(reads.iter().all(|q| **q == read), "{reads:?}");
        let ops = &report["steps"][0]["ops"];
        assert!(ops["hot_reply"]["count"].as_u64() > Some(0), "{report}");

        // Without a seeded hot root the authors post nothing and the step aborts.
        let run = hot_thread_run(false, TWO_AUTHORS).await;
        let report = run.outcome.expect_err("a missing hot root fails the run");
        assert!(run.posted.is_empty());
        let reason = report["steps"][0]["abort_reason"]
            .as_str()
            .unwrap_or_default();
        assert!(reason.starts_with("thread_fetch"), "{report}");
    }

    /// Every author is mid-reply at the stop, held there by the relay, with
    /// its next reply already due. Each must finish the reply it started and
    /// start no other: exactly one reply per author settles after the stop,
    /// and the relay receives no more.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn hot_thread_authors_start_nothing_once_the_run_stops() {
        let gate = Arc::new(Gate::default());
        let relay = hot_relay(true, Some(gate.clone())).await;
        let scenario = hot_scenario(&relay.addr, "authors = 8\nreplies_per_hour = 36000000.0");
        let net = Net::new(&scenario).await.unwrap();
        let stats = Arc::new(Stats::default());
        let (stop_tx, stop) = watch::channel(false);
        let shared = Arc::new(Shared::new(scenario, net, stats.clone(), stop));
        let authors: Vec<_> = (0..8)
            .map(|i| tokio::spawn(hot_author(shared.clone(), i)))
            .collect();
        let deadline = Duration::from_secs(10);
        let arrived = tokio::time::timeout(deadline, async {
            while gate.arrived.load(Ordering::SeqCst) < 8 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;

        stop_tx.send(true).unwrap();
        let at_stop = stats.snapshot();
        // Release before any assertion, so a failure never leaves handlers
        // blocked in the gate and the runtime unable to shut down.
        gate.release();
        arrived.expect("every author starts a reply");
        for author in authors {
            tokio::time::timeout(deadline, author)
                .await
                .expect("authors return once stopped")
                .unwrap();
        }
        let settled = ops_json(&stats.snapshot().since(&at_stop), 1.0);
        assert_eq!(settled["hot_reply"]["count"], 8, "{settled}");
        assert_eq!(relay.posted.lock().unwrap().len(), 8, "{settled}");
    }

    #[tokio::test]
    async fn root_lookups_search_the_channel_creators_posts() {
        let queries: Log<Value> = Default::default();
        let query_log = queries.clone();
        let addr = super::super::seed::tests::fake_http(move |_, _, body| {
            let filter: Value = serde_json::from_slice(body).unwrap();
            query_log.lock().unwrap().push(filter[0].clone());
            (200, "[]".to_string())
        })
        .await;
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
community_host_template = "lg-{{run}}-{{i}}.example"
connect_addr = "{addr}"
[shape]
communities = 2
channels_per_community = 17
[population]
humans = 1
agents = 0
channels_per_member = 1
[[ramp]]
users = 1.0
hold_secs = 1
"#
        ))
        .unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let (_stop, stop) = watch::channel(false);
        let shared = Shared::new(scenario, net, Arc::new(Stats::default()), stop);
        let reader = Keys::generate();
        for (community, channel) in [(0, 0), (0, 9), (1, 16)] {
            let id = identity::channel_id("t", community, channel);
            let c = &shared.communities[community];
            roots(&shared, c, &reader, &id, None).await;
            let author = channel_creator(&shared.scenario, community, channel);
            let lookup = queries.lock().unwrap().pop().unwrap();
            assert_eq!(lookup["#h"], json!([id]));
            assert_eq!(lookup["authors"], json!([author.public_key().to_hex()]));
        }
        let owner = identity::keys("t", Role::Owner, 1).public_key();
        assert_eq!(channel_creator(&shared.scenario, 1, 7).public_key(), owner);
        assert_ne!(channel_creator(&shared.scenario, 1, 16).public_key(), owner);
    }
}
