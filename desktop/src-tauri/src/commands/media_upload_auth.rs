//! Minting Blossom auth for media uploads, and explaining a refused one.
//!
//! An upload token is stamped with this machine's clock and verified against
//! the relay's. When the two disagree by more than media auth tolerates, the
//! upload comes back as a bare `401` that cannot say so — a user-fixable,
//! non-Buzz problem wearing the costume of an authentication failure.
//!
//! After a refusal, a short `HEAD` of the upload route reads the relay's `Date`
//! header and brackets this machine's offset against it. Only that dedicated
//! round trip is short enough to bound: the relay verifies Blossom auth
//! *before* it reads the body, so the `Date` on a large upload's rejection was
//! stamped at the start of a transfer that may have run for minutes. The
//! decision logic lives in [`buzz_core_pkg::clock_skew`], shared with the CLI.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use buzz_core_pkg::clock_skew::ClockOffsetBounds;
use chrono::Utc;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use tokio_util::sync::CancellationToken;

use super::media::extract_server_authority;
use crate::app_state::AppState;

/// Timeout for the clock probe. Short on purpose: it only runs after an upload
/// has already failed, and must not stretch out the failure.
const CLOCK_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Mint the `t=upload` Blossom auth event for a blob.
fn sign_blossom_upload_auth(
    keys: &Keys,
    sha256: &str,
    expiry_secs: u64,
    base_url: &str,
) -> Result<nostr::Event, String> {
    let now = Timestamp::now().as_secs();
    let mut tags = vec![
        Tag::parse(vec!["t", "upload"]).map_err(|e| e.to_string())?,
        Tag::parse(vec!["x", sha256]).map_err(|e| e.to_string())?,
        Tag::parse(vec!["expiration", &(now + expiry_secs).to_string()])
            .map_err(|e| e.to_string())?,
    ];
    if let Some(domain) = extract_server_authority(base_url) {
        tags.push(Tag::parse(vec!["server".to_string(), domain]).map_err(|e| e.to_string())?);
    }
    EventBuilder::new(Kind::from(24242), "Upload buzz-media")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| e.to_string())
}

/// Mint the `Authorization` header value for an upload.
pub(super) fn mint_upload_auth_header(
    state: &AppState,
    base_url: &str,
    sha256: &str,
    expiry_secs: u64,
) -> Result<String, String> {
    let keys = state.signing_keys()?;
    let auth_event = sign_blossom_upload_auth(&keys, sha256, expiry_secs, base_url)?;
    Ok(format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(auth_event.as_json().as_bytes())
    ))
}

