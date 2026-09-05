//! Fetch authority before boot can mint a first private head from stale disk.
//! Failure suppresses boot publication until the next workspace apply; it does
//! not turn a partial/unauthenticated history into an authoritative empty set.

use buzz_core_pkg::kind::{KIND_DELETION, KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT};
use nostr::JsonUtil;
use tauri::Manager;

const PAGE_LIMIT: usize = 500;
const MAX_PAGES: usize = 200;
const MAX_HISTORY_BYTES: usize = 32 * 1024 * 1024;

/// Fetch the complete history before applying any of it. HTTP queries are
/// NIP-98 authenticated with the captured owner, not an unauthenticated WS REQ.
pub(super) async fn bootstrap<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    owner_keys: &nostr::Keys,
) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    let scope = crate::managed_agents::retention::active_retention_scope(app, &state)?;
    if scope.owner_keys.public_key() != owner_keys.public_key() {
        return Err("managed-agent bootstrap owner changed".into());
    }
    let result = bootstrap_scope(app, owner_keys, &scope).await;
    // A transient toast can be missed during startup. Keep this exact scope's
    // failure queryable until a successful retry; never publish another scope's
    // late result after a community or identity change.
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    ensure_current_scope(app, &scope)?;
    *state
        .managed_agent_bootstrap_error
        .lock()
        .map_err(|e| e.to_string())? = result
        .as_ref()
        .err()
        .map(|error| (scope.db_path.clone(), error.clone()));
    result
}

async fn bootstrap_scope<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    owner_keys: &nostr::Keys,
    scope: &crate::managed_agents::retention::RetentionScope,
) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    let owner = owner_keys.public_key().to_hex();
    let base_url = crate::relay::relay_http_base_url(&scope.relay_url);
    let mut history = History::default();
    // Bound total startup delay as well as page count. Cached authority can
    // still hydrate offline; no new boot publication is allowed on timeout.
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for _ in 0..MAX_PAGES {
            let mut filter = serde_json::json!({
                "kinds": [KIND_DELETION, KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT],
                "authors": [&owner], "limit": PAGE_LIMIT,
            });
            if let Some((until, before_id)) = history.cursor {
                filter["until"] = until.into();
                filter["before_id"] = before_id.to_hex().into();
            }
            let page = crate::relay::query_relay_at_with_keys(
                &state,
                &base_url,
                &[filter],
                owner_keys,
                None,
            )
            .await?;
            if history.push(page, owner_keys.public_key())? {
                return Ok(());
            }
        }
        Err("managed-agent history exceeds bootstrap page limit".to_string())
    })
    .await
    .map_err(|_| "managed-agent history bootstrap timed out".to_string())??;

    let app = app.clone();
    let scope = crate::managed_agents::retention::RetentionScope {
        db_path: scope.db_path.clone(),
        relay_url: scope.relay_url.clone(),
        owner_keys: scope.owner_keys.clone(),
    };
    tokio::task::spawn_blocking(move || {
        // Inbound intentionally no-ops stale arrivals. Such a no-op cannot be
        // counted as a completed bootstrap (including an empty response).
        ensure_current_scope(&app, &scope)?;
        // Apply private heads first so a newer explicit recreation survives an
        // older tombstone. Retention's cross-kind watermark rejects the reverse
        // ordering as well, including our own unpublished local deletions.
        history
            .events
            .sort_by_key(|event| event.kind.as_u16() == KIND_DELETION as u16);
        // Public-only recreations from old clients protect local identity/key
        // cleanup, but are not retained as already-applied public policy. The
        // ordinary subscription still owns that policy's stop/restart path.
        let mut public_heads = std::collections::HashMap::<String, u64>::new();
        for head in &history.events {
            if head.kind.as_u16() as u32 == KIND_MANAGED_AGENT {
                if let Some(agent) = head.tags.identifier() {
                    public_heads
                        .entry(agent.to_string())
                        .and_modify(|timestamp| {
                            *timestamp = (*timestamp).max(head.created_at.as_secs())
                        })
                        .or_insert(head.created_at.as_secs());
                }
            }
        }
        for event in history.events {
            if crate::commands::retain_bootstrap_deletion_with_public_witness(
                &event,
                &public_heads,
                &scope.relay_url,
                &app,
            )? {
                continue;
            }
            crate::commands::reconcile_managed_agent_bootstrap_event(
                &event,
                &scope.relay_url,
                &app,
            )?;
        }
        ensure_current_scope(&app, &scope)
    })
    .await
    .map_err(|error| format!("managed-agent bootstrap task failed: {error}"))?
}

fn ensure_current_scope<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    expected: &crate::managed_agents::retention::RetentionScope,
) -> Result<(), String> {
    let current = crate::managed_agents::retention::active_retention_scope(app, &app.state())?;
    if current.db_path != expected.db_path {
        return Err("managed-agent bootstrap scope changed".into());
    }
    Ok(())
}

#[derive(Default)]
struct History {
    cursor: Option<(u64, nostr::EventId)>,
    events: Vec<nostr::Event>,
    bytes: usize,
}

impl History {
    /// Match /query's keyset order: created_at DESC, id ASC. Reject ignored
    /// cursors and unordered responses rather than skipping unseen history.
    fn push(&mut self, page: Vec<nostr::Event>, owner: nostr::PublicKey) -> Result<bool, String> {
        let complete = page.len() < PAGE_LIMIT;
        if page.len() > PAGE_LIMIT {
            return Err("managed-agent history exceeded requested page size".into());
        }
        for event in page {
            event
                .verify()
                .map_err(|error| format!("invalid bootstrap event: {error}"))?;
            if event.pubkey != owner
                || !matches!(
                    event.kind.as_u16() as u32,
                    KIND_DELETION | KIND_MANAGED_AGENT | KIND_PRIVATE_MANAGED_AGENT
                )
            {
                return Err("out-of-scope managed-agent bootstrap event".into());
            }
            let timestamp = event.created_at.as_secs();
            if self.cursor.is_some_and(|(until, before_id)| {
                timestamp > until || (timestamp == until && event.id <= before_id)
            }) {
                return Err("managed-agent history ignored its cursor or ordering".into());
            }
            self.bytes = self.bytes.saturating_add(event.as_json().len());
            if self.bytes > MAX_HISTORY_BYTES {
                return Err("managed-agent history exceeds bootstrap byte limit".into());
            }
            self.cursor = Some((timestamp, event.id));
            self.events.push(event);
        }
        Ok(complete)
    }
}

#[cfg(test)]
mod tests;
