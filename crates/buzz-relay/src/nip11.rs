//! NIP-11 relay information document.

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::config::DEFAULT_MAX_FRAME_BYTES;

/// NIPs unconditionally supported by this relay, advertised in the NIP-11
/// document. Kept as a module-level constant so tests can verify it without
/// constructing a full `Config` (which reads env vars and races with
/// config.rs tests).
///
/// NIP-43 (relay membership) is advertised separately by [`RelayInfo::build`]
/// only when membership enforcement is actually enabled — see that function.
pub(crate) const SUPPORTED_NIPS: &[u32] = &[1, 2, 10, 11, 16, 17, 23, 25, 29, 33, 38, 42, 50];

/// NIP-43 (relay membership). Advertised only when the relay actually
/// enforces membership (`BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`) AND has a
/// stable signing key — both are required for kind 13534/8000/8001 events
/// to be verifiable by clients.
pub(crate) const NIP_RELAY_MEMBERSHIP: u32 = 43;

/// Relay information document served at `GET /` with `Accept: application/nostr+json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayInfo {
    /// Human-readable relay name.
    pub name: String,
    /// Human-readable relay description.
    pub description: String,
    /// Workspace icon URL (NIP-11 `icon`), per-community, set by relay
    /// admins/owners via the kind:9033 command. Omitted when no icon is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Relay operator's public key (hex), if published.
    pub pubkey: Option<String>,
    /// Contact address for the relay operator.
    pub contact: Option<String>,
    /// NIPs supported by this relay.
    pub supported_nips: Vec<u32>,
    /// Draft/extension protocol identifiers supported by this relay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_extensions: Option<Vec<String>>,
    /// URL of the relay software repository.
    pub software: String,
    /// Relay software version string.
    pub version: String,
    /// Protocol and resource limits advertised to clients.
    pub limitation: Option<RelayLimitation>,
    /// Relay's own signing pubkey (NIP-11 `self` field, NIP-43).
    #[serde(rename = "self", skip_serializing_if = "Option::is_none")]
    pub relay_self: Option<String>,
}

/// Protocol and resource limits advertised in the NIP-11 document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayLimitation {
    /// Maximum WebSocket frame size in bytes.
    pub max_message_length: Option<u64>,
    /// Maximum number of concurrent subscriptions per connection.
    pub max_subscriptions: Option<u32>,
    /// Maximum number of filters per subscription.
    pub max_filters: Option<u32>,
    /// Maximum value of the `limit` field in a filter.
    pub max_limit: Option<u32>,
    /// Maximum length of a subscription ID string.
    pub max_subid_length: Option<u32>,
    /// Minimum proof-of-work difficulty required for events.
    pub min_pow_difficulty: Option<u32>,
    /// Whether NIP-42 authentication is required before subscribing or
    /// publishing events.
    pub auth_required: bool,
    /// Whether payment is required to use the relay.
    pub payment_required: bool,
    /// Whether writes are restricted to authorized pubkeys.
    pub restricted_writes: bool,
    /// NIP-ER: how the relay delivers due reminders ("push" or "lazy").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_delivery_mode: Option<String>,
    /// NIP-ER: maximum allowed `not_before` horizon in seconds from now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_not_before_delta: Option<u64>,
}

