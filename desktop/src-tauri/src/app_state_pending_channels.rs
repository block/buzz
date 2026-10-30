//! Pending-owner channel overlay for [`AppState`].
//!
//! A channel this identity just created via `create_channel` is relay-signed
//! (kind:39000), so its kind:39002 owner membership does not land immediately.
//! Until it does, the `(creator_pubkey, channel_id)` overlay keeps the channel
//! classified `is_member=true` without an all-open directory scan (#1761). The
//! set is keyed by pubkey so an in-process identity swap never inherits another
//! identity's entry, and entries clear once real membership is observed.

use crate::app_state::AppState;

impl AppState {
    /// Record that `channel_id` was just created by `creator_pubkey` and its
    /// kind:39002 owner membership has not yet been observed.
    pub fn mark_pending_owned_channel(&self, creator_pubkey: &str, channel_id: &str) {
        if let Ok(mut set) = self.pending_owned_channels.lock() {
            set.insert((creator_pubkey.to_string(), channel_id.to_string()));
        }
    }

    /// Whether `channel_id` is still awaiting `my_pubkey`'s kind:39002 entry.
    /// Bound to `my_pubkey` so an in-process identity swap never inherits
    /// another identity's pending-owner entry for the same channel id.
    pub fn is_pending_owned_channel(&self, my_pubkey: &str, channel_id: &str) -> bool {
        self.pending_owned_channels
            .lock()
            .map(|set| set.contains(&(my_pubkey.to_string(), channel_id.to_string())))
            .unwrap_or(false)
    }

    /// Channel ids `my_pubkey` created whose kind:39002 membership has not yet
    /// been observed. The member-only channel poll unions these with the real
    /// member set so a just-created channel stays visible without an all-open
    /// directory scan (#1761).
    pub fn pending_owned_channel_ids(&self, my_pubkey: &str) -> Vec<String> {
        self.pending_owned_channels
            .lock()
            .map(|set| {
                set.iter()
                    .filter(|(owner, _)| owner == my_pubkey)
                    .map(|(_, channel_id)| channel_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Drop the `(my_pubkey, channel_id)` entry from the pending-owner
    /// overlay once that identity's real kind:39002 membership has been
    /// observed.
    pub fn clear_pending_owned_channel(&self, my_pubkey: &str, channel_id: &str) {
        if let Ok(mut set) = self.pending_owned_channels.lock() {
            set.remove(&(my_pubkey.to_string(), channel_id.to_string()));
        }
    }
}
