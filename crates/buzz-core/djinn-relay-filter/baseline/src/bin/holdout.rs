//! Held out after optimization: h first, varied UUIDs, sorted interior selectors.
//! Never used by the optimizer's frozen first/last objective.
use buzz_core::CommunityId;
use nostr::Kind;
use relay_filter_baseline::{
    current_subscription::SubscriptionRegistry as Current,
    fanout::{self, FanCase, Registry},
    fixtures,
    historical_subscription::SubscriptionRegistry as Historical,
};
use serde_json::{json, Value};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn cases(count: usize, values: usize, sequence: bool) -> Vec<FanCase> {
    let community = CommunityId::from_uuid(fixtures::channel(90001));
    let channels = fanout::live_channels(values);
    // Filter stores a BTreeSet, so choose ranks only after canonical text sorting.
    let selectors: Vec<_> = channels.iter().map(ToString::to_string).collect();
    let parsed = fixtures::filter(json!({"kinds":[1,9,11],"#h":selectors,"since":100}));
    let sorted = parsed
        .generic_tags
        .values()
        .next()
        .unwrap()
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(sorted, selectors);
    let mut state = 0x19b7_73af_6f2c_1015u64;
    let ranks = if sequence {
        (0..16)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                1 + (state as usize % (values - 2))
            })
            .collect::<Vec<_>>()
    } else {
        vec![values / 2]
    };
    ranks
        .into_iter()
        .map(|rank| {
            let channel = channels[rank];
            let mut event = fixtures::event(fanout::live_tags(channel, 0), Some(channel));
            event.event.kind = Kind::from(9);
            assert_eq!(
                event
                    .event
                    .tags
                    .iter()
                    .filter(|t| t.kind().to_string() == "h")
                    .count(),
                1
            );
            assert_eq!(
                event.event.tags.iter().next().unwrap().content(),
                Some(channel.to_string().as_str())
            );
            FanCase {
                case: fixtures::Case {
                    name: format!("heldout_n{count}_h{values}_rank{rank}"),
                    filters: vec![parsed.clone()],
                    event,
                    expected: true,
                },
                count,
                community,
                query_community: community,
                channels: Some(channels.clone()),
                expected_count: count,
            }
        })
        .collect()
}
fn measure<R: Registry>(cases: &[FanCase], label: &str, name: &str) -> Value {
    let registry = fanout::populate::<R>(&cases[0]);
    for case in cases {
        let mut actual = registry.fanout(case.query_community, &case.case.event);
        actual.sort();
        let mut expected: Vec<_> = (0..case.count)
            .map(|i| (fixtures::channel(100000 + i), "live".to_owned()))
            .collect();
        expected.sort();
        assert_eq!(actual, expected);
    }
    let eval = || {
        let mut consumed = 0;
        for case in cases {
            consumed += black_box(
                registry.fanout(black_box(case.query_community), black_box(&case.case.event)),
            )
            .len();
        }
        black_box(consumed)
    };
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
        raw.push(elapsed * 1e9 / iterations as f64 / cases.len() as f64);
        durations.push(elapsed);
    }
    let mut sorted = raw.clone();
    sorted.sort_by(f64::total_cmp);
    json!({"case":name,"registry":label,"sequence_length":cases.len(),"ranks":cases.iter().map(|c| &c.case.name).collect::<Vec<_>>(),"iterations_per_batch":iterations,"ns_per_fanout":raw,"batch_seconds":durations,"median_ns":sorted[4],"min_ns":sorted[0],"max_ns":sorted[8]})
}
fn main() {
    let label = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "historical".into());
    let mut rows = Vec::new();
    for values in [50, 128] {
        for count in [1, 100] {
            for sequence in [false, true] {
                let cases = cases(count, values, sequence);
                let name = format!(
                    "heldout_n{count}_h{values}_{}",
                    if sequence { "seeded16" } else { "middle" }
                );
                rows.push(if label == "historical" {
                    measure::<Historical>(&cases, &label, &name)
                } else {
                    measure::<Current>(&cases, &label, &name)
                });
            }
        }
    }
    println!("{}",serde_json::to_string_pretty(&json!({"boundary":"actual fan_out_scoped, exactly one first h, canonical varied UUIDv4, eight held-out configurations; fixture/registry allocation outside timing; projection/result allocation inside", "trials":9,"rows":rows})).unwrap());
}