/// Canonical `RelayLimitation` advertised by this relay.
///
/// `auth_required` is always `true`: the REQ, EVENT, and COUNT handlers
/// unconditionally reject connections that are not in
/// `AuthState::Authenticated`. This is independent of the REST API token
/// toggle (`config.require_auth_token`).
fn relay_limitation(max_message_length: usize) -> RelayLimitation {
    let max_not_before_delta: u64 = std::env::var("SPROUT_MAX_NOT_BEFORE_DELTA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(31_536_000); // 1 year default

    RelayLimitation {
        max_message_length: Some(max_message_length as u64),
        max_subscriptions: Some(1024),
        max_filters: Some(10),
        max_limit: Some(10_000),
        max_subid_length: Some(256),
        min_pow_difficulty: None,
        auth_required: true,
        payment_required: false,
        restricted_writes: true,
        due_delivery_mode: Some("push".to_string()),
        max_not_before_delta: Some(max_not_before_delta),
    }
}

impl RelayInfo {
    /// Builds the relay's NIP-11 information document.
    ///
    /// `relay_self` is the relay's own signing pubkey (hex), advertised as the
    /// NIP-11 `self` field. NIP-11 defines `self` generically as the relay's
    /// identity key; other NIPs reference it. Notably NIP-29 (group metadata
    /// kinds 39000/39001/39002, which Buzz signs with `state.relay_keypair`
    /// unconditionally) requires clients to verify those events against
    /// `self`. Pass `Some` whenever the relay has a stable signing key.
    ///
    /// `icon` is the community's workspace icon (see
    /// [`workspace_icon_for_host`]) — a host-scoped scalar, pre-fetched by
    /// the caller so `build` itself stays static-input.
    ///
    /// `advertise_nip43` controls whether NIP-43 (relay membership) is added
    /// to `supported_nips`. Set `true` only when the relay actually emits and
    /// gates on NIP-43 events — i.e. has a stable key AND enforces
    /// membership. NIP-43 events are verified against `self`, so it is a
    /// programmer error to advertise NIP-43 without a `relay_self`.
    pub fn build(
        relay_self: Option<&str>,
        icon: Option<&str>,
        advertise_nip43: bool,
        max_message_length: usize,
    ) -> Self {
        debug_assert!(
            !advertise_nip43 || relay_self.is_some(),
            "advertise_nip43=true requires relay_self=Some — NIP-43 events are verified against `self`"
        );

        let mut supported_nips = SUPPORTED_NIPS.to_vec();
        if advertise_nip43 {
            supported_nips.push(NIP_RELAY_MEMBERSHIP);
        }

        Self {
            name: "Buzz Relay".to_string(),
            description: "Buzz — private team communication relay".to_string(),
            icon: icon.filter(|s| !s.is_empty()).map(|s| s.to_string()),
            pubkey: None,
            contact: None,
            supported_nips,
            supported_extensions: Some(vec!["nip-er".to_string()]),
            software: "https://github.com/block/buzz".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            limitation: Some(relay_limitation(max_message_length)),
            relay_self: relay_self.map(|s| s.to_string()),
        }
    }
}

/// Axum handler that returns the NIP-11 relay information document as JSON.
pub async fn relay_info_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::state::AppState>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Json<RelayInfo> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    axum::response::Json(nip11_document(&state, raw_host).await)
}

/// Builds the served NIP-11 document for a request arriving on `raw_host`.
///
/// Centralised so the content-negotiated root handler and the dedicated
/// `/info` endpoint can't drift apart. Every input to `RelayInfo::build`
/// stays a pre-derived scalar: [`nip11_facts`] (config + keypair) plus the
/// host-scoped workspace icon.
pub(crate) async fn nip11_document(state: &crate::state::AppState, raw_host: &str) -> RelayInfo {
    let (relay_self, advertise_nip43) = nip11_facts(state);
    let icon = workspace_icon_for_host(state, raw_host).await;
    RelayInfo::build(
        relay_self.as_deref(),
        icon.as_deref(),
        advertise_nip43,
        state.config.max_frame_bytes,
    )
}

/// Fetches the workspace icon for the community bound to `raw_host`, as the
/// host-scoped scalar consumed by [`RelayInfo::build`].
///
/// The icon is per-community state (`communities.icon`, set by relay
/// admins/owners via the kind:9033 command) served in the standard NIP-11
/// `icon` field. The lookup is scoped through
/// [`crate::tenant::bind_community`] — never an unscoped query. Fails open to
/// `None` (no `icon` field): NIP-11 is intentionally served to unmapped hosts
/// too, and an icon lookup failure must not break that.
async fn workspace_icon_for_host(state: &crate::state::AppState, raw_host: &str) -> Option<String> {
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .ok()?;
    state
        .db
        .get_community_icon(tenant.community())
        .await
        .ok()
        .flatten()
}

