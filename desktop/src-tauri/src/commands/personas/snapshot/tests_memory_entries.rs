//! Tests for `memory_entries_from_listing` — the shared level → entries
//! selection used by both snapshot export and card minting. Split from
//! `tests.rs` to keep that file under the 1500-line gate; `#[path]`-included
//! from there as a child module, so `super::*` resolves to `tests`'s parent
//! scope re-exports.

use super::*;

fn listing_fixture() -> crate::commands::engrams::AgentMemoryListing {
    let entry = |slug: &str, body: &str| crate::commands::engrams::EngramEntry {
        slug: slug.to_string(),
        body: body.to_string(),
        event_id: "e".repeat(64),
        created_at: 1,
        outgoing_refs: vec![],
    };
    crate::commands::engrams::AgentMemoryListing {
        core: Some(entry("core", "core body")),
        memories: vec![entry("mem/a", "a body"), entry("mem/b", "b body")],
        truncated: false,
        fetched_at: 1,
    }
}

#[test]
fn memory_entries_core_takes_core_only() {
    let entries = memory_entries_from_listing(listing_fixture(), MemoryLevel::Core);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].slug, "core");
    assert_eq!(entries[0].body, "core body");
}

#[test]
fn memory_entries_everything_appends_mem_entries_after_core() {
    let entries = memory_entries_from_listing(listing_fixture(), MemoryLevel::Everything);
    assert_eq!(
        entries.iter().map(|e| e.slug.as_str()).collect::<Vec<_>>(),
        vec!["core", "mem/a", "mem/b"]
    );
}

#[test]
fn memory_entries_missing_core_still_yields_mem_entries_for_everything() {
    let mut listing = listing_fixture();
    listing.core = None;
    let entries = memory_entries_from_listing(listing, MemoryLevel::Everything);
    assert_eq!(
        entries.iter().map(|e| e.slug.as_str()).collect::<Vec<_>>(),
        vec!["mem/a", "mem/b"]
    );

    let mut core_only = listing_fixture();
    core_only.core = None;
    assert!(memory_entries_from_listing(core_only, MemoryLevel::Core).is_empty());
}
