//! Drives the simulated population through the scenario's ramp.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use buzz_core::kind::{
    KIND_AGENT_OBSERVER_FRAME, KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION,
    KIND_PRESENCE_UPDATE, KIND_REACTION, KIND_STREAM_MESSAGE, KIND_TYPING_INDICATOR,
};
use buzz_core::observer::{
    encrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY,
};
use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, PublicKey, Tag, Timestamp};
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex};

use super::identity::{self, Role};
use super::net::{http_reason, Net};
use super::scenario::{Community, Scenario};
use super::seed::find_roots;
use super::stats::{abort_reason, ops_json, Op, Stats};

/// Content prefix of timed messages: `lg:<send unix micros>`.
const TIMED_PREFIX: &str = "lg:";
/// Longest an idle client blocks on its socket before rechecking the stop flag.
const MAX_IDLE: Duration = Duration::from_secs(1);
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

struct Shared {
    scenario: Scenario,
    communities: Vec<Community>,
    net: Net,
    stats: Arc<Stats>,
    stop: watch::Receiver<bool>,
    /// Seeded thread roots per channel id, resolved lazily by whichever client needs them first.
    roots: Mutex<HashMap<String, Arc<Vec<String>>>>,
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
    let shared = Arc::new(Shared {
        communities: scenario.communities(),
        scenario,
        net,
        stats: stats.clone(),
        stop,
        roots: Mutex::new(HashMap::new()),
    });
    let scenario = &shared.scenario;
    let pop = &scenario.population;
    let connect_gap = Duration::from_secs_f64(1.0 / pop.connect_rate_per_sec);

    let mut tasks = Vec::new();
    let (mut humans, mut agents) = (0usize, 0usize);
    let mut steps = Vec::new();
    let mut aborted = false;
    // End of the last reported window and start of the last judged one.
    let mut reported = stats.snapshot();
    let mut last_window = reported.clone();

    for (n, step) in scenario.ramp.iter().enumerate() {
        let want_humans = (pop.humans as f64 * step.users).ceil() as usize;
        let want_agents = (pop.agents as f64 * step.users).ceil() as usize;
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
        let ramp_secs = ramp_start.elapsed().as_secs_f64();

        let hold_start = Instant::now();
        tokio::time::sleep(Duration::from_secs(step.hold_secs)).await;
        let hold_secs = hold_start.elapsed().as_secs_f64();
        let after = stats.snapshot();
        let summaries = after.since(&before);

        // Clients only return once stopped, so a finished one panicked.
        let lost = tasks.iter().filter(|t| t.is_finished()).count();
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
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// Next arrival of a Poisson process with `per_hour` events per hour.
    fn after(&mut self, per_hour: f64) -> Option<Instant> {
        if per_hour <= 0.0 {
            return None;
        }
        let secs = -(1.0 - self.unit()).ln() * 3600.0 / per_hour;
        Some(Instant::now() + Duration::from_secs_f64(secs))
    }
}

fn now_micros() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
}

fn tag<const N: usize>(parts: [&str; N]) -> Tag {
    Tag::parse(parts).expect("static tag shape")
}

fn kind(k: u32) -> Kind {
    Kind::Custom(k as u16)
}

fn stopped(shared: &Shared) -> bool {
    *shared.stop.borrow()
}

/// Resolves once the run is stopping.
async fn stopping(shared: &Shared) {
    let mut stop = shared.stop.clone();
    let _ = stop.wait_for(|s| *s).await;
}

/// Jittered exponential backoff between failed connects or failed sessions.
struct Backoff(f64);

impl Backoff {
    const MAX_SECS: f64 = 30.0;

    fn new() -> Self {
        Self(1.0)
    }

