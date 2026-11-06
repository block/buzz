//! A desktop app client. It makes the relay requests the desktop app makes,
//! modeled on the app's live subscription manager (the desktop app's
//! `live.ts`) and its session store. It follows the request pattern from
//! before (`legacy`) or after (`current`) the app's client volume fixes.
//!
//! The planning functions here say what a connection sends: the REQs it
//! opens, the HTTP reads it makes, and what a recovery tick or a renewal
//! adds. [`desktop`] executes those plans over one socket, plus one HTTP read
//! task per client. See `perf/LOADGEN.md` for the model and what it omits.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use buzz_core::kind::KIND_PRESENCE_UPDATE;
use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};
use futures_util::stream::{self, StreamExt};
use nostr::Keys;
use serde_json::{json, Value};

use super::identity::{self, Role};
use super::net::http_reason;
use super::run::{
    bounded, connect, observe, roots, stopped, Activity, Backoff, Directory, Rng, Seen, Shared,
    Step, MAX_IDLE,
};
use super::scenario::{DesktopMode, WideSplit};
use super::stats::{Op, Pending};

/// Kinds the app subscribes to for every joined channel.
const CHANNEL_KINDS: [u32; 20] = [
    9, 40002, 40008, 45001, 45003, 40099, 40100, 40003, 5, 9005, 7, 39000, 39002, 39005, 20002,
    45010, 48100, 48101, 48102, 48103,
];
/// Kinds of a channel window page: channel rows without notices.
const WINDOW_KINDS: [u32; 5] = [9, 40002, 40008, 48100, 48103];
/// Per-filter limit of a replaying channel filter.
const REPLAY_LIMIT: usize = 500;
/// Channels per replay REQ (one filter each), and per `legacy` batch REQ.
const REPLAY_BATCH: usize = 10;
/// Channel ids in one `current` live-only batch filter.
const LIVE_BATCH: usize = 2048;
/// REQs awaiting EOSE at once while a connection establishes.
const SETUP_CONCURRENCY: usize = 4;
/// How long a REQ may wait for its EOSE, and a silence probe for any frame.
const REQ_TIMEOUT: Duration = Duration::from_secs(10);
/// Frames handled in one pass before deadlines are judged.
const DRAIN_FRAMES: usize = 256;
/// A resumed subscription starts this long before its cursor.
const RESUME_LAG_SECS: u64 = 960;
/// A first subscription starts this long before now.
const STARTUP_FLOOR_SECS: u64 = 300;
/// HTTP reads in flight at once per client.
const HTTP_CONCURRENCY: usize = 6;
/// Deadline of one HTTP read.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Page size of roster, metadata and profile reads.
const PAGE: usize = 500;
/// Most roster pages one read follows.
const MAX_ROSTER_PAGES: usize = 20;
/// Profiles the client holds.
const PROFILE_CAP: usize = 1024;
/// Status authors read.
const STATUS_AUTHORS: usize = 100;
/// Presence authors watched: the app's cap.
const PRESENCE_AUTHORS: usize = 256;
/// Channels per unread repair read.
const UNREAD_CHANNELS: usize = 128;
/// Events per channel window page.
const WINDOW_PAGE: usize = 20;
/// Older window pages a catch-up reads after the head, at most.
const CATCH_UP_PAGES: usize = 5;
/// The app's custom emoji set.
const EMOJI_SET: &str = "buzz:custom-emoji";

/// What a subscription carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// The global profiles route: profiles, statuses and custom emoji.
    Profiles,
    /// The global membership route: grants, read state and reminders.
    Membership,
    /// The global live-only presence route, which replays nothing.
    Presence,
    /// A channel batch, which stays open.
    Channels,
    /// A replay of channels behind a live-only batch, closed at its EOSE.
    Replay,
}

impl Route {
    /// Whether the route takes part in [`Establishment`]. A replay serves
    /// its batch, and presence replays nothing, so neither is resumed,
    /// carried nor recovered, and neither cuts a renewal's carry.
    fn establishes(self) -> bool {
        !matches!(self, Route::Presence | Route::Replay)
    }
}

/// One REQ frame.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Req {
    pub(super) id: String,
    pub(super) filters: Vec<Value>,
    pub(super) route: Route,
}

impl Req {
    fn frame(&self) -> Value {
        let mut frame = vec![json!("REQ"), json!(self.id)];
        frame.extend(self.filters.iter().cloned());
        Value::Array(frame)
    }

    /// A route whose replay counts as cut whatever it sends: a live-only
    /// channel batch over more than ten channels replays nothing itself.
    fn cut_by_design(&self) -> bool {
        let live_only =
            self.route == Route::Channels && self.filters.iter().all(|f| f["limit"] == json!(0));
        let scope: usize = self
            .filters
            .iter()
            .map(|f| f["#h"].as_array().map_or(0, Vec::len))
            .sum();
        live_only && scope > REPLAY_BATCH
    }
}

/// Whether a route's replay was cut at its EOSE: it replayed the app's limit
/// of events across its filters, or it was cut by design.
pub(super) fn limited(cut_by_design: bool, events: usize) -> bool {
    cut_by_design || events >= REPLAY_LIMIT
}

fn channel_filter(ids: &[String], since: u64, limit: usize) -> Value {
    json!({"kinds": CHANNEL_KINDS, "#h": ids, "since": since, "limit": limit})
}

/// The global subscriptions: profiles, membership and, while the client
/// watches anyone's presence, a live-only presence route over those
/// `presence` authors.
fn global_reqs(viewer: &str, presence: &[String], since: u64) -> Vec<Req> {
    let limit = REPLAY_LIMIT;
    let mut reqs = vec![
        Req {
            id: "profiles".into(),
            filters: vec![
                json!({"kinds": [0, 10100, 30177], "since": since, "limit": limit}),
                json!({"kinds": [30315], "#d": ["general"], "since": since, "limit": limit}),
                json!({"kinds": [30030], "#d": [EMOJI_SET], "since": since, "limit": limit}),
            ],
            route: Route::Profiles,
        },
        Req {
            id: "membership".into(),
            filters: vec![
                json!({"kinds": [44100, 44101], "#p": [viewer], "since": since, "limit": limit}),
                json!({"kinds": [44100, 44101], "#p": [viewer], "#t": ["system"], "since": since, "limit": limit}),
                json!({"kinds": [30078], "authors": [viewer], "#t": ["read-state"], "since": since, "limit": limit}),
                json!({"kinds": [30300], "authors": [viewer], "since": since, "limit": limit}),
            ],
            route: Route::Membership,
        },
    ];
    if !presence.is_empty() {
        reqs.push(Req {
            id: "presence".into(),
            filters: vec![
                json!({"kinds": [KIND_PRESENCE_UPDATE], "authors": presence, "since": since, "limit": 0}),
            ],
            route: Route::Presence,
        });
    }
    reqs
}

/// Channel batch subscriptions, which stay open. Up to 10 channels share
/// one REQ with a filter per channel. Beyond that, `legacy` sends a REQ per
/// 10 channels, and `current` sends one live-only filter (limit 0) per 2048
/// channels and replays separately ([`replay_reqs`]). `limit` applies to
/// per-channel filters: [`REPLAY_LIMIT`] on connect, 0 for a tick's
/// replacement.
pub(super) fn batch_reqs(
    mode: DesktopMode,
    channels: &[String],
    since: u64,
    limit: usize,
    generation: u64,
) -> Vec<Req> {
    let id = |i: usize| format!("batch-{generation}-{i}");
    if mode == DesktopMode::Legacy || channels.len() <= REPLAY_BATCH {
        channels
            .chunks(REPLAY_BATCH)
            .enumerate()
            .map(|(i, chunk)| Req {
                id: id(i),
                filters: chunk
                    .iter()
                    .map(|c| channel_filter(std::slice::from_ref(c), since, limit))
                    .collect(),
                route: Route::Channels,
            })
            .collect()
    } else {
        channels
            .chunks(LIVE_BATCH)
            .enumerate()
            .map(|(i, chunk)| Req {
                id: id(i),
                filters: vec![channel_filter(chunk, since, 0)],
                route: Route::Channels,
            })
            .collect()
    }
}

/// `current` mode's replay of more than 10 channels behind live-only
/// batches: a REQ per 10 channels, one filter each, closed at its EOSE.
pub(super) fn replay_reqs(
    mode: DesktopMode,
    channels: &[String],
    since: u64,
    generation: u64,
) -> Vec<Req> {
    if mode == DesktopMode::Legacy || channels.len() <= REPLAY_BATCH {
        return Vec::new();
    }
    channels
        .chunks(REPLAY_BATCH)
        .enumerate()
        .map(|(i, chunk)| Req {
            id: format!("replay-{generation}-{i}"),
            filters: chunk
                .iter()
                .map(|c| channel_filter(std::slice::from_ref(c), since, REPLAY_LIMIT))
                .collect(),
            route: Route::Replay,
        })
        .collect()
}

/// Every REQ a connection sends while it establishes, in order.
pub(super) fn connect_reqs(
    mode: DesktopMode,
    viewer: &str,
    presence: &[String],
    channels: &[String],
    since: u64,
    generation: u64,
) -> Vec<Req> {
    let mut reqs = global_reqs(viewer, presence, since);
    reqs.extend(batch_reqs(mode, channels, since, REPLAY_LIMIT, generation));
    reqs.extend(replay_reqs(mode, channels, since, generation));
    reqs
}

/// Why a connection is being established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Situation {
    /// The client's first connection.
    Cold,
    /// Any other reconnect: routes resume from the last connection.
    Reconnect,
    /// A `current` identity renewal: routes are carried over, and none
    /// establishes unless one of them replays a cut window.
    Renewal,
}

/// What an identity renewal is in `mode`. Before the fixes it was an
/// ordinary reconnect.
pub(super) fn renewal(mode: DesktopMode) -> Situation {
    match mode {
        DesktopMode::Legacy => Situation::Reconnect,
        DesktopMode::Current => Situation::Renewal,
    }
}

/// One HTTP read job. A job may make several requests: pages, or chunks of
/// ids. `Window(i)` and `Thread(i)` are the client's i-th open window and
/// thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fetch {
    Nip11,
    Roster,
    Metadata,
    Sidebar,
    Profiles,
    Statuses,
    Emoji,
    /// Unread repair. With `snapshot`, a full repair: the read-state
    /// snapshot first, then the repair. Without, the membership route's
    /// replay covered read state and only the repair runs.
    Unread {
        snapshot: bool,
    },
    Reminders,
    Window(usize),
    Thread(usize),
}

const FULL_REPAIR: Fetch = Fetch::Unread { snapshot: true };
const COVERED_REPAIR: Fetch = Fetch::Unread { snapshot: false };

impl Fetch {
    /// Whether this job, queued or running, already does `other`'s work.
    fn covers(self, other: Fetch) -> bool {
        self == other || (self == FULL_REPAIR && other == COVERED_REPAIR)
    }
}

fn channel_recovery(windows: usize, threads: usize) -> impl Iterator<Item = Fetch> {
    (0..windows)
        .map(Fetch::Window)
        .chain((0..threads).map(Fetch::Thread))
}

/// The reads a connection makes as soon as it opens. `legacy` reads
/// everything here: all of it cold, all but channel names and sidebar
/// preferences on a reconnect, which it treats a renewal as. `current`
/// reads only what no route covers; the rest follows each route's
/// establishment ([`establish_fetches`]).
pub(super) fn connect_fetches(
    mode: DesktopMode,
    situation: Situation,
    windows: usize,
    threads: usize,
) -> Vec<Fetch> {
    let cold = situation == Situation::Cold;
    match (mode, situation) {
        (DesktopMode::Legacy, _) => {
            let mut reads = vec![Fetch::Nip11, Fetch::Roster];
            if cold {
                reads.extend([Fetch::Metadata, Fetch::Sidebar]);
            }
            reads.extend([
                Fetch::Profiles,
                Fetch::Statuses,
                Fetch::Emoji,
                FULL_REPAIR,
                Fetch::Reminders,
            ]);
            reads.extend(channel_recovery(windows, threads));
            reads
        }
        (DesktopMode::Current, Situation::Cold) => {
            vec![Fetch::Nip11, Fetch::Metadata, Fetch::Sidebar]
        }
        (DesktopMode::Current, Situation::Reconnect) => vec![Fetch::Nip11],
        (DesktopMode::Current, Situation::Renewal) => Vec::new(),
    }
}

/// Reads still owed after a failed or unfinished one, as the app's stores
/// track them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Owed {
    pub(super) roster: bool,
    pub(super) profiles: bool,
    /// The read-state snapshot, owed until a snapshot read succeeds. Only a
    /// full repair reads it, so a covered repair never settles it.
    pub(super) snapshot: bool,
    /// The unread repair of the channels.
    pub(super) unread: bool,
}

/// The reads `current` makes when `route` establishes. A `continuous`
/// global route replayed every change since the last connection, so it
/// skips its full read unless one is still owed: profiles resume their
/// statuses and emoji, and membership resumes read state and reminders and
/// only repairs unreads, unless the read-state snapshot is owed, which takes
/// a full repair. A channel route recovers the open windows and threads.
pub(super) fn establish_fetches(
    route: Route,
    continuous: bool,
    owed: Owed,
    windows: usize,
    threads: usize,
) -> Vec<Fetch> {
    match route {
        Route::Profiles if !continuous => vec![Fetch::Profiles, Fetch::Statuses, Fetch::Emoji],
        Route::Profiles if owed.profiles => vec![Fetch::Profiles],
        Route::Profiles => Vec::new(),
        Route::Membership => {
            let mut reads = Vec::new();
            if !continuous || owed.roster {
                reads.push(Fetch::Roster);
            }
            if continuous && !owed.snapshot {
                reads.push(COVERED_REPAIR);
            } else if continuous {
                reads.push(FULL_REPAIR);
            } else {
                reads.extend([FULL_REPAIR, Fetch::Reminders]);
            }
            reads
        }
        Route::Channels => channel_recovery(windows, threads).collect(),
        Route::Presence | Route::Replay => Vec::new(),
    }
}

/// Which of a connection's routes establish, and when. Each route kind
/// establishes once per connection. A route resumes only if the previous
/// connection had it caught up; one that had not counts as cut. On a
/// renewal, routes are carried: one whose replay kept continuity establishes
/// nothing, until any route's replay is cut, which establishes every route
/// again.
#[derive(Debug)]
pub(super) struct Establishment {
    /// Routes the previous connection had caught up.
    resumed: Vec<Route>,
    carried: bool,
    /// Routes that reached EOSE while carried, with their continuity.
    held: Vec<(Route, bool)>,
    done: Vec<Route>,
    /// Routes established only because their EOSE never came: recovered,
    /// but not live, so not caught up.
    timed_out: Vec<Route>,
}

