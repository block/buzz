//! Subscription registry with active WebSocket indexes for targeted fan-out.

use std::collections::{HashMap, HashSet};

use dashmap::DashMap;
use nostr::{Alphabet, Filter, Kind, SingleLetterTag};
use uuid::Uuid;

use sprout_core::{filter::filters_match, StoredEvent};

/// Connection identifier — a UUID assigned to each WebSocket connection.
pub type ConnId = Uuid;
/// Subscription identifier — the client-supplied string from a REQ message.
pub type SubId = String;
/// Stored subscription entry: filters paired with an optional channel scope.
pub type SubEntry = (Vec<Filter>, Option<Uuid>);

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
    kind: Kind,
    p: String,
}

/// Thread-safe registry of active subscriptions with targeted in-memory fan-out indexes.
#[derive(Debug, Default)]
pub struct SubscriptionRegistry {
    /// Maps conn_id → sub_id → (filters, channel_id).
    /// Storing channel_id alongside filters enables O(1) targeted index removal.
    subs: DashMap<ConnId, HashMap<SubId, SubEntry>>,
    channel_kind_index: DashMap<IndexKey, Vec<(ConnId, SubId)>>,
    /// Subscriptions with a channel_id but no kind filter — need to receive ALL kinds.
    channel_wildcard_index: DashMap<Uuid, Vec<(ConnId, SubId)>>,
    /// Global subscriptions indexed by kind — avoids O(all_subs) scan for global events.
    global_kind_index: DashMap<Kind, Vec<(ConnId, SubId)>>,
    /// Global subscriptions indexed by both kind and `#p` recipient.
    global_p_kind_index: DashMap<GlobalPKindIndexKey, Vec<(ConnId, SubId)>>,
    /// Global subscriptions with no kind filter — wildcard, receives all global events.
    global_wildcard_index: DashMap<(), Vec<(ConnId, SubId)>>,
}

