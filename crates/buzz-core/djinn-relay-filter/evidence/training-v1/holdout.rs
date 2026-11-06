//! Independent middle-position workload, excluded from optimization objective.
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
fn measure<R: Registry>(case: &FanCase, label: &str) -> Value {
    let registry = fanout::populate::<R>(case);
    let result = registry.fanout(case.query_community, &case.case.event);
    let mut actual: Vec<_> = result.into_iter().map(|x| x.0).collect();
    actual.sort();
    let mut expected: Vec<_> = (0..case.count)
        .map(|i| fixtures::channel(100000 + i))
        .collect();
    expected.sort();
    assert_eq!(actual, expected);
    let eval = || {
        black_box(registry.fanout(black_box(case.query_community), black_box(&case.case.event)))
            .len()
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
        raw.push(elapsed * 1e9 / iterations as f64);
        durations.push(elapsed);
    }
    let mut sorted = raw.clone();
    sorted.sort_by(f64::total_cmp);
    json!({"case":case.case.name,"registry":label,"iterations_per_batch":iterations,"ns_per_call":raw,
        "batch_seconds":durations,"median_ns":sorted[4],"min_ns":sorted[0],"max_ns":sorted[8]})
}
fn main() {
    let mut rows = Vec::new();
    let community = CommunityId::from_uuid(fixtures::channel(90001));
    for count in [1, 10, 100, 1000] {
        for values in [1, 50, 128] {
            let channels: Vec<_> = (0..values).map(fixtures::channel).collect();
            let ch = channels[values / 2];
            let mut tags = fixtures::mixed_tags(16);
            tags[8] = vec!["h".into(), ch.to_string()];
            let mut event = fixtures::event(tags, Some(ch));
            event.event.kind = Kind::from(9);
            let case = FanCase {
                case: fixtures::Case {
                    name: format!("holdout_fanout{count}_h{values}_middle"),
                    filters: vec![fixtures::filter(
                        json!({"kinds":[7,9,11],"#h":channels.iter().map(ToString::to_string).collect::<Vec<_>>(),"since":100}),
                    )],
                    event,
                    expected: true,
                },
                count,
                community,
                query_community: community,
                channels: Some(channels),
                expected_count: count,
            };
            rows.push(measure::<Historical>(&case, "historical"));
            rows.push(measure::<Current>(&case, "final"));
        }
    }
    println!("{}",serde_json::to_string_pretty(&json!({"boundary":"actual fan_out_scoped, middle allowed value and middle tag; not used by optimizer","trials":9,"rows":rows})).unwrap());
}
