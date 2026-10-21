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

/// Server-resolved live-routing scope: the community-global topic, a set of
/// authorized channels, or both.
///
/// Each filter of a live subscription has its own scope, and a live event
/// reaches the subscription only through a filter that both matches the event
/// and has the event's channel (or the global topic, for a channel-less event)
/// in its scope. Filters are ORed, so one request-wide scope would let a
/// filter receive a channel that only a sibling filter may read.
///
/// The same type describes a subscription's retained pubsub topics: the union
/// of its filters' scopes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubscriptionScope {
    global: bool,
    channels: Vec<Uuid>,
}

impl SubscriptionScope {
    /// Community-global events only.
    pub fn global() -> Self {
        Self {
            global: true,
            channels: Vec::new(),
        }
    }

    /// Events from any of these authorized channels. Duplicates are dropped.
    pub fn channels(channel_ids: impl IntoIterator<Item = Uuid>) -> Self {
        let mut scope = Self::default();
        for channel_id in channel_ids {
            scope.add_channel(channel_id);
        }
        scope
    }

    /// This scope plus community-global events.
    pub fn with_global(mut self) -> Self {
        self.global = true;
        self
    }

    fn add_channel(&mut self, channel_id: Uuid) {
        // Sorted, so equal sets compare equal whatever the request order.
        if let Err(at) = self.channels.binary_search(&channel_id) {
            self.channels.insert(at, channel_id);
        }
    }

    /// Drop `channel_id`. Returns whether the scope held it.
    fn remove_channel(&mut self, channel_id: Uuid) -> bool {
        let before = self.channels.len();
        self.channels.retain(|candidate| *candidate != channel_id);
        self.channels.len() != before
    }

    fn matches_channel(&self, channel_id: Option<Uuid>) -> bool {
        match channel_id {
            None => self.global,
            Some(channel_id) => self.channels.contains(&channel_id),
        }
    }

    fn is_empty(&self) -> bool {
        !self.global && self.channels.is_empty()
    }

    /// The union of `scopes`: every topic any of them retains, once.
    pub fn union<'a>(scopes: impl IntoIterator<Item = &'a SubscriptionScope>) -> Self {
        let mut union = Self::default();
        for scope in scopes {
            union.global |= scope.global;
            for &channel_id in &scope.channels {
                union.add_channel(channel_id);
            }
        }
        union
    }

    /// Return the channels retained by this routing scope.
    pub fn channel_ids(&self) -> &[Uuid] {
        &self.channels
    }

    /// Whether this routing scope retains the community-global topic.
    pub fn is_global(&self) -> bool {
        self.global
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

/// One fan-out index entry a subscription occupies. All slots of one
/// subscription share its community.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum IndexSlot {
    ChannelKind(IndexKey),
    ChannelWildcard(Uuid),
    GlobalPKind(GlobalPKindIndexKey),
    GlobalKind(Kind),
    GlobalWildcard,
}

/// A registered live subscription: each filter with its server-resolved scope.
#[derive(Debug, Clone)]
struct LiveSubscription {
    community_id: CommunityId,
    filters: Vec<(Filter, SubscriptionScope)>,
}

impl LiveSubscription {
    /// The pubsub topics this subscription retains.
    fn topics(&self) -> SubscriptionScope {
        SubscriptionScope::union(self.filters.iter().map(|(_, scope)| scope))
    }

    /// Whether some filter both matches `event` and has its channel in scope.
    fn matches(&self, event: &StoredEvent) -> bool {
        self.filters.iter().any(|(filter, scope)| {
            scope.matches_channel(event.channel_id)
                && filters_match(std::slice::from_ref(filter), event)
        })
    }

    /// Every index entry that can surface an event this subscription matches.
    /// Each filter is indexed under its own scope.
    fn index_slots(&self) -> HashSet<IndexSlot> {
        let mut slots = HashSet::new();
        for (filter, scope) in &self.filters {
            let kinds = filter.kinds.as_ref();
            for &channel_id in &scope.channels {
                match kinds {
                    // No `kinds` constraint: every kind in this channel.
                    None => {
                        slots.insert(IndexSlot::ChannelWildcard(channel_id));
                    }
                    // Per NIP-01, `kinds: []` matches nothing, so an empty
                    // list adds no slot.
                    Some(kinds) => {
                        slots.extend(
                            kinds
                                .iter()
                                .map(|&kind| IndexSlot::ChannelKind(IndexKey { channel_id, kind })),
                        );
                    }
                }
            }
            if !scope.global {
                continue;
            }
            // A fully p-constrained filter uses the narrower (kind, #p) index;
            // a broader one stays on the generic kind/wildcard indexes.
            if let Some(keys) =
                extract_global_p_kind_index_keys(self.community_id, std::slice::from_ref(filter))
            {
                slots.extend(keys.into_iter().map(IndexSlot::GlobalPKind));
            } else {
                match kinds {
                    None => {
                        slots.insert(IndexSlot::GlobalWildcard);
                    }
                    Some(kinds) => {
                        slots.extend(kinds.iter().map(|&kind| IndexSlot::GlobalKind(kind)));
                    }
                }
            }
        }
        slots
    }
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
    /// Whether no topics remain and the subscription was removed.
    pub removed: bool,
}

/// Thread-safe registry of active subscriptions with targeted in-memory fan-out indexes.
#[derive(Debug, Default)]
pub struct SubscriptionRegistry {
    /// Maps conn_id → sub_id → live subscription (filters with their scopes).
    /// Storing scopes alongside filters enables O(1) targeted index removal
    /// and gives lifecycle code the exact Redis topics to release.
    subs: DashMap<ConnId, HashMap<SubId, LiveSubscription>>,
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
    /// to the server-resolved community that owns the connection. Every filter
    /// gets the same scope: `channel_id`, or community-global when `None`.
    pub fn register_scoped(
        &self,
        community_id: CommunityId,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<Filter>,
        channel_id: Option<Uuid>,
    ) -> Option<RemovedSubscription> {
        let scope = channel_id
            .map(|channel_id| SubscriptionScope::channels([channel_id]))
            .unwrap_or_else(SubscriptionScope::global);
        self.register_same_scope(community_id, conn_id, sub_id, filters, scope)
    }

