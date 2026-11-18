//! Google's attestation status list: an in-memory snapshot the verifier reads
//! synchronously, and the background refresh that keeps it current.
//!
//! Enrollment never waits on a fetch. The verifier reads whatever snapshot is
//! installed; with none, or one older than the grace period, it fails closed.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::Duration,
};

use async_trait::async_trait;
use rand::RngExt;
use serde::Deserialize;

/// Where Google publishes the status list. See
/// <https://developer.android.com/privacy-and-security/security-key-attestation#certificate_status>.
pub const GOOGLE_STATUS_LIST_URL: &str = "https://android.googleapis.com/attestation/status";

/// Largest status list body accepted. The published list is a few hundred KiB.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Refresh pacing when Google's `Cache-Control` is missing or out of range.
const DEFAULT_REFRESH_SECONDS: u64 = 3600;
const MIN_REFRESH_SECONDS: u64 = 300;
const MAX_REFRESH_SECONDS: u64 = 86_400;
const BACKOFF_START_SECONDS: u64 = 30;
const BACKOFF_CAP_SECONDS: u64 = 3600;

/// Why a chain could not be checked against the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RevocationFailure {
    /// No list has been fetched yet.
    Unavailable,
    /// The newest list is older than the grace period.
    Stale,
    /// A certificate in the chain appears in the list.
    Listed,
}

struct Snapshot {
    fetched_at: i64,
    serials: HashSet<String>,
}

/// The newest attestation status list and how long it may be relied on.
pub struct RevocationList {
    grace_seconds: i64,
    snapshot: RwLock<Option<Snapshot>>,
}

impl RevocationList {
    /// An empty list. Until [`RevocationList::replace`] runs, every chain
    /// check fails closed.
    pub fn new(grace_seconds: i64) -> Self {
        Self {
            grace_seconds,
            snapshot: RwLock::new(None),
        }
    }

    /// Install a freshly fetched set of listed serials.
    pub fn replace(&self, serials: HashSet<String>, fetched_at: i64) {
        // A poisoned lock still holds a complete snapshot: writes are one swap.
        let mut guard = self
            .snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(Snapshot {
            fetched_at,
            serials,
        });
    }

    /// Fail unless the list is fresh enough and names none of `serials`
    /// (lowercase hex without leading zeros).
    pub(super) fn check(&self, serials: &[String], now: i64) -> Result<(), RevocationFailure> {
        let guard = self
            .snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let snapshot = guard.as_ref().ok_or(RevocationFailure::Unavailable)?;
        if now.saturating_sub(snapshot.fetched_at) > self.grace_seconds {
            return Err(RevocationFailure::Stale);
        }
        if serials
            .iter()
            .any(|serial| snapshot.serials.contains(serial))
        {
            return Err(RevocationFailure::Listed);
        }
        Ok(())
    }
}

/// Normalize a certificate serial or list key to lowercase hex with no
/// leading zeros, the form Google's list uses.
pub(super) fn serial_key(raw: &[u8]) -> String {
    let hex = hex::encode(raw);
    match hex.trim_start_matches('0') {
        "" => "0".to_owned(),
        trimmed => trimmed.to_owned(),
    }
}

#[derive(Deserialize)]
struct StatusDocument {
    entries: HashMap<String, serde::de::IgnoredAny>,
}

/// Decode the status list. Every entry is a certificate Google no longer
/// vouches for (REVOKED or SUSPENDED), so any listed serial is rejected
/// whatever its status or expiry says.
pub fn parse_status_list(body: &[u8]) -> Result<HashSet<String>, FetchError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(FetchError::TooLarge);
    }
    let document: StatusDocument =
        serde_json::from_slice(body).map_err(|_| FetchError::Malformed)?;
    document
        .entries
        .into_keys()
        .map(|key| {
            let lower = key.to_ascii_lowercase();
            let valid = !lower.is_empty() && lower.bytes().all(|b| b.is_ascii_hexdigit());
            let trimmed = lower.trim_start_matches('0');
            valid
                .then(|| {
                    if trimmed.is_empty() {
                        "0".to_owned()
                    } else {
                        trimmed.to_owned()
                    }
                })
                .ok_or(FetchError::Malformed)
        })
        .collect()
}

/// Why a refresh did not produce a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The request failed or timed out.
    #[error("status list request failed")]
    Request,
    /// Google answered with a non-success status.
    #[error("status list request was refused")]
    Refused,
    /// The body exceeded the size limit.
    #[error("status list is too large")]
    TooLarge,
    /// The body was not a status list.
    #[error("status list is malformed")]
    Malformed,
}

