use super::*;

// ── is_dev_data_dir_name predicate ──────────────────────────────────────────

#[test]
fn is_dev_data_dir_name_rejects_prod_identifier() {
    assert!(!is_dev_data_dir_name("xyz.block.buzz.app"));
}

#[test]
fn is_dev_data_dir_name_accepts_canonical_dev_identifier() {
    assert!(is_dev_data_dir_name("xyz.block.buzz.app.dev"));
}

#[test]
fn is_dev_data_dir_name_accepts_worktree_dev_identifier() {
    assert!(is_dev_data_dir_name("xyz.block.buzz.app.dev.some-worktree"));
}

/// Prefix-collision guard: an identifier that merely starts with the dev
/// prefix but is not dot-separated must be treated as prod, not dev.
/// `xyz.block.buzz.app.developer` is a hypothetical prod variant, not a
/// worktree of `xyz.block.buzz.app.dev`.
#[test]
fn is_dev_data_dir_name_rejects_prefix_collision() {
    assert!(!is_dev_data_dir_name("xyz.block.buzz.app.developer"));
}
