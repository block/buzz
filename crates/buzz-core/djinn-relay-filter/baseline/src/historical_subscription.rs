//! Subscription registry with active WebSocket indexes for targeted fan-out.

use std::collections::{HashMap, HashSet};

use dashmap::DashMap;
use nostr::{Alphabet, Filter, Kind, SingleLetterTag};
use uuid::Uuid;

use buzz_core::{filter::filters_match, CommunityId, StoredEvent};

/// Connection identifier — a UUID assigned to each WebSocket connection.
pub type ConnId = Uuid;
/// Subscription identifier — the client-supplied string from a REQ message.
pub type SubId = String;
/// Stored subscription entry: filters paired with server-resolved community and optional channel scope.
pub type SubEntry = (Vec<Filter>, CommunityId, SubscriptionScope);

/// Server-resolved live-routing scope for a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionScope {
    /// Community-global events only.
    Global,
    /// Events from any of these authorized channels.
    Channels(Vec<Uuid>),
}

impl SubscriptionScope {
    fn matches_channel(&self, channel_id: Option<Uuid>) -> bool {
        match (self, channel_id) {
            (Self::Global, None) => true,
            (Self::Channels(channels), Some(channel_id)) => channels.contains(&channel_id),
            _ => false,
        }
    }

    /// Return the channels retained by this routing scope.
    pub fn channel_ids(&self) -> &[Uuid] {
        match self {
            Self::Global => &[],
            Self::Channels(channels) => channels,
        }
    }

    /// Whether this routing scope retains the community-global topic.
    pub fn is_global(&self) -> bool {
        matches!(self, Self::Global)
    }
}

/// Index key combining a channel and event kind for O(1) fan-out lookups.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexKey {
    /// The channel this key is scoped to.
    pub channel_id: Uuid,
    /// The Nostr event kind this key is scoped to.
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GlobalPKindIndexKey {
    community_id: CommunityId,
    kind: Kind,
    p: String,
}

/// A removed subscription's server-resolved routing scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedSubscription {
    /// Server-resolved community this subscription belonged to.
    pub community_id: CommunityId,
    /// Server-resolved topics retained by the removed subscription.
    pub scope: SubscriptionScope,
}

/// Result of removing one revoked channel from a live subscription scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSubscriptionUpdate {
    /// Client-supplied subscription identifier.
    pub sub_id: SubId,
    /// Whether no authorized channels remain and the subscription was removed.
    pub removed: bool,
}

/// Thread-safe registry of active subscriptions with targeted in-memory fan-out indexes.
#[derive(Debug, Default)]
pub struct SubscriptionRegistry {
    /// Maps conn_id → sub_id → (filters, community_id, channel_id).
    /// Storing scope alongside filters enables O(1) targeted index removal and
    /// gives lifecycle code the exact Redis topic to release.
    subs: DashMap<ConnId, HashMap<SubId, SubEntry>>,
    channel_kind_index: DashMap<(CommunityId, IndexKey), Vec<(ConnId, SubId)>>,
    /// Subscriptions with a channel_id but no kind filter — need to receive ALL kinds.
    channel_wildcard_index: DashMap<(CommunityId, Uuid), Vec<(ConnId, SubId)>>,
    /// Global subscriptions indexed by kind — avoids O(all_subs) scan for global events.
    global_kind_index: DashMap<(CommunityId, Kind), Vec<(ConnId, SubId)>>,
    /// Global subscriptions indexed by both kind and `#p` recipient.
    global_p_kind_index: DashMap<GlobalPKindIndexKey, Vec<(ConnId, SubId)>>,
    /// Global subscriptions with no kind filter — wildcard, receives all global events.
    global_wildcard_index: DashMap<CommunityId, Vec<(ConnId, SubId)>>,
}