/// One successful download.
pub struct StatusDownload {
    pub body: Vec<u8>,
    /// Seconds from Google's `Cache-Control: max-age`, when present.
    pub max_age: Option<u64>,
}

/// The network boundary of the refresh, replaced by a fake in tests.
#[async_trait]
pub trait StatusListSource: Send + Sync {
    /// Download the status list once.
    async fn fetch(&self) -> Result<StatusDownload, FetchError>;
}

/// Downloads the list over HTTPS with a bounded time and size.
pub struct HttpStatusListSource {
    client: reqwest::Client,
    url: String,
}

impl HttpStatusListSource {
    /// A source for [`GOOGLE_STATUS_LIST_URL`].
    pub fn google() -> Result<Self, FetchError> {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|_| FetchError::Request)?;
        Ok(Self {
            client,
            url: GOOGLE_STATUS_LIST_URL.to_owned(),
        })
    }
}

fn max_age(value: &str) -> Option<u64> {
    value
        .split(',')
        .find_map(|directive| directive.trim().strip_prefix("max-age="))
        .and_then(|seconds| seconds.trim().parse().ok())
}

#[async_trait]
impl StatusListSource for HttpStatusListSource {
    async fn fetch(&self) -> Result<StatusDownload, FetchError> {
        let mut response = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|_| FetchError::Request)?;
        if !response.status().is_success() {
            return Err(FetchError::Refused);
        }
        let max_age = response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .and_then(max_age);
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| FetchError::Request)? {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(FetchError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(StatusDownload { body, max_age })
    }
}

impl RevocationList {
    /// The longest wait between refresh attempts that still leaves room inside
    /// the grace period for jitter (up to 10%), the fetch timeout and a retry,
    /// so a list refreshed on schedule never goes stale.
    fn longest_refresh_wait(&self) -> u64 {
        u64::try_from(self.grace_seconds / 2)
            .unwrap_or(0)
            .saturating_sub(FETCH_TIMEOUT.as_secs())
            .max(1)
    }
}

/// One refresh attempt: fetch, parse, and install on success. The previous
/// snapshot stays in place on failure. Returns the seconds to wait before the
/// next attempt, before jitter, never longer than the grace period allows.
pub async fn refresh_once(
    list: &RevocationList,
    source: &dyn StatusListSource,
    now: i64,
    consecutive_failures: &mut u32,
) -> u64 {
    let wait =
        match source.fetch().await.and_then(|download| {
            parse_status_list(&download.body).map(|set| (set, download.max_age))
        }) {
            Ok((serials, max_age)) => {
                *consecutive_failures = 0;
                list.replace(serials, now);
                max_age
                    .unwrap_or(DEFAULT_REFRESH_SECONDS)
                    .clamp(MIN_REFRESH_SECONDS, MAX_REFRESH_SECONDS)
            }
            Err(error) => {
                *consecutive_failures = consecutive_failures.saturating_add(1);
                tracing::warn!(
                    operation = "attestation_status_list",
                    %error,
                    failures = *consecutive_failures,
                    "Android attestation status list refresh failed"
                );
                BACKOFF_START_SECONDS
                    .saturating_mul(1u64 << (*consecutive_failures - 1).min(16))
                    .min(BACKOFF_CAP_SECONDS)
            }
        };
    wait.min(list.longest_refresh_wait())
}