impl SubscriptionRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces any existing subscription with the same sub_id (NIP-01).
    pub fn register(
        &self,
        conn_id: ConnId,
        sub_id: SubId,
        filters: Vec<Filter>,
        channel_id: Option<Uuid>,
    ) {
        self.remove_subscription(conn_id, &sub_id);

        self.subs
            .entry(conn_id)
            .or_default()
            .insert(sub_id.clone(), (filters.clone(), channel_id));
        metrics::gauge!("sprout_subscriptions_active").increment(1.0);

        if let Some(ch_id) = channel_id {
            match extract_kinds_from_filters(&filters) {
                None => {
                    // At least one filter has no `kinds` constraint — wildcard,
                    // this sub wants all kinds in this channel.
                    self.channel_wildcard_index
                        .entry(ch_id)
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
                            .entry(key)
                            .or_default()
                            .push((conn_id, sub_id.clone()));
                    }
                }
            }
        } else {
            // Global subscription. Fully p-constrained filters can use the
            // narrower (kind, #p) index; broader filters stay on the generic
            // kind/wildcard indexes.
            if let Some(keys) = extract_global_p_kind_index_keys(&filters) {
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
                            .entry(())
                            .or_default()
                            .push((conn_id, sub_id.clone()));
                    }
                    Some(kinds) if kinds.is_empty() => {}
                    Some(kinds) => {
                        for kind in kinds {
                            self.global_kind_index
                                .entry(kind)
                                .or_default()
                                .push((conn_id, sub_id.clone()));
                        }
                    }
                }
            }
        }
    }

    /// Remove a single subscription and clean up its index entries.
    pub fn remove_subscription(&self, conn_id: ConnId, sub_id: &str) {
        if let Some(mut conn_subs) = self.subs.get_mut(&conn_id) {
            if let Some((filters, channel_id)) = conn_subs.remove(sub_id) {
                self.remove_from_index(conn_id, sub_id, &filters, channel_id);
                metrics::gauge!("sprout_subscriptions_active").decrement(1.0);
            }
        }
    }

    /// Remove all subscriptions for a connection and clean up index entries.
    pub fn remove_connection(&self, conn_id: ConnId) {
        if let Some((_, conn_subs)) = self.subs.remove(&conn_id) {
            let count = conn_subs.len();
            for (sub_id, (filters, channel_id)) in &conn_subs {
                self.remove_from_index(conn_id, sub_id, filters, *channel_id);
            }
            metrics::gauge!("sprout_subscriptions_active").decrement(count as f64);
        }
    }

    /// Remove all subscriptions on `conn_id` scoped to `channel_id`.
    pub fn remove_channel_subscriptions(&self, conn_id: ConnId, channel_id: Uuid) -> Vec<SubId> {
        let sub_ids: Vec<SubId> = self
            .subs
            .get(&conn_id)
            .map(|conn_subs| {
                conn_subs
                    .iter()
                    .filter_map(|(sub_id, (_, sub_channel_id))| {
                        (*sub_channel_id == Some(channel_id)).then_some(sub_id.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();

        for sub_id in &sub_ids {
            self.remove_subscription(conn_id, sub_id);
        }

        sub_ids
    }

    /// Return the distinct connection IDs holding any subscription scoped to
    /// `channel_id` (both kind-filtered and wildcard channel subscriptions).
    pub fn channel_subscriber_conns(&self, channel_id: Uuid) -> Vec<ConnId> {
        let mut conns: HashSet<ConnId> = HashSet::new();
        for entry in self.channel_kind_index.iter() {
            if entry.key().channel_id == channel_id {
                conns.extend(entry.value().iter().map(|(conn_id, _)| *conn_id));
            }
        }
        if let Some(entry) = self.channel_wildcard_index.get(&channel_id) {
            conns.extend(entry.value().iter().map(|(conn_id, _)| *conn_id));
        }
        conns.into_iter().collect()
    }

    /// Return all (conn_id, sub_id) pairs whose filters match the given event.
    pub fn fan_out(&self, event: &StoredEvent) -> Vec<(ConnId, SubId)> {
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        if let Some(channel_id) = event.channel_id {
            let key = IndexKey {
                channel_id,
                kind: event.event.kind,
            };
            if let Some(candidates) = self.channel_kind_index.get(&key) {
                for (conn_id, sub_id) in candidates.iter() {
                    self.push_match(*conn_id, sub_id, event, &mut results, &mut seen);
                }
            }
            // Also check wildcard (channel-only, kindless) index
            if let Some(wildcards) = self.channel_wildcard_index.get(&channel_id) {
                for (conn_id, sub_id) in wildcards.iter() {
                    self.push_match(*conn_id, sub_id, event, &mut results, &mut seen);
                }
            }
        } else {
            // Global event (channel_id = None) — use global indexes for sub-linear fan-out.
            // Channel-scoped subscriptions are never in these indexes, preserving the
            // scoping invariant without an explicit skip check.
            for p in event_p_tag_values(event) {
                let key = GlobalPKindIndexKey {
                    kind: event.event.kind,
                    p,
                };
                if let Some(candidates) = self.global_p_kind_index.get(&key) {
                    for (conn_id, sub_id) in candidates.iter() {
                        self.push_match(*conn_id, sub_id, event, &mut results, &mut seen);
                    }
                }
            }
            if let Some(candidates) = self.global_kind_index.get(&event.event.kind) {
                for (conn_id, sub_id) in candidates.iter() {
                    self.push_match(*conn_id, sub_id, event, &mut results, &mut seen);
                }
            }
            // Also check global wildcard (kindless global subs).
            if let Some(wildcards) = self.global_wildcard_index.get(&()) {
                for (conn_id, sub_id) in wildcards.iter() {
                    self.push_match(*conn_id, sub_id, event, &mut results, &mut seen);
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

    /// Return the filters for a specific subscription, or `None` if not found.
    pub fn get_filters(&self, conn_id: ConnId, sub_id: &str) -> Option<Vec<Filter>> {
        self.subs
            .get(&conn_id)
            .and_then(|conn_subs| conn_subs.get(sub_id).map(|(filters, _)| filters.clone()))
    }

    /// Return the total number of active subscriptions across all connections.
    pub fn total_subscriptions(&self) -> usize {
        self.subs.iter().map(|e| e.value().len()).sum()
    }

    /// Return the total number of connections with at least one active subscription.
    pub fn total_connections(&self) -> usize {
        self.subs.len()
    }

    fn push_match(
        &self,
        conn_id: ConnId,
        sub_id: &str,
        event: &StoredEvent,
        results: &mut Vec<(ConnId, SubId)>,
        seen: &mut HashSet<(ConnId, SubId)>,
    ) {
        if let Some(conn_subs) = self.subs.get(&conn_id) {
            if let Some((filters, _)) = conn_subs.get(sub_id) {
                if filters_match(filters, event) {
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
    ///
    /// If `channel_id` is None the subscription was never indexed (slow-path), so there
    /// is nothing to remove.
    fn remove_from_index(
        &self,
        conn_id: ConnId,
        sub_id: &str,
        filters: &[Filter],
        channel_id: Option<Uuid>,
    ) {
        if let Some(ch_id) = channel_id {
            match extract_kinds_from_filters(filters) {
                // None = wildcard (at least one filter had no kinds constraint)
                None => {
                    // Was in wildcard index
                    if let Some(mut entries) = self.channel_wildcard_index.get_mut(&ch_id) {
                        entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                        if entries.is_empty() {
                            drop(entries);
                            self.channel_wildcard_index.remove(&ch_id);
                        }
                    }
                }
                Some(kinds) if kinds.is_empty() => {
                    // `kinds: []` subscriptions are never indexed (they match nothing),
                    // so there is nothing to remove here.
                }
                Some(kinds) => {
                    // Was in kind-specific index
                    for kind in kinds {
                        let key = IndexKey {
                            channel_id: ch_id,
                            kind,
                        };
                        if let Some(mut entries) = self.channel_kind_index.get_mut(&key) {
                            entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                            if entries.is_empty() {
                                drop(entries);
                                self.channel_kind_index.remove(&key);
                            }
                        }
                    }
                }
            }
        } else {
            // Global subscription — remove from the same global index chosen at registration.
            if let Some(keys) = extract_global_p_kind_index_keys(filters) {
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
                        if let Some(mut entries) = self.global_wildcard_index.get_mut(&()) {
                            entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                            if entries.is_empty() {
                                drop(entries);
                                self.global_wildcard_index.remove(&());
                            }
                        }
                    }
                    Some(kinds) if kinds.is_empty() => {}
                    Some(kinds) => {
                        for kind in kinds {
                            if let Some(mut entries) = self.global_kind_index.get_mut(&kind) {
                                entries.retain(|(cid, sid)| !(*cid == conn_id && sid == sub_id));
                                if entries.is_empty() {
                                    drop(entries);
                                    self.global_kind_index.remove(&kind);
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

fn extract_global_p_kind_index_keys(filters: &[Filter]) -> Option<Vec<GlobalPKindIndexKey>> {
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
mod tests {
    use super::*;
    use chrono::Utc;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sprout_core::StoredEvent;

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
        assert!(registry.channel_kind_index.get(&key_x).is_none());

        let key_y = IndexKey {
            channel_id: channel_y,
            kind: Kind::TextNote,
        };
        let entries = registry
            .channel_kind_index
            .get(&key_y)
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

        assert!(registry.channel_wildcard_index.get(&channel_id).is_some());

        registry.remove_subscription(conn_id, "sub1");

        assert!(registry.channel_wildcard_index.get(&channel_id).is_none());

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
        assert!(registry.channel_wildcard_index.get(&channel_id).is_some());

        registry.register(
            conn_id,
            "sub1".to_string(),
            vec![Filter::new().kind(Kind::TextNote)],
            Some(channel_id),
        );

        assert!(registry.channel_wildcard_index.get(&channel_id).is_none());

        let key = IndexKey {
            channel_id,
            kind: Kind::TextNote,
        };
        assert!(registry.channel_kind_index.get(&key).is_some());

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
            registry.channel_wildcard_index.get(&channel_id).is_none(),
            "kinds:[] sub must NOT be in the wildcard index"
        );

        let key = IndexKey {
            channel_id,
            kind: Kind::TextNote,
        };
        assert!(
            registry.channel_kind_index.get(&key).is_none(),
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

        assert!(registry.channel_wildcard_index.get(&channel_id).is_none());
        let key = IndexKey {
            channel_id,
            kind: Kind::TextNote,
        };
        assert!(registry.channel_kind_index.get(&key).is_none());
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
        let kind = Kind::Custom(sprout_core::kind::KIND_AGENT_OBSERVER_FRAME as u16);
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
            registry.global_kind_index.get(&kind).is_none(),
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
        let kind = Kind::Custom(sprout_core::kind::KIND_AGENT_OBSERVER_FRAME as u16);
        let p = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let filter = Filter::new().kind(kind).custom_tags(p_tag(), [p]);
        let key = GlobalPKindIndexKey {
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
        assert!(registry.global_kind_index.get(&Kind::TextNote).is_some());

        registry.remove_subscription(conn_id, "g1");
        assert!(registry.global_kind_index.get(&Kind::TextNote).is_none());

        // Wildcard global sub.
        registry.register(conn_id, "g2".to_string(), vec![Filter::new()], None);
        assert!(registry.global_wildcard_index.get(&()).is_some());

        registry.remove_subscription(conn_id, "g2");
        assert!(registry.global_wildcard_index.get(&()).is_none());
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
}