/// Bracket this machine's clock offset against the relay's, with a short probe.
///
/// The local clock is read either side of the round trip, so the result is an
/// interval the observation actually proves rather than a point estimate that
/// network latency would inflate. `None` whenever no trustworthy measurement is
/// available — the probe failed, or the reply carried no parseable `Date`.
async fn measure_clock_bounds(state: &AppState, base_url: &str) -> Option<ClockOffsetBounds> {
    let before = Utc::now().timestamp_millis();
    let resp = state
        .http_client
        .head(format!("{base_url}/upload"))
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .timeout(CLOCK_PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;
    let after = Utc::now().timestamp_millis();
    // Only a cache sends `Age`, and a cached reply carries the origin's
    // original `Date` — a clock reading as stale as the cache entry. Refuse to
    // measure rather than try to correct for it.
    if resp.headers().contains_key(reqwest::header::AGE) {
        return None;
    }
    let date = resp.headers().get(reqwest::header::DATE)?.to_str().ok()?;
    ClockOffsetBounds::measure(before, after, date)
}

/// Explain a refused upload when this machine's clock is provably why.
///
/// `token_lifetime_secs` is the `expiration` this client stamped on the token:
/// a clock behind by more than that mints one already expired on arrival, which
/// is the tightest of the verifier's backward bounds and the only one the
/// client knows exactly.
///
/// Returns `None` for any other failure, and for a clock the relay would have
/// accepted — the caller then reports the relay's own message unchanged. A
/// cancelled upload is not probed: the user has stopped caring, and the probe
/// would hold the failure open for up to [`CLOCK_PROBE_TIMEOUT`].
pub(super) async fn diagnose_upload_rejection(
    state: &AppState,
    base_url: &str,
    status: reqwest::StatusCode,
    token_lifetime_secs: u64,
    cancellation: Option<&CancellationToken>,
) -> Option<String> {
    if !should_probe(status, cancellation.map(|c| c.is_cancelled())) {
        return None;
    }
    measure_clock_bounds(state, base_url)
        .await?
        .media_auth_advice(token_lifetime_secs)
}

/// Whether a failed upload is worth spending a clock probe on.
///
/// Split out from the async path so the guard is testable without a live
/// `AppState` or relay.
fn should_probe(status: reqwest::StatusCode, cancelled: Option<bool>) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED && cancelled != Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a 401 is worth a probe, and only while the user still wants the
    /// upload. Anything else must cost nothing.
    #[test]
    fn only_a_live_401_is_worth_probing() {
        assert!(should_probe(reqwest::StatusCode::UNAUTHORIZED, None));
        assert!(should_probe(
            reqwest::StatusCode::UNAUTHORIZED,
            Some(false)
        ));

        assert!(
            !should_probe(reqwest::StatusCode::UNAUTHORIZED, Some(true)),
            "a cancelled upload must not be held open for a diagnostic"
        );
        for other in [
            reqwest::StatusCode::FORBIDDEN,
            reqwest::StatusCode::PAYLOAD_TOO_LARGE,
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert!(!should_probe(other, None), "{other} is not an auth failure");
        }
    }

    /// A cancelled token is what `do_upload` actually holds, so pin the wiring
    /// rather than just the boolean.
    #[test]
    fn a_cancelled_token_suppresses_the_probe() {
        let token = CancellationToken::new();
        assert!(should_probe(
            reqwest::StatusCode::UNAUTHORIZED,
            Some(token.is_cancelled())
        ));
        token.cancel();
        assert!(!should_probe(
            reqwest::StatusCode::UNAUTHORIZED,
            Some(token.is_cancelled())
        ));
    }

    /// The token this module mints must carry the tags the relay's verifier
    /// requires, with an `expiration` derived from the lifetime it was given —
    /// that lifetime is also what the clock diagnosis measures against, so a
    /// drift between the two would silently misjudge every rejection.
    #[test]
    fn the_minted_token_carries_the_expiry_it_was_given() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let minted_at = Timestamp::now().as_secs();

        let event = sign_blossom_upload_auth(&keys, &sha256, 600, "https://relay.example")
            .expect("signing succeeds");

        assert_eq!(event.kind, Kind::from(24242));
        let tag = |name: &str| {
            event
                .tags
                .iter()
                .find(|t| t.as_slice().first().map(String::as_str) == Some(name))
                .and_then(|t| t.as_slice().get(1).cloned())
        };
        assert_eq!(tag("t").as_deref(), Some("upload"));
        assert_eq!(tag("x").as_deref(), Some(sha256.as_str()));
        assert_eq!(tag("server").as_deref(), Some("relay.example"));

        let expiration: u64 = tag("expiration")
            .expect("expiration tag")
            .parse()
            .expect("numeric expiration");
        assert!(
            expiration >= minted_at + 600 && expiration <= minted_at + 601,
            "expiration {expiration} must be the mint time plus the 600s lifetime"
        );
    }

    /// The `server` tag is omitted rather than faked when the base URL has no
    /// authority to derive one from.
    #[test]
    fn an_unusable_base_url_yields_no_server_tag() {
        let event = sign_blossom_upload_auth(&Keys::generate(), &"b".repeat(64), 300, "not-a-url")
            .expect("signing succeeds");
        assert!(!event
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(String::as_str) == Some("server")));
    }
}