impl Establishment {
    /// A connection's establishment, after one that had `caught_up` routes
    /// (see [`Establishment::caught_up`]). A cold start resumes nothing.
    pub(super) fn new(situation: Situation, caught_up: &[Route]) -> Self {
        let resumed = match situation {
            Situation::Cold => Vec::new(),
            _ => caught_up.to_vec(),
        };
        Self {
            resumed,
            carried: situation == Situation::Renewal,
            held: Vec::new(),
            done: Vec::new(),
            timed_out: Vec::new(),
        }
    }

    /// `route` reached its EOSE, its replay `cut` or not. Returns the routes
    /// that establish now, each with its continuity.
    pub(super) fn eose(&mut self, route: Route, cut: bool) -> Vec<(Route, bool)> {
        if !route.establishes() {
            return Vec::new();
        }
        let cut = cut || !self.resumed.contains(&route);
        let mut now = Vec::new();
        if self.carried {
            if !cut {
                self.held.push((route, true));
                return Vec::new();
            }
            self.carried = false;
            now = std::mem::take(&mut self.held);
        }
        now.push((route, !cut));
        now.retain(|(route, _)| {
            let first = !self.done.contains(route);
            if first {
                self.done.push(*route);
            }
            first
        });
        now
    }

    /// `route` missed its EOSE deadline. It is recovered as if its replay
    /// were cut, but only an EOSE proves a route live: it is not caught up,
    /// so the next connection cannot resume or carry it.
    pub(super) fn timeout(&mut self, route: Route) -> Vec<(Route, bool)> {
        if route.establishes() {
            self.timed_out.push(route);
        }
        self.eose(route, true)
    }

    /// The routes this connection has caught up: established at their EOSE,
    /// or carried with their continuity kept.
    pub(super) fn caught_up(&self) -> Vec<Route> {
        let held = self.held.iter().map(|(route, _)| *route);
        let done = self.done.iter().copied();
        done.filter(|route| !self.timed_out.contains(route))
            .chain(held)
            .collect()
    }
}

/// What a recovery tick re-sends or rereads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Due {
    /// Replace every channel batch with a live-only copy.
    pub(super) batches: bool,
    pub(super) roster: bool,
    /// A full unread repair.
    pub(super) unread: bool,
}

/// How long since each tick item was last done. `None`: never, or a
/// replacement that never established.
#[derive(Clone, Copy, Debug)]
pub(super) struct Clocks {
    /// Since the batches were last sent.
    pub(super) batches: Option<Duration>,
    /// Since the roster was last read in full.
    pub(super) roster: Option<Duration>,
    /// Since a full unread repair was last asked for.
    pub(super) unread: Option<Duration>,
}

/// A tick's work. `legacy` does all of it every tick. `current` waits for
/// `backstop` since each was last done, and retries owed reads at once.
pub(super) fn tick(mode: DesktopMode, backstop: Duration, clocks: Clocks, owed: Owed) -> Due {
    let stale = |since: Option<Duration>| since.is_none_or(|d| d >= backstop);
    match mode {
        DesktopMode::Legacy => Due {
            batches: true,
            roster: true,
            unread: true,
        },
        DesktopMode::Current => Due {
            batches: stale(clocks.batches),
            roster: owed.roster || stale(clocks.roster),
            unread: owed.unread || owed.snapshot || stale(clocks.unread),
        },
    }
}

/// The HTTP reads a tick makes.
pub(super) fn tick_fetches(due: Due) -> Vec<Fetch> {
    let mut reads = Vec::new();
    if due.roster {
        reads.push(Fetch::Roster);
    }
    if due.unread {
        reads.push(FULL_REPAIR);
    }
    reads
}

/// A window page's position: `(created_at, id)` of the row the next page
/// starts after.
pub(super) type Cursor = (u64, String);

/// The relay's verdict on a window page: the cursor of the next page, or
/// `None` once history is exhausted. The relay answers every window with
/// one kind 39006 bounds event for this channel and cursor, signed by
/// `relay` (when known); rows, aux events and summaries say nothing about
/// exhaustion. Anything else is an error, as in the app.
pub(super) fn window_bounds(
    events: &[Value],
    channel: &str,
    cursor: Option<&Cursor>,
    relay: Option<&str>,
) -> Result<Option<Cursor>, String> {
    let suffix = match cursor {
        Some((until, id)) => format!("{channel}:{until}:{id}"),
        None => format!("{channel}:head"),
    };
    let bounds: Vec<&Value> = events
        .iter()
        .filter(|e| {
            e["kind"] == json!(39006)
                && relay.is_none_or(|r| e["pubkey"] == json!(r))
                && has_tag(e, "h", channel)
                && has_tag(e, "d", &suffix)
        })
        .collect();
    let [bound] = bounds[..] else {
        return Err(format!("{} window bounds for {suffix}", bounds.len()));
    };
    let body: Value = serde_json::from_str(bound["content"].as_str().unwrap_or_default())
        .map_err(|e| format!("window bounds: {e}"))?;
    let has_more = body["has_more"]
        .as_bool()
        .ok_or("window bounds omit has_more")?;
    let next = match &body["next_cursor"] {
        Value::Null => None,
        next => {
            let until = next["created_at"].as_u64();
            let id = next["id"].as_str().filter(|id| id.len() == 64);
            let (Some(until), Some(id)) = (until, id) else {
                return Err("window cursor is malformed".into());
            };
            Some((until, id.to_string()))
        }
    };
    if has_more != next.is_some() {
        return Err("window bounds contradict their cursor".into());
    }
    // History runs newest first, then by ascending id at equal times.
    if let (Some((until, id)), Some((next_until, next_id))) = (cursor, &next) {
        if next_until > until || (next_until == until && next_id <= id) {
            return Err("window cursor does not advance".into());
        }
    }
    Ok(next)
}

fn has_tag(event: &Value, name: &str, value: &str) -> bool {
    event["tags"].as_array().is_some_and(|tags| {
        tags.iter()
            .any(|t| t[0] == json!(name) && t[1] == json!(value))
    })
}

/// The last row of a page in the relay's order (newest first, then by
/// ascending id): the cursor of the page after it.
pub(super) fn last_row(page: &[Value]) -> Option<(u64, &str)> {
    page.iter()
        .filter_map(|e| Some((e["created_at"].as_u64()?, e["id"].as_str()?)))
        .min_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(a.1)))
}

/// Points `filter` at the page after `cursor`. The relay takes `until` and
/// `before_id` together.
fn page_before(filter: &mut Value, (until, id): (u64, &str)) {
    filter["until"] = json!(until);
    filter["before_id"] = json!(id);
}

/// The events of a `/query` response: an array, or the object a read-state
/// snapshot answers with.
fn events(text: &str) -> serde_json::Result<Vec<Value>> {
    match serde_json::from_str(text)? {
        Value::Object(mut body) => {
            serde_json::from_value(body.remove("events").unwrap_or_default())
        }
        other => serde_json::from_value(other),
    }
}

