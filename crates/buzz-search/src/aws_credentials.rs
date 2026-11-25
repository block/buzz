//! AWS credentials for signing OpenSearch requests.
//!
//! The OpenSearch client asks its credentials provider on every request, and
//! the AWS default chain does not cache: used directly, each search and bulk
//! write would call the token service or the instance metadata endpoint.
//! [`CachedCredentials`] keeps one set and reloads it shortly before it
//! expires, so a long-running relay follows role rotation.

use std::future::Future;
use std::time::{Duration, SystemTime};

use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_config::Region;
use aws_credential_types::provider::error::CredentialsError;
use aws_credential_types::provider::{self, future, ProvideCredentials};
use aws_credential_types::Credentials;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Reload this long before the credentials expire.
const REFRESH_BEFORE_EXPIRY: Duration = Duration::from_secs(5 * 60);

/// Reload credentials that carry no expiry this often, so a rotated key in
/// the environment's files is picked up.
const WITHOUT_EXPIRY_TTL: Duration = Duration::from_secs(15 * 60);

/// The shortest time from the end of one load to the start of the next,
/// after a success or a failure. A source that is down, or that hands out
/// credentials already near their expiry, is asked this often and not once
/// per request.
const MIN_LOAD_INTERVAL: Duration = Duration::from_secs(30);

/// Bound on a load when there are no valid credentials to fall back on, so
/// a stalled credential source cannot hold requests.
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Bound on a reload while the current credentials are still valid: the
/// request goes on with those instead of waiting.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

/// The AWS default credential chain: environment, shared profile, web
/// identity token, container role, then instance role.
pub(crate) fn default_chain(region: Region) -> impl ProvideCredentials {
    Rebuilt {
        build: move || {
            DefaultCredentialsChain::builder()
                .region(region.clone())
                .build()
        },
    }
}

/// A provider built anew for every load. The AWS providers read the shared
/// profile files once and keep what they read, so one kept across loads
/// would go on signing with a key that has since been rotated out.
struct Rebuilt<B> {
    build: B,
}

impl<B> std::fmt::Debug for Rebuilt<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rebuilt").finish_non_exhaustive()
    }
}

