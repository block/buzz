// ── Cheap vs. forced discovery: the auth-probe split ────────────────────────
//
// `discover_acp_providers(force: true)` spawns one CLI auth probe per available
// runtime; the cheap default path must reuse the last cached status and spawn
// nothing. These tests pin that split through the real `discover_acp_runtimes_from`
// pipeline with a fake `claude` CLI that records every invocation to a sentinel.

/// Build a fake `claude` runtime on a fresh PATH: the adapter (`claude-agent-acp`)
/// and the CLI (`claude`). The CLI appends a line to `probe_log` each time it
/// runs and exits 0 (→ `LoggedIn`), so the log's existence proves whether the
/// auth probe was spawned.
#[cfg(unix)]
#[test]
fn forced_discovery_probes_auth_but_cheap_discovery_reuses_cached_status() {
    use crate::managed_agents::custom_harnesses::registry_test_lock;
    use crate::managed_agents::discovery::{clear_resolve_cache, discover_acp_runtimes_from};
    use crate::managed_agents::{AcpAvailabilityStatus, AuthStatus};
    use std::os::unix::fs::PermissionsExt;

    let _path_guard = crate::managed_agents::lock_path_mutex();
    let _registry_guard = registry_test_lock();

    let dir = tempfile::tempdir().expect("tempdir");
    let probe_log = dir.path().join("claude-probe.log");

    for name in ["claude-agent-acp", "claude"] {
        let bin = dir.path().join(name);
        // The adapter is never executed; only `claude` logs + exits 0.
        let script = format!(
            "#!/bin/sh\necho ran >> \"{}\"\nexit 0\n",
            probe_log.display()
        );
        std::fs::write(&bin, script).expect("write fake bin");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    // Start from a clean resolve + auth cache, and a PATH that only sees our fakes.
    clear_resolve_cache();
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = vec![dir.path().to_path_buf()];
    new_path.extend(std::env::split_paths(&old_path));
    std::env::set_var("PATH", std::env::join_paths(&new_path).expect("join PATH"));

    let result = std::panic::catch_unwind(|| {
        // ── Forced: probes run, status is LoggedIn, cache is warmed. ──────────
        let forced = discover_acp_runtimes_from(None, true);
        let claude = forced
            .iter()
            .find(|e| e.id == "claude")
            .expect("claude entry present");
        assert_eq!(claude.availability, AcpAvailabilityStatus::Available);
        assert_eq!(claude.auth_status, AuthStatus::LoggedIn);
        assert!(
            probe_log.exists(),
            "forced discovery must spawn the auth probe"
        );
        assert!(
            super::super::auth_status_cache::len() > 0,
            "forced discovery must warm the auth-status cache"
        );

        // ── Cheap: no probe spawned, status reused from cache. ────────────────
        std::fs::remove_file(&probe_log).expect("clear probe log");
        let cheap = discover_acp_runtimes_from(None, false);
        let claude = cheap
            .iter()
            .find(|e| e.id == "claude")
            .expect("claude entry present");
        assert_eq!(
            claude.availability,
            AcpAvailabilityStatus::Available,
            "cheap path keeps availability (resolved from cache)"
        );
        assert_eq!(
            claude.auth_status,
            AuthStatus::LoggedIn,
            "cheap path must reuse the cached auth status"
        );
        assert!(
            !probe_log.exists(),
            "cheap discovery must not spawn any auth probe"
        );
    });

    // Restore global state before propagating any panic.
    std::env::set_var("PATH", &old_path);
    clear_resolve_cache();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// Before any forced probe warms the resolve cache, the cheap path resolves
/// nothing live — it must not resolve a present-but-uncached binary by spawning
/// a login shell to discover it. This is the flip side of the zero-spawn
/// contract: cache-only resolution cannot see a binary the forced path has not
/// yet cached. The forced path (exercised on every surface mount) resolves it
/// and warms the cache; a subsequent cheap call then sees it Available (covered
/// by `forced_discovery_probes_auth_but_cheap_discovery_reuses_cached_status`).
///
/// The assertion is scoped to what holds on any machine: the fake PATH-only
/// `claude` CLI must not be resolved by the cheap path (availability is never
/// `Available`, auth stays `Unknown`) and no login shell is spawned. It does
/// not pin the exact `NotInstalled` vs `CliMissing` variant, because a real
/// Buzz-managed `claude-agent-acp` shim on the host resolves via a filesystem
/// stat (production-correct, never a spawn) and yields `CliMissing` — a genuine
/// environment difference, not a regression.
#[cfg(unix)]
#[test]
fn cheap_discovery_reports_absent_before_any_forced_probe() {
    use crate::managed_agents::custom_harnesses::registry_test_lock;
    use crate::managed_agents::discovery::{
        clear_resolve_cache, discover_acp_runtimes_from, login_shell_spawn_probe,
    };
    use crate::managed_agents::{AcpAvailabilityStatus, AuthStatus};
    use std::os::unix::fs::PermissionsExt;

    let _path_guard = crate::managed_agents::lock_path_mutex();
    let _registry_guard = registry_test_lock();

    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["claude-agent-acp", "claude"] {
        let bin = dir.path().join(name);
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").expect("write fake bin");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    clear_resolve_cache(); // also clears the auth-status cache
    login_shell_spawn_probe::reset();
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = vec![dir.path().to_path_buf()];
    new_path.extend(std::env::split_paths(&old_path));
    std::env::set_var("PATH", std::env::join_paths(&new_path).expect("join PATH"));

    let result = std::panic::catch_unwind(|| {
        let cheap = discover_acp_runtimes_from(None, false);
        let claude = cheap
            .iter()
            .find(|e| e.id == "claude")
            .expect("claude entry present");
        assert_ne!(
            claude.availability,
            AcpAvailabilityStatus::Available,
            "cache-only cheap discovery must not resolve the PATH-only claude CLI live"
        );
        assert_eq!(
            claude.auth_status,
            AuthStatus::Unknown,
            "an unresolved runtime with no cached status stays Unknown"
        );
        assert_eq!(
            login_shell_spawn_probe::count(),
            0,
            "cheap discovery must not spawn a login shell to resolve the PATH-only CLI"
        );
    });

    std::env::set_var("PATH", &old_path);
    clear_resolve_cache();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
