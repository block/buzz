//! Frozen optimization objective: actual indexed live WebSocket fanout.
//! Includes current-entry guards/lookups, lazy event projection, the complete
//! generated matcher, deduplication and result construction. Registration and
//! immutable selector preparation belong to a separate lifetime/measurement.
//! Equal case weighting is synthetic; this is not a measured traffic mix.
use relay_filter_baseline::{
    current_subscription::SubscriptionRegistry,
    fanout::{self, Registry},
};
use std::{hint::black_box, sync::OnceLock};

pub fn workload() {
    static REGISTRIES: OnceLock<Vec<SubscriptionRegistry>> = OnceLock::new();
    let registries = REGISTRIES.get_or_init(|| {
        fanout::cases()
            .iter()
            .map(|case| {
                let registry = fanout::populate::<SubscriptionRegistry>(case);
                assert_eq!(
                    registry
                        .fanout(case.query_community, &case.case.event)
                        .len(),
                    case.expected_count,
                    "{}",
                    case.case.name
                );
                registry
            })
            .collect()
    });
    let mut hits = 0usize;
    for (registry, case) in registries.iter().zip(fanout::cases()) {
        hits += black_box(
            registry.fanout(black_box(case.query_community), black_box(&case.case.event)),
        )
        .len();
    }
    black_box(hits);
}
