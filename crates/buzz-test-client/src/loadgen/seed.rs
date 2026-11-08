//! Creates the scenario's fixture: communities, channels, history and deep threads.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag};
use reqwest::StatusCode;
use serde_json::json;

use super::identity::{self, Role};
use super::net::{http_reason, truncate, Net};
use super::scenario::{Community, Member, Members, Scenario};
use super::stats::{is_throttle, Op, Snapshot, Stats};

/// Content prefix of seeded thread roots; `run` finds roots by it.
pub const ROOT_PREFIX: &str = "lg-root";
/// Content prefix of the seeded hot-thread root; `run` finds it by this.
pub const HOT_ROOT_PREFIX: &str = "lg-hot";

/// Author of every hot-thread root in run `run_id`.
pub fn hot_root_keys(run_id: &str) -> Keys {
    identity::keys(run_id, Role::HotRoot, 0)
}

/// Most people one kind:9000 may name, the relay's limit.
const MAX_ADDS_PER_EVENT: usize = 100;

/// Channels each creator creates and seeds; see [`channel_creator`].
pub const CHANNELS_PER_CREATOR: usize = 8;

/// Creator of channel `channel` in community `community`: the community
/// owner for its first [`CHANNELS_PER_CREATOR`] channels, then one
/// [`Role::Creator`] identity per further block of that many. The creator
/// signs its channels' kind:9007 and kind:9000 events and their thread roots.
/// The relay limits HTTP calls per pubkey, so one signer for a community's
/// tens of thousands of channels is throttled for hours; blocks of 8 keep
/// those writes spread out, and [`paced`] absorbs what still bunches up.
pub fn channel_creator(scenario: &Scenario, community: usize, channel: usize) -> Keys {
    let run_id = &scenario.run_id;
    let block = channel / CHANNELS_PER_CREATOR;
    if block == 0 {
        return identity::keys(run_id, Role::Owner, community);
    }
    let blocks = scenario
        .shape
        .channels_per_community
        .div_ceil(CHANNELS_PER_CREATOR);
    identity::keys(run_id, Role::Creator, community * blocks + block)
}

/// Most times one seed write is retried after the relay throttles its signer.
const MAX_THROTTLED_RETRIES: usize = 10;

