use super::*;

// ── ensure_repos_symlink ──────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn ensure_repos_symlink_none_creates_real_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    ensure_repos_symlink(&root, None).unwrap();

    let repos = root.join("REPOS");
    assert!(repos.is_dir(), "REPOS should be a real directory");
    assert!(
        !repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "REPOS should not be a symlink when repos_dir is None"
    );
}

#[cfg(unix)]
#[test]
fn ensure_repos_symlink_none_reverts_existing_symlink_to_real_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let external = tmp.path().join("Development");
    fs::create_dir_all(&external).unwrap();
    let payload = external.join("KEEP.md");
    fs::write(&payload, "data").unwrap();

    // First point REPOS at the external dir, then clear the field.
    ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();
    ensure_repos_symlink(&root, None).unwrap();

    let repos = root.join("REPOS");
    assert!(
        repos.is_dir(),
        "REPOS should be a real directory after clear"
    );
    assert!(
        !repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "REPOS should no longer be a symlink after clearing repos_dir"
    );
    assert!(
        payload.exists(),
        "clearing repos_dir must not touch the external target's contents"
    );
}

#[cfg(unix)]
#[test]
fn ensure_repos_symlink_absent_creates_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let external = tmp.path().join("Development");
    fs::create_dir_all(&external).unwrap();

    ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();

    let repos = root.join("REPOS");
    assert!(repos.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(repos.read_link().unwrap(), external.canonicalize().unwrap());
}

#[cfg(unix)]
#[test]
fn ensure_repos_symlink_repoints_existing_wrong_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let old = tmp.path().join("old");
    let new = tmp.path().join("new");
    fs::create_dir_all(&old).unwrap();
    fs::create_dir_all(&new).unwrap();
    let payload = old.join("KEEP.md");
    fs::write(&payload, "data").unwrap();

    ensure_repos_symlink(&root, Some(old.to_str().unwrap())).unwrap();
    ensure_repos_symlink(&root, Some(new.to_str().unwrap())).unwrap();

    let repos = root.join("REPOS");
    assert_eq!(repos.read_link().unwrap(), new.canonicalize().unwrap());
    assert!(
        payload.exists(),
        "re-pointing a symlink must not touch the old target's contents"
    );
}

#[cfg(unix)]
#[test]
fn ensure_repos_symlink_correct_symlink_is_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let external = tmp.path().join("Development").canonicalize_or_make();

    ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();
    ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();

    let repos = root.join("REPOS");
    assert_eq!(repos.read_link().unwrap(), external);
}

#[cfg(unix)]
#[test]
fn ensure_repos_symlink_empty_real_dir_converts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(root.join("REPOS")).unwrap();
    let external = tmp.path().join("Development");
    fs::create_dir_all(&external).unwrap();

    ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();

    let repos = root.join("REPOS");
    assert!(
        repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "an empty real REPOS should convert to a symlink"
    );
}

#[cfg(unix)]
#[test]
fn ensure_repos_symlink_nonempty_real_dir_refuses_and_preserves() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    let repos = root.join("REPOS");
    fs::create_dir_all(&repos).unwrap();
    let checkout = repos.join("buzz");
    fs::create_dir_all(&checkout).unwrap();
    fs::write(checkout.join("code.rs"), "fn main() {}").unwrap();
    let external = tmp.path().join("Development");
    fs::create_dir_all(&external).unwrap();

    let result = ensure_repos_symlink(&root, Some(external.to_str().unwrap()));

    assert!(result.is_err(), "non-empty real REPOS must refuse");
    assert!(
        !repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "refused REPOS must stay a real directory"
    );
    assert!(
        checkout.join("code.rs").exists(),
        "refusal must never delete existing repositories"
    );
}

// ensure_nest_at must NOT clobber an existing REPOS symlink on startup.
// Regression guard for Finding 1: the startup `ensure_repos_symlink(_, None)`
// call used to remove a configured symlink and mint an empty real REPOS,
// which async-restored agents could write into — the FE re-point then
// refused the now-non-empty dir, silently breaking a configured repos_dir.
#[cfg(unix)]
#[test]
fn ensure_nest_startup_preserves_existing_repos_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");

    // First launch creates the real nest with a real REPOS dir.
    crate::managed_agents::ensure_nest_at(&root).unwrap();

    // Simulate a configured repos_dir: REPOS points at an external dir
    // holding agent checkouts.
    let external = tmp.path().join("Development");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("KEEP.md"), "data").unwrap();
    fs::remove_dir(root.join("REPOS")).unwrap();
    std::os::unix::fs::symlink(&external, root.join("REPOS")).unwrap();

    // Next launch must leave the configured symlink intact.
    crate::managed_agents::ensure_nest_at(&root).unwrap();

    let repos = root.join("REPOS");
    assert!(
        repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "an existing REPOS symlink must survive startup"
    );
    assert_eq!(repos.read_link().unwrap(), external);
    assert!(
        external.join("KEEP.md").exists(),
        "the symlink's target contents must be untouched"
    );
}

