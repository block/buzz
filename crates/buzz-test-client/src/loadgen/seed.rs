//! Creates the scenario's fixture: communities, channels, history and deep threads.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use futures_util::stream::{self, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::json;

use super::identity::{self, Role};
use super::net::{truncate, Net};
use super::scenario::{Community, Scenario};
use super::stats::{Op, Snapshot, Stats};

/// Content prefix of seeded thread roots; `run` finds roots by it.
pub const ROOT_PREFIX: &str = "lg-root";

/// Seeds every community. Idempotent for communities and channels; history
/// and threads are appended on every call. Fails unless every planned post
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
    let planned = roots.len() + rest.len();
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

/// Submits `posts` and returns the ids of the accepted ones.
async fn post_all(
    scenario: &Scenario,
    net: &Net,
    stats: &Stats,
    communities: &[Community],
    posts: Vec<Post>,
    roots: &RootIds,
) -> RootIds {
    stream::iter(posts)
        .map(|post| {
            let community = &communities[post.community];
            async move {
                let Some((keys, event)) = post.build(scenario, roots) else {
                    stats.err(Op::Seed, "thread root missing");
                    return None;
                };
                let t0 = Instant::now();
                match net.submit(&keys, community, &event, None).await {
                    Ok(()) => {
                        stats.ok(Op::Seed, t0.elapsed());
                        Some(((post.community, post.channel, post.seq), event.id.to_hex()))
                    }
                    Err(e) => {
                        stats.err(Op::Seed, &e.to_string());
                        None
                    }
                }
            }
        })
        .buffer_unordered(scenario.shape.seed_concurrency)
        .filter_map(std::future::ready)
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
    let body = serde_json::to_vec(&json!({
        "host": host,
        "initial_owner_pubkey": owner.public_key().to_hex(),
    }))?;
    let (status, text) = net
        .post(&operator, base, "/operator/communities", body, None)
        .await?;
    if status.is_success() || text.contains("community already exists") {
        Ok(())
    } else {
        anyhow::bail!("provision {host}: HTTP {status}: {}", truncate(&text))
    }
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
                Err(e) if !e.to_string().contains("exist") => Err(e),
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
/// A root's `seq` is its index within its channel.
fn plan_posts(scenario: &Scenario) -> (Vec<Post>, Vec<Post>) {
    let shape = &scenario.shape;
    let mut roots = Vec::new();
    let mut rest = Vec::new();
    for community in 0..shape.communities {
        for channel in 0..shape.channels_per_community {
            for seq in 0..shape.history_messages_per_channel {
                rest.push(Post {
                    community,
                    channel,
                    kind: PostKind::History,
                    seq,
                });
            }
            for root in 0..shape.thread_roots_per_channel {
                roots.push(Post {
                    community,
                    channel,
                    kind: PostKind::Root,
                    seq: root,
                });
                for seq in 0..shape.thread_depth {
                    rest.push(Post {
                        community,
                        channel,
                        kind: PostKind::Reply { root },
                        seq,
                    });
                }
            }
        }
    }
    (roots, rest)
}

impl Post {
    fn build(&self, scenario: &Scenario, roots: &RootIds) -> Option<(Keys, Event)> {
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
                tags.push(tag(["e", root_id, "", "reply"]));
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
        anyhow::bail!("HTTP {status}: {}", truncate(&body));
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

    /// A minimal `/events` endpoint: rejects posts whose content is `reject`,
    /// and replies whose root it never accepted; accepts everything else.
    async fn fake_relay(reject: Arc<str>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let accepted = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let accepted = accepted.clone();
                let reject = reject.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(socket);
                    loop {
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
                        let event: Event = serde_json::from_slice(&body).unwrap();
                        let root = event
                            .tags
                            .iter()
                            .find(|t| t.kind().as_str() == "e")
                            .and_then(|t| t.content())
                            .map(str::to_string);
                        let ok = event.content != *reject
                            && root.is_none_or(|r| accepted.lock().unwrap().contains(&r));
                        if ok {
                            accepted.lock().unwrap().insert(event.id.to_hex());
                        }
                        let reply =
                            json!({"accepted": ok, "message": if ok { "" } else { "blocked" }})
                                .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{reply}",
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

    async fn seed_against(reject: &str) -> (anyhow::Result<()>, Arc<Stats>) {
        let addr = fake_relay(reject.into()).await;
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
        (seed(&scenario, &net, &stats).await, stats)
    }

    #[tokio::test]
    async fn replies_use_accepted_roots_and_missing_posts_fail_the_seed() {
        let (outcome, stats) = seed_against("none").await;
        outcome.expect("every post accepted");
        assert_eq!(seeded(&stats.snapshot(), &Snapshot::default()), 10);

        let (outcome, stats) = seed_against("lg-history 1").await;
        let err = outcome.expect_err("a rejected post fails the seed");
        assert!(err.to_string().contains("seeded 9 of 10"), "{err}");
        assert_eq!(seeded(&stats.snapshot(), &Snapshot::default()), 9);

        // A rejected root leaves its three replies unpostable.
        let (outcome, _) = seed_against(&format!("{ROOT_PREFIX} 1")).await;
        let err = outcome.expect_err("a rejected root fails the seed");
        assert!(err.to_string().contains("seeded 6 of 10"), "{err}");
    }
}