    /// Register a subscription whose every filter is scoped to all of
    /// `channel_ids`.
    pub fn register_channels_scoped(
        &self,
        community_id: CommunityId,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<Filter>,
        channel_ids: Vec<Uuid>,
    ) -> Option<RemovedSubscription> {
        self.register_same_scope(
            community_id,
            conn_id,
            sub_id,
            filters,
            SubscriptionScope::channels(channel_ids),
        )
    }

    fn register_same_scope(
        &self,
        community_id: CommunityId,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<Filter>,
        scope: SubscriptionScope,
    ) -> Option<RemovedSubscription> {
        let filters = filters
            .into_iter()
            .map(|filter| (filter, scope.clone()))
            .collect();
        self.register_filters_scoped(community_id, conn_id, sub_id, filters)
    }

    /// Register a subscription in which each filter has its own
    /// server-resolved scope, replacing any existing subscription with the
    /// same sub_id (NIP-01). Returns the replaced subscription, whose topics
    /// the caller releases.
    pub fn register_filters_scoped(
        &self,
        community_id: CommunityId,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<(Filter, SubscriptionScope)>,
    ) -> Option<RemovedSubscription> {
        let removed = self.remove_subscription(conn_id, &sub_id);

        let subscription = LiveSubscription {
            community_id,
            filters,
        };
        let slots = subscription.index_slots();
        self.subs
            .entry(conn_id)
            .or_default()
            .insert(sub_id.clone(), subscription);
        metrics::gauge!("buzz_subscriptions_active").increment(1.0);

        for slot in slots {
            self.add_to_slot(community_id, slot, conn_id, &sub_id);
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
        let subscription = conn_subs.remove(sub_id)?;

        after_remove();
        self.remove_from_index(conn_id, sub_id, &subscription);
        drop(conn_subs);

        metrics::gauge!("buzz_subscriptions_active").decrement(1.0);
        Some(RemovedSubscription {
            community_id: subscription.community_id,
            scope: subscription.topics(),
        })
    }

    /// Remove all subscriptions for a connection and clean up index entries.
    pub fn remove_connection(&self, conn_id: ConnId) -> Vec<RemovedSubscription> {
        let mut removed = Vec::new();
        if let Some((_, conn_subs)) = self.subs.remove(&conn_id) {
            let count = conn_subs.len();
            for (sub_id, subscription) in &conn_subs {
                self.remove_from_index(conn_id, sub_id, subscription);
                removed.push(RemovedSubscription {
                    community_id: subscription.community_id,
                    scope: subscription.topics(),
                });
            }
            metrics::gauge!("buzz_subscriptions_active").decrement(count as f64);
        }
        removed
    }

    /// Remove one revoked channel from every filter of every matching
    /// subscription in a community. Each affected subscription is reported
    /// once, because it retained the channel's topic once. A subscription keeps
    /// its other channels and any global scope; one with no topics left is
    /// removed entirely.
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
                    .filter_map(|(sub_id, subscription)| {
                        (subscription.community_id == community_id
                            && subscription
                                .filters
                                .iter()
                                .any(|(_, scope)| scope.channel_ids().contains(&channel_id)))
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
            let Some(subscription) = conn_subs.get_mut(&sub_id) else {
                continue;
            };
            let before = subscription.index_slots();
            let mut held = false;
            for (_, scope) in &mut subscription.filters {
                held |= scope.remove_channel(channel_id);
            }
            if !held {
                // A same-ID replacement took the ID after the snapshot.
                continue;
            }
            let removed = subscription.topics().is_empty();
            // Drop only the slots no remaining filter needs, so the
            // subscription stays reachable for its other topics throughout.
            let after = subscription.index_slots();
            for slot in before.difference(&after) {
                self.remove_from_slot(community_id, slot, conn_id, &sub_id);
            }
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
            // Only filters with a global scope are in these indexes, and
            // `push_match` re-checks the matching filter's scope.
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

        // NOTE: The scoping invariant is per filter and symmetric:
        // - A filter without a global scope does NOT receive global events.
        // - A filter receives a channel event only if that channel is in its
        //   own scope, never through a sibling filter's scope.
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
        self.subs.get(&conn_id).and_then(|conn_subs| {
            conn_subs.get(sub_id).map(|subscription| {
                subscription
                    .filters
                    .iter()
                    .map(|(filter, _)| filter.clone())
                    .collect()
            })
        })
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
            for subscription in conn_entry.value().values() {
                *counts.entry(subscription.community_id).or_default() += 1;
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
            if let Some(subscription) = conn_subs.get(sub_id) {
                // Candidate snapshots can become stale while a same-ID replacement
                // moves the subscription. Re-check its authoritative per-filter
                // scope before matching so an old index entry cannot deliver
                // across scopes.
                if subscription.community_id == community_id && subscription.matches(event) {
                    let entry = (conn_id, sub_id.to_string());
                    if seen.insert(entry.clone()) {
                        results.push(entry);
                    }
                }
            }
        }
    }

    /// Removes a subscription from every index slot it occupies, using
    /// targeted lookups instead of a full scan.
    fn remove_from_index(&self, conn_id: ConnId, sub_id: &str, subscription: &LiveSubscription) {
        for slot in subscription.index_slots() {
            self.remove_from_slot(subscription.community_id, &slot, conn_id, sub_id);
        }
    }

    fn add_to_slot(
        &self,
        community_id: CommunityId,
        slot: IndexSlot,
        conn_id: ConnId,
        sub_id: &str,
    ) {
        let entry = (conn_id, sub_id.to_string());
        match slot {
            IndexSlot::ChannelKind(key) => self
                .channel_kind_index
                .entry((community_id, key))
                .or_default()
                .push(entry),
            IndexSlot::ChannelWildcard(channel_id) => self
                .channel_wildcard_index
                .entry((community_id, channel_id))
                .or_default()
                .push(entry),
            IndexSlot::GlobalPKind(key) => {
                self.global_p_kind_index.entry(key).or_default().push(entry)
            }
            IndexSlot::GlobalKind(kind) => self
                .global_kind_index
                .entry((community_id, kind))
                .or_default()
                .push(entry),
            IndexSlot::GlobalWildcard => self
                .global_wildcard_index
                .entry(community_id)
                .or_default()
                .push(entry),
        }
    }

    fn remove_from_slot(
        &self,
        community_id: CommunityId,
        slot: &IndexSlot,
        conn_id: ConnId,
        sub_id: &str,
    ) {
        fn remove_entry<K: Eq + std::hash::Hash>(
            index: &DashMap<K, Vec<(ConnId, SubId)>>,
            key: K,
            conn_id: ConnId,
            sub_id: &str,
        ) {
            if let Some(mut entries) = index.get_mut(&key) {
                entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                if entries.is_empty() {
                    drop(entries);
                    index.remove_if(&key, |_, entries| entries.is_empty());
                }
            }
        }
        match slot {
            IndexSlot::ChannelKind(key) => remove_entry(
                &self.channel_kind_index,
                (community_id, key.clone()),
                conn_id,
                sub_id,
            ),
            IndexSlot::ChannelWildcard(channel_id) => remove_entry(
                &self.channel_wildcard_index,
                (community_id, *channel_id),
                conn_id,
                sub_id,
            ),
            IndexSlot::GlobalPKind(key) => {
                remove_entry(&self.global_p_kind_index, key.clone(), conn_id, sub_id)
            }
            IndexSlot::GlobalKind(kind) => remove_entry(
                &self.global_kind_index,
                (community_id, *kind),
                conn_id,
                sub_id,
            ),
            IndexSlot::GlobalWildcard => {
                remove_entry(&self.global_wildcard_index, community_id, conn_id, sub_id)
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

#[cfg(test)]
fn test_community() -> CommunityId {
    CommunityId::from_uuid(Uuid::nil())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::StoredEvent;
    use chrono::Utc;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn make_stored_event(kind: Kind, channel_id: Option<Uuid>) -> StoredEvent {
        let keys = Keys::generate();
        let event = EventBuilder::new(kind, "test")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        StoredEvent::with_received_at(event, Utc::now(), channel_id, true)
    }

    fn make_stored_event_with_p(kind: Kind, p: &str, channel_id: Option<Uuid>) -> StoredEvent {
        let keys = Keys::generate();
        let event = EventBuilder::new(kind, "test")
            .tags([Tag::parse(["p", p]).expect("valid p tag")])
            .sign_with_keys(&keys)
            .expect("sign");
        StoredEvent::with_received_at(event, Utc::now(), channel_id, true)
    }

    #[test]
    fn test_subscription_registry_register_and_fan_out() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let sub_id = "sub1".to_string();

        let filters = vec![Filter::new().kind(Kind::TextNote)];
        registry.register(conn_id, sub_id.clone(), filters, Some(channel_id));

        let event = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, conn_id);
        assert_eq!(matches[0].1, sub_id);
    }

    #[test]
    fn multi_channel_subscription_fans_out_only_requested_channels() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_a = Uuid::new_v4();
        let channel_b = Uuid::new_v4();
        let unrelated = Uuid::new_v4();
        let sub_id = "multi-channel".to_string();
        let filters = vec![Filter::new()
            .kind(Kind::TextNote)
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::H),
                channel_a.to_string(),
            )
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::H),
                channel_b.to_string(),
            )];

