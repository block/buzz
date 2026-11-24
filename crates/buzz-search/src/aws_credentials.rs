//! AWS credentials for signing OpenSearch requests.
//!
//! The OpenSearch client asks its credentials provider on every request, and
//! the AWS default chain does not cache: used directly, each search and bulk
//! write would call the token service or the instance metadata endpoint.
//! [`CachedCredentials`] keeps one set and reloads it shortly before it
//! expires, so a long-running relay follows role rotation.

use std::error::Error;
use std::time::{Duration, SystemTime};

use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_config::Region;
use aws_credential_types::provider::error::CredentialsError;
use aws_credential_types::provider::{self, future, ProvideCredentials};
use aws_credential_types::Credentials;
use tokio::sync::{Mutex, OnceCell};

/// Reload this long before the credentials expire.
const REFRESH_BEFORE_EXPIRY: Duration = Duration::from_secs(5 * 60);

/// Reload credentials that carry no expiry this often, so a rotated key in
/// the environment's files is picked up.
const WITHOUT_EXPIRY_TTL: Duration = Duration::from_secs(15 * 60);

/// The shortest time between two loads, after a success or a failure. A
/// source that is down, or that hands out credentials already near their
/// expiry, is asked this often and not once per request.
const MIN_LOAD_INTERVAL: Duration = Duration::from_secs(30);

/// Bound on a load when there are no valid credentials to fall back on, so
/// a stalled credential source cannot hold requests.
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Bound on a reload while the current credentials are still valid: the
/// request goes on with those instead of waiting.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

/// The AWS default credential chain: environment, shared profile, web
/// identity token, container role, then instance role. Built on first use.
pub(crate) struct DefaultChain {
    region: Region,
    chain: OnceCell<DefaultCredentialsChain>,
}

impl DefaultChain {
    pub(crate) fn new(region: Region) -> Self {
        Self {
            region,
            chain: OnceCell::new(),
        }
    }
}

impl std::fmt::Debug for DefaultChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultChain").finish_non_exhaustive()
    }
}

impl ProvideCredentials for DefaultChain {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(async {
            let chain = self
                .chain
                .get_or_init(|| {
                    DefaultCredentialsChain::builder()
                        .region(self.region.clone())
                        .build()
                })
                .await;
            chain.provide_credentials().await
        })
    }
}

/// Credentials from `source`, loaded once and reused until shortly before
/// they expire. Concurrent requests share one load, whether it succeeds or
/// fails.
pub(crate) struct CachedCredentials<P> {
    source: P,
    state: Mutex<State>,
}

struct State {
    /// The last credentials loaded.
    credentials: Option<Credentials>,
    /// No load before this time.
    next_load: SystemTime,
}

impl<P> CachedCredentials<P> {
    pub(crate) fn new(source: P) -> Self {
        Self {
            source,
            state: Mutex::new(State {
                credentials: None,
                next_load: SystemTime::UNIX_EPOCH,
            }),
        }
    }
}

impl<P> std::fmt::Debug for CachedCredentials<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedCredentials").finish_non_exhaustive()
    }
}

impl<P: ProvideCredentials> CachedCredentials<P> {
    async fn credentials(&self) -> provider::Result {
        let mut state = self.state.lock().await;
        let now = SystemTime::now();
        let valid = state
            .credentials
            .clone()
            .filter(|credentials| credentials.expiry().is_none_or(|expiry| now < expiry));
        if now < state.next_load {
            return valid.ok_or_else(|| {
                CredentialsError::not_loaded("loading AWS credentials failed moments ago")
            });
        }
        let timeout = if valid.is_some() {
            REFRESH_TIMEOUT
        } else {
            LOAD_TIMEOUT
        };
        let earliest = now + MIN_LOAD_INTERVAL;
        let error = match tokio::time::timeout(timeout, self.source.provide_credentials()).await {
            Ok(Ok(credentials)) => {
                state.next_load = match credentials.expiry() {
                    Some(expiry) => expiry
                        .checked_sub(REFRESH_BEFORE_EXPIRY)
                        .map_or(earliest, |refresh| refresh.max(earliest))
                        .min(expiry),
                    None => now + WITHOUT_EXPIRY_TTL,
                };
                state.credentials = Some(credentials.clone());
                return Ok(credentials);
            }
            Ok(Err(error)) => error,
            Err(_) => CredentialsError::provider_timed_out(timeout),
        };
        tracing::warn!(
            error = %with_sources(&error),
            "loading AWS credentials for OpenSearch failed"
        );
        // A failed reload is no reason to stop while the old set still works.
        // Without one, requests fail at once until the next load is due.
        let expiry = valid.as_ref().and_then(Credentials::expiry);
        state.next_load = expiry.map_or(earliest, |expiry| earliest.min(expiry));
        valid.ok_or(error)
    }
}

impl<P: ProvideCredentials> ProvideCredentials for CachedCredentials<P> {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(self.credentials())
    }
}

/// An error's message followed by those of its causes. Credential errors
/// keep the useful part (which source failed, and why) in their causes.
fn with_sources(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Succeeds `successes` times with hour-long credentials, then fails,
    /// counting the loads.
    #[derive(Debug)]
    struct Source {
        successes: usize,
        loads: AtomicUsize,
    }

    impl ProvideCredentials for Source {
        fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
        where
            Self: 'a,
        {
            let load = self.loads.fetch_add(1, Ordering::SeqCst);
            future::ProvideCredentials::ready(if load < self.successes {
                Ok(Credentials::new(
                    format!("access-{load}"),
                    "secret",
                    None,
                    Some(SystemTime::now() + Duration::from_secs(3600)),
                    "test",
                ))
            } else {
                Err(CredentialsError::not_loaded("source is down"))
            })
        }
    }

    fn cache(successes: usize) -> CachedCredentials<Source> {
        CachedCredentials::new(Source {
            successes,
            loads: AtomicUsize::new(0),
        })
    }

    async fn access_key(cached: &CachedCredentials<Source>) -> Option<String> {
        let credentials = cached.provide_credentials().await.ok()?;
        Some(credentials.access_key_id().to_string())
    }

    /// Make the next request reload, as when the credentials near expiry.
    async fn reload_is_due(cached: &CachedCredentials<Source>) {
        cached.state.lock().await.next_load = SystemTime::UNIX_EPOCH;
    }

    #[tokio::test]
    async fn credentials_are_loaded_once_and_survive_a_failed_reload() {
        let cached = cache(2);
        assert_eq!(access_key(&cached).await.as_deref(), Some("access-0"));
        assert_eq!(access_key(&cached).await.as_deref(), Some("access-0"));
        reload_is_due(&cached).await;
        assert_eq!(access_key(&cached).await.as_deref(), Some("access-1"));

        // The source goes down: the valid set stays in use, and the source
        // is not asked again on the next request.
        reload_is_due(&cached).await;
        assert_eq!(access_key(&cached).await.as_deref(), Some("access-1"));
        assert_eq!(access_key(&cached).await.as_deref(), Some("access-1"));
        assert_eq!(cached.source.loads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_failed_first_load_is_not_repeated_per_request() {
        let cached = cache(0);
        assert_eq!(access_key(&cached).await, None);
        assert_eq!(access_key(&cached).await, None);
        assert_eq!(cached.source.loads.load(Ordering::SeqCst), 1);
    }
}
