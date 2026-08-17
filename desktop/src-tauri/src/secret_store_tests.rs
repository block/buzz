//! Unit tests for [`crate::secret_store`], extracted to a sibling file to
//! keep `secret_store.rs` under the desktop file-size ratchet. Wired in via
//! `#[path = "secret_store_tests.rs"] mod tests;` under the same cfg gate.

use super::*;

// Test-only constructor: pre-seed the cache without touching the OS keychain.
impl SecretStore {
    fn with_cache(service: &str, cache: Option<HashMap<String, String>>) -> Self {
        SecretStore {
            service: service.to_string(),
            cache: Mutex::new(cache),
        }
    }
}

#[test]
fn probe_returns_present_when_key_in_cache() {
    let mut map = HashMap::new();
    map.insert("identity".to_string(), "nsec1test".to_string());
    let store = SecretStore::with_cache("buzz-test-cache-hit", Some(map));
    // Cache is warm and contains "identity" — probe must return Present
    // without touching the keychain.
    assert_eq!(store.probe("identity"), KeyringProbe::Present);
}

#[test]
fn load_returns_value_when_key_in_cache() {
    let mut map = HashMap::new();
    map.insert("identity".to_string(), "nsec1test".to_string());
    let store = SecretStore::with_cache("buzz-test-load-cache-hit", Some(map));
    // Cache is warm and contains "identity" — load must return the value
    // without touching the keychain.
    assert_eq!(
        store.load("identity").unwrap(),
        Some("nsec1test".to_string())
    );
}

// ── Cross-process race tests (require real OS keychain) ────────────────

#[ignore = "requires real OS keychain (run locally)"]
#[test]
fn test_stale_warm_cache_add_observes_prior_write() {
    // Simulates the cross-process race that stranded Will's agent keys.
    //
    // Setup: two SecretStore instances for the same service (= two
    // "processes" with separate caches). Process A warms its cache to
    // {k1}. Process B then writes {k1, k2}. Without the fix, A's next
    // mutate_blob would build from its stale {k1} cache and write
    // {k1, k3}, silently dropping k2. With the fix, A always re-reads
    // from the keychain inside the lock, so the result is {k1, k2, k3}.
    let svc = "buzz-test-race-stale-cache";

    // Clean state.
    let setup = SecretStore::keyring(svc);
    let _ = setup.delete("k1");
    let _ = setup.delete("k2");
    let _ = setup.delete("k3");

    // Process A: write k1, warming its cache.
    let store_a = SecretStore::keyring(svc);
    store_a.store("k1", "v1").unwrap();

    // Process B: write k2 (separate instance = separate cache).
    let store_b = SecretStore::keyring(svc);
    store_b.store("k2", "v2").unwrap();

    // Process A: write k3. With the fix, A re-reads inside the lock and
    // sees {k1, k2} before appending k3 — result must be {k1, k2, k3}.
    store_a.store("k3", "v3").unwrap();

    // Verify via a third reader (clean cache).
    let reader = SecretStore::keyring(svc);
    assert_eq!(
        reader.load("k1").unwrap(),
        Some("v1".to_string()),
        "k1 must survive"
    );
    assert_eq!(
        reader.load("k2").unwrap(),
        Some("v2".to_string()),
        "k2 must not be dropped"
    );
    assert_eq!(
        reader.load("k3").unwrap(),
        Some("v3".to_string()),
        "k3 must be written"
    );

    // Cleanup.
    let _ = reader.delete("k1");
    let _ = reader.delete("k2");
    let _ = reader.delete("k3");
}

#[ignore = "requires real OS keychain (run locally)"]
#[test]
fn test_concurrent_adds_neither_key_dropped() {
    // Two sequential stores from distinct instances (simulating two
    // processes each adding one key) must both be durably visible.
    let svc = "buzz-test-race-concurrent-add";

    let setup = SecretStore::keyring(svc);
    let _ = setup.delete("agent_a");
    let _ = setup.delete("agent_b");

    let store1 = SecretStore::keyring(svc);
    store1.store("agent_a", "nsec1aaa").unwrap();

    let store2 = SecretStore::keyring(svc);
    store2.store("agent_b", "nsec1bbb").unwrap();

    let reader = SecretStore::keyring(svc);
    assert_eq!(
        reader.load("agent_a").unwrap(),
        Some("nsec1aaa".to_string()),
        "agent_a must not be dropped"
    );
    assert_eq!(
        reader.load("agent_b").unwrap(),
        Some("nsec1bbb".to_string()),
        "agent_b must not be dropped"
    );

    // Cleanup.
    let _ = reader.delete("agent_a");
    let _ = reader.delete("agent_b");
}