        registry.register_channels_scoped(
            test_community(),
            conn_id,
            sub_id.clone(),
            filters,
            vec![channel_a, channel_b],
        );

        assert_eq!(
            registry.fan_out(&make_stored_event(Kind::TextNote, Some(channel_a))),
            vec![(conn_id, sub_id.clone())]
        );
        assert_eq!(
            registry.fan_out(&make_stored_event(Kind::TextNote, Some(channel_b))),
            vec![(conn_id, sub_id)]
        );
        assert!(registry
            .fan_out(&make_stored_event(Kind::TextNote, Some(unrelated)))
            .is_empty());
        assert!(registry
            .fan_out(&make_stored_event(Kind::TextNote, None))
            .is_empty());
    }

    #[test]
    fn test_subscription_registry_remove() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let sub_id = "sub1".to_string();

        let filters = vec![Filter::new().kind(Kind::TextNote)];
        registry.register(conn_id, sub_id.clone(), filters, Some(channel_id));

        registry.remove_subscription(conn_id, &sub_id);

        let event = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&event);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_subscription_removal_cannot_delete_replacement_index() {
        let registry = Arc::new(SubscriptionRegistry::new());
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let sub_id = "same-id".to_string();
        let filters = vec![Filter::new().kind(Kind::TextNote)];
        registry.register(conn_id, sub_id.clone(), filters.clone(), Some(channel_id));

        let (removed_tx, removed_rx) = std::sync::mpsc::sync_channel(0);
        let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(0);
        let remove_registry = Arc::clone(&registry);
        let remove_sub_id = sub_id.clone();
        let remove = std::thread::spawn(move || {
            remove_registry.remove_subscription_inner(conn_id, &remove_sub_id, || {
                removed_tx.send(()).expect("signal authoritative removal");
                resume_rx.recv().expect("resume index cleanup");
            })
        });

        removed_rx.recv().expect("old subscription removed");

        let (registered_tx, registered_rx) = std::sync::mpsc::sync_channel(0);
        let register_registry = Arc::clone(&registry);
        let register_sub_id = sub_id.clone();
        let register = std::thread::spawn(move || {
            register_registry.register(conn_id, register_sub_id, filters, Some(channel_id));
            registered_tx
                .send(())
                .expect("signal replacement registration");
        });

        let replacement_finished_early = registered_rx
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        resume_tx.send(()).expect("resume old cleanup");
        remove.join().expect("removal thread completes");
        if !replacement_finished_early {
            registered_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("replacement registration completes");
        }
        register.join().expect("registration thread completes");
        assert!(
            !replacement_finished_early,
            "replacement must wait until old index cleanup is complete"
        );

        let event = make_stored_event(Kind::TextNote, Some(channel_id));
        assert_eq!(
            registry.fan_out(&event),
            vec![(conn_id, sub_id)],
            "replacement must remain reachable through its index"
        );
    }

    #[test]
    fn test_stale_candidate_snapshot_does_not_cross_subscription_scope() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_a = Uuid::new_v4();
        let channel_b = Uuid::new_v4();
        let sub_id = "same-id".to_string();
        let filters = vec![Filter::new().kind(Kind::TextNote)];
        registry.register(conn_id, sub_id.clone(), filters.clone(), Some(channel_a));

        // Reproduce fan-out's unlocked candidate snapshot, then move the same
        // subscription ID before the authoritative subscription lookup.
        let key = IndexKey {
            channel_id: channel_a,
            kind: Kind::TextNote,
        };
        let candidates = registry
            .channel_kind_index
            .get(&(test_community(), key))
            .expect("channel A candidate exists")
            .value()
            .clone();
        registry.register(conn_id, sub_id, filters, Some(channel_b));

        let event = make_stored_event(Kind::TextNote, Some(channel_a));
        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for (candidate_conn_id, candidate_sub_id) in candidates {
            registry.push_match(
                candidate_conn_id,
                &candidate_sub_id,
                test_community(),
                &event,
                &mut results,
                &mut seen,
            );
        }

        assert!(
            results.is_empty(),
            "replacement on channel B received channel A event through stale snapshot"
        );
    }

    #[test]
    fn test_fan_out_concurrent_with_subscription_replacement_completes() {
        let registry = Arc::new(SubscriptionRegistry::new());
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let sub_id = "sub1".to_string();
        let filters = vec![Filter::new().kind(Kind::TextNote)];
        registry.register(conn_id, sub_id.clone(), filters.clone(), Some(channel_id));
        let event = Arc::new(make_stored_event(Kind::TextNote, Some(channel_id)));
        let deadline = Instant::now() + Duration::from_secs(2);

        let fan_out_registry = Arc::clone(&registry);
        let fan_out_event = Arc::clone(&event);
        let fan_out = std::thread::spawn(move || {
            while Instant::now() < deadline {
                let _ = fan_out_registry.fan_out(&fan_out_event);
            }
        });

        let replace_registry = Arc::clone(&registry);
        let replace = std::thread::spawn(move || {
            while Instant::now() < deadline {
                replace_registry.register(
                    conn_id,
                    sub_id.clone(),
                    filters.clone(),
                    Some(channel_id),
                );
            }
        });

        fan_out.join().expect("fan-out thread completes");
        replace.join().expect("replacement thread completes");
    }

    #[test]
    fn test_subscription_registry_remove_connection() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        registry.register(
            conn_id,
            "sub1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );
        registry.register(
            conn_id,
            "sub2".to_string(),
            vec![Filter::new().kind(Kind::Metadata)],
            Some(channel_id),
        );

        assert_eq!(registry.total_subscriptions(), 2);

        registry.remove_connection(conn_id);

        assert_eq!(registry.total_subscriptions(), 0);
        assert_eq!(registry.total_connections(), 0);
    }

    #[test]
    fn test_subscription_registry_channel_kind_index() {
        let registry = SubscriptionRegistry::new();
        let channel_id = Uuid::new_v4();

        let mut conn_ids = Vec::new();
        for i in 0..3 {
            let conn_id = Uuid::new_v4();
            conn_ids.push(conn_id);
            registry.register(
                conn_id,
                format!("sub{i}"),
                vec![Filter::new().kind(Kind::TextNote)],
                Some(channel_id),
            );
        }

        let event = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&event);
        assert_eq!(matches.len(), 3);

        let event_meta = make_stored_event(Kind::Metadata, Some(channel_id));
        let matches_meta = registry.fan_out(&event_meta);
        assert!(matches_meta.is_empty());
    }

    #[test]
    fn test_subscription_registry_replace_existing() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        registry.register(
            conn_id,
            "sub1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );

        registry.register(
            conn_id,
            "sub1".to_string(),
            vec![Filter::new().kind(Kind::Metadata)],
            Some(channel_id),
        );

        let event1 = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches1 = registry.fan_out(&event1);
        assert!(matches1.is_empty());

        let event0 = make_stored_event(Kind::Metadata, Some(channel_id));
        let matches0 = registry.fan_out(&event0);
        assert_eq!(matches0.len(), 1);
    }

    #[test]
    fn test_subscription_registry_no_channel_slow_path() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();

        registry.register(
            conn_id,
            "sub1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            None, // no channel
        );

        let event = make_stored_event(Kind::TextNote, None);
        let matches = registry.fan_out(&event);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_subscription_registry_get_filters() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let filters = vec![Filter::new().kind(Kind::TextNote)];

        registry.register(conn_id, "sub1".to_string(), filters.clone(), None);

        let retrieved = registry.get_filters(conn_id, "sub1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len(), 1);

        let missing = registry.get_filters(conn_id, "nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_remove_from_index_targeted_no_full_scan() {
        // Verify that removing a subscription only touches the relevant index keys.
        // We register subs for two different channels and two different kinds,
        // then remove one and confirm the other channel's index is untouched.
        let registry = SubscriptionRegistry::new();
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let channel_x = Uuid::new_v4();
        let channel_y = Uuid::new_v4();

        registry.register(
            conn_a,
            "sub_a".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_x),
        );
        registry.register(
            conn_b,
            "sub_b".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_y),
        );

        registry.remove_subscription(conn_a, "sub_a");

        let key_x = IndexKey {
            channel_id: channel_x,
            kind: Kind::TextNote,
        };
        assert!(registry
            .channel_kind_index
            .get(&(test_community(), key_x))
            .is_none());

        let key_y = IndexKey {
            channel_id: channel_y,
            kind: Kind::TextNote,
        };
        let entries = registry
            .channel_kind_index
            .get(&(test_community(), key_y))
            .expect("channel_y index intact");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, conn_b);
    }

    #[test]
    fn test_kindless_channel_subscription_receives_all_kinds() {
        // A subscription with channel_id but NO kind filter should receive events
        // of any kind posted to that channel.
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let sub_id = "wildcard_sub".to_string();

        let filters = vec![Filter::new()]; // kindless — no .kind() constraint
        registry.register(conn_id, sub_id.clone(), filters, Some(channel_id));

        let event_text = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&event_text);
        assert_eq!(matches.len(), 1, "kindless sub should receive TextNote");
        assert_eq!(matches[0].0, conn_id);
        assert_eq!(matches[0].1, sub_id);

        let event_meta = make_stored_event(Kind::Metadata, Some(channel_id));
        let matches = registry.fan_out(&event_meta);
        assert_eq!(matches.len(), 1, "kindless sub should receive Metadata");

        let event_custom = make_stored_event(Kind::Custom(9999), Some(channel_id));
        let matches = registry.fan_out(&event_custom);
        assert_eq!(matches.len(), 1, "kindless sub should receive custom kind");

        let other_channel = Uuid::new_v4();
        let event_other = make_stored_event(Kind::TextNote, Some(other_channel));
        let matches = registry.fan_out(&event_other);
        assert!(
            matches.is_empty(),
            "kindless sub should not receive events from other channels"
        );
    }

    #[test]
    fn test_kindless_subscription_remove_cleans_wildcard_index() {
        // Verify that removing a kindless subscription cleans up the wildcard index.
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        let filters = vec![Filter::new()]; // kindless
        registry.register(conn_id, "sub1".to_string(), filters, Some(channel_id));

        assert!(registry
            .channel_wildcard_index
            .get(&(test_community(), channel_id))
            .is_some());

        registry.remove_subscription(conn_id, "sub1");

        assert!(registry
            .channel_wildcard_index
            .get(&(test_community(), channel_id))
            .is_none());

        let event = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&event);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_kindless_and_kinded_subs_coexist() {
        // Both a kindless sub and a kind-specific sub in the same channel should
        // both receive events of the matching kind.
        let registry = SubscriptionRegistry::new();
        let conn_wildcard = Uuid::new_v4();
        let conn_kinded = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        registry.register(
            conn_wildcard,
            "sub_wildcard".to_string(),
            vec![Filter::new()],
            Some(channel_id),
        );

        registry.register(
            conn_kinded,
            "sub_kinded".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );

        let event_text = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&event_text);
        assert_eq!(
            matches.len(),
            2,
            "both wildcard and kinded sub should match TextNote"
        );

        let event_meta = make_stored_event(Kind::Metadata, Some(channel_id));
        let matches = registry.fan_out(&event_meta);
        assert_eq!(matches.len(), 1, "only wildcard sub should match Metadata");
        assert_eq!(matches[0].0, conn_wildcard);
    }

    #[test]
    fn test_kindless_subscription_replace() {
        // Replacing a kindless sub with a kinded sub should move it from wildcard
        // index to kind-specific index, and vice versa.
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        registry.register(
            conn_id,
            "sub1".to_string(),
            vec![Filter::new()],
            Some(channel_id),
        );
        assert!(registry
            .channel_wildcard_index
            .get(&(test_community(), channel_id))
            .is_some());

        registry.register(
            conn_id,
            "sub1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );

        assert!(registry
            .channel_wildcard_index
            .get(&(test_community(), channel_id))
            .is_none());

        let key = IndexKey {
            channel_id,
            kind: Kind::TextNote,
        };
        assert!(registry
            .channel_kind_index
            .get(&(test_community(), key))
            .is_some());

        let event_meta = make_stored_event(Kind::Metadata, Some(channel_id));
        let matches = registry.fan_out(&event_meta);
        assert!(matches.is_empty());

        let event_text = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&event_text);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_empty_kinds_array_matches_nothing() {
        // NIP-01: `kinds: []` means "match no kinds". A subscription with an
        // explicit empty kinds list should never receive any events — it should
        // NOT be treated as a wildcard (match-all).
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        let filter_empty_kinds = Filter::new().kinds(vec![] as Vec<Kind>);
        registry.register(
            conn_id,
            "sub_empty_kinds".to_string(),
            vec![filter_empty_kinds],
            Some(channel_id),
        );

        assert!(
            registry
                .channel_wildcard_index
                .get(&(test_community(), channel_id))
                .is_none(),
            "kinds:[] sub must NOT be in the wildcard index"
        );

        let key = IndexKey {
            channel_id,
            kind: Kind::TextNote,
        };
        assert!(
            registry
                .channel_kind_index
                .get(&(test_community(), key))
                .is_none(),
            "kinds:[] sub must NOT be in the kind-specific index"
        );

        let event = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&event);
        assert!(
            matches.is_empty(),
            "kinds:[] sub must not receive any events (got {:?})",
            matches
        );

        let event_meta = make_stored_event(Kind::Metadata, Some(channel_id));
        let matches = registry.fan_out(&event_meta);
        assert!(
            matches.is_empty(),
            "kinds:[] sub must not receive Metadata events"
        );
    }

    #[test]
    fn test_global_sub_does_not_receive_channel_events() {
        // Security regression test: a global subscription (channel_id = None) must
        // NOT receive events that are scoped to a channel. Doing so would bypass the
        // channel membership check and leak private channel content.
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        registry.register(
            conn_id,
            "global_sub".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            None, // global — no channel scope
        );

        let channel_event = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&channel_event);
        assert!(
            matches.is_empty(),
            "global sub must not receive channel-scoped events (got {:?})",
            matches
        );

        let global_event = make_stored_event(Kind::TextNote, None);
        let matches = registry.fan_out(&global_event);
        assert_eq!(
            matches.len(),
            1,
            "global sub should still receive non-channel events"
        );
        assert_eq!(matches[0].0, conn_id);
    }

    #[test]
    fn test_empty_kinds_array_remove_is_noop() {
        // Removing a kinds:[] subscription should not panic or corrupt the index.
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        let filter_empty_kinds = Filter::new().kinds(vec![] as Vec<Kind>);
        registry.register(
            conn_id,
            "sub_empty".to_string(),
            vec![filter_empty_kinds],
            Some(channel_id),
        );

        registry.remove_subscription(conn_id, "sub_empty");

        assert!(registry
            .channel_wildcard_index
            .get(&(test_community(), channel_id))
            .is_none());
        let key = IndexKey {
            channel_id,
            kind: Kind::TextNote,
        };
        assert!(registry
            .channel_kind_index
            .get(&(test_community(), key))
            .is_none());
    }

    #[test]
    fn test_remove_channel_subscriptions_only_evicts_target_channel() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let channel_a = Uuid::new_v4();
        let channel_b = Uuid::new_v4();

        registry.register(
            conn_id,
            "sub-a".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_a),
        );
        registry.register(
            conn_id,
            "sub-b".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_b),
        );

        let removed = registry.remove_channel_subscriptions(conn_id, channel_a);
        assert_eq!(removed, vec!["sub-a".to_string()]);

        let event_a = make_stored_event(Kind::TextNote, Some(channel_a));
        assert!(registry.fan_out(&event_a).is_empty());

        let event_b = make_stored_event(Kind::TextNote, Some(channel_b));
        let matches_b = registry.fan_out(&event_b);
        assert_eq!(matches_b.len(), 1);
        assert_eq!(matches_b[0].1, "sub-b");
    }

    #[test]
    fn test_channel_subscriber_conns_dedupes_and_scopes_to_channel() {
        let registry = SubscriptionRegistry::new();
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let conn_other = Uuid::new_v4();
        let channel = Uuid::new_v4();
        let channel_other = Uuid::new_v4();

        // conn_a: a kinded + a wildcard sub on the channel — must dedupe to one entry.
        registry.register(
            conn_a,
            "kinded".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel),
        );
        registry.register(
            conn_a,
            "wildcard".to_string(),
            vec![Filter::new()],
            Some(channel),
        );
        // conn_b: kinded sub on the channel.
        registry.register(
            conn_b,
            "b".to_string(),
            vec![Filter::new().kind(Kind::Metadata)],
            Some(channel),
        );
        // conn_other: subscribed to a different channel — must be excluded.
        registry.register(
            conn_other,
            "other".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_other),
        );

        let mut conns = registry.channel_subscriber_conns(channel);
        conns.sort();
        let mut expected = vec![conn_a, conn_b];
        expected.sort();
        assert_eq!(conns, expected);
    }

    #[test]
    fn test_evict_all_subscribers_leaves_no_channel_subscriptions() {
        // Models the reaper's evict_all_channel_subscriptions: remove every
        // subscriber conn of a channel, leaving the channel with zero live subs
        // while an unrelated channel is untouched. This is the registry behavior
        // that lets an auto-archived channel stop fanning out without a storm.
        let registry = SubscriptionRegistry::new();
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let reaped = Uuid::new_v4();
        let survivor = Uuid::new_v4();

        registry.register(
            conn_a,
            "a".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(reaped),
        );
        registry.register(
            conn_b,
            "b".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(reaped),
        );
        registry.register(
            conn_a,
            "keep".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(survivor),
        );

        for conn_id in registry.channel_subscriber_conns(reaped) {
            registry.remove_channel_subscriptions(conn_id, reaped);
        }

        assert!(registry.channel_subscriber_conns(reaped).is_empty());
        assert!(registry
            .fan_out(&make_stored_event(Kind::TextNote, Some(reaped)))
            .is_empty());
        // Unrelated channel still fans out.
        let survivor_matches = registry.fan_out(&make_stored_event(Kind::TextNote, Some(survivor)));
        assert_eq!(survivor_matches.len(), 1);
        assert_eq!(survivor_matches[0].1, "keep");
    }

    #[test]
    fn test_global_kind_index_fan_out() {
        // Global subscriptions with explicit kinds should use the global_kind_index
        // for sub-linear fan-out instead of scanning all subs.
        let registry = SubscriptionRegistry::new();
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();

        registry.register(
            conn_a,
            "global_text".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            None,
        );
        registry.register(
            conn_b,
            "global_meta".to_string(),
            vec![Filter::new().kind(Kind::Metadata)],
            None,
        );

        let event_text = make_stored_event(Kind::TextNote, None);
        let matches = registry.fan_out(&event_text);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, conn_a);

        let event_meta = make_stored_event(Kind::Metadata, None);
        let matches = registry.fan_out(&event_meta);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, conn_b);

        // Unrelated kind matches nobody.
        let event_custom = make_stored_event(Kind::Custom(9999), None);
        assert!(registry.fan_out(&event_custom).is_empty());
    }

    #[test]
    fn test_global_p_kind_index_fan_out_targets_matching_p() {
        let registry = SubscriptionRegistry::new();
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let kind = Kind::Custom(buzz_core::kind::KIND_AGENT_OBSERVER_FRAME as u16);
        let p_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let p_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        registry.register(
            conn_a,
            "observer_a".to_string(),
            vec![Filter::new().kind(kind).custom_tags(p_tag(), [p_a])],
            None,
        );
        registry.register(
            conn_b,
            "observer_b".to_string(),
            vec![Filter::new().kind(kind).custom_tags(p_tag(), [p_b])],
            None,
        );

        assert!(
            registry
                .global_kind_index
                .get(&(test_community(), kind))
                .is_none(),
            "fully p-constrained global subscriptions should use the p-kind index"
        );

        let event = make_stored_event_with_p(kind, p_a, None);
        let matches = registry.fan_out(&event);
        assert_eq!(matches, vec![(conn_a, "observer_a".to_string())]);
    }

    #[test]
    fn test_global_p_kind_index_removal_cleanup() {
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();
        let kind = Kind::Custom(buzz_core::kind::KIND_AGENT_OBSERVER_FRAME as u16);
        let p = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let filter = Filter::new().kind(kind).custom_tags(p_tag(), [p]);
        let key = GlobalPKindIndexKey {
            community_id: test_community(),
            kind,
            p: p.to_string(),
        };

        registry.register(conn_id, "observer".to_string(), vec![filter], None);
        assert!(registry.global_p_kind_index.get(&key).is_some());

        registry.remove_subscription(conn_id, "observer");
        assert!(registry.global_p_kind_index.get(&key).is_none());

        let event = make_stored_event_with_p(kind, p, None);
        assert!(registry.fan_out(&event).is_empty());
    }

    #[test]
    fn test_global_wildcard_index_fan_out() {
        // A global subscription with no kind filter should receive all global events.
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();

        registry.register(
            conn_id,
            "global_wildcard".to_string(),
            vec![Filter::new()], // kindless
            None,
        );

        let event_text = make_stored_event(Kind::TextNote, None);
        let matches = registry.fan_out(&event_text);
        assert_eq!(matches.len(), 1);

        let event_meta = make_stored_event(Kind::Metadata, None);
        let matches = registry.fan_out(&event_meta);
        assert_eq!(matches.len(), 1);

        // Must NOT receive channel-scoped events.
        let channel_event = make_stored_event(Kind::TextNote, Some(Uuid::new_v4()));
        assert!(registry.fan_out(&channel_event).is_empty());
    }

    #[test]
    fn test_global_index_removal_cleanup() {
        // Removing a global subscription should clean up the global indexes.
        let registry = SubscriptionRegistry::new();
        let conn_id = Uuid::new_v4();

        // Kind-specific global sub.
        registry.register(
            conn_id,
            "g1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            None,
        );
        assert!(registry
            .global_kind_index
            .get(&(test_community(), Kind::TextNote))
            .is_some());

        registry.remove_subscription(conn_id, "g1");
        assert!(registry
            .global_kind_index
            .get(&(test_community(), Kind::TextNote))
            .is_none());

        // Wildcard global sub.
        registry.register(conn_id, "g2".to_string(), vec![Filter::new()], None);
        assert!(registry
            .global_wildcard_index
            .get(&test_community())
            .is_some());

        registry.remove_subscription(conn_id, "g2");
        assert!(registry
            .global_wildcard_index
            .get(&test_community())
            .is_none());
    }

    #[test]
    fn test_global_and_channel_subs_are_isolated() {
        // Global subs must not see channel events; channel subs must not see global events.
        // This tests the invariant with the new global index in place.
        let registry = SubscriptionRegistry::new();
        let conn_global = Uuid::new_v4();
        let conn_channel = Uuid::new_v4();
        let channel_id = Uuid::new_v4();

        registry.register(
            conn_global,
            "global".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            None,
        );
        registry.register(
            conn_channel,
            "channel".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );

        let global_event = make_stored_event(Kind::TextNote, None);
        let matches = registry.fan_out(&global_event);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].0, conn_global,
            "only global sub sees global event"
        );

        let channel_event = make_stored_event(Kind::TextNote, Some(channel_id));
        let matches = registry.fan_out(&channel_event);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].0, conn_channel,
            "only channel sub sees channel event"
        );
    }

    #[test]
    fn scoped_registry_does_not_cross_fanout_same_channel_and_kind() {
        let registry = SubscriptionRegistry::new();
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xbbbb));
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let channel_id = Uuid::from_u128(0xcccc);

        registry.register_scoped(
            community_a,
            conn_a,
            "a".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );
        registry.register_scoped(
            community_b,
            conn_b,
            "b".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );

        let event = make_stored_event(Kind::TextNote, Some(channel_id));
        assert_eq!(
            registry.fan_out_scoped(community_a, &event),
            vec![(conn_a, "a".to_string())]
        );
        assert_eq!(
            registry.fan_out_scoped(community_b, &event),
            vec![(conn_b, "b".to_string())]
        );
    }

    #[test]
    fn scoped_global_registry_does_not_cross_fanout_same_kind() {
        let registry = SubscriptionRegistry::new();
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xbbbb));
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();

        registry.register_scoped(
            community_a,
            conn_a,
            "a".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            None,
        );
        registry.register_scoped(
            community_b,
            conn_b,
            "b".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            None,
        );

        let event = make_stored_event(Kind::TextNote, None);
        assert_eq!(
            registry.fan_out_scoped(community_a, &event),
            vec![(conn_a, "a".to_string())]
        );
        assert_eq!(
            registry.fan_out_scoped(community_b, &event),
            vec![(conn_b, "b".to_string())]
        );
    }

    #[test]
    fn scoped_remove_channel_subscriptions_keeps_same_channel_in_other_community() {
        let registry = SubscriptionRegistry::new();
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xbbbb));
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let channel_id = Uuid::from_u128(0xcccc);

        registry.register_scoped(
            community_a,
            conn_a,
            "a".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );
        registry.register_scoped(
            community_b,
            conn_b,
            "b".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );

        let removed = registry.remove_channel_subscriptions_scoped(community_a, conn_a, channel_id);
        assert_eq!(removed.len(), 1);

        let event = make_stored_event(Kind::TextNote, Some(channel_id));
        assert!(registry.fan_out_scoped(community_a, &event).is_empty());
        assert_eq!(
            registry.fan_out_scoped(community_b, &event),
            vec![(conn_b, "b".to_string())]
        );
    }

    #[test]
    fn revoking_one_channel_keeps_multi_channel_subscription_live() {
        let registry = SubscriptionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let conn = Uuid::new_v4();
        let channel_a = Uuid::new_v4();
        let channel_b = Uuid::new_v4();
        let filters = vec![Filter::new().kind(Kind::TextNote)];
        registry.register_channels_scoped(
            community,
            conn,
            "multi".to_string(),
            filters,
            vec![channel_a, channel_b],
        );

        let updates = registry.remove_channel_subscriptions_scoped(community, conn, channel_a);
        assert_eq!(
            updates,
            vec![ChannelSubscriptionUpdate {
                sub_id: "multi".to_string(),
                removed: false,
            }]
        );
        assert!(registry
            .fan_out_scoped(
                community,
                &make_stored_event(Kind::TextNote, Some(channel_a))
            )
            .is_empty());
        assert_eq!(
            registry.fan_out_scoped(
                community,
                &make_stored_event(Kind::TextNote, Some(channel_b))
            ),
            vec![(conn, "multi".to_string())]
        );

        let updates = registry.remove_channel_subscriptions_scoped(community, conn, channel_b);
        assert_eq!(
            updates,
            vec![ChannelSubscriptionUpdate {
                sub_id: "multi".to_string(),
                removed: true,
            }]
        );
    }

    #[test]
    fn per_community_subscriptions_snapshot_is_correctly_scoped() {
        // Verify that per_community_subscriptions() returns the correct
        // per-community counts and that removal keeps the snapshot accurate.
        let registry = SubscriptionRegistry::new();
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xbbbb));
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let channel = Uuid::new_v4();

        // 2 subs in community A, 1 sub in community B.
        registry.register_scoped(
            community_a,
            conn_a,
            "a1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel),
        );
        registry.register_scoped(
            community_a,
            conn_a,
            "a2".to_string(),
            vec![Filter::new().kind(Kind::Metadata)],
            None,
        );
        registry.register_scoped(
            community_b,
            conn_b,
            "b1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel),
        );

        let snap = registry.per_community_subscriptions();
        assert_eq!(snap.get(&community_a), Some(&2), "community A: 2 subs");
        assert_eq!(snap.get(&community_b), Some(&1), "community B: 1 sub");

        // Remove one sub from community A — snapshot should reflect it.
        registry.remove_subscription(conn_a, "a1");
        let snap2 = registry.per_community_subscriptions();
        assert_eq!(
            snap2.get(&community_a),
            Some(&1),
            "community A: 1 sub after removal"
        );
        assert_eq!(snap2.get(&community_b), Some(&1), "community B: unchanged");
    }

    #[test]
    fn per_community_subscriptions_drops_to_zero_when_all_subs_removed() {
        // Regression: when the last subscription for a community is removed,
        // per_community_subscriptions() must return no entry (not stale nonzero).
        // The usage poller emits one explicit zero for the previously observed
        // label before allowing its idle timeout to remove the series.
        let registry = SubscriptionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xcccc));
        let conn = Uuid::new_v4();

        registry.register_scoped(
            community,
            conn,
            "c1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            None,
        );

        // Verify nonzero before removal.
        let snap1 = registry.per_community_subscriptions();
        assert_eq!(snap1.get(&community), Some(&1), "1 sub before removal");

        // Remove the only sub — community should no longer appear in snapshot.
        registry.remove_subscription(conn, "c1");
        let snap2 = registry.per_community_subscriptions();
        assert!(
            snap2.get(&community).copied().unwrap_or(0) == 0,
            "community must have 0 subs after last removal; got {:?}",
            snap2.get(&community)
        );
    }

    fn group_state_event(channel_id: Uuid, extra: &[[&str; 2]]) -> StoredEvent {
        let keys = Keys::generate();
        let mut tags = vec![Tag::parse(["d", &channel_id.to_string()]).expect("d tag")];
        tags.extend(extra.iter().map(|tag| Tag::parse(*tag).expect("tag")));
        let event = EventBuilder::new(Kind::Custom(39000), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign");
        StoredEvent::with_received_at(event, Utc::now(), Some(channel_id), true)
    }

    fn group_state_filter() -> Filter {
        Filter::new().kind(Kind::Custom(39000))
    }

    #[test]
    fn channel_reaches_only_the_filter_that_names_it() {
        // Filter A names channel `a` by `#d` but also requires `#t:x`; filter
        // B (`#P`) has only the global scope. A replacement for `a` without
        // `t:x` matches B but not A, so it must not reach the subscription:
        // B may not read `a` just because its sibling named it.
        let registry = SubscriptionRegistry::new();
        let conn = Uuid::new_v4();
        let a = Uuid::new_v4();
        let creator = Keys::generate().public_key().to_hex();
        let named = group_state_filter()
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), a.to_string())
            .custom_tag(SingleLetterTag::lowercase(Alphabet::T), "x");
        let by_creator =
            group_state_filter().custom_tag(SingleLetterTag::uppercase(Alphabet::P), &creator);
        registry.register_filters_scoped(
            test_community(),
            conn,
            "s".to_string(),
            vec![
                (named, SubscriptionScope::channels([a]).with_global()),
                (by_creator, SubscriptionScope::global()),
            ],
        );

        assert!(
            registry
                .fan_out(&group_state_event(a, &[["P", &creator]]))
                .is_empty(),
            "a sibling's channel must not reach a filter that does not name it"
        );
        assert_eq!(
            registry.fan_out(&group_state_event(a, &[["t", "x"]])),
            vec![(conn, "s".to_string())],
            "the naming filter receives its own channel's replacement"
        );
    }

    #[test]
    fn mixed_named_and_global_filters_each_keep_live_delivery() {
        let registry = SubscriptionRegistry::new();
        let conn = Uuid::new_v4();
        let a = Uuid::new_v4();
        let other = Uuid::new_v4();
        let named =
            group_state_filter().custom_tag(SingleLetterTag::lowercase(Alphabet::D), a.to_string());
        let global = Filter::new().kind(Kind::TextNote);
        registry.register_filters_scoped(
            test_community(),
            conn,
            "s".to_string(),
            vec![
                (named, SubscriptionScope::channels([a]).with_global()),
                (global, SubscriptionScope::global()),
            ],
        );
        let hit = vec![(conn, "s".to_string())];

        assert_eq!(registry.fan_out(&group_state_event(a, &[])), hit);
        assert_eq!(
            registry.fan_out(&make_stored_event(Kind::TextNote, None)),
            hit
        );
        assert!(registry.fan_out(&group_state_event(other, &[])).is_empty());
        assert!(registry
            .fan_out(&make_stored_event(Kind::TextNote, Some(a)))
            .is_empty());
    }

    #[test]
    fn per_filter_topics_retain_and_release_each_topic_once() {
        let registry = SubscriptionRegistry::new();
        let conn = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let filters = vec![
            (
                group_state_filter(),
                SubscriptionScope::channels([a, b]).with_global(),
            ),
            (
                Filter::new().kind(Kind::TextNote),
                SubscriptionScope::channels([a]),
            ),
            (
                Filter::new().kind(Kind::Metadata),
                SubscriptionScope::global(),
            ),
        ];
        assert!(registry
            .register_filters_scoped(test_community(), conn, "s".into(), filters.clone())
            .is_none());

        // Replacement reports the union of the old filters' topics, once each.
        let replaced = registry
            .register_filters_scoped(test_community(), conn, "s".into(), filters)
            .expect("replaced");
        assert_eq!(
            replaced.scope,
            SubscriptionScope::channels([a, b]).with_global()
        );

        // Revoking `a` reports the subscription once and keeps it live for
        // `b` and the global topic; both filters lose `a`.
        let updates = registry.remove_channel_subscriptions_scoped(test_community(), conn, a);
        assert_eq!(
            updates,
            vec![ChannelSubscriptionUpdate {
                sub_id: "s".into(),
                removed: false,
            }]
        );
        assert!(registry.fan_out(&group_state_event(a, &[])).is_empty());
        assert!(registry
            .fan_out(&make_stored_event(Kind::TextNote, Some(a)))
            .is_empty());
        assert_eq!(registry.fan_out(&group_state_event(b, &[])).len(), 1);
        assert_eq!(
            registry
                .fan_out(&make_stored_event(Kind::Metadata, None))
                .len(),
            1
        );
        assert!(registry.channel_subscriber_conns(a).is_empty());

        let removed = registry.remove_connection(conn);
        assert_eq!(
            removed,
            vec![RemovedSubscription {
                community_id: test_community(),
                scope: SubscriptionScope::channels([b]).with_global(),
            }]
        );
        assert!(registry.channel_kind_index.is_empty());
        assert!(registry.global_kind_index.is_empty());
    }

    #[test]
    fn revoking_the_last_channel_of_a_channel_only_subscription_removes_it() {
        let registry = SubscriptionRegistry::new();
        let conn = Uuid::new_v4();
        let a = Uuid::new_v4();
        registry.register_filters_scoped(
            test_community(),
            conn,
            "s".into(),
            vec![
                (
                    Filter::new().kind(Kind::TextNote),
                    SubscriptionScope::channels([a]),
                ),
                (group_state_filter(), SubscriptionScope::channels([a])),
            ],
        );
        let updates = registry.remove_channel_subscriptions_scoped(test_community(), conn, a);
        assert_eq!(
            updates,
            vec![ChannelSubscriptionUpdate {
                sub_id: "s".into(),
                removed: true,
            }]
        );
        assert!(registry.get_filters(conn, "s").is_none());
        assert!(registry.channel_kind_index.is_empty());
    }
}
