//! Creates the scenario's fixture: communities, channels, history and deep threads.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use futures_util::stream::{self, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use reqwest::StatusCode;
use serde_json::json;

use super::identity::{self, Role};
use super::net::{http_reason, Net};
use super::scenario::{Community, Scenario};
use super::stats::{Op, Snapshot, Stats};

/// Content prefix of seeded thread roots; `run` finds roots by it.
pub const ROOT_PREFIX: &str = "lg-root";

/// Seeds every community. Idempotent for communities this run's fixture
/// owner owns, and for channels; history and threads are appended on every
/// call. Fails unless every planned post
/// was accepted.
pub async fn seed(scenario: &Scenario, net: &Net, stats: &Arc<Stats>) -> anyhow::Result<()> {
    for community in scenario.communities() {
        let owner = identity::keys(&scenario.run_id, Role::Owner, community.index);
        if let Some(host) = &community.host {
            provision(scenario, net, host, &owner).await?;
        }
        create_channels(scenario, net, &community, &owner).await?;
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
    let root_ids = post_all(scenario, net, stats, &communities, roots, &RootIds::new()).await;
    post_all(scenario, net, stats, &communities, rest, &root_ids).await;
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
    chains: Vec<Chain>,
    roots: &RootIds,
) -> RootIds {
    stream::iter(chains)
        .map(|chain| async move {
            let mut accepted = Vec::new();
            let mut previous: Option<String> = None;
            for post in chain {
                let community = &communities[post.community];
                let Some((keys, event)) = post.build(scenario, roots, previous.as_deref()) else {
                    stats.err(Op::Seed, "thread parent missing");
                    previous = None;
                    continue;
                };
                let t0 = Instant::now();
                match net.submit(&keys, community, &event, None).await {
                    Ok(()) => {
                        stats.ok(Op::Seed, t0.elapsed());
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
    let operator = Keys::parse(&secret)?;
    provision_host(net, base, &operator, host, owner).await
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

async fn create_channels(
    scenario: &Scenario,
    net: &Net,
    community: &Community,
    owner: &Keys,
) -> anyhow::Result<()> {
    let results: Vec<anyhow::Result<()>> = stream::iter(scenario.channel_ids(community.index))
        .enumerate()
        .map(|(i, channel)| async move {
            let event = EventBuilder::new(Kind::Custom(9007), "")
                .tags([
                    tag(["h", &channel]),
                    tag(["name", &format!("lg-{}-{i}", scenario.run_id)]),
                    tag(["channel_type", "stream"]),
                    tag(["visibility", "open"]),
                ])
                .sign_with_keys(owner)?;
            match net.submit(owner, community, &event, None).await {
                Err(e) if !e.to_string().contains("already exists") => Err(e),
                _ => Ok(()),
            }
        })
        .buffer_unordered(scenario.shape.seed_concurrency)
        .collect()
        .await;
    results.into_iter().collect()
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
}

/// Thread roots, then history and replies (posted once root ids are known).
/// A root's `seq` is its index within its channel. Each thread's replies form
/// one chain, so reply `k` sits at depth `k + 1`; threads and history posts
/// are independent of each other.
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
    (roots, rest)
}

impl Post {
    /// Builds the event. A reply attaches to `previous`, the accepted reply
    /// before it in its thread; the first reply attaches to the thread root.
    fn build(
        &self,
        scenario: &Scenario,
        roots: &RootIds,
        previous: Option<&str>,
    ) -> Option<(Keys, Event)> {
        let channel = identity::channel_id(&scenario.run_id, self.community, self.channel);
        // Rotate authors across humans to stay under per-pubkey rate limits.
        let humans = scenario.population.humans.max(1);
        let author = identity::keys(
            &scenario.run_id,
            Role::Human,
            (self.seq * scenario.shape.communities + self.community) % humans,
        );
        let mut tags = vec![tag(["h", &channel])];
        let (keys, content) = match self.kind {
            PostKind::History => (author, format!("lg-history {}", self.seq)),
            PostKind::Root => (
                identity::keys(&scenario.run_id, Role::Owner, self.community),
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
fn reply_tags(root: &str, parent: &str) -> [Tag; 2] {
    [
        tag(["e", root, "", "root"]),
        tag(["e", parent, "", "reply"]),
    ]
}

/// Up to the 100 newest seeded thread-root ids in `channel`, oldest first.
/// `run` replies into these; a sample of roots is enough load.
pub async fn find_roots(
    net: &Net,
    community: &Community,
    reader: &Keys,
    owner: &Keys,
    channel: &str,
) -> anyhow::Result<Vec<String>> {
    let filter = json!([{
        "kinds": [9],
        "authors": [owner.public_key().to_hex()],
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
            e.content.starts_with(ROOT_PREFIX) && !e.tags.iter().any(|t| t.kind().as_str() == "e")
        })
        .map(|e| (e.created_at.as_secs(), e.id.to_hex()))
        .collect();
    roots.sort();
    Ok(roots.into_iter().map(|(_, id)| id).collect())
}

fn tag<const N: usize>(parts: [&str; N]) -> Tag {
    Tag::parse(parts).expect("static tag shape")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    /// Serves HTTP/1.1 on loopback. `handler(method, target, body)` returns
    /// the status and JSON body of each response.
    async fn fake_http<F>(handler: F) -> String
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
                .build(&scenario, &known, previous.as_deref())
                .expect("reply builds once its parent is known");
            let markers = buzz_core::nip10::parse_thread_markers(&event.tags);
            assert_eq!(markers.root.as_deref(), Some(root_id.as_str()), "reply {k}");
            let parent = previous.as_deref().unwrap_or(&root_id);
            assert_eq!(markers.reply.as_deref(), Some(parent), "reply {k}");
            previous = Some(event.id.to_hex());
        }
        // A reply cannot be placed without its predecessor.
        assert!(rest[0][2].build(&scenario, &known, None).is_none());
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
            let owner = identity::keys("t", Role::Owner, 0);
            create_channels(&scenario, &net, community, &owner).await
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
