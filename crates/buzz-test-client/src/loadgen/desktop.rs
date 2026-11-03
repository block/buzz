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

use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};
use futures_util::stream::{self, StreamExt};
use nostr::Keys;
use serde_json::{json, Value};

use super::identity::{self, Role};
use super::net::http_reason;
use super::run::{
    connect, observe, roots, stopped, Activity, Backoff, Directory, Rng, Seen, Shared, Step,
    MAX_IDLE,
};
use super::scenario::DesktopMode;
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
/// How long a REQ may wait for its EOSE.
const REQ_TIMEOUT: Duration = Duration::from_secs(10);
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
/// Channels per unread repair read.
const UNREAD_CHANNELS: usize = 128;
/// Events per channel window page.
const WINDOW_PAGE: usize = 20;
/// Older window pages a catch-up reads after the head, at most.
const CATCH_UP_PAGES: usize = 5;
/// The app's custom emoji set.
const EMOJI_SET: &str = "buzz:custom-emoji";

/// One REQ frame.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Req {
    pub(super) id: String,
    pub(super) filters: Vec<Value>,
    /// A replay subscription, closed at its EOSE. Others stay open.
    pub(super) replay: bool,
}

impl Req {
    fn frame(&self) -> Value {
        let mut frame = vec![json!("REQ"), json!(self.id)];
        frame.extend(self.filters.iter().cloned());
        Value::Array(frame)
    }
}

fn channel_filter(ids: &[String], since: u64, limit: usize) -> Value {
    json!({"kinds": CHANNEL_KINDS, "#h": ids, "since": since, "limit": limit})
}

/// The two global subscriptions, profiles and membership.
fn global_reqs(viewer: &str, since: u64) -> [Req; 2] {
    let limit = REPLAY_LIMIT;
    [
        Req {
            id: "profiles".into(),
            filters: vec![
                json!({"kinds": [0, 10100, 30177], "since": since, "limit": limit}),
                json!({"kinds": [30315], "#d": ["general"], "since": since, "limit": limit}),
                json!({"kinds": [30030], "#d": [EMOJI_SET], "since": since, "limit": limit}),
            ],
            replay: false,
        },
        Req {
            id: "membership".into(),
            filters: vec![
                json!({"kinds": [44100, 44101], "#p": [viewer], "since": since, "limit": limit}),
                json!({"kinds": [44100, 44101], "#p": [viewer], "#t": ["system"], "since": since, "limit": limit}),
                json!({"kinds": [30078], "authors": [viewer], "#t": ["read-state"], "since": since, "limit": limit}),
                json!({"kinds": [30300], "authors": [viewer], "since": since, "limit": limit}),
            ],
            replay: false,
        },
    ]
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
                replay: false,
            })
            .collect()
    } else {
        channels
            .chunks(LIVE_BATCH)
            .enumerate()
            .map(|(i, chunk)| Req {
                id: id(i),
                filters: vec![channel_filter(chunk, since, 0)],
                replay: false,
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
            replay: true,
        })
        .collect()
}

/// Every REQ a connection sends while it establishes, in order.
pub(super) fn connect_reqs(
    mode: DesktopMode,
    viewer: &str,
    channels: &[String],
    since: u64,
    generation: u64,
) -> Vec<Req> {
    let mut reqs = Vec::from(global_reqs(viewer, since));
    reqs.extend(batch_reqs(mode, channels, since, REPLAY_LIMIT, generation));
    reqs.extend(replay_reqs(mode, channels, since, generation));
    reqs
}

/// Why a connection is being established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Situation {
    /// The client's first connection.
    Cold,
    /// A reconnect whose replay is not continuous. Everything is read again
    /// except channel names and sidebar preferences, which the client
    /// already holds.
    Reconnect,
    /// A `current` renewal with every channel in one replaying batch: the
    /// subscriptions are re-sent and nothing is read.
    QuietRenewal,
    /// A `current` renewal whose batches are live-only (more than 10
    /// channels). Channel routes establish again, so windows, threads and
    /// unreads are refreshed; global reads are skipped.
    LimitedRenewal,
}