/// Where a resumed subscription starts: [`RESUME_LAG_SECS`] before the
/// last connection dropped, but not before the client's own start floor.
fn resume_since(started: u64, cursor: Option<u64>, now: u64) -> u64 {
    match cursor {
        Some(cursor) => cursor
            .saturating_sub(RESUME_LAG_SECS)
            .max(started.saturating_sub(STARTUP_FLOOR_SECS)),
        None => now.saturating_sub(STARTUP_FLOOR_SECS),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Up to [`PROFILE_CAP`] other humans in `channels` of `community`, in
/// channel order: the profiles the client holds.
fn co_members(
    directory: &Directory,
    community: usize,
    me: usize,
    channels: &[usize],
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &channel in channels {
        for member in &directory.members[community][channel] {
            if member.role == Role::Human && member.index != me && seen.insert(member.index) {
                out.push(directory.humans[member.index].clone());
                if out.len() == PROFILE_CAP {
                    return out;
                }
            }
        }
    }
    out
}

/// What the client's reads have settled: what is still owed and when the
/// roster was last read in full. Its read tasks write it; ticks and route
/// establishments read it.
#[derive(Debug)]
struct Ledger {
    owed: Owed,
    roster_read: Option<Instant>,
}

/// What a client's reads use. Its sessions and read tasks share it.
struct Reader {
    shared: Arc<Shared>,
    community: usize,
    keys: Keys,
    viewer: String,
    channels: Vec<String>,
    profiles: Vec<String>,
    windows: usize,
    /// The relay's own pubkey from NIP-11 `self`, the author of notices and
    /// window bounds.
    relay: std::sync::Mutex<Option<String>>,
    /// When the last unread repair started (Unix seconds).
    unread_mark: std::sync::Mutex<Option<u64>>,
    ledger: std::sync::Mutex<Ledger>,
}

impl Reader {
    /// One `POST /query`, timed as `op`. Returns the events, or `None` once
    /// it failed (recorded) or the run is stopping (not started).
    async fn query(&self, op: Op, filters: Value) -> Option<Vec<Value>> {
        self.query_with(op, filters, |events| Ok(events.to_vec()))
            .await
    }

    /// [`Reader::query`], where the response only succeeds once `check`
    /// accepts its events.
    async fn query_with<T>(
        &self,
        op: Op,
        filters: Value,
        check: impl FnOnce(&[Value]) -> Result<T, String>,
    ) -> Option<T> {
        if stopped(&self.shared) {
            return None;
        }
        let shared = &self.shared;
        let base = &shared.communities[self.community].http_url;
        let body = filters.to_string().into_bytes();
        let pending = shared.stats.begin(op);
        let Some(response) = bounded(
            shared,
            HTTP_TIMEOUT,
            shared.net.post(&self.keys, base, "/query", body, None),
        )
        .await
        else {
            pending.forget();
            return None;
        };
        match response {
            Err(_) => pending.err("read timeout"),
            Ok(Err(e)) => pending.err(&e.to_string()),
            Ok(Ok((status, text))) if !status.is_success() => {
                pending.err(&http_reason(status, &text))
            }
            Ok(Ok((_, text))) => match events(&text) {
                Ok(events) => match check(&events) {
                    Ok(value) => {
                        pending.ok();
                        return Some(value);
                    }
                    Err(e) => pending.err(&e),
                },
                Err(e) => pending.err(&format!("decode: {e}")),
            },
        }
        None
    }

    fn owed(&self) -> Owed {
        lock(&self.ledger).owed
    }

    async fn fetch(&self, fetch: Fetch) {
        let viewer = &self.viewer;
        match fetch {
            Fetch::Nip11 => self.nip11().await,
            Fetch::Roster => {
                let read = self.roster().await;
                let mut ledger = lock(&self.ledger);
                ledger.owed.roster = !read;
                if read {
                    ledger.roster_read = Some(Instant::now());
                }
            }
            Fetch::Metadata => {
                for ids in self.channels.chunks(PAGE) {
                    let filter = json!([{"kinds": [39000], "#d": ids, "limit": PAGE}]);
                    self.query(Op::DesktopMetadata, filter).await;
                }
            }
            Fetch::Sidebar => {
                let filters: Vec<Value> = [
                    "channel-sections",
                    "channel-stars",
                    "channel-mutes",
                    "channel-sort",
                    "channel-filter",
                ]
                .iter()
                .map(|d| json!({"kinds": [30078], "authors": [viewer], "#d": [d], "limit": 1}))
                .collect();
                self.query(Op::DesktopSidebar, Value::Array(filters)).await;
            }
            Fetch::Profiles => {
                lock(&self.ledger).owed.profiles = true;
                let mut read = true;
                for authors in self.profiles.chunks(PAGE) {
                    let filter = json!([{"kinds": [0], "authors": authors, "limit": PAGE}]);
                    read &= self.query(Op::DesktopProfiles, filter).await.is_some();
                }
                lock(&self.ledger).owed.profiles = !read;
            }
            Fetch::Statuses => {
                let authors = &self.profiles[..self.profiles.len().min(STATUS_AUTHORS)];
                if !authors.is_empty() {
                    let filter = json!([{
                        "kinds": [30315], "#d": ["general"], "authors": authors, "limit": authors.len(),
                    }]);
                    self.query(Op::DesktopStatuses, filter).await;
                }
            }
            Fetch::Emoji => {
                let filter = json!([{"kinds": [30030], "#d": [EMOJI_SET], "limit": PAGE}]);
                self.query(Op::DesktopEmoji, filter).await;
            }
            Fetch::Unread { snapshot } => {
                // The snapshot and the channel repair settle separately:
                // a repair that succeeds without the snapshot leaves it owed.
                if snapshot {
                    let filter = json!([{
                        "kinds": [30078], "authors": [viewer], "read_state_snapshot": 1,
                    }]);
                    let read = self.query(Op::DesktopReadState, filter).await.is_some();
                    let mut ledger = lock(&self.ledger);
                    ledger.owed.snapshot = !read;
                    if !read {
                        // The repair waits for the snapshot it builds on.
                        ledger.owed.unread = true;
                        return;
                    }
                }
                let repaired = self.repair().await;
                lock(&self.ledger).owed.unread = !repaired;
            }
            Fetch::Reminders => {
                let filter = json!([{"kinds": [30300], "authors": [viewer], "limit": 201}]);
                self.query(Op::DesktopReminders, filter).await;
            }
            Fetch::Window(i) => self.window(&self.channels[i]).await,
            Fetch::Thread(i) => self.thread(i).await,
        }
    }

    /// The roster in pages until a short page. Whether every page was read.
    async fn roster(&self) -> bool {
        let mut filter = json!({"kinds": [39002], "#p": [self.viewer], "limit": PAGE});
        for _ in 0..MAX_ROSTER_PAGES {
            let Some(page) = self.query(Op::DesktopRoster, json!([filter])).await else {
                return false;
            };
            match last_row(&page) {
                Some(cursor) if page.len() >= PAGE => page_before(&mut filter, cursor),
                _ => break,
            }
        }
        true
    }

    /// An unread repair of the channels, one chunk after another, as the
    /// app does. Whether it all succeeded.
    async fn repair(&self) -> bool {
        let now = unix_now();
        let mark = lock(&self.unread_mark).replace(now);
        for ids in self.channels.chunks(UNREAD_CHANNELS) {
            let mut filter = json!({
                "kinds": [9, 40002, 40008], "#h": ids, "include_aux": true, "limit": REPLAY_LIMIT,
            });
            if let Some(mark) = mark {
                filter["since"] = json!(mark.saturating_sub(RESUME_LAG_SECS));
            }
            if self
                .query(Op::DesktopUnread, json!([filter]))
                .await
                .is_none()
            {
                return false;
            }
        }
        true
    }

    /// `GET /` for the relay's NIP-11 document, remembering its `self`.
    async fn nip11(&self) {
        if stopped(&self.shared) {
            return;
        }
        let shared = &self.shared;
        let base = &shared.communities[self.community].http_url;
        let pending = shared.stats.begin(Op::DesktopNip11);
        let Some(response) = bounded(shared, HTTP_TIMEOUT, shared.net.relay_info(base)).await
        else {
            pending.forget();
            return;
        };
        match response {
            Err(_) => pending.err("read timeout"),
            Ok(Err(e)) => pending.err(&e.to_string()),
            Ok(Ok((status, text))) if !status.is_success() => {
                pending.err(&http_reason(status, &text))
            }
            Ok(Ok((_, text))) => {
                pending.ok();
                let info: Value = serde_json::from_str(&text).unwrap_or_default();
                if let Some(relay) = info["self"].as_str() {
                    *lock(&self.relay) = Some(relay.to_string());
                }
            }
        }
    }

    /// A window catch-up: the head page, then older pages while the relay's
    /// bounds say there are more (at most [`CATCH_UP_PAGES`]), then the
    /// channel's notices.
    async fn window(&self, channel: &str) {
        let relay = lock(&self.relay).clone();
        let mut cursor: Option<Cursor> = None;
        for _ in 0..=CATCH_UP_PAGES {
            let mut filter = json!({
                "kinds": WINDOW_KINDS, "#h": [channel], "limit": WINDOW_PAGE,
                "top_level": true, "include_aux": true, "include_summaries": true,
            });
            if let Some((until, id)) = &cursor {
                page_before(&mut filter, (*until, id));
            }
            let at = cursor.as_ref();
            let next = self
                .query_with(Op::DesktopWindow, json!([filter]), |events| {
                    window_bounds(events, channel, at, relay.as_deref())
                })
                .await;
            match next {
                Some(Some(next)) => cursor = Some(next),
                _ => break,
            }
        }
        let mut notices = json!({"kinds": [40099], "#h": [channel], "limit": REPLAY_LIMIT});
        if let Some(relay) = relay {
            notices["authors"] = json!([relay]);
        }
        self.query(Op::DesktopNotices, json!([notices])).await;
    }

    /// Refreshes open thread `i`: a seeded root in one of the open windows'
    /// channels, read by id and then its first thread window page.
    async fn thread(&self, i: usize) {
        let windows = self.windows.clamp(1, self.channels.len());
        let channel = &self.channels[i % windows];
        let shared = &self.shared;
        let community = &shared.communities[self.community];
        let found = roots(shared, community, &self.keys, channel, Some(HTTP_TIMEOUT)).await;
        let Some(root) = found.get((i / windows) % found.len().max(1)) else {
            return;
        };
        let lookup = json!([{"ids": [root], "#h": [channel], "limit": 1}]);
        if self.query(Op::DesktopThreadRoot, lookup).await.is_none() {
            return;
        }
        let page = json!([{
            "kinds": [9, 40002], "#h": [channel], "#e": [root], "depth_limit": 100,
            "include_aux": true, "thread_window": true, "limit": 10,
        }]);
        self.query(Op::DesktopThreadWindow, page).await;
    }
}

/// The value behind `mutex`; a panicked holder never left it half-written.
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A client's HTTP reads, run by one task at most [`HTTP_CONCURRENCY`] at a
/// time, so reads never pile up behind a slow relay. A read that comes due
/// while the same work is running or queued is not added again; a read
/// leaves the running set as soon as it settles, so a failed one can be
/// retried while others are still under way. Dropping it (the client was
/// cancelled) aborts the task, so its unfinished reads are recorded.
#[derive(Default)]
struct Fetcher {
    task: Option<tokio::task::JoinHandle<()>>,
    work: Arc<std::sync::Mutex<Work>>,
    /// Wakes the task for newly queued reads.
    wake: Arc<tokio::sync::Notify>,
}

/// A [`Fetcher`]'s reads, shared with its task.
#[derive(Default)]
struct Work {
    /// Whether the task is under way; it clears this when it runs dry.
    active: bool,
    running: Vec<Fetch>,
    queued: Vec<Fetch>,
}

impl Work {
    /// The next queued read that may start beside the `running` ones, taken
    /// off the queue. NIP-11 runs alone and before anything queued with it,
    /// since notices and window bounds need the relay's pubkey.
    fn next(&mut self) -> Option<Fetch> {
        if self.running.len() >= HTTP_CONCURRENCY || self.running.contains(&Fetch::Nip11) {
            return None;
        }
        let i = match self.queued.iter().position(|f| *f == Fetch::Nip11) {
            Some(_) if !self.running.is_empty() => return None,
            Some(i) => i,
            None if self.queued.is_empty() => return None,
            None => 0,
        };
        let fetch = self.queued.remove(i);
        self.running.push(fetch);
        Some(fetch)
    }
}

impl Fetcher {
    fn start(&mut self, reader: &Arc<Reader>, fetches: Vec<Fetch>) {
        let mut work = lock(&self.work);
        for fetch in fetches {
            let covered = |have: &Fetch| have.covers(fetch);
            if work.running.iter().any(covered) || work.queued.iter().any(covered) {
                continue;
            }
            // A full repair queued now does a queued covered one's work too.
            work.queued.retain(|have| !fetch.covers(*have));
            work.queued.push(fetch);
        }
        if work.queued.is_empty() {
            return;
        }
        if work.active {
            self.wake.notify_one();
            return;
        }
        work.active = true;
        drop(work);
        let task = drain(reader.clone(), self.work.clone(), self.wake.clone());
        self.task = Some(tokio::spawn(task));
    }

    /// Waits for the reads under way and queued. The handle stays here
    /// until the task ends, so a client cancelled meanwhile still aborts it
    /// on drop instead of detaching it.
    async fn finish(&mut self) {
        if let Some(task) = &mut self.task {
            let _ = task.await;
        }
        self.task = None;
    }
}

/// A [`Fetcher`]'s task: starts queued reads as they may start, until none
/// are running or queued.
async fn drain(
    reader: Arc<Reader>,
    work: Arc<std::sync::Mutex<Work>>,
    wake: Arc<tokio::sync::Notify>,
) {
    let mut jobs = stream::FuturesUnordered::new();
    loop {
        {
            let mut work = lock(&work);
            while let Some(fetch) = work.next() {
                let reader = reader.clone();
                jobs.push(async move {
                    reader.fetch(fetch).await;
                    fetch
                });
            }
            if work.running.is_empty() {
                work.active = false;
                return;
            }
        }
        tokio::select! {
            Some(fetch) = jobs.next() => {
                let mut work = lock(&work);
                if let Some(i) = work.running.iter().position(|f| *f == fetch) {
                    work.running.remove(i);
                }
            }
            () = wake.notified() => {}
        }
    }
}

impl Drop for Fetcher {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

/// How a session ended.
enum End {
    Stopped,
    /// A planned identity renewal.
    Renewal,
    /// The human's scheduled session end, an unplanned reconnect.
    Dropped,
    Failed,
}

/// A REQ awaiting its EOSE.
struct Waiting<'a> {
    route: Route,
    /// [`Req::cut_by_design`].
    cut: bool,
    /// For a tick's replacement, the batch it takes over at its EOSE.
    slot: Option<usize>,
    /// Events replayed so far.
    events: usize,
    pending: Pending<'a>,
    sent: Instant,
}

/// A silence probe awaiting any frame.
struct Probe<'a> {
    pending: Pending<'a>,
    sent: Instant,
}

/// A desktop client. Its sessions differ by [`Situation`]: the first is
/// cold, a renewal follows [`renewal`], and any other reconnect is a
/// [`Situation::Reconnect`].
pub(super) async fn desktop(shared: Arc<Shared>, global: usize) {
    run_client(shared, global, REQ_TIMEOUT).await;
}

/// [`desktop`], where a REQ may wait `req_timeout` for its EOSE.
async fn run_client(shared: Arc<Shared>, global: usize, req_timeout: Duration) {
    let scenario = &shared.scenario;
    let profile = &scenario.desktop;
    let community = global % shared.communities.len();
    let keys = identity::keys(&scenario.run_id, Role::Human, global);
    let indices = scenario.member_channel_indices(Role::Human, global);
    let directory = shared.directory().await;
    let reader = Arc::new(Reader {
        shared: shared.clone(),
        community,
        viewer: keys.public_key().to_hex(),
        keys: keys.clone(),
        channels: scenario.member_channels(community, Role::Human, global),
        profiles: co_members(&directory, community, global, &indices),
        windows: profile.windows.min(indices.len()),
        relay: Default::default(),
        unread_mark: Default::default(),
        ledger: std::sync::Mutex::new(Ledger {
            owed: Owed {
                profiles: true,
                ..Owed::default()
            },
            roster_read: None,
        }),
    });
    let mut client = Client {
        shared: shared.clone(),
        reader,
        rng: Rng::new(global as u64 ^ 0xD35C_70B0),
        fetcher: Fetcher::default(),
        situation: Situation::Cold,
        req_timeout,
        next_tick: Instant::now() + Duration::from_secs_f64(profile.tick_secs),
        unread_at: None,
        caught_up: Vec::new(),
        generation: 0,
        probes: 0,
        first_renewal: true,
        last_message: None,
        split: scenario.wide_split(Role::Human, global),
    };
    let started = unix_now();
    let mut cursor = None;
    let mut backoff = Backoff::new();
    let target = &shared.communities[community];
    while let Some(mut conn) =
        connect(&shared, target, &keys, None, &mut client.rng, &mut backoff).await
    {
        let since = resume_since(started, cursor, unix_now());
        let end = client.session(&mut conn, since).await;
        cursor = Some(unix_now());
        let _ = conn.disconnect().await;
        client.situation = match end {
            End::Stopped => break,
            End::Renewal => renewal(profile.mode),
            End::Dropped => {
                backoff = Backoff::new();
                Situation::Reconnect
            }
            End::Failed => {
                // A failed session backs off like a failed connect.
                backoff.wait(&shared, &mut client.rng).await;
                Situation::Reconnect
            }
        };
    }
    client.fetcher.finish().await;
}

struct Client {
    shared: Arc<Shared>,
    reader: Arc<Reader>,
    rng: Rng,
    fetcher: Fetcher,
    situation: Situation,
    req_timeout: Duration,
    /// The recovery tick runs on the client's clock, across connections.
    next_tick: Instant,
    /// When a full unread repair was last asked for.
    unread_at: Option<Instant>,
    /// The routes the last connection had caught up.
    caught_up: Vec<Route>,
    generation: u64,
    /// Silence probes sent, across connections.
    probes: u64,
    first_renewal: bool,
    last_message: Option<Seen>,
    split: Option<WideSplit>,
}

impl Client {
    /// Starts `fetches`. Asking for a full unread repair restarts its
    /// backstop; a covered repair reads no snapshot, so it does not.
    fn fetch(&mut self, fetches: Vec<Fetch>) {
        if fetches.contains(&FULL_REPAIR) {
            self.unread_at = Some(Instant::now());
        }
        self.fetcher.start(&self.reader, fetches);
    }

    /// One connection: establishes per the situation, then runs ticks,
    /// presence and the human's own traffic until it ends.
    async fn session(&mut self, conn: &mut NostrWsConnection, since: u64) -> End {
        let shared = self.shared.clone();
        let stats = &shared.stats;
        let scenario = &shared.scenario;
        let profile = &scenario.desktop;
        let mode = profile.mode;
        let reader = self.reader.clone();
        let channels = &reader.channels;
        let community = &shared.communities[reader.community];
        let (windows, threads) = (reader.windows, profile.threads);

        self.generation += 1;
        let presence = &reader.profiles[..reader.profiles.len().min(PRESENCE_AUTHORS)];
        let reqs = connect_reqs(
            mode,
            &reader.viewer,
            presence,
            channels,
            since,
            self.generation,
        );
        let mut batches: Vec<String> = reqs
            .iter()
            .filter(|r| r.route == Route::Channels)
            .map(|r| r.id.clone())
            .collect();
        // `None` once a replacement missed its deadline: the next tick retries.
        let mut batches_at = Some(Instant::now());
        let mut setup: VecDeque<(Req, Option<usize>)> =
            reqs.into_iter().map(|r| (r, None)).collect();
        self.fetch(connect_fetches(mode, self.situation, windows, threads));
        let mut establishment = Establishment::new(self.situation, &self.caught_up);

        let mut waiting: HashMap<String, Waiting<'_>> = HashMap::new();
        let mut open: HashSet<String> = HashSet::new();
        // Whether any frame came since the last tick, the probe a silent
        // tick sends, and that probe's subscription, closed at its EOSE or,
        // failing that, by the next probe.
        let mut heard = false;
        // When the deadlines past due were first deferred to a backlog.
        let mut deferred_since: Option<Instant> = None;
        let mut probe: Option<Probe<'_>> = None;
        let mut probe_id: Option<String> = None;
        let mut activity = Activity::new(&scenario.human, &mut self.rng, self.split);
        activity.root_bound = Some(HTTP_TIMEOUT);
        let tick_every = Duration::from_secs_f64(profile.tick_secs);
        let backstop = Duration::from_secs_f64(profile.backstop_secs);
        let next_renewal = (profile.renewal_secs > 0.0).then(|| {
            // Clients connecting together do not renew together.
            let mut secs = profile.renewal_secs;
            if std::mem::take(&mut self.first_renewal) {
                secs *= self.rng.unit();
            }
            Instant::now() + Duration::from_secs_f64(secs)
        });
        let disconnected = |e: WsClientError| {
            stats.err(Op::WsConnect, &format!("disconnected: {e}"));
            End::Failed
        };

        let end = 'session: loop {
            if stopped(&shared) {
                break End::Stopped;
            }
            while waiting.len() < SETUP_CONCURRENCY {
                let Some((req, slot)) = setup.pop_front() else {
                    break;
                };
                if let Err(e) = conn.send_raw(&req.frame()).await {
                    break 'session disconnected(e);
                }
                let op = match req.route {
                    Route::Replay => Op::DesktopReplay,
                    _ => Op::DesktopLive,
                };
                let waited = Waiting {
                    route: req.route,
                    cut: req.cut_by_design(),
                    slot,
                    events: 0,
                    pending: stats.begin(op),
                    sent: Instant::now(),
                };
                waiting.insert(req.id, waited);
            }

            let due = [
                activity.due(),
                Some(self.next_tick),
                next_renewal,
                probe.as_ref().map(|p| p.sent + self.req_timeout),
            ]
            .into_iter()
            .flatten()
            .min();
            let wait = due
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(MAX_IDLE)
                .min(MAX_IDLE);
            // The first frame waits; after it, only frames already in hand
            // are handled (at most DRAIN_FRAMES), so an EOSE buffered while a
            // publish waited for its OK is seen before deadlines are judged.
            // Each frame is judged by when it arrived, not when it is read:
            // one that came after its deadline is no answer.
            let mut established = Vec::new();
            // REQs whose EOSE came after their deadline.
            let mut late = Vec::new();
            let req_timeout = self.req_timeout;
            let in_time = move |sent: Instant, at: Instant| at.duration_since(sent) < req_timeout;
            let mut drained = 0;
            let mut next = conn.next_event_received(wait).await;
            let backlogged = loop {
                let (at, message) = match next {
                    Ok(received) => received,
                    Err(WsClientError::Timeout) => break false,
                    Err(e) => break 'session disconnected(e),
                };
                match message {
                    RelayMessage::Eose { subscription_id } => {
                        if probe_id.as_deref() == Some(subscription_id.as_str()) {
                            // The app closes its probe at its EOSE.
                            probe_id = None;
                            if let Err(e) = close(conn, &subscription_id).await {
                                break 'session disconnected(e);
                            }
                        } else if waiting
                            .get(&subscription_id)
                            .is_some_and(|w| !in_time(w.sent, at))
                        {
                            late.extend(waiting.remove_entry(&subscription_id));
                        } else if let Some(w) = waiting.remove(&subscription_id) {
                            w.pending.ok();
                            let retired = match (w.route, w.slot) {
                                (Route::Replay, _) => Some(subscription_id),
                                (_, Some(slot)) => {
                                    open.insert(subscription_id.clone());
                                    let old =
                                        std::mem::replace(&mut batches[slot], subscription_id);
                                    open.remove(&old);
                                    Some(old)
                                }
                                (route, None) => {
                                    open.insert(subscription_id);
                                    let cut = limited(w.cut, w.events);
                                    established.extend(establishment.eose(route, cut));
                                    None
                                }
                            };
                            if let Some(id) = retired {
                                if let Err(e) = close(conn, &id).await {
                                    break 'session disconnected(e);
                                }
                            }
                        }
                    }
                    RelayMessage::Closed {
                        subscription_id,
                        message,
                    } => {
                        // Live subscriptions the relay refuses end the session,
                        // as for the basic human; a refused replay is an error.
                        if let Some(w) = waiting.remove(&subscription_id) {
                            w.pending.err(&message);
                            if w.route != Route::Replay {
                                break 'session End::Failed;
                            }
                        } else if open.remove(&subscription_id) {
                            stats.err(
                                Op::DesktopLive,
                                &format!("CLOSED {subscription_id}: {message}"),
                            );
                            break 'session End::Failed;
                        }
                    }
                    message => {
                        // An event before EOSE is replay, which the app counts
                        // against its limit; only events after EOSE are delivery.
                        if let RelayMessage::Event {
                            subscription_id, ..
                        } = &message
                        {
                            if let Some(w) = waiting.get_mut(subscription_id) {
                                w.events += 1;
                            } else if open.contains(subscription_id) {
                                if let Some(target) = observe(stats, &message) {
                                    self.last_message = Some(target);
                                }
                            }
                        }
                    }
                }
                // Any frame proves the socket still reaches the relay, but
                // answers the probe only if it came by the probe's deadline.
                heard = true;
                if let Some(p) = probe.take_if(|p| in_time(p.sent, at)) {
                    p.pending.ok();
                }
                drained += 1;
                if drained == DRAIN_FRAMES {
                    break true;
                }
                next = conn.next_event_received(Duration::ZERO).await;
            };

            // A REQ past its deadline: a replay or a replacement is closed (a
            // replacement keeps the batch it would have replaced, and the next
            // tick tries again); a connection's live route stays open, and
            // establishes as cut, since it may have missed history.
            let overdue = |w: &Waiting| w.sent.elapsed() >= self.req_timeout;
            let deferring = defer(
                &mut deferred_since,
                backlogged,
                waiting.values().any(overdue),
                self.req_timeout,
            );
            let timed_out: Vec<String> = waiting
                .iter()
                .filter(|(_, w)| !deferring && overdue(w))
                .map(|(id, _)| id.clone())
                .collect();
            // A REQ whose EOSE came late timed out, whatever the backlog
            // still holds.
            let mut expired = late;
            expired.extend(timed_out.iter().filter_map(|id| waiting.remove_entry(id)));
            for (id, w) in expired {
                w.pending.err("EOSE timeout");
                if w.route == Route::Replay || w.slot.is_some() {
                    if w.slot.is_some() {
                        batches_at = None;
                    }
                    if let Err(e) = close(conn, &id).await {
                        break 'session disconnected(e);
                    }
                } else {
                    open.insert(id);
                    established.extend(establishment.timeout(w.route));
                }
            }

            // A probe nothing answered: the socket stopped delivering. Any
            // frame drained above that came in time has answered it.
            if let Some(p) = probe.take_if(|p| p.sent.elapsed() >= self.req_timeout) {
                p.pending.err("probe timeout");
                break End::Failed;
            }

            // `legacy` read everything when it connected.
            if mode == DesktopMode::Current {
                let mut reads = Vec::new();
                for (route, continuous) in established {
                    reads.extend(establish_fetches(
                        route,
                        continuous,
                        reader.owed(),
                        windows,
                        threads,
                    ));
                }
                if !reads.is_empty() {
                    self.fetch(reads);
                }
            }

            // While deadlines wait on the backlog, the pass only drains: a
            // publish waiting for its OK, or a tick, could outlast the
            // deferral and expire an EOSE already in hand.
            if deferring {
                continue;
            }

            let step = activity
                .step(
                    &shared,
                    community,
                    &reader.keys,
                    channels,
                    conn,
                    &mut self.rng,
                    &mut self.last_message,
                )
                .await;
            match step {
                Ok(Step::Continue) => {}
                // A publish's OK is a frame too, though the step consumed it;
                // one in time answers the probe.
                Ok(Step::Answered) => {
                    heard = true;
                    if let Some(p) = probe.take_if(|p| p.sent.elapsed() < self.req_timeout) {
                        p.pending.ok();
                    }
                }
                Ok(Step::SessionEnd) => break End::Dropped,
                Err(e) => break disconnected(e),
            }

            let now = Instant::now();
            if now >= self.next_tick {
                self.next_tick = now + tick_every;
                let (owed, roster_read) = {
                    let ledger = lock(&reader.ledger);
                    (ledger.owed, ledger.roster_read)
                };
                let clocks = Clocks {
                    batches: batches_at.map(|t| now.saturating_duration_since(t)),
                    roster: roster_read.map(|t| now.saturating_duration_since(t)),
                    unread: self.unread_at.map(|t| now.saturating_duration_since(t)),
                };
                let due = tick(mode, backstop, clocks, owed);
                // A replacement still waiting for its EOSE is not replaced again.
                let replacing = setup.iter().any(|(_, slot)| slot.is_some())
                    || waiting.values().any(|w| w.slot.is_some());
                if due.batches && !replacing {
                    self.generation += 1;
                    let fresh = batch_reqs(mode, channels, unix_now(), 0, self.generation);
                    setup.extend(
                        fresh
                            .into_iter()
                            .zip(0..batches.len())
                            .map(|(r, i)| (r, Some(i))),
                    );
                    batches_at = Some(now);
                }
                self.fetch(tick_fetches(due));
                // A silent minute asks the relay for an empty answer, in
                // both modes; no frame by the deadline ends the session.
                if !std::mem::take(&mut heard) && probe.is_none() {
                    if let Some(old) = probe_id.take() {
                        if let Err(e) = close(conn, &old).await {
                            break disconnected(e);
                        }
                    }
                    self.probes += 1;
                    let id = format!("probe-{}", self.probes);
                    let req =
                        json!(["REQ", id, {"kinds": [44100], "#p": [reader.viewer], "limit": 0}]);
                    if let Err(e) = conn.send_raw(&req).await {
                        break disconnected(e);
                    }
                    probe = Some(Probe {
                        pending: stats.begin(Op::DesktopProbe),
                        sent: Instant::now(),
                    });
                    probe_id = Some(id);
                }
            }
            if next_renewal.is_some_and(|t| now >= t) {
                break End::Renewal;
            }
        };
        // Subscriptions still setting up are abandoned with the socket, not failed.
        for (_, w) in waiting.drain() {
            w.pending.forget();
        }
        if let Some(p) = probe {
            p.pending.forget();
        }
        self.caught_up = establishment.caught_up();
        end
    }
}