impl<B, F, P> ProvideCredentials for Rebuilt<B>
where
    B: Fn() -> F + Send + Sync,
    F: Future<Output = P> + Send,
    P: ProvideCredentials,
{
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(async {
            let provider = (self.build)().await;
            provider.provide_credentials().await
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
    /// No load before this time. `None` until the first load.
    next_load: Option<Instant>,
}

impl<P> CachedCredentials<P> {
    pub(crate) fn new(source: P) -> Self {
        Self {
            source,
            state: Mutex::new(State {
                credentials: None,
                next_load: None,
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
        if state.next_load.is_some_and(|next| Instant::now() < next) {
            return valid.ok_or_else(|| {
                CredentialsError::not_loaded("loading AWS credentials failed moments ago")
            });
        }
        let timeout = if valid.is_some() {
            REFRESH_TIMEOUT
        } else {
            LOAD_TIMEOUT
        };
        let loaded = tokio::time::timeout(timeout, self.source.provide_credentials()).await;
        // The wait for the next load starts when this one ends: requests
        // that queued behind a slow load must not each start another.
        let until =
            |expiry: SystemTime| expiry.duration_since(SystemTime::now()).unwrap_or_default();
        // A credential helper can hand out a set whose expiry has passed. It
        // signs nothing, so it counts as a failed load and is not asked for
        // again on every request.
        let arrived_expired = matches!(
            &loaded,
            Ok(Ok(credentials)) if credentials.expiry().is_some_and(|expiry| until(expiry).is_zero())
        );
        let error = match loaded {
            Ok(Ok(_)) if arrived_expired => {
                CredentialsError::provider_error("the credentials loaded had already expired")
            }
            Ok(Ok(credentials)) => {
                let wait = match credentials.expiry().map(until) {
                    Some(left) => left
                        .saturating_sub(REFRESH_BEFORE_EXPIRY)
                        .max(MIN_LOAD_INTERVAL)
                        .min(left),
                    None => WITHOUT_EXPIRY_TTL,
                };
                state.next_load = Some(Instant::now() + wait);
                state.credentials = Some(credentials.clone());
                return Ok(credentials);
            }
            Ok(Err(error)) => error,
            Err(_) => CredentialsError::provider_timed_out(timeout),
        };
        // Only the kind of error is logged. Its causes can carry the output
        // of a credential helper, which may hold secrets.
        tracing::warn!(
            %error,
            arrived_expired,
            "loading AWS credentials for OpenSearch failed"
        );
        // A failed reload is no reason to stop while the old set still works.
        // Without one, requests fail at once until the next load is due.
        let wait = match valid.as_ref().and_then(Credentials::expiry).map(until) {
            Some(left) => MIN_LOAD_INTERVAL.min(left),
            None => MIN_LOAD_INTERVAL,
        };
        state.next_load = Some(Instant::now() + wait);
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    // The names that replace these live in a crate this one does not
    // depend on directly.
    #[allow(deprecated)]
    use aws_config::profile::profile_file::{ProfileFileKind, ProfileFiles};
    use aws_config::profile::ProfileFileCredentialsProvider;

    use super::*;

    /// Succeeds `successes` times with credentials that expire at `expiry`,
    /// then fails, counting the loads.
    #[derive(Debug)]
    struct Source {
        successes: usize,
        expiry: SystemTime,
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
                    Some(self.expiry),
                    "test",
                ))
            } else {
                Err(CredentialsError::not_loaded("source is down"))
            })
        }
    }

    fn cache(successes: usize) -> CachedCredentials<Source> {
        cache_expiring(successes, SystemTime::now() + Duration::from_secs(3600))
    }

    fn cache_expiring(successes: usize, expiry: SystemTime) -> CachedCredentials<Source> {
        CachedCredentials::new(Source {
            successes,
            expiry,
            loads: AtomicUsize::new(0),
        })
    }

    async fn access_key(cached: &CachedCredentials<impl ProvideCredentials>) -> Option<String> {
        let credentials = cached.provide_credentials().await.ok()?;
        Some(credentials.access_key_id().to_string())
    }

    /// Make the next request reload, as when the credentials near expiry.
    async fn reload_is_due<P>(cached: &CachedCredentials<P>) {
        cached.state.lock().await.next_load = None;
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

    #[tokio::test]
    async fn credentials_that_arrive_expired_are_not_reloaded_per_request() {
        let cached = cache_expiring(usize::MAX, SystemTime::now() - Duration::from_secs(60));
        assert_eq!(access_key(&cached).await, None);
        assert_eq!(access_key(&cached).await, None);
        assert_eq!(cached.source.loads.load(Ordering::SeqCst), 1);
    }

    /// Never answers, counting the loads.
    #[derive(Debug, Default)]
    struct Stalled {
        loads: AtomicUsize,
    }

    impl ProvideCredentials for Stalled {
        fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
        where
            Self: 'a,
        {
            self.loads.fetch_add(1, Ordering::SeqCst);
            future::ProvideCredentials::new(std::future::pending())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn requests_waiting_on_a_stalled_load_share_its_failure() {
        let cached = CachedCredentials::new(Stalled::default());
        let (first, second) = tokio::join!(access_key(&cached), access_key(&cached));
        assert_eq!((first, second), (None, None));
        assert_eq!(cached.source.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn a_key_rotated_in_the_shared_profile_is_picked_up() {
        let path =
            std::env::temp_dir().join(format!("buzz-search-rotation-{}", std::process::id()));
        let profile = |key: &str| {
            let text = format!("[default]\naws_access_key_id = {key}\naws_secret_access_key = x\n");
            std::fs::write(&path, text).expect("write the profile file");
        };
        let cached = CachedCredentials::new(Rebuilt {
            build: || async {
                let files = ProfileFiles::builder()
                    .with_file(ProfileFileKind::Credentials, &path)
                    .build();
                ProfileFileCredentialsProvider::builder()
                    .profile_files(files)
                    .profile_name("default")
                    .build()
            },
        });

        profile("before");
        assert_eq!(access_key(&cached).await.as_deref(), Some("before"));
        profile("after");
        reload_is_due(&cached).await;
        let rotated = access_key(&cached).await;
        let _ = std::fs::remove_file(&path);
        assert_eq!(rotated.as_deref(), Some("after"));
    }
}
