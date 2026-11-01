//! Checked, warmed release samples; allocations belong to the named boundary.
use crate::fixtures;
use buzz_core::StoredEvent;
use nostr::Filter;
use serde_json::{json, Value};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};
pub type Matcher = fn(&[Filter], &StoredEvent) -> bool;
fn measure(eval: impl Fn() -> bool) -> Value {
    for _ in 0..1000 {
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
    let mut samples = Vec::new();
    let mut durations = Vec::new();
    let mut hits = 0u64;
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..iterations {
            hits += u64::from(eval());
        }
        let elapsed = start.elapsed().as_secs_f64();
        samples.push(elapsed * 1e9 / iterations as f64);
        durations.push(elapsed);
    }
    black_box(hits);
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    json!({"iterations_per_batch":iterations,"ns_per_call":samples,"batch_seconds":durations,"median_ns":sorted[4],"min_ns":sorted[0],"max_ns":sorted[8]})
}
pub fn run(matchers: &[(&str, Matcher)]) {
    let mut rows = Vec::new();
    for case in fixtures::cases() {
        for &(name, matcher) in matchers {
            assert_eq!(
                matcher(&case.filters, &case.event),
                case.expected,
                "{} / {}",
                case.name,
                name
            );
            let matcher = black_box(matcher);
            let mut row =
                measure(|| black_box(matcher(black_box(&case.filters), black_box(&case.event))));
            row["case"] = json!(case.name);
            row["matcher"] = json!(name);
            rows.push(row);
        }
    }
    println!("{}",serde_json::to_string_pretty(&json!({"boundary":"public typed Filter/StoredEvent through complete operation; rejected prototype includes fresh filter and event projection","trials":9,"warmup_calls":1000,"rows":rows})).unwrap());
}
pub fn run_prepared() {
    use crate::prepared_filter::{filters_match, prepare_event, prepare_filters};
    let mut rows = Vec::new();
    for case in fixtures::cases() {
        let filters = prepare_filters(&case.filters);
        let event = prepare_event(&case.event);
        assert_eq!(
            filters_match(&filters, &event),
            case.expected,
            "{}",
            case.name
        );
        for mode in [
            "prepared_kernel",
            "complete_predicate_plus_event_projection",
        ] {
            let mut row = measure(|| {
                if mode == "prepared_kernel" {
                    black_box(filters_match(black_box(&filters), black_box(&event)))
                } else {
                    black_box(filters_match(
                        black_box(&filters),
                        black_box(&prepare_event(black_box(&case.event))),
                    ))
                }
            });
            row["case"] = json!(case.name);
            row["matcher"] = json!(mode);
            rows.push(row);
        }
    }
    println!("{}",serde_json::to_string_pretty(&json!({"boundary":"complete generated predicate, prepared selectors; named mode determines whether event projection is timed; excludes registry","trials":9,"warmup_calls":1000,"rows":rows})).unwrap());
}
