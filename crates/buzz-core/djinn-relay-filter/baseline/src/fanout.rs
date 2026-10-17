//! Full real registry driver. Each row is synthetic, not a traffic-weighted claim.
use crate::fixtures::{self, Case};
use buzz_core::{CommunityId, StoredEvent};
use nostr::{Filter, Kind};
use serde_json::{json, Value};
use std::{
    hint::black_box,
    sync::OnceLock,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Varied canonical UUIDv4 bytes; sorting is explicit before ranks are selected.
pub fn live_channels(count: usize) -> Vec<Uuid> {
    let mut state = 0x5eed_2026_1015_a77bu64;
    let mut next = || {
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    };
    let mut channels: Vec<_> = (0..count)
        .map(|_| {
            let mut bytes = [0; 16];
            bytes[..8].copy_from_slice(&next().to_be_bytes());
            bytes[8..].copy_from_slice(&next().to_be_bytes());
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            Uuid::from_bytes(bytes)
        })
        .collect();
    channels.sort_by_key(Uuid::to_string);
    channels.dedup();
    assert_eq!(channels.len(), count);
    channels
}
/// Exactly one h tag: production's first parseable h equals stored channel.
pub fn live_tags(channel: Uuid, h_position: usize) -> Vec<Vec<String>> {
    let mut tags: Vec<_> = (0..16)
        .map(|i| match i % 3 {
            0 => vec!["e".into(), fixtures::event_id(10000 + i)],
            1 => vec!["p".into(), fixtures::AUTHOR.into()],
            _ => vec!["t".into(), format!("topic-{i}")],
        })
        .collect();
    tags[h_position] = vec!["h".into(), channel.to_string()];
    tags
}

pub trait Registry: Sized {
    fn new() -> Self;
    fn register(
        &self,
        community: CommunityId,
        conn: Uuid,
        filters: Vec<Filter>,
        channels: Option<Vec<Uuid>>,
    );
    fn fanout(&self, community: CommunityId, event: &StoredEvent) -> Vec<(Uuid, String)>;
}
macro_rules! registry_impl {
    ($ty:path) => {
        impl Registry for $ty {
            fn new() -> Self {
                Self::new()
            }
            fn register(
                &self,
                c: CommunityId,
                id: Uuid,
                f: Vec<Filter>,
                channels: Option<Vec<Uuid>>,
            ) {
                if let Some(channels) = channels {
                    self.register_channels_scoped(c, id, "live".into(), f, channels);
                } else {
                    self.register_scoped(c, id, "live".into(), f, None);
                }
            }
            fn fanout(&self, c: CommunityId, e: &StoredEvent) -> Vec<(Uuid, String)> {
                self.fan_out_scoped(c, e)
            }
        }
    };
}
registry_impl!(crate::historical_subscription::SubscriptionRegistry);
registry_impl!(crate::current_subscription::SubscriptionRegistry);

pub struct FanCase {
    pub case: Case,
    pub count: usize,
    pub community: CommunityId,
    pub query_community: CommunityId,
    pub channels: Option<Vec<Uuid>>,
    pub expected_count: usize,
}
pub fn cases() -> &'static [FanCase] {
    static CASES: OnceLock<Vec<FanCase>> = OnceLock::new();
    CASES.get_or_init(|| {
        let c = CommunityId::from_uuid(fixtures::channel(90001));
        let other = CommunityId::from_uuid(fixtures::channel(90002));
        let mut rows = Vec::new();
        for n in [1, 10, 100, 1000] {
            for values in [1, 50, 128] {
                let channels: Vec<_> = live_channels(values);
                let selectors: Vec<_> = channels.iter().map(Uuid::to_string).collect();
                for (position, value_position) in [
                    ("early", "first"),
                    ("late", "first"),
                    ("early", "last"),
                    ("late", "last"),
                    ("miss", "last"),
                ] {
                    let mut tags = live_tags(fixtures::channel(40000), 0);
                    let ch = channels[if value_position == "first" {
                        0
                    } else {
                        values - 1
                    }];
                    if position == "early" {
                        tags[0] = vec!["h".into(), ch.to_string()];
                    }
                    if position == "late" {
                        tags[0] = vec!["e".into(), fixtures::event_id(10000)];
                        tags[15] = vec!["h".into(), ch.to_string()];
                    }
                    let stored = if position == "miss" {
                        fixtures::channel(40000)
                    } else {
                        ch
                    };
                    let mut event = fixtures::event(tags, Some(stored));
                    event.event.kind = Kind::from(9);
                    rows.push(FanCase {
                        case: Case {
                            name: format!("live_fanout{n}_h{values}_{value_position}_{position}"),
                            filters: vec![fixtures::filter(
                                json!({"kinds":[1,9,11],"#h":selectors,"since":100}),
                            )],
                            event,
                            expected: position != "miss",
                        },
                        count: n,
                        community: c,
                        query_community: c,
                        channels: Some(channels.clone()),
                        expected_count: if position == "miss" { 0 } else { n },
                    });
                }
            }
            for scenario in [
                "zero_candidates",
                "other_community",
                "wrong_kind",
                "global_p_index",
            ] {
                let mut filters = json!({"kinds":[1],"#h":[fixtures::channel(0).to_string()]});
                let mut query = c;
                let mut scope = Some(vec![fixtures::channel(0)]);
                let mut event = fixtures::event(
                    live_tags(fixtures::channel(0), 0),
                    Some(fixtures::channel(0)),
                );
                let mut expected = 0;
                match scenario {
                    "zero_candidates" => {
                        event = fixtures::event(
                            live_tags(fixtures::channel(40000), 0),
                            Some(fixtures::channel(40000)),
                        );
                    }
                    "other_community" => {
                        query = other;
                    }
                    "wrong_kind" => {
                        filters["kinds"] = json!([9]);
                    }
                    _ => {
                        filters = json!({"kinds":[1],"#p":[fixtures::AUTHOR]});
                        scope = None;
                        event =
                            fixtures::event(vec![vec!["p".into(), fixtures::AUTHOR.into()]], None);
                        expected = n;
                    }
                }
                rows.push(FanCase {
                    case: Case {
                        name: format!("fanout{n}_{scenario}"),
                        filters: vec![fixtures::filter(filters)],
                        event,
                        expected: expected != 0,
                    },
                    count: n,
                    community: c,
                    query_community: query,
                    channels: scope,
                    expected_count: expected,
                });
            }
        }
        rows
    })
}
/// Check routing fidelity and complete recipients outside measured regions.
pub fn assert_result<R: Registry>(registry: &R, case: &FanCase) {
    let hs: Vec<_> = case
        .case
        .event
        .event
        .tags
        .iter()
        .filter(|t| t.kind().to_string() == "h")
        .collect();
    if let Some(channel) = case.case.event.channel_id {
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].content(), Some(channel.to_string().as_str()));
    } else {
        assert!(hs.is_empty());
    }
    let mut actual = registry.fanout(case.query_community, &case.case.event);
    actual.sort();
    let mut expected: Vec<_> = (0..case.expected_count)
        .map(|i| (fixtures::channel(100000 + i), "live".to_owned()))
        .collect();
    expected.sort();
    assert_eq!(actual, expected, "{}", case.case.name);
}
pub fn populate<R: Registry>(case: &FanCase) -> R {
    let registry = R::new();
    for i in 0..case.count {
        registry.register(
            case.community,
            fixtures::channel(100000 + i),
            case.case.filters.clone(),
            case.channels.clone(),
        );
    }
    registry
}
fn samples(mut eval: impl FnMut() -> usize) -> Value {
    for _ in 0..100 {
        black_box(eval());
    }
    let mut iterations = 1u64;
    loop {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(eval());
        }
        if start.elapsed() >= Duration::from_millis(40) || iterations >= (1 << 30) {
            break;
        }
        iterations *= 2;
    }
    let mut raw = Vec::new();
    let mut durations = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(eval());
        }
        let elapsed = start.elapsed().as_secs_f64();
        raw.push(elapsed * 1e9 / iterations as f64);
        durations.push(elapsed);
    }
    let mut sorted = raw.clone();
    sorted.sort_by(f64::total_cmp);
    json!({"iterations_per_batch":iterations,"ns_per_call":raw,"batch_seconds":durations,
        "median_ns":sorted[4],"min_ns":sorted[0],"max_ns":sorted[8]})
}
pub fn measure<R: Registry>(label: &str) -> Vec<Value> {
    let mut rows = Vec::new();
    for case in cases() {
        let registry = populate::<R>(case);
        assert_result(&registry, case);
        let result = registry.fanout(case.query_community, &case.case.event);
        assert_eq!(result.len(), case.expected_count, "{}", case.case.name);
        let mut actual: Vec<_> = result.into_iter().map(|x| x.0).collect();
        actual.sort();
        let mut expected: Vec<_> = (0..case.expected_count)
            .map(|i| fixtures::channel(100000 + i))
            .collect();
        expected.sort();
        assert_eq!(actual, expected);
        let mut row = samples(|| {
            black_box(registry.fanout(black_box(case.query_community), black_box(&case.case.event)))
                .len()
        });
        row["case"] = json!(case.case.name);
        row["registry"] = json!(label);
        row["candidates_registered"] = json!(case.count);
        row["h_values"] = json!(case.channels.as_ref().map(Vec::len));
        rows.push(row);
    }
    rows
}
/// Includes caller-side cloned input allocation, removal and authoritative replacement.
pub fn registration<R: Registry>(label: &str) -> Vec<Value> {
    let mut rows = Vec::new();
    for case in cases()
        .iter()
        .filter(|c| c.count == 1 && c.case.name.ends_with("late"))
    {
        let registry = R::new();
        let id = fixtures::channel(100000);
        let mut row = samples(|| {
            registry.register(
                case.community,
                id,
                black_box(case.case.filters.clone()),
                case.channels.clone(),
            );
            0
        });
        assert_eq!(
            registry
                .fanout(case.query_community, &case.case.event)
                .len(),
            1
        );
        row["case"] = json!(case.case.name);
        row["registry"] = json!(label);
        row["boundary"] = json!("same-ID registration/replacement, including input clone");
        rows.push(row);
    }
    rows
}
