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
use super::stats::{Op, Stats};

/// Content prefix of seeded thread roots; `run` finds roots by it.
pub const ROOT_PREFIX: &str = "lg-root";

/// Seeds every community. Idempotent for communities and channels; history
/// and thread replies are appended on every call.
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
    let (roots, rest) = plan_posts(scenario);
    let root_count = roots.len();
    post_all(scenario, net, stats, &communities, roots, &HashMap::new()).await;
    let root_ids = resolve_roots(scenario, net, &communities).await?;
    let rest_count = rest.len();
    post_all(scenario, net, stats, &communities, rest, &root_ids).await;
    eprintln!(
        "seed: {} posts in {:.1}s",
        root_count + rest_count,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Seeded root ids keyed by (community, channel) index.
type RootIds = HashMap<(usize, usize), Vec<String>>;
type RootEntry = ((usize, usize), Vec<String>);

async fn post_all(
    scenario: &Scenario,
    net: &Net,
    stats: &Stats,
    communities: &[Community],
    posts: Vec<Post>,
    roots: &RootIds,
) {
    stream::iter(posts)
        .for_each_concurrent(scenario.shape.seed_concurrency, |post| {
            let community = &communities[post.community];
            async move {
                let Some((keys, event)) = post.build(scenario, roots) else {
                    stats.err(Op::Seed, "thread root missing");
                    return;
                };
                let t0 = Instant::now();
                match net.submit(&keys, community, &event, None).await {
                    Ok(()) => stats.ok(Op::Seed, t0.elapsed()),
                    Err(e) => stats.err(Op::Seed, &e.to_string()),
                }
            }
        })
        .await;
}

/// Looks up every channel's seeded root ids once, reading as rotating humans.
async fn resolve_roots(
    scenario: &Scenario,
    net: &Net,
    communities: &[Community],
) -> anyhow::Result<RootIds> {
    if scenario.shape.thread_roots_per_channel == 0 {
        return Ok(RootIds::new());
    }
    let channels: Vec<(usize, usize)> = communities
        .iter()
        .flat_map(|c| (0..scenario.shape.channels_per_community).map(move |ch| (c.index, ch)))
        .collect();
    let results: Vec<anyhow::Result<RootEntry>> = stream::iter(channels)
        .enumerate()
        .map(|(n, (community, channel))| async move {
            let reader = identity::keys(
                &scenario.run_id,
                Role::Human,
                n % scenario.population.humans.max(1),
            );
            let owner = identity::keys(&scenario.run_id, Role::Owner, community);
            let id = identity::channel_id(&scenario.run_id, community, channel);
            let roots = find_roots(net, &communities[community], &reader, &owner, &id).await?;
            Ok(((community, channel), roots))
        })
        .buffer_unordered(scenario.shape.seed_concurrency)
        .collect()
        .await;
    results.into_iter().collect()
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
                let root_id = roots.get(&(self.community, self.channel))?.get(root)?;
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

/// Seeded thread-root ids in `channel`, oldest first.
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