#[cfg(unix)]
#[test]
fn validate_repos_dir_rejects_tilde_relative_and_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    assert!(validate_repos_dir(&root, "~/Development").is_err());
    assert!(validate_repos_dir(&root, "relative/path").is_err());
    assert!(validate_repos_dir(&root, "/no/such/dir/here").is_err());

    let file = tmp.path().join("afile");
    fs::write(&file, "x").unwrap();
    assert!(validate_repos_dir(&root, file.to_str().unwrap()).is_err());
}

#[cfg(unix)]
#[test]
fn validate_repos_dir_rejects_nest_ancestor() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("home").join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let parent = root.parent().unwrap();

    assert!(
        validate_repos_dir(&root, parent.to_str().unwrap()).is_err(),
        "a parent of the nest would nest REPOS inside its own target"
    );
}

// ── persisted repos_dir dotfile ───────────────────────────────────────

#[test]
fn persisted_repos_dir_roundtrips_write_read_clear() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    assert_eq!(read_persisted_repos_dir(&root), None, "absent → None");

    write_persisted_repos_dir(&root, Some("  /Users/me/Development  ")).unwrap();
    assert_eq!(
        read_persisted_repos_dir(&root).as_deref(),
        Some("/Users/me/Development"),
        "value is trimmed on write/read"
    );

    write_persisted_repos_dir(&root, None).unwrap();
    assert_eq!(read_persisted_repos_dir(&root), None, "cleared → None");
    assert!(
        !root.join(".repos-dir").exists(),
        "clearing removes the dotfile"
    );
}

#[test]
fn persisted_repos_dir_empty_value_clears() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    write_persisted_repos_dir(&root, Some("/Users/me/Development")).unwrap();

    write_persisted_repos_dir(&root, Some("   ")).unwrap();
    assert_eq!(
        read_persisted_repos_dir(&root),
        None,
        "a whitespace-only value clears the override"
    );
}

#[test]
fn persisted_repos_dir_clear_when_absent_is_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    write_persisted_repos_dir(&root, None).expect("clearing an absent dotfile is not an error");
}

#[cfg(unix)]
#[test]
fn boot_resolves_symlink_from_persisted_value_into_empty_repos() {
    // Mirrors the boot sequence: ensure_nest leaves REPOS an empty real
    // dir, then the setup hook reads the persisted value and symlinks.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(root.join("REPOS")).unwrap();
    let external = tmp.path().join("Development");
    fs::create_dir_all(&external).unwrap();

    write_persisted_repos_dir(&root, Some(external.to_str().unwrap())).unwrap();
    let persisted = read_persisted_repos_dir(&root);
    ensure_repos_symlink(&root, persisted.as_deref()).unwrap();

    let repos = root.join("REPOS");
    assert!(
        repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "boot must convert the empty real REPOS into a symlink"
    );
    assert_eq!(repos.read_link().unwrap(), external.canonicalize().unwrap());
}

#[cfg(unix)]
#[test]
fn boot_leaves_already_correct_symlink_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let external = tmp.path().join("Development");
    fs::create_dir_all(&external).unwrap();

    write_persisted_repos_dir(&root, Some(external.to_str().unwrap())).unwrap();
    // First boot converts; second boot must be a noop.
    let persisted = read_persisted_repos_dir(&root);
    ensure_repos_symlink(&root, persisted.as_deref()).unwrap();
    ensure_repos_symlink(&root, persisted.as_deref()).unwrap();

    let repos = root.join("REPOS");
    assert!(repos.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(repos.read_link().unwrap(), external.canonicalize().unwrap());
}

