//! The one cached NIP-FI assertion the headless adapter session holds for its
//! relay: one `exp` parser and one refresh rule. The Buzz desktop link keeps
//! no copy; the desktop app's cache is the only owner of its assertions.

use tokio::sync::Mutex;

use crate::error::CliError;

/// Refresh at least this long before expiry.
const REFRESH_MARGIN_SECS: u64 = 30;

/// Fetches per [`CachedAssertion::get`] before an assertion that arrives with
/// no local lifetime left is reported instead of retried.
const ACQUIRE_ATTEMPTS: usize = 3;

/// A fetched assertion and the monotonic time ([`monotonic_now`]) the
/// request that returned it was sent.
pub(crate) type Fetched = Option<(String, u64)>;

/// An evidence link's current assertion, or its answer that it has none.
#[derive(Default)]
pub(crate) struct CachedAssertion(Mutex<Option<(Option<String>, u64)>>);

impl CachedAssertion {
    /// The cached answer until its refresh point, else `fetch`'s. A fetched
    /// assertion must be a compact JWS with `exp` and `iat`, and must have
    /// local lifetime left; one with none is fetched again, a bounded number
    /// of times. `None` is kept until the next command. Concurrent callers
    /// share one fetch.
    pub(crate) async fn get<F, Fut>(&self, fetch: F) -> Result<Option<String>, CliError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<Fetched, CliError>>,
    {
        let mut cached = self.0.lock().await;
        if let Some((assertion, refresh)) = cached.as_ref() {
            if monotonic_now() < *refresh {
                return Ok(assertion.clone());
            }
        }
        *cached = None;
        for _ in 0..ACQUIRE_ATTEMPTS {
            let Some((jws, request_start)) = fetch().await? else {
                *cached = Some((None, u64::MAX));
                return Ok(None);
            };
            let claims = jws_claims(&jws)
                .filter(|c| c.iat.is_some())
                .ok_or_else(|| {
                    CliError::Auth("the evidence source returned an unreadable assertion".into())
                })?;
            let receipt = monotonic_now();
            let deadline = local_deadline(request_start, receipt, unix_now(), claims);
            if deadline > receipt {
                *cached = Some((Some(jws.clone()), refresh_at(receipt, deadline)));
                return Ok(Some(jws));
            }
        }
        Err(CliError::Auth(format!(
            "the evidence source returned {ACQUIRE_ATTEMPTS} assertions with no lifetime left; \
             check that this computer's clock is correct"
        )))
    }
}

/// The local monotonic deadline of an assertion whose request was sent at
/// `request_start` and answered at `receipt` (monotonic seconds), with the
/// wall clock at `wall_at_receipt`: the issuer's lifetime counted from the
/// send, capped by `exp` against the local wall clock. The first term keeps a
/// skewed local clock from stretching the token; the second keeps a short
/// session-capped token from outliving its `exp`. A deadline at or before
/// `receipt` means no lifetime is left. `claims.iat` must be present.
fn local_deadline(request_start: u64, receipt: u64, wall_at_receipt: u64, claims: Claims) -> u64 {
    let by_exp = receipt + claims.exp.saturating_sub(wall_at_receipt);
    let by_lifetime = request_start + claims.exp.saturating_sub(claims.iat.unwrap_or(claims.exp));
    by_exp.min(by_lifetime)
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
    jws_claims(token).map(|c| c.exp)
}

/// The `exp` and, when present, `iat` claims of an assertion.
#[derive(Clone, Copy)]
struct Claims {
    exp: u64,
    iat: Option<u64>,
}

/// [`jws_exp`]'s structural check, also returning `iat`.
fn jws_claims(token: &str) -> Option<Claims> {
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
    let payload = json(payload)?;
    Some(Claims {
        exp: payload.get("exp")?.as_u64()?,
        iat: payload.get("iat").and_then(serde_json::Value::as_u64),
    })
}

/// Whole seconds on a monotonic clock, for local deadlines a wall-clock
/// change cannot move.
pub(crate) fn monotonic_now() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let now = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs();
    #[cfg(test)]
    let now = now + test_clock::SKEW.with(std::cell::Cell::get);
    now
}

pub(crate) fn unix_now() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    #[cfg(test)]
    let now = now + test_clock::SKEW.with(std::cell::Cell::get);
    now
}

/// A clock tests move forward instead of sleeping. Per thread, so it moves
/// only for the test (and its current-thread runtime) that advances it.
#[cfg(test)]
pub(crate) mod test_clock {
    thread_local! {
        pub(super) static SKEW: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
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
    fn the_deadline_counts_the_lifetime_from_the_request_start() {
        let claims = |iat, exp| Claims {
            exp,
            iat: Some(iat),
        };
        // Issuer clock: iat 10_000, exp 10_180. The reply took 20 s.
        // A local clock 120 s slow would read 180 + 120 s left by `exp`;
        // the lifetime from the request start caps it.
        assert_eq!(local_deadline(0, 20, 9_900, claims(10_000, 10_180)), 180);
        // A local clock 120 s fast: `exp` leaves 40 s past receipt, the tighter bound.
        assert_eq!(local_deadline(0, 20, 10_140, claims(10_000, 10_180)), 60);
        // A short session-capped token: `exp` 30 s after `iat`.
        assert_eq!(local_deadline(0, 20, 10_000, claims(10_000, 10_030)), 30);
        // Already past `exp` locally: no budget left at receipt.
        assert_eq!(local_deadline(0, 20, 10_200, claims(10_000, 10_180)), 20);
        // A 10 s token whose reply took 20 s: exhausted before it arrived.
        assert_eq!(local_deadline(0, 20, 9_900, claims(10_000, 10_010)), 10);
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
