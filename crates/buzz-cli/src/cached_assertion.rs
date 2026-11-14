//! The one cached NIP-FI assertion an evidence link holds for its relay,
//! shared by the headless adapter session and the Buzz desktop link: one
//! `exp` parser and one refresh rule.

use tokio::sync::Mutex;

use crate::error::CliError;

/// Refresh at least this long before expiry.
const REFRESH_MARGIN_SECS: u64 = 30;

/// An evidence link's current assertion, or its answer that it has none.
#[derive(Default)]
pub(crate) struct CachedAssertion(Mutex<Option<(Option<String>, u64)>>);

impl CachedAssertion {
    /// The cached answer until its refresh point, else `fetch`'s. A fetched
    /// assertion must be a compact JWS with an `exp`; `None` is kept until
    /// the next command. Concurrent callers share one fetch.
    pub(crate) async fn get<F>(&self, fetch: F) -> Result<Option<String>, CliError>
    where
        F: std::future::Future<Output = Result<Option<String>, CliError>>,
    {
        let mut cached = self.0.lock().await;
        if let Some((assertion, refresh)) = cached.as_ref() {
            if unix_now() < *refresh {
                return Ok(assertion.clone());
            }
        }
        let assertion = fetch.await?;
        let refresh = match assertion.as_deref() {
            Some(jws) => refresh_at(
                unix_now(),
                jws_exp(jws).ok_or_else(|| {
                    CliError::Auth("the evidence source returned an unreadable assertion".into())
                })?,
            ),
            None => u64::MAX,
        };
        *cached = Some((assertion.clone(), refresh));
        Ok(assertion)
    }
}

/// Refresh at the earlier of half-life and [`REFRESH_MARGIN_SECS`] before
/// `exp`; when that margin is already past, at half-life, so a short-lived
/// assertion is not fetched again on every request.
pub(crate) fn refresh_at(now: u64, exp: u64) -> u64 {
    let half_life = now + exp.saturating_sub(now) / 2;
    match exp.saturating_sub(REFRESH_MARGIN_SECS) {
        early if early > now => early.min(half_life),
        _ => half_life,
    }
}

/// The `exp` claim of a structurally valid compact JWS: three non-empty
/// unpadded base64url segments, a JSON-object header naming `alg`, and a
/// JSON-object payload with an integer `exp`. The relay verifies the
/// signature; the CLI only refuses output it could not present or refresh.
pub(crate) fn jws_exp(token: &str) -> Option<u64> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let mut segments = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return None;
    };
    let json = |segment: &str| -> Option<serde_json::Map<String, serde_json::Value>> {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segment).ok()?).ok()
    };
    if signature.is_empty() || URL_SAFE_NO_PAD.decode(signature).is_err() {
        return None;
    }
    json(header)?.get("alg")?.as_str()?;
    json(payload)?.get("exp")?.as_u64()
}

pub(crate) fn unix_now() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    #[cfg(test)]
    let now = test_clock::FROZEN.with(std::cell::Cell::get).unwrap_or(now)
        + test_clock::SKEW.with(std::cell::Cell::get);
    now
}

/// A clock tests move forward instead of sleeping. Per thread, so it moves
/// only for the test (and its current-thread runtime) that advances it.
#[cfg(test)]
pub(crate) mod test_clock {
    thread_local! {
        pub(super) static FROZEN: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
        pub(super) static SKEW: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }
    /// Pin this thread's clock to `at`, so reads on either side of a
    /// wall-clock second boundary agree; `advance` still moves it.
    pub(crate) fn freeze(at: u64) {
        FROZEN.with(|frozen| frozen.set(Some(at)));
    }
    pub(crate) fn advance(secs: u64) {
        SKEW.with(|skew| skew.set(skew.get() + secs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_is_at_the_earlier_of_half_life_and_the_margin() {
        assert_eq!(refresh_at(1_000, 1_300), 1_150, "long-lived: half-life");
        assert_eq!(refresh_at(1_000, 1_060), 1_030);
        assert_eq!(refresh_at(1_000, 1_040), 1_010, "short-lived: the margin");
        assert_eq!(refresh_at(1_000, 1_020), 1_010, "margin past: half-life");
        assert_eq!(refresh_at(1_000, 1_001), 1_000);
    }

    #[test]
    fn jws_exp_needs_a_compact_jws_with_alg_and_exp() {
        let payload = "eyJleHAiOjE3OTAwMDAzMDB9"; // {"exp":1790000300}
        let header = "eyJhbGciOiJFUzI1NiJ9"; // {"alg":"ES256"}
        assert_eq!(
            jws_exp(&format!("{header}.{payload}.c2ln")),
            Some(1_790_000_300)
        );
        assert_eq!(jws_exp(&format!("e30.{payload}.c2ln")), None, "no alg");
        assert_eq!(jws_exp(&format!("{header}.{payload}")), None);
        assert_eq!(jws_exp(&format!("{header}.{payload}.c2ln.x")), None);
    }
}