/// Refresh the list forever: immediately, then on Google's cache lifetime, with
/// capped exponential backoff after failures and up to 10% jitter so replicas
/// do not fetch in step.
pub async fn run_refresh_loop(
    list: Arc<RevocationList>,
    source: Arc<dyn StatusListSource>,
    now: fn() -> i64,
) {
    let mut failures = 0;
    loop {
        let wait = refresh_once(&list, source.as_ref(), now(), &mut failures).await;
        let jitter = rand::rng().random_range(0..=wait / 10);
        tokio::time::sleep(Duration::from_secs(wait + jitter)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Replays scripted downloads in order.
    struct Script(Mutex<Vec<Result<StatusDownload, FetchError>>>);

    #[async_trait]
    impl StatusListSource for Script {
        async fn fetch(&self) -> Result<StatusDownload, FetchError> {
            self.0.lock().unwrap().remove(0)
        }
    }

    fn download(body: &str, max_age: Option<u64>) -> Result<StatusDownload, FetchError> {
        Ok(StatusDownload {
            body: body.as_bytes().to_vec(),
            max_age,
        })
    }

    const LIST: &str = r#"{"entries": {
        "2C8CDDDFD5E03BFC": {"status": "REVOKED", "reason": "KEY_COMPROMISE", "expires": "2030-01-01"},
        "c8966fcb2fbb0d7a": {"status": "SUSPENDED"},
        "6681152659205225093": {"status": "REVOKED"}
    }}"#;

    #[test]
    fn status_list_parsing_normalizes_serials_and_rejects_everything_else() {
        let serials = parse_status_list(LIST.as_bytes()).unwrap();
        assert_eq!(serials.len(), 3);
        // Leading zeros and case do not change a serial's identity.
        for serial in [
            &[0, 0x2c, 0x8c, 0xdd, 0xdf, 0xd5, 0xe0, 0x3b, 0xfc][..],
            &[0xc8, 0x96, 0x6f, 0xcb, 0x2f, 0xbb, 0x0d, 0x7a],
        ] {
            assert!(serials.contains(&serial_key(serial)));
        }
        for body in [
            "".as_bytes(),
            b"{}",
            br#"{"entries": []}"#,
            br#"{"entries": {"not hex": {"status": "REVOKED"}}}"#,
            br#"{"entries": {"": {"status": "REVOKED"}}}"#,
        ] {
            assert_eq!(parse_status_list(body), Err(FetchError::Malformed));
        }
        assert_eq!(
            parse_status_list(&vec![b' '; MAX_BODY_BYTES + 1]),
            Err(FetchError::TooLarge)
        );
        assert!(parse_status_list(br#"{"entries": {}}"#).unwrap().is_empty());
    }

    #[tokio::test]
    async fn refresh_installs_good_lists_and_keeps_the_last_one_through_failures() {
        let list = RevocationList::new(200_000);
        let source = Script(Mutex::new(vec![
            download(LIST, Some(60)),
            Err(FetchError::Request),
            download("not json", None),
            Err(FetchError::Refused),
            download(r#"{"entries": {}}"#, Some(999_999)),
        ]));
        let mut failures = 0;
        let listed = [serial_key(&[
            0x2c, 0x8c, 0xdd, 0xdf, 0xd5, 0xe0, 0x3b, 0xfc,
        ])];

        // Google's cache lifetime paces refreshes, within sane bounds.
        assert_eq!(
            refresh_once(&list, &source, 1_000, &mut failures).await,
            MIN_REFRESH_SECONDS
        );
        assert_eq!(list.check(&listed, 1_000), Err(RevocationFailure::Listed));

        // Failures back off with a cap and never discard the installed list.
        let mut waits = Vec::new();
        for _ in 0..3 {
            waits.push(refresh_once(&list, &source, 1_050, &mut failures).await);
        }
        assert_eq!(waits, [30, 60, 120]);
        assert_eq!(list.check(&listed, 1_050), Err(RevocationFailure::Listed));
        // Past the grace period without a fresh list, checks fail closed.
        assert_eq!(list.check(&[], 201_001), Err(RevocationFailure::Stale));

        assert_eq!(
            refresh_once(&list, &source, 1_200, &mut failures).await,
            MAX_REFRESH_SECONDS
        );
        assert_eq!(failures, 0);
        assert_eq!(list.check(&listed, 1_200), Ok(()));
        assert_eq!(
            RevocationList::new(1).check(&[], 0),
            Err(RevocationFailure::Unavailable)
        );

        // A short grace period caps the wait, even when Google's cache
        // lifetime is longer, so a list refreshed on schedule never goes stale.
        let short = RevocationList::new(3600);
        let source = Script(Mutex::new(vec![
            download(LIST, Some(3600)),
            download(LIST, None),
            download(LIST, Some(86_400)),
        ]));
        let mut failures = 0;
        for _ in 0..3 {
            let wait = refresh_once(&short, &source, 0, &mut failures).await;
            assert_eq!(wait, 1770);
            assert!(wait + wait / 10 + FETCH_TIMEOUT.as_secs() < 3600);
        }
    }

    #[tokio::test]
    async fn backoff_is_capped_and_cache_control_is_read() {
        let list = RevocationList::new(604_800);
        let mut failures = 40;
        let source = Script(Mutex::new(vec![Err(FetchError::Request)]));
        let wait = refresh_once(&list, &source, 0, &mut failures).await;
        assert_eq!(wait, BACKOFF_CAP_SECONDS);
        assert_eq!(max_age("public, max-age=86400"), Some(86_400));
        assert_eq!(max_age("no-store"), None);
        assert_eq!(max_age("max-age=soon"), None);
    }
}