#[test]
fn test_blob_lockfile_path_is_in_tmp_with_uid() {
    // The lockfile must be at a deterministic per-user path under /tmp —
    // invariant to $TMPDIR — so both a GUI-launched DMG (env-stripped by
    // launchd) and a terminal-launched dev build resolve the same inode and
    // achieve mutual exclusion.
    let path = blob_lockfile_path("buzz-desktop");
    #[cfg(unix)]
    {
        let uid = unsafe { libc::getuid() };
        assert!(
            path.starts_with("/tmp"),
            "lockfile {path:?} must start with /tmp (not $TMPDIR)"
        );
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(
            name.contains(&uid.to_string()),
            "lockfile {path:?} must contain uid {uid}"
        );
        assert!(
            name.contains("buzz-keychain"),
            "lockfile name must contain 'buzz-keychain'"
        );
    }
    #[cfg(not(unix))]
    {
        assert!(
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("buzz-keychain")),
            "lockfile name must contain 'buzz-keychain'"
        );
    }
}

#[test]
fn test_blob_lock_acquire_and_release() {
    // Verify the advisory lock can be acquired and released without errors.
    // This exercises the real flock/mutex path on the current platform.
    let guard = acquire_blob_lock("buzz-test-lock-smoke");
    assert!(
        guard.is_ok(),
        "advisory lock acquire must succeed: {:?}",
        guard.err()
    );
    // Drop the guard — lock is released. A second acquire must succeed.
    drop(guard);
    let guard2 = acquire_blob_lock("buzz-test-lock-smoke");
    assert!(
        guard2.is_ok(),
        "advisory lock re-acquire after release must succeed: {:?}",
        guard2.err()
    );
}

// ── W6-B: blob lockfile hardening against tmp-cleaner unlink/recreate ──────
//
// The blob lock is a service-keyed `/tmp` lockfile (distinct from the
// directory-inode transaction lock). A tmp cleaner can unlink it while a holder
// keeps its `flock`, after which a recreate under the same pathname is a fresh
// inode a second process can lock in parallel — the classic split. The fix is
// an inode recheck after the lock is granted: a holder whose locked inode no
// longer matches the live pathname re-acquires against the live one, so all
// contenders converge on a single inode.