#[cfg(unix)]
#[test]
fn boot_with_cleared_value_reverts_symlink_to_real_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let external = tmp.path().join("Development");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("KEEP.md"), "data").unwrap();

    // Configure, then clear the field.
    write_persisted_repos_dir(&root, Some(external.to_str().unwrap())).unwrap();
    ensure_repos_symlink(&root, read_persisted_repos_dir(&root).as_deref()).unwrap();
    write_persisted_repos_dir(&root, None).unwrap();

    // Next boot reads None and reverts REPOS to a real in-nest dir.
    ensure_repos_symlink(&root, read_persisted_repos_dir(&root).as_deref()).unwrap();

    let repos = root.join("REPOS");
    assert!(
        !repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "clearing the persisted value reverts REPOS to a real dir"
    );
    assert!(
        external.join("KEEP.md").exists(),
        "reverting must not touch the external target's contents"
    );
}

#[cfg(unix)]
#[test]
fn effective_repos_dir_drives_persisted_dotfile_for_all_three_cases() {
    // Pins the CRITICAL persist decision on `.repos-dir` CONTENTS, not just
    // a return value: a bad path must clear the dotfile so the next boot's
    // `should_restore_agents(false, _)` restores agents (the regression
    // this hardening fixes). Drives each case through the same
    // effective→persist path `apply_workspace` uses.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let good = tmp.path().join("Development");
    fs::create_dir_all(&good).unwrap();
    let good_str = good.to_str().unwrap();

    // Pre-seed a value so each case proves it overwrites/clears, not just
    // that an absent dotfile stays absent.
    let seed = |root: &Path| write_persisted_repos_dir(root, Some(good_str)).unwrap();
    let persist = |root: &Path, candidate: Option<&str>| {
        let effective = effective_repos_dir(root, candidate);
        // Mirror apply_workspace: Err clears the override (persist None).
        let to_persist = effective.unwrap_or(None);
        write_persisted_repos_dir(root, to_persist.as_deref()).unwrap();
    };

    // Bad path → Err → dotfile cleared (the CRITICAL).
    seed(&root);
    persist(&root, Some("/no/such/dir/here"));
    assert_eq!(
        read_persisted_repos_dir(&root),
        None,
        "a bad repos_dir must clear `.repos-dir` so the next boot restores agents"
    );

    // Good path → Ok(Some(raw)) → dotfile holds the raw trimmed value.
    persist(&root, Some(&format!("  {good_str}  ")));
    assert_eq!(
        read_persisted_repos_dir(&root).as_deref(),
        Some(good_str),
        "a valid repos_dir must persist the raw trimmed path (not the canonical path)"
    );

    // Empty/whitespace → Ok(None) → dotfile cleared.
    seed(&root);
    persist(&root, Some("   "));
    assert_eq!(
        read_persisted_repos_dir(&root),
        None,
        "an empty repos_dir clears the override"
    );

    // None candidate → Ok(None) → dotfile cleared.
    seed(&root);
    persist(&root, None);
    assert_eq!(
        read_persisted_repos_dir(&root),
        None,
        "no repos_dir clears the override"
    );
}

// ── per-relay repos_dir map ───────────────────────────────────────────

#[test]
fn repos_dir_map_roundtrips_upsert_and_clear() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-a.example"),
        None,
        "absent map → None"
    );

    persist_workspace_repos_dir(&root, "wss://relay-a.example", Some("  /Users/me/DevA  "))
        .unwrap();
    persist_workspace_repos_dir(&root, "wss://relay-b.example", Some("/Users/me/DevB")).unwrap();

    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-a.example").as_deref(),
        Some("/Users/me/DevA"),
        "value is trimmed on write"
    );
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-b.example").as_deref(),
        Some("/Users/me/DevB"),
        "entries are independent per relay"
    );

    // Clearing one workspace's override must not touch the other's.
    persist_workspace_repos_dir(&root, "wss://relay-a.example", None).unwrap();
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-a.example"),
        None
    );
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-b.example").as_deref(),
        Some("/Users/me/DevB")
    );

    // Clearing the last entry removes the file entirely.
    persist_workspace_repos_dir(&root, "wss://relay-b.example", Some("   ")).unwrap();
    assert!(
        !root.join(".repos-dirs.json").exists(),
        "an empty map removes the dotfile"
    );
}

