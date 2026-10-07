//! Side-by-side benchmark (not for merge). Identical file in both branches.
//! BENCH_THREADS (default 5000) followed threads, 3 replies each.
use super::postgres_tests::{apply, channel_read, mention};
use super::*;
use buzz_core::CommunityId;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    Db,
};
use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::postgres::PgPoolOptions;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn ts() -> u64 {
    nostr::Timestamp::now().as_secs()
}

fn ev(author: &Keys, at: u64, tags: Vec<Tag>) -> nostr::Event {
    EventBuilder::new(Kind::Custom(9), format!("bench {at} {}", Uuid::new_v4()))
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(at))
        .sign_with_keys(author)
        .unwrap()
}

async fn top(db: &Db, c: CommunityId, ch: Uuid, e: &nostr::Event) -> Duration {
    let t = Instant::now();
    db.insert_event(c, e, Some(ch)).await.unwrap();
    t.elapsed()
}

async fn rep(db: &Db, c: CommunityId, ch: Uuid, root: &nostr::Event, e: &nostr::Event) -> Duration {
    let stamp = |e: &nostr::Event| chrono::DateTime::from_timestamp(e.created_at.as_secs() as i64, 0).unwrap();
    let t = Instant::now();
    db.insert_event_with_thread_metadata(
        c,
        e,
        Some(ch),
        Some(crate::event::ThreadMetadataParams {
            event_id: e.id.as_bytes(),
            event_created_at: stamp(e),
            channel_id: ch,
            parent_event_id: Some(root.id.as_bytes()),
            parent_event_created_at: Some(stamp(root)),
            root_event_id: Some(root.id.as_bytes()),
            root_event_created_at: Some(stamp(root)),
            depth: 1,
            broadcast: false,
        }),
    )
    .await
    .unwrap();
    t.elapsed()
}

fn stats(label: &str, mut v: Vec<Duration>) {
    v.sort();
    let p = |q: f64| v[((v.len() as f64 - 1.0) * q) as usize].as_secs_f64() * 1000.0;
    println!(
        "BENCH {label}: n={} p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms",
        v.len(),
        p(0.5),
        p(0.95),
        p(0.99),
        p(1.0)
    );
}

async fn sidebar_bench(db: &Db, c: CommunityId, actor: &Keys, label: &str) {
    let mut v = Vec::new();
    let mut last = None;
    for _ in 0..30 {
        let t = Instant::now();
        let page = db.personal_read_sidebar(c, &actor.public_key(), 20, None).await.unwrap();
        v.push(t.elapsed());
        last = Some(page);
    }
    let page = last.unwrap();
    let busy = &page.channels.iter().max_by_key(|r| r.unread).unwrap();
    println!(
        "BENCH {label}: busiest unread={} attention={} threads={} channels={}",
        busy.unread,
        busy.attention,
        busy.unread_thread_count,
        page.channels.len()
    );
    stats(&format!("sidebar {label}"), v);
}

#[tokio::test]
#[ignore = "benchmark"]
async fn bench_unread() {
    let threads: usize = std::env::var("BENCH_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(5000);
    let pool = PgPoolOptions::new()
        .max_connections(40)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let c = db
        .ensure_configured_community(&format!("bench-{}.local", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let actor = Keys::generate();
    let mut channels = Vec::new();
    for i in 0..20 {
        channels.push(
            db.create_channel(c, &format!("bench {i}"), ChannelType::Stream, ChannelVisibility::Open, None, &actor.public_key().to_bytes(), None)
                .await
                .unwrap()
                .id,
        );
    }
    let busy = channels[0];
    let others: Vec<Keys> = (0..8).map(|_| Keys::generate()).collect();
    let base = ts() - 3600;
    // A first message in every channel, and the first intent starts the actor.
    let mut firsts = Vec::new();
    for ch in &channels {
        let e = ev(&others[0], base, vec![]);
        top(&db, c, *ch, &e).await;
        firsts.push(e);
    }
    assert_eq!(apply(&db, c, &actor, channel_read(busy, &firsts[0])).await, IntentOutcome::Applied);

    // Phase 1: followed threads, serial ingest.
    let (mut roots_t, mut reps_t) = (Vec::new(), Vec::new());
    let mut roots = Vec::new();
    let mut latest = firsts[0].clone();
    for i in 0..threads {
        let at = base + 1 + (i as u64 % 3000);
        let root = ev(&others[i % 8], at, vec![]);
        roots_t.push(top(&db, c, busy, &root).await);
        let mine = ev(&actor, at, vec![]);
        reps_t.push(rep(&db, c, busy, &root, &mine).await);
        for j in 1..3 {
            let r = ev(&others[(i + j) % 8], at, vec![]);
            reps_t.push(rep(&db, c, busy, &root, &r).await);
            latest = r;
        }
        roots.push(root);
    }
    stats("ingest top-level serial", roots_t);
    stats("ingest reply serial", reps_t);
    assert_eq!(apply(&db, c, &actor, channel_read(busy, &latest)).await, IntentOutcome::Applied);
    for (ch, e) in channels.iter().zip(&firsts).skip(1) {
        apply(&db, c, &actor, channel_read(*ch, e)).await;
    }
    sqlx::query("ANALYZE").execute(&pool).await.unwrap();
    sidebar_bench(&db, c, &actor, &format!("caught-up {threads} threads")).await;

    // Phase 2: a 20k timeline backlog with 12 mentions.
    let now = ts();
    let mut t2 = Vec::new();
    for i in 0..20_000u64 {
        let tags = if i % 1667 == 0 { mention(&actor) } else { vec![] };
        let e = ev(&others[(i % 8) as usize], now - 600 + i / 40, tags);
        t2.push(top(&db, c, busy, &e).await);
    }
    stats("ingest top-level serial (busy)", t2);
    sqlx::query("ANALYZE").execute(&pool).await.unwrap();
    sidebar_bench(&db, c, &actor, "20k timeline backlog").await;

    // Phase 3: 500 followed threads get a new reply.
    for root in roots.iter().take(500) {
        let r = ev(&others[3], ts(), vec![]);
        rep(&db, c, busy, root, &r).await;
    }
    sidebar_bench(&db, c, &actor, "20k backlog + 500 unread threads").await;

    // Concurrency: 16 writers into one channel, then into 16 channels.
    for (label, spread) in [("same channel", false), ("16 channels", true)] {
        let started = Instant::now();
        let mut handles = Vec::new();
        for w in 0..16usize {
            let db = db.clone();
            let author = others[w % 8].clone();
            let ch = if spread { channels[w + 1] } else { busy };
            handles.push(tokio::spawn(async move {
                let mut v = Vec::new();
                for i in 0..300u64 {
                    let e = ev(&author, ts() - (i % 30), vec![]);
                    v.push(top(&db, c, ch, &e).await);
                }
                v
            }));
        }
        let mut all = Vec::new();
        for h in handles {
            all.extend(h.await.unwrap());
        }
        let n = all.len();
        let secs = started.elapsed().as_secs_f64();
        println!("BENCH concurrent {label}: {n} inserts in {secs:.2}s = {:.0}/s", n as f64 / secs);
        stats(&format!("ingest concurrent {label}"), all);
    }
}
