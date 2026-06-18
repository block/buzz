//! Thread-safe in-memory invite token registry.

use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

use crate::error::ProxyError;
use crate::invite::InviteToken;

/// In-memory invite token store backed by DashMap.
pub struct InviteStore {
    tokens: DashMap<String, InviteToken>,
}

impl InviteStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            tokens: DashMap::new(),
        }
    }

    /// Insert a new invite token.
    pub fn insert(&self, token: InviteToken) {
        self.tokens.insert(token.token.clone(), token);
    }

    /// Validate a token string. Returns the list of channel UUIDs if valid.
    pub fn validate(&self, token_str: &str) -> Result<Vec<Uuid>, ProxyError> {
        let entry = self
            .tokens
            .get(token_str)
            .ok_or(ProxyError::InviteNotFound)?;
        entry.validate(Utc::now())?;
        Ok(entry.channel_ids.clone())
    }

    /// Atomically validate and consume a token (increment use count).
    /// Returns the list of channel UUIDs if valid.
    pub fn validate_and_consume(&self, token_str: &str) -> Result<Vec<Uuid>, ProxyError> {
        let mut entry = self
            .tokens
            .get_mut(token_str)
            .ok_or(ProxyError::InviteNotFound)?;
        entry.validate(Utc::now())?;
        let channels = entry.channel_ids.clone();
        entry.consume();
        Ok(channels)
    }

    /// Remove a token from the store.
    pub fn remove(&self, token_str: &str) -> Option<InviteToken> {
        self.tokens.remove(token_str).map(|(_, v)| v)
    }

    /// Number of tokens in the store.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

impl Default for InviteStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn future_token(token: &str, channels: Vec<Uuid>, max_uses: u32) -> InviteToken {
        InviteToken::new(token, channels, Utc::now() + Duration::hours(1), max_uses)
    }

    fn expired_token(token: &str) -> InviteToken {
        InviteToken::new(token, vec![], Utc::now() - Duration::seconds(1), 5)
    }

    #[test]
    fn validate_returns_channels() {
        let store = InviteStore::new();
        let ch1 = Uuid::new_v4();
        let ch2 = Uuid::new_v4();
        store.insert(future_token("tok-1", vec![ch1, ch2], 5));

        let channels = store.validate("tok-1").unwrap();
        assert_eq!(channels, vec![ch1, ch2]);
    }

    #[test]
    fn validate_not_found() {
        let store = InviteStore::new();
        assert!(matches!(
            store.validate("nonexistent"),
            Err(ProxyError::InviteNotFound)
        ));
    }

    #[test]
    fn validate_expired() {
        let store = InviteStore::new();
        store.insert(expired_token("tok-expired"));
        assert!(matches!(
            store.validate("tok-expired"),
            Err(ProxyError::InviteExpired)
        ));
    }

    #[test]
    fn validate_and_consume_increments() {
        let store = InviteStore::new();
        store.insert(future_token("tok-use", vec![], 2));

        store.validate_and_consume("tok-use").unwrap();
        store.validate_and_consume("tok-use").unwrap();

        // Third use should fail
        assert!(matches!(
            store.validate_and_consume("tok-use"),
            Err(ProxyError::InviteExhausted)
        ));
    }

    #[test]
    fn remove_works() {
        let store = InviteStore::new();
        store.insert(future_token("tok-rm", vec![], 5));
        assert_eq!(store.len(), 1);

        let removed = store.remove("tok-rm");
        assert!(removed.is_some());
        assert_eq!(store.len(), 0);
    }
}
