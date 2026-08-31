//! Historical reads used by WS REQ and the HTTP bridge.
//!
//! Host reconciliation is a write decision, not a bounded-stale display read.
use buzz_core::{kind::KIND_HOST, StoredEvent};
use buzz_db::{Db, EventQuery};
use nostr::Filter;

#[derive(Debug, thiserror::Error)]
pub(crate) enum HistoryError {
    #[error("database error: {0}")]
    Database(#[from] buzz_db::DbError),
}

impl HistoryError {
    // Never put database diagnostics or private query metadata on the wire.
    pub(crate) fn wire_message(&self) -> &'static str {
        match self {
            Self::Database(_) => "error: database error",
        }
    }
}

pub(crate) fn explicitly_requests_hosts(filter: &Filter) -> bool {
    filter.kinds.as_ref().is_some_and(|kinds| {
        kinds.iter().any(|kind| {
            matches!(
                u32::from(kind.as_u16()),
                KIND_HOST | buzz_core::kind::KIND_HOST_COMMAND | buzz_core::kind::KIND_HOST_RECEIPT
            )
        })
    })
}

pub(crate) fn requires_primary(filter: &Filter) -> bool {
    // Kindless known-ID reads can also return a host. Do not let that spelling
    // turn a reconciliation read into a replica read. Explicit unrelated kinds
    // keep their existing routing. Kind 50000 currently admits only buzz.host.v1.
    filter.kinds.is_none() || explicitly_requests_hosts(filter)
}

pub(crate) async fn query(
    db: &Db,
    path: &'static str,
    filter: &Filter,
    mut params: EventQuery,
) -> Result<Vec<StoredEvent>, HistoryError> {
    if !requires_primary(filter) {
        return Ok(db.query_events_routed(path, &params).await?);
    }
    // event_mentions is a best-effort index written AFTER the event commit.
    // Even on the primary that join can transiently (or permanently, on index
    // write failure) hide an existing registration. Use the authoritative event
    // tags for host-capable reads. All other per-result gates still run.
    // The bridge's unrelated buzz-channel extension also uses custom_tag;
    // retain it rather than changing the semantics of a kindless query.
    if params.custom_tag.is_none() {
        if let Some(owner) = params.p_tag_hex.take() {
            params.custom_tag = Some(("p".into(), owner));
        }
    }
    if explicitly_requests_hosts(filter) {
        // Push every generic tag into the authoritative query. A short page
        // now proves exhaustion, even when thousands of unrelated profiles or
        // other owners' records would otherwise consume the candidate LIMIT.
        params.p_tag_hex = None;
        params.exact_tags = filter
            .generic_tags
            .iter()
            .map(|(name, values)| (name.to_string(), values.iter().cloned().collect()))
            .collect();
    }
    Ok(db.query_events(&params).await?)
}

#[cfg(test)]
mod tests;