    /// Sleeps for the next interval, returning early when the run stops.
    async fn wait(&mut self, shared: &Shared, rng: &mut Rng) {
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
struct Seen {
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
fn observe(stats: &Stats, message: &RelayMessage) -> Option<Seen> {
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

/// Publishes over the socket and records send→OK latency.
async fn publish(
    stats: &Stats,
    conn: &mut NostrWsConnection,
    op: Op,
    event: Event,
) -> Result<(), WsClientError> {
    let pending = stats.begin(op);
    match conn.send_event(event).await {
        Ok(ok) if ok.accepted => pending.ok(),
        Ok(ok) => pending.err(&ok.message),
        Err(WsClientError::Timeout) => pending.err("OK timeout"),
        Err(e) => {
            // The caller records the broken socket.
            pending.forget();
            return Err(e);
        }
    }
    Ok(())
}

/// Connects with retry until connected or stopped. Each attempt is bounded
/// by the connect deadline, which should be shorter than [`SHUTDOWN_GRACE`]
/// so an attempt under way at the stop settles before clients are cancelled.
async fn connect(
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

/// Thread roots of `channel`, cached once found. Concurrent first callers may
/// each query; a failed query is not cached, so a later reply retries it.
async fn roots(
    shared: &Shared,
    community: &Community,
    reader: &Keys,
    channel: &str,
) -> Arc<Vec<String>> {
    if let Some(found) = shared.roots.lock().await.get(channel) {
        return found.clone();
    }
    let owner = identity::keys(&shared.scenario.run_id, Role::Owner, community.index);
    let pending = shared.stats.begin(Op::HttpQuery);
    let found = match find_roots(&shared.net, community, reader, &owner, channel).await {
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
    let me = keys.public_key().to_hex();
    let channels = scenario.member_channels(community.index, global);
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

        let interval = profile.presence_interval_secs;
        let mut next_presence =
            Some(Instant::now() + Duration::from_secs_f64(interval * rng.unit()));
        let mut next_typing = rng.after(profile.typing_per_hour);
        let mut next_message = rng.after(profile.messages_per_hour);
        let mut next_reaction = rng.after(profile.reactions_per_hour);
        let session_end = (profile.mean_session_secs > 0.0)
            .then(|| rng.after(3600.0 / profile.mean_session_secs))
            .flatten();

        while ok && !stopped(&shared) {
            let due = [
                next_presence,
                next_typing,
                next_message,
                next_reaction,
                session_end,
            ]
            .into_iter()
            .flatten()
            .min();
            let wait = due
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
            let now = Instant::now();
            let result = if next_presence.is_some_and(|t| t <= now) {
                next_presence = Some(now + Duration::from_secs_f64(interval));
                let event =
                    EventBuilder::new(kind(KIND_PRESENCE_UPDATE), "online").sign_with_keys(&keys);
                send(&shared.stats, &mut conn, Op::PublishEphemeral, event).await
            } else if next_typing.is_some_and(|t| t <= now) {
                next_typing = rng.after(profile.typing_per_hour);
                let channel = &channels[rng.below(channels.len())];
                let event = EventBuilder::new(kind(KIND_TYPING_INDICATOR), "")
                    .tags([tag(["h", channel])])
                    .sign_with_keys(&keys);
                send(&shared.stats, &mut conn, Op::PublishEphemeral, event).await
            } else if next_message.is_some_and(|t| t <= now) {
                next_message = rng.after(profile.messages_per_hour);
                let channel = &channels[rng.below(channels.len())];
                let mut tags = vec![tag(["h", channel])];
                if rng.unit() < profile.thread_reply_fraction {
                    let roots = roots(&shared, community, &keys, channel).await;
                    if !roots.is_empty() {
                        tags.push(tag(["e", &roots[rng.below(roots.len())], "", "reply"]));
                    }
                }
                let event = EventBuilder::new(
                    kind(KIND_STREAM_MESSAGE),
                    format!("{TIMED_PREFIX}{} {me}", now_micros()),
                )
                .tags(tags)
                .sign_with_keys(&keys);
                send(&shared.stats, &mut conn, Op::PublishMessage, event).await
            } else if next_reaction.is_some_and(|t| t <= now) {
                next_reaction = rng.after(profile.reactions_per_hour);
                // React to each seen message at most once; the relay rejects duplicates.
                match last_message.take() {
                    Some(target) => {
                        let event = EventBuilder::new(kind(KIND_REACTION), "+")
                            .tags([tag(["h", &target.channel]), tag(["e", &target.id])])
                            .sign_with_keys(&keys);
                        send(&shared.stats, &mut conn, Op::PublishMessage, event).await
                    }
                    None => Ok(()),
                }
            } else if session_end.is_some_and(|t| t <= now) {
                break;
            } else {
                Ok(())
            };
            if let Err(e) = result {
                shared
                    .stats
                    .err(Op::WsConnect, &format!("disconnected: {e}"));
                ok = false;
                break;
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

async fn send(
    stats: &Stats,
    conn: &mut NostrWsConnection,
    op: Op,
    event: Result<Event, nostr::event::builder::Error>,
) -> Result<(), WsClientError> {
    match event {
        Ok(event) => publish(stats, conn, op, event).await,
        Err(e) => {
            stats.err(op, &format!("sign: {e}"));
            Ok(())
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

/// An agent client: NIP-OA authenticated socket, HTTP `/query` polling,
/// HTTP `/events` posts and encrypted observer frames to its owner.
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
    let channels = scenario.member_channels(community.index, global);
    let me = keys.public_key().to_hex();
    let mut rng = Rng::new(u64::MAX - global as u64);
    let mut backoff = Backoff::new();
    let mut last_message = None;

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
        let mut ok = match send_req(
            &mut conn,
            "mentions",
            mentions_filter(keys.public_key(), since),
        )
        .await
        {
            Ok(()) => true,
            Err(e) => {
                shared
                    .stats
                    .err(Op::WsConnect, &format!("disconnected: {e}"));
                false
            }
        };
        let query_per_hour = 3600.0 / profile.query_interval_secs;
        let mut next_query = rng.after(query_per_hour);
        let mut next_post = rng.after(profile.events_per_hour);
        let mut next_frame = rng.after(profile.observer_frames_per_hour);

        while ok && !stopped(&shared) {
            let due = [next_query, next_post, next_frame]
                .into_iter()
                .flatten()
                .min();
            let wait = due
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(MAX_IDLE)
                .min(MAX_IDLE);
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
            let now = Instant::now();
            let channel = &channels[rng.below(channels.len())];
            if next_query.is_some_and(|t| t <= now) {
                next_query = rng.after(query_per_hour);
                let window = Timestamp::now().as_secs().saturating_sub(3600);
                let filters = json!([
                    { "kinds": [KIND_STREAM_MESSAGE], "#h": [channel], "since": window, "limit": 20 },
                    { "#p": [me], "since": window, "limit": 20 },
                ]);
                http(
                    &shared,
                    Op::HttpQuery,
                    &keys,
                    community,
                    "/query",
                    &filters,
                    &auth,
                )
                .await;
            } else if next_post.is_some_and(|t| t <= now) {
                next_post = rng.after(profile.events_per_hour);
                let event = EventBuilder::new(
                    kind(KIND_STREAM_MESSAGE),
                    format!("{TIMED_PREFIX}{} {me}", now_micros()),
                )
                .tags([tag(["h", channel])])
                .sign_with_keys(&keys);
                match event {
                    Ok(event) => {
                        let pending = shared.stats.begin(Op::HttpEvents);
                        match shared
                            .net
                            .submit(&keys, community, &event, Some(&auth))
                            .await
                        {
                            Ok(()) => pending.ok(),
                            Err(e) => pending.err(&e.to_string()),
                        }
                    }
                    Err(e) => shared.stats.err(Op::HttpEvents, &format!("sign: {e}")),
                }
            } else if next_frame.is_some_and(|t| t <= now) {
                next_frame = rng.after(profile.observer_frames_per_hour);
                let payload = json!({ "type": "loadgen", "sent_micros": now_micros().to_string() });
                let event = encrypt_observer_payload(&keys, &owner.public_key(), &payload)
                    .map_err(|e| e.to_string())
                    .and_then(|content| {
                        EventBuilder::new(kind(KIND_AGENT_OBSERVER_FRAME), content)
                            .tags([
                                tag(["p", &owner.public_key().to_hex()]),
                                tag([OBSERVER_AGENT_TAG, &me]),
                                tag([OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]),
                            ])
                            .sign_with_keys(&keys)
                            .map_err(|e| e.to_string())
                    });
                let result = match event {
                    Ok(event) => {
                        publish(&shared.stats, &mut conn, Op::PublishEphemeral, event).await
                    }
                    Err(e) => {
                        shared
                            .stats
                            .err(Op::PublishEphemeral, &format!("observer: {e}"));
                        Ok(())
                    }
                };
                if let Err(e) = result {
                    shared
                        .stats
                        .err(Op::WsConnect, &format!("disconnected: {e}"));
                    ok = false;
                }
            }
        }
        let _ = conn.disconnect().await;
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

async fn http(
    shared: &Shared,
    op: Op,
    keys: &Keys,
    community: &Community,
    path: &str,
    body: &Value,
    auth: &Tag,
) {
    let pending = shared.stats.begin(op);
    let body = serde_json::to_vec(body).expect("serialize JSON body");
    match shared
        .net
        .post(keys, &community.http_url, path, body, Some(auth))
        .await
    {
        Ok((status, _)) if status.is_success() => pending.ok(),
        Ok((status, text)) => pending.err(&http_reason(status, &text)),
        Err(e) => pending.err(&e.to_string()),
    }
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
    }

    impl FakeRelay {
        const PROMPT: Self = Self {
            challenge_delay: Duration::ZERO,
            ok_delay: Some(Duration::ZERO),
            history_delay: Duration::ZERO,
            slow_ok: None,
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
                                let Some(mut delay) = relay.ok_delay else {
                                    continue;
                                };
                                if let Some((after, slow)) = relay.slow_ok {
                                    if opened.elapsed() >= after {
                                        delay = slow;
                                    }
                                }
                                tokio::time::sleep(delay).await;
                                json!(["OK", frame[1]["id"], true, ""])
                            }
                            _ => continue,
                        };
                        if ws
                            .send(Message::Text(reply.to_string().into()))
                            .await
                            .is_err()
                        {
                            return;
                        }
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
}