#[test]
fn repos_dir_map_keys_are_normalized() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    // Cosmetic URL differences (case, trailing slash) are the same relay:
    // a lookup must find the entry, and a re-persist must overwrite it
    // rather than minting a second entry.
    persist_workspace_repos_dir(&root, "WSS://Relay-A.Example/", Some("/Users/me/DevA")).unwrap();
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-a.example").as_deref(),
        Some("/Users/me/DevA")
    );

    persist_workspace_repos_dir(&root, "wss://relay-a.example", Some("/Users/me/Other")).unwrap();
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "WSS://Relay-A.Example/").as_deref(),
        Some("/Users/me/Other"),
        "cosmetic variants address the same entry"
    );
    assert_eq!(
        read_repos_dir_map(&root).len(),
        1,
        "no duplicate entries for cosmetic variants"
    );
}

#[test]
fn repos_dir_map_rejects_empty_relay_and_survives_malformed_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    assert!(
        persist_workspace_repos_dir(&root, "   ", Some("/Users/me/DevA")).is_err(),
        "an empty relay URL has no identity to key on"
    );

    fs::write(root.join(".repos-dirs.json"), "not-json{{{").unwrap();
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-a.example"),
        None,
        "a malformed map degrades to empty rather than failing"
    );
    persist_workspace_repos_dir(&root, "wss://relay-a.example", Some("/Users/me/DevA")).unwrap();
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-a.example").as_deref(),
        Some("/Users/me/DevA"),
        "persisting over a malformed file recovers it"
    );
}

#[test]
fn repos_dir_map_write_leaves_no_temp_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    // The map is written via temp file + rename so a crash mid-write can
    // never leave truncated JSON (read degrades malformed to an empty map,
    // silently dropping isolation). A stale temp file — e.g. from a crash
    // between write and rename — must be overwritten, and no temp file may
    // outlive a successful write.
    fs::write(root.join(".repos-dirs.json.tmp"), "half-written{").unwrap();
    persist_workspace_repos_dir(&root, "wss://relay-a.example", Some("/Users/me/DevA")).unwrap();

    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://relay-a.example").as_deref(),
        Some("/Users/me/DevA")
    );
    assert!(
        !root.join(".repos-dirs.json.tmp").exists(),
        "the temp file is renamed into place, not left behind"
    );
}

#[test]
fn rebind_workspace_repos_dir_moves_entry_to_new_relay() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    persist_workspace_repos_dir(&root, "wss://old.example", Some("/Users/me/DevA")).unwrap();

    let moved =
        rebind_workspace_repos_dir(&root, "wss://old.example/", "WSS://New.Example").unwrap();

    assert!(moved);
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://old.example"),
        None,
        "the old relay's entry is gone"
    );
    assert_eq!(
        workspace_repos_dir_for_relay(&root, "wss://new.example").as_deref(),
        Some("/Users/me/DevA"),
        "the entry follows the edited relay URL"
    );

    // Rebinding a relay with no entry is a no-op, not an error.
    assert!(!rebind_workspace_repos_dir(&root, "wss://absent.example", "wss://x.example").unwrap());
    // A cosmetic-only edit (same normalized relay) moves nothing.
    assert!(!rebind_workspace_repos_dir(&root, "wss://new.example", "WSS://New.Example/").unwrap());
}

// ── resolve_agent_repos_dir (spawn-time isolation) ────────────────────

// THE Phase-4 immunity property: each agent resolves its own workspace's
// repos dir as a real path, regardless of where the shared REPOS symlink
// currently points (it follows the ACTIVE workspace).
#[cfg(unix)]
#[test]
fn resolve_agent_repos_dir_is_immune_to_active_workspace_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let dir_a = tmp.path().join("DevA");
    let dir_b = tmp.path().join("DevB");
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();

    // Workspace A and B each applied with their own repos_dir; B is the
    // active workspace, so REPOS symlinks to B's dir.
    persist_workspace_repos_dir(
        &root,
        "wss://relay-a.example",
        Some(dir_a.to_str().unwrap()),
    )
    .unwrap();
    persist_workspace_repos_dir(
        &root,
        "wss://relay-b.example",
        Some(dir_b.to_str().unwrap()),
    )
    .unwrap();
    ensure_repos_symlink(&root, Some(dir_b.to_str().unwrap())).unwrap();

    assert_eq!(
        resolve_agent_repos_dir(&root, "wss://relay-a.example").unwrap(),
        dir_a.canonicalize().unwrap(),
        "an A-pinned agent resolves A's real dir even while REPOS points at B"
    );
    assert_eq!(
        resolve_agent_repos_dir(&root, "wss://relay-b.example").unwrap(),
        dir_b.canonicalize().unwrap()
    );
}