/// What an identity renewal is for a client in `mode` with `joined` channels.
/// Before the fixes, it was an ordinary reconnect.
pub(super) fn renewal(mode: DesktopMode, joined: usize) -> Situation {
    match mode {
        DesktopMode::Legacy => Situation::Reconnect,
        DesktopMode::Current if joined <= REPLAY_BATCH => Situation::QuietRenewal,
        DesktopMode::Current => Situation::LimitedRenewal,
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
    ReadState,
    Unread,
    Reminders,
    Window(usize),
    Thread(usize),
}

/// The reads a connection makes once established in `situation`, with
/// `windows` channel windows and `threads` thread views open.
pub(super) fn connect_fetches(situation: Situation, windows: usize, threads: usize) -> Vec<Fetch> {
    let mut reads = match situation {
        Situation::Cold => vec![
            Fetch::Nip11,
            Fetch::Roster,
            Fetch::Metadata,
            Fetch::Sidebar,
            Fetch::Profiles,
            Fetch::Statuses,
            Fetch::Emoji,
            Fetch::ReadState,
            Fetch::Unread,
            Fetch::Reminders,
        ],
        Situation::Reconnect => vec![
            Fetch::Nip11,
            Fetch::Roster,
            Fetch::Profiles,
            Fetch::Statuses,
            Fetch::Emoji,
            Fetch::ReadState,
            Fetch::Unread,
            Fetch::Reminders,
        ],
        Situation::QuietRenewal => return Vec::new(),
        Situation::LimitedRenewal => vec![Fetch::Unread],
    };
    reads.extend((0..windows).map(Fetch::Window));
    reads.extend((0..threads).map(Fetch::Thread));
    reads
}

/// What a recovery tick re-sends or rereads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Due {
    /// Replace every channel batch with a live-only copy.
    pub(super) batches: bool,
    pub(super) roster: bool,
    /// Read state and unread repair.
    pub(super) unread: bool,
}

