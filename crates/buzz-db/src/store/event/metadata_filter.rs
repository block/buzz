//! Current relay metadata and its ACL share the query's MVCC snapshot.

use sqlx::{Postgres, QueryBuilder};

/// Trusted request context for channel metadata reads (never event-supplied).
#[derive(Debug, Clone)]
pub struct ChannelMetadataRead {
    /// Authenticated reader after community and token admission.
    pub reader: Vec<u8>,
    /// Configured relay signer, not a client-supplied authors filter.
    pub relay: Vec<u8>,
}

pub(super) fn push_predicates(
    qb: &mut QueryBuilder<Postgres>,
    col_prefix: &str,
    read: Option<&ChannelMetadataRead>,
) {
    let Some(read) = read else { return };
    let table = if col_prefix.is_empty() { "events" } else { "e" };
    qb.push(format!(" AND ({table}.kind <> 39000 OR ({table}.pubkey = "))
        .push_bind(read.relay.clone())
        .push(format!(
            " AND EXISTS (SELECT 1 FROM channels mc WHERE mc.community_id = {table}.community_id \
             AND mc.id = {table}.channel_id AND mc.deleted_at IS NULL \
             AND (mc.visibility = 'open' OR EXISTS (SELECT 1 FROM channel_members mm \
             WHERE mm.community_id = mc.community_id AND mm.channel_id = mc.id \
             AND mm.removed_at IS NULL AND mm.pubkey = "
        ))
        .push_bind(read.reader.clone())
        .push(format!(
            "))) AND NOT EXISTS (SELECT 1 FROM events newer \
             WHERE newer.community_id = {table}.community_id AND newer.kind = 39000 \
             AND newer.pubkey = {table}.pubkey AND newer.channel_id = {table}.channel_id \
             AND newer.deleted_at IS NULL AND (newer.created_at > {table}.created_at \
             OR (newer.created_at = {table}.created_at AND newer.id < {table}.id)))))"
        ));
}

#[cfg(test)]
mod postgres_tests;
