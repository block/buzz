use std::{env, fs, path::PathBuf};
fn main() {
    let registry = PathBuf::from("../../../buzz-relay/src/subscription.rs");
    println!("cargo:rerun-if-changed={}", registry.display());
    let text = fs::read_to_string(&registry).unwrap();
    let old = "use buzz_core::{\n    prepared_filter::{filters_match, prepare_event, prepare_filters, EventInput, FilterInput},\n    CommunityId, StoredEvent,\n};";
    assert_eq!(text.matches(old).count(), 1);
    let generated = text.replace(old, "use crate::first_prepared_filter::{filters_match, prepare_event, prepare_filters, EventInput, FilterInput};\nuse buzz_core::{CommunityId, StoredEvent};").replace("//!", "//");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("first_subscription.rs"),
        generated,
    )
    .unwrap();
}