#[test]
fn resolve_agent_repos_dir_fails_closed_when_override_unresolvable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let missing = tmp.path().join("not-mounted-yet");
    persist_workspace_repos_dir(
        &root,
        "wss://relay-a.example",
        Some(missing.to_str().unwrap()),
    )
    .unwrap();

    let result = resolve_agent_repos_dir(&root, "wss://relay-a.example");

    let error = result.expect_err("a configured-but-unresolvable repos dir must refuse spawn");
    assert!(
        error.contains("refusing to start"),
        "the error explains the fail-closed refusal: {error}"
    );
}

#[test]
fn resolve_agent_repos_dir_defaults_to_nest_repos_without_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();

    assert_eq!(
        resolve_agent_repos_dir(&root, "wss://relay-a.example").unwrap(),
        root.join("REPOS"),
        "no override → the shared in-nest REPOS default (pre-map behavior)"
    );
}

// ── resolve_repos_at_boot (boot sequence + fail-closed gate) ──────────

#[cfg(unix)]
#[test]
fn resolve_repos_at_boot_converts_empty_real_repos_and_allows_restore() {
    // Drives the real boot sequence: ensure_repos_setup_default (called by
    // ensure_nest) provisions REPOS as an empty real dir, then the setup
    // hook calls resolve_repos_at_boot. Asserts the convert happens at that
    // position and restore is allowed.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let external = tmp.path().join("Development");
    fs::create_dir_all(&external).unwrap();
    write_persisted_repos_dir(&root, Some(external.to_str().unwrap())).unwrap();

    ensure_repos_setup_default(&root).unwrap();
    let repos = root.join("REPOS");
    assert!(
        repos.is_dir() && !repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "ensure_nest provisions REPOS as an empty real dir before the boot resolve"
    );

    let restore = resolve_repos_at_boot(&root);

    assert!(restore, "restore proceeds when the symlink resolves");
    assert!(
        repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "boot resolve converts the empty real REPOS into the configured symlink"
    );
    assert_eq!(repos.read_link().unwrap(), external.canonicalize().unwrap());
}

#[cfg(unix)]
#[test]
fn resolve_repos_at_boot_fails_closed_when_target_unresolvable() {
    // Persisted repos_dir whose target does not exist at boot (transiently
    // unavailable external volume) → ensure_repos_symlink Errs. Restore must
    // be skipped and REPOS left as the empty real dir, never the symlink, so
    // no agent clones into the wrong place.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    let missing = tmp.path().join("not-mounted-yet");
    write_persisted_repos_dir(&root, Some(missing.to_str().unwrap())).unwrap();
    ensure_repos_setup_default(&root).unwrap();

    let restore = resolve_repos_at_boot(&root);

    assert!(
        !restore,
        "restore must be skipped when a configured repos_dir cannot resolve at boot"
    );
    let repos = root.join("REPOS");
    assert!(
        !repos.symlink_metadata().unwrap().file_type().is_symlink(),
        "REPOS must not become a symlink to an unresolved target"
    );
}

#[cfg(unix)]
#[test]
fn resolve_repos_at_boot_allows_restore_with_no_repos_dir() {
    // No configured repos_dir → the real in-nest REPOS default is correct,
    // restore proceeds normally (the fail-closed gate must not fire here).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    fs::create_dir_all(&root).unwrap();
    ensure_repos_setup_default(&root).unwrap();

    assert!(
        resolve_repos_at_boot(&root),
        "restore proceeds when no repos_dir is configured"
    );
}

#[test]
fn should_restore_agents_only_blocks_configured_unresolved_boot() {
    assert!(
        should_restore_agents(false, &Ok(())),
        "no repos_dir, symlink ok → restore"
    );
    assert!(
        should_restore_agents(false, &Err("boom".into())),
        "no repos_dir → real-dir default is correct even on Err → restore"
    );
    assert!(
        should_restore_agents(true, &Ok(())),
        "configured repos_dir resolved → restore"
    );
    assert!(
        !should_restore_agents(true, &Err("boom".into())),
        "configured repos_dir unresolved → fail closed, skip restore"
    );
}

/// Test helper: canonicalize a path, creating it as a directory first.
#[cfg(unix)]
trait CanonicalizeOrMake {
    fn canonicalize_or_make(&self) -> PathBuf;
}
#[cfg(unix)]
impl CanonicalizeOrMake for PathBuf {
    fn canonicalize_or_make(&self) -> PathBuf {
        fs::create_dir_all(self).unwrap();
        self.canonicalize().unwrap()
    }
}
