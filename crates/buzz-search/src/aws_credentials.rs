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

/// After a failed reload, keep using credentials that are still valid for
/// this long before trying again.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(30);

/// Bound on one load, so a stalled credential source cannot hold requests.
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

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
/// they expire. Concurrent requests share one load.
pub(crate) struct CachedCredentials<P> {
    source: P,
    /// The credentials and when to reload them.
    cached: Mutex<Option<(Credentials, SystemTime)>>,
}

impl<P> CachedCredentials<P> {
    pub(crate) fn new(source: P) -> Self {
        Self {
            source,
            cached: Mutex::new(None),
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
        let mut cached = self.cached.lock().await;
        let now = SystemTime::now();
        if let Some((credentials, reload_at)) = cached.as_ref() {
            if now < *reload_at {
                return Ok(credentials.clone());
            }
        }
        let error =
            match tokio::time::timeout(LOAD_TIMEOUT, self.source.provide_credentials()).await {
                Ok(Ok(credentials)) => {
                    let reload_at = match credentials.expiry() {
                        Some(expiry) => expiry
                            .checked_sub(REFRESH_BEFORE_EXPIRY)
                            .unwrap_or(SystemTime::UNIX_EPOCH),
                        None => now + WITHOUT_EXPIRY_TTL,
                    };
                    *cached = Some((credentials.clone(), reload_at));
                    return Ok(credentials);
                }
                Ok(Err(error)) => error,
                Err(_) => CredentialsError::provider_timed_out(LOAD_TIMEOUT),
            };
        tracing::warn!(
            error = %with_sources(&error),
            "loading AWS credentials for OpenSearch failed"
        );
        // A failed reload is no reason to stop while the old set still works.
        if let Some((credentials, reload_at)) = cached.as_mut() {
            let expiry = credentials.expiry();
            if expiry.is_none_or(|expiry| now < expiry) {
                let retry_at = now + RETRY_AFTER_FAILURE;
                *reload_at = expiry.map_or(retry_at, |expiry| retry_at.min(expiry));
                return Ok(credentials.clone());
            }
        }
        Err(error)
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

    /// Hands out credentials valid for `lifetime`, counting the loads.
    #[derive(Debug)]
    struct Counting {
        lifetime: Duration,
        loads: AtomicUsize,
    }

    impl ProvideCredentials for Counting {
        fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
        where
            Self: 'a,
        {
            self.loads.fetch_add(1, Ordering::SeqCst);
            future::ProvideCredentials::ready(Ok(Credentials::new(
                "access",
                "secret",
                None,
                Some(SystemTime::now() + self.lifetime),
                "test",
            )))
        }
    }

    async fn loads_for_two_requests(lifetime: Duration) -> usize {
        let cached = CachedCredentials::new(Counting {
            lifetime,
            loads: AtomicUsize::new(0),
        });
        for _ in 0..2 {
            cached.provide_credentials().await.expect("credentials");
        }
        cached.source.loads.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn credentials_are_reused_until_they_near_expiry() {
        assert_eq!(loads_for_two_requests(Duration::from_secs(3600)).await, 1);
        assert_eq!(loads_for_two_requests(Duration::from_secs(60)).await, 2);
    }
}
