fn main() {
    use relay_filter_baseline::{
        current_subscription::SubscriptionRegistry as Current, fanout,
        historical_subscription::SubscriptionRegistry as Historical,
    };
    let label = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "historical".into());
    let (rows, registrations) = if label == "historical" {
        (
            fanout::measure::<Historical>(&label),
            fanout::registration::<Historical>(&label),
        )
    } else {
        (
            fanout::measure::<Current>(&label),
            fanout::registration::<Current>(&label),
        )
    };
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"boundary":"actual fan_out_scoped: indexes, current entry lookup, matching, dedup, result construction, lazy projection where deployed","trials":9,"rows":rows,"registrations":registrations})).unwrap());
}
