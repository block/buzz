//! Community thread policy (NIP-TP) as the harness sees it.
//!
//! An admin can switch the community between `nested` and `flat` while the
//! harness runs, so the policy is re-read from NIP-11 at most once per
//! [`THREAD_POLICY_TTL`] instead of once at startup.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::relay::{RelayError, RestClient, ThreadPolicy};

/// How long a fetched policy is used before the next prompt re-reads NIP-11.
/// A failed refresh also waits this long before it is retried.
const THREAD_POLICY_TTL: Duration = Duration::from_secs(30);
/// Upper bound on one refresh, which runs before a prompt is sent.
const THREAD_POLICY_FETCH_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
struct State {
    policy: ThreadPolicy,
    checked_at: Option<Instant>,
    /// Number of the last fetch started.
    started: u64,
    /// Number of the fetch whose result `policy` holds; 0 = none yet.
    applied: u64,
}

/// Cached community thread policy, refreshed from the relay's NIP-11 document.
#[derive(Debug, Clone)]
pub struct ThreadPolicyResolver {
    rest_client: RestClient,
    state: Arc<Mutex<State>>,
}

impl ThreadPolicyResolver {
    /// Create a resolver that reads NIP-11 through `rest_client`.
    pub fn new(rest_client: RestClient) -> Self {
        Self {
            rest_client,
            state: Arc::default(),
        }
    }

    /// The current thread policy, refreshed when the cached value is older
    /// than [`THREAD_POLICY_TTL`].
    ///
    /// A failed refresh keeps the last fetched policy; before any fetch has
    /// succeeded that is the NIP-TP default, `nested`. A refresh that finishes
    /// after a newer one has been applied is discarded, so a slow stale
    /// response cannot undo a policy change.
    pub async fn resolve(&self) -> ThreadPolicy {
        let fetch = match self.begin_refresh() {
            Ok(fetch) => fetch,
            Err(cached) => return cached,
        };
        let fetched = tokio::time::timeout(
            THREAD_POLICY_FETCH_TIMEOUT,
            self.rest_client.thread_policy(),
        )
        .await
        .unwrap_or(Err(RelayError::Timeout));
        self.finish_refresh(fetch, fetched)
    }

    /// Start a refresh and return its number, or `Err` with the cached policy
    /// while it is still fresh.
    fn begin_refresh(&self) -> Result<u64, ThreadPolicy> {
        let mut state = self.lock();
        if state
            .checked_at
            .is_some_and(|checked_at| checked_at.elapsed() < THREAD_POLICY_TTL)
        {
            return Err(state.policy);
        }
        state.started += 1;
        Ok(state.started)
    }

    /// Record the result of refresh `fetch` and return the current policy.
    fn finish_refresh(
        &self,
        fetch: u64,
        fetched: Result<ThreadPolicy, RelayError>,
    ) -> ThreadPolicy {
        let mut state = self.lock();
        match fetched {
            Ok(policy) if fetch > state.applied => {
                state.policy = policy;
                state.applied = fetch;
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(
                %error,
                policy = ?state.policy,
                "thread policy refresh failed; keeping last known policy"
            ),
        }
        state.checked_at = Some(Instant::now());
        state.policy
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> ThreadPolicyResolver {
        ThreadPolicyResolver::new(RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:0".to_string(),
            keys: nostr::Keys::generate(),
            auth_tag_json: None,
        })
    }

    fn expire(resolver: &ThreadPolicyResolver) {
        resolver.lock().checked_at = None;
    }

    #[tokio::test]
    async fn failed_refresh_keeps_last_known_policy_else_nested() {
        let resolver = resolver();
        // The relay is unreachable before any fetch succeeded: nested.
        assert_eq!(resolver.resolve().await, ThreadPolicy::Nested);

        expire(&resolver);
        let fetch = resolver.begin_refresh().expect("stale cache refreshes");
        assert_eq!(
            resolver.finish_refresh(fetch, Ok(ThreadPolicy::Flat)),
            ThreadPolicy::Flat
        );
        // Within the TTL the cached policy is served without a fetch.
        assert_eq!(resolver.begin_refresh(), Err(ThreadPolicy::Flat));

        // A later failure keeps the last fetched policy.
        expire(&resolver);
        assert_eq!(resolver.resolve().await, ThreadPolicy::Flat);
    }

    #[test]
    fn older_refresh_finishing_late_does_not_override_newer_policy() {
        let resolver = resolver();
        let older = resolver.begin_refresh().expect("first refresh");
        let newer = resolver.begin_refresh().expect("concurrent refresh");
        assert_eq!(
            resolver.finish_refresh(newer, Ok(ThreadPolicy::Flat)),
            ThreadPolicy::Flat
        );
        assert_eq!(
            resolver.finish_refresh(older, Ok(ThreadPolicy::Nested)),
            ThreadPolicy::Flat
        );
    }
}
