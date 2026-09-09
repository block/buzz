//! Navigation admission policy for plugin browser surfaces.
//!
//! The host, never the plugin, decides which addresses may load. Only
//! `http`/`https` addresses are admitted, and admission additionally excludes
//! the hosts that reach Buzz's own Tauri IPC surface. This table is shared by
//! plugin-resolved URLs ([`crate::plugin_host::mod`]'s `resolve`) and the
//! native `with_navigation_handler` admit closure, so both a plugin answer
//! and an in-page navigation are checked identically.

use url::Url;

/// Hosts that reach Buzz's own Tauri IPC surface, denied regardless of port.
const DENIED_HOSTS: &[&str] = &["ipc.localhost", "buzz-media.localhost", "tauri.localhost"];

/// The Tauri dev server port on `localhost`, denied explicitly.
const DENIED_LOCALHOST_PORT: u16 = 1420;

/// Maximum admitted address length.
const MAX_ADDRESS_LENGTH: usize = 2048;

/// Parses and validates one address against the allowed schemes and hosts.
///
/// Returns the parsed URL when admitted, `None` otherwise. A non-`http(s)`
/// scheme (`about:`, `file:`, `data:`, `javascript:`, `tauri:`,
/// `buzz-media:`, `asset:`, or any other) is always denied; among `http(s)`
/// addresses, the Tauri-owned hosts above are denied at any port and
/// `localhost` is denied specifically at port 1420.
pub fn parse_admitted(address: &str) -> Option<Url> {
    let trimmed = address.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_ADDRESS_LENGTH {
        return None;
    }
    let url = Url::parse(trimmed).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    if DENIED_HOSTS.contains(&host.as_str()) {
        return None;
    }
    if host == "localhost" && url.port() == Some(DENIED_LOCALHOST_PORT) {
        return None;
    }
    Some(url)
}

/// Returns whether `address` is admitted, discarding the parsed URL.
///
/// Used as the native adapter's `admit` closure, which only needs a
/// boolean decision.
pub fn is_admitted(address: &str) -> bool {
    parse_admitted(address).is_some()
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
