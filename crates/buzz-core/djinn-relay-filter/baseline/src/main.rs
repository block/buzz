#[path = "../../src/fixtures.rs"]
mod fixtures;
#[path = "../../src/historical.rs"]
mod historical;
#[path = "../../src/timing.rs"]
mod timing;
pub use buzz_core::prepared_filter;
fn prototype(filters: &[nostr::Filter], event: &buzz_core::StoredEvent) -> bool {
    prepared_filter::filters_match(
        &prepared_filter::prepare_filters(filters),
        &prepared_filter::prepare_event(event),
    )
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("prepared") {
        timing::run_prepared();
    } else {
        timing::run(&[
            ("historical", historical::filters_match),
            ("rejected_per_call_prototype", prototype),
        ]);
    }
}