/// Runs `write` until it succeeds or fails other than by the relay's
/// per-pubkey quota. A throttled write waits out the window the relay names
/// (`retry in <n>s`) and tries again, at most [`MAX_THROTTLED_RETRIES`] times,
/// so a signer whose writes bunch up slows down instead of failing the seed.
/// With `stats`, each retried rejection counts as a throttled seed write.
/// Returns how long the accepted attempt took, without the waits before it.
async fn paced<F, Fut>(stats: Option<&Stats>, mut write: F) -> anyhow::Result<Duration>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let mut retries = 0;
    loop {
        let t0 = Instant::now();
        match write().await {
            Ok(()) => return Ok(t0.elapsed()),
            Err(e) if retries < MAX_THROTTLED_RETRIES && is_throttle(&e.to_string()) => {
                let reason = e.to_string();
                if let Some(stats) = stats {
                    stats.err(Op::Seed, &reason);
                }
                retries += 1;
                tokio::time::sleep(retry_after(&reason)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// How long to wait after a quota rejection: the `retry in <n>s` the relay
/// names, capped at a minute, plus a quarter second for clock skew.
fn retry_after(reason: &str) -> Duration {
    let secs = reason
        .split("retry in ")
        .nth(1)
        .and_then(|rest| rest.split('s').next())
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or(1)
        .min(60);
    Duration::from_secs(secs) + Duration::from_millis(250)
}

/// Seeds every community. Idempotent for communities this run's fixture
/// owner owns, for channels and for memberships (each signed by its
/// [`channel_creator`]); history and threads are
/// appended on every call. Fails unless every planned post was accepted and,
/// with private channels, every member was added.
pub async fn seed(scenario: &Scenario, net: &Net, stats: &Arc<Stats>) -> anyhow::Result<()> {
    let members = scenario
        .shape
        .private_channels
        .then(|| scenario.channel_members());
    for community in scenario.communities() {
        let owner = identity::keys(&scenario.run_id, Role::Owner, community.index);
        if let Some(host) = &community.host {
            provision(scenario, net, host, &owner).await?;
        }
        create_channels(scenario, net, &community).await?;
        if let Some(members) = &members {
            let ensured = add_members(scenario, net, &community, members).await?;
            eprintln!(
                "seed: community {} memberships ensured ({ensured} people)",
                community.index
            );
        }
        eprintln!(
            "seed: community {} ready ({} channels)",
            community.index, scenario.shape.channels_per_community
        );
    }

    let communities = scenario.communities();
    let started = Instant::now();
    let before = stats.snapshot();
    let (roots, rest) = plan_posts(scenario);
    let planned: usize = roots.iter().chain(&rest).map(Vec::len).sum();
    let members = members.as_ref();
    let root_ids = post_all(
        scenario,
        net,
        stats,
        &communities,
        members,
        roots,
        &RootIds::new(),
    )
    .await;
    post_all(scenario, net, stats, &communities, members, rest, &root_ids).await;
    let accepted = seeded(&stats.snapshot(), &before);
    eprintln!(
        "seed: {accepted} of {planned} posts accepted in {:.1}s",
        started.elapsed().as_secs_f64()
    );
    if accepted < planned {
        anyhow::bail!(
            "seeded {accepted} of {planned} posts; {} missing",
            planned - accepted
        );
    }
    Ok(())
}

fn seeded(now: &Snapshot, before: &Snapshot) -> usize {
    now.since(before)
        .iter()
        .find(|(op, _)| *op == Op::Seed)
        .map_or(0, |(_, s)| s.count as usize)
}

/// Accepted post ids keyed by (community, channel, seq).
type RootIds = HashMap<(usize, usize, usize), String>;

/// Posts submitted in order, each attaching to the previous accepted one.
type Chain = Vec<Post>;

/// Submits the chains concurrently, each in order, and returns the ids of the
/// accepted posts. A chain stops making progress at its first failed post.
async fn post_all(
    scenario: &Scenario,
    net: &Net,
    stats: &Stats,
    communities: &[Community],
    members: Option<&Members>,
    chains: Vec<Chain>,
    roots: &RootIds,
) -> RootIds {
    stream::iter(chains)
        .map(|chain| async move {
            let mut accepted = Vec::new();
            let mut previous: Option<String> = None;
            for post in chain {
                let community = &communities[post.community];
                let Some((keys, event)) = post.build(scenario, members, roots, previous.as_deref())
                else {
                    stats.err(Op::Seed, "thread parent missing");
                    previous = None;
                    continue;
                };
                match paced(Some(stats), || net.submit(&keys, community, &event, None)).await {
                    Ok(latency) => {
                        stats.ok(Op::Seed, latency);
                        let id = event.id.to_hex();
                        accepted.push(((post.community, post.channel, post.seq), id.clone()));
                        previous = Some(id);
                    }
                    Err(e) => {
                        stats.err(Op::Seed, &e.to_string());
                        previous = None;
                    }
                }
            }
            accepted
        })
        .buffer_unordered(scenario.shape.seed_concurrency)
        .flat_map(stream::iter)
        .collect()
        .await
}

async fn provision(scenario: &Scenario, net: &Net, host: &str, owner: &Keys) -> anyhow::Result<()> {
    let (base, operator) = operator(scenario)?;
    provision_host(net, base, &operator, host, owner).await
}

/// The operator API base URL and the operator's keys, for templated communities.
pub(super) fn operator(scenario: &Scenario) -> anyhow::Result<(&str, Keys)> {
    let base = scenario
        .target
        .operator_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("templated communities need target.operator_url"))?;
    let secret = std::env::var(&scenario.target.operator_key_env).map_err(|_| {
        anyhow::anyhow!(
            "{} must hold the operator secret key",
            scenario.target.operator_key_env
        )
    })?;
    Ok((base, Keys::parse(&secret)?))
}

/// Creates `host` owned by `owner`. The request is `create_only`, so the relay
/// never converges an existing host or rotates its owner. A host that already
/// exists is reused only when `owner` owns it, e.g. when `seed` is re-run;
/// anything else is a collision with a community this run does not own.
async fn provision_host(
    net: &Net,
    base: &str,
    operator: &Keys,
    host: &str,
    owner: &Keys,
) -> anyhow::Result<()> {
    let owner_hex = owner.public_key().to_hex();
    let body = serde_json::to_vec(&json!({
        "host": host,
        "initial_owner_pubkey": owner_hex,
        "create_only": true,
    }))?;
    let (status, text) = net
        .post(operator, base, "/operator/communities", body, None)
        .await?;
    if status.is_success() {
        return Ok(());
    }
    if status != StatusCode::CONFLICT || !text.contains("community already exists") {
        anyhow::bail!("provision {host}: {}", http_reason(status, &text));
    }
    // The relay reports no owner for a host, but lists the hosts an owner owns.
    let url = format!("{base}/operator/communities?owner_pubkey={owner_hex}");
    let (status, text) = net.get(operator, &url).await?;
    if !status.is_success() {
        anyhow::bail!(
            "provision {host}: it already exists and its owner could not be checked: {}",
            http_reason(status, &text)
        );
    }
    let owned: serde_json::Value = serde_json::from_str(&text)?;
    let owns = owned["communities"]
        .as_array()
        .is_some_and(|list| list.iter().any(|c| c["host"] == host));
    if !owns {
        anyhow::bail!(
            "provision {host}: it already exists and is not owned by this run's fixture owner \
             {owner_hex}; use a fresh run_id or a host template that is unique per run"
        );
    }
    Ok(())
}

/// Creates every channel of `community`, each signed by its [`channel_creator`].
async fn create_channels(
    scenario: &Scenario,
    net: &Net,
    community: &Community,
) -> anyhow::Result<()> {
    let visibility = if scenario.shape.private_channels {
        "private"
    } else {
        "open"
    };
    let results: Vec<anyhow::Result<()>> = stream::iter(scenario.channel_ids(community.index))
        .enumerate()
        .map(|(i, channel)| async move {
            let creator = channel_creator(scenario, community.index, i);
            let event = EventBuilder::new(Kind::Custom(9007), "")
                .tags([
                    tag(["h", &channel]),
                    tag(["name", &format!("lg-{}-{i}", scenario.run_id)]),
                    tag(["channel_type", "stream"]),
                    tag(["visibility", visibility]),
                ])
                .sign_with_keys(&creator)?;
            match paced(None, || net.submit(&creator, community, &event, None)).await {
                Err(e) if !e.to_string().contains("already exists") => Err(e),
                _ => Ok(()),
            }
        })
        .buffer_unordered(scenario.shape.seed_concurrency)
        .collect()
        .await;
    results.into_iter().collect()
}

/// Adds every member of `community`'s channels with kind:9000 events from
/// the channel's [`channel_creator`], at most [`MAX_ADDS_PER_EVENT`] people each and
/// `seed_concurrency` at a time. Channel 0 of community 0 also gets the
/// hot-thread root author and authors. Adding a current member at its role
/// changes nothing, so re-running is safe. Returns how many people were
/// named; fails if any event, or any person in one, was refused.
async fn add_members(
    scenario: &Scenario,
    net: &Net,
    community: &Community,
    members: &Members,
) -> anyhow::Result<usize> {
    let run_id = &scenario.run_id;
    let mut adds = Vec::new();
    for (channel, list) in members[community.index].iter().enumerate() {
        let mut people: Vec<Keys> = list
            .iter()
            .map(|m| identity::keys(run_id, m.role, m.index))
            .collect();
        if community.index == 0 && channel == 0 && scenario.hot_thread.enabled() {
            people.push(hot_root_keys(run_id));
            people.extend(
                (0..scenario.peak().hot_authors)
                    .map(|i| identity::keys(run_id, Role::HotAuthor, i)),
            );
        }
        let id = identity::channel_id(run_id, community.index, channel);
        let creator = channel_creator(scenario, community.index, channel);
        for chunk in people.chunks(MAX_ADDS_PER_EVENT) {
            adds.push((
                creator.clone(),
                id.clone(),
                chunk.iter().map(|k| k.public_key().to_hex()).collect(),
            ));
        }
    }
    let named: usize = adds
        .iter()
        .map(|(_, _, p): &(Keys, String, Vec<String>)| p.len())
        .sum();
    let failures: Vec<String> = stream::iter(adds)
        .map(|(creator, channel, people)| async move {
            paced(None, || {
                put_users(net, community, &creator, &channel, &people)
            })
            .await
            .err()
            .map(|e| format!("{channel}: {e}"))
        })
        .buffer_unordered(scenario.shape.seed_concurrency)
        .filter_map(|failure| async move { failure })
        .collect()
        .await;
    if let Some(first) = failures.first() {
        anyhow::bail!(
            "{} membership events failed in community {}; first: {first}",
            failures.len(),
            community.index
        );
    }
    Ok(named)
}

/// Submits one kind:9000 naming `people` in `channel`. The relay accepts an
/// event that adds some people and refuses others, listing the refused ones
/// under `rejected` in its message, so that counts as a failure too.
async fn put_users(
    net: &Net,
    community: &Community,
    owner: &Keys,
    channel: &str,
    people: &[String],
) -> anyhow::Result<()> {
    let tags = std::iter::once(tag(["h", channel])).chain(people.iter().map(|p| tag(["p", p])));
    let event = EventBuilder::new(Kind::Custom(9000), "")
        .tags(tags)
        .sign_with_keys(owner)?;
    let (status, body) = net
        .post(
            owner,
            &community.http_url,
            "/events",
            event.as_json().into_bytes(),
            None,
        )
        .await?;
    if !status.is_success() {
        anyhow::bail!("{}", http_reason(status, &body));
    }
    let reply: serde_json::Value = serde_json::from_str(&body)?;
    let message = reply["message"].as_str().unwrap_or_default();
    if reply["accepted"].as_bool() != Some(true) || message.contains("\"rejected\"") {
        anyhow::bail!("rejected: {}", truncate(message));
    }
    Ok(())
}

/// One seeded post: history message, thread root, or thread reply.
struct Post {
    community: usize,
    channel: usize,
    kind: PostKind,
    seq: usize,
}

enum PostKind {
    History,
    Root,
    Reply { root: usize },
    HotRoot,
    HotReply,
}

/// Key of the hot root in [`RootIds`]: the root after channel 0's seeded ones.
fn hot_key(scenario: &Scenario) -> (usize, usize, usize) {
    (0, 0, scenario.shape.thread_roots_per_channel)
}

/// Thread roots, then history and replies (posted once root ids are known).
/// A root's `seq` is its index within its channel. Each thread's replies form
/// one chain, so reply `k` sits at depth `k + 1`; threads and history posts
/// are independent of each other. The hot thread's replies all answer its
/// root directly, so they post concurrently.
fn plan_posts(scenario: &Scenario) -> (Vec<Chain>, Vec<Chain>) {
    let shape = &scenario.shape;
    let mut roots = Vec::new();
    let mut rest = Vec::new();
    for community in 0..shape.communities {
        for channel in 0..shape.channels_per_community {
            for seq in 0..shape.history_messages_per_channel {
                rest.push(vec![Post {
                    community,
                    channel,
                    kind: PostKind::History,
                    seq,
                }]);
            }
            for root in 0..shape.thread_roots_per_channel {
                roots.push(vec![Post {
                    community,
                    channel,
                    kind: PostKind::Root,
                    seq: root,
                }]);
                let replies: Chain = (0..shape.thread_depth)
                    .map(|seq| Post {
                        community,
                        channel,
                        kind: PostKind::Reply { root },
                        seq,
                    })
                    .collect();
                if !replies.is_empty() {
                    rest.push(replies);
                }
            }
        }
    }
    if scenario.hot_thread.enabled() {
        let (community, channel, seq) = hot_key(scenario);
        roots.push(vec![Post {
            community,
            channel,
            kind: PostKind::HotRoot,
            seq,
        }]);
        rest.extend((0..scenario.hot_thread.seed_replies).map(|seq| {
            vec![Post {
                community,
                channel,
                kind: PostKind::HotReply,
                seq,
            }]
        }));
    }
    (roots, rest)
}

impl Post {
    /// Builds the event. A reply attaches to `previous`, the accepted reply
    /// before it in its thread; the first reply attaches to the thread root.
    /// With `members` (private channels), authors are the channel's humans.
    fn build(
        &self,
        scenario: &Scenario,
        members: Option<&Members>,
        roots: &RootIds,
        previous: Option<&str>,
    ) -> Option<(Keys, Event)> {
        let channel = identity::channel_id(&scenario.run_id, self.community, self.channel);
        let author = self.author(scenario, members);
        let mut tags = vec![tag(["h", &channel])];
        let (keys, content) = match self.kind {
            PostKind::History => (author, format!("lg-history {}", self.seq)),
            PostKind::Root => (
                channel_creator(scenario, self.community, self.channel),
                format!("{ROOT_PREFIX} {}", self.seq),
            ),
            PostKind::Reply { root } => {
                let root_id = roots.get(&(self.community, self.channel, root))?;
                let parent = if self.seq == 0 {
                    root_id.as_str()
                } else {
                    previous?
                };
                tags.extend(reply_tags(root_id, parent));
                (author, format!("lg-reply {}", self.seq))
            }
            PostKind::HotRoot => (hot_root_keys(&scenario.run_id), HOT_ROOT_PREFIX.to_string()),
            PostKind::HotReply => {
                let root_id = roots.get(&hot_key(scenario))?;
                tags.extend(reply_tags(root_id, root_id));
                (author, format!("lg-hot-reply {}", self.seq))
            }
        };
        let event = EventBuilder::new(Kind::Custom(9), content)
            .tags(tags)
            .sign_with_keys(&keys)
            .ok()?;
        Some((keys, event))
    }
}

/// NIP-10 markers for a reply. `root` always names the thread root, since the
/// relay rejects a reply whose root differs from its parent's, and `reply`
/// names the parent, from which the relay derives depth (parent depth + 1).
pub fn reply_tags(root: &str, parent: &str) -> [Tag; 2] {
    [
        tag(["e", root, "", "root"]),
        tag(["e", parent, "", "reply"]),
    ]
}

/// Ids of `author`'s seeded roots in `channel` whose content starts with
/// `prefix`, oldest first, among its 100 newest posts there. `run` replies
/// into these; a sample of roots is enough load.
pub async fn find_roots(
    net: &Net,
    community: &Community,
    reader: &Keys,
    author: &Keys,
    channel: &str,
    prefix: &str,
) -> anyhow::Result<Vec<String>> {
    let filter = json!([{
        "kinds": [9],
        "authors": [author.public_key().to_hex()],
        "#h": [channel],
        "limit": 100,
    }]);
    let (status, body) = net
        .post(
            reader,
            &community.http_url,
            "/query",
            serde_json::to_vec(&filter)?,
            None,
        )
        .await?;
    if !status.is_success() {
        anyhow::bail!("{}", http_reason(status, &body));
    }
    let events: Vec<Event> = serde_json::from_str(&body)?;
    let mut roots: Vec<(u64, String)> = events
        .into_iter()
        .filter(|e| {
            e.content.starts_with(prefix) && !e.tags.iter().any(|t| t.kind().as_str() == "e")
        })
        .map(|e| (e.created_at.as_secs(), e.id.to_hex()))
        .collect();
    roots.sort();
    Ok(roots.into_iter().map(|(_, id)| id).collect())
}

impl Post {
    /// Rotates authors across humans to stay under per-pubkey rate limits:
    /// across all humans for open channels, across the channel's own human
    /// members for private ones, falling back to the channel's
    /// [`channel_creator`] when it has none. Only humans look roots up, so in
    /// their channels the creator signs nothing but roots, and no volume of
    /// history or replies crowds roots out of that lookup's newest 100.
    fn author(&self, scenario: &Scenario, members: Option<&Members>) -> Keys {
        let run_id = &scenario.run_id;
        let Some(members) = members else {
            let humans = scenario.population.humans.max(1);
            let index = (self.seq * scenario.shape.communities + self.community) % humans;
            return identity::keys(run_id, Role::Human, index);
        };
        let humans: Vec<&Member> = members[self.community][self.channel]
            .iter()
            .filter(|m| m.role == Role::Human)
            .collect();
        match humans.get(self.seq % humans.len().max(1)) {
            Some(m) => identity::keys(run_id, Role::Human, m.index),
            None => channel_creator(scenario, self.community, self.channel),
        }
    }
}

fn tag<const N: usize>(parts: [&str; N]) -> Tag {
    Tag::parse(parts).expect("static tag shape")
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::collections::HashSet;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    /// Serves HTTP/1.1 on loopback. `handler(method, target, body)` returns
    /// the status and JSON body of each response.
    pub(in crate::loadgen) async fn fake_http<F>(handler: F) -> String
    where
        F: Fn(&str, &str, &[u8]) -> (u16, String) + Send + Sync + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(socket);
                    loop {
                        let mut request_line = String::new();
                        if reader.read_line(&mut request_line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        let mut length = 0;
                        let mut line = String::new();
                        loop {
                            line.clear();
                            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                                return;
                            }
                            if line == "\r\n" {
                                break;
                            }
                            if let Some(v) =
                                line.to_ascii_lowercase().strip_prefix("content-length:")
                            {
                                length = v.trim().parse().unwrap();
                            }
                        }
                        let mut body = vec![0; length];
                        reader.read_exact(&mut body).await.unwrap();
                        let mut parts = request_line.split_whitespace();
                        let (method, target) = (parts.next().unwrap(), parts.next().unwrap());
                        let (status, reply) = handler(method, target, &body);
                        let response = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{reply}",
                            reply.len()
                        );
                        if reader
                            .get_mut()
                            .write_all(response.as_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        addr
    }

    /// Accepted event id to (thread root id, depth); a root's depth is 0.
    type Accepted = Arc<std::sync::Mutex<HashMap<String, (String, usize)>>>;

    /// A minimal `/events` endpoint that rejects posts whose content is
    /// `reject`. Like the relay it accepts a reply only when its parent was
    /// accepted and its `root` marker names the parent's root, and it derives
    /// depth as the parent's depth + 1.
    async fn fake_relay(reject: Arc<str>) -> (String, Accepted) {
        let accepted: Accepted = Default::default();
        let known = accepted.clone();
        let addr = fake_http(move |_, _, body| {
            let event: Event = serde_json::from_slice(body).unwrap();
            let mut known = known.lock().unwrap();
            let id = event.id.to_hex();
            let placed = match buzz_core::nip10::parse_thread_markers(&event.tags).resolve() {
                None => Some((id.clone(), 0)),
                Some((root, parent)) => known
                    .get(&parent)
                    .filter(|(parent_root, _)| *parent_root == root || parent == root)
                    .map(|(_, depth)| (root.clone(), depth + 1)),
            };
            let placed = placed.filter(|_| event.content != *reject);
            let ok = placed.is_some();
            if let Some(placed) = placed {
                known.insert(id, placed);
            }
            let reply = json!({"accepted": ok, "message": if ok { "" } else { "blocked" }});
            (200, reply.to_string())
        })
        .await;
        (addr, accepted)
    }

    async fn seed_against(reject: &str) -> (anyhow::Result<()>, Arc<Stats>, Accepted) {
        seed_with(reject, "").await
    }

    /// Seeds two history posts and two chains of three replies, plus `extra`
    /// scenario TOML, against a relay that rejects content `reject`.
    async fn seed_with(reject: &str, extra: &str) -> (anyhow::Result<()>, Arc<Stats>, Accepted) {
        let (addr, accepted) = fake_relay(reject.into()).await;
        let scenario = Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 1
history_messages_per_channel = 2
thread_roots_per_channel = 2
thread_depth = 3
[population]
humans = 4
agents = 0
channels_per_member = 1
[[ramp]]
users = 1.0
hold_secs = 1
{extra}
"#
        ))
        .unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let stats = Arc::new(Stats::default());
        (seed(&scenario, &net, &stats).await, stats, accepted)
    }

    #[tokio::test]
    async fn replies_use_accepted_roots_and_missing_posts_fail_the_seed() {
        let (outcome, stats, _) = seed_against("none").await;
        outcome.expect("every post accepted");
        assert_eq!(seeded(&stats.snapshot(), &Snapshot::default()), 10);

        let (outcome, stats, _) = seed_against("lg-history 1").await;
        let err = outcome.expect_err("a rejected post fails the seed");
        assert!(err.to_string().contains("seeded 9 of 10"), "{err}");
        assert_eq!(seeded(&stats.snapshot(), &Snapshot::default()), 9);

        // A rejected root leaves its three replies unpostable.
        let (outcome, _, _) = seed_against(&format!("{ROOT_PREFIX} 1")).await;
        let err = outcome.expect_err("a rejected root fails the seed");
        assert!(err.to_string().contains("seeded 6 of 10"), "{err}");

        // A rejected reply leaves the replies chained to it unpostable.
        let (outcome, _, _) = seed_against("lg-reply 1").await;
        let err = outcome.expect_err("a rejected reply fails the seed");
        assert!(err.to_string().contains("seeded 6 of 10"), "{err}");
    }

    #[tokio::test]
    async fn thread_depth_builds_deep_threads() {
        let (outcome, _, accepted) = seed_against("none").await;
        outcome.expect("every post accepted");
        // Two threads of three chained replies, not six direct replies.
        let mut depths: Vec<usize> = accepted
            .lock()
            .unwrap()
            .values()
            .map(|(_, depth)| *depth)
            .filter(|depth| *depth > 0)
            .collect();
        depths.sort();
        assert_eq!(depths, [1, 1, 2, 2, 3, 3]);
    }

    #[tokio::test]
    async fn the_hot_thread_seeds_direct_replies_under_its_own_root() {
        let hot = "[hot_thread]\nseed_replies = 4";
        let (outcome, stats, accepted) = seed_with("none", hot).await;
        outcome.expect("every post accepted");
        // The base 10 posts plus the hot root and its 4 replies.
        assert_eq!(seeded(&stats.snapshot(), &Snapshot::default()), 15);
        let hot_depths: Vec<usize> = {
            let accepted = accepted.lock().unwrap();
            let roots: Vec<&String> = accepted
                .iter()
                .filter(|(_, (_, depth))| *depth == 0)
                .map(|(id, _)| id)
                .collect();
            // Every reply under the most-replied root sits directly on it.
            let hot_root = roots
                .iter()
                .max_by_key(|root| accepted.values().filter(|(r, _)| r == **root).count())
                .unwrap();
            accepted
                .iter()
                .filter(|(id, (root, _))| root == *hot_root && id != hot_root)
                .map(|(_, (_, depth))| *depth)
                .collect()
        };
        assert_eq!(hot_depths, [1, 1, 1, 1]);

        // A rejected hot root leaves its replies unpostable.
        let (outcome, _, _) = seed_with(HOT_ROOT_PREFIX, hot).await;
        let err = outcome.expect_err("a rejected hot root fails the seed");
        assert!(err.to_string().contains("seeded 10 of 15"), "{err}");
    }

    /// Seeds `scenario` against a relay that accepts everything, answering
    /// kind:9000 with `put_user_message`. Returns the outcome and every
    /// event submitted, in order.
    async fn seed_logged(
        scenario: &str,
        put_user_message: &'static str,
    ) -> (anyhow::Result<()>, Scenario, Vec<Event>) {
        let log: Arc<std::sync::Mutex<Vec<Event>>> = Default::default();
        let logged = log.clone();
        let addr = fake_http(move |_, _, body| {
            let event: Event = serde_json::from_slice(body).unwrap();
            let message = if event.kind == Kind::Custom(9000) {
                put_user_message
            } else {
                ""
            };
            logged.lock().unwrap().push(event);
            (
                200,
                json!({"accepted": true, "message": message}).to_string(),
            )
        })
        .await;
        let scenario = Scenario::parse(&scenario.replace("{addr}", &addr)).unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let stats = Arc::new(Stats::default());
        let outcome = seed(&scenario, &net, &stats).await;
        let events = log.lock().unwrap().clone();
        (outcome, scenario, events)
    }

    fn values(event: &Event, name: &str) -> Vec<String> {
        event
            .tags
            .iter()
            .filter(|t| t.kind().as_str() == name)
            .filter_map(|t| t.content().map(str::to_string))
            .collect()
    }

    const PRIVATE: &str = r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 20
history_messages_per_channel = 3
thread_roots_per_channel = 1
thread_depth = 3
private_channels = true
wide_channels = 1
[population]
humans = 150
agents = 2
channels_per_member = 1
wide_member_fraction = 1.0
[hot_thread]
seed_replies = 2
authors = 2
[[ramp]]
users = 1.0
hold_secs = 1
[[ramp]]
users = 1.5
hold_secs = 1
"#;

    #[tokio::test]
    async fn private_channels_add_every_member_before_anything_is_posted() {
        let (outcome, scenario, events) = seed_logged(PRIVATE, "").await;
        outcome.expect("seed passes");
        let kind = |e: &Event| e.kind.as_u16();
        let creates: Vec<&Event> = events.iter().filter(|e| kind(e) == 9007).collect();
        assert_eq!(creates.len(), 20);
        assert!(creates
            .iter()
            .all(|e| values(e, "visibility") == ["private"]));
        let index = |e: &Event| {
            let [channel] = values(e, "h").try_into().unwrap();
            (0..20)
                .find(|c| identity::channel_id("t", 0, *c) == channel)
                .unwrap()
        };
        let creator = |c| channel_creator(&scenario, 0, c).public_key();
        // The owner creates channels 0-7; each further block of 8 has its own creator.
        for create in &creates {
            assert_eq!(create.pubkey, creator(index(create)));
        }
        let owner = identity::keys("t", Role::Owner, 0).public_key();
        assert_eq!(creator(7), owner);
        assert_ne!(creator(8), owner);
        assert_eq!(creator(8), creator(15));
        assert_ne!(creator(16), creator(8));
        let first_put = events.iter().position(|e| kind(e) == 9000).unwrap();
        let last_put = events.iter().rposition(|e| kind(e) == 9000).unwrap();
        let first_post = events.iter().position(|e| kind(e) == 9).unwrap();
        let last_create = events.iter().rposition(|e| kind(e) == 9007).unwrap();
        assert!(last_create < first_put && last_put < first_post);

        let members = scenario.channel_members();
        let mut added: HashMap<String, Vec<String>> = HashMap::new();
        for put in events.iter().filter(|e| kind(e) == 9000) {
            assert_eq!(put.pubkey, creator(index(put)));
            let people = values(put, "p");
            assert!(!people.is_empty() && people.len() <= MAX_ADDS_PER_EVENT);
            let [channel] = values(put, "h").try_into().unwrap();
            added.entry(channel).or_default().extend(people);
        }
        // Every human up to the peak (225) joins the wide channel 0, and so do
        // the hot thread's root author and its 3 peak authors.
        for (c, list) in members[0].iter().enumerate() {
            let mut want: Vec<String> = list
                .iter()
                .map(|m| identity::keys("t", m.role, m.index).public_key().to_hex())
                .collect();
            if c == 0 {
                want.push(hot_root_keys("t").public_key().to_hex());
                want.extend((0..3).map(|i| {
                    identity::keys("t", Role::HotAuthor, i)
                        .public_key()
                        .to_hex()
                }));
                assert_eq!(want.len(), 225 + 4);
            }
            let mut got = added.remove(&identity::channel_id("t", 0, c)).unwrap();
            got.sort();
            want.sort();
            assert_eq!(got, want, "channel {c}");
        }
        assert!(added.is_empty());

        // The creator signs each channel's roots; every other post is by a
        // human member of its channel, so none crowds a root out of `run`'s
        // newest-100 lookup.
        for post in events.iter().filter(|e| kind(e) == 9) {
            let c = index(post);
            let author = post.pubkey.to_hex();
            if post.content.starts_with(ROOT_PREFIX) {
                assert_eq!(post.pubkey, creator(c));
                continue;
            }
            let member = members[0][c].iter().any(|m| {
                m.role == Role::Human
                    && identity::keys("t", Role::Human, m.index)
                        .public_key()
                        .to_hex()
                        == author
            });
            let hot_root = author == hot_root_keys("t").public_key().to_hex();
            let hot_author = (0..3).any(|i| {
                identity::keys("t", Role::HotAuthor, i)
                    .public_key()
                    .to_hex()
                    == author
            });
            assert!(
                member || hot_root || hot_author,
                "{author} posted in channel {c}"
            );
        }
    }

    /// One community of many private channels, wide channels and a tier of
    /// humans in most channels, at 2x.
    const MANY_CHANNELS: &str = r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 400
history_messages_per_channel = 10
thread_roots_per_channel = 1
thread_depth = 5
seed_concurrency = 64
private_channels = true
wide_channels = 3
[population]
humans = 300
agents = 30
channels_per_member = 3
wide_member_fraction = 0.75
tiers = [{ fraction = 0.1, channels = 280 }]
[[ramp]]
users = 2.0
hold_secs = 1
"#;

    /// The relay's default HTTP calls a minute for one pubkey.
    const HTTP_CALLS_PER_MIN: usize = 300;

    #[tokio::test]
    async fn no_channel_creator_signs_more_seed_writes_than_one_minute_of_its_quota() {
        let (outcome, scenario, events) = seed_logged(MANY_CHANNELS, "").await;
        outcome.expect("seed passes");
        let creators: HashSet<String> = (0..400)
            .map(|c| channel_creator(&scenario, 0, c).public_key().to_hex())
            .collect();
        assert_eq!(creators.len(), 400 / CHANNELS_PER_CREATOR);
        let mut writes: HashMap<String, usize> = HashMap::new();
        for event in events
            .iter()
            .filter(|e| creators.contains(&e.pubkey.to_hex()))
        {
            *writes.entry(event.pubkey.to_hex()).or_default() += 1;
        }
        // Creators sign every create, member add and root. Under one minute's
        // quota in total, a creator is never throttled, however its writes
        // bunch up; humans' posts are paced instead.
        let (busiest, most) = writes.iter().max_by_key(|(_, n)| **n).unwrap();
        assert!(
            *most < HTTP_CALLS_PER_MIN,
            "{busiest} signed {most} of {} seed writes",
            events.len()
        );
    }

    /// When each write was submitted, keyed by what it says: a kind:9000 is
    /// re-signed on each try, so its id changes.
    type Tries = HashMap<String, Vec<Instant>>;

    /// Seeds `scenario` against a relay that answers each write's first
    /// `throttles` submissions with the relay's HTTP 429, naming a
    /// `retry_secs` window. Returns the outcome and every write's submissions.
    async fn seed_throttled(
        scenario: &str,
        throttles: usize,
        retry_secs: u64,
    ) -> (anyhow::Result<()>, Tries, Arc<Stats>) {
        let tries: Arc<std::sync::Mutex<Tries>> = Default::default();
        let seen = tries.clone();
        let addr = fake_http(move |_, _, body| {
            let event: Event = serde_json::from_slice(body).unwrap();
            let write = format!("{} {} {:?}", event.pubkey, event.content, event.tags);
            let mut seen = seen.lock().unwrap();
            let at = seen.entry(write).or_default();
            at.push(Instant::now());
            if at.len() <= throttles {
                let error = format!("rate-limited: quota exceeded; retry in {retry_secs}s");
                return (429, json!({ "error": error }).to_string());
            }
            (200, json!({"accepted": true, "message": ""}).to_string())
        })
        .await;
        let scenario = Scenario::parse(&scenario.replace("{addr}", &addr)).unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let stats = Arc::new(Stats::default());
        let outcome = seed(&scenario, &net, &stats).await;
        let tries = tries.lock().unwrap().clone();
        (outcome, tries, stats)
    }

    #[tokio::test]
    async fn throttled_writes_wait_out_the_quota_and_a_lasting_quota_fails_the_seed() {
        let (outcome, tries, _) = seed_throttled(PRIVATE, 2, 0).await;
        outcome.expect("throttled writes are retried");
        assert!(tries.values().all(|at| at.len() == 3), "{tries:?}");

        let (outcome, tries, _) = seed_throttled(PRIVATE, usize::MAX, 0).await;
        let err = outcome.expect_err("a quota that never clears fails the seed");
        assert!(err.to_string().contains("quota exceeded"), "{err}");
        let most = MAX_THROTTLED_RETRIES + 1;
        assert!(tries.values().all(|at| at.len() == most), "{tries:?}");

        // A retry waits out the window the relay names, and the post's
        // rejection counts as throttled without its wait counting as latency;
        // one channel and one post keep the wait short.
        let one = r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
[shape]
communities = 1
channels_per_community = 1
history_messages_per_channel = 1
[population]
humans = 1
agents = 0
channels_per_member = 1
[[ramp]]
users = 1.0
hold_secs = 1
"#;
        let (outcome, tries, stats) = seed_throttled(one, 1, 1).await;
        outcome.expect("throttled writes are retried");
        assert_eq!(tries.len(), 2, "{tries:?}");
        for at in tries.values() {
            let [first, retry] = at[..] else {
                panic!("{at:?}");
            };
            assert!(retry - first >= Duration::from_secs(1), "{at:?}");
        }
        let posts = stats.snapshot().since(&Snapshot::default());
        let (_, posts) = posts.iter().find(|(op, _)| *op == Op::Seed).unwrap();
        assert_eq!((posts.count, posts.throttled, posts.errors), (1, 1, 0));
        assert!(posts.max_ms < 1000.0, "{posts:?}");
    }

    #[test]
    fn retry_after_waits_the_named_window_capped_at_a_minute() {
        let quarter = Duration::from_millis(250);
        let wait = |reason: &str| retry_after(reason) - quarter;
        let body = |n: &str| {
            format!("HTTP 429 Too Many Requests: {{\"error\":\"rate-limited: quota exceeded; retry in {n}s\"}}")
        };
        assert_eq!(wait(&body("7")), Duration::from_secs(7));
        assert_eq!(wait(&body("0")), Duration::ZERO);
        assert_eq!(wait(&body("3600")), Duration::from_secs(60));
        assert_eq!(wait("HTTP 429 Too Many Requests: "), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn refused_members_fail_the_seed_and_open_channels_add_nobody() {
        // The relay accepts an event that refuses some of the people it names.
        let refused =
            r#"members: {"added":["aa"],"rejected":[{"reason":"blocked","pubkeys":["bb"]}]}"#;
        let (outcome, _, _) = seed_logged(PRIVATE, refused).await;
        let err = outcome.expect_err("a refused member fails the seed");
        assert!(err.to_string().contains("membership"), "{err}");
        // Re-adding current members changes nothing and passes.
        let (outcome, _, _) = seed_logged(PRIVATE, r#"members: {"already_member":["aa"]}"#).await;
        outcome.expect("re-running the seed passes");

        let open = PRIVATE.replace("private_channels = true", "");
        let (outcome, _, events) = seed_logged(&open, "").await;
        outcome.expect("seed passes");
        assert!(!events.iter().any(|e| e.kind.as_u16() == 9000));
        let creates: Vec<&Event> = events.iter().filter(|e| e.kind.as_u16() == 9007).collect();
        assert!(creates.iter().all(|e| values(e, "visibility") == ["open"]));
    }

    #[test]
    fn reply_k_points_reply_at_reply_k_minus_one_and_root_at_the_root() {
        let scenario = Scenario::parse(
            r#"
run_id = "t"
[target]
ws_url = "ws://localhost:3000"
http_url = "http://localhost:3000"
[shape]
communities = 1
channels_per_community = 1
thread_roots_per_channel = 1
thread_depth = 4
[population]
humans = 2
agents = 0
channels_per_member = 1
[[ramp]]
users = 1.0
hold_secs = 1
"#,
        )
        .unwrap();
        let (roots, rest) = plan_posts(&scenario);
        assert_eq!(roots.len(), 1);
        assert_eq!(rest.len(), 1, "one chain per thread");
        let root_id = "a".repeat(64);
        let known: RootIds = [((0, 0, 0), root_id.clone())].into();

        let mut previous: Option<String> = None;
        for (k, post) in rest[0].iter().enumerate() {
            let (_, event) = post
                .build(&scenario, None, &known, previous.as_deref())
                .expect("reply builds once its parent is known");
            let markers = buzz_core::nip10::parse_thread_markers(&event.tags);
            assert_eq!(markers.root.as_deref(), Some(root_id.as_str()), "reply {k}");
            let parent = previous.as_deref().unwrap_or(&root_id);
            assert_eq!(markers.reply.as_deref(), Some(parent), "reply {k}");
            previous = Some(event.id.to_hex());
        }
        // A reply cannot be placed without its predecessor.
        assert!(rest[0][2].build(&scenario, None, &known, None).is_none());
    }

    /// An operator endpoint that lists hosts by owner, as `GET
    /// /operator/communities` does, and refuses a create that is not
    /// `create_only` (which would converge ownership on an existing host).
    async fn fake_operator(hosts: Vec<(String, String)>) -> String {
        let hosts = std::sync::Mutex::new(hosts);
        fake_http(move |method, target, body| {
            let mut hosts = hosts.lock().unwrap();
            if method == "GET" {
                let owner = target.rsplit('=').next().unwrap();
                let owned: Vec<_> = hosts
                    .iter()
                    .filter(|(_, o)| o == owner)
                    .map(|(host, _)| json!({ "host": host }))
                    .collect();
                return (200, json!({ "communities": owned }).to_string());
            }
            let request: serde_json::Value = serde_json::from_slice(body).unwrap();
            if request["create_only"] != true {
                return (400, json!({"error": "create_only required"}).to_string());
            }
            let host = request["host"].as_str().unwrap().to_string();
            if hosts.iter().any(|(h, _)| *h == host) {
                return (
                    409,
                    json!({"error": "community already exists"}).to_string(),
                );
            }
            let owner = request["initial_owner_pubkey"].as_str().unwrap();
            hosts.push((host, owner.to_string()));
            (200, json!({"status": "created"}).to_string())
        })
        .await
    }

    #[tokio::test]
    async fn provisioning_never_takes_over_a_host_the_fixture_owner_does_not_own() {
        let owner = identity::keys("t", Role::Owner, 0);
        let someone_else = Keys::generate().public_key().to_hex();
        let operator = Keys::generate();
        let addr = fake_operator(vec![
            ("mine.example".to_string(), owner.public_key().to_hex()),
            ("theirs.example".to_string(), someone_else),
        ])
        .await;
        let base = format!("http://{addr}");
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
[[ramp]]
users = 1.0
hold_secs = 1
"#
        ))
        .unwrap();
        let net = Net::new(&scenario).await.unwrap();
        let provision = |host: &'static str| provision_host(&net, &base, &operator, host, &owner);

        provision("fresh.example")
            .await
            .expect("a new host is created");
        provision("fresh.example")
            .await
            .expect("re-running reuses a host the fixture owner owns");
        provision("mine.example")
            .await
            .expect("an existing host owned by the fixture owner is reused");
        let err = provision("theirs.example")
            .await
            .expect_err("another owner's host is not taken over");
        assert!(err.to_string().contains("not owned by"), "{err}");
    }

    #[tokio::test]
    async fn only_already_exists_channel_errors_are_ignored() {
        let outcome = |message: &'static str| async move {
            let addr = fake_http(move |_, _, _| {
                (
                    200,
                    json!({"accepted": false, "message": message}).to_string(),
                )
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
channels_per_community = 2
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
            let community = &scenario.communities()[0];
            create_channels(&scenario, &net, community).await
        };
        outcome("duplicate: channel already exists")
            .await
            .expect("an existing channel is fine");
        let err = outcome("invalid: community does not exist")
            .await
            .expect_err("a missing community is not an existing channel");
        assert!(err.to_string().contains("does not exist"), "{err}");
    }
}