impl SubscriptionRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces any existing subscription with the same sub_id (NIP-01), scoped
    /// to the server-resolved community that owns the connection.
    pub fn register_scoped(
        &self,
        community_id: CommunityId,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<Filter>,
        channel_id: Option<Uuid>,
    ) -> Option<RemovedSubscription> {
        let scope = channel_id
            .map(|channel_id| SubscriptionScope::Channels(vec![channel_id]))
            .unwrap_or(SubscriptionScope::Global);
        self.register_with_scope(community_id, conn_id, sub_id, filters, scope)
    }

    /// Register a subscription under every authorized requested channel.
    pub fn register_channels_scoped(
        &self,
        community_id: CommunityId,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<Filter>,
        channel_ids: Vec<Uuid>,
    ) -> Option<RemovedSubscription> {
        self.register_with_scope(
            community_id,
            conn_id,
            sub_id,
            filters,
            SubscriptionScope::Channels(channel_ids),
        )
    }

    fn register_with_scope(
        &self,
        community_id: CommunityId,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<Filter>,
        scope: SubscriptionScope,
    ) -> Option<RemovedSubscription> {
        let removed = self.remove_subscription(conn_id, &sub_id);

        self.subs.entry(conn_id).or_default().insert(
            sub_id.clone(),
            (filters.clone(), community_id, scope.clone()),
        );
        metrics::gauge!("buzz_subscriptions_active").increment(1.0);

        if let SubscriptionScope::Channels(channel_ids) = &scope {
            for ch_id in channel_ids {
                let ch_id = *ch_id;
                match extract_kinds_from_filters(&filters) {
                    None => {
                        // At least one filter has no `kinds` constraint — wildcard,
                        // this sub wants all kinds in this channel.
                        self.channel_wildcard_index
                            .entry((community_id, ch_id))
                            .or_default()
                            .push((conn_id, sub_id.clone()));
                    }
                    Some(kinds) if kinds.is_empty() => {
                        // All filters had explicit empty kinds lists (`kinds: []`).
                        // Per NIP-01, `kinds: []` means "match no kinds" — this
                        // subscription will never receive any events. Do not index it
                        // anywhere; `filters_match` will reject all events at fan-out.
                    }
                    Some(kinds) => {
                        for kind in kinds {
                            let key = IndexKey {
                                channel_id: ch_id,
                                kind,
                            };
                            self.channel_kind_index
                                .entry((community_id, key))
                                .or_default()
                                .push((conn_id, sub_id.clone()));
                        }
                    }
                }
            }
        } else {
            // Global subscription. Fully p-constrained filters can use the
            // narrower (kind, #p) index; broader filters stay on the generic
            // kind/wildcard indexes.
            if let Some(keys) = extract_global_p_kind_index_keys(community_id, &filters) {
                for key in keys {
                    self.global_p_kind_index
                        .entry(key)
                        .or_default()
                        .push((conn_id, sub_id.clone()));
                }
            } else {
                match extract_kinds_from_filters(&filters) {
                    None => {
                        self.global_wildcard_index
                            .entry(community_id)
                            .or_default()
                            .push((conn_id, sub_id.clone()));
                    }
                    Some(kinds) if kinds.is_empty() => {}
                    Some(kinds) => {
                        for kind in kinds {
                            self.global_kind_index
                                .entry((community_id, kind))
                                .or_default()
                                .push((conn_id, sub_id.clone()));
                        }
                    }
                }
            }
        }

        removed
    }

    /// Test-only convenience wrapper preserving the original single-tenant test API.
    #[cfg(test)]
    pub fn register(
        &self,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<Filter>,
        channel_id: Option<Uuid>,
    ) {
        self.register_scoped(test_community(), conn_id, sub_id, filters, channel_id);
    }

    /// Remove a single subscription and clean up its index entries.
    pub fn remove_subscription(
        &self,
        conn_id: ConnId,
        sub_id: &str,
    ) -> Option<RemovedSubscription> {
        self.remove_subscription_inner(conn_id, sub_id, || {})
    }

    fn remove_subscription_inner<F>(
        &self,
        conn_id: ConnId,
        sub_id: &str,
        after_remove: F,
    ) -> Option<RemovedSubscription>
    where
        F: FnOnce(),
    {
        let mut conn_subs = self.subs.get_mut(&conn_id)?;
        let (filters, community_id, scope) = conn_subs.remove(sub_id)?;

        after_remove();
        self.remove_from_index(conn_id, sub_id, &filters, community_id, &scope);
        drop(conn_subs);

        metrics::gauge!("buzz_subscriptions_active").decrement(1.0);
        Some(RemovedSubscription {
            community_id,
            scope,
        })
    }

    /// Remove all subscriptions for a connection and clean up index entries.
    pub fn remove_connection(&self, conn_id: ConnId) -> Vec<RemovedSubscription> {
        let mut removed = Vec::new();
        if let Some((_, conn_subs)) = self.subs.remove(&conn_id) {
            let count = conn_subs.len();
            for (sub_id, (filters, community_id, scope)) in &conn_subs {
                self.remove_from_index(conn_id, sub_id, filters, *community_id, scope);
                removed.push(RemovedSubscription {
                    community_id: *community_id,
                    scope: scope.clone(),
                });
            }
            metrics::gauge!("buzz_subscriptions_active").decrement(count as f64);
        }
        removed
    }

    /// Remove one revoked channel from every matching subscription in a community.
    /// Multi-channel subscriptions are re-indexed with their remaining scope;
    /// subscriptions with no channels left are removed entirely.
    pub fn remove_channel_subscriptions_scoped(
        &self,
        community_id: CommunityId,
        conn_id: ConnId,
        channel_id: Uuid,
    ) -> Vec<ChannelSubscriptionUpdate> {
        let sub_ids: Vec<SubId> = self
            .subs
            .get(&conn_id)
            .map(|conn_subs| {
                conn_subs
                    .iter()
                    .filter_map(|(sub_id, (_, sub_community_id, scope))| {
                        (*sub_community_id == community_id
                            && scope.channel_ids().contains(&channel_id))
                        .then_some(sub_id.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut updates = Vec::with_capacity(sub_ids.len());
        for sub_id in sub_ids {
            let Some(mut conn_subs) = self.subs.get_mut(&conn_id) else {
                break;
            };
            let Some((filters, _, scope)) = conn_subs.get_mut(&sub_id) else {
                continue;
            };
            let filters = filters.clone();
            let SubscriptionScope::Channels(channel_ids) = scope else {
                continue;
            };
            channel_ids.retain(|candidate| *candidate != channel_id);
            let removed = channel_ids.is_empty();
            self.remove_from_index(
                conn_id,
                &sub_id,
                &filters,
                community_id,
                &SubscriptionScope::Channels(vec![channel_id]),
            );
            if removed {
                conn_subs.remove(&sub_id);
                metrics::gauge!("buzz_subscriptions_active").decrement(1.0);
            }
            updates.push(ChannelSubscriptionUpdate { sub_id, removed });
        }
        updates
    }

    /// Test-only convenience wrapper preserving the original single-tenant test API.
    #[cfg(test)]
    pub fn remove_channel_subscriptions(&self, conn_id: ConnId, channel_id: Uuid) -> Vec<SubId> {
        self.remove_channel_subscriptions_scoped(test_community(), conn_id, channel_id)
            .into_iter()
            .filter(|update| update.removed)
            .map(|update| update.sub_id)
            .collect()
    }

    /// Return the distinct connection IDs holding any subscription scoped to
    /// `channel_id` (both kind-filtered and wildcard channel subscriptions) in
    /// one server-resolved community.
    pub fn channel_subscriber_conns_scoped(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Vec<ConnId> {
        let mut conns: HashSet<ConnId> = HashSet::new();
        for entry in self.channel_kind_index.iter() {
            if entry.key().0 == community_id && entry.key().1.channel_id == channel_id {
                conns.extend(entry.value().iter().map(|(conn_id, _)| *conn_id));
            }
        }
        if let Some(entry) = self.channel_wildcard_index.get(&(community_id, channel_id)) {
            conns.extend(entry.value().iter().map(|(conn_id, _)| *conn_id));
        }
        conns.into_iter().collect()
    }

    /// Test-only convenience wrapper preserving the original single-tenant test API.
    #[cfg(test)]
    pub fn channel_subscriber_conns(&self, channel_id: Uuid) -> Vec<ConnId> {
        self.channel_subscriber_conns_scoped(test_community(), channel_id)
    }

    /// Return all (conn_id, sub_id) pairs whose filters match the given event in
    /// one server-resolved community.
    #[tracing::instrument(skip_all)]
    pub fn fan_out_scoped(
        &self,
        community_id: CommunityId,
        event: &StoredEvent,
    ) -> Vec<(ConnId, SubId)> {
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        if let Some(channel_id) = event.channel_id {
            let key = IndexKey {
                channel_id,
                kind: event.event.kind,
            };
            if let Some(candidates) = self
                .channel_kind_index
                .get(&(community_id, key))
                .map(|entry| entry.value().clone())
            {
                for (conn_id, sub_id) in candidates {
                    self.push_match(
                        conn_id,
                        &sub_id,
                        community_id,
                        event,
                        &mut results,
                        &mut seen,
                    );
                }
            }
            // Also check wildcard (channel-only, kindless) index.
            if let Some(wildcards) = self
                .channel_wildcard_index
                .get(&(community_id, channel_id))
                .map(|entry| entry.value().clone())
            {
                for (conn_id, sub_id) in wildcards {
                    self.push_match(
                        conn_id,
                        &sub_id,
                        community_id,
                        event,
                        &mut results,
                        &mut seen,
                    );
                }
            }
        } else {
            // Global event (channel_id = None) — use global indexes for sub-linear fan-out.
            // Channel-scoped subscriptions are never in these indexes, preserving the
            // scoping invariant without an explicit skip check.
            for p in event_p_tag_values(event) {
                let key = GlobalPKindIndexKey {
                    community_id,
                    kind: event.event.kind,
                    p,
                };
                if let Some(candidates) = self
                    .global_p_kind_index
                    .get(&key)
                    .map(|entry| entry.value().clone())
                {
                    for (conn_id, sub_id) in candidates {
                        self.push_match(
                            conn_id,
                            &sub_id,
                            community_id,
                            event,
                            &mut results,
                            &mut seen,
                        );
                    }
                }
            }
            if let Some(candidates) = self
                .global_kind_index
                .get(&(community_id, event.event.kind))
                .map(|entry| entry.value().clone())
            {
                for (conn_id, sub_id) in candidates {
                    self.push_match(
                        conn_id,
                        &sub_id,
                        community_id,
                        event,
                        &mut results,
                        &mut seen,
                    );
                }
            }
            // Also check global wildcard (kindless global subs).
            if let Some(wildcards) = self
                .global_wildcard_index
                .get(&community_id)
                .map(|entry| entry.value().clone())
            {
                for (conn_id, sub_id) in wildcards {
                    self.push_match(
                        conn_id,
                        &sub_id,
                        community_id,
                        event,
                        &mut results,
                        &mut seen,
                    );
                }
            }
        }

        // NOTE: The scoping invariant is symmetric:
        // - Global subscriptions (channel_id = None) do NOT receive channel-scoped events.
        // - Channel-scoped subscriptions do NOT receive global events.
        // This prevents both directions of information leakage: channel content
        // leaking to global subscribers, and global infrastructure events (like
        // membership notifications) leaking to channel subscribers.

        results
    }

    /// Test-only convenience wrapper preserving the original single-tenant test API.
    #[cfg(test)]
    pub fn fan_out(&self, event: &StoredEvent) -> Vec<(ConnId, SubId)> {
        self.fan_out_scoped(test_community(), event)
    }

    /// Return the filters for a specific subscription, or `None` if not found.
    pub fn get_filters(&self, conn_id: ConnId, sub_id: &str) -> Option<Vec<Filter>> {
        self.subs
            .get(&conn_id)
            .and_then(|conn_subs| conn_subs.get(sub_id).map(|(filters, _, _)| filters.clone()))
    }

    /// Return the total number of active subscriptions across all connections.
    pub fn total_subscriptions(&self) -> usize {
        self.subs.iter().map(|e| e.value().len()).sum()
    }

    /// Return the total number of connections with at least one active subscription.
    pub fn total_connections(&self) -> usize {
        self.subs.len()
    }

    /// Snapshot the number of active subscriptions per community.
    ///
    /// Used by the usage poller to emit `buzz_community_subscriptions{community}`.
    /// Snapshotting avoids gauge drift from mismatched inc/dec across communities.
    pub fn per_community_subscriptions(&self) -> std::collections::HashMap<CommunityId, u64> {
        let mut counts: std::collections::HashMap<CommunityId, u64> =
            std::collections::HashMap::new();
        for conn_entry in self.subs.iter() {
            for (_, community_id, _) in conn_entry.value().values() {
                *counts.entry(*community_id).or_default() += 1;
            }
        }
        counts
    }

    fn push_match(
        &self,
        conn_id: ConnId,
        sub_id: &str,
        community_id: CommunityId,
        event: &StoredEvent,
        results: &mut Vec<(ConnId, SubId)>,
        seen: &mut HashSet<(ConnId, SubId)>,
    ) {
        if let Some(conn_subs) = self.subs.get(&conn_id) {
            if let Some((filters, sub_community_id, scope)) = conn_subs.get(sub_id) {
                // Candidate snapshots can become stale while a same-ID replacement
                // moves the subscription. Re-check its authoritative scope before
                // matching so an old index entry cannot deliver across scopes.
                if *sub_community_id == community_id
                    && scope.matches_channel(event.channel_id)
                    && filters_match(filters, event)
                {
                    let entry = (conn_id, sub_id.to_string());
                    if seen.insert(entry.clone()) {
                        results.push(entry);
                    }
                }
            }
        }
    }

    /// Removes a subscription from the channel_kind_index (or channel_wildcard_index) using
    /// targeted O(k) lookup where k = number of kinds in the filters, instead of O(n) full-scan.
    fn remove_from_index(
        &self,
        conn_id: ConnId,
        sub_id: &str,
        filters: &[Filter],
        community_id: CommunityId,
        scope: &SubscriptionScope,
    ) {
        if let SubscriptionScope::Channels(channel_ids) = scope {
            for ch_id in channel_ids {
                let ch_id = *ch_id;
                match extract_kinds_from_filters(filters) {
                    // None = wildcard (at least one filter had no kinds constraint).
                    None => {
                        // Was in wildcard index.
                        if let Some(mut entries) =
                            self.channel_wildcard_index.get_mut(&(community_id, ch_id))
                        {
                            entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                            if entries.is_empty() {
                                drop(entries);
                                self.channel_wildcard_index.remove(&(community_id, ch_id));
                            }
                        }
                    }
                    Some(kinds) if kinds.is_empty() => {
                        // `kinds: []` subscriptions are never indexed (they match nothing),
                        // so there is nothing to remove here.
                    }
                    Some(kinds) => {
                        // Was in kind-specific index.
                        for kind in kinds {
                            let key = IndexKey {
                                channel_id: ch_id,
                                kind,
                            };
                            if let Some(mut entries) = self
                                .channel_kind_index
                                .get_mut(&(community_id, key.clone()))
                            {
                                entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                                if entries.is_empty() {
                                    drop(entries);
                                    self.channel_kind_index.remove(&(community_id, key));
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // Global subscription — remove from the same global index chosen at registration.
            if let Some(keys) = extract_global_p_kind_index_keys(community_id, filters) {
                for key in keys {
                    if let Some(mut entries) = self.global_p_kind_index.get_mut(&key) {
                        entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                        if entries.is_empty() {
                            drop(entries);
                            self.global_p_kind_index.remove(&key);
                        }
                    }
                }
            } else {
                match extract_kinds_from_filters(filters) {
                    None => {
                        if let Some(mut entries) = self.global_wildcard_index.get_mut(&community_id)
                        {
                            entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                            if entries.is_empty() {
                                drop(entries);
                                self.global_wildcard_index.remove(&community_id);
                            }
                        }
                    }
                    Some(kinds) if kinds.is_empty() => {}
                    Some(kinds) => {
                        for kind in kinds {
                            if let Some(mut entries) =
                                self.global_kind_index.get_mut(&(community_id, kind))
                            {
                                entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                                if entries.is_empty() {
                                    drop(entries);
                                    self.global_kind_index.remove(&(community_id, kind));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn p_tag() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::P)
}

fn extract_global_p_kind_index_keys(
    community_id: CommunityId,
    filters: &[Filter],
) -> Option<Vec<GlobalPKindIndexKey>> {
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    let p_tag = p_tag();

    for filter in filters {
        let kinds = filter.kinds.as_ref()?;
        if kinds.is_empty() {
            continue;
        }

        let p_values = filter.generic_tags.get(&p_tag)?;
        if p_values.is_empty() {
            return None;
        }

        for kind in kinds {
            for p in p_values {
                let key = GlobalPKindIndexKey {
                    community_id,
                    kind: *kind,
                    p: p.clone(),
                };
                if seen.insert(key.clone()) {
                    keys.push(key);
                }
            }
        }
    }

    Some(keys)
}

fn event_p_tag_values(event: &StoredEvent) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for tag in event.event.tags.iter() {
        if tag.kind().to_string() != "p" {
            continue;
        }
        if let Some(value) = tag.content() {
            let value = value.to_string();
            if seen.insert(value.clone()) {
                values.push(value);
            }
        }
    }
    values
}

/// Returns the union of all `kinds` across filters, or `None` if any filter
/// lacks a `kinds` array (meaning that filter matches all kinds — wildcard).
///
/// NIP-01 OR semantics: a subscription with multiple filters is satisfied when
/// *any* filter matches. If one filter has no `kinds` constraint it matches
/// every kind, making the whole subscription a wildcard regardless of the other
/// filters.
fn extract_kinds_from_filters(filters: &[Filter]) -> Option<Vec<Kind>> {
    let mut seen = std::collections::HashSet::new();
    let mut kinds = Vec::new();
    for f in filters {
        match &f.kinds {
            Some(filter_kinds) => {
                for k in filter_kinds {
                    if seen.insert(*k) {
                        kinds.push(*k);
                    }
                }
            }
            None => {
                // At least one filter has no kind constraint — the whole
                // subscription is a wildcard.
                return None;
            }
        }
    }
    Some(kinds)
}

#[cfg(test)]
fn test_community() -> CommunityId {
    CommunityId::from_uuid(Uuid::nil())
}