/// Whether a pass defers the deadlines already past. A backlog still in hand
/// may hold their EOSEs in time, so they wait for the next pass, but for at
/// most `timeout` from the first pass that deferred them (`since`): traffic
/// that keeps every pass full does not defer them forever.
fn defer(since: &mut Option<Instant>, backlogged: bool, overdue: bool, timeout: Duration) -> bool {
    if !(backlogged && overdue) {
        *since = None;
        return false;
    }
    since.get_or_insert_with(Instant::now).elapsed() < timeout
}

async fn close(conn: &mut NostrWsConnection, id: &str) -> Result<(), WsClientError> {
    conn.send_raw(&json!(["CLOSE", id])).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("c{i}")).collect()
    }

    /// (filters, limit of the first filter, `#h` ids of the first filter,
    /// replay) per REQ.
    fn shape(reqs: &[Req]) -> Vec<(usize, u64, usize, bool)> {
        reqs.iter()
            .map(|r| {
                let first = &r.filters[0];
                (
                    r.filters.len(),
                    first["limit"].as_u64().unwrap(),
                    first["#h"].as_array().map_or(0, Vec::len),
                    r.route == Route::Replay,
                )
            })
            .collect()
    }

    #[test]
    fn up_to_ten_channels_share_one_replaying_batch_in_both_modes() {
        for mode in [DesktopMode::Legacy, DesktopMode::Current] {
            let reqs = connect_reqs(mode, "v", &[], &ids(5), 100, 1);
            let names: Vec<&str> = reqs.iter().map(|r| r.id.as_str()).collect();
            assert_eq!(names, ["profiles", "membership", "batch-1-0"], "{mode:?}");
            assert_eq!(shape(&reqs[2..]), [(5, 500, 1, false)], "{mode:?}");
            assert_eq!(reqs[2].filters[0]["since"], 100);
            assert_eq!(reqs[2].filters[0]["kinds"], json!(CHANNEL_KINDS));
            assert!(reqs.iter().all(|r| !r.cut_by_design()), "{mode:?}");
        }
    }

    #[test]
    fn more_than_ten_channels_batch_by_ten_before_and_replay_separately_after() {
        let legacy = connect_reqs(DesktopMode::Legacy, "v", &[], &ids(25), 100, 1);
        assert_eq!(
            shape(&legacy[2..]),
            [(10, 500, 1, false), (10, 500, 1, false), (5, 500, 1, false)]
        );
        assert!(legacy.iter().all(|r| !r.cut_by_design()));
        let current = connect_reqs(DesktopMode::Current, "v", &[], &ids(25), 100, 1);
        assert_eq!(
            shape(&current[2..]),
            [
                (1, 0, 25, false),
                (10, 500, 1, true),
                (10, 500, 1, true),
                (5, 500, 1, true)
            ]
        );
        // The live-only batch replays nothing, so its window counts as cut.
        let cut: Vec<_> = current.iter().map(Req::cut_by_design).collect();
        assert_eq!(cut, [false, false, true, false, false, false]);
        // One live-only filter holds up to 2048 channels.
        let wide = connect_reqs(DesktopMode::Current, "v", &[], &ids(2049), 100, 1);
        let live: Vec<_> = wide[2..]
            .iter()
            .filter(|r| r.route == Route::Channels)
            .collect();
        assert_eq!(live.len(), 2);
        assert_eq!(wide.len(), 2 + 2 + 205);
    }

    #[test]
    fn tick_replacements_are_live_only() {
        let legacy = batch_reqs(DesktopMode::Legacy, &ids(25), 100, 0, 2);
        assert_eq!(
            shape(&legacy),
            [(10, 0, 1, false), (10, 0, 1, false), (5, 0, 1, false)]
        );
        let current = batch_reqs(DesktopMode::Current, &ids(25), 100, 0, 2);
        assert_eq!(shape(&current), [(1, 0, 25, false)]);
        let small = batch_reqs(DesktopMode::Current, &ids(5), 100, 0, 2);
        assert_eq!(shape(&small), [(5, 0, 1, false)]);
        assert_eq!(small[0].id, "batch-2-0");
    }

    #[test]
    fn a_replay_is_limited_at_the_apps_cap() {
        assert!(!limited(false, REPLAY_LIMIT - 1));
        assert!(limited(false, REPLAY_LIMIT));
        assert!(limited(true, 0));
    }

    const NO_OWED: Owed = Owed {
        roster: false,
        profiles: false,
        snapshot: false,
        unread: false,
    };

    #[test]
    fn legacy_ticks_do_everything_and_current_ticks_wait_for_the_backstop() {
        let backstop = Duration::from_secs(900);
        let minute = Some(Duration::from_secs(60));
        let clocks = |batches, roster, unread| Clocks {
            batches,
            roster,
            unread,
        };
        let all = Due {
            batches: true,
            roster: true,
            unread: true,
        };
        let none = Due {
            batches: false,
            roster: false,
            unread: false,
        };
        let fresh = clocks(minute, minute, minute);
        assert_eq!(tick(DesktopMode::Legacy, backstop, fresh, NO_OWED), all);
        assert_eq!(tick(DesktopMode::Current, backstop, fresh, NO_OWED), none);
        let stale = clocks(Some(backstop), Some(backstop), Some(backstop));
        assert_eq!(tick(DesktopMode::Current, backstop, stale, NO_OWED), all);
        // Never done counts as stale.
        let never = clocks(None, None, None);
        assert_eq!(tick(DesktopMode::Current, backstop, never, NO_OWED), all);
        // Each backstop runs on its own clock.
        let roster_only = tick(
            DesktopMode::Current,
            backstop,
            clocks(minute, Some(backstop), minute),
            NO_OWED,
        );
        assert_eq!(tick_fetches(roster_only), [Fetch::Roster]);
        assert_eq!(tick_fetches(all), [Fetch::Roster, FULL_REPAIR]);
        assert!(tick_fetches(none).is_empty());
    }

    #[test]
    fn owed_reads_retry_on_the_next_tick_before_their_backstop() {
        let backstop = Duration::from_secs(900);
        let minute = Some(Duration::from_secs(60));
        let fresh = Clocks {
            batches: minute,
            roster: minute,
            unread: minute,
        };
        let owed = Owed {
            roster: true,
            unread: true,
            ..NO_OWED
        };
        let due = tick(DesktopMode::Current, backstop, fresh, owed);
        assert!(!due.batches);
        assert_eq!(tick_fetches(due), [Fetch::Roster, FULL_REPAIR]);
        let snapshot = Owed {
            snapshot: true,
            ..NO_OWED
        };
        let due = tick(DesktopMode::Current, backstop, fresh, snapshot);
        assert_eq!(tick_fetches(due), [FULL_REPAIR]);
    }

    #[test]
    fn renewals_reconnect_before_and_carry_routes_after() {
        assert_eq!(renewal(DesktopMode::Legacy), Situation::Reconnect);
        assert_eq!(renewal(DesktopMode::Current), Situation::Renewal);

        let cold = connect_fetches(DesktopMode::Legacy, Situation::Cold, 2, 1);
        assert_eq!(cold.len(), 9 + 3);
        let reconnect = connect_fetches(DesktopMode::Legacy, Situation::Reconnect, 2, 1);
        // A reconnect rereads everything but names and sidebar preferences.
        let skipped: Vec<_> = cold.iter().filter(|r| !reconnect.contains(r)).collect();
        assert_eq!(skipped, [&Fetch::Metadata, &Fetch::Sidebar]);

        // `current` reads up front only what no route covers.
        let current = |s| connect_fetches(DesktopMode::Current, s, 2, 1);
        assert_eq!(
            current(Situation::Cold),
            [Fetch::Nip11, Fetch::Metadata, Fetch::Sidebar]
        );
        assert_eq!(current(Situation::Reconnect), [Fetch::Nip11]);
        assert!(current(Situation::Renewal).is_empty());
    }

    #[test]
    fn a_route_establishes_its_reads_by_continuity() {
        let channels = [Fetch::Window(0), Fetch::Window(1), Fetch::Thread(0)];
        let est = |route, continuous, owed| establish_fetches(route, continuous, owed, 2, 1);
        assert_eq!(
            est(Route::Profiles, false, NO_OWED),
            [Fetch::Profiles, Fetch::Statuses, Fetch::Emoji]
        );
        assert!(est(Route::Profiles, true, NO_OWED).is_empty());
        let profiles_owed = Owed {
            profiles: true,
            ..NO_OWED
        };
        assert_eq!(est(Route::Profiles, true, profiles_owed), [Fetch::Profiles]);
        assert_eq!(
            est(Route::Membership, false, NO_OWED),
            [Fetch::Roster, FULL_REPAIR, Fetch::Reminders]
        );
        assert_eq!(est(Route::Membership, true, NO_OWED), [COVERED_REPAIR]);
        let roster_owed = Owed {
            roster: true,
            ..NO_OWED
        };
        assert_eq!(
            est(Route::Membership, true, roster_owed),
            [Fetch::Roster, COVERED_REPAIR]
        );
        // A continuous route cannot settle an owed snapshot without reading it.
        let snapshot_owed = Owed {
            snapshot: true,
            ..NO_OWED
        };
        assert_eq!(est(Route::Membership, true, snapshot_owed), [FULL_REPAIR]);
        assert_eq!(est(Route::Channels, true, NO_OWED), channels);
        assert_eq!(est(Route::Channels, false, NO_OWED), channels);
        assert!(est(Route::Replay, false, NO_OWED).is_empty());
        assert!(FULL_REPAIR.covers(COVERED_REPAIR));
        assert!(!COVERED_REPAIR.covers(FULL_REPAIR));
    }

    #[test]
    fn routes_establish_once_and_a_renewal_holds_them_until_a_cut() {
        let all = [Route::Profiles, Route::Membership, Route::Channels];
        // Cold: every route establishes at its EOSE, not continuous.
        let mut cold = Establishment::new(Situation::Cold, &all);
        assert_eq!(
            cold.eose(Route::Profiles, false),
            [(Route::Profiles, false)]
        );
        assert!(cold.eose(Route::Replay, false).is_empty());
        assert_eq!(
            cold.eose(Route::Channels, false),
            [(Route::Channels, false)]
        );
        assert!(cold.eose(Route::Channels, false).is_empty());
        assert_eq!(cold.caught_up(), [Route::Profiles, Route::Channels]);

        // Reconnect: each route by its own replay, and only if the last
        // connection had it caught up.
        let mut reconnect = Establishment::new(Situation::Reconnect, &all[..2]);
        assert_eq!(
            reconnect.eose(Route::Membership, false),
            [(Route::Membership, true)]
        );
        assert_eq!(
            reconnect.eose(Route::Profiles, true),
            [(Route::Profiles, false)]
        );
        assert_eq!(
            reconnect.eose(Route::Channels, false),
            [(Route::Channels, false)]
        );

        // Renewal without a cut: nothing establishes, all stay caught up.
        let mut quiet = Establishment::new(Situation::Renewal, &all);
        for route in all {
            assert!(quiet.eose(route, false).is_empty());
        }
        assert_eq!(quiet.caught_up(), all);

        // Renewal with a cut: the held routes establish with it.
        let mut cut = Establishment::new(Situation::Renewal, &all);
        assert!(cut.eose(Route::Profiles, false).is_empty());
        assert_eq!(
            cut.eose(Route::Channels, true),
            [(Route::Profiles, true), (Route::Channels, false)]
        );
        assert_eq!(
            cut.eose(Route::Membership, false),
            [(Route::Membership, true)]
        );

        // A timed-out route recovers as cut, but is not caught up.
        let mut late = Establishment::new(Situation::Reconnect, &all);
        assert_eq!(
            late.timeout(Route::Membership),
            [(Route::Membership, false)]
        );
        assert_eq!(late.eose(Route::Profiles, false), [(Route::Profiles, true)]);
        assert_eq!(late.caught_up(), [Route::Profiles]);

        // A renewal before a route ever caught up reads it in full.
        let mut early = Establishment::new(Situation::Renewal, &all[..1]);
        assert_eq!(
            early.eose(Route::Membership, false),
            [(Route::Membership, false)]
        );

        // Presence, like a replay, establishes nothing: neither its timeout
        // nor its EOSE after one cuts a renewal's carry, and it is never
        // caught up.
        let mut presence = Establishment::new(Situation::Renewal, &all);
        assert!(presence.timeout(Route::Presence).is_empty());
        assert!(presence.eose(Route::Presence, false).is_empty());
        for route in all {
            assert!(presence.eose(route, false).is_empty());
        }
        assert_eq!(presence.caught_up(), all);
        let mut after = Establishment::new(Situation::Renewal, &presence.caught_up());
        assert!(after.eose(Route::Presence, false).is_empty());
        assert!(after.eose(Route::Profiles, false).is_empty());
    }

    #[test]
    fn presence_is_a_live_only_global_only_while_someone_is_watched() {
        for mode in [DesktopMode::Legacy, DesktopMode::Current] {
            let watched = ["a".to_string(), "b".to_string()];
            let reqs = connect_reqs(mode, "v", &watched, &ids(5), 100, 1);
            let routes: Vec<Route> = reqs.iter().map(|r| r.route).collect();
            assert_eq!(
                routes,
                [
                    Route::Profiles,
                    Route::Membership,
                    Route::Presence,
                    Route::Channels
                ],
                "{mode:?}"
            );
            // Live-only, as the app sends it: no history query.
            assert_eq!(
                reqs[2].filters,
                [json!({"kinds": [20001], "authors": ["a", "b"], "since": 100, "limit": 0})]
            );
            let alone = connect_reqs(mode, "v", &[], &ids(5), 100, 1);
            assert!(alone.iter().all(|r| r.route != Route::Presence));
        }
        assert!(establish_fetches(Route::Presence, false, Owed::default(), 2, 1).is_empty());
    }

    #[test]
    fn resumed_subscriptions_start_before_the_cursor_but_not_before_the_floor() {
        assert_eq!(resume_since(10_000, None, 20_000), 19_700);
        assert_eq!(resume_since(10_000, Some(15_000), 20_000), 14_040);
        assert_eq!(resume_since(10_000, Some(10_100), 20_000), 9_700);
    }

    fn hex(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn bound(d: &str, content: Value) -> Value {
        json!({
            "kind": 39006, "pubkey": "relay", "content": content.to_string(),
            "tags": [["d", d], ["h", "c0"]],
        })
    }

    #[test]
    fn window_pages_follow_the_relays_bounds() {
        let next = json!({"has_more": true, "next_cursor": {"created_at": 1000, "id": hex('b')}});
        let row = json!({"kind": 9, "created_at": 1000, "id": hex('a')});
        let head = [row.clone(), bound("c0:head", next.clone())];
        let cursor = (1000, hex('b'));
        assert_eq!(
            window_bounds(&head, "c0", None, Some("relay")),
            Ok(Some(cursor.clone()))
        );
        // Exhausted.
        let done = json!({"has_more": false, "next_cursor": null});
        let d = format!("c0:1000:{}", hex('b'));
        let last = [bound(&d, done.clone())];
        assert_eq!(window_bounds(&last, "c0", Some(&cursor), None), Ok(None));
        // No bounds, two bounds, another signer, or the wrong cursor.
        assert!(window_bounds(&[row], "c0", None, None).is_err());
        let twice = [
            bound("c0:head", next.clone()),
            bound("c0:head", next.clone()),
        ];
        assert!(window_bounds(&twice, "c0", None, None).is_err());
        assert!(window_bounds(&head, "c0", None, Some("other")).is_err());
        assert!(window_bounds(&last, "c0", None, None).is_err());
        // Contradictions and a cursor that does not move on.
        let contradict =
            json!({"has_more": false, "next_cursor": {"created_at": 1000, "id": hex('b')}});
        assert!(window_bounds(&[bound("c0:head", contradict)], "c0", None, None).is_err());
        let back = [bound(
            &d,
            json!({"has_more": true, "next_cursor": {"created_at": 1000, "id": hex('a')}}),
        )];
        assert!(window_bounds(&back, "c0", Some(&cursor), None).is_err());
        let short = json!({"has_more": true, "next_cursor": {"created_at": 1000, "id": "ab"}});
        assert!(window_bounds(&[bound("c0:head", short)], "c0", None, None).is_err());
    }

    #[test]
    fn the_last_row_is_the_oldest_then_the_highest_id() {
        let page = [
            json!({"created_at": 1000, "id": "b"}),
            json!({"created_at": 999, "id": "a"}),
            json!({"created_at": 999, "id": "c"}),
            json!({"created_at": 1001, "id": "z"}),
        ];
        assert_eq!(last_row(&page), Some((999, "c")));
        assert_eq!(last_row(&[]), None);
    }

    // The client against loopback fakes: every connect, REQ, CLOSE and HTTP
    // read below goes through the production paths.

    use super::super::net::Net;
    use super::super::run::kind;
    use super::super::scenario::Scenario;
    use super::super::seed::tests::fake_http;
    use super::super::stats::Stats;
    use nostr::{EventBuilder, Tag};
    use tokio::sync::watch;

    type Shared2<T> = Arc<std::sync::Mutex<T>>;

    /// `(connection, verb, subscription id)` of every REQ and CLOSE the fake
    /// relay got, connections numbered as accepted.
    type Frames = Shared2<Vec<(usize, String, String)>>;

    /// How the fake relay answers a REQ on a connection: events replayed
    /// before its EOSE, and how long the answer waits.
    type Plan = Arc<dyn Fn(usize, &str) -> (usize, Duration) + Send + Sync>;

    fn instant() -> Plan {
        Arc::new(|_, _| (0, Duration::ZERO))
    }

    /// A loopback relay that challenges, accepts AUTH and every EVENT (its OK
    /// `ok_delay` late), answers each REQ per `plan` and records REQs,
    /// CLOSEs and EVENTs (by kind). With `drop_first`, it closes its first
    /// connection that long after accepting it.
    async fn fake_ws_relay(
        plan: Plan,
        drop_first: Option<Duration>,
        ok_delay: Duration,
    ) -> (String, Frames) {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let frames = Frames::default();
        let seen = frames.clone();
        let replayed = EventBuilder::new(kind(9), "replayed")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let replayed = serde_json::to_value(replayed).unwrap();
        tokio::spawn(async move {
            let mut conn = 0;
            while let Ok((stream, _)) = listener.accept().await {
                let (seen, plan, replayed) = (seen.clone(), plan.clone(), replayed.clone());
                let index = conn;
                conn += 1;
                tokio::spawn(async move {
                    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    let (mut sink, mut stream) = ws.split();
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
                    tokio::spawn(async move {
                        while let Some(message) = rx.recv().await {
                            let close = matches!(message, Message::Close(_));
                            if sink.send(message).await.is_err() || close {
                                return;
                            }
                        }
                    });
                    let text = |v: Value| Message::Text(v.to_string().into());
                    let _ = tx.send(text(json!(["AUTH", "challenge"])));
                    if let Some(after) = drop_first.filter(|_| index == 0) {
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(after).await;
                            let _ = tx.send(Message::Close(None));
                        });
                    }
                    while let Some(Ok(Message::Text(raw))) = stream.next().await {
                        let frame: Value = serde_json::from_str(&raw).unwrap();
                        let verb = frame[0].as_str().unwrap_or_default().to_string();
                        let id = match frame[1].get("kind") {
                            Some(kind) => kind.to_string(),
                            None => frame[1].as_str().unwrap_or_default().to_string(),
                        };
                        match verb.as_str() {
                            "AUTH" => {
                                let _ = tx.send(text(json!(["OK", frame[1]["id"], true, ""])));
                            }
                            "EVENT" => {
                                let (tx, ok) =
                                    (tx.clone(), json!(["OK", frame[1]["id"], true, ""]));
                                tokio::spawn(async move {
                                    tokio::time::sleep(ok_delay).await;
                                    let _ = tx.send(text(ok));
                                });
                            }
                            "REQ" => {
                                let (events, delay) = plan(index, &id);
                                let (tx, replayed, id) = (tx.clone(), replayed.clone(), id.clone());
                                tokio::spawn(async move {
                                    tokio::time::sleep(delay).await;
                                    for _ in 0..events {
                                        let _ = tx.send(text(json!(["EVENT", id, replayed])));
                                    }
                                    let _ = tx.send(text(json!(["EOSE", id])));
                                });
                            }
                            _ => {}
                        }
                        if matches!(verb.as_str(), "REQ" | "CLOSE" | "EVENT") {
                            seen.lock().unwrap().push((index, verb, id));
                        }
                    }
                });
            }
        });
        (addr, frames)
    }

    /// Reads the fake API fails once each, by name: `roster`, `read_state`,
    /// `unread`, `reminders`, `roots`.
    type FailOnce = &'static [&'static str];

    /// A read the fake API answers late, by name, and how late.
    type Slow = Option<(&'static str, Duration)>;

    fn signed(keys: &Keys, k: u32, content: &str, at: u64, tags: Vec<Tag>) -> Value {
        let event = EventBuilder::new(kind(k), content)
            .custom_created_at(nostr::Timestamp::from(at))
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap();
        serde_json::to_value(event).unwrap()
    }

    fn tag(name: &str, value: &str) -> Tag {
        Tag::parse([name, value]).unwrap()
    }

    /// A loopback HTTP API, recording every query's first filter. NIP-11
    /// names the relay's key; the owner's root search finds one seeded root.
    /// A window head page holds a full page of rows at one timestamp, an
    /// older aux event and a summary; its relay-signed bounds point after the
    /// last row. The page there is full too, and its bounds say history is
    /// exhausted. Any other window cursor is refused. The read-state snapshot
    /// answers in its own shape; `fail` reads fail the first time, and a
    /// `slow` read blocks its connection's handler (run such a test on the
    /// multi-thread runtime).
    async fn fake_api(run_id: &str, fail: FailOnce, slow: Slow) -> (String, Shared2<Vec<Value>>) {
        let owner = identity::keys(run_id, Role::Owner, 0);
        let relay = Keys::generate();
        let root = Value::Array(vec![signed(&owner, 9, "lg-root 0", 2_000, vec![])]);
        let rows = |at: u64| -> Vec<Value> {
            (0..WINDOW_PAGE)
                .map(|i| signed(&owner, 9, &format!("w{i}"), at, vec![]))
                .collect()
        };
        let head_rows = rows(1_000);
        let last = head_rows
            .iter()
            .filter_map(|e| e["id"].as_str())
            .max()
            .unwrap()
            .to_string();
        let older = rows(990);
        let info = json!({"self": relay.public_key().to_hex()}).to_string();
        let owner_hex = owner.public_key().to_hex();
        let queries = Shared2::<Vec<Value>>::default();
        let recorded = queries.clone();
        let failed = Shared2::<Vec<&'static str>>::default();
        let snapshot =
            json!({"read_state_snapshot": 1, "complete": true, "events": []}).to_string();
        let http = fake_http(move |method, target, body| {
            if method == "GET" && target == "/" {
                return (200, info.clone());
            }
            let filters: Value = serde_json::from_slice(body).unwrap_or_default();
            let filter = filters[0].clone();
            recorded.lock().unwrap().push(filter.clone());
            let name = if filter["kinds"] == json!([39002]) {
                "roster"
            } else if filter["kinds"] == json!([9, 40002, 40008]) {
                "unread"
            } else if filter.get("read_state_snapshot").is_some() {
                "read_state"
            } else if filter["kinds"] == json!([30300]) {
                "reminders"
            } else if filter["kinds"] == json!([9]) && filter.get("authors").is_some() {
                "roots"
            } else {
                ""
            };
            if let Some((_, delay)) = slow.filter(|(slow, _)| *slow == name) {
                // Off the worker, so tasks queued behind this one still run.
                tokio::task::block_in_place(|| std::thread::sleep(delay));
            }
            if fail.contains(&name) && !failed.lock().unwrap().contains(&name) {
                failed.lock().unwrap().push(name);
                return (500, r#"{"error":"once"}"#.into());
            }
            if filter.get("read_state_snapshot").is_some() {
                return (200, snapshot.clone());
            }
            if filter.get("top_level").is_some() {
                let channel = filter["#h"][0].as_str().unwrap_or_default();
                let (d, mut page, next) = match (filter.get("until"), filter.get("before_id")) {
                    (None, None) => {
                        let mut page = head_rows.clone();
                        page.push(signed(&owner, 7, "+", 900, vec![tag("h", channel)]));
                        page.push(signed(&relay, 39005, "{}", 1_000, vec![tag("h", channel)]));
                        let next = json!({"created_at": 1_000, "id": last});
                        (format!("{channel}:head"), page, next)
                    }
                    (Some(until), Some(id)) if *until == json!(1_000) && *id == json!(last) => {
                        (format!("{channel}:1000:{last}"), older.clone(), Value::Null)
                    }
                    _ => return (400, r#"{"error":"unknown window cursor"}"#.into()),
                };
                let content = json!({"has_more": !next.is_null(), "next_cursor": next});
                let tags = vec![tag("d", &d), tag("h", channel)];
                page.push(signed(&relay, 39006, &content.to_string(), 1_000, tags));
                return (200, Value::Array(page).to_string());
            }
            if filter["kinds"] == json!([9]) && filter["authors"] == json!([owner_hex]) {
                (200, root.to_string())
            } else {
                (200, "[]".into())
            }
        })
        .await;
        (http, queries)
    }

    /// `humans` desktop humans in one community of 25 channels, each in
    /// `joined`, with two windows and one thread open and only the traffic
    /// of their own that `rig` sets.
    fn desktops(rig: &Rig, ws: &str, http: &str) -> Scenario {
        let (humans, joined, desktop) = (rig.humans, rig.joined, &rig.desktop);
        let (presence, messages, reactions) = (
            rig.presence_secs,
            rig.messages_per_hour,
            rig.reactions_per_hour,
        );
        Scenario::parse(&format!(
            r#"
run_id = "d"
[target]
ws_url = "ws://{ws}"
http_url = "http://{http}"
[shape]
communities = 1
channels_per_community = 25
[population]
humans = {humans}
agents = 0
channels_per_member = {joined}
[human]
presence_interval_secs = {presence:?}
typing_per_hour = 0.0
messages_per_hour = {messages:?}
reactions_per_hour = {reactions:?}
thread_reply_fraction = 1.0
history_limit = 1
[desktop]
fraction = 1.0
windows = 2
threads = 1
{desktop}
[[ramp]]
users = 1.0
hold_secs = 1
"#
        ))
        .unwrap()
    }

    /// Per-op `(count, errors)` by op name, plus the relay's frames and the
    /// API's queries.
    struct Outcome {
        ops: HashMap<&'static str, (u64, u64)>,
        frames: Vec<(usize, String, String)>,
        queries: Vec<Value>,
    }

    impl Outcome {
        fn count(&self, op: &str) -> u64 {
            self.ops.get(op).map_or(0, |c| c.0)
        }

        fn errors(&self, op: &str) -> u64 {
            self.ops.get(op).map_or(0, |c| c.1)
        }

        fn saw(&self, verb: &str, id: &str) -> bool {
            self.at(verb, id).is_some()
        }

        /// Where a frame came in the relay's record.
        fn at(&self, verb: &str, id: &str) -> Option<usize> {
            self.frames
                .iter()
                .position(|(_, v, i)| v == verb && i == id)
        }

        /// Counts of the desktop ops, the ones the model owns.
        fn desktop(&self) -> HashMap<&'static str, u64> {
            self.ops
                .iter()
                .filter(|(op, _)| op.starts_with("desktop_"))
                .map(|(op, c)| (*op, c.0))
                .collect()
        }

        /// Desktop op counts beyond `before`, zeroes dropped.
        fn since(&self, before: &HashMap<&'static str, u64>) -> HashMap<&'static str, u64> {
            self.desktop()
                .into_iter()
                .map(|(op, n)| (op, n - before.get(op).copied().unwrap_or(0)))
                .filter(|(_, n)| *n > 0)
                .collect()
        }
    }

    fn outcome(stats: &Stats, frames: &Frames, queries: &Shared2<Vec<Value>>) -> Outcome {
        let ops = stats
            .snapshot()
            .since(&Default::default())
            .into_iter()
            .filter(|(_, s)| s.count + s.errors > 0)
            .map(|(op, s)| (op.name(), (s.count, s.errors)))
            .collect();
        Outcome {
            ops,
            frames: frames.lock().unwrap().clone(),
            queries: queries.lock().unwrap().clone(),
        }
    }

    /// One loopback run.
    struct Rig {
        /// Humans in the scenario, whose profiles a client reads.
        humans: usize,
        /// Clients that run: the first `clients` humans.
        clients: usize,
        joined: usize,
        desktop: String,
        plan: Plan,
        drop_first: Option<Duration>,
        fail: FailOnce,
        slow: Slow,
        req_timeout: Duration,
        /// How late the relay acknowledges each EVENT.
        ok_delay: Duration,
        /// The humans' presence interval.
        presence_secs: f64,
        /// Their messages, each a thread reply, and reactions.
        messages_per_hour: f64,
        reactions_per_hour: f64,
        /// A tick shorter than a scenario may ask for.
        tick_secs: Option<f64>,
    }

    /// One client, with a co-member whose profile it reads.
    fn solo(joined: usize, desktop: &str) -> Rig {
        Rig {
            clients: 1,
            ..rig(2, joined, desktop)
        }
    }

    fn rig(humans: usize, joined: usize, desktop: &str) -> Rig {
        Rig {
            humans,
            clients: humans,
            joined,
            desktop: desktop.into(),
            plan: instant(),
            drop_first: None,
            fail: &[],
            slow: None,
            req_timeout: REQ_TIMEOUT,
            ok_delay: Duration::ZERO,
            presence_secs: 3600.0,
            messages_per_hour: 0.0,
            reactions_per_hour: 0.0,
            tick_secs: None,
        }
    }

    /// Runs the clients for at least `min`, until `done` holds, then stops
    /// them and returns what they did. Fails after 15 s without `done`.
    async fn drive(rig: Rig, min: Duration, done: impl Fn(&Outcome) -> bool) -> Outcome {
        let (ws, frames) = fake_ws_relay(rig.plan.clone(), rig.drop_first, rig.ok_delay).await;
        let (http, queries) = fake_api("d", rig.fail, rig.slow).await;
        let mut scenario = desktops(&rig, &ws, &http);
        if let Some(secs) = rig.tick_secs {
            scenario.desktop.tick_secs = secs;
        }
        let net = Net::new(&scenario).await.unwrap();
        let stats = Arc::new(Stats::default());
        let (stop_tx, stop) = watch::channel(false);
        let shared = Arc::new(Shared::new(scenario, net, stats.clone(), stop));
        let clients: Vec<_> = (0..rig.clients)
            .map(|g| tokio::spawn(run_client(shared.clone(), g, rig.req_timeout)))
            .collect();
        let started = Instant::now();
        loop {
            let now = outcome(&stats, &frames, &queries);
            if started.elapsed() >= min && done(&now) {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "never done: {:?} {:?}",
                now.ops,
                now.frames
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        stop_tx.send(true).unwrap();
        for client in clients {
            tokio::time::timeout(Duration::from_secs(5), client)
                .await
                .expect("a stopped client returns")
                .unwrap();
        }
        outcome(&stats, &frames, &queries)
    }

    fn no_errors(out: &Outcome) {
        let failed: Vec<_> = out.ops.iter().filter(|(_, c)| c.1 > 0).collect();
        assert!(failed.is_empty(), "errors: {failed:?}");
    }

    /// What one cold start reads over HTTP with two windows and a thread.
    fn cold_reads(clients: u64) -> HashMap<&'static str, u64> {
        [
            ("desktop_nip11", 1),
            ("desktop_roster", 1),
            ("desktop_metadata", 1),
            ("desktop_sidebar", 1),
            ("desktop_profiles", 1),
            ("desktop_statuses", 1),
            ("desktop_emoji", 1),
            ("desktop_read_state", 1),
            ("desktop_unread", 1),
            ("desktop_reminders", 1),
            // Per window, the head page and the page its bounds point to.
            ("desktop_window", 4),
            ("desktop_notices", 2),
            ("desktop_thread_root", 1),
            ("desktop_thread_window", 1),
        ]
        .into_iter()
        .map(|(op, n)| (op, n * clients))
        .collect()
    }

    fn with(
        mut ops: HashMap<&'static str, u64>,
        extra: &[(&'static str, u64)],
    ) -> HashMap<&'static str, u64> {
        ops.extend(extra.iter().copied());
        ops
    }

    fn ops(list: &[(&'static str, u64)]) -> HashMap<&'static str, u64> {
        list.iter().copied().collect()
    }

    /// What a channel route's establishment reads: both windows (two pages
    /// and notices each) and the thread.
    const CHANNEL_RECOVERY: [(&str, u64); 4] = [
        ("desktop_window", 4),
        ("desktop_notices", 2),
        ("desktop_thread_root", 1),
        ("desktop_thread_window", 1),
    ];

    const QUIET: &str = "tick_secs = 3600.0";

    #[tokio::test]
    async fn a_cold_start_sends_the_modeled_requests_once() {
        // Legacy: per-channel batches of ten for 25 channels; current: one
        // live-only batch and three replays. Both open the three globals
        // (each client watches the other's presence) and read the same over
        // HTTP.
        let cases = [
            ("legacy", 25, vec![("desktop_live_eose", 2 * 6)]),
            (
                "current",
                25,
                vec![("desktop_live_eose", 2 * 4), ("desktop_replay_eose", 2 * 3)],
            ),
            ("legacy", 5, vec![("desktop_live_eose", 2 * 4)]),
            ("current", 5, vec![("desktop_live_eose", 2 * 4)]),
        ];
        for (mode, joined, subs) in cases {
            let want = with(cold_reads(2), &subs);
            let toml = format!("mode = \"{mode}\"\n{QUIET}");
            let out = drive(rig(2, joined, &toml), Duration::from_millis(300), |o| {
                o.desktop() == want
            })
            .await;
            no_errors(&out);
            assert_eq!(out.desktop(), want, "{mode} J={joined}");
            assert!(out.saw("REQ", "presence"), "{mode} J={joined}");
            assert_eq!(out.count("ws_connect_auth"), 2, "{mode} J={joined}");
        }
    }

    #[tokio::test]
    async fn a_window_catch_up_follows_the_relays_cursor_until_it_is_exhausted() {
        let toml = format!("mode = \"current\"\n{QUIET}");
        let out = drive(solo(5, &toml), Duration::ZERO, |o| {
            o.count("desktop_notices") == 2
        })
        .await;
        no_errors(&out);
        let top: Vec<&Value> = out
            .queries
            .iter()
            .filter(|f| f.get("top_level").is_some())
            .collect();
        let channel = &top[0]["#h"];
        let windows: Vec<&Value> = top.into_iter().filter(|f| f["#h"] == *channel).collect();
        // The head, then the page after the head's last row (not after its
        // older aux event), and no third read of an exhausted history,
        // though that page came back full.
        assert_eq!(windows.len(), 2, "{windows:?}");
        assert!(windows[0].get("until").is_none());
        assert_eq!(windows[1]["until"], 1_000);
        let ids: Vec<&str> = out
            .queries
            .iter()
            .filter(|f| f.get("before_id").is_some())
            .filter_map(|f| f["before_id"].as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|id| id.len() == 64 && *id == ids[0]));
        assert_eq!(out.count("desktop_window"), 4);
    }

    /// Holds once `ready` has held for `settle`: what a connection reads has
    /// had time to finish.
    fn settled(ready: impl Fn(&Outcome) -> bool, settle: Duration) -> impl Fn(&Outcome) -> bool {
        let since = std::cell::Cell::new(None::<Instant>);
        move |o| {
            if !ready(o) {
                return false;
            }
            let at = since.get().unwrap_or_else(Instant::now);
            since.set(Some(at));
            at.elapsed() >= settle
        }
    }

    /// The reads beyond its cold start of a `current` client in 5 channels,
    /// once its first renewal's connection established with each replay
    /// sending 499 events, except 500 on subscriptions whose id starts
    /// with `cut`.
    async fn renewed(cut: &'static str) -> HashMap<&'static str, u64> {
        let mut rig = solo(
            5,
            "mode = \"current\"\nrenewal_secs = 4.0\ntick_secs = 3600.0",
        );
        rig.plan = Arc::new(move |conn, id| {
            let events = match conn {
                0 => 0,
                _ if id.starts_with(cut) => REPLAY_LIMIT,
                _ => REPLAY_LIMIT - 1,
            };
            (events, Duration::ZERO)
        });
        let cold = with(cold_reads(1), &[("desktop_live_eose", 3)]);
        let out = drive(
            rig,
            Duration::ZERO,
            settled(
                |o| o.count("desktop_live_eose") >= 6,
                Duration::from_millis(800),
            ),
        )
        .await;
        no_errors(&out);
        assert_eq!(out.count("ws_connect_auth"), 2, "{cut}");
        out.since(&cold)
            .into_iter()
            .filter(|(op, _)| *op != "desktop_live_eose")
            .collect()
    }

    #[tokio::test]
    async fn a_renewal_recovers_only_what_a_cut_replay_lost() {
        // 499 events everywhere: every route kept continuity.
        assert_eq!(renewed("none").await, ops(&[]));
        // A cut channel replay: the windows and the thread recover, and the
        // carried membership route only repairs unreads.
        assert_eq!(
            renewed("batch").await,
            ops(&[CHANNEL_RECOVERY.as_slice(), &[("desktop_unread", 1)]].concat())
        );
        // A cut membership replay: its full refresh, and the channels.
        assert_eq!(
            renewed("membership").await,
            ops(&[
                CHANNEL_RECOVERY.as_slice(),
                &[
                    ("desktop_roster", 1),
                    ("desktop_read_state", 1),
                    ("desktop_unread", 1),
                    ("desktop_reminders", 1),
                ]
            ]
            .concat())
        );
        // A cut profiles replay: its refresh, the channels, and the covered
        // membership route's repair.
        assert_eq!(
            renewed("profiles").await,
            ops(&[
                CHANNEL_RECOVERY.as_slice(),
                &[
                    ("desktop_profiles", 1),
                    ("desktop_statuses", 1),
                    ("desktop_emoji", 1),
                    ("desktop_unread", 1),
                ]
            ]
            .concat())
        );
    }

    /// The reads beyond its cold start of a `current` client in 5 channels
    /// whose first connection drops after it established, once the next
    /// connection established; its global replays send `globals` events.
    async fn reconnected(globals: usize) -> HashMap<&'static str, u64> {
        let mut rig = solo(5, &format!("mode = \"current\"\n{QUIET}"));
        rig.drop_first = Some(Duration::from_millis(700));
        rig.plan = Arc::new(move |conn, id| {
            let global = id == "profiles" || id == "membership";
            let events = if conn > 0 && global { globals } else { 0 };
            (events, Duration::ZERO)
        });
        let cold = with(cold_reads(1), &[("desktop_live_eose", 3)]);
        let out = drive(
            rig,
            Duration::ZERO,
            settled(
                |o| o.count("ws_connect_auth") >= 2 && o.count("desktop_live_eose") >= 6,
                Duration::from_millis(800),
            ),
        )
        .await;
        // The drop is the only error.
        let failed: Vec<_> = out
            .ops
            .iter()
            .filter(|(op, c)| c.1 > 0 && **op != "ws_connect_auth")
            .collect();
        assert!(failed.is_empty(), "errors: {failed:?}");
        assert_eq!(out.count("ws_connect_auth"), 2);
        out.since(&cold)
            .into_iter()
            .filter(|(op, _)| *op != "desktop_live_eose")
            .collect()
    }

    #[tokio::test]
    async fn a_reconnect_rereads_a_global_only_when_its_replay_was_cut() {
        // Complete replays: the globals resume, unreads are only repaired,
        // and the channels recover.
        assert_eq!(
            reconnected(REPLAY_LIMIT - 1).await,
            ops(&[
                CHANNEL_RECOVERY.as_slice(),
                &[("desktop_nip11", 1), ("desktop_unread", 1)]
            ]
            .concat())
        );
        // Truncated global replays: both globals refresh in full.
        assert_eq!(
            reconnected(REPLAY_LIMIT).await,
            ops(&[
                CHANNEL_RECOVERY.as_slice(),
                &[
                    ("desktop_nip11", 1),
                    ("desktop_profiles", 1),
                    ("desktop_statuses", 1),
                    ("desktop_emoji", 1),
                    ("desktop_roster", 1),
                    ("desktop_read_state", 1),
                    ("desktop_unread", 1),
                    ("desktop_reminders", 1),
                ]
            ]
            .concat())
        );
    }

    #[tokio::test]
    async fn read_state_snapshots_recur_under_renewals_shorter_than_the_backstop() {
        // Every renewal of 25 channels establishes (the live-only batch is
        // cut by design) and repairs unreads without a snapshot; that must
        // not hold off the snapshot's backstop.
        let toml = "mode = \"current\"\ntick_secs = 1.0\nbackstop_secs = 2.0\nrenewal_secs = 1.0";
        let out = drive(solo(25, toml), Duration::ZERO, |o| {
            o.count("desktop_read_state") >= 2 && o.count("ws_connect_auth") >= 3
        })
        .await;
        no_errors(&out);
        assert!(out.count("desktop_unread") > out.count("desktop_read_state"));
    }

    #[tokio::test]
    async fn a_replacement_past_its_deadline_is_closed_and_the_next_tick_retries() {
        let mut rig = solo(
            25,
            "mode = \"current\"\ntick_secs = 1.0\nbackstop_secs = 1.0",
        );
        rig.req_timeout = Duration::from_millis(300);
        rig.plan = Arc::new(|_, id| {
            let late = if id == "batch-2-0" {
                Duration::from_secs(1)
            } else {
                Duration::ZERO
            };
            (0, late)
        });
        let out = drive(rig, Duration::ZERO, |o| o.saw("CLOSE", "batch-1-0")).await;
        assert_eq!(out.errors("desktop_live_eose"), 1);
        // The late replacement is closed; the next tick replaces the batch.
        let closed = out
            .at("CLOSE", "batch-2-0")
            .expect("late replacement closed");
        let next = out.at("REQ", "batch-3-0").expect("next tick replaced");
        let retired = out.at("CLOSE", "batch-1-0").expect("old batch closed");
        assert!(closed < next && next < retired, "{:?}", out.frames);
        // One batch wire stays open.
        let opened = |verb: &str| {
            out.frames
                .iter()
                .filter(|(_, v, id)| v == verb && id.starts_with("batch-"))
                .count()
        };
        assert_eq!(opened("REQ") - opened("CLOSE"), 1, "{:?}", out.frames);
    }

    #[tokio::test]
    async fn a_failed_read_retries_on_the_next_tick_and_a_good_one_does_not() {
        let mut rig = solo(25, "mode = \"current\"\ntick_secs = 1.0");
        rig.fail = &["roster", "unread"];
        let out = drive(rig, Duration::from_millis(3_500), |o| {
            o.count("desktop_roster") == 1 && o.count("desktop_unread") == 1
        })
        .await;
        let ops = &out.ops;
        assert_eq!(
            (out.count("desktop_roster"), out.errors("desktop_roster")),
            (1, 1),
            "{ops:?}"
        );
        assert_eq!(
            (out.count("desktop_unread"), out.errors("desktop_unread")),
            (1, 1),
            "{ops:?}"
        );
        // The retry reread the snapshot with its repair; nothing else reran.
        assert_eq!(out.count("desktop_read_state"), 2);
        assert_eq!(out.count("desktop_reminders"), 1);
        assert_eq!(out.count("desktop_profiles"), 1);
    }

    #[tokio::test]
    async fn an_owed_snapshot_survives_a_covered_repair() {
        // The cold snapshot read fails. Before any tick, the connection
        // drops and the membership route resumes with a complete replay:
        // its repair must read the snapshot still owed.
        let mut rig = solo(5, &format!("mode = \"current\"\n{QUIET}"));
        rig.drop_first = Some(Duration::from_millis(700));
        rig.fail = &["read_state"];
        let out = drive(
            rig,
            Duration::ZERO,
            settled(
                |o| o.count("ws_connect_auth") >= 2 && o.count("desktop_live_eose") >= 6,
                Duration::from_millis(800),
            ),
        )
        .await;
        let ops = &out.ops;
        assert_eq!(
            (
                out.count("desktop_read_state"),
                out.errors("desktop_read_state")
            ),
            (1, 1),
            "{ops:?}"
        );
        // The failed snapshot held the cold repair back; the resumed route
        // repaired once and reread neither roster nor reminders.
        assert_eq!(out.count("desktop_unread"), 1, "{ops:?}");
        assert_eq!(out.count("desktop_roster"), 1, "{ops:?}");
        assert_eq!(out.count("desktop_reminders"), 1, "{ops:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_failed_read_retries_while_an_unrelated_slow_one_runs() {
        // The cold roster read fails while reminders take 3 s; the tick at
        // 1 s retries the roster without waiting for reminders.
        let mut rig = solo(5, "mode = \"current\"\ntick_secs = 1.0");
        rig.fail = &["roster"];
        rig.slow = Some(("reminders", Duration::from_secs(3)));
        let reminders = std::cell::Cell::new(None);
        let out = drive(rig, Duration::ZERO, |o| {
            let retried = o.count("desktop_roster") == 1;
            if retried && reminders.get().is_none() {
                reminders.set(Some(o.count("desktop_reminders")));
            }
            retried
        })
        .await;
        assert_eq!(
            reminders.get(),
            Some(0),
            "the roster retry waited for reminders"
        );
        assert_eq!(out.errors("desktop_roster"), 1);
        // Still slow at the stop, the reminders read is abandoned there.
        assert_eq!(out.ops.get("desktop_reminders"), None);
    }

    #[tokio::test]
    async fn a_timed_out_route_is_recovered_again_after_a_renewal() {
        // The first connection's membership EOSE never comes: the route is
        // recovered as cut, but it was never live. The renewal's complete
        // replay therefore cannot carry it, and it recovers in full again.
        // The first renewal is jittered; with this period it comes after the
        // short deadline and before the test ends.
        let mut rig = solo(
            5,
            "mode = \"current\"\nrenewal_secs = 20.0\ntick_secs = 3600.0",
        );
        rig.req_timeout = Duration::from_millis(100);
        rig.plan = Arc::new(|conn, id| {
            let withheld = conn == 0 && id == "membership";
            let delay = if withheld {
                Duration::from_secs(60)
            } else {
                Duration::ZERO
            };
            (0, delay)
        });
        let out = drive(
            rig,
            Duration::ZERO,
            settled(
                |o| o.count("ws_connect_auth") >= 2 && o.count("desktop_live_eose") >= 5,
                Duration::from_millis(800),
            ),
        )
        .await;
        let ops = &out.ops;
        assert_eq!(out.errors("desktop_live_eose"), 1, "{ops:?}");
        assert_eq!(out.count("ws_connect_auth"), 2, "{ops:?}");
        for op in ["desktop_roster", "desktop_read_state", "desktop_reminders"] {
            assert_eq!(out.count(op), 2, "{op}: {ops:?}");
        }
        // The routes that were live carried: no second profile read.
        assert_eq!(out.count("desktop_profiles"), 1, "{ops:?}");
    }

    #[tokio::test]
    async fn legacy_ticks_reread_and_replace_batches_every_tick() {
        let out = drive(
            rig(2, 25, "mode = \"legacy\"\ntick_secs = 1.0"),
            Duration::ZERO,
            |o| o.count("desktop_roster") >= 4 && o.saw("CLOSE", "batch-1-2"),
        )
        .await;
        no_errors(&out);
        // Each replacement is sent before the batch it replaces is closed.
        assert!(out.saw("REQ", "batch-2-0") && out.saw("CLOSE", "batch-1-0"));
        assert!(out.count("desktop_read_state") >= 4);
        assert!(out.count("desktop_unread") >= 4);
        // Ticks never reread names or open windows.
        assert_eq!(out.count("desktop_metadata"), 2);
        assert_eq!(out.count("desktop_window"), 8);
    }

    #[tokio::test]
    async fn current_ticks_wait_for_the_backstop() {
        let before = drive(
            rig(2, 25, "mode = \"current\"\ntick_secs = 1.0"),
            Duration::from_millis(2500),
            |o| o.count("desktop_thread_window") == 2,
        )
        .await;
        no_errors(&before);
        // Silent ticks probe, as many times as the run had silent ticks.
        let mut ops = before.desktop();
        ops.remove("desktop_probe");
        assert_eq!(
            ops,
            with(
                cold_reads(2),
                &[("desktop_live_eose", 8), ("desktop_replay_eose", 6)]
            )
        );
        assert!(!before
            .frames
            .iter()
            .any(|(_, _, id)| id.starts_with("batch-2")));

        let after = drive(
            rig(
                2,
                25,
                "mode = \"current\"\ntick_secs = 1.0\nbackstop_secs = 1.0",
            ),
            Duration::ZERO,
            |o| {
                o.count("desktop_roster") >= 4
                    && o.count("desktop_unread") >= 4
                    && o.saw("CLOSE", "batch-1-0")
            },
        )
        .await;
        no_errors(&after);
        assert!(after.saw("REQ", "batch-2-0"));
        assert!(after.count("desktop_unread") >= 4);
        // A replacement is live-only: no replays after the connect's three.
        assert_eq!(after.count("desktop_replay_eose"), 6);
    }

    #[tokio::test]
    async fn legacy_renewals_are_full_reconnects() {
        let out = drive(
            rig(2, 25, "mode = \"legacy\"\nrenewal_secs = 1.0\n"),
            Duration::ZERO,
            |o| o.count("desktop_nip11") >= 4 && o.count("desktop_roster") >= 4,
        )
        .await;
        no_errors(&out);
        assert!(out.count("ws_connect_auth") >= 4);
        // A reconnect keeps channel names and sidebar preferences.
        assert_eq!(out.count("desktop_metadata"), 2);
        assert_eq!(out.count("desktop_sidebar"), 2);
    }

    #[tokio::test]
    async fn current_renewals_carry_small_sessions_and_recover_channels_in_large_ones() {
        // More than ten channels: the live-only batch is cut, so replays,
        // channel catch-up and an unread repair again, but no global reads.
        let large = drive(
            rig(2, 25, "mode = \"current\"\nrenewal_secs = 1.0\n"),
            Duration::ZERO,
            |o| {
                o.count("desktop_replay_eose") >= 12
                    && o.count("desktop_unread") >= 4
                    && o.count("ws_connect_auth") >= 4
                    && o.count("desktop_window") >= 16
            },
        )
        .await;
        no_errors(&large);
        assert!(large.count("ws_connect_auth") >= 4);
        for op in [
            "desktop_nip11",
            "desktop_roster",
            "desktop_metadata",
            "desktop_profiles",
            "desktop_read_state",
        ] {
            assert_eq!(large.count(op), 2, "{op}");
        }
        assert!(large.count("desktop_window") >= 16);

        // Up to ten channels: subscriptions are re-sent and nothing is read,
        // but for one unread repair when a client's first renewal (jittered)
        // came before its cold connection caught up.
        let small = drive(
            rig(2, 5, "mode = \"current\"\nrenewal_secs = 1.0\n"),
            Duration::ZERO,
            |o| {
                o.count("desktop_live_eose") >= 12
                    && o.count("ws_connect_auth") >= 4
                    && o.count("desktop_thread_window") == 2
            },
        )
        .await;
        no_errors(&small);
        assert!(small.count("ws_connect_auth") >= 4);
        let reads: HashMap<_, _> = small
            .desktop()
            .into_iter()
            .filter(|(op, _)| *op != "desktop_live_eose" && *op != "desktop_unread")
            .collect();
        let mut cold = cold_reads(2);
        cold.remove("desktop_unread");
        assert_eq!(reads, cold);
        assert!(small.count("desktop_unread") <= 4);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_read_in_flight_at_the_stop_is_abandoned_not_waited_for() {
        // The roster read takes far longer than the stop may: `drive` gives
        // a stopped client 5 s to return.
        let mut rig = solo(5, &format!("mode = \"current\"\n{QUIET}"));
        rig.slow = Some(("roster", Duration::from_secs(8)));
        let out = drive(rig, Duration::ZERO, |o| {
            o.queries.iter().any(|f| f["kinds"] == json!([39002]))
        })
        .await;
        // Abandoned, like an EOSE wait at the stop: neither a success nor
        // an error.
        assert_eq!(out.count("desktop_roster"), 0);
        assert_eq!(out.errors("desktop_roster"), 0);
    }

    #[tokio::test]
    async fn a_cancelled_finish_still_aborts_the_read_task() {
        let (alive, ended) = tokio::sync::oneshot::channel::<()>();
        let mut fetcher = Fetcher::default();
        fetcher.task = Some(tokio::spawn(async move {
            let _alive = alive;
            std::future::pending::<()>().await;
        }));
        let finishing = tokio::spawn(async move { fetcher.finish().await });
        tokio::task::yield_now().await;
        finishing.abort();
        let _ = finishing.await;
        let ended = tokio::time::timeout(Duration::from_secs(2), ended).await;
        assert!(ended.expect("the read task ended").is_err());
    }

    /// One client whose four routes each get their EOSE `eose_after` the
    /// REQ, while the first presence heartbeat (within a second of the
    /// connect) waits 3.5 s for its OK, past the 2.5 s deadline. Every EOSE
    /// arrives while the client waits for that OK.
    async fn eoses_behind_a_slow_ok(eose_after: Duration) -> Outcome {
        let mut rig = solo(5, &format!("mode = \"current\"\n{QUIET}"));
        rig.presence_secs = 1.0;
        rig.ok_delay = Duration::from_millis(3_500);
        rig.req_timeout = Duration::from_millis(2_500);
        rig.plan = Arc::new(move |_, _| (0, eose_after));
        drive(rig, Duration::ZERO, |o| {
            let eoses = o.count("desktop_live_eose") + o.errors("desktop_live_eose");
            o.count("publish_ephemeral_ok") >= 1 && eoses >= 4
        })
        .await
    }

    #[tokio::test]
    async fn an_eose_in_time_is_not_expired_behind_a_slow_ok() {
        let out = eoses_behind_a_slow_ok(Duration::from_millis(1_200)).await;
        assert_eq!(out.errors("desktop_live_eose"), 0, "{:?}", out.ops);
        assert_eq!(out.count("desktop_live_eose"), 4, "{:?}", out.ops);
        assert!(!out.saw("CLOSE", "batch-1-0"), "{:?}", out.frames);
    }

    #[tokio::test]
    async fn a_late_eose_is_not_answered_behind_a_slow_ok() {
        // Read only once the OK comes, but they came after the deadline.
        let out = eoses_behind_a_slow_ok(Duration::from_millis(3_000)).await;
        assert_eq!(out.errors("desktop_live_eose"), 4, "{:?}", out.ops);
        assert_eq!(out.count("desktop_live_eose"), 0, "{:?}", out.ops);
    }

    #[tokio::test]
    async fn a_silent_tick_probes_and_an_unanswered_probe_reconnects() {
        let mut rig = solo(5, "mode = \"current\"\ntick_secs = 1.0");
        rig.req_timeout = Duration::from_millis(500);
        // The first connection never answers its probe.
        rig.plan = Arc::new(|conn, id| {
            let late = conn == 0 && id.starts_with("probe-");
            (0, Duration::from_secs(if late { 30 } else { 0 }))
        });
        let out = drive(rig, Duration::ZERO, |o| o.count("desktop_probe") >= 1).await;
        assert_eq!(out.errors("desktop_probe"), 1, "{:?}", out.ops);
        let probes: Vec<_> = out
            .frames
            .iter()
            .filter(|(_, v, id)| v == "REQ" && id.starts_with("probe-"))
            .map(|(conn, _, id)| (*conn, id.as_str()))
            .collect();
        assert_eq!(probes[0], (0, "probe-1"), "{:?}", out.frames);
        assert!(
            probes.iter().any(|(conn, _)| *conn == 1),
            "{:?}",
            out.frames
        );
        assert_eq!(out.count("ws_connect_auth"), 2, "{:?}", out.ops);
    }

    #[test]
    fn a_backlog_defers_deadlines_for_at_most_one_timeout() {
        let t = Duration::from_secs(10);
        let mut since = None;
        // Nothing past due, or nothing in hand: nothing to defer.
        assert!(!defer(&mut since, true, false, t));
        assert!(!defer(&mut since, false, true, t));
        assert!(since.is_none());
        // A backlog may hold the EOSEs: wait for them, however late the
        // first deferring pass comes, but not forever.
        assert!(defer(&mut since, true, true, t));
        assert!(since.is_some());
        since = Instant::now().checked_sub(Duration::from_secs(9));
        assert!(defer(&mut since, true, true, t));
        since = Instant::now().checked_sub(t);
        assert!(!defer(&mut since, true, true, t));
        // A drained backlog starts the next deferral afresh.
        assert!(!defer(&mut since, false, true, t));
        assert!(since.is_none());
    }

    #[tokio::test]
    async fn a_long_backlog_behind_slow_oks_keeps_its_timely_eoses() {
        // Every REQ answers 1.2 s late with 600 events, more than one pass
        // drains, and its EOSE, while heartbeats each second wait 3.5 s for
        // OKs, more than twice the 1.5 s deadline. The deferral runs from
        // the first pass that defers, not from the REQ, and no heartbeat may
        // run between the backlog's passes: a second OK wait would outlast it.
        let mut rig = solo(5, &format!("mode = \"current\"\n{QUIET}"));
        rig.presence_secs = 1.0;
        rig.ok_delay = Duration::from_millis(3_500);
        rig.req_timeout = Duration::from_millis(1_500);
        rig.plan = Arc::new(|_, _| (600, Duration::from_millis(1_200)));
        let out = drive(rig, Duration::ZERO, |o| {
            o.count("publish_ephemeral_ok") >= 2 && o.count("desktop_live_eose") >= 4
        })
        .await;
        assert_eq!(out.errors("desktop_live_eose"), 0, "{:?}", out.ops);
        assert_eq!(out.count("desktop_live_eose"), 4, "{:?}", out.ops);
        assert!(!out.saw("CLOSE", "batch-1-0"), "{:?}", out.frames);
    }

    #[tokio::test]
    async fn a_probe_answered_behind_a_late_ok_does_not_reconnect() {
        // Ticks every 0.25 s; a heartbeat every 1 s waits 0.5 s for its OK.
        // The tick after an OK hears it, so the next one, 0.25 s before the
        // next heartbeat, probes. The probe's EOSE comes 0.3 s later, while
        // that heartbeat waits, and is read once its OK comes, after the
        // probe's 0.7 s deadline.
        let mut rig = solo(5, "mode = \"current\"");
        rig.tick_secs = Some(0.25);
        rig.presence_secs = 1.0;
        rig.ok_delay = Duration::from_millis(500);
        rig.req_timeout = Duration::from_millis(700);
        rig.plan = Arc::new(|_, id| {
            let late = if id.starts_with("probe-") { 300 } else { 0 };
            (0, Duration::from_millis(late))
        });
        let out = drive(rig, Duration::ZERO, |o| o.saw("REQ", "probe-3")).await;
        assert_eq!(out.errors("desktop_probe"), 0, "{:?}", out.ops);
        assert!(out.count("desktop_probe") >= 1, "{:?}", out.ops);
        assert_eq!(out.count("ws_connect_auth"), 1, "{:?}", out.ops);
        assert!(
            out.frames.iter().all(|(conn, ..)| *conn == 0),
            "{:?}",
            out.frames
        );
    }

    #[tokio::test]
    async fn a_probe_answered_late_behind_a_slow_ok_reconnects() {
        // Ticks every 0.25 s; a heartbeat every 1 s waits 0.6 s for its OK.
        // The tick after an OK hears it, so the next one probes, 0.15 s
        // before the next heartbeat. The first probe may come before any
        // heartbeat and is answered at once. On the first connection, every
        // later probe's EOSE comes 0.6 s after it, past its 0.4 s deadline,
        // while that heartbeat waits; the OK itself comes later still.
        // Neither answers.
        let mut rig = solo(5, "mode = \"current\"");
        rig.tick_secs = Some(0.25);
        rig.presence_secs = 1.0;
        rig.ok_delay = Duration::from_millis(600);
        rig.req_timeout = Duration::from_millis(400);
        rig.plan = Arc::new(|conn, id| {
            let late = conn == 0 && id.starts_with("probe-") && id != "probe-1";
            (0, Duration::from_millis(if late { 600 } else { 0 }))
        });
        let out = drive(rig, Duration::ZERO, |o| {
            o.errors("desktop_probe") >= 1 && o.count("ws_connect_auth") >= 2
        })
        .await;
        assert_eq!(out.errors("desktop_probe"), 1, "{:?}", out.ops);
        // The probe that failed went out before a heartbeat on its
        // connection, whose OK wait held its EOSE.
        let first = out.at("REQ", "probe-2").expect("probe-2 was sent");
        assert!(
            out.frames[first..]
                .iter()
                .any(|(conn, verb, id)| *conn == 0 && verb == "EVENT" && id == "20001"),
            "{:?}",
            out.frames
        );
    }

    #[tokio::test]
    async fn an_answered_probe_is_closed_at_its_eose() {
        // The app closes a probe at its EOSE, not when it sends the next.
        let rig = solo(5, "mode = \"current\"\ntick_secs = 1.0");
        let out = drive(rig, Duration::ZERO, |o| o.saw("CLOSE", "probe-1")).await;
        assert_eq!(out.count("desktop_probe"), 1, "{:?}", out.ops);
        assert!(!out.saw("REQ", "probe-2"), "{:?}", out.frames);
    }

    #[tokio::test]
    async fn a_publish_ok_in_time_answers_an_outstanding_probe() {
        // No probe ever gets its EOSE. Ticks every 0.5 s; a heartbeat every
        // second waits 0.2 s for its OK. A probe follows a silent tick, and
        // the next heartbeat's OK comes well inside its 3 s deadline.
        let mut rig = solo(5, "mode = \"current\"");
        rig.tick_secs = Some(0.5);
        rig.presence_secs = 1.0;
        rig.ok_delay = Duration::from_millis(200);
        rig.req_timeout = Duration::from_secs(3);
        rig.plan = Arc::new(|_, id| {
            let late = if id.starts_with("probe-") { 60 } else { 0 };
            (0, Duration::from_secs(late))
        });
        let out = drive(rig, Duration::ZERO, |o| o.saw("REQ", "probe-2")).await;
        assert_eq!(out.errors("desktop_probe"), 0, "{:?}", out.ops);
        assert!(out.count("desktop_probe") >= 1, "{:?}", out.ops);
        assert_eq!(out.count("ws_connect_auth"), 1, "{:?}", out.ops);
    }

    #[tokio::test]
    async fn a_reaction_with_no_target_is_not_heard_as_a_frame() {
        // Reactions come several times a second but have nothing to react
        // to, so nothing is sent and nothing comes back: a silent tick
        // still probes.
        let mut rig = solo(5, "mode = \"current\"");
        rig.tick_secs = Some(0.5);
        rig.reactions_per_hour = 36_000.0;
        let out = drive(rig, Duration::ZERO, |o| o.count("desktop_probe") >= 1).await;
        assert_eq!(out.errors("desktop_probe"), 0, "{:?}", out.ops);
        assert!(!out.saw("EVENT", "7"), "{:?}", out.frames);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_reply_whose_root_lookup_the_stop_cut_short_is_not_published() {
        // The first message is a reply whose root search outlasts the stop
        // may; its OK would never come in time either.
        let mut rig = solo(5, &format!("mode = \"current\"\n{QUIET}"));
        rig.messages_per_hour = 36_000.0;
        rig.slow = Some(("roots", Duration::from_secs(8)));
        rig.ok_delay = Duration::from_secs(60);
        let out = drive(rig, Duration::from_secs(1), |o| {
            o.queries
                .iter()
                .any(|f| f["kinds"] == json!([9]) && f.get("authors").is_some())
        })
        .await;
        assert!(!out.saw("EVENT", "9"), "{:?}", out.frames);
        assert_eq!(out.count("http_query"), 0, "{:?}", out.ops);
        assert_eq!(out.errors("http_query"), 0, "{:?}", out.ops);
    }
}
