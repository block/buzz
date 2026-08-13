use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

use reqwest::header::RETRY_AFTER;
use url::Url;

pub(super) const MAX_IMAGE_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);
const MAX_IMAGE_HOST_COOLDOWNS: usize = 128;

static IMAGE_HOST_COOLDOWNS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

pub(super) fn retry_after_duration(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after)
        .map(|duration| duration.min(MAX_IMAGE_RETRY_AFTER))
}

/// Parse a `Retry-After` header value. RFC 9110 §10.2.3 permits either a
/// non-negative delta-seconds count or an HTTP-date; a server may send either,
/// so we accept both. An HTTP-date is converted to a delay from now, floored at
/// zero (a date already in the past means "retry immediately").
fn parse_retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = httpdate::parse_http_date(value).ok()?;
    Some(
        retry_at
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

/// Marker prefix the caller matches to distinguish a rate limit (429) from an
/// ordinary transient blip. When the server supplied a Retry-After, its whole
/// seconds are appended so the caller can wait exactly that long instead of
/// blindly retrying or falling back to the default transient window.
pub(super) const RATE_LIMITED_ERROR_PREFIX: &str = "link preview rate limited";

/// Marker prefix for a permanent validation/policy rejection (non-HTTPS or
/// credentialed URL, non-default port, a host that resolves to a private or
/// reserved address, or an unparseable URL/redirect target). These are NOT
/// transient: an inline retry or periodic self-heal poll would only repeat the
/// same rejected work, so the caller must cache them as a hard miss. Kept in
/// sync with PERMANENTLY_REJECTED_ERROR_PREFIX in useResolvedLinkPreviews.ts.
pub(super) const PERMANENTLY_REJECTED_ERROR_PREFIX: &str = "link preview rejected";

/// Tag a permanent rejection with [`PERMANENTLY_REJECTED_ERROR_PREFIX`] so the
/// caller can distinguish it from a transient failure and skip all retries.
pub(super) fn permanently_rejected_error(reason: &str) -> String {
    format!("{PERMANENTLY_REJECTED_ERROR_PREFIX}: {reason}")
}

pub(super) fn rate_limited_error(retry_after: Option<Duration>) -> String {
    match retry_after {
        Some(retry_after) => {
            format!(
                "{RATE_LIMITED_ERROR_PREFIX}: retry-after {}",
                retry_after.as_secs()
            )
        }
        None => RATE_LIMITED_ERROR_PREFIX.to_string(),
    }
}

/// A 429 is a genuine rate limit: an immediate retry would just be throttled
/// again, so surface it distinctly (carrying any Retry-After) rather than as an
/// ordinary transient blip. Returns `None` for every other status so the caller
/// keeps its usual transient/hard-miss handling.
pub(super) fn rate_limited_response_error(response: &reqwest::Response) -> Option<String> {
    (response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS)
        .then(|| rate_limited_error(retry_after_duration(response)))
}

/// A retryable HTTP status: rate limit, request timeout, too early, or any
/// server error. These are transient and should be retried soon rather than
/// cached as a genuine "no metadata" miss.
pub(super) fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_EARLY
        || status.is_server_error()
}

pub(super) fn image_host_cooldown_remaining(url: &Url) -> Option<Duration> {
    let host = url.host_str()?;
    let cooldowns = IMAGE_HOST_COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut cooldowns) = cooldowns.lock() else {
        return None;
    };
    let expires_at = cooldowns.get(host).copied()?;
    let now = Instant::now();
    if expires_at <= now {
        cooldowns.remove(host);
        return None;
    }
    Some(expires_at.duration_since(now))
}

pub(super) fn set_image_host_cooldown(url: &Url, retry_after: Duration) {
    let Some(host) = url.host_str() else {
        return;
    };
    let now = Instant::now();
    let Some(expires_at) = now.checked_add(retry_after) else {
        return;
    };
    let cooldowns = IMAGE_HOST_COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut cooldowns) = cooldowns.lock() {
        cooldowns.retain(|_, current_expiry| *current_expiry > now);
        if cooldowns.len() >= MAX_IMAGE_HOST_COOLDOWNS && !cooldowns.contains_key(host) {
            let oldest_host = cooldowns
                .iter()
                .min_by_key(|(_, current_expiry)| *current_expiry)
                .map(|(current_host, _)| current_host.clone());
            if let Some(oldest_host) = oldest_host {
                cooldowns.remove(&oldest_host);
            }
        }
        cooldowns.insert(host.to_string(), expires_at);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_retry_after, permanently_rejected_error, rate_limited_error,
        PERMANENTLY_REJECTED_ERROR_PREFIX, RATE_LIMITED_ERROR_PREFIX,
    };
    use std::time::{Duration, SystemTime};

    #[test]
    fn parse_retry_after_reads_delta_seconds() {
        // The common form: a non-negative integer count of seconds.
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("  30 "), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
    }

    #[test]
    fn parse_retry_after_reads_http_date() {
        // RFC 9110 also permits an HTTP-date; a future date parses to the delay
        // from now. Allow slack for the second that elapses during the test.
        let value = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(300));
        let parsed = parse_retry_after(&value).expect("http-date should parse");
        assert!(
            parsed <= Duration::from_secs(300) && parsed >= Duration::from_secs(295),
            "expected ~300s, got {parsed:?}",
        );
    }

    #[test]
    fn parse_retry_after_floors_past_http_date_at_zero() {
        // A date already in the past means "retry immediately" — never a
        // negative or wildly large duration from an underflow.
        let value = httpdate::fmt_http_date(SystemTime::now() - Duration::from_secs(300));
        assert_eq!(parse_retry_after(&value), Some(Duration::ZERO));
    }

    #[test]
    fn parse_retry_after_rejects_garbage() {
        assert_eq!(parse_retry_after("soon"), None);
        assert_eq!(parse_retry_after(""), None);
        // A negative value is neither valid delta-seconds nor an HTTP-date.
        assert_eq!(parse_retry_after("-5"), None);
    }

    #[test]
    fn permanently_rejected_error_carries_marker() {
        // A permanent validation/policy rejection surfaces a distinct marker so
        // the caller skips every retry and caches it as a hard miss.
        assert_eq!(
            permanently_rejected_error("host resolved to a private or reserved address"),
            format!(
                "{PERMANENTLY_REJECTED_ERROR_PREFIX}: host resolved to a private or reserved address"
            ),
        );
    }

    #[test]
    fn rate_limited_error_carries_retry_after_seconds() {
        // A rate limit surfaces a distinct marker so the caller skips the fast
        // inline retry; when the server gave a Retry-After we append its whole
        // seconds so the caller can wait exactly that long.
        assert_eq!(
            rate_limited_error(Some(Duration::from_secs(45))),
            format!("{RATE_LIMITED_ERROR_PREFIX}: retry-after 45"),
        );
        // Sub-second remainders truncate to whole seconds.
        assert_eq!(
            rate_limited_error(Some(Duration::from_millis(1_500))),
            format!("{RATE_LIMITED_ERROR_PREFIX}: retry-after 1"),
        );
        // No Retry-After: bare marker, caller falls back to its default window.
        assert_eq!(
            rate_limited_error(None),
            RATE_LIMITED_ERROR_PREFIX.to_string(),
        );
    }
}
