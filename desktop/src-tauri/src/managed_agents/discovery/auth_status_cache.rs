//! Auth-status cache for cheap ACP runtime discovery.
//!
//! A forced discovery (`discover_acp_providers(force: true)`) spawns one CLI
//! auth probe per available runtime — the expensive pipeline. The cheap default
//! discovery must not pay that cost, so it reuses the last known auth statuses
//! from this cache instead of probing. The cache is keyed by runtime id, warmed
//! by the forced probe phase, and cleared by `clear_resolve_cache` (which a
//! forced discovery calls before re-probing).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::managed_agents::AuthStatus;

fn cache() -> &'static Mutex<HashMap<String, AuthStatus>> {
    static CACHE: OnceLock<Mutex<HashMap<String, AuthStatus>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn clear() {
    if let Ok(mut guard) = cache().lock() {
        guard.clear();
    }
}

pub(super) fn store(runtime_id: &str, status: &AuthStatus) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(runtime_id.to_string(), status.clone());
    }
}

/// Last known auth status for `runtime_id`, or `AuthStatus::Unknown` when no
/// forced discovery has probed it yet. Never spawns a process.
pub(super) fn get(runtime_id: &str) -> AuthStatus {
    cache()
        .lock()
        .ok()
        .and_then(|g| g.get(runtime_id).cloned())
        .unwrap_or(AuthStatus::Unknown)
}

#[cfg(test)]
pub(crate) fn len() -> usize {
    cache().lock().map(|g| g.len()).unwrap_or(0)
}

/// Resolve the auth status of every available, probeable runtime in `partials`,
/// patching each entry's `auth_status` + `login_hint` in place.
///
/// Forced discovery spawns one CLI auth probe per available runtime (in
/// parallel; total cost = max(probe latency)) and warms this cache. The cheap
/// default path spawns nothing — it reuses the last cached status, falling back
/// to `Unknown` for a runtime never probed this session.
pub(super) fn resolve_auth_statuses(partials: &mut [super::PartialEntry], force: bool) {
    use crate::managed_agents::AcpAvailabilityStatus;

    if force {
        let probe_handles: Vec<(usize, std::thread::JoinHandle<AuthStatus>)> = partials
            .iter()
            .enumerate()
            .filter_map(|(idx, partial)| {
                if partial.entry.availability != AcpAvailabilityStatus::Available {
                    return None;
                }
                let probe_args = partial.runtime.auth_probe_args?;
                // Need the resolved binary path for the CLI (e.g. the actual `claude` binary).
                let binary_path = super::resolve_command(probe_args[0])?;
                let probe_args_owned: Vec<String> =
                    probe_args.iter().map(|s| s.to_string()).collect();

                let handle = std::thread::spawn(move || {
                    let refs: Vec<&str> = probe_args_owned.iter().map(String::as_str).collect();
                    super::probe_auth_status(&binary_path, &refs)
                });
                Some((idx, handle))
            })
            .collect();

        for (idx, handle) in probe_handles {
            let status = handle.join().unwrap_or(AuthStatus::Unknown);
            store(&partials[idx].entry.id, &status);
            patch_entry(&mut partials[idx], status);
        }
    } else {
        for partial in partials.iter_mut() {
            if partial.entry.availability != AcpAvailabilityStatus::Available
                || partial.runtime.auth_probe_args.is_none()
            {
                continue;
            }
            let status = get(&partial.entry.id);
            patch_entry(partial, status);
        }
    }
}

fn patch_entry(partial: &mut super::PartialEntry, status: AuthStatus) {
    partial.entry.login_hint = if matches!(status, AuthStatus::LoggedIn | AuthStatus::NotApplicable)
    {
        None
    } else {
        partial.runtime.login_hint.map(str::to_string)
    };
    partial.entry.auth_status = status;
}
