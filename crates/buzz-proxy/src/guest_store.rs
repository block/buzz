//! Thread-safe in-memory guest registry for pubkey-based access control.

use dashmap::DashMap;
use nostr::PublicKey;
use uuid::Uuid;

/// In-memory guest registry. Maps external Nostr pubkeys to their allowed channels.
///
/// Guests registered here can authenticate via NIP-42 alone — no invite token needed.
/// The store is backed by [`DashMap`] for lock-free concurrent access.
pub struct GuestStore {
    guests: DashMap<PublicKey, Vec<Uuid>>,
}

impl GuestStore {
    /// Create an empty guest store.
    pub fn new() -> Self {
        Self {
            guests: DashMap::new(),
        }
    }

    /// Look up a pubkey. Returns `Some(channels)` if registered, `None` otherwise.
    pub fn lookup(&self, pubkey: &PublicKey) -> Option<Vec<Uuid>> {
        self.guests.get(pubkey).map(|entry| entry.clone())
    }

    /// Register a guest pubkey with access to specific channels.
    ///
    /// Overwrites any existing registration for this pubkey.
    pub fn register(&self, pubkey: PublicKey, channels: Vec<Uuid>) {
        self.guests.insert(pubkey, channels);
    }

    /// Remove a guest's registration.
    ///
    /// Returns `true` if the pubkey was found and removed, `false` if it was not registered.
    pub fn remove(&self, pubkey: &PublicKey) -> bool {
        self.guests.remove(pubkey).is_some()
    }

    /// Number of registered guests.
    pub fn len(&self) -> usize {
        self.guests.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.guests.is_empty()
    }

    /// List all registered guests.
    ///
    /// Returns a snapshot of all `(pubkey, channels)` pairs, suitable for admin listing.
    pub fn all(&self) -> Vec<(PublicKey, Vec<Uuid>)> {
        self.guests
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect()
    }
}

impl Default for GuestStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn random_pubkey() -> PublicKey {
        Keys::generate().public_key()
    }

    #[test]
    fn register_and_lookup_succeeds() {
        let store = GuestStore::new();
        let pubkey = random_pubkey();
        let ch1 = Uuid::new_v4();
        let ch2 = Uuid::new_v4();

        store.register(pubkey, vec![ch1, ch2]);

        let channels = store
            .lookup(&pubkey)
            .expect("should find registered pubkey");
        assert_eq!(channels, vec![ch1, ch2]);
    }

    #[test]
    fn lookup_unknown_returns_none() {
        let store = GuestStore::new();
        let pubkey = random_pubkey();
        assert!(store.lookup(&pubkey).is_none());
    }

    #[test]
    fn remove_returns_true_for_existing_false_for_unknown() {
        let store = GuestStore::new();
        let pubkey = random_pubkey();

        store.register(pubkey, vec![Uuid::new_v4()]);
        assert!(
            store.remove(&pubkey),
            "should return true for existing pubkey"
        );
        assert!(
            !store.remove(&pubkey),
            "should return false for already-removed pubkey"
        );

        let unknown = random_pubkey();
        assert!(
            !store.remove(&unknown),
            "should return false for never-registered pubkey"
        );
    }

    #[test]
    fn register_overwrites_previous_channels() {
        let store = GuestStore::new();
        let pubkey = random_pubkey();
        let ch1 = Uuid::new_v4();
        let ch2 = Uuid::new_v4();

        store.register(pubkey, vec![ch1]);
        store.register(pubkey, vec![ch2]);

        let channels = store.lookup(&pubkey).expect("should find pubkey");
        assert_eq!(
            channels,
            vec![ch2],
            "second register should overwrite first"
        );
        assert!(!channels.contains(&ch1), "old channel should be gone");
    }

    #[test]
    fn all_returns_all_registered_guests() {
        let store = GuestStore::new();
        let pk1 = random_pubkey();
        let pk2 = random_pubkey();
        let ch1 = Uuid::new_v4();
        let ch2 = Uuid::new_v4();

        store.register(pk1, vec![ch1]);
        store.register(pk2, vec![ch2]);

        let all = store.all();
        assert_eq!(all.len(), 2);

        let keys: Vec<PublicKey> = all.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&pk1));
        assert!(keys.contains(&pk2));
    }

    #[test]
    fn len_and_is_empty() {
        let store = GuestStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        store.register(random_pubkey(), vec![]);
        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
    }
}
