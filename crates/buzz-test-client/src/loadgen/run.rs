//! Drives the simulated population through the scenario's ramp.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use buzz_core::kind::{
    KIND_AGENT_OBSERVER_FRAME, KIND_PRESENCE_UPDATE, KIND_REACTION, KIND_STREAM_MESSAGE,
    KIND_TYPING_INDICATOR,
};
use buzz_core::observer::{
    encrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY,
};
use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, PublicKey, Tag, Timestamp};
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex};

use super::identity::{self, Role};
use super::net::{truncate, Net};
use super::scenario::{Community, Scenario};
use super::seed::find_roots;
use super::stats::{ops_json, Op, Stats};

/// Content prefix of timed messages: `lg:<send unix micros>`.
const TIMED_PREFIX: &str = "lg:";
/// Longest an idle client blocks on its socket before rechecking the stop flag.
const MAX_IDLE: Duration = Duration::from_secs(1);

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

    for (n, step) in scenario.ramp.iter().enumerate() {
        let want_humans = (pop.humans as f64 * step.users).ceil() as usize;
        let want_agents = (pop.agents as f64 * step.users).ceil() as usize;
        // The window covers the ramp too, so connect/auth and history
        // failures from new clients count toward the abort check.
        let before = stats.snapshot();
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

        let worst = summaries
            .iter()
            .filter(|(op, s)| op.gates_abort() && s.count + s.errors > 0)
            .fold((0.0f64, 0.0f64), |(rate, p99), (_, s)| {
                (rate.max(s.error_rate()), p99.max(s.p99_ms))
            });
        let abort = worst.0 > scenario.abort.max_error_rate || worst.1 > scenario.abort.max_p99_ms;
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
            "aborted": abort,
        });
        eprintln!("{report}");
        steps.push(report);
        if abort {
            aborted = true;
            break;
        }
    }

    let _ = stop_tx.send(true);
    for task in tasks {
        let _ = task.await;
    }
    let report = json!({ "run_id": shared.scenario.run_id, "steps": steps });
    if aborted {
        Err(report)
    } else {
        Ok(report)
    }
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
    let t0 = Instant::now();
    match conn.send_event(event).await {
        Ok(ok) if ok.accepted => stats.ok(op, t0.elapsed()),
        Ok(ok) => stats.err(op, &ok.message),
        Err(WsClientError::Timeout) => stats.err(op, "OK timeout"),
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Connects with retry until connected or stopped.
async fn connect(
    shared: &Shared,
    community: &Community,
    keys: &Keys,
    auth: Option<&Tag>,
    rng: &mut Rng,
) -> Option<NostrWsConnection> {
    let mut backoff = 1.0;
    while !stopped(shared) {
        let t0 = Instant::now();
        match shared.net.connect(community, keys, auth).await {
            Ok(conn) => {
                shared.stats.ok(Op::WsConnect, t0.elapsed());
                return Some(conn);
            }
            Err(e) => shared.stats.err(Op::WsConnect, &e.to_string()),
        }
        let wait = backoff * (0.5 + rng.unit());
        tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        backoff = (backoff * 2.0f64).min(30.0);
    }
    None
}

/// Thread roots of `channel`, queried once per shard.
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
    let t0 = Instant::now();
    let found = match find_roots(&shared.net, community, reader, &owner, channel).await {
        Ok(found) => {
            shared.stats.ok(Op::HttpQuery, t0.elapsed());
            found
        }
        Err(e) => {
            shared.stats.err(Op::HttpQuery, &e.to_string());
            Vec::new()
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
    let mut last_message: Option<Seen> = None;

    while let Some(mut conn) = connect(&shared, community, &keys, None, &mut rng).await {
        let since = Timestamp::now();
        let subs = [
            (
                "live",
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
                "mentions",
                Filter::new().pubkey(keys.public_key()).since(since),
            ),
            (
                "typing",
                Filter::new().kind(kind(KIND_TYPING_INDICATOR)).custom_tags(
                    nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                    channels.clone(),
                ),
            ),
            (
                "presence",
                Filter::new()
                    .kind(kind(KIND_PRESENCE_UPDATE))
                    .authors(peers.clone()),
            ),
        ];
        let mut ok = true;
        for (id, filter) in subs {
            if send_req(&mut conn, id, filter).await.is_err() {
                ok = false;
                break;
            }
        }
        if ok && profile.history_limit > 0 {
            for (n, channel) in channels.iter().enumerate() {
                let id = format!("history-{n}");
                let filter = Filter::new()
                    .kind(kind(KIND_STREAM_MESSAGE))
                    .custom_tag(
                        nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                        channel.clone(),
                    )
                    .limit(profile.history_limit);
                if history(&shared.stats, &mut conn, &id, filter, &mut last_message)
                    .await
                    .is_err()
                {
                    ok = false;
                    break;
                }
            }
        }

        let interval = profile.presence_interval_secs.max(1.0);
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
                    if let Some(target) = observe(&shared.stats, &message) {
                        last_message = Some(target);
                    }
                }
                Err(WsClientError::Timeout) => {}
                Err(e) => {
                    shared
                        .stats
                        .err(Op::WsConnect, &format!("disconnected: {e}"));
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
                break;
            }
        }
        let _ = conn.disconnect().await;
        if stopped(&shared) {
            return;
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

/// Fetches history until EOSE, records the latency and closes the subscription.
async fn history(
    stats: &Stats,
    conn: &mut NostrWsConnection,
    id: &str,
    filter: Filter,
    last_message: &mut Option<Seen>,
) -> Result<(), WsClientError> {
    let t0 = Instant::now();
    send_req(conn, id, filter).await?;
    let deadline = t0 + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match conn.next_event(remaining).await {
            Ok(RelayMessage::Eose { subscription_id }) if subscription_id == id => {
                stats.ok(Op::History, t0.elapsed());
                break;
            }
            Ok(RelayMessage::Closed {
                subscription_id,
                message,
            }) if subscription_id == id => {
                stats.err(Op::History, &message);
                return Ok(());
            }
            Ok(message) => {
                if let Some(target) = seen(&message) {
                    *last_message = Some(target);
                }
            }
            Err(WsClientError::Timeout) => {
                stats.err(Op::History, "EOSE timeout");
                break;
            }
            Err(e) => return Err(e),
        }
    }
    conn.send_raw(&json!(["CLOSE", id])).await
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

    while let Some(mut conn) = connect(&shared, community, &keys, Some(&auth), &mut rng).await {
        let since = Timestamp::now();
        let mut ok = send_req(
            &mut conn,
            "mentions",
            Filter::new().pubkey(keys.public_key()).since(since),
        )
        .await
        .is_ok();
        let query_per_hour = 3600.0 / profile.query_interval_secs.max(0.1);
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
                    observe(&shared.stats, &message);
                }
                Err(WsClientError::Timeout) => {}
                Err(e) => {
                    shared
                        .stats
                        .err(Op::WsConnect, &format!("disconnected: {e}"));
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
                        let t0 = Instant::now();
                        match shared
                            .net
                            .submit(&keys, community, &event, Some(&auth))
                            .await
                        {
                            Ok(()) => shared.stats.ok(Op::HttpEvents, t0.elapsed()),
                            Err(e) => shared.stats.err(Op::HttpEvents, &e.to_string()),
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
    let t0 = Instant::now();
    let body = serde_json::to_vec(body).expect("serialize JSON body");
    match shared
        .net
        .post(keys, &community.http_url, path, body, Some(auth))
        .await
    {
        Ok((status, _)) if status.is_success() => shared.stats.ok(op, t0.elapsed()),
        Ok((status, text)) => shared
            .stats
            .err(op, &format!("HTTP {status}: {}", truncate(&text))),
        Err(e) => shared.stats.err(op, &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
