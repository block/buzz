//! Discovery execution + single-flight coalescing for the ACP runtime catalog.
//!
//! `force: false` serves from the process caches (no clear, no PATH re-fetch, no
//! CLI auth probes) — the low-millisecond path hot surfaces render from.
//!
//! `force: true` runs the expensive probe pipeline. React Query already dedups
//! the hook consumers; the single-flight here is the seatbelt for non-hook
//! invoke paths, so a burst of forced triggers coalesces onto one in-flight run
//! instead of stacking the pipeline.

use super::AcpRuntimeCatalogEntry;

type BoxedDiscovery = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<AcpRuntimeCatalogEntry>, String>> + Send>,
>;
type SharedDiscovery = futures_util::future::Shared<BoxedDiscovery>;

fn inflight() -> &'static std::sync::Mutex<Option<SharedDiscovery>> {
    use std::sync::{Mutex, OnceLock};
    static INFLIGHT: OnceLock<Mutex<Option<SharedDiscovery>>> = OnceLock::new();
    INFLIGHT.get_or_init(|| Mutex::new(None))
}

/// Discover the ACP runtime catalog. Cheap calls run directly; forced calls
/// coalesce onto a single shared run (see module docs).
pub(super) async fn discover(
    app: tauri::AppHandle,
    force: bool,
) -> Result<Vec<AcpRuntimeCatalogEntry>, String> {
    if !force {
        return run(app, false).await;
    }

    let shared = {
        let mut guard = inflight().lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(existing) => existing.clone(),
            None => {
                let fut: BoxedDiscovery = Box::pin(run(app, true));
                let shared = futures_util::FutureExt::shared(fut);
                *guard = Some(shared.clone());
                shared
            }
        }
    };

    let result = shared.clone().await;

    // Clear the slot so the next forced call re-runs — but only if it still
    // points at the future we just awaited (a newer run may have replaced it).
    {
        let mut guard = inflight().lock().unwrap_or_else(|e| e.into_inner());
        if guard
            .as_ref()
            .is_some_and(|current| current.ptr_eq(&shared))
        {
            *guard = None;
        }
    }

    result
}

async fn run(app: tauri::AppHandle, force: bool) -> Result<Vec<AcpRuntimeCatalogEntry>, String> {
    tokio::task::spawn_blocking(move || {
        use tauri::Manager;
        if force {
            crate::managed_agents::clear_resolve_cache();
            crate::managed_agents::refresh_login_shell_path();
        }
        let custom_dir = app
            .path()
            .app_data_dir()
            .ok()
            .map(|d| d.join("custom_harnesses"));
        crate::managed_agents::discover_acp_runtimes_from(custom_dir.as_deref(), force)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}
