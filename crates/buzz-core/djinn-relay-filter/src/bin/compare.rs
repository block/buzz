fn main() {
    djinn_relay_filter::timing::run(&[
        ("historical", djinn_relay_filter::historical::filters_match),
        (
            "rejected_per_call_prototype",
            djinn_relay_filter::per_call_prototype,
        ),
        ("production", buzz_core::filter::filters_match),
    ]);
}