/// A tick's work. `legacy` does all of it every tick. `current` waits for
/// `backstop` since each was last done.
pub(super) fn tick(
    mode: DesktopMode,
    backstop: Duration,
    since_batches: Duration,
    since_roster: Duration,
    since_unread: Duration,
) -> Due {
    match mode {
        DesktopMode::Legacy => Due {
            batches: true,
            roster: true,
            unread: true,
        },
        DesktopMode::Current => Due {
            batches: since_batches >= backstop,
            roster: since_roster >= backstop,
            unread: since_unread >= backstop,
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
        reads.extend([Fetch::ReadState, Fetch::Unread]);
    }
    reads
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

/// What a client's reads use. Its sessions and read tasks share it.
struct Reader {
    shared: Arc<Shared>,
    community: usize,
    keys: Keys,
    viewer: String,
    channels: Vec<String>,
    profiles: Vec<String>,
    windows: usize,
    /// The relay's own pubkey from NIP-11 `self`, the author of notices.
    relay: std::sync::Mutex<Option<String>>,
    /// When the last unread repair started (Unix seconds).
    unread_mark: std::sync::Mutex<Option<u64>>,
}

impl Reader {
    /// One `POST /query`, timed as `op`. Returns the events, or `None` once
    /// it failed (recorded) or the run is stopping (not started).
    async fn query(&self, op: Op, filters: Value) -> Option<Vec<Value>> {
        if stopped(&self.shared) {
            return None;
        }
        let shared = &self.shared;
        let base = &shared.communities[self.community].http_url;
        let body = filters.to_string().into_bytes();
        let pending = shared.stats.begin(op);
        let response = tokio::time::timeout(
            HTTP_TIMEOUT,
            shared.net.post(&self.keys, base, "/query", body, None),
        )
        .await;
        match response {
            Err(_) => pending.err("read timeout"),
            Ok(Err(e)) => pending.err(&e.to_string()),
            Ok(Ok((status, text))) if !status.is_success() => {
                pending.err(&http_reason(status, &text))
            }
            Ok(Ok((_, text))) => match serde_json::from_str::<Vec<Value>>(&text) {
                Ok(events) => {
                    pending.ok();
                    return Some(events);
                }
                Err(e) => pending.err(&format!("decode: {e}")),
            },
        }
        None
    }

    async fn fetch(&self, fetch: Fetch) {
        let viewer = &self.viewer;
        match fetch {
            Fetch::Nip11 => self.nip11().await,
            Fetch::Roster => {
                let mut filter = json!({"kinds": [39002], "#p": [viewer], "limit": PAGE});
                for _ in 0..MAX_ROSTER_PAGES {
                    let Some(page) = self.query(Op::DesktopRoster, json!([filter])).await else {
                        break;
                    };
                    match oldest(&page) {
                        Some(until) if page.len() >= PAGE => filter["until"] = json!(until),
                        _ => break,
                    }
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
                for authors in self.profiles.chunks(PAGE) {
                    let filter = json!([{"kinds": [0], "authors": authors, "limit": PAGE}]);
                    self.query(Op::DesktopProfiles, filter).await;
                }
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
            Fetch::ReadState => {
                let filter =
                    json!([{"kinds": [30078], "authors": [viewer], "read_state_snapshot": 1}]);
                self.query(Op::DesktopReadState, filter).await;
            }
            Fetch::Unread => {
                let now = unix_now();
                let mark = lock(&self.unread_mark).replace(now);
                // Sequential, as the app repairs one chunk after another.
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
                        break;
                    }
                }
            }
            Fetch::Reminders => {
                let filter = json!([{"kinds": [30300], "authors": [viewer], "limit": 201}]);
                self.query(Op::DesktopReminders, filter).await;
            }
            Fetch::Window(i) => self.window(&self.channels[i]).await,
            Fetch::Thread(i) => self.thread(i).await,
        }
    }

    /// `GET /` for the relay's NIP-11 document, remembering its `self`.
    async fn nip11(&self) {
        if stopped(&self.shared) {
            return;
        }
        let shared = &self.shared;
        let base = &shared.communities[self.community].http_url;
        let pending = shared.stats.begin(Op::DesktopNip11);
        match tokio::time::timeout(HTTP_TIMEOUT, shared.net.relay_info(base)).await {
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

    /// A window catch-up: the head page, then older pages while each comes
    /// back full (at most [`CATCH_UP_PAGES`]), then the channel's notices.
    async fn window(&self, channel: &str) {
        let mut filter = json!({
            "kinds": WINDOW_KINDS, "#h": [channel], "limit": WINDOW_PAGE,
            "top_level": true, "include_aux": true, "include_summaries": true,
        });
        for _ in 0..=CATCH_UP_PAGES {
            let Some(page) = self.query(Op::DesktopWindow, json!([filter])).await else {
                break;
            };
            match oldest(&page) {
                Some(until) if page.len() >= WINDOW_PAGE => filter["until"] = json!(until),
                _ => break,
            }
        }
        let mut notices = json!({"kinds": [40099], "#h": [channel], "limit": REPLAY_LIMIT});
        if let Some(relay) = lock(&self.relay).clone() {
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
        let found = roots(shared, community, &self.keys, channel).await;
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

/// `created_at` of the oldest event in a page.
fn oldest(page: &[Value]) -> Option<u64> {
    page.iter().filter_map(|e| e["created_at"].as_u64()).min()
}

/// Runs `fetches`: NIP-11 first, since notices need the relay's pubkey, then
/// the rest at most [`HTTP_CONCURRENCY`] at a time.
async fn run_fetches(reader: Arc<Reader>, fetches: Vec<Fetch>) {
    if fetches.contains(&Fetch::Nip11) {
        reader.nip11().await;
    }
    stream::iter(fetches.into_iter().filter(|f| *f != Fetch::Nip11))
        .map(|fetch| {
            let reader = reader.clone();
            async move { reader.fetch(fetch).await }
        })
        .buffer_unordered(HTTP_CONCURRENCY)
        .collect::<Vec<()>>()
        .await;
}

/// A client's HTTP reads: one task at a time, so reads never pile up behind
/// a slow relay. Reads that come due while it runs are queued, once each,
/// for the next task. Dropping it (the client was cancelled) aborts the task,
/// so its unfinished reads are recorded.
#[derive(Default)]
struct Fetcher {
    task: Option<tokio::task::JoinHandle<()>>,
    queued: Vec<Fetch>,
}

impl Fetcher {
    fn start(&mut self, reader: &Arc<Reader>, fetches: Vec<Fetch>) {
        for fetch in fetches {
            if !self.queued.contains(&fetch) {
                self.queued.push(fetch);
            }
        }
        self.poll(reader);
    }

    /// Starts the queued reads once the previous task finished.
    fn poll(&mut self, reader: &Arc<Reader>) {
        if self.queued.is_empty() || self.task.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let fetches = std::mem::take(&mut self.queued);
        self.task = Some(tokio::spawn(run_fetches(reader.clone(), fetches)));
    }

    /// Waits for the task under way. Reads still queued are dropped.
    async fn finish(&mut self) {
        if let Some(task) = self.task.take() {
            let _ = task.await;
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
    replay: bool,
    pending: Pending<'a>,
    sent: Instant,
}

/// A desktop client. Its sessions differ by [`Situation`]: the first is
/// cold, a renewal follows [`renewal`], and any other reconnect is a
/// [`Situation::Reconnect`].
pub(super) async fn desktop(shared: Arc<Shared>, global: usize) {
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
    });
    let mut client = Client {
        shared: shared.clone(),
        reader,
        rng: Rng::new(global as u64 ^ 0xD35C_70B0),
        fetcher: Fetcher::default(),
        situation: Situation::Cold,
        roster_at: Instant::now(),
        unread_at: Instant::now(),
        generation: 0,
        first_renewal: true,
        last_message: None,
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
            End::Renewal => renewal(profile.mode, client.reader.channels.len()),
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
    roster_at: Instant,
    unread_at: Instant,
    generation: u64,
    first_renewal: bool,
    last_message: Option<Seen>,
}

impl Client {
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

        self.generation += 1;
        let reqs = connect_reqs(mode, &reader.viewer, channels, since, self.generation);
        let mut batches: Vec<String> = reqs
            .iter()
            .filter(|r| r.id.starts_with("batch-"))
            .map(|r| r.id.clone())
            .collect();
        let mut batches_at = Instant::now();
        let mut setup: VecDeque<Req> = reqs.into();
        let reads = connect_fetches(self.situation, reader.windows, profile.threads);
        if reads.contains(&Fetch::Roster) {
            self.roster_at = Instant::now();
        }
        if reads.contains(&Fetch::Unread) {
            self.unread_at = Instant::now();
        }
        self.fetcher.start(&reader, reads);

        let mut waiting: HashMap<String, Waiting<'_>> = HashMap::new();
        let mut open: HashSet<String> = HashSet::new();
        // New batch id to the batch it replaces, closed at the new one's EOSE.
        let mut replacing: HashMap<String, String> = HashMap::new();
        let mut activity = Activity::new(&scenario.human, &mut self.rng);
        let tick_every = Duration::from_secs_f64(profile.tick_secs);
        let backstop = Duration::from_secs_f64(profile.backstop_secs);
        let mut next_tick = Instant::now() + tick_every;
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
                let Some(req) = setup.pop_front() else {
                    break;
                };
                if let Err(e) = conn.send_raw(&req.frame()).await {
                    break 'session disconnected(e);
                }
                let op = if req.replay {
                    Op::DesktopReplay
                } else {
                    Op::DesktopLive
                };
                let waited = Waiting {
                    replay: req.replay,
                    pending: stats.begin(op),
                    sent: Instant::now(),
                };
                waiting.insert(req.id, waited);
            }
            let expired: Vec<String> = waiting
                .iter()
                .filter(|(_, w)| w.sent.elapsed() >= REQ_TIMEOUT)
                .map(|(id, _)| id.clone())
                .collect();
            for id in expired {
                let Some(w) = waiting.remove(&id) else {
                    continue;
                };
                w.pending.err("EOSE timeout");
                // A late live subscription may still deliver; a replay is abandoned.
                if w.replay {
                    if let Err(e) = close(conn, &id).await {
                        break 'session disconnected(e);
                    }
                } else {
                    open.insert(id);
                }
            }
            self.fetcher.poll(&reader);

            let due = [activity.due(), Some(next_tick), next_renewal]
                .into_iter()
                .flatten()
                .min();
            let wait = due
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(MAX_IDLE)
                .min(MAX_IDLE);
            match conn.next_event(wait).await {
                Ok(RelayMessage::Eose { subscription_id }) => {
                    if let Some(w) = waiting.remove(&subscription_id) {
                        w.pending.ok();
                        let retired = if w.replay {
                            Some(subscription_id)
                        } else {
                            let old = replacing.remove(&subscription_id);
                            open.insert(subscription_id);
                            old.inspect(|old| {
                                open.remove(old);
                            })
                        };
                        if let Some(id) = retired {
                            if let Err(e) = close(conn, &id).await {
                                break 'session disconnected(e);
                            }
                        }
                    }
                }
                Ok(RelayMessage::Closed {
                    subscription_id,
                    message,
                }) => {
                    // Live subscriptions the relay refuses end the session,
                    // as for the basic human; a refused replay is an error.
                    if let Some(w) = waiting.remove(&subscription_id) {
                        w.pending.err(&message);
                        if !w.replay {
                            break End::Failed;
                        }
                    } else if open.remove(&subscription_id) {
                        stats.err(
                            Op::DesktopLive,
                            &format!("CLOSED {subscription_id}: {message}"),
                        );
                        break End::Failed;
                    }
                }
                Ok(message) => {
                    // Only live events after EOSE are delivery; replays are history.
                    let live = matches!(
                        &message,
                        RelayMessage::Event { subscription_id, .. } if open.contains(subscription_id)
                    );
                    if live {
                        if let Some(target) = observe(stats, &message) {
                            self.last_message = Some(target);
                        }
                    }
                }
                Err(WsClientError::Timeout) => {}
                Err(e) => break disconnected(e),
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
                Ok(Step::SessionEnd) => break End::Dropped,
                Err(e) => break disconnected(e),
            }

            let now = Instant::now();
            if now >= next_tick {
                next_tick = now + tick_every;
                let due = tick(
                    mode,
                    backstop,
                    batches_at.elapsed(),
                    self.roster_at.elapsed(),
                    self.unread_at.elapsed(),
                );
                // A replacement still waiting for its EOSE is not replaced again.
                if due.batches && replacing.is_empty() {
                    self.generation += 1;
                    let fresh = batch_reqs(mode, channels, unix_now(), 0, self.generation);
                    for (new, old) in fresh.iter().zip(&batches) {
                        replacing.insert(new.id.clone(), old.clone());
                    }
                    batches = fresh.iter().map(|r| r.id.clone()).collect();
                    setup.extend(fresh);
                    batches_at = now;
                }
                if due.roster {
                    self.roster_at = now;
                }
                if due.unread {
                    self.unread_at = now;
                }
                self.fetcher.start(&reader, tick_fetches(due));
            }
            if next_renewal.is_some_and(|t| now >= t) {
                break End::Renewal;
            }
        };
        // Subscriptions still setting up are abandoned with the socket, not failed.
        for (_, w) in waiting.drain() {
            w.pending.forget();
        }
        end
    }
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

    /// (filters, limit of the first filter, `#h` ids of the first filter) per REQ.
    fn shape(reqs: &[Req]) -> Vec<(usize, u64, usize, bool)> {
        reqs.iter()
            .map(|r| {
                let first = &r.filters[0];
                (
                    r.filters.len(),
                    first["limit"].as_u64().unwrap(),
                    first["#h"].as_array().map_or(0, Vec::len),
                    r.replay,
                )
            })
            .collect()
    }

    #[test]
    fn up_to_ten_channels_share_one_replaying_batch_in_both_modes() {
        for mode in [DesktopMode::Legacy, DesktopMode::Current] {
            let reqs = connect_reqs(mode, "v", &ids(5), 100, 1);
            let names: Vec<&str> = reqs.iter().map(|r| r.id.as_str()).collect();
            assert_eq!(names, ["profiles", "membership", "batch-1-0"], "{mode:?}");
            assert_eq!(shape(&reqs[2..]), [(5, 500, 1, false)], "{mode:?}");
            assert_eq!(reqs[2].filters[0]["since"], 100);
            assert_eq!(reqs[2].filters[0]["kinds"], json!(CHANNEL_KINDS));
        }
    }

    #[test]
    fn more_than_ten_channels_batch_by_ten_before_and_replay_separately_after() {
        let legacy = connect_reqs(DesktopMode::Legacy, "v", &ids(25), 100, 1);
        assert_eq!(
            shape(&legacy[2..]),
            [(10, 500, 1, false), (10, 500, 1, false), (5, 500, 1, false)]
        );
        let current = connect_reqs(DesktopMode::Current, "v", &ids(25), 100, 1);
        assert_eq!(
            shape(&current[2..]),
            [
                (1, 0, 25, false),
                (10, 500, 1, true),
                (10, 500, 1, true),
                (5, 500, 1, true)
            ]
        );
        // One live-only filter holds up to 2048 channels.
        let wide = connect_reqs(DesktopMode::Current, "v", &ids(2049), 100, 1);
        let live: Vec<_> = wide[2..].iter().filter(|r| !r.replay).collect();
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
    fn legacy_ticks_do_everything_and_current_ticks_wait_for_the_backstop() {
        let backstop = Duration::from_secs(900);
        let minute = Duration::from_secs(60);
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
        assert_eq!(
            tick(DesktopMode::Legacy, backstop, minute, minute, minute),
            all
        );
        assert_eq!(
            tick(DesktopMode::Current, backstop, minute, minute, minute),
            none
        );
        assert_eq!(
            tick(DesktopMode::Current, backstop, backstop, backstop, backstop),
            all
        );
        // Each backstop runs on its own clock.
        let roster_only = tick(DesktopMode::Current, backstop, minute, backstop, minute);
        assert_eq!(tick_fetches(roster_only), [Fetch::Roster]);
        assert_eq!(
            tick_fetches(all),
            [Fetch::Roster, Fetch::ReadState, Fetch::Unread]
        );
        assert!(tick_fetches(none).is_empty());
    }

    #[test]
    fn renewals_reconnect_before_and_carry_or_limit_after() {
        assert_eq!(renewal(DesktopMode::Legacy, 5), Situation::Reconnect);
        assert_eq!(renewal(DesktopMode::Legacy, 25), Situation::Reconnect);
        assert_eq!(renewal(DesktopMode::Current, 10), Situation::QuietRenewal);
        assert_eq!(renewal(DesktopMode::Current, 11), Situation::LimitedRenewal);

        assert!(connect_fetches(Situation::QuietRenewal, 2, 1).is_empty());
        assert_eq!(
            connect_fetches(Situation::LimitedRenewal, 2, 1),
            [
                Fetch::Unread,
                Fetch::Window(0),
                Fetch::Window(1),
                Fetch::Thread(0)
            ]
        );
        let cold = connect_fetches(Situation::Cold, 2, 1);
        assert_eq!(cold.len(), 10 + 3);
        let reconnect = connect_fetches(Situation::Reconnect, 2, 1);
        // A reconnect rereads everything but names and sidebar preferences.
        let skipped: Vec<_> = cold.iter().filter(|r| !reconnect.contains(r)).collect();
        assert_eq!(skipped, [&Fetch::Metadata, &Fetch::Sidebar]);
    }

    #[test]
    fn resumed_subscriptions_start_before_the_cursor_but_not_before_the_floor() {
        assert_eq!(resume_since(10_000, None, 20_000), 19_700);
        assert_eq!(resume_since(10_000, Some(15_000), 20_000), 14_040);
        assert_eq!(resume_since(10_000, Some(10_100), 20_000), 9_700);
    }

    // The client against loopback fakes: every connect, REQ, CLOSE and HTTP
    // read below goes through the production paths.

    use super::super::net::Net;
    use super::super::run::kind;
    use super::super::scenario::Scenario;
    use super::super::seed::tests::fake_http;
    use super::super::stats::Stats;
    use nostr::EventBuilder;
    use tokio::sync::watch;

    /// `(verb, subscription id)` of every REQ and CLOSE the fake relay got.
    type Frames = Arc<std::sync::Mutex<Vec<(String, String)>>>;

    /// A loopback relay that challenges, accepts AUTH and every EVENT, answers
    /// each REQ with EOSE and records REQs and CLOSEs.
    async fn fake_ws_relay() -> (String, Frames) {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let frames = Frames::default();
        let seen = frames.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seen = seen.clone();
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    let challenge = json!(["AUTH", "challenge"]).to_string();
                    if ws.send(Message::Text(challenge.into())).await.is_err() {
                        return;
                    }
                    while let Some(Ok(Message::Text(text))) = ws.next().await {
                        let frame: Value = serde_json::from_str(&text).unwrap();
                        let verb = frame[0].as_str().unwrap_or_default().to_string();
                        let reply = match verb.as_str() {
                            "AUTH" | "EVENT" => json!(["OK", frame[1]["id"], true, ""]),
                            "REQ" => json!(["EOSE", frame[1]]),
                            _ => Value::Null,
                        };
                        if verb == "REQ" || verb == "CLOSE" {
                            let id = frame[1].as_str().unwrap_or_default().to_string();
                            seen.lock().unwrap().push((verb, id));
                        }
                        if !reply.is_null()
                            && ws
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
        (addr, frames)
    }

    /// A loopback HTTP API: NIP-11 names a relay key, the owner's root search
    /// finds one seeded root, and every other query is empty.
    async fn fake_api(run_id: &str) -> String {
        let owner = identity::keys(run_id, Role::Owner, 0);
        let root = EventBuilder::new(kind(9), "lg-root 0")
            .sign_with_keys(&owner)
            .unwrap();
        let root = serde_json::to_string(&[root]).unwrap();
        let info = json!({"self": Keys::generate().public_key().to_hex()}).to_string();
        let owner = owner.public_key().to_hex();
        fake_http(move |method, target, body| {
            if method == "GET" && target == "/" {
                return (200, info.clone());
            }
            let filters: Value = serde_json::from_slice(body).unwrap_or_default();
            let filter = &filters[0];
            if filter["kinds"] == json!([9]) && filter["authors"] == json!([owner]) {
                (200, root.clone())
            } else {
                (200, "[]".into())
            }
        })
        .await
    }

    /// Two desktop humans in one community of 25 channels, each in `joined`,
    /// with two windows and one thread open and no traffic of their own.
    fn two_desktops(ws: &str, http: &str, joined: usize, desktop: &str) -> Scenario {
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
humans = 2
agents = 0
channels_per_member = {joined}
[human]
presence_interval_secs = 3600.0
typing_per_hour = 0.0
messages_per_hour = 0.0
reactions_per_hour = 0.0
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

    /// Per-op `(count, errors)` by op name, plus the relay's frames.
    struct Outcome {
        ops: HashMap<&'static str, (u64, u64)>,
        frames: Vec<(String, String)>,
    }

    impl Outcome {
        fn count(&self, op: &str) -> u64 {
            self.ops.get(op).map_or(0, |c| c.0)
        }

        fn saw(&self, verb: &str, id: &str) -> bool {
            self.frames.iter().any(|(v, i)| v == verb && i == id)
        }

        /// Counts of the desktop ops, the ones the model owns.
        fn desktop(&self) -> HashMap<&'static str, u64> {
            self.ops
                .iter()
                .filter(|(op, _)| op.starts_with("desktop_"))
                .map(|(op, c)| (*op, c.0))
                .collect()
        }
    }

    fn outcome(stats: &Stats, frames: &Frames) -> Outcome {
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
        }
    }

    /// Runs both clients for at least `min`, until `done` holds, then stops
    /// them and returns what they did. Fails after 15 s without `done`.
    async fn drive(
        joined: usize,
        desktop_toml: &str,
        min: Duration,
        done: impl Fn(&Outcome) -> bool,
    ) -> Outcome {
        let (ws, frames) = fake_ws_relay().await;
        let http = fake_api("d").await;
        let scenario = two_desktops(&ws, &http, joined, desktop_toml);
        let net = Net::new(&scenario).await.unwrap();
        let stats = Arc::new(Stats::default());
        let (stop_tx, stop) = watch::channel(false);
        let shared = Arc::new(Shared::new(scenario, net, stats.clone(), stop));
        let clients: Vec<_> = (0..2)
            .map(|g| tokio::spawn(desktop(shared.clone(), g)))
            .collect();
        let started = Instant::now();
        loop {
            let now = outcome(&stats, &frames);
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
        let out = outcome(&stats, &frames);
        let failed: Vec<_> = out.ops.iter().filter(|(_, c)| c.1 > 0).collect();
        assert!(failed.is_empty(), "errors: {failed:?}");
        out
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
            ("desktop_window", 2),
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

    const QUIET: &str = "tick_secs = 3600.0";

    #[tokio::test]
    async fn a_cold_start_sends_the_modeled_requests_once() {
        // Legacy: per-channel batches of ten for 25 channels; current: one
        // live-only batch and three replays. Both read the same over HTTP.
        let cases = [
            ("legacy", 25, vec![("desktop_live_eose", 2 * 5)]),
            (
                "current",
                25,
                vec![("desktop_live_eose", 2 * 3), ("desktop_replay_eose", 2 * 3)],
            ),
            ("legacy", 5, vec![("desktop_live_eose", 2 * 3)]),
            ("current", 5, vec![("desktop_live_eose", 2 * 3)]),
        ];
        for (mode, joined, subs) in cases {
            let want = with(cold_reads(2), &subs);
            let toml = format!("mode = \"{mode}\"\n{QUIET}");
            let out = drive(joined, &toml, Duration::from_millis(300), |o| {
                o.desktop() == want
            })
            .await;
            assert_eq!(out.desktop(), want, "{mode} J={joined}");
            assert_eq!(out.count("ws_connect_auth"), 2, "{mode} J={joined}");
        }
    }

    #[tokio::test]
    async fn legacy_ticks_reread_and_replace_batches_every_tick() {
        let out = drive(
            25,
            "mode = \"legacy\"\ntick_secs = 1.0",
            Duration::ZERO,
            |o| o.count("desktop_roster") >= 4 && o.saw("CLOSE", "batch-1-2"),
        )
        .await;
        // Each replacement is sent before the batch it replaces is closed.
        assert!(out.saw("REQ", "batch-2-0") && out.saw("CLOSE", "batch-1-0"));
        assert!(out.count("desktop_read_state") >= 4);
        assert!(out.count("desktop_unread") >= 4);
        // Ticks never reread names or open windows.
        assert_eq!(out.count("desktop_metadata"), 2);
        assert_eq!(out.count("desktop_window"), 4);
    }

    #[tokio::test]
    async fn current_ticks_wait_for_the_backstop() {
        let before = drive(
            25,
            "mode = \"current\"\ntick_secs = 1.0",
            Duration::from_millis(2500),
            |o| o.count("desktop_thread_window") == 2,
        )
        .await;
        assert_eq!(
            before.desktop(),
            with(
                cold_reads(2),
                &[("desktop_live_eose", 6), ("desktop_replay_eose", 6)]
            )
        );
        assert!(!before
            .frames
            .iter()
            .any(|(_, id)| id.starts_with("batch-2")));

        let after = drive(
            25,
            "mode = \"current\"\ntick_secs = 1.0\nbackstop_secs = 1.0",
            Duration::ZERO,
            |o| o.count("desktop_roster") >= 4 && o.saw("CLOSE", "batch-1-0"),
        )
        .await;
        assert!(after.saw("REQ", "batch-2-0"));
        assert!(after.count("desktop_unread") >= 4);
        // A replacement is live-only: no replays after the connect's three.
        assert_eq!(after.count("desktop_replay_eose"), 6);
    }

    #[tokio::test]
    async fn legacy_renewals_are_full_reconnects() {
        let out = drive(
            25,
            "mode = \"legacy\"\nrenewal_secs = 1.0\n",
            Duration::ZERO,
            |o| o.count("desktop_nip11") >= 4 && o.count("desktop_roster") >= 4,
        )
        .await;
        assert!(out.count("ws_connect_auth") >= 4);
        // A reconnect keeps channel names and sidebar preferences.
        assert_eq!(out.count("desktop_metadata"), 2);
        assert_eq!(out.count("desktop_sidebar"), 2);
    }

    #[tokio::test]
    async fn current_renewals_carry_small_sessions_and_limit_large_ones() {
        // More than ten channels: replays and channel catch-up again, no globals.
        let large = drive(
            25,
            "mode = \"current\"\nrenewal_secs = 1.0\n",
            Duration::ZERO,
            |o| o.count("desktop_replay_eose") >= 12 && o.count("desktop_unread") >= 4,
        )
        .await;
        assert!(large.count("ws_connect_auth") >= 4);
        for op in [
            "desktop_nip11",
            "desktop_roster",
            "desktop_metadata",
            "desktop_profiles",
        ] {
            assert_eq!(large.count(op), 2, "{op}");
        }
        assert!(large.count("desktop_window") >= 8);

        // Up to ten channels: subscriptions are re-sent and nothing is read.
        let small = drive(
            5,
            "mode = \"current\"\nrenewal_secs = 1.0\n",
            Duration::ZERO,
            |o| o.count("desktop_live_eose") >= 12 && o.count("desktop_thread_window") == 2,
        )
        .await;
        assert!(small.count("ws_connect_auth") >= 4);
        let reads: HashMap<_, _> = small
            .desktop()
            .into_iter()
            .filter(|(op, _)| *op != "desktop_live_eose")
            .collect();
        assert_eq!(reads, cold_reads(2));
    }
}
