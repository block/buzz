use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

pub(crate) const MAX_SIBLING_CACHE_ENTRIES: usize = 256;
pub(crate) const NEGATIVE_SIBLING_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
enum CachedSiblingAuthorization {
    Authorized,
    DeniedUntil(tokio::time::Instant),
}

/// Cache for the agent's owner pubkey and sibling authorization lookups.
///
/// Verified siblings remain cached for the process lifetime because their
/// attestations are immutable. Negative results expire so a profile that later
/// gains a valid attestation can be retried. Callers must not cache
/// [`SiblingAuthorization::Unavailable`].
pub(crate) struct OwnerCache {
    pubkey: Option<String>,
    siblings: Mutex<HashMap<String, CachedSiblingAuthorization>>,
}

impl OwnerCache {
    pub(crate) fn new(initial: Option<String>) -> Self {
        Self {
            pubkey: initial,
            siblings: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn get(&self) -> Option<&str> {
        self.pubkey.as_deref()
    }

    pub(crate) fn is_known_sibling(&self, author: &str) -> Option<bool> {
        self.is_known_sibling_at(author, tokio::time::Instant::now())
    }

    pub(crate) fn is_known_sibling_at(
        &self,
        author: &str,
        now: tokio::time::Instant,
    ) -> Option<bool> {
        let mut cache = self.siblings.lock().ok()?;
        match cache.get(author).copied() {
            Some(CachedSiblingAuthorization::Authorized) => Some(true),
            Some(CachedSiblingAuthorization::DeniedUntil(expires_at)) if expires_at > now => {
                Some(false)
            }
            Some(CachedSiblingAuthorization::DeniedUntil(_)) => {
                cache.remove(author);
                None
            }
            None => None,
        }
    }

    pub(crate) fn cache_sibling(&self, author: String, is_sibling: bool) {
        self.cache_sibling_at(author, is_sibling, tokio::time::Instant::now());
    }

    pub(crate) fn cache_sibling_at(
        &self,
        author: String,
        is_sibling: bool,
        now: tokio::time::Instant,
    ) {
        if let Ok(mut cache) = self.siblings.lock() {
            if !cache.contains_key(&author) && cache.len() >= MAX_SIBLING_CACHE_ENTRIES {
                cache.clear();
            }
            let authorization = if is_sibling {
                CachedSiblingAuthorization::Authorized
            } else {
                CachedSiblingAuthorization::DeniedUntil(now + NEGATIVE_SIBLING_CACHE_TTL)
            };
            cache.insert(author, authorization);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SiblingAuthorization {
    Authorized,
    Denied,
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_authorization_does_not_expire() {
        let cache = OwnerCache::new(Some("owner".into()));
        let now = tokio::time::Instant::now();
        cache.cache_sibling_at("sibling".into(), true, now);

        assert_eq!(
            cache.is_known_sibling_at(
                "sibling",
                now + NEGATIVE_SIBLING_CACHE_TTL + Duration::from_secs(1),
            ),
            Some(true)
        );
    }

    #[test]
    fn negative_authorization_expires_at_ttl() {
        let cache = OwnerCache::new(Some("owner".into()));
        let now = tokio::time::Instant::now();
        cache.cache_sibling_at("stranger".into(), false, now);

        assert_eq!(cache.is_known_sibling_at("stranger", now), Some(false));
        assert_eq!(
            cache.is_known_sibling_at("stranger", now + NEGATIVE_SIBLING_CACHE_TTL),
            None
        );
    }

    #[test]
    fn cache_never_exceeds_entry_bound() {
        let cache = OwnerCache::new(Some("owner".into()));
        let now = tokio::time::Instant::now();
        for index in 0..=MAX_SIBLING_CACHE_ENTRIES {
            cache.cache_sibling_at(format!("author-{index}"), false, now);
            assert!(cache.siblings.lock().unwrap().len() <= MAX_SIBLING_CACHE_ENTRIES);
        }
    }
}