#[cfg(all(unix, feature = "system-keyring"))]
#[test]
fn test_locked_inode_is_live_true_for_untouched_file() {
    // A freshly opened lockfile that no one has churned must read as live.
    let path = std::env::temp_dir().join(format!("buzz-w6b-live-{}.lock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = std::fs::File::create(&path).expect("create lockfile");
    assert!(
        locked_inode_is_live(&file, &path),
        "an untouched lockfile must resolve to its own locked inode"
    );
    let _ = std::fs::remove_file(&path);
}

#[cfg(all(unix, feature = "system-keyring"))]
#[test]
fn test_locked_inode_is_live_false_after_unlink_recreate() {
    // A held fd whose pathname was unlinked and recreated points at a dead
    // inode while the pathname resolves to a fresh one — the split condition
    // the recheck exists to catch.
    let path = std::env::temp_dir().join(format!("buzz-w6b-churn-{}.lock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let held = std::fs::File::create(&path).expect("create original lockfile");
    std::fs::remove_file(&path).expect("unlink original");
    std::fs::File::create(&path).expect("recreate lockfile (fresh inode)");
    assert!(
        !locked_inode_is_live(&held, &path),
        "a recreated pathname must NOT match the dead inode a holder still owns"
    );
    let _ = std::fs::remove_file(&path);
}

#[cfg(all(unix, feature = "system-keyring"))]
#[test]
fn test_locked_inode_is_live_false_when_pathname_absent() {
    // Unlinked with no recreate: the pathname does not resolve, so the holder's
    // inode cannot be the live one. The acquire loop must then re-create it.
    let path = std::env::temp_dir().join(format!("buzz-w6b-gone-{}.lock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let held = std::fs::File::create(&path).expect("create lockfile");
    std::fs::remove_file(&path).expect("unlink lockfile");
    assert!(
        !locked_inode_is_live(&held, &path),
        "an absent pathname must read as not-live"
    );
}

#[cfg(all(unix, feature = "system-keyring"))]
#[test]
fn test_blob_lock_converges_on_live_inode_after_recreate() {
    // End-to-end: a holder acquires the lock, a tmp cleaner unlinks+recreates
    // the lockfile (leaving the first guard on a dead inode), and a second
    // acquire must land on the LIVE inode — not the dead one — so a peer
    // process contending on that live pathname is still excluded.
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::AsRawFd;

    let service = format!("buzz-w6b-converge-{}", std::process::id());
    let path = blob_lockfile_path(&service);
    let _ = std::fs::remove_file(&path);

    let first = acquire_blob_lock(&service).expect("first acquire");
    let dead_ino = first.file.metadata().expect("first meta").ino();

    // Tmp cleaner churns the pathname out from under the first holder.
    std::fs::remove_file(&path).expect("unlink lockfile");
    std::fs::File::create(&path).expect("recreate lockfile");
    let live_ino = std::fs::metadata(&path).expect("live meta").ino();
    assert_ne!(dead_ino, live_ino, "recreate must yield a fresh inode");

    // A second acquire (the analog of process B) converges on the live inode.
    let second = acquire_blob_lock(&service).expect("second acquire");
    assert_eq!(
        second.file.metadata().expect("second meta").ino(),
        live_ino,
        "the hardened acquire must lock the LIVE inode, not the dead one"
    );

    // A peer opening the live pathname must be excluded while `second` holds.
    let peer = std::fs::File::open(&path).expect("peer opens live lockfile");
    let rc = unsafe { libc::flock(peer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(
        rc, -1,
        "a peer must NOT acquire the live inode while the hardened guard holds it"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EWOULDBLOCK),
        "peer exclusion must fail with EWOULDBLOCK"
    );

    drop(second);
    drop(first);
    let _ = std::fs::remove_file(&path);
}

// ── F5a: cross-process transaction lock (directory-inode based) ───────

#[test]
fn test_store_txn_lock_dir_resolves_symlinked_file_to_canonical_parent() {
    // Two dev worktrees each symlink `managed-agents.json` to one canonical
    // file (the parent dir is NOT symlinked — see `SHARED_AGENT_FILES`). The
    // transaction lock must key by the RESOLVED canonical directory so both
    // worktrees contend on the same inode; keying by the worktree's own path
    // would give them different locks over the same store.
    let root = std::env::temp_dir().join(format!("buzz-txn-key-{}", std::process::id()));
    let canonical = root.join("canonical/agents");
    std::fs::create_dir_all(&canonical).expect("create canonical agents dir");
    let real_file = canonical.join("managed-agents.json");
    std::fs::write(&real_file, b"[]").expect("write canonical store");

    let mut resolved = Vec::new();
    for wt in ["wtA", "wtB"] {
        let wt_dir = root.join(wt).join("agents");
        std::fs::create_dir_all(&wt_dir).expect("create worktree agents dir");
        let link = wt_dir.join("managed-agents.json");
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_file, &link).expect("symlink store file");
        resolved.push(store_txn_lock_dir(&link));
    }

    #[cfg(unix)]
    {
        let want = std::fs::canonicalize(&canonical).expect("canonicalize canonical dir");
        assert_eq!(
            resolved[0], want,
            "worktree A must resolve to canonical dir"
        );
        assert_eq!(
            resolved[1], want,
            "worktree B must resolve to canonical dir"
        );
        assert_eq!(
            resolved[0], resolved[1],
            "both worktrees must key the transaction lock by ONE canonical dir inode"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_store_txn_lock_dir_falls_back_to_parent_when_file_absent() {
    // First boot: the store file does not exist yet. The lock dir must still
    // resolve to the file's parent so the very first save serializes (there is
    // no committed record to lose in that window, but the API must not panic
    // or hand back a nonsense path).
    let dir = std::env::temp_dir().join(format!("buzz-txn-absent-{}/agents", std::process::id()));
    let absent = dir.join("managed-agents.json");
    assert_eq!(
        store_txn_lock_dir(&absent),
        dir,
        "an absent store file must key by its parent directory"
    );
}

/// Two INDEPENDENT participants (distinct guards on the same resolved store
/// directory — the analog of two Desktop processes contending on one shared
/// canonical `agents/` dir) must mutually exclude: while one holds the
/// transaction lock, a non-blocking acquire by the other must fail.
///
/// Uses the real `flock` path over a unique temp directory so it needs no OS
/// keychain and does not touch the shared store.
#[cfg(all(unix, feature = "system-keyring"))]
#[test]
fn test_txn_lock_excludes_a_second_participant() {
    use std::os::unix::io::AsRawFd;

    let dir = std::env::temp_dir().join(format!("buzz-txn-excl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create store dir");
    std::fs::write(dir.join("managed-agents.json"), b"[]").expect("seed store file");

    // Participant A takes the transaction lock on the directory inode.
    let guard_a = transaction_lock_at(&dir).expect("participant A acquires txn lock");

    // Participant B is an independent open file description on the SAME
    // directory (what a second process would have). A non-blocking exclusive
    // flock must fail with EWOULDBLOCK while A holds the lock.
    let dir_b = std::fs::File::open(&dir).expect("participant B opens dir");
    let rc = unsafe { libc::flock(dir_b.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(
        rc, -1,
        "a second participant must NOT acquire the txn lock while A holds it"
    );
    let err = std::io::Error::last_os_error();
    assert!(
        matches!(err.raw_os_error(), Some(libc::EWOULDBLOCK)),
        "second acquire must fail with EWOULDBLOCK, got {err:?}"
    );

    // After A releases, B's non-blocking acquire succeeds.
    drop(guard_a);
    let rc2 = unsafe { libc::flock(dir_b.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(rc2, 0, "second participant must acquire once A releases");
    let _ = unsafe { libc::flock(dir_b.as_raw_fd(), libc::LOCK_UN) };
    let _ = std::fs::remove_dir_all(&dir);
}

/// Finding 3 adversarial regression: the lock must survive a tmp-cleaner-style
/// unlink+recreate of a FILE inside the store directory. The old `/tmp`
/// lockfile split here — A held an flock on an inode, the pathname was
/// unlinked and recreated, and B locked a fresh inode with both "holding" the
/// lock. Because the transaction lock is on the DIRECTORY inode (not a file
/// inside it), churning files within cannot swap the locked inode: B must
/// still be excluded while A holds.
#[cfg(all(unix, feature = "system-keyring"))]
#[test]
fn test_txn_lock_survives_unlink_recreate_of_file_inside() {
    use std::os::unix::io::AsRawFd;

    let dir = std::env::temp_dir().join(format!("buzz-txn-unlink-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create store dir");
    let store_file = dir.join("managed-agents.json");
    std::fs::write(&store_file, b"[]").expect("seed store file");

    // A holds the directory-inode transaction lock.
    let guard_a = transaction_lock_at(&dir).expect("participant A acquires txn lock");

    // A tmp-cleaner unlinks and recreates the store file (fresh file inode).
    // The DIRECTORY inode is unchanged — it is non-empty and cannot be rmdir'd.
    std::fs::remove_file(&store_file).expect("unlink store file");
    std::fs::write(&store_file, b"[]").expect("recreate store file");

    // B (independent fd on the same directory) must STILL be excluded — the
    // exact failure the /tmp-lockfile design could not prevent.
    let dir_b = std::fs::File::open(&dir).expect("participant B opens dir");
    let rc = unsafe { libc::flock(dir_b.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(
        rc, -1,
        "B must NOT acquire after an unlink/recreate of a file inside the locked dir"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EWOULDBLOCK),
        "exclusion must hold across the file churn"
    );

    drop(guard_a);
    let rc2 = unsafe { libc::flock(dir_b.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(rc2, 0, "B acquires once A releases");
    let _ = unsafe { libc::flock(dir_b.as_raw_fd(), libc::LOCK_UN) };
    let _ = std::fs::remove_dir_all(&dir);
}

#[ignore = "requires real OS keychain (run locally)"]
#[test]
fn mutate_blob_does_not_advance_cache_on_write_failure() {
    // Copy-on-write safety: if `write_blob_raw` fails (denied prompt,
    // transient outage, ACL rejection), the cache must stay at the last
    // known durable state. A subsequent `store()` for the same key/value
    // must NOT be skipped as a no-op — the equality check must compare
    // against the durable cache, not an unpersisted candidate.
    //
    // This is a real-keychain integration test. Run locally with:
    //   cargo test -p buzz-desktop -- --ignored mutate_blob_does_not_advance
    //
    // On a machine with a reachable keychain the `store()` call succeeds
    // (result.is_ok()) and the write-failure branch is skipped — the test
    // still passes. On a machine where the write is denied (e.g., user
    // clicks Deny in the macOS prompt) result.is_err() and the assertions
    // below verify the cache invariant. We verify that after an error:
    //   1. The cache is not advanced (the previously cached key is intact).
    //   2. The failed key is not present (the dirty candidate was discarded).
    let mut map = HashMap::new();
    map.insert("existing".to_string(), "durable_val".to_string());
    let store = SecretStore::with_cache("buzz-test-cow-write-fail", Some(map));

    // Attempt to add a new key — this calls write_blob_raw against the
    // real keychain; with copy-on-write the cache must remain at {existing}
    // if the write fails.
    let result = store.store("new_key", "new_val");

    if result.is_err() {
        // Write failed (e.g., user denied the keychain prompt): confirm
        // cache was not advanced — the existing key is still intact and
        // the new key was never committed to the in-memory state.
        assert_eq!(
            store.load("existing").unwrap(),
            Some("durable_val".to_string()),
            "cache must remain at last durable state after write failure"
        );
        // load("new_key") goes through the unchanged cache (no entry),
        // then attempts migrate_legacy_key which also fails on a denied
        // keychain, returning either Ok(None) or Err — either is correct
        // since the key was never durably stored.
        let after = store.load("new_key");
        assert!(
            matches!(after, Ok(None) | Err(_)),
            "a key whose write failed must not be visible via load: {after:?}"
        );
    }
    // If result.is_ok() the write succeeded — the cache-integrity invariant
    // does not apply to the success path; no assertion needed here.
}

#[test]
fn availability_error_discriminator() {
    assert!(is_keyring_availability_error("dbus connection failed"));
    assert!(is_keyring_availability_error(
        "org.freedesktop.secrets not provided"
    ));
    assert!(is_keyring_availability_error("No Secret Service"));
    assert!(is_keyring_availability_error(
        "Platform secure storage failure"
    ));
    // A plain "not found" is per-entry, not an availability failure.
    assert!(!is_keyring_availability_error("entry not found"));
}

#[cfg(target_os = "macos")]
#[test]
fn dpk_error_discriminators() {
    // errSecMissingEntitlement = -34018 signals unsigned dev build.
    let e = SFError::from_code(-34018);
    assert!(is_dpk_unavailable(&e));
    assert!(!is_not_found(&e));
    // errSecItemNotFound = -25300 is not a DPK-unavailable error.
    let e = SFError::from_code(-25300);
    assert!(is_not_found(&e));
    assert!(!is_dpk_unavailable(&e));
}

// Integration tests that exercise the real OS keychain. Skipped in CI
// (unsigned builds lack keychain entitlements); run locally with:
//   cargo test -p buzz-desktop -- --ignored blob_
//
// Each test uses a unique service name to avoid cross-test pollution.

#[ignore = "requires real OS keychain (run locally)"]
#[test]
fn blob_stores_and_retrieves_multiple_keys() {
    let store = SecretStore::keyring("buzz-test-blob-multi");
    store.store("key_a", "val_a").unwrap();
    store.store("key_b", "val_b").unwrap();
    assert_eq!(store.load("key_a").unwrap(), Some("val_a".to_string()));
    assert_eq!(store.load("key_b").unwrap(), Some("val_b".to_string()));
    assert_eq!(store.load("key_c").unwrap(), None);
    // Cleanup.
    let _ = store.delete("key_a");
    let _ = store.delete("key_b");
}

#[ignore = "requires real OS keychain (run locally)"]
#[test]
fn blob_probe_present_absent_unreachable() {
    let store = SecretStore::keyring("buzz-test-blob-probe");
    // No blob yet — key absent, backend reachable.
    assert_eq!(store.probe("identity"), KeyringProbe::ReachableButEmpty);
    store.store("identity", "nsec1test").unwrap();
    // Key now present.
    assert_eq!(store.probe("identity"), KeyringProbe::Present);
    // Different key — blob exists but key absent.
    assert_eq!(store.probe("other"), KeyringProbe::ReachableButEmpty);
    // Cleanup.
    let _ = store.delete("identity");
}

#[ignore = "requires real OS keychain (run locally)"]
#[test]
fn blob_delete_removes_key_not_others() {
    let store = SecretStore::keyring("buzz-test-blob-delete");
    store.store("keep", "keep_val").unwrap();
    store.store("remove", "remove_val").unwrap();
    store.delete("remove").unwrap();
    assert_eq!(store.load("keep").unwrap(), Some("keep_val".to_string()));
    assert_eq!(store.load("remove").unwrap(), None);
    // Cleanup.
    let _ = store.delete("keep");
}

#[ignore = "requires real OS keychain (run locally)"]
#[test]
fn blob_migration_from_per_key_entry() {
    let svc = "buzz-test-blob-migration";
    let key = "identity";
    let value = "nsec1migrationtest";

    // Seed a per-key entry (old format) — no blob exists.
    let entry = keyring_entry(svc, key).unwrap();
    entry.set_password(value).unwrap();

    // Fresh store — no blob in the keychain yet.
    let store = SecretStore::keyring(svc);

    // probe should find the legacy key.
    assert_eq!(store.probe(key), KeyringProbe::Present);

    // load should migrate it into the blob and return the value.
    assert_eq!(store.load(key).unwrap(), Some(value.to_string()));

    // Old per-key entry should be cleaned up.
    let entry = keyring_entry(svc, key).unwrap();
    assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));

    // Key is now in the blob — probe confirms.
    let store2 = SecretStore::keyring(svc);
    assert_eq!(store2.probe(key), KeyringProbe::Present);
    assert_eq!(store2.load(key).unwrap(), Some(value.to_string()));

    // Cleanup.
    let _ = store2.delete(key);
}

#[ignore = "requires real OS keychain (run locally)"]
#[test]
fn delete_all_with_legacy_cleanup_removes_per_key_identity() {
    let svc = "buzz-test-delete-all-legacy";
    let key = "identity";
    let value = "nsec1legacytest";

    // Seed a legacy per-key entry (old format, pre-blob migration).
    let entry = keyring_entry(svc, key).unwrap();
    entry.set_password(value).unwrap();

    // Also seed a blob with a different key to exercise the full path.
    let store = SecretStore::keyring(svc);
    store.store("agent:abc123", "nsec1agent").unwrap();

    // Legacy per-key identity should be discoverable via probe.
    let store2 = SecretStore::keyring(svc);
    assert_eq!(store2.probe(key), KeyringProbe::Present);

    // Wipe everything via the sign-out path.
    store2.delete_all_with_legacy_cleanup().unwrap();

    // Fresh store — neither the blob nor the per-key entry should remain.
    let store3 = SecretStore::keyring(svc);
    assert_eq!(
        store3.probe(key),
        KeyringProbe::ReachableButEmpty,
        "per-key identity must not survive delete_all_with_legacy_cleanup"
    );
    assert_eq!(
        store3.load(key).unwrap(),
        None,
        "load must not resurrect the legacy per-key identity"
    );
    // Agent key should also be gone.
    assert_eq!(store3.load("agent:abc123").unwrap(), None);
}
