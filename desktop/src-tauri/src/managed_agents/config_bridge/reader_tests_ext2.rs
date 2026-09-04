//! Additional tests for `config_bridge/reader.rs` — split out to keep
//! `reader_tests_ext.rs` under the 1000-line file-size ratchet.
//!
//! Included as `mod ext2` inside `reader_tests.rs`, so `use super::*` gives
//! access to all helpers and types from that module.

use super::*;

// ── Fix (external review #4): reader resolves record effort keys ─────────────
// case-insensitively, matching the launch projection.
//
// Windows `Command` case-folds env names, so a hand-set `goose_thinking_effort`
// is the same variable as its canonical form. The reader must resolve it as the
// record-native effort winner AND hide it from Advanced, or the panel disagrees
// with the child the launch projection already consumed the key for.

/// Mixed-case native record key `goose_thinking_effort=high` wins the record
/// tier and is hidden from Advanced (not shown as a spurious editable extra).
#[test]
fn record_mixed_case_native_effort_wins_and_hidden_from_advanced_reader() {
    let mut record = test_record();
    record
        .env_vars
        .insert("goose_thinking_effort".to_string(), "high".to_string());
    let runtime = test_runtime(); // Goose (native GOOSE_THINKING_EFFORT)

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), None, &no_tiers(), None)
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("mixed-case native key must surface as the record effort winner");
    assert_eq!(effort.value.as_deref(), Some("high"));

    let advanced_keys: Vec<&str> = surface.advanced.iter().map(|f| f.key.as_str()).collect();
    assert!(
        !advanced_keys.contains(&"goose_thinking_effort"),
        "consumed mixed-case native effort key must not appear in advanced; got {advanced_keys:?}"
    );
}

/// Mixed-case legacy record key `buzz_agent_thinking_effort=high` (no native,
/// no column) supplies the record effort AND is hidden from Advanced.
#[test]
fn record_mixed_case_legacy_effort_consumed_and_hidden_from_advanced_reader() {
    let mut record = test_record();
    record
        .env_vars
        .insert("buzz_agent_thinking_effort".to_string(), "high".to_string());
    let runtime = test_runtime(); // Goose

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), None, &no_tiers(), None)
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("mixed-case legacy key must surface as effort via record-tier alias");
    assert_eq!(effort.value.as_deref(), Some("high"));

    let advanced_keys: Vec<&str> = surface.advanced.iter().map(|f| f.key.as_str()).collect();
    assert!(
        !advanced_keys.contains(&"buzz_agent_thinking_effort"),
        "consumed mixed-case legacy effort key must not appear in advanced; got {advanced_keys:?}"
    );
}