/// Derives the two NIP-11 facts that depend on runtime config:
///
/// - `relay_self`: the NIP-11 `self` pubkey, set whenever the relay has a
///   stable signing key. Consumed by NIP-29 (group metadata verification)
///   and NIP-43, among others. Ephemeral keys are excluded because they
///   change on restart, leaving previously-signed events unverifiable.
/// - `advertise_nip43`: whether to list NIP-43 in `supported_nips`. True
///   only when membership is actually enforced AND we have a stable key
///   (NIP-43 events must be verifiable against `self`).
///
/// Centralised so the content-negotiated root handler and the dedicated
/// `/info` endpoint can't drift apart.
pub(crate) fn nip11_facts(state: &crate::state::AppState) -> (Option<String>, bool) {
    let has_stable_key = state.config.relay_private_key.is_some();
    let relay_self = has_stable_key.then(|| state.relay_keypair.public_key().to_hex());
    let advertise_nip43 = has_stable_key && state.config.require_relay_membership;
    (relay_self, advertise_nip43)
}

/// Multi-tenant conformance static-input fence (surface row "NIP-11 relay info
/// and relay `self`").
///
/// The conformance obligation: `RelayInfo::build` "must not grow unscoped
/// DB/search/audit inputs", so an unauthenticated NIP-11 read can never become
/// an enumeration oracle for *other* communities. `build` takes only static
/// and scalar inputs — the per-deployment facts arrive pre-derived through
/// [`nip11_facts`] (config + relay keypair), and the one host-scoped fact
/// (the workspace `icon`) arrives as a scalar from
/// [`workspace_icon_for_host`], whose DB lookup is scoped through
/// [`crate::tenant::bind_community`] and can therefore only ever surface the
/// requesting host's own community state.
///
/// This const binds `RelayInfo::build` to its **exact** allowed signature. The
/// moment someone adds a `&Db`, `&AppState`, a search handle, an audit handle,
/// or any other unscoped input, the function pointer's type stops matching and
/// **this file fails to compile** — turning a silent cross-tenant leak into a
/// hard build break, the same way a deny-lint would. If you must change this
/// signature, you are changing the conformance contract: update the conformance
/// doc and prove the new input is host-scoped, not unscoped, first.
const _RELAY_INFO_BUILD_STATIC_INPUT_FENCE: fn(
    Option<&str>,
    Option<&str>,
    bool,
    usize,
) -> RelayInfo = RelayInfo::build;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_nips_includes_nip23_and_nip33() {
        // Tests the production SUPPORTED_NIPS constant directly — no Config::from_env()
        // needed, avoiding the env-var race with config.rs tests.
        assert!(
            SUPPORTED_NIPS.contains(&23),
            "NIP-23 (long-form content) must be advertised"
        );
        assert!(
            SUPPORTED_NIPS.contains(&33),
            "NIP-33 (parameterized replaceable) must be advertised"
        );
    }

    #[test]
    fn supported_nips_includes_nip38() {
        assert!(
            SUPPORTED_NIPS.contains(&38),
            "NIP-38 (user statuses) must be advertised"
        );
    }

    #[test]
    fn build_advertises_buzz_repository_url() {
        let info = RelayInfo::build(None, None, false, DEFAULT_MAX_FRAME_BYTES);
        assert_eq!(info.software, "https://github.com/block/buzz");
    }

    /// NIP-WP → NIP-11 mirror: a set workspace icon is served in the standard
    /// `icon` field; no icon (or a cleared, empty icon) omits the field
    /// entirely so the JSON matches pre-icon documents byte-for-byte.
    #[test]
    fn icon_is_mirrored_and_empty_or_absent_is_omitted() {
        let info = RelayInfo::build(
            None,
            Some("data:image/webp;base64,UklGRg=="),
            false,
            DEFAULT_MAX_FRAME_BYTES,
        );
        assert_eq!(
            info.icon.as_deref(),
            Some("data:image/webp;base64,UklGRg==")
        );
        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(
            json.get("icon").and_then(|v| v.as_str()),
            Some("data:image/webp;base64,UklGRg==")
        );

        for icon in [None, Some("")] {
            let info = RelayInfo::build(None, icon, false, DEFAULT_MAX_FRAME_BYTES);
            assert!(info.icon.is_none());
            let json = serde_json::to_value(&info).expect("serialize");
            assert!(
                json.get("icon").is_none(),
                "unset/cleared icon must omit the `icon` field, not serialize null/empty"
            );
        }
    }

    #[test]
    fn auth_required_is_advertised_true() {
        // REQ, EVENT, and COUNT all unconditionally require
        // `AuthState::Authenticated` (see `crates/buzz-relay/src/handlers/`),
        // so the NIP-11 doc must advertise it.
        assert!(relay_limitation(DEFAULT_MAX_FRAME_BYTES).auth_required);
    }

    #[test]
    fn max_message_length_uses_configured_frame_limit() {
        let info = RelayInfo::build(None, None, false, 262_144);
        let limitation = info.limitation.expect("limitation");
        assert_eq!(limitation.max_message_length, Some(262_144));
    }

    #[test]
    fn supported_nips_are_sorted() {
        let mut sorted = SUPPORTED_NIPS.to_vec();
        sorted.sort();
        assert_eq!(
            SUPPORTED_NIPS,
            &sorted[..],
            "supported_nips should be sorted"
        );
    }

    #[test]
    fn nip43_not_in_static_supported_nips() {
        // NIP-43 advertisement is conditional on runtime config (stable signing
        // key + membership enforcement) and must NOT live in the static list.
        // The desktop pairing probe keys off this NIP — advertising it on
        // open relays misroutes pairing peers to a non-existent /pair sidecar.
        assert!(
            !SUPPORTED_NIPS.contains(&NIP_RELAY_MEMBERSHIP),
            "NIP-43 must be advertised only when advertise_nip43=true is passed to RelayInfo::build"
        );
    }

    /// Open relay, ephemeral key — both `self` and NIP-43 are absent.
    #[test]
    fn build_open_relay_ephemeral_key_omits_self_and_nip43() {
        let info = RelayInfo::build(None, None, false, DEFAULT_MAX_FRAME_BYTES);
        assert!(info.relay_self.is_none());
        assert!(!info.supported_nips.contains(&NIP_RELAY_MEMBERSHIP));
    }

    /// Open relay with a stable signing key (e.g. for NIP-29 group metadata
    /// signing): `self` MUST be advertised so clients can verify those
    /// events; NIP-43 must NOT be, because the relay isn't enforcing
    /// membership. This is the staging-default shape — the bug we're
    /// fixing — and the regression we must not reintroduce.
    #[test]
    fn build_open_relay_stable_key_advertises_self_but_not_nip43() {
        let pk = "0000000000000000000000000000000000000000000000000000000000000001";
        let info = RelayInfo::build(Some(pk), None, false, DEFAULT_MAX_FRAME_BYTES);
        assert_eq!(info.relay_self.as_deref(), Some(pk));
        assert!(!info.supported_nips.contains(&NIP_RELAY_MEMBERSHIP));
    }

    /// Membership-enforcing relay: both `self` and NIP-43 advertised.
    #[test]
    fn build_membership_relay_advertises_self_and_nip43() {
        let pk = "0000000000000000000000000000000000000000000000000000000000000001";
        let info = RelayInfo::build(Some(pk), None, true, DEFAULT_MAX_FRAME_BYTES);
        assert_eq!(info.relay_self.as_deref(), Some(pk));
        assert!(info.supported_nips.contains(&NIP_RELAY_MEMBERSHIP));
    }

    /// NIP-43 events are verified against `self`; advertising NIP-43 without
    /// `self` would give clients no way to verify membership events. The
    /// debug_assert in `build` catches this in tests/debug builds.
    #[test]
    #[should_panic(expected = "advertise_nip43=true requires relay_self=Some")]
    fn build_nip43_without_self_panics_in_debug() {
        let _ = RelayInfo::build(None, None, true, DEFAULT_MAX_FRAME_BYTES);
    }
}
