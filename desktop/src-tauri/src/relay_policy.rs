use serde::Serialize;
use url::Url;

pub const DEFAULT_RELAY_WS_URL: &str = "ws://localhost:3000";
const LOCKED_RELAY_ERROR: &str = "This Buzz build only allows its configured relay.";

pub(crate) fn compiled_relay_ws_url() -> Option<&'static str> {
    option_env!("BUZZ_DESKTOP_BUILD_RELAY_URL")
}

pub fn lock_to_default_relay_enabled() -> bool {
    option_env!("BUZZ_DESKTOP_BUILD_LOCK_TO_DEFAULT_RELAY").is_some()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayConnectionPolicy {
    pub locked_to_default_relay: bool,
    pub default_relay_url: String,
}

#[derive(Debug, PartialEq, Eq)]
struct RelayOrigin {
    scheme: String,
    host: String,
    port: u16,
}

fn relay_origin(raw: &str) -> Result<RelayOrigin, String> {
    let parsed = Url::parse(raw.trim()).map_err(|_| "invalid relay URL".to_string())?;
    if !matches!(parsed.scheme(), "ws" | "wss")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !matches!(parsed.path(), "" | "/")
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("relay URL must be a ws:// or wss:// origin".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "relay URL must include a host".to_string())?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "relay URL must include a valid port".to_string())?;
    Ok(RelayOrigin {
        scheme: parsed.scheme().to_string(),
        host: host.to_ascii_lowercase(),
        port,
    })
}

fn ensure_relay_url_allowed_with_policy(
    relay_url: &str,
    locked: bool,
    allowed_relay_url: Option<&str>,
) -> Result<(), String> {
    if !locked {
        return Ok(());
    }
    let allowed = allowed_relay_url.ok_or_else(|| LOCKED_RELAY_ERROR.to_string())?;
    let candidate_origin = relay_origin(relay_url).map_err(|_| LOCKED_RELAY_ERROR.to_string())?;
    let allowed_origin = relay_origin(allowed).map_err(|_| LOCKED_RELAY_ERROR.to_string())?;
    if candidate_origin != allowed_origin {
        return Err(LOCKED_RELAY_ERROR.to_string());
    }
    Ok(())
}

pub fn ensure_relay_url_allowed(relay_url: &str) -> Result<(), String> {
    ensure_relay_url_allowed_with_policy(
        relay_url,
        lock_to_default_relay_enabled(),
        compiled_relay_ws_url(),
    )
}

pub fn relay_connection_policy() -> Result<RelayConnectionPolicy, String> {
    let locked = lock_to_default_relay_enabled();
    let default_relay_url = compiled_relay_ws_url()
        .unwrap_or(DEFAULT_RELAY_WS_URL)
        .to_string();
    ensure_relay_url_allowed_with_policy(&default_relay_url, locked, compiled_relay_ws_url())?;
    Ok(RelayConnectionPolicy {
        locked_to_default_relay: locked,
        default_relay_url,
    })
}

#[cfg(test)]
mod tests {
    use super::{ensure_relay_url_allowed_with_policy, relay_origin};

    #[test]
    fn unlocked_build_allows_any_relay_string() {
        assert!(ensure_relay_url_allowed_with_policy("not a URL", false, None).is_ok());
    }

    #[test]
    fn locked_build_accepts_only_the_compiled_origin() {
        let allowed = "wss://buzz.block.builderlab.xyz";
        assert!(ensure_relay_url_allowed_with_policy(
            "wss://BUZZ.block.builderlab.xyz/",
            true,
            Some(allowed),
        )
        .is_ok());

        for rejected in [
            "ws://buzz.block.builderlab.xyz",
            "wss://public.example.com",
            "wss://public.buzz.block.builderlab.xyz",
            "wss://buzz.block.builderlab.xyz:8443",
            "wss://buzz.block.builderlab.xyz/path",
            "wss://user@buzz.block.builderlab.xyz",
            "wss://buzz.block.builderlab.xyz?redirect=public.example.com",
        ] {
            assert!(
                ensure_relay_url_allowed_with_policy(rejected, true, Some(allowed)).is_err(),
                "expected {rejected} to be rejected"
            );
        }
    }

    #[test]
    fn locked_build_fails_closed_without_a_compiled_relay() {
        assert!(ensure_relay_url_allowed_with_policy(
            "wss://buzz.block.builderlab.xyz",
            true,
            None,
        )
        .is_err());
    }

    #[test]
    fn relay_origin_requires_a_bare_websocket_origin() {
        assert!(relay_origin("https://buzz.block.builderlab.xyz").is_err());
        assert!(relay_origin("wss://buzz.block.builderlab.xyz/channel").is_err());
        assert!(relay_origin("wss://buzz.block.builderlab.xyz").is_ok());
    }
}
